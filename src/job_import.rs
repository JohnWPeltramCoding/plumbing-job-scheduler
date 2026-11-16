//! Import jobs from another system's CSV export (see
//! templates/Jobs Import Guide.md for the file layout and rules).
//!
//! - A job is recognized by its **Source ID**: new IDs are added, known ones
//!   updated, and nothing is ever deleted.
//! - A blank cell leaves the job's current value alone; `CLEAR` erases it.
//! - On existing jobs, crew / date / time / hours / days / weekend settings
//!   only change when "update scheduling" is on, so a job a scheduler moved
//!   in the app isn't moved back by the next export.
//! - Rows with errors are skipped and listed; the rest import.
//! - The database is backed up before anything is saved.

use std::collections::HashSet;

use chrono::NaiveDate;
use eframe::egui::{self, vec2, Button, CollapsingHeader, Color32, Grid, RichText, ScrollArea};

use crate::app::{today, SchedulerApp};
use crate::db::Db;
use crate::model::*;

const CLEAR: &str = "CLEAR";
const WARN: Color32 = Color32::from_rgb(220, 140, 20);
const ERR: Color32 = Color32::from_rgb(210, 50, 50);

// ------------------------------------------------------------ CSV reading

/// Reads CSV text into records, each with the line number it starts on.
/// Handles quoted fields with commas, doubled quotes and line breaks, CRLF
/// or LF line endings, and a UTF-8 byte-order mark.
pub fn parse_csv(text: &str) -> Vec<(usize, Vec<String>)> {
    let text = text.trim_start_matches('\u{feff}');
    let mut records = Vec::new();
    let mut fields: Vec<String> = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut line, mut start) = (false, 1usize, 1usize);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' if quoted => quoted = false,
            '"' if cur.trim().is_empty() => {
                cur.clear();
                quoted = true;
            }
            ',' if !quoted => fields.push(std::mem::take(&mut cur)),
            '\r' if !quoted => {}
            '\n' if !quoted => {
                fields.push(std::mem::take(&mut cur));
                records.push((start, std::mem::take(&mut fields)));
                line += 1;
                start = line;
            }
            '\n' => {
                cur.push('\n');
                line += 1;
            }
            '\r' => {}
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() || !fields.is_empty() {
        fields.push(cur);
        records.push((start, fields));
    }
    records
}

// ------------------------------------------------------------ value parsing

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn parse_schedule(s: &str) -> Option<Board> {
    match norm(s).replace(['-', ' '], "").as_str() {
        "roughin" | "ri" | "rough" => Some(Board::RoughIn),
        "trim" | "tr" => Some(Board::Trim),
        _ => None,
    }
}

fn parse_status(s: &str) -> Option<&'static str> {
    STATUSES.iter().copied().find(|st| norm(st) == norm(s))
}

/// YYYY-MM-DD or M/D/YYYY (also M/D/YY).
pub fn parse_date(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(d);
    }
    let p: Vec<&str> = s.split('/').collect();
    if p.len() != 3 {
        return None;
    }
    let (m, d, y): (u32, u32, i32) = (p[0].trim().parse().ok()?, p[1].trim().parse().ok()?, p[2].trim().parse().ok()?);
    NaiveDate::from_ymd_opt(if y < 100 { 2000 + y } else { y }, m, d)
}

/// "7:00 AM", "7 AM", "12:30 pm", "07:00", "13:30" -> minutes after
/// midnight, rounded to 15 minutes.
pub fn parse_time(s: &str) -> Option<i32> {
    let t = s.trim().to_lowercase().replace('.', "");
    let (clock, ampm) = if let Some(x) = t.strip_suffix("am") {
        (x.trim().to_string(), Some(false))
    } else if let Some(x) = t.strip_suffix("pm") {
        (x.trim().to_string(), Some(true))
    } else {
        (t.clone(), None)
    };
    let (h, m): (i32, i32) = match clock.split_once(':') {
        Some((h, m)) => (h.trim().parse().ok()?, m.trim().get(..2).unwrap_or(m.trim()).parse().ok()?),
        None => (clock.parse().ok()?, 0),
    };
    if !(0..60).contains(&m) {
        return None;
    }
    let h = match ampm {
        Some(pm) if (1..=12).contains(&h) => (h % 12) + if pm { 12 } else { 0 },
        Some(_) => return None,
        None if (0..24).contains(&h) => h,
        None => return None,
    };
    let total = h * 60 + m;
    Some(((total + 7) / 15 * 15).min(23 * 60 + 45))
}

fn parse_yes_no(s: &str) -> Option<bool> {
    match norm(s).as_str() {
        "yes" | "y" | "true" | "1" | "x" => Some(true),
        "no" | "n" | "false" | "0" => Some(false),
        _ => None,
    }
}

// ------------------------------------------------------------ planning

/// One problem with a row.
#[derive(Clone, Debug, PartialEq)]
pub struct RowNote {
    /// Line in the file where the row starts.
    pub line: usize,
    pub source_id: String,
    pub message: String,
}

#[derive(Default, Debug)]
pub struct JobPlan {
    pub add: Vec<Job>,
    /// Existing jobs with their changes, and what changed.
    pub update: Vec<(Job, Vec<String>)>,
    pub unchanged: usize,
    /// Rows skipped because of an error.
    pub errors: Vec<RowNote>,
    /// Rows imported, but worth a look.
    pub warnings: Vec<RowNote>,
    /// Existing jobs whose crew/date/time in the file differ from the app,
    /// left alone because "update scheduling" is off.
    pub schedule_differs: usize,
    /// Columns in the file the import doesn't use.
    pub ignored_columns: Vec<String>,
}

/// Column positions found in the header row.
struct Cols {
    source: usize,
    schedule: usize,
    code: usize,
    name: Option<usize>,
    lot: Option<usize>,
    phase: Option<usize>,
    status: Option<usize>,
    crew: Option<usize>,
    date: Option<usize>,
    time: Option<usize>,
    hours: Option<usize>,
    days: Option<usize>,
    sat: Option<usize>,
    sun: Option<usize>,
    tank: Option<usize>,
    sink: Option<usize>,
    notes: Option<usize>,
    comments: Option<usize>,
    /// (column, Additional Field name) for "Field: …" columns.
    fields: Vec<(usize, String)>,
}

fn changed_fields(a: &Job, b: &Job) -> Vec<String> {
    let mut out = Vec::new();
    let mut check = |name: &str, same: bool| {
        if !same {
            out.push(name.to_string());
        }
    };
    check("Schedule", a.board == b.board);
    check("Project Code", a.project_code == b.project_code);
    check("Project Name", a.project_name == b.project_name);
    check("Lot", a.lot == b.lot);
    check("Phase", a.phase == b.phase);
    check("Status", a.status == b.status);
    check("Crew", a.crew_id == b.crew_id);
    check("Start Date", a.date == b.date);
    check("Start Time", a.start_min == b.start_min);
    check("Hours Per Day", a.duration_min == b.duration_min);
    check("Days", a.days == b.days);
    check("Weekend", a.weekend == b.weekend);
    check("Water Tank Size", a.tank_size == b.tank_size);
    check("Kitchen Sink", a.kitchen_sink == b.kitchen_sink);
    check("Notes", a.notes == b.notes);
    check("Comments", a.comments == b.comments);
    check("Additional Fields", a.fields == b.fields);
    check("Source ID", a.source_id == b.source_id);
    out
}

/// Works out what importing `csv` would do. `update_scheduling` lets the
/// file change crews, dates and times on jobs already in the app.
pub fn plan(csv: &str, jobs: &[Job], crews: &[Crew], update_scheduling: bool, today: NaiveDate) -> Result<JobPlan, String> {
    let mut records = parse_csv(csv).into_iter().filter(|(_, r)| r.iter().any(|f| !f.trim().is_empty()));
    let (_, header) = records.next().ok_or("The file is empty.")?;
    let names: Vec<String> = header.iter().map(|h| norm(h)).collect();
    let col = |n: &str| names.iter().position(|h| h == n);
    let need = |n: &str, label: &str| col(n).ok_or_else(|| format!("The header row has no “{label}” column. Start from Jobs Import Template.csv."));
    let mut plan = JobPlan::default();
    let known = [
        "source id", "schedule", "project code", "project name", "lot", "phase", "status", "crew", "start date",
        "start time", "hours per day", "days", "include saturday", "include sunday", "water tank size",
        "kitchen sink", "notes", "comments",
    ];
    let mut fields = Vec::new();
    for (i, (raw, n)) in header.iter().zip(&names).enumerate() {
        if n.starts_with("field:") {
            let label = raw.split_once(':').map(|x| x.1.trim()).unwrap_or("").to_string();
            if !label.is_empty() {
                fields.push((i, label));
            }
        } else if !n.is_empty() && !known.contains(&n.as_str()) {
            plan.ignored_columns.push(raw.trim().to_string());
        }
    }
    let c = Cols {
        source: need("source id", "Source ID")?,
        schedule: need("schedule", "Schedule")?,
        code: need("project code", "Project Code")?,
        name: col("project name"),
        lot: col("lot"),
        phase: col("phase"),
        status: col("status"),
        crew: col("crew"),
        date: col("start date"),
        time: col("start time"),
        hours: col("hours per day"),
        days: col("days"),
        sat: col("include saturday"),
        sun: col("include sunday"),
        tank: col("water tank size"),
        sink: col("kitchen sink"),
        notes: col("notes"),
        comments: col("comments"),
        fields,
    };

    let mut seen: HashSet<String> = HashSet::new();
    for (line, row) in records {
        // Text cells keep inner line breaks (notes) but lose outer spaces.
        let cell = |i: Option<usize>| -> String { i.and_then(|i| row.get(i)).map(|s| s.trim().to_string()).unwrap_or_default() };
        let is_clear = |s: &str| s.eq_ignore_ascii_case(CLEAR);
        let src = cell(Some(c.source));
        let mut errs: Vec<String> = Vec::new();
        let mut warns: Vec<String> = Vec::new();
        if src.is_empty() {
            errs.push("Source ID is missing".into());
        } else if !seen.insert(src.to_lowercase()) {
            errs.push("this Source ID appears more than once in the file (only the first row is used)".into());
        }
        let board = parse_schedule(&cell(Some(c.schedule)));
        if board.is_none() {
            errs.push(format!("Schedule “{}” must be Rough-In or Trim", cell(Some(c.schedule))));
        }
        let code = cell(Some(c.code));
        if code.is_empty() || is_clear(&code) {
            errs.push("Project Code is missing".into());
        }
        let status_cell = cell(c.status);
        let status = if status_cell.is_empty() {
            None
        } else {
            match parse_status(&status_cell) {
                Some(s) => Some(s),
                None => {
                    errs.push(format!("Status “{status_cell}” isn't one of: {}", STATUSES.join(", ")));
                    None
                }
            }
        };
        // Scheduling cells: None = blank, Some(None) = CLEAR, Some(Some(v)) = a value.
        let date_cell = cell(c.date);
        let date: Option<Option<NaiveDate>> = if date_cell.is_empty() {
            None
        } else if is_clear(&date_cell) {
            Some(None)
        } else {
            match parse_date(&date_cell) {
                Some(d) => {
                    if (d - today).num_days().abs() > 365 {
                        warns.push(format!("Start Date {} is more than a year from today", d.format("%b %-d, %Y")));
                    }
                    Some(Some(d))
                }
                None => {
                    errs.push(format!("Start Date “{date_cell}” isn't a date (use YYYY-MM-DD)"));
                    None
                }
            }
        };
        let time_cell = cell(c.time);
        let time = if time_cell.is_empty() || is_clear(&time_cell) {
            None
        } else {
            let t = parse_time(&time_cell);
            if t.is_none() {
                errs.push(format!("Start Time “{time_cell}” isn't a time (use 7:00 AM or 07:00)"));
            }
            t
        };
        let hours_cell = cell(c.hours);
        let hours = if hours_cell.is_empty() || is_clear(&hours_cell) {
            None
        } else {
            match hours_cell.parse::<f64>() {
                Ok(h) if (0.25..=24.0).contains(&h) => Some(((h * 60.0 / 15.0).round() as i32) * 15),
                _ => {
                    errs.push(format!("Hours Per Day “{hours_cell}” must be a number from 0.25 to 24"));
                    None
                }
            }
        };
        let days_cell = cell(c.days);
        let days = if days_cell.is_empty() || is_clear(&days_cell) {
            None
        } else {
            match days_cell.parse::<i32>() {
                Ok(d) if (1..=MAX_JOB_DAYS).contains(&d) => Some(d),
                _ => {
                    errs.push(format!("Days “{days_cell}” must be a whole number from 1 to {MAX_JOB_DAYS}"));
                    None
                }
            }
        };
        let mut yes_no = |i: Option<usize>, label: &str| -> Option<bool> {
            let v = cell(i);
            if v.is_empty() || is_clear(&v) {
                return None;
            }
            let b = parse_yes_no(&v);
            if b.is_none() {
                errs.push(format!("{label} “{v}” must be Yes or No"));
            }
            b
        };
        let sat = yes_no(c.sat, "Include Saturday");
        let sun = yes_no(c.sun, "Include Sunday");
        let crew_cell = cell(c.crew);
        // None = blank, Some(None) = unassign, Some(Some(id)) = a crew.
        let crew: Option<Option<i64>> = if crew_cell.is_empty() {
            None
        } else if is_clear(&crew_cell) {
            Some(None)
        } else {
            match crews.iter().find(|cr| norm(&cr.name) == norm(&crew_cell)) {
                Some(cr) => {
                    if Some(cr.board) != board && board.is_some() {
                        warns.push(format!("crew “{}” is a {} crew on a {} job", cr.name, cr.board.as_str(), board.unwrap().as_str()));
                    }
                    Some(Some(cr.id))
                }
                None => {
                    warns.push(format!("crew “{crew_cell}” isn't a crew in the scheduler, so the crew wasn't set"));
                    None
                }
            }
        };

        if !errs.is_empty() {
            for m in errs {
                plan.errors.push(RowNote { line, source_id: src.clone(), message: m });
            }
            continue;
        }
        let board = board.unwrap();
        let existing = jobs.iter().find(|j| !j.source_id.is_empty() && j.source_id.eq_ignore_ascii_case(&src));
        let mut job = match existing {
            Some(j) => j.clone(),
            None => {
                let mut j = Job::new(None, 7 * 60);
                j.status = STATUSES[0].into();
                j
            }
        };
        let is_new = existing.is_none();
        job.source_id = src.clone();
        job.board = board;
        job.project_code = code;
        let set_text = |target: &mut String, v: String| {
            if is_clear(&v) {
                target.clear();
            } else if !v.is_empty() {
                *target = v;
            }
        };
        set_text(&mut job.project_name, cell(c.name));
        set_text(&mut job.lot, cell(c.lot));
        set_text(&mut job.phase, cell(c.phase));
        set_text(&mut job.tank_size, cell(c.tank));
        set_text(&mut job.kitchen_sink, cell(c.sink));
        set_text(&mut job.notes, cell(c.notes));
        set_text(&mut job.comments, cell(c.comments));
        if let Some(s) = status {
            job.status = s.into();
        }
        for (i, label) in &c.fields {
            let v = cell(Some(*i));
            let pos = job.fields.iter().position(|f| norm(&f.label) == norm(label));
            if is_clear(&v) {
                if let Some(p) = pos {
                    job.fields.remove(p);
                }
            } else if !v.is_empty() {
                match pos {
                    Some(p) => job.fields[p].value = v,
                    None => job.fields.push(CustomField { label: label.clone(), value: v }),
                }
            }
        }

        // Scheduling: always for new jobs; for existing ones only when allowed.
        let before = job.clone();
        let mut sched = job.clone();
        if let Some(d) = date {
            sched.date = d;
        }
        if let Some(t) = time {
            sched.start_min = t;
        }
        if let Some(h) = hours {
            sched.duration_min = h;
        }
        if let Some(d) = days {
            sched.days = d;
        }
        if let Some(s) = sat {
            sched.weekend.sat = s;
        }
        if let Some(s) = sun {
            sched.weekend.sun = s;
        }
        if let Some(cr) = crew {
            sched.crew_id = cr;
        }
        if is_new || update_scheduling {
            job = sched;
        } else if sched != before {
            plan.schedule_differs += 1;
        }

        for m in warns {
            plan.warnings.push(RowNote { line, source_id: src.clone(), message: m });
        }
        match existing {
            None => plan.add.push(job),
            Some(old) if *old == job => plan.unchanged += 1,
            Some(old) => {
                let what = changed_fields(old, &job);
                plan.update.push((job, what));
            }
        }
    }
    Ok(plan)
}

/// Backs the database up next to itself before an import. Returns the
/// backup's file name (None for a temporary in-memory database).
fn backup(db: &Db) -> Result<Option<String>, String> {
    let Some(dir) = db.folder() else { return Ok(None) };
    let name = format!("schedule.backup-{}-before-job-import.db", chrono::Local::now().format("%Y%m%d-%H%M%S"));
    match db.backup_to(&dir.join(&name)) {
        Ok(true) => Ok(Some(name)),
        Ok(false) => Ok(None),
        Err(e) => Err(format!("Couldn't back up the database first, so nothing was imported: {e}")),
    }
}

fn save(db: &mut Db, plan: &JobPlan, user: Option<i64>) -> Result<Option<String>, String> {
    let backup_name = backup(db)?;
    let all: Vec<Job> = plan.add.iter().cloned().chain(plan.update.iter().map(|(j, _)| j.clone())).collect();
    db.save_jobs(&all, user).map_err(|e| format!("Import failed, nothing was changed: {e}"))?;
    Ok(backup_name)
}

// ------------------------------------------------------------ window

/// A job import waiting for the user to check the preview.
pub struct JobImport {
    file: String,
    text: String,
    update_scheduling: bool,
    plan: JobPlan,
}

/// Asks for the CSV file and opens the preview.
pub fn start(app: &mut SchedulerApp) {
    let Some(path) = rfd::FileDialog::new()
        .set_title("Import jobs (CSV)")
        .add_filter("CSV", &["csv", "txt"])
        .add_filter("All files", &["*"])
        .pick_file()
    else {
        return;
    };
    let file = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    let text = match std::fs::read(&path) {
        Ok(b) => String::from_utf8_lossy(&b).to_string(),
        Err(e) => return app.set_error(format!("Could not read {file}: {e}")),
    };
    match plan(&text, &app.jobs, &app.crews, false, today()) {
        Ok(p) => app.job_import = Some(JobImport { file, text, update_scheduling: false, plan: p }),
        Err(e) => app.set_error(format!("{file}: {e}")),
    }
}

fn notes_grid(ui: &mut egui::Ui, id: &str, notes: &[RowNote], color: Color32) {
    Grid::new(id).striped(true).spacing([14.0, 3.0]).show(ui, |ui| {
        for n in notes {
            ui.label(RichText::new(format!("line {}", n.line)).weak());
            ui.label(if n.source_id.is_empty() { "(no Source ID)".to_string() } else { n.source_id.clone() });
            ui.colored_label(color, &n.message);
            ui.end_row();
        }
    });
}

/// The preview window.
pub fn window(app: &mut SchedulerApp, ctx: &egui::Context) {
    let Some(imp) = app.job_import.as_mut() else { return };
    let (mut open, mut go, mut cancel, mut replan) = (true, false, false, false);
    let crews = &app.crews;
    let crew_name = |id: Option<i64>| id.and_then(|id| crews.iter().find(|c| c.id == id)).map(|c| c.name.clone()).unwrap_or_default();
    egui::Window::new("Import Jobs")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(760.0)
        .default_height(640.0)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            let p = &imp.plan;
            ui.label(RichText::new(&imp.file).weak());
            ui.add_space(2.0);
            ui.label(
                RichText::new(format!(
                    "{} new · {} to update · {} already up to date · {} row{} with errors (skipped)",
                    p.add.len(),
                    p.update.len(),
                    p.unchanged,
                    p.errors.iter().map(|e| e.line).collect::<HashSet<_>>().len(),
                    if p.errors.iter().map(|e| e.line).collect::<HashSet<_>>().len() == 1 { "" } else { "s" },
                ))
                .strong()
                .size(15.0),
            );
            if ui
                .checkbox(&mut imp.update_scheduling, "Update scheduling (crews, dates, times, hours, days) on jobs already in the app")
                .on_hover_text("Off: jobs a scheduler moved or crewed in the app keep their schedule. New jobs always get the file's schedule.")
                .changed()
            {
                replan = true;
            }
            if p.schedule_differs > 0 && !imp.update_scheduling {
                ui.colored_label(
                    WARN,
                    format!(
                        "{} existing job{} in the file {} a different crew or schedule than the app; left as they are.",
                        p.schedule_differs,
                        if p.schedule_differs == 1 { "" } else { "s" },
                        if p.schedule_differs == 1 { "has" } else { "have" }
                    ),
                );
            }
            if !p.ignored_columns.is_empty() {
                ui.weak(format!("Columns not used: {}", p.ignored_columns.join(", ")));
            }
            ui.separator();
            ScrollArea::vertical().max_height(430.0).auto_shrink([false, true]).show(ui, |ui| {
                if !p.errors.is_empty() {
                    CollapsingHeader::new(RichText::new(format!("Errors: these rows will be skipped ({})", p.errors.len())).color(ERR).strong())
                        .id_salt("ji_errors")
                        .default_open(true)
                        .show(ui, |ui| notes_grid(ui, "ji_err_grid", &p.errors, ERR));
                }
                if !p.warnings.is_empty() {
                    CollapsingHeader::new(RichText::new(format!("Warnings ({})", p.warnings.len())).color(WARN).strong())
                        .id_salt("ji_warnings")
                        .default_open(true)
                        .show(ui, |ui| notes_grid(ui, "ji_warn_grid", &p.warnings, WARN));
                }
                CollapsingHeader::new(RichText::new(format!("New jobs ({})", p.add.len())).strong())
                    .id_salt("ji_new")
                    .default_open(p.add.len() <= 40)
                    .show(ui, |ui| {
                        Grid::new("ji_new_grid").striped(true).spacing([14.0, 3.0]).show(ui, |ui| {
                            for h in ["Source ID", "Schedule", "Project", "Lot", "Phase", "Start", "Days", "Crew", "Status"] {
                                ui.label(RichText::new(h).strong());
                            }
                            ui.end_row();
                            for j in &p.add {
                                ui.label(&j.source_id);
                                ui.label(j.board.as_str());
                                ui.label(&j.project_code);
                                ui.label(&j.lot);
                                ui.label(&j.phase);
                                ui.label(match j.date {
                                    Some(d) => format!("{} {}", d.format("%b %-d"), fmt_time(j.start_min)),
                                    None => "Unscheduled".into(),
                                });
                                ui.label(j.days.to_string());
                                ui.label(crew_name(j.crew_id));
                                ui.label(&j.status);
                                ui.end_row();
                            }
                        });
                    });
                CollapsingHeader::new(RichText::new(format!("Jobs to update ({})", p.update.len())).strong())
                    .id_salt("ji_upd")
                    .default_open(p.update.len() <= 40)
                    .show(ui, |ui| {
                        Grid::new("ji_upd_grid").striped(true).spacing([14.0, 3.0]).show(ui, |ui| {
                            for h in ["Source ID", "Project", "Lot", "What changes"] {
                                ui.label(RichText::new(h).strong());
                            }
                            ui.end_row();
                            for (j, what) in &p.update {
                                ui.label(&j.source_id);
                                ui.label(&j.project_code);
                                ui.label(&j.lot);
                                ui.label(what.join(", "));
                                ui.end_row();
                            }
                        });
                    });
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let n = p.add.len() + p.update.len();
                let label = RichText::new(format!("Import {n} job{}", if n == 1 { "" } else { "s" })).strong();
                if ui.add_enabled(n > 0, Button::new(label).fill(Color32::from_rgb(210, 235, 215))).clicked() {
                    go = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
                ui.weak("The database is backed up before anything is saved.");
            });
        });

    if replan {
        let today = today();
        let imp = app.job_import.as_mut().unwrap();
        if let Ok(p) = plan(&imp.text, &app.jobs, &app.crews, imp.update_scheduling, today) {
            imp.plan = p;
        }
    }
    if go {
        let imp = app.job_import.take().unwrap();
        let user = app.user_id();
        match save(&mut app.db, &imp.plan, user) {
            Ok(backup_name) => {
                app.reload();
                let skipped = imp.plan.errors.iter().map(|e| e.line).collect::<HashSet<_>>().len();
                app.set_status(format!(
                    "Imported {}: {} new, {} updated{}.{}",
                    imp.file,
                    imp.plan.add.len(),
                    imp.plan.update.len(),
                    if skipped > 0 { format!(", {skipped} rows skipped") } else { String::new() },
                    backup_name.map(|b| format!(" Backup: {b}")).unwrap_or_default()
                ));
            }
            Err(e) => app.set_error(e),
        }
    } else if cancel || !open {
        app.job_import = None;
    }
}

// ------------------------------------------------------------ command line

/// `--import-jobs <file.csv> [--update-scheduling] [--apply]`. Returns the exit code.
pub fn run(mut db: Db, path: &str, update_scheduling: bool, apply: bool) -> i32 {
    let text = match std::fs::read(path) {
        Ok(b) => String::from_utf8_lossy(&b).to_string(),
        Err(e) => {
            eprintln!("Can't read {path}: {e}");
            return 1;
        }
    };
    let (jobs, crews) = match (db.load_jobs(), db.load_crews()) {
        (Ok(j), Ok(c)) => (j, c),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("Can't read the database: {e}");
            return 1;
        }
    };
    let p = match plan(&text, &jobs, &crews, update_scheduling, today()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    for j in &p.add {
        println!("  add     {:<12} {:<8} {} lot {}  {}", j.source_id, j.board.as_str(), j.project_code, j.lot, j.date.map(|d| d.to_string()).unwrap_or("unscheduled".into()));
    }
    for (j, what) in &p.update {
        println!("  update  {:<12} {} lot {}: {}", j.source_id, j.project_code, j.lot, what.join(", "));
    }
    for n in &p.warnings {
        println!("  WARNING line {} {}: {}", n.line, n.source_id, n.message);
    }
    for n in &p.errors {
        println!("  ERROR   line {} {}: {}", n.line, n.source_id, n.message);
    }
    println!(
        "\n{} new, {} to update, {} up to date, {} error(s), {} warning(s){}.",
        p.add.len(),
        p.update.len(),
        p.unchanged,
        p.errors.len(),
        p.warnings.len(),
        if p.schedule_differs > 0 && !update_scheduling {
            format!("; {} job(s) have a different schedule in the file (add --update-scheduling to apply it)", p.schedule_differs)
        } else {
            String::new()
        }
    );
    if !apply {
        println!("Nothing saved. Run again with --apply to save.");
        return 0;
    }
    if p.add.is_empty() && p.update.is_empty() {
        println!("Nothing to save.");
        return 0;
    }
    match save(&mut db, &p, None) {
        Ok(b) => {
            println!("Saved.{}", b.map(|b| format!(" Backup: {b}")).unwrap_or_default());
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn csv_quotes_commas_and_line_breaks() {
        let rows = parse_csv("\u{feff}A,B,C\r\n1,\"x, y\",\"he said \"\"hi\"\"\"\r\n2,\"two\nlines\",z\n\n3,,\n");
        assert_eq!(rows[0], (1, vec!["A".into(), "B".into(), "C".into()]));
        assert_eq!(rows[1], (2, vec!["1".into(), "x, y".into(), "he said \"hi\"".into()]));
        assert_eq!(rows[2], (3, vec!["2".into(), "two\nlines".into(), "z".into()]));
        assert_eq!(rows[3].1, vec![String::new()], "a blank line is an empty record");
        assert_eq!(rows[4], (6, vec!["3".into(), String::new(), String::new()]));
    }

    #[test]
    fn values() {
        assert_eq!(parse_time("7:00 AM"), Some(420));
        assert_eq!(parse_time("12:30 pm"), Some(750));
        assert_eq!(parse_time("12:00 AM"), Some(0));
        assert_eq!(parse_time("13:30"), Some(810));
        assert_eq!(parse_time("7 AM"), Some(420));
        assert_eq!(parse_time("7:08 AM"), Some(435), "rounded to 15 minutes");
        assert_eq!(parse_time("13:00 PM"), None);
        assert_eq!(parse_time("noon"), None);
        assert_eq!(parse_date("2026-11-20"), Some(d("2026-11-20")));
        assert_eq!(parse_date("11/20/2026"), Some(d("2026-11-20")));
        assert_eq!(parse_date("20/11/2026"), None);
        assert_eq!(parse_schedule("rough in"), Some(Board::RoughIn));
        assert_eq!(parse_schedule("TRIM"), Some(Board::Trim));
        assert_eq!(parse_status("in progress"), Some("In Progress"));
    }

    fn crews() -> Vec<Crew> {
        let mut a = Crew::new([0, 0, 0]);
        a.id = 1;
        a.name = "Zachary McBride".into();
        a.board = Board::RoughIn;
        let mut b = Crew::new([0, 0, 0]);
        b.id = 2;
        b.name = "Darrel Whitfield".into();
        b.board = Board::RoughIn;
        vec![a, b]
    }

    const EXAMPLE: &str = include_str!("../templates/Jobs Import Example.csv");

    #[test]
    fn the_example_file_imports() {
        let p = plan(EXAMPLE, &[], &crews(), false, d("2026-11-13")).unwrap();
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        assert!(p.warnings.is_empty(), "{:?}", p.warnings);
        assert_eq!(p.add.len(), 5);
        let j = &p.add[0];
        assert_eq!((j.source_id.as_str(), j.board, j.project_code.as_str(), j.lot.as_str()), ("JOB-10421", Board::RoughIn, "WIL-220", "41"));
        assert_eq!((j.crew_id, j.date, j.start_min, j.duration_min, j.days), (Some(1), Some(d("2026-11-18")), 420, 480, 2));
        assert_eq!(j.notes, "Stub-outs per plan sheet P2, verify tub location");
        assert_eq!(j.fields.iter().find(|f| f.label == "Permit #").unwrap().value, "BLD-26-0412");
        assert_eq!(j.fields.len(), 5);
        let sat = &p.add[2];
        assert_eq!((sat.days, sat.weekend.sat, sat.weekend.sun, sat.duration_min), (3, true, false, 600));
        let trim = &p.add[3];
        assert_eq!((trim.board, trim.tank_size.as_str(), trim.kitchen_sink.as_str(), trim.start_min), (Board::Trim, "50 gal", "Double Bowl", 750));
        let held = &p.add[4];
        assert_eq!((held.date, held.status.as_str(), held.days, held.duration_min), (None, "On Hold", 1, 480));
        assert!(held.fields.is_empty());
    }

    #[test]
    fn reimport_updates_and_protects_scheduling() {
        let mut db = Db::open_in_memory().unwrap();
        let cr = crews();
        let p = plan(EXAMPLE, &[], &cr, false, d("2026-11-13")).unwrap();
        // The test crews aren't in this database; drop crew links before saving.
        let mut p = p;
        for j in &mut p.add {
            j.crew_id = None;
        }
        db.save_jobs(&p.add, None).unwrap();
        let jobs = db.load_jobs().unwrap();
        assert_eq!(jobs.len(), 5);

        // Same file again: nothing to do (crew differs only because we dropped it).
        let again = plan(EXAMPLE, &jobs, &[], false, d("2026-11-13")).unwrap();
        assert_eq!((again.add.len(), again.update.len(), again.unchanged), (0, 0, 5));

        // A scheduler moved JOB-10422 in the app; the source changes its notes and date.
        let mut moved = jobs.iter().find(|j| j.source_id == "JOB-10422").unwrap().clone();
        moved.date = Some(d("2026-11-24"));
        db.save_job(&moved, None).unwrap();
        let jobs = db.load_jobs().unwrap();
        let csv = "Source ID,Schedule,Project Code,Notes,Start Date,Lot,Field: Permit #\n\
                   JOB-10422,Rough-In,WIL-220,Locate ticket #8812 called in,2026-11-21,,CLEAR\n\
                   JOB-10433,Trim,OAK-101,,,CLEAR,\n";
        let p = plan(csv, &jobs, &[], false, d("2026-11-13")).unwrap();
        assert_eq!(p.update.len(), 2);
        assert_eq!(p.schedule_differs, 1, "the file's date isn't applied");
        let (j, what) = &p.update[0];
        assert_eq!(j.notes, "Locate ticket #8812 called in");
        assert_eq!(j.date, Some(d("2026-11-24")), "the scheduler's move is kept");
        assert_eq!(j.lot, "42", "blank leaves it alone");
        assert!(j.fields.iter().all(|f| f.label != "Permit #"), "CLEAR removes the field");
        assert_eq!(what, &vec!["Notes".to_string(), "Additional Fields".to_string()]);
        assert_eq!(p.update[1].0.lot, "", "CLEAR erases");

        // With "update scheduling" on, the file's date wins.
        let p = plan(csv, &jobs, &[], true, d("2026-11-13")).unwrap();
        assert_eq!(p.update[0].0.date, Some(d("2026-11-21")));
        assert_eq!(p.schedule_differs, 0);
    }

    #[test]
    fn errors_skip_rows_and_warnings_dont() {
        let csv = "Source ID,Schedule,Project Code,Status,Start Date,Start Time,Hours Per Day,Days,Include Saturday,Crew,Mystery\n\
                   ,Rough-In,A,,,,,,,,\n\
                   J1,Plumbing,A,,,,,,,,\n\
                   J2,Trim,,Done,2026-13-01,25:00,0,99,maybe,,\n\
                   J3,Rough-In,C,,2026-11-20,,,,,Nobody,\n\
                   J3,Rough-In,C,,,,,,,,\n\
                   J4,Trim,D,,2029-01-01,,,,,Zachary McBride,x\n";
        let p = plan(csv, &[], &crews(), false, d("2026-11-13")).unwrap();
        let lines: HashSet<usize> = p.errors.iter().map(|e| e.line).collect();
        assert_eq!(lines, [2, 3, 4, 6].into_iter().collect());
        assert_eq!(p.errors.iter().filter(|e| e.line == 4).count(), 7, "{:#?}", p.errors);
        assert_eq!(p.add.iter().map(|j| j.source_id.as_str()).collect::<Vec<_>>(), ["J3", "J4"]);
        assert_eq!(p.add[0].crew_id, None);
        assert_eq!(p.warnings.len(), 3, "{:#?}", p.warnings); // unknown crew; RI crew on Trim job; far-off date
        assert_eq!(p.ignored_columns, vec!["Mystery".to_string()]);
    }

    #[test]
    fn header_must_have_the_key_columns() {
        assert!(plan("Project Code,Lot\nA,1\n", &[], &[], false, d("2026-11-13")).unwrap_err().contains("Source ID"));
        assert!(plan("", &[], &[], false, d("2026-11-13")).is_err());
    }

    #[test]
    fn import_backs_up_and_saves() {
        let dir = std::env::temp_dir().join(format!("pjs-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("PLUMBING_SCHEDULER_DB", dir.join("t.db"));
        let (mut db, warn) = Db::open_default();
        std::env::remove_var("PLUMBING_SCHEDULER_DB");
        assert!(warn.is_none());
        let p = plan(EXAMPLE, &[], &[], false, d("2026-11-13")).unwrap();
        let name = save(&mut db, &p, None).unwrap().expect("a backup file");
        assert!(dir.join(&name).exists());
        assert_eq!(db.load_jobs().unwrap().len(), 5);
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
