//! Month and week calendar views, the unscheduled-jobs panel, and the
//! drag-and-drop machinery that ties them together.
//!
//! Drag and drop is done by hand rather than with egui's dnd helpers so a drop
//! can resolve to a precise date *and* time: every view registers drop target
//! rectangles each frame, and `finish_drag` (run last in the frame) paints the
//! drag ghost and applies the move when the mouse is released.

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, Duration, Months, NaiveDate};
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CursorIcon, FontId, Id, Key, LayerId, Order, Painter, Rect,
    Response, RichText, ScrollArea, Sense, Stroke, Ui, Vec2,
};

use crate::app::{today, CrewFilter, SchedulerApp, ViewMode};
use crate::model::*;

pub const DAY_START_MIN: i32 = 5 * 60;
pub const DAY_END_MIN: i32 = 20 * 60;
const SNAP_MIN: i32 = 15;
const WEEKDAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

#[derive(Clone, Debug)]
pub struct DragState {
    pub job_id: i64,
    /// Pointer position relative to the card's top-left when the drag began.
    pub grab_offset: Vec2,
    pub size: Vec2,
    /// Which day of a multi-day job was grabbed (0 = its start day). The job
    /// moves so that this day lands where it's dropped.
    pub day_index: usize,
}

#[derive(Clone, Debug)]
pub enum DropTarget {
    /// Month cell or week header: change the date, keep the time.
    Day(NaiveDate),
    /// Week-view time column: change date and time.
    Slot { date: NaiveDate, grid_top: f32, px_per_min: f32, col: Rect },
    /// Unscheduled side panel.
    Unscheduled,
    /// Crew timeline cell (time runs left to right): change date, time and crew.
    CrewSlot { crew_id: Option<i64>, date: NaiveDate, x0: f32, px_per_min: f32, cell: Rect },
}

enum Action {
    Open(i64),
    New(Option<NaiveDate>, Option<i32>),
    NewForCrew(NaiveDate, i32, Option<i64>),
    StartDrag(DragState),
    GotoWeek(NaiveDate),
    GotoDay(NaiveDate),
    SelectCrewDay(Option<NaiveDate>),
}

#[derive(Clone, Copy, PartialEq)]
enum CardStyle {
    /// One line, used in month cells.
    Compact,
    /// Multi-line block sized by duration, used in week view.
    Block,
    /// Multi-line, no time, used in the unscheduled panel.
    Unscheduled,
    /// Two short lines (time, lot) for narrow crew-timeline bars.
    Mini,
}

fn apply_actions(app: &mut SchedulerApp, actions: Vec<Action>) {
    for a in actions {
        match a {
            Action::Open(id) => {
                if app.drag.is_none() {
                    app.open_job(id);
                }
            }
            Action::New(d, t) => app.open_new_job(d, t),
            Action::NewForCrew(d, t, crew) => {
                if !app.require_edit() {
                    continue;
                }
                app.open_new_job(Some(d), Some(t));
                if let Some(ed) = app.editor.as_mut() {
                    ed.job.crew_id = crew;
                }
            }
            Action::StartDrag(d) => {
                // View-only accounts can look but not move jobs.
                if app.can_edit() {
                    app.drag = Some(d);
                }
            }
            Action::GotoWeek(d) => {
                app.focus = d;
                app.view = ViewMode::Week;
            }
            Action::GotoDay(d) => {
                app.focus = d;
                app.view = ViewMode::Day;
            }
            Action::SelectCrewDay(d) => {
                app.crew_view_day = d;
                if let Some(d) = d {
                    app.focus = d;
                }
            }
        }
    }
}

pub fn week_start(d: NaiveDate) -> NaiveDate {
    d - Duration::days(d.weekday().num_days_from_sunday() as i64)
}

/// Ids of jobs whose crew is double-booked (overlapping times on the same day).
pub fn conflicts(jobs: &[Job]) -> HashSet<i64> {
    let mut groups: HashMap<(NaiveDate, i64), Vec<&Job>> = HashMap::new();
    // Multi-day jobs count on every day they run.
    for j in jobs {
        if let Some(c) = j.crew_id {
            if !j.is_cancelled() {
                for d in j.work_dates() {
                    groups.entry((d, c)).or_default().push(j);
                }
            }
        }
    }
    let mut out = HashSet::new();
    for list in groups.values_mut() {
        list.sort_by_key(|j| j.start_min);
        for i in 0..list.len() {
            for k in i + 1..list.len() {
                if list[k].start_min < list[i].end_min() {
                    out.insert(list[i].id);
                    out.insert(list[k].id);
                } else {
                    break;
                }
            }
        }
    }
    out
}

/// Side-by-side layout for overlapping blocks. Input sorted by start.
/// Returns (lane, lanes in this overlap cluster) per item.
fn layout_lanes(spans: &[(i32, i32)]) -> Vec<(usize, usize)> {
    let mut result = vec![(0usize, 1usize); spans.len()];
    let mut cluster_start = 0;
    let mut cluster_end = i32::MIN;
    let mut lane_ends: Vec<i32> = Vec::new();
    for (i, &(s, e)) in spans.iter().enumerate() {
        if i > 0 && s >= cluster_end {
            for r in &mut result[cluster_start..i] {
                r.1 = lane_ends.len();
            }
            lane_ends.clear();
            cluster_start = i;
        }
        let lane = match lane_ends.iter().position(|&le| le <= s) {
            Some(l) => l,
            None => {
                lane_ends.push(i32::MIN);
                lane_ends.len() - 1
            }
        };
        lane_ends[lane] = e;
        result[i].0 = lane;
        cluster_end = cluster_end.max(e);
    }
    let n = lane_ends.len().max(1);
    for r in &mut result[cluster_start..] {
        r.1 = n;
    }
    result
}

fn snap(min: i32) -> i32 {
    ((min as f32 / SNAP_MIN as f32).round() as i32) * SNAP_MIN
}

// ------------------------------------------------------------------ cards

#[allow(clippy::too_many_arguments)]
fn paint_card(
    painter: &Painter,
    rect: Rect,
    job: &Job,
    color: Color32,
    crew_name: &str,
    conflict: bool,
    placeholder: bool,
    style: CardStyle,
    // (day number from 1, total days) for a multi-day job.
    day_tag: Option<(usize, usize)>,
    // Allocated man-hours (Rough-In jobs in Week/Day view), shown when it fits.
    alloc: Option<f32>,
) {
    let painter = painter.with_clip_rect(rect.expand(1.0).intersect(painter.clip_rect()));
    let alloc_text = alloc.map(|h| format!("{} hrs allocated", fmt_hours(h)));
    let short_tag = day_tag.map(|(i, n)| format!("{i}/{n} ")).unwrap_or_default();
    let dim = job.is_complete() || job.is_cancelled();
    let mut fill = if dim { mix(color, Color32::from_gray(200), 0.55) } else { color };
    if placeholder {
        fill = fill.gamma_multiply(0.3);
    }
    painter.rect_filled(rect, 4.0, fill);
    // Darker left accent strip in the crew color.
    painter.rect_filled(
        Rect::from_min_size(rect.min, vec2(4.0, rect.height())),
        egui::Rounding { nw: 4.0, sw: 4.0, ne: 0.0, se: 0.0 },
        mix(color, Color32::BLACK, 0.3),
    );
    if conflict {
        painter.rect_stroke(rect.shrink(1.0), 4.0, Stroke::new(2.0_f32, Color32::from_rgb(230, 30, 30)));
    }
    if placeholder {
        return;
    }
    let tc = text_color_for(fill);
    let warn = if conflict { "⚠ " } else { "" };
    let x = rect.left() + 8.0;

    match style {
        CardStyle::Compact => {
            let mut text = format!(
                "{warn}{short_tag}{}{} {}",
                job.status_icon(),
                fmt_time_compact(job.start_min),
                job.short_title()
            );
            // On a short block, add the hours at the end if there's room.
            if let Some(h) = alloc {
                let more = format!("{text} · {} hrs", fmt_hours(h));
                let w = painter.layout_no_wrap(more.clone(), FontId::proportional(12.5), tc).size().x;
                if w <= rect.width() - 12.0 {
                    text = more;
                }
            }
            painter.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, text, FontId::proportional(12.5), tc);
        }
        CardStyle::Mini => {
            let time = format!(
                "{warn}{short_tag}{}{}–{}",
                job.status_icon(),
                fmt_time_compact(job.start_min),
                fmt_time_compact(job.end_min())
            );
            let lot = job.lot_label();
            let what = if lot.is_empty() { job.short_title() } else { lot };
            painter.text(pos2(x, rect.top() + 3.0), Align2::LEFT_TOP, time, FontId::proportional(10.5), tc);
            painter.text(pos2(x, rect.top() + 17.0), Align2::LEFT_TOP, what, FontId::proportional(12.5), tc);
        }
        CardStyle::Block | CardStyle::Unscheduled => {
            let mut lines: Vec<(String, f32)> = Vec::new();
            if style == CardStyle::Block {
                let day = day_tag.map(|(i, n)| format!(" · Day {i} of {n}")).unwrap_or_default();
                lines.push((
                    format!("{warn}{}{} – {}{day}", job.status_icon(), fmt_time(job.start_min), fmt_time(job.end_min())),
                    11.0,
                ));
                lines.push((job.short_title(), 13.0));
                if let Some(t) = &alloc_text {
                    lines.push((t.clone(), 11.0));
                }
            } else {
                let days = if job.days > 1 { format!(" · {} days", job.days) } else { String::new() };
                lines.push((format!("{warn}{}{}{days}", job.status_icon(), job.short_title()), 13.0));
            }
            if !job.project_name.trim().is_empty() && !job.project_code.trim().is_empty() {
                lines.push((job.project_name.clone(), 11.0));
            }
            let mut detail = job.phase.clone();
            if !crew_name.is_empty() {
                if !detail.is_empty() {
                    detail.push_str(" · ");
                }
                detail.push_str(crew_name);
            }
            lines.push((detail, 11.0));

            let mut y = rect.top() + 3.0;
            for (text, size) in lines {
                if text.is_empty() {
                    continue;
                }
                if y + size > rect.bottom() + 2.0 {
                    break;
                }
                painter.text(pos2(x, y), Align2::LEFT_TOP, text, FontId::proportional(size), tc);
                y += size + 3.0;
            }
            if style == CardStyle::Block {
                paint_block_details(&painter, job, x, y, rect, tc);
            }
        }
    }
}

/// Under a time block's header: Notes, Comments and each additional field,
/// wrapped to the block's width. Whatever doesn't fit is cut off with "…";
/// a "…" in the corner means whole sections were left out. (The hover
/// tooltip always has everything.)
fn paint_block_details(painter: &Painter, job: &Job, x: f32, mut y: f32, rect: Rect, color: Color32) {
    let mut sections: Vec<String> = Vec::new();
    let notes = job.notes.trim();
    if !notes.is_empty() {
        sections.push(format!("Notes: {notes}"));
    }
    let comments = job.comments.trim();
    if !comments.is_empty() {
        sections.push(format!("Comments: {comments}"));
    }
    for f in &job.fields {
        let (label, value) = (f.label.trim(), f.value.trim());
        if !value.is_empty() {
            sections.push(if label.is_empty() { value.to_string() } else { format!("{label}: {value}") });
        }
    }
    if sections.is_empty() {
        return;
    }

    let font = FontId::proportional(11.0);
    let row_h = painter.ctx().fonts(|f| f.row_height(&font));
    let max_width = (rect.right() - x - 4.0).max(10.0);
    let bottom = rect.bottom() - 3.0;
    y += 2.0; // small gap below the header lines
    let mut left_out = false;
    for text in &sections {
        let rows = ((bottom - y) / row_h).floor();
        if rows < 1.0 {
            left_out = true;
            break;
        }
        let mut layout = egui::text::LayoutJob::single_section(
            text.clone(),
            egui::TextFormat { font_id: font.clone(), color, ..Default::default() },
        );
        layout.wrap = egui::text::TextWrapping {
            max_width,
            max_rows: rows as usize,
            break_anywhere: false,
            overflow_character: Some('…'),
        };
        let galley = painter.layout_job(layout);
        let height = galley.size().y;
        painter.galley(pos2(x, y), galley, color);
        y += height + 2.0;
    }
    if left_out {
        painter.text(
            pos2(rect.right() - 5.0, rect.bottom() - 1.0),
            Align2::RIGHT_BOTTOM,
            "…",
            FontId::proportional(14.0),
            color,
        );
    }
}

fn card_tooltip(ui: &mut Ui, job: &Job, crew_name: &str, conflict: bool) {
    ui.label(RichText::new(job.short_title()).strong().size(14.0));
    if !job.project_name.is_empty() {
        ui.label(&job.project_name);
    }
    match job.date {
        Some(d) => ui.label(format!(
            "{}  {} – {}  ({})",
            d.format("%a %b %-d, %Y"),
            fmt_time(job.start_min),
            fmt_time(job.end_min()),
            fmt_duration(job.duration_min)
        )),
        None => ui.label("Unscheduled"),
    };
    if job.days > 1 {
        match (job.date, job.last_date()) {
            (Some(_), Some(last)) => {
                ui.label(format!("{} days, through {} ({})", job.days, last.format("%a %b %-d"), job.weekend.describe()))
            }
            _ => ui.label(format!("{} work days", job.days)),
        };
    }
    ui.label(format!("Crew: {crew_name}"));
    if !job.phase.is_empty() {
        ui.label(format!("Phase: {}", job.phase));
    }
    if job.board == Board::Trim && !job.tank_size.trim().is_empty() {
        ui.label(format!("Water tank: {}", job.tank_size));
    }
    if job.board == Board::Trim && !job.kitchen_sink.trim().is_empty() {
        ui.label(format!("Kitchen sink: {}", job.kitchen_sink));
    }
    ui.label(format!("Status: {}", job.status));
    if conflict {
        ui.colored_label(Color32::from_rgb(230, 30, 30), "⚠ This crew is double-booked at this time");
    }
    if !job.notes.trim().is_empty() {
        let mut n: String = job.notes.chars().take(200).collect();
        if job.notes.chars().count() > 200 {
            n.push('…');
        }
        ui.label(format!("Notes: {n}"));
    }
    for f in job.fields.iter().filter(|f| !f.value.trim().is_empty()) {
        ui.label(format!("{}: {}", f.label, f.value));
    }
    ui.add_space(2.0);
    ui.weak("Click to edit · drag to move");
}

#[allow(clippy::too_many_arguments)]
fn card_widget(
    ui: &Ui,
    painter: &Painter,
    rect: Rect,
    job: &Job,
    app: &SchedulerApp,
    conflict: bool,
    style: CardStyle,
    // The day this card stands for (None = unscheduled panel).
    date: Option<NaiveDate>,
    alloc: Option<f32>,
    actions: &mut Vec<Action>,
) {
    // A multi-day job has a card on each day it runs, so the id includes the day.
    let resp: Response = ui.interact(rect, Id::new(("job_card", job.id, date)), Sense::click_and_drag());
    let color = app.job_color(job);
    let crew_name = app.crew_name(job);
    let dragging = app.drag.as_ref().map(|d| d.job_id);
    let day_index = date.and_then(|d| job.day_index(d)).unwrap_or(0);
    let day_tag = (job.days > 1 && date.is_some()).then(|| (day_index + 1, job.days as usize));
    paint_card(painter, rect, job, color, &crew_name, conflict, dragging == Some(job.id), style, day_tag, alloc);

    if resp.drag_started() {
        let origin = resp.ctx.input(|i| i.pointer.press_origin()).unwrap_or(rect.min);
        actions.push(Action::StartDrag(DragState {
            job_id: job.id,
            grab_offset: (origin - rect.min).max(Vec2::ZERO).min(rect.size()),
            size: rect.size(),
            day_index,
        }));
    } else if resp.clicked() {
        actions.push(Action::Open(job.id));
    }
    if dragging.is_none() {
        let cursor = if app.can_edit() { CursorIcon::Grab } else { CursorIcon::PointingHand };
        resp.on_hover_cursor(cursor)
            .on_hover_ui(|ui| {
                card_tooltip(ui, job, &crew_name, conflict);
                if job.board == Board::RoughIn && job.date.is_some() {
                    ui.label(format!("Allocated hours: {}", fmt_hours(app.job_allocated_hours(job))));
                }
            });
    }
}

// -------------------------------------------------------------- month view

pub fn month_view(app: &mut SchedulerApp, ui: &mut Ui) {
    let first = app.focus.with_day(1).unwrap_or(app.focus);
    let grid_start = week_start(first);
    let next_month = first.checked_add_months(Months::new(1)).unwrap_or(first);
    let days = (next_month - grid_start).num_days();
    let rows = ((days + 6) / 7) as usize;
    let today = today();

    let rect = ui.available_rect_before_wrap();
    ui.allocate_rect(rect, Sense::hover());
    let header_h = 24.0;
    let cell_w = rect.width() / 7.0;
    let cell_h = ((rect.height() - header_h) / rows as f32).max(48.0);
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals().clone();
    let grid_stroke = Stroke::new(1.0_f32, visuals.widgets.noninteractive.bg_stroke.color);

    for (i, name) in WEEKDAYS.iter().enumerate() {
        let x = rect.left() + cell_w * (i as f32 + 0.5);
        let label = if cell_w < 120.0 { &name[..3] } else { name };
        painter.text(
            pos2(x, rect.top() + header_h / 2.0),
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(13.0),
            visuals.text_color(),
        );
    }

    let conflicts = conflicts(&app.jobs);
    let mut by_date: HashMap<NaiveDate, Vec<&Job>> = HashMap::new();
    for j in &app.jobs {
        if !app.job_visible(j) {
            continue;
        }
        // Start date only, or every day it runs.
        let dates = if app.show_start_only { j.date.into_iter().collect() } else { j.work_dates() };
        for d in dates {
            by_date.entry(d).or_default().push(j);
        }
    }
    for v in by_date.values_mut() {
        v.sort_by_key(|j| (j.start_min, j.id));
    }

    let mut targets = Vec::new();
    let mut actions = Vec::new();
    let card_h = 20.0;
    let gap = 2.0;

    for r in 0..rows {
        for c in 0..7 {
            let date = grid_start + Duration::days((r * 7 + c) as i64);
            let cell = Rect::from_min_size(
                pos2(rect.left() + c as f32 * cell_w, rect.top() + header_h + r as f32 * cell_h),
                vec2(cell_w, cell_h),
            );
            let in_month = date.month() == first.month();
            let weekend = c == 0 || c == 6;
            let bg = if !in_month {
                mix(visuals.panel_fill, visuals.weak_text_color(), 0.12)
            } else if weekend {
                mix(visuals.extreme_bg_color, visuals.weak_text_color(), 0.05)
            } else {
                visuals.extreme_bg_color
            };
            painter.rect_filled(cell, 0.0, bg);
            painter.rect_stroke(cell, 0.0, grid_stroke);
            targets.push((cell, DropTarget::Day(date)));

            let cell_resp = ui.interact(cell, Id::new(("month_cell", date)), Sense::click());
            if cell_resp.double_clicked() {
                actions.push(Action::New(Some(date), None));
            }

            // Day number (click = open that week).
            let num_center = pos2(cell.left() + 15.0, cell.top() + 13.0);
            let num_color = if in_month { visuals.text_color() } else { visuals.weak_text_color() };
            if date == today {
                painter.circle_filled(num_center, 11.0, visuals.selection.bg_fill);
                painter.text(num_center, Align2::CENTER_CENTER, date.day().to_string(), FontId::proportional(13.0), Color32::WHITE);
            } else {
                painter.text(num_center, Align2::CENTER_CENTER, date.day().to_string(), FontId::proportional(13.0), num_color);
            }
            let num_resp = ui.interact(
                Rect::from_center_size(num_center, vec2(26.0, 22.0)),
                Id::new(("month_daynum", date)),
                Sense::click(),
            );
            if num_resp.clicked() {
                actions.push(Action::GotoWeek(date));
            }
            num_resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text("Open this week");

            let list: &[&Job] = by_date.get(&date).map(|v| v.as_slice()).unwrap_or(&[]);
            if !list.is_empty() {
                painter.text(
                    pos2(cell.right() - 6.0, cell.top() + 13.0),
                    Align2::RIGHT_CENTER,
                    format!("{} job{}", list.len(), if list.len() == 1 { "" } else { "s" }),
                    FontId::proportional(11.0),
                    visuals.weak_text_color(),
                );
            }

            let top = cell.top() + 27.0;
            let avail = (cell.bottom() - 3.0 - top).max(0.0);
            let capacity = ((avail + gap) / (card_h + gap)).floor() as usize;
            let show = if list.len() > capacity { capacity.saturating_sub(1) } else { list.len() };
            for (k, job) in list.iter().take(show).enumerate() {
                let card = Rect::from_min_size(
                    pos2(cell.left() + 3.0, top + k as f32 * (card_h + gap)),
                    vec2(cell_w - 6.0, card_h),
                );
                card_widget(ui, &painter, card, job, app, conflicts.contains(&job.id), CardStyle::Compact, Some(date), None, &mut actions);
            }
            if list.len() > show {
                let more = Rect::from_min_size(
                    pos2(cell.left() + 3.0, top + show as f32 * (card_h + gap)),
                    vec2(cell_w - 6.0, card_h),
                );
                let resp = ui.interact(more, Id::new(("month_more", date)), Sense::click());
                painter.text(
                    pos2(more.left() + 6.0, more.center().y),
                    Align2::LEFT_CENTER,
                    format!("+{} more…", list.len() - show),
                    FontId::proportional(12.0),
                    visuals.hyperlink_color,
                );
                if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    actions.push(Action::GotoWeek(date));
                }
            }
        }
    }

    app.drop_targets.extend(targets);
    apply_actions(app, actions);
}

// --------------------------------------------------------------- week view

pub fn week_view(app: &mut SchedulerApp, ui: &mut Ui) {
    let start = week_start(app.focus);
    let days: Vec<NaiveDate> = (0..7).map(|i| start + Duration::days(i)).collect();
    time_grid(app, ui, &days);
}

/// One day, full width, by the hour.
pub fn day_view(app: &mut SchedulerApp, ui: &mut Ui) {
    let day = app.focus;
    time_grid(app, ui, &[day]);
}

/// Days side by side with time running down (Week = 7 columns, Day = 1).
fn time_grid(app: &mut SchedulerApp, ui: &mut Ui, days: &[NaiveDate]) {
    let today = today();
    let visuals = ui.visuals().clone();
    let conflicts = conflicts(&app.jobs);
    let grid_stroke = Stroke::new(1.0_f32, visuals.widgets.noninteractive.bg_stroke.color);
    let faint_stroke = Stroke::new(1.0_f32, visuals.widgets.noninteractive.bg_stroke.color.gamma_multiply(0.4));
    let single = days.len() == 1;
    // Allocated hours for the Rough-In jobs shown (cards include it when it fits).
    let alloc = app.allocations_on(days);

    let gutter = 60.0;
    let header_h = 44.0;
    let px_per_min = 1.0_f32; // 60 px per hour
    let full = ui.available_rect_before_wrap();
    let n_days = days.len() as f32;
    let col_w = ((full.width() - gutter - 14.0) / n_days).max(70.0);

    let mut targets = Vec::new();
    let mut actions = Vec::new();

    // Day headers.
    let (hrect, _) = ui.allocate_exact_size(vec2(full.width(), header_h), Sense::hover());
    let hpainter = ui.painter_at(hrect);
    for (d, date) in days.iter().copied().enumerate() {
        let r = Rect::from_min_size(pos2(hrect.left() + gutter + d as f32 * col_w, hrect.top()), vec2(col_w, header_h));
        if date == today {
            hpainter.rect_filled(r.shrink(2.0), 4.0, visuals.selection.bg_fill.gamma_multiply(0.25));
        }
        let weekday = WEEKDAYS[date.weekday().num_days_from_sunday() as usize];
        let (top, bottom) = if single {
            (weekday.to_string(), date.format("%B %-d, %Y").to_string())
        } else {
            (weekday[..3].to_string(), date.format("%b %-d").to_string())
        };
        hpainter.text(r.center_top() + vec2(0.0, 5.0), Align2::CENTER_TOP, top, FontId::proportional(12.0), visuals.weak_text_color());
        hpainter.text(r.center_top() + vec2(0.0, 21.0), Align2::CENTER_TOP, bottom, FontId::proportional(15.0), visuals.text_color());
        targets.push((r, DropTarget::Day(date)));
        let resp = ui.interact(r, Id::new(("week_hdr", date)), Sense::click());
        let hint = if single { "Double-click to add a job" } else { "Click to open this day · double-click to add a job" };
        let resp = resp.on_hover_text(hint);
        if resp.double_clicked() {
            actions.push(Action::New(Some(date), None));
        } else if resp.clicked() && !single {
            actions.push(Action::GotoDay(date));
        }
    }

    ScrollArea::vertical()
        .id_salt("week_scroll")
        .auto_shrink([false, false])
        .drag_to_scroll(false)
        .show(ui, |ui| {
            let total_h = (DAY_END_MIN - DAY_START_MIN) as f32 * px_per_min;
            let (grid, _) = ui.allocate_exact_size(vec2(gutter + col_w * n_days, total_h), Sense::hover());
            let clip = ui.clip_rect();
            let painter = ui.painter_at(grid);

            for (d, date) in days.iter().copied().enumerate() {
                let col = Rect::from_min_size(pos2(grid.left() + gutter + d as f32 * col_w, grid.top()), vec2(col_w, total_h));
                let weekend = matches!(date.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun);
                let bg = if date == today {
                    mix(visuals.extreme_bg_color, visuals.selection.bg_fill, 0.08)
                } else if weekend {
                    mix(visuals.extreme_bg_color, visuals.panel_fill, 0.6)
                } else {
                    visuals.extreme_bg_color
                };
                painter.rect_filled(col, 0.0, bg);
            }

            for h in (DAY_START_MIN / 60)..=(DAY_END_MIN / 60) {
                let y = grid.top() + (h * 60 - DAY_START_MIN) as f32 * px_per_min;
                painter.hline(grid.left() + gutter..=grid.right(), y, grid_stroke);
                if h * 60 + 30 < DAY_END_MIN {
                    painter.hline(grid.left() + gutter..=grid.right(), y + 30.0 * px_per_min, faint_stroke);
                }
                painter.text(
                    pos2(grid.left() + gutter - 8.0, y + 2.0),
                    Align2::RIGHT_TOP,
                    fmt_time(h * 60).replace(":00", ""),
                    FontId::proportional(11.0),
                    visuals.weak_text_color(),
                );
            }

            for (d, date) in days.iter().copied().enumerate() {
                let col = Rect::from_min_size(pos2(grid.left() + gutter + d as f32 * col_w, grid.top()), vec2(col_w, total_h));
                painter.vline(col.left(), grid.top()..=grid.bottom(), grid_stroke);
                if single {
                    painter.vline(col.right(), grid.top()..=grid.bottom(), grid_stroke);
                }

                let visible = col.intersect(clip);
                if visible.is_positive() {
                    targets.push((visible, DropTarget::Slot { date, grid_top: grid.top(), px_per_min, col }));
                }
                let bg = ui.interact(col, Id::new(("week_col", date)), Sense::click());
                if bg.double_clicked() {
                    if let Some(p) = bg.interact_pointer_pos() {
                        let m = DAY_START_MIN + ((p.y - grid.top()) / px_per_min) as i32;
                        let m = (m / 30) * 30;
                        actions.push(Action::New(Some(date), Some(m.clamp(DAY_START_MIN, DAY_END_MIN - 30))));
                    }
                }

                let mut list: Vec<&Job> = app
                    .jobs
                    .iter()
                    .filter(|j| app.job_on(j, date) && app.job_visible(j))
                    .collect();
                list.sort_by_key(|j| (j.start_min, j.id));
                let spans: Vec<(i32, i32)> = list
                    .iter()
                    .map(|j| (j.start_min, j.end_min().max(j.start_min + SNAP_MIN)))
                    .collect();
                let lanes = layout_lanes(&spans);
                for (k, job) in list.iter().enumerate() {
                    let (lane, n) = lanes[k];
                    let lane_w = (col_w - 6.0) / n as f32;
                    let s = job.start_min.clamp(DAY_START_MIN, DAY_END_MIN - SNAP_MIN);
                    let e = job.end_min().clamp(s + SNAP_MIN, DAY_END_MIN);
                    let r = Rect::from_min_max(
                        pos2(col.left() + 3.0 + lane as f32 * lane_w, grid.top() + (s - DAY_START_MIN) as f32 * px_per_min + 1.0),
                        pos2(col.left() + 3.0 + (lane + 1) as f32 * lane_w - 2.0, grid.top() + (e - DAY_START_MIN) as f32 * px_per_min - 1.0),
                    );
                    let style = if r.height() < 34.0 { CardStyle::Compact } else { CardStyle::Block };
                    let a = alloc.get(&job.id).copied();
                    card_widget(ui, &painter, r, job, app, conflicts.contains(&job.id), style, Some(date), a, &mut actions);
                }
            }
        });

    app.drop_targets.extend(targets);
    apply_actions(app, actions);
}

// ----------------------------------------------------------- crew timeline

struct CrewRow {
    crew_id: Option<i64>,
    name: String,
    color: Color32,
    subtitle: String,
}

/// "Crew Week" view: one row per crew, time running left to right by the
/// hour. Shows the whole week (a column per day) or a single day full-width.
/// Dragging a job onto another crew's row reassigns it.
pub fn crew_view(app: &mut SchedulerApp, ui: &mut Ui) {
    let start = week_start(app.focus);
    let today = today();
    let visuals = ui.visuals().clone();
    let conflicts = conflicts(&app.jobs);
    let board = app.board();
    let other_board = board.other();
    let grid_stroke = Stroke::new(1.0_f32, visuals.widgets.noninteractive.bg_stroke.color);
    let faint_stroke = Stroke::new(1.0_f32, visuals.widgets.noninteractive.bg_stroke.color.gamma_multiply(0.45));
    let mut actions = Vec::new();
    let mut targets = Vec::new();

    let day_sel = app.crew_view_day.filter(|d| week_start(*d) == start);

    // Day picker.
    ui.horizontal(|ui| {
        ui.label("Show:");
        if ui.selectable_label(day_sel.is_none(), "Whole week").clicked() {
            actions.push(Action::SelectCrewDay(None));
        }
        ui.separator();
        for i in 0..7 {
            let date = start + Duration::days(i);
            let mut text = RichText::new(date.format("%a %-d").to_string());
            if date == today {
                text = text.strong();
            }
            if ui
                .selectable_label(day_sel == Some(date), text)
                .on_hover_text("Show this day by the hour")
                .clicked()
            {
                actions.push(Action::SelectCrewDay(Some(date)));
            }
        }
        ui.separator();
        ui.checkbox(&mut app.crew_view_all_boards, format!("Include {} jobs", other_board.as_str()))
            .on_hover_text("Crews can work both schedules. Jobs from the other schedule are shown faded.");
    });
    ui.add_space(4.0);

    let days: Vec<NaiveDate> = match day_sel {
        Some(d) => vec![d],
        None => (0..7).map(|i| start + Duration::days(i)).collect(),
    };
    let day_mode = day_sel.is_some();
    let week_end = start + Duration::days(6);
    // Rough-In crews change day to day: show who's on each crew on the day
    // being viewed (or today / the week's Monday in the whole-week view).
    let people_day = day_sel.unwrap_or(if today >= start && today <= week_end { today } else { start + Duration::days(1) });
    let crew_ids: Vec<i64> = app.crews.iter().map(|c| c.id).collect();
    let people: HashMap<i64, Vec<String>> = crew_ids.into_iter().map(|id| (id, app.crew_names_on(id, people_day))).collect();
    let show_job =|j: &Job| app.job_passes_filters(j) && (app.crew_view_all_boards || j.board == board);

    // Rows: active crews (or any crew with work this week), then Unassigned.
    let mut rows: Vec<CrewRow> = Vec::new();
    for c in &app.crews {
        let wanted = match app.crew_filter {
            CrewFilter::All => true,
            CrewFilter::Crew(id) => id == c.id,
            CrewFilter::Unassigned => false,
        };
        let busy = app
            .jobs
            .iter()
            .any(|j| j.crew_id == Some(c.id) && j.date.is_some_and(|d| d >= start && d <= week_end));
        // Crews from this schedule's staff group, plus any other crew that has
        // jobs on this schedule this week.
        let busy_here = app.jobs.iter().any(|j| {
            j.crew_id == Some(c.id) && j.board == board && j.date.is_some_and(|d| d >= start && d <= week_end)
        });
        let in_group = c.board == board || busy_here;
        if wanted && in_group && (c.active || busy) {
            let names = people.get(&c.id).cloned().unwrap_or_default();
            let subtitle = if c.board == Board::RoughIn && day_mode {
                // The single-day view lists that day's crew.
                if names.is_empty() { "No one on this crew today".to_string() } else { names.join(", ") }
            } else {
                let n = names.len();
                let mut s = format!("{n} member{}", if n == 1 { "" } else { "s" });
                if c.board == Board::RoughIn {
                    s.push_str(&format!(" on {}", people_day.format("%a")));
                }
                if !c.foreman.trim().is_empty() {
                    s = format!("{} · {s}", c.foreman.trim());
                }
                s
            };
            rows.push(CrewRow { crew_id: Some(c.id), name: c.name.clone(), color: c.color32(), subtitle });
        }
    }
    if matches!(app.crew_filter, CrewFilter::All | CrewFilter::Unassigned) {
        rows.push(CrewRow {
            crew_id: None,
            name: "Unassigned".into(),
            color: UNASSIGNED_COLOR,
            subtitle: "Drop a job here to unassign it".into(),
        });
    }

    // Jobs per row per day, stacked into lanes when they overlap.
    let mut laid: Vec<Vec<Vec<(Job, usize)>>> = Vec::new();
    let mut row_lanes: Vec<usize> = Vec::new();
    for row in &rows {
        let mut per_day = Vec::new();
        let mut max_lanes = 1;
        for date in &days {
            let mut list: Vec<Job> = app
                .jobs
                .iter()
                .filter(|j| app.job_on(j, *date) && j.crew_id == row.crew_id && show_job(j))
                .cloned()
                .collect();
            list.sort_by_key(|j| (j.start_min, j.id));
            let spans: Vec<(i32, i32)> =
                list.iter().map(|j| (j.start_min, j.end_min().max(j.start_min + SNAP_MIN))).collect();
            let lanes = layout_lanes(&spans);
            max_lanes = max_lanes.max(lanes.iter().map(|l| l.0 + 1).max().unwrap_or(1));
            per_day.push(list.into_iter().zip(lanes.into_iter().map(|l| l.0)).collect());
        }
        laid.push(per_day);
        row_lanes.push(max_lanes);
    }

    // Geometry.
    let gutter = 190.0;
    let full = ui.available_rect_before_wrap();
    let width = full.width() - 16.0;
    let col_w = ((width - gutter) / days.len() as f32).max(90.0);
    let ppm = col_w / (DAY_END_MIN - DAY_START_MIN) as f32;
    // Rows grow to fill the window (within limits); overlapping jobs share it.
    let (min_row_h, max_row_h, max_lane_h): (f32, f32, f32) = if day_mode { (60.0, 120.0, 50.0) } else { (48.0, 96.0, 42.0) };
    let header_h = if day_mode { 26.0 } else { 46.0 };
    let tick_every = if day_mode { 60 } else { 180 };
    let x_of = |cell_left: f32, m: i32| cell_left + (m - DAY_START_MIN) as f32 * ppm;

    // Header.
    let (hrect, _) = ui.allocate_exact_size(vec2(full.width(), header_h), Sense::hover());
    let hp = ui.painter_at(hrect);
    if day_mode {
        let d = days[0];
        hp.text(
            pos2(hrect.left() + 8.0, hrect.center().y),
            Align2::LEFT_CENTER,
            d.format("%A, %b %-d").to_string(),
            FontId::proportional(15.0),
            visuals.strong_text_color(),
        );
        let mut t = DAY_START_MIN;
        while t < DAY_END_MIN {
            let x = x_of(hrect.left() + gutter, t);
            hp.vline(x, hrect.bottom() - 6.0..=hrect.bottom(), grid_stroke);
            hp.text(
                pos2(x + 3.0, hrect.bottom() - 3.0),
                Align2::LEFT_BOTTOM,
                fmt_time(t).replace(":00", ""),
                FontId::proportional(11.0),
                visuals.weak_text_color(),
            );
            t += 60;
        }
    } else {
        for (di, date) in days.iter().enumerate() {
            let r = Rect::from_min_size(pos2(hrect.left() + gutter + di as f32 * col_w, hrect.top()), vec2(col_w, header_h));
            if *date == today {
                hp.rect_filled(r.shrink(1.0), 4.0, visuals.selection.bg_fill.gamma_multiply(0.25));
            }
            hp.text(
                r.center_top() + vec2(0.0, 3.0),
                Align2::CENTER_TOP,
                date.format("%a %b %-d").to_string(),
                FontId::proportional(14.0),
                visuals.text_color(),
            );
            for h in [6, 9, 12, 15, 18] {
                hp.text(
                    pos2(x_of(r.left(), h * 60), r.bottom() - 2.0),
                    Align2::CENTER_BOTTOM,
                    fmt_time_compact(h * 60),
                    FontId::proportional(10.0),
                    visuals.weak_text_color(),
                );
            }
            let resp = ui.interact(r, Id::new(("crew_hdr", *date)), Sense::click());
            if resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text("Show this day by the hour").clicked() {
                actions.push(Action::SelectCrewDay(Some(*date)));
            }
        }
    }

    if rows.is_empty() {
        ui.add_space(20.0);
        ui.weak("No crews to show. Add crews in the Crews tab, or change the crew filter.");
    }

    let now_min = {
        use chrono::Timelike;
        let now = chrono::Local::now();
        (now.hour() * 60 + now.minute()) as i32
    };

    ScrollArea::vertical()
        .id_salt("crew_scroll")
        .auto_shrink([false, false])
        .drag_to_scroll(false)
        .show(ui, |ui| {
            let fill = (ui.available_height() / rows.len().max(1) as f32).clamp(min_row_h, max_row_h);
            let heights: Vec<f32> = row_lanes.iter().map(|&l| fill.max(l as f32 * 24.0 + 10.0)).collect();
            let lane_hs: Vec<f32> =
                row_lanes.iter().zip(&heights).map(|(&l, &h)| ((h - 10.0) / l as f32).min(max_lane_h)).collect();
            let total: f32 = heights.iter().sum::<f32>().max(1.0);
            let (area, _) = ui.allocate_exact_size(vec2(gutter + col_w * days.len() as f32, total), Sense::hover());
            let clip = ui.clip_rect();
            let painter = ui.painter_at(area);
            let mut y = area.top();

            for (ri, row) in rows.iter().enumerate() {
                let h = heights[ri];
                // Crew label.
                let label = Rect::from_min_size(pos2(area.left(), y), vec2(gutter, h));
                let lp = painter.with_clip_rect(label.intersect(painter.clip_rect()));
                lp.rect_filled(
                    Rect::from_min_size(label.min + vec2(6.0, 6.0), vec2(5.0, h - 12.0)),
                    2.0,
                    row.color,
                );
                lp.text(label.min + vec2(18.0, 8.0), Align2::LEFT_TOP, &row.name, FontId::proportional(14.0), visuals.strong_text_color());
                // Subtitle wraps to the rows that fit, then ends in "…".
                let mut job = egui::text::LayoutJob::simple(
                    row.subtitle.clone(),
                    FontId::proportional(11.0),
                    visuals.weak_text_color(),
                    gutter - 26.0,
                );
                job.wrap.max_rows = (((h - 31.0) / 13.0).floor() as usize).max(1);
                job.wrap.break_anywhere = false;
                job.wrap.overflow_character = Some('…');
                let galley = lp.layout_job(job);
                lp.galley(label.min + vec2(18.0, 27.0), galley, visuals.weak_text_color());

                for (di, date) in days.iter().enumerate() {
                    let cell = Rect::from_min_size(pos2(area.left() + gutter + di as f32 * col_w, y), vec2(col_w, h));
                    let weekend = matches!(date.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun);
                    let bg = if *date == today {
                        mix(visuals.extreme_bg_color, visuals.selection.bg_fill, 0.08)
                    } else if weekend {
                        mix(visuals.extreme_bg_color, visuals.weak_text_color(), 0.05)
                    } else {
                        visuals.extreme_bg_color
                    };
                    painter.rect_filled(cell, 0.0, bg);
                    let mut t = DAY_START_MIN + tick_every - DAY_START_MIN % tick_every;
                    while t < DAY_END_MIN {
                        painter.vline(x_of(cell.left(), t), cell.y_range(), faint_stroke);
                        t += tick_every;
                    }
                    painter.vline(cell.left(), cell.y_range(), grid_stroke);
                    if *date == today && (DAY_START_MIN..DAY_END_MIN).contains(&now_min) {
                        painter.vline(x_of(cell.left(), now_min), cell.y_range(), Stroke::new(2.0_f32, Color32::from_rgb(230, 40, 40)));
                    }

                    let visible = cell.intersect(clip);
                    if visible.is_positive() {
                        targets.push((
                            visible,
                            DropTarget::CrewSlot { crew_id: row.crew_id, date: *date, x0: cell.left(), px_per_min: ppm, cell },
                        ));
                    }
                    let bg_resp = ui.interact(cell, Id::new(("crew_cell", row.crew_id, *date)), Sense::click());
                    if bg_resp.double_clicked() {
                        if let Some(p) = bg_resp.interact_pointer_pos() {
                            let m = DAY_START_MIN + ((p.x - cell.left()) / ppm) as i32;
                            let m = ((m / 30) * 30).clamp(DAY_START_MIN, DAY_END_MIN - 30);
                            actions.push(Action::NewForCrew(*date, m, row.crew_id));
                        }
                    }

                    for (job, lane) in &laid[ri][di] {
                        let s = job.start_min.clamp(DAY_START_MIN, DAY_END_MIN - SNAP_MIN);
                        let e = job.end_min().clamp(s + SNAP_MIN, DAY_END_MIN);
                        let r = Rect::from_min_max(
                            pos2(x_of(cell.left(), s) + 1.0, y + 5.0 + *lane as f32 * lane_hs[ri]),
                            pos2(x_of(cell.left(), e) - 1.0, y + 5.0 + (*lane + 1) as f32 * lane_hs[ri] - 3.0),
                        );
                        let style = match (day_mode, r.height() >= 32.0) {
                            (true, true) => CardStyle::Block,
                            (false, true) => CardStyle::Mini,
                            _ => CardStyle::Compact,
                        };
                        card_widget(ui, &painter, r, job, app, conflicts.contains(&job.id), style, Some(*date), None, &mut actions);
                        if job.board != board {
                            // Other schedule's job: fade it and tag it.
                            painter.rect_filled(r, 4.0, visuals.extreme_bg_color.gamma_multiply(0.55));
                            if r.width() > 70.0 && r.height() >= 30.0 {
                                painter.text(
                                    r.right_top() + vec2(-5.0, 3.0),
                                    Align2::RIGHT_TOP,
                                    job.board.as_str(),
                                    FontId::proportional(10.0),
                                    visuals.strong_text_color(),
                                );
                            }
                        }
                    }
                }
                painter.hline(area.x_range(), y + h, grid_stroke);
                y += h;
            }
            painter.vline(area.left() + gutter, area.y_range(), grid_stroke);
        });

    app.drop_targets.extend(targets);
    apply_actions(app, actions);
}

// -------------------------------------------------------- unscheduled panel

pub fn unscheduled_panel(app: &mut SchedulerApp, ctx: &egui::Context) {
    egui::SidePanel::right("unscheduled_panel")
        .resizable(true)
        .default_width(250.0)
        .width_range(190.0..=420.0)
        .show(ctx, |ui| {
            let panel_rect = ui.max_rect();
            let mut actions = Vec::new();
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading("Unscheduled");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(app.can_edit(), egui::Button::new("+ Add").small())
                        .on_hover_text("New unscheduled job")
                        .clicked()
                    {
                        actions.push(Action::New(None, None));
                    }
                });
            });
            ui.label(
                RichText::new("Drag a job onto the calendar to schedule it. Drop a scheduled job here to unschedule it.")
                    .weak()
                    .small(),
            );
            ui.add_space(4.0);

            let jobs: Vec<Job> = app
                .jobs
                .iter()
                .filter(|j| j.date.is_none() && app.job_visible(j))
                .cloned()
                .collect();
            if jobs.is_empty() {
                ui.add_space(8.0);
                ui.weak("No unscheduled jobs.");
            }
            ScrollArea::vertical()
                .id_salt("unscheduled_scroll")
                .auto_shrink([false, false])
                .drag_to_scroll(false)
                .show(ui, |ui| {
                    for job in &jobs {
                        let (_, rect) = ui.allocate_space(vec2(ui.available_width(), 50.0));
                        let painter = ui.painter().clone();
                        card_widget(ui, &painter, rect, job, app, false, CardStyle::Unscheduled, None, None, &mut actions);
                    }
                });

            app.drop_targets.push((panel_rect, DropTarget::Unscheduled));
            apply_actions(app, actions);
        });
}

// ------------------------------------------------------------ drag & drop

#[derive(Debug, PartialEq)]
pub struct ResolvedDrop {
    /// None = unschedule.
    pub date: Option<NaiveDate>,
    pub start_min: i32,
    /// Area to highlight while hovering.
    pub highlight: Rect,
    pub clip: Rect,
    /// True when dropping into a time slot (week view or crew timeline).
    pub slot: bool,
    /// Some(crew) when the drop also reassigns the crew (Some(None) = unassigned).
    pub crew: Option<Option<i64>>,
}

/// Works out where a dragged job would land if released at `p`.
/// `grab` is where the card was grabbed, relative to its top-left.
/// Later targets win, so cells drawn on top take priority.
pub fn resolve_drop(targets: &[(Rect, DropTarget)], p: egui::Pos2, grab: Vec2, job: &Job) -> Option<ResolvedDrop> {
    let (rect, target) = targets.iter().rev().find(|(r, _)| r.contains(p))?;
    Some(match target {
        DropTarget::Day(date) => ResolvedDrop {
            date: Some(*date),
            start_min: job.start_min,
            highlight: *rect,
            clip: *rect,
            slot: false,
            crew: None,
        },
        DropTarget::CrewSlot { crew_id, date, x0, px_per_min, cell } => {
            // Cards grabbed in other views can be wider than this job's bar;
            // never offset the start by more than the bar's own length.
            let grab_x = grab.x.min(job.duration_min as f32 * px_per_min);
            let m = DAY_START_MIN + ((p.x - grab_x - x0) / px_per_min).round() as i32;
            let m = snap(m).clamp(DAY_START_MIN, DAY_END_MIN - SNAP_MIN);
            let end = (m + job.duration_min).min(DAY_END_MIN);
            ResolvedDrop {
                date: Some(*date),
                start_min: m,
                highlight: Rect::from_min_max(
                    pos2(x0 + (m - DAY_START_MIN) as f32 * px_per_min, cell.top() + 2.0),
                    pos2(x0 + (end - DAY_START_MIN) as f32 * px_per_min, cell.bottom() - 2.0),
                ),
                clip: *rect,
                slot: true,
                crew: Some(*crew_id),
            }
        }
        DropTarget::Slot { date, grid_top, px_per_min, col } => {
            let top_y = p.y - grab.y;
            let m = DAY_START_MIN + ((top_y - grid_top) / px_per_min).round() as i32;
            let m = snap(m).clamp(DAY_START_MIN, DAY_END_MIN - SNAP_MIN);
            let end = (m + job.duration_min).min(DAY_END_MIN);
            ResolvedDrop {
                date: Some(*date),
                start_min: m,
                highlight: Rect::from_min_max(
                    pos2(col.left() + 2.0, grid_top + (m - DAY_START_MIN) as f32 * px_per_min),
                    pos2(col.right() - 2.0, grid_top + (end - DAY_START_MIN) as f32 * px_per_min),
                ),
                clip: *rect,
                slot: true,
                crew: None,
            }
        }
        DropTarget::Unscheduled => ResolvedDrop {
            date: None,
            start_min: job.start_min,
            highlight: *rect,
            clip: *rect,
            slot: false,
            crew: None,
        },
    })
}

/// Called last each frame: paints the drop highlight and drag ghost, and
/// applies the move when the mouse button is released.
pub fn finish_drag(app: &mut SchedulerApp, ctx: &egui::Context) {
    let Some(drag) = app.drag.clone() else {
        return;
    };
    let Some(job) = app.jobs.iter().find(|j| j.id == drag.job_id).cloned() else {
        app.drag = None;
        return;
    };
    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        app.drag = None;
        app.set_status("Move cancelled");
        return;
    }
    ctx.set_cursor_icon(CursorIcon::Grabbing);

    let (pointer, released) = ctx.input(|i| {
        (i.pointer.latest_pos(), i.pointer.any_released() || !i.pointer.primary_down())
    });
    let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("drag_ghost_layer")));
    let accent = ctx.style().visuals.selection.bg_fill;
    let mut drop: Option<(Option<NaiveDate>, i32, Option<Option<i64>>)> = None;

    if let Some(p) = pointer {
        if let Some(res) = resolve_drop(&app.drop_targets, p, drag.grab_offset, &job) {
            let clipped = painter.with_clip_rect(res.clip);
            let (fill, rounding) = if res.slot { (0.25, 4.0) } else { (0.15, 0.0) };
            clipped.rect_filled(res.highlight, rounding, accent.gamma_multiply(fill));
            clipped.rect_stroke(res.highlight.shrink(1.0), rounding, Stroke::new(2.0_f32, accent));
            drop = Some((res.date, res.start_min, res.crew));
        }

        // Ghost card following the pointer.
        let ghost = Rect::from_min_size(p - drag.grab_offset, drag.size);
        painter.rect_filled(ghost.translate(vec2(3.0, 4.0)), 4.0, Color32::from_black_alpha(60));
        let style = if drag.size.y < 34.0 { CardStyle::Compact } else { CardStyle::Unscheduled };
        paint_card(&painter, ghost, &job, app.job_color(&job), &app.crew_name(&job), false, false, style, None, None);

        // Where it will land.
        let hint = match drop {
            Some((Some(d), m, crew)) => {
                let mut h = format!("{} · {}", d.format("%a %b %-d"), fmt_time(m));
                if drag.day_index > 0 {
                    // Dragging a later day of a multi-day job moves the whole job.
                    let start = start_for_day(d, drag.day_index, job.weekend);
                    h = format!("Day {} on {h} · starts {}", drag.day_index + 1, start.format("%a %b %-d"));
                }
                if let Some(c) = crew {
                    h.push_str(" · ");
                    h.push_str(app.crew(c).map(|c| c.name.as_str()).unwrap_or("Unassigned"));
                }
                h
            }
            Some((None, _, _)) => "Unschedule".to_string(),
            None => "Release to cancel".to_string(),
        };
        let galley = painter.layout_no_wrap(hint, FontId::proportional(12.5), Color32::WHITE);
        let hint_rect = Rect::from_min_size(pos2(ghost.left(), ghost.bottom() + 6.0), galley.size()).expand(5.0);
        painter.rect_filled(hint_rect, 4.0, Color32::from_black_alpha(200));
        painter.galley(hint_rect.min + vec2(5.0, 5.0), galley, Color32::WHITE);
    }

    if released {
        if std::env::var_os("PLUMBING_SCHEDULER_DEBUG").is_some() {
            eprintln!("drop: pointer={pointer:?} drop={drop:?} targets={}", app.drop_targets.len());
        }
        app.drag = None;
        match drop {
            Some((date, m, crew)) => {
                let date = date.map(|d| start_for_day(d, drag.day_index, job.weekend));
                app.move_job(job.id, date, m, crew)
            }
            None => app.set_status("Move cancelled"),
        }
    }
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, day).unwrap()
    }

    fn job(start: i32, dur: i32) -> Job {
        let mut j = Job::new(Some(d(12)), start);
        j.duration_min = dur;
        j
    }

    fn targets() -> Vec<(Rect, DropTarget)> {
        let col = Rect::from_min_size(pos2(100.0, 50.0), vec2(80.0, 900.0));
        vec![
            // Side panel pushed first (as in the app), cells later.
            (Rect::from_min_size(pos2(1000.0, 0.0), vec2(200.0, 1000.0)), DropTarget::Unscheduled),
            (Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 100.0)), DropTarget::Day(d(14))),
            (col, DropTarget::Slot { date: d(20), grid_top: 50.0, px_per_min: 1.0, col }),
        ]
    }

    #[test]
    fn new_jobs_default_to_an_8_hour_day() {
        let j = Job::new(Some(d(13)), 7 * 60);
        assert_eq!((j.duration_min, j.days), (8 * 60, 1));
        assert_eq!(fmt_time(j.end_min()), fmt_time(15 * 60));
    }

    #[test]
    fn multi_day_jobs_skip_weekends() {
        // Nov 13, 2026 is a Friday.
        let mut j = job(420, 240);
        j.date = Some(d(13));
        j.days = 3;
        assert_eq!(j.work_dates(), vec![d(13), d(16), d(17)]);
        assert_eq!(j.last_date(), Some(d(17)));
        assert_eq!((j.day_index(d(13)), j.day_index(d(14)), j.day_index(d(17))), (Some(0), None, Some(2)));
        assert_eq!(j.day_index(d(18)), None);
        // A one-day job is only on its date.
        j.days = 1;
        assert_eq!(j.work_dates(), vec![d(13)]);
        // Unscheduled: no days.
        j.date = None;
        assert!(j.work_dates().is_empty());
        // A job may start on a Saturday; the days after it skip the weekend.
        assert_eq!(work_dates(d(14), 2, Weekend::SKIP), vec![d(14), d(16)]);
    }

    #[test]
    fn ticked_weekend_days_count_as_job_days() {
        let sat = Weekend { sat: true, sun: false };
        let sun = Weekend { sat: false, sun: true };
        let both = Weekend { sat: true, sun: true };
        // Friday start, 2 days.
        assert_eq!(work_dates(d(13), 2, Weekend::SKIP), vec![d(13), d(16)]);
        assert_eq!(work_dates(d(13), 2, sat), vec![d(13), d(14)]);
        assert_eq!(work_dates(d(13), 2, sun), vec![d(13), d(15)]);
        assert_eq!(work_dates(d(13), 3, both), vec![d(13), d(14), d(15)]);
        let mut j = job(420, 480);
        j.date = Some(d(13));
        j.days = 3;
        j.weekend = sat;
        assert_eq!(j.work_dates(), vec![d(13), d(14), d(16)]);
        assert_eq!(j.day_index(d(14)), Some(1));
        assert_eq!(j.day_index(d(15)), None);
        // Dragging day 2 (a Saturday) back onto Sat 14 keeps the Friday start.
        assert_eq!(start_for_day(d(14), 1, sat), d(13));
        assert_eq!(start_for_day(d(16), 2, sat), d(13));
    }

    #[test]
    fn dragging_a_later_day_moves_the_whole_job() {
        // Day 2 dropped on Tue 17 → starts Mon 16; day 2 dropped on Mon 16 → starts Fri 13.
        let s = Weekend::SKIP;
        assert_eq!(start_for_day(d(17), 1, s), d(16));
        assert_eq!(start_for_day(d(16), 1, s), d(13));
        assert_eq!(start_for_day(d(18), 0, s), d(18));
        assert_eq!(start_for_day(d(18), 3, s), d(13));
    }

    #[test]
    fn multi_day_job_conflicts_on_later_days() {
        let mut long = job(420, 240);
        long.id = 1;
        long.crew_id = Some(7);
        long.date = Some(d(13));
        long.days = 2; // Fri 13 and Mon 16
        let mut monday = job(480, 60);
        monday.id = 2;
        monday.crew_id = Some(7);
        monday.date = Some(d(16));
        let c = conflicts(&[long.clone(), monday.clone()]);
        assert!(c.contains(&1) && c.contains(&2));
        monday.date = Some(d(17));
        assert!(conflicts(&[long, monday]).is_empty());
    }

    #[test]
    fn drop_on_month_cell_keeps_time() {
        let r = resolve_drop(&targets(), pos2(50.0, 50.0), vec2(0.0, 5.0), &job(450, 240)).unwrap();
        assert_eq!((r.date, r.start_min), (Some(d(14)), 450));
    }

    #[test]
    fn drop_on_week_slot_snaps_time() {
        // Card top lands 3h07m below 5:00 AM -> snaps to 8:00 AM.
        let r = resolve_drop(&targets(), pos2(120.0, 50.0 + 187.0 + 10.0), vec2(0.0, 10.0), &job(420, 240)).unwrap();
        assert_eq!((r.date, r.start_min), (Some(d(20)), 8 * 60));
        assert!(r.slot);
        // Above the grid clamps to the first slot.
        let r = resolve_drop(&targets(), pos2(120.0, 55.0), vec2(0.0, 40.0), &job(420, 240)).unwrap();
        assert_eq!(r.start_min, DAY_START_MIN);
    }

    #[test]
    fn drop_on_panel_unschedules_and_outside_cancels() {
        let r = resolve_drop(&targets(), pos2(1100.0, 300.0), vec2(0.0, 0.0), &job(420, 60)).unwrap();
        assert_eq!(r.date, None);
        assert!(resolve_drop(&targets(), pos2(500.0, 500.0), vec2(0.0, 0.0), &job(420, 60)).is_none());
    }

    #[test]
    fn drop_on_crew_row_sets_crew_and_time() {
        // 1 px per minute, cell starts at x=200 (= 5:00 AM).
        let cell = Rect::from_min_size(pos2(200.0, 400.0), vec2(900.0, 50.0));
        let t = vec![(cell, DropTarget::CrewSlot { crew_id: Some(7), date: d(13), x0: 200.0, px_per_min: 1.0, cell })];
        // Bar grabbed 30 px from its left edge; its left lands at 200+240 -> 9:00 AM.
        let r = resolve_drop(&t, pos2(200.0 + 240.0 + 30.0, 420.0), vec2(30.0, 10.0), &job(420, 120)).unwrap();
        assert_eq!((r.date, r.start_min, r.crew), (Some(d(13)), 9 * 60, Some(Some(7))));
        // A wide card from another view: grab offset is capped to the bar length (120 px).
        let r = resolve_drop(&t, pos2(200.0 + 360.0 + 2.0, 420.0), vec2(500.0, 10.0), &job(420, 120)).unwrap();
        assert_eq!(r.start_min, 5 * 60 + 240);
        // Unassigned row.
        let t = vec![(cell, DropTarget::CrewSlot { crew_id: None, date: d(13), x0: 200.0, px_per_min: 1.0, cell })];
        let r = resolve_drop(&t, pos2(260.0, 420.0), Vec2::ZERO, &job(420, 60)).unwrap();
        assert_eq!(r.crew, Some(None));
    }

    #[test]
    fn overlapping_jobs_get_separate_lanes() {
        // 7-11, 8-9, 9-10, 12-13
        let lanes = layout_lanes(&[(420, 660), (480, 540), (540, 600), (720, 780)]);
        assert_eq!(lanes, vec![(0, 2), (1, 2), (1, 2), (0, 1)]);
    }

    #[test]
    fn detects_double_booked_crew() {
        let mut a = job(420, 240);
        a.id = 1;
        a.crew_id = Some(7);
        let mut b = job(600, 60);
        b.id = 2;
        b.crew_id = Some(7);
        let mut c = job(660, 60); // starts exactly when a ends: OK
        c.id = 3;
        c.crew_id = Some(7);
        let mut other = job(420, 240);
        other.id = 4;
        other.crew_id = Some(8);
        let set = conflicts(&[a, b, c, other]);
        assert_eq!(set, HashSet::from([1, 2]));
    }
}
