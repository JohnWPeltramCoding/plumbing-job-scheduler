//! Rough-In Staff → Attendance: a week grid of Rough-In employees with a
//! status per day (Present, Late, Absent, Sick, PTO, Off) and an optional note.

use std::collections::HashMap;

use chrono::{Duration, NaiveDate};
use eframe::egui::{self, vec2, Align, Button, Color32, Grid, Layout, RichText, ScrollArea, TextEdit};

use crate::app::{today, SchedulerApp};
use crate::calendar::week_start;
use crate::model::*;

#[derive(Default)]
pub struct AttendanceState {
    /// Single-day crew board instead of the week grid.
    day_view: bool,
    /// Records for the week being shown, reloaded after every change.
    cache: Option<(NaiveDate, HashMap<(i64, NaiveDate), AttendanceEntry>)>,
    edit: Option<CellEdit>,
}

struct CellEdit {
    employee_id: i64,
    name: String,
    date: NaiveDate,
    entry: AttendanceEntry,
    existed: bool,
}

enum Act {
    Edit(i64, String, NaiveDate),
    AllPresent(NaiveDate),
    Shift(i64),
    ThisWeek,
    /// Look at (and edit the crews of) this day.
    Pick(NaiveDate),
    /// Change someone's crew from the grid (None = no crew).
    MoveCrew(i64, Option<i64>),
    /// Switch between the week grid (false) and the day board (true).
    SetDayView(bool),
}

/// Statuses that mean the person isn't working that day.
pub const NOT_WORKING: &[&str] = &["PTO", "Off", "Absent", "Sick"];

/// Marking someone not-working takes them off their crew for that day (crews
/// are recorded per day, so other days are unaffected).
pub fn takes_off_crew(status: &str) -> bool {
    NOT_WORKING.contains(&status)
}

fn status_color(status: &str) -> Color32 {
    ATTENDANCE
        .iter()
        .find(|(s, _, _)| *s == status)
        .map(|(_, _, c)| Color32::from_rgb(c[0], c[1], c[2]))
        .unwrap_or(Color32::GRAY)
}

fn status_short(status: &str) -> &str {
    ATTENDANCE.iter().find(|(s, _, _)| *s == status).map(|(_, short, _)| *short).unwrap_or(status)
}

pub fn show(app: &mut SchedulerApp, ctx: &egui::Context) {
    let today = today();
    // The day shown is the Rough-In "crews on" day, shared with Crews/Employees.
    let focus = app.roster_day;
    let day_view = app.attendance_state.day_view;
    let start = week_start(focus);
    let end = start + Duration::days(6);

    // Load (or reuse) this week's records.
    if app.attendance_state.cache.as_ref().map(|(s, _)| *s) != Some(start) {
        match app.db.load_attendance(start, end) {
            Ok(map) => app.attendance_state.cache = Some((start, map)),
            Err(e) => {
                app.set_error(format!("Could not load attendance: {e}"));
                app.attendance_state.cache = Some((start, HashMap::new()));
            }
        }
    }
    let editable = app.can_edit();
    let days: Vec<NaiveDate> = (0..7).map(|i| start + Duration::days(i)).collect();
    let mut acts: Vec<Act> = Vec::new();

    // Rough-In employees: active ones, plus inactive ones with entries this week.
    let records = &app.attendance_state.cache.as_ref().unwrap().1;
    let crew_name = |id: Option<i64>| {
        id.and_then(|id| app.crews.iter().find(|c| c.id == id)).map(|c| c.name.clone()).unwrap_or_default()
    };
    let rough_crews: Vec<(i64, String, Color32)> = app
        .crews
        .iter()
        .filter(|c| c.board == Board::RoughIn && c.active)
        .map(|c| (c.id, c.name.clone(), c.color32()))
        .collect();
    let mut people: Vec<&Employee> = app
        .employees
        .iter()
        .filter(|e| e.staff == Board::RoughIn)
        .filter(|e| e.active || days.iter().any(|d| records.contains_key(&(e.id, *d))))
        .collect();
    people.sort_by_key(|e| (crew_name(e.crew_id).is_empty(), crew_name(e.crew_id), e.name.to_lowercase()));

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.selectable_label(!day_view, "Week").on_hover_text("Everyone, day by day").clicked() && day_view {
                acts.push(Act::SetDayView(false));
            }
            if ui.selectable_label(day_view, "Day").on_hover_text("One day, laid out by crew").clicked() && !day_view {
                acts.push(Act::SetDayView(true));
            }
            ui.separator();
            let (step, unit, home) = if day_view { (1, "day", "Today") } else { (7, "week", "This Week") };
            if ui.button(RichText::new("◀").size(16.0)).on_hover_text(format!("Previous {unit}")).clicked() {
                acts.push(Act::Shift(-step));
            }
            if ui.button(home).clicked() {
                acts.push(Act::ThisWeek);
            }
            if ui.button(RichText::new("▶").size(16.0)).on_hover_text(format!("Next {unit}")).clicked() {
                acts.push(Act::Shift(step));
            }
            ui.add_space(8.0);
            let title = if day_view {
                let mut t = format!("Attendance · {}", focus.format("%A, %b %-d, %Y"));
                if focus == today {
                    t.push_str(" (today)");
                }
                t
            } else {
                format!("Attendance · {} – {}", start.format("%b %-d"), end.format("%b %-d, %Y"))
            };
            ui.label(RichText::new(title).size(19.0).strong());
            if day_view
                && editable
                && focus <= today
                && ui.small_button("All present").on_hover_text("Mark everyone without an entry this day as Present").clicked()
            {
                acts.push(Act::AllPresent(focus));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                for (status, short, c) in ATTENDANCE.iter().rev() {
                    let color = Color32::from_rgb(c[0], c[1], c[2]);
                    ui.label(RichText::new(*status).small());
                    ui.add(
                        Button::new(RichText::new(*short).small().color(Color32::WHITE))
                            .fill(color)
                            .sense(egui::Sense::hover()),
                    );
                }
            });
        });
        ui.add_space(4.0);
        if people.is_empty() {
            ui.add_space(20.0);
            ui.weak("No Rough-In employees yet. Add them on the Employees sub-tab.");
            return;
        }
        if !editable {
            ui.disable();
        }
        if day_view {
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                day_board(ui, app, records, focus, &rough_crews, &mut acts);
            });
            return;
        }

        ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
            Grid::new("attendance_grid").striped(true).spacing([6.0, 4.0]).min_col_width(70.0).show(ui, |ui| {
                // Header row.
                ui.label(RichText::new("Employee").strong());
                ui.label(RichText::new(format!("Crew on {}", focus.format("%a %-d"))).strong())
                    .on_hover_text("Crews change day to day. Click a day's heading to see and change that day's crews.");
                for d in &days {
                    ui.vertical_centered(|ui| {
                        let mut t = RichText::new(d.format("%a %-d").to_string()).strong();
                        if *d == today {
                            t = t.color(ui.visuals().selection.stroke.color);
                        }
                        if ui
                            .selectable_label(*d == focus, t)
                            .on_hover_text("Show this day's crews in the Crew column")
                            .clicked()
                            && *d != focus
                        {
                            acts.push(Act::Pick(*d));
                        }
                        if *d <= today
                            && ui
                                .small_button("All present")
                                .on_hover_text("Mark everyone without an entry this day as Present")
                                .clicked()
                        {
                            acts.push(Act::AllPresent(*d));
                        }
                    });
                }
                ui.label(RichText::new("This week").strong());
                ui.end_row();

                for e in &people {
                    ui.horizontal(|ui| {
                        let mut name = RichText::new(&e.name);
                        if !e.active {
                            name = name.weak().italics();
                        }
                        ui.label(name);
                        if !e.role.is_empty() {
                            ui.weak(&e.role);
                        }
                    });
                    // Crew: change it right here (Rough-In crews only).
                    let current = crew_name(e.crew_id);
                    let shown = if current.is_empty() { "No crew".to_string() } else { current };
                    egui::ComboBox::from_id_salt(("att_crew", e.id))
                        .selected_text(RichText::new(shown).size(12.5))
                        .width(150.0)
                        .show_ui(ui, |ui| {
                            for (cid, cname, color) in &rough_crews {
                                ui.horizontal(|ui| {
                                    let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), egui::Sense::hover());
                                    ui.painter().rect_filled(r, 2.0, *color);
                                    let is_current = e.crew_id == Some(*cid);
                                    if ui.selectable_label(is_current, cname.as_str()).clicked() && !is_current {
                                        acts.push(Act::MoveCrew(e.id, Some(*cid)));
                                    }
                                });
                            }
                            ui.separator();
                            if ui.selectable_label(e.crew_id.is_none(), "No crew").clicked() && e.crew_id.is_some() {
                                acts.push(Act::MoveCrew(e.id, None));
                            }
                        });
                    let mut counts: Vec<(String, usize)> = Vec::new();
                    for d in &days {
                        let resp = match records.get(&(e.id, *d)) {
                            Some(entry) => {
                                let mut text = status_short(&entry.status).to_string();
                                if !entry.note.trim().is_empty() {
                                    text.push_str(" •");
                                }
                                let r = ui.add_sized(
                                    [70.0, 24.0],
                                    Button::new(RichText::new(text).color(Color32::WHITE).strong())
                                        .fill(status_color(&entry.status)),
                                );
                                match counts.iter_mut().find(|(s, _)| *s == entry.status) {
                                    Some(c) => c.1 += 1,
                                    None => counts.push((entry.status.clone(), 1)),
                                }
                                let tip = if entry.note.trim().is_empty() {
                                    entry.status.clone()
                                } else {
                                    format!("{}: {}", entry.status, entry.note)
                                };
                                r.on_hover_text(tip)
                            }
                            None => ui
                                .add_sized([70.0, 24.0], Button::new(RichText::new("–").weak()))
                                .on_hover_text("Click to record attendance"),
                        };
                        if resp.clicked() {
                            acts.push(Act::Edit(e.id, e.name.clone(), *d));
                        }
                    }
                    // Week totals, in the same order as the legend.
                    let summary: Vec<String> = ATTENDANCE
                        .iter()
                        .filter_map(|(status, short, _)| {
                            counts.iter().find(|(s, _)| s == status).map(|(_, n)| format!("{short} {n}"))
                        })
                        .collect();
                    ui.weak(summary.join(" · "));
                    ui.end_row();
                }
            });
        });
    });

    // Edit window for one cell.
    let mut save: Option<(i64, NaiveDate, Option<AttendanceEntry>)> = None;
    // Crew of the person being edited (for the "takes them off" warning).
    // (Their crew on the day being edited, which may not be the day shown.)
    let edit_who = app.attendance_state.edit.as_ref().map(|ed| (ed.employee_id, ed.date));
    let edit_crew: Option<String> = edit_who
        .and_then(|(emp, date)| app.crew_of_on(emp, date))
        .and_then(|c| app.crews.iter().find(|x| x.id == c))
        .map(|c| c.name.clone());
    if let Some(edit) = app.attendance_state.edit.as_mut() {
        let mut open = true;
        let mut close = false;
        egui::Window::new(format!("{} · {}", edit.name, edit.date.format("%a %b %-d")))
            .id(egui::Id::new("attendance_edit"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (status, _, c) in ATTENDANCE {
                        let selected = edit.entry.status == *status;
                        let color = Color32::from_rgb(c[0], c[1], c[2]);
                        let mut b = Button::new(RichText::new(*status).color(if selected { Color32::WHITE } else { color }).strong())
                            .min_size(vec2(64.0, 28.0));
                        if selected {
                            b = b.fill(color);
                        }
                        if ui.add(b).clicked() {
                            edit.entry.status = status.to_string();
                        }
                    }
                });
                ui.add_space(6.0);
                ui.label("Note (optional)");
                ui.add(TextEdit::singleline(&mut edit.entry.note).desired_width(320.0).hint_text("e.g. left at noon, doctor appt."));
                if let Some(c) = &edit_crew {
                    if takes_off_crew(&edit.entry.status) {
                        ui.add_space(4.0);
                        ui.colored_label(
                            Color32::from_rgb(230, 150, 30),
                            format!(
                                "Saving also takes {} off {} for {} only (they're back on it the next day).",
                                edit.name,
                                c,
                                edit.date.format("%a %b %-d")
                            ),
                        );
                    }
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let can_save = !edit.entry.status.is_empty();
                    if ui.add_enabled(can_save, Button::new(RichText::new("Save").strong())).clicked() {
                        save = Some((edit.employee_id, edit.date, Some(edit.entry.clone())));
                    }
                    if edit.existed && ui.button("Clear").on_hover_text("Remove this day's entry").clicked() {
                        save = Some((edit.employee_id, edit.date, None));
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            });
        if !open || close {
            app.attendance_state.edit = None;
        }
    }

    // Apply changes.
    let user = app.user_id();
    if let Some((emp, date, entry)) = save {
        let result = match &entry {
            Some(e) => app.db.set_attendance(emp, date, e, user),
            None => app.db.clear_attendance(emp, date),
        };
        match result {
            Ok(()) => {
                app.attendance_state.edit = None;
                app.attendance_state.cache = None;
                let name = app.employees.iter().find(|e| e.id == emp).map(|e| e.name.clone()).unwrap_or_default();
                let day = date.format("%a %b %-d");
                // PTO / Off / Absent / Sick: off their crew for that day only;
                // anything else (or clearing the entry) puts them back on it.
                let out = entry.as_ref().is_some_and(|e| takes_off_crew(&e.status));
                let was_on = app.crew_of_on(emp, date);
                let crew = app.db.set_roster_out(date, emp, out);
                app.reload();
                let crew_name =
                    |id: Option<i64>| id.and_then(|c| app.crews.iter().find(|x| x.id == c)).map(|c| c.name.clone());
                match crew {
                    Err(e) => app.set_error(format!("Saved attendance, but could not update crews: {e}")),
                    Ok(c) => {
                        let mut msg = match &entry {
                            Some(e) => format!("Marked {name} {} for {day}.", e.status),
                            None => format!("Cleared {name}'s entry for {day}."),
                        };
                        match (out, crew_name(was_on), crew_name(c)) {
                            (true, Some(from), _) => msg.push_str(&format!(" Off {from} for {day} only.")),
                            (false, None, Some(to)) => msg.push_str(&format!(" Back on {to} for {day}.")),
                            _ => {}
                        }
                        app.set_status(msg);
                    }
                }
            }
            Err(e) => app.set_error(format!("Could not save attendance: {e}")),
        }
    }
    for act in acts {
        match act {
            Act::MoveCrew(id, crew) => match app.move_employee(id, crew) {
                Ok(msg) => app.set_status(msg),
                Err(e) => app.set_error(e),
            },
            Act::Edit(id, name, date) => {
                if !app.can_edit() {
                    continue;
                }
                let existing = app.attendance_state.cache.as_ref().and_then(|(_, m)| m.get(&(id, date)).cloned());
                app.attendance_state.edit = Some(CellEdit {
                    employee_id: id,
                    name,
                    date,
                    existed: existing.is_some(),
                    entry: existing.unwrap_or(AttendanceEntry { status: "Present".into(), note: String::new() }),
                });
            }
            Act::AllPresent(date) => {
                if !app.require_edit() {
                    continue;
                }
                let map = app.attendance_state.cache.as_ref().map(|(_, m)| m.clone()).unwrap_or_default();
                let ids: Vec<i64> = app
                    .employees
                    .iter()
                    .filter(|e| e.staff == Board::RoughIn && e.active && !map.contains_key(&(e.id, date)))
                    .map(|e| e.id)
                    .collect();
                let present = AttendanceEntry { status: "Present".into(), note: String::new() };
                let mut failed = None;
                for id in &ids {
                    if let Err(e) = app.db.set_attendance(*id, date, &present, user) {
                        failed = Some(e);
                        break;
                    }
                }
                app.attendance_state.cache = None;
                match failed {
                    Some(e) => app.set_error(format!("Could not save attendance: {e}")),
                    None => app.set_status(format!(
                        "Marked {} employee{} Present for {}.",
                        ids.len(),
                        if ids.len() == 1 { "" } else { "s" },
                        date.format("%a %b %-d")
                    )),
                }
            }
            Act::Shift(n) => app.set_roster_day(focus + Duration::days(n)),
            Act::ThisWeek => app.set_roster_day(today),
            Act::Pick(d) => app.set_roster_day(d),
            Act::SetDayView(on) => app.attendance_state.day_view = on,
        }
    }
}

/// One box on the Day board: a crew, or a group of people not on a crew.
#[derive(Debug, PartialEq)]
pub struct DayCard {
    pub title: String,
    /// Crew id for crew cards.
    pub crew: Option<i64>,
    /// Second line under the title: the crew's vehicle, else its notes.
    pub sub: String,
    /// Set when the crew's vehicle is In Shop or Out of Service.
    pub vehicle_warning: Option<String>,
    /// For the group cards: the status they're grouped by (colors the title).
    pub status: Option<String>,
    /// (employee id, is the lead), lead first.
    pub people: Vec<(i64, bool)>,
}

/// Who leads a crew: the employee named as its foreman/lead, else its Lead,
/// else its Foreman.
fn lead_of(crew: &Crew, members: &[&Employee]) -> Option<i64> {
    let named = crew.foreman.trim().to_lowercase();
    members
        .iter()
        .find(|e| !named.is_empty() && e.name.trim().to_lowercase() == named)
        .or_else(|| members.iter().find(|e| e.role == "Lead"))
        .or_else(|| members.iter().find(|e| e.role == "Foreman"))
        .map(|e| e.id)
}

/// Lay out one day: a card per active Rough-In crew (lead first, then by role
/// and name), then Field Supervisors, then people out that day grouped by
/// status, then anyone else not on a crew.
pub fn day_cards(
    employees: &[Employee],
    crews: &[Crew],
    vehicles: &[Vehicle],
    records: &HashMap<(i64, NaiveDate), AttendanceEntry>,
    day: NaiveDate,
) -> Vec<DayCard> {
    let role_rank = |r: &str| MEMBER_ROLES.iter().position(|m| *m == r).unwrap_or(MEMBER_ROLES.len());
    let staff: Vec<&Employee> = employees
        .iter()
        .filter(|e| e.staff == Board::RoughIn && (e.active || records.contains_key(&(e.id, day))))
        .collect();
    let mut crew_list: Vec<&Crew> = crews.iter().filter(|c| c.board == Board::RoughIn && c.active).collect();
    crew_list.sort_by_key(|c| c.name.to_lowercase());

    let mut cards = Vec::new();
    for c in &crew_list {
        let mut members: Vec<&Employee> = staff.iter().copied().filter(|e| e.crew_id == Some(c.id)).collect();
        let lead = lead_of(c, &members);
        members.sort_by_key(|e| (Some(e.id) != lead, role_rank(&e.role), e.name.to_lowercase()));
        let vehicle = c.vehicle_id.and_then(|id| vehicles.iter().find(|v| v.id == id));
        cards.push(DayCard {
            title: c.name.clone(),
            crew: Some(c.id),
            sub: match vehicle {
                Some(v) => format!("🚚 {}  {}", v.label(), v.description()).trim().to_string(),
                None => c.notes.lines().next().unwrap_or("").trim().to_string(),
            },
            vehicle_warning: vehicle.and_then(|v| match v.status.as_str() {
                "In Shop" => Some(match v.info_date(SHOP_PICKUP) {
                    Some(p) => format!("Truck in shop (pickup {})", p.format("%b %-d")),
                    None => "Truck in shop".to_string(),
                }),
                "Out of Service" => Some("Truck out of service".to_string()),
                _ => None,
            }),
            status: None,
            people: members.iter().map(|e| (e.id, Some(e.id) == lead)).collect(),
        });
    }

    // Everyone not on one of those crews.
    let mut loose: Vec<&Employee> = staff
        .iter()
        .copied()
        .filter(|e| !e.crew_id.is_some_and(|id| crew_list.iter().any(|c| c.id == id)))
        .collect();
    loose.sort_by_key(|e| e.name.to_lowercase());
    let status_of = |e: &Employee| records.get(&(e.id, day)).map(|r| r.status.as_str());
    let mut group = |title: &str, status: Option<&str>, people: Vec<i64>| {
        if !people.is_empty() {
            cards.push(DayCard {
                title: title.to_string(),
                crew: None,
                sub: String::new(),
                vehicle_warning: None,
                status: status.map(str::to_string),
                people: people.into_iter().map(|id| (id, false)).collect(),
            });
        }
    };
    let is_out = |e: &Employee| status_of(e).is_some_and(|s| NOT_WORKING.contains(&s));
    group(
        "Field Supervisors",
        None,
        loose.iter().filter(|e| e.role == "Field Supervisor").map(|e| e.id).collect(),
    );
    for (status, _, _) in ATTENDANCE {
        if NOT_WORKING.contains(status) {
            group(
                status,
                Some(status),
                loose
                    .iter()
                    .filter(|e| e.role != "Field Supervisor" && status_of(e) == Some(*status))
                    .map(|e| e.id)
                    .collect(),
            );
        }
    }
    group(
        "Unassigned",
        None,
        loose.iter().filter(|e| e.role != "Field Supervisor" && !is_out(e)).map(|e| e.id).collect(),
    );
    cards
}

/// The Day view: crew cards in a grid, like the paper attendance sheet.
fn day_board(
    ui: &mut egui::Ui,
    app: &SchedulerApp,
    records: &HashMap<(i64, NaiveDate), AttendanceEntry>,
    day: NaiveDate,
    rough_crews: &[(i64, String, Color32)],
    acts: &mut Vec<Act>,
) {
    const CARD_W: f32 = 240.0;
    const GAP: f32 = 10.0;
    let cards = day_cards(&app.employees, &app.crews, &app.vehicles, records, day);
    if cards.is_empty() {
        ui.add_space(20.0);
        ui.weak("No Rough-In crews or employees yet.");
        return;
    }
    let dark = ui.visuals().dark_mode;
    let lead_red = if dark { Color32::from_rgb(240, 95, 95) } else { Color32::from_rgb(205, 30, 30) };
    let cols = (((ui.available_width() + GAP) / (CARD_W + GAP)).floor() as usize).max(1);
    let editable = app.can_edit();
    // Names are for clicking and dragging, not selecting text.
    ui.style_mut().interaction.selectable_labels = false;

    ui.add_space(4.0);
    for row in cards.chunks(cols) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for card in row {
                let card_resp = egui::Frame::group(ui.style()).inner_margin(8.0).show(ui, |ui| ui.vertical(|ui| {
                    ui.set_width(CARD_W - 16.0);
                    ui.spacing_mut().item_spacing.x = 6.0;
                    // Title: crew color swatch + name, or the group's status color.
                    ui.horizontal(|ui| {
                        let crew = card.crew.and_then(|id| app.crews.iter().find(|c| c.id == id));
                        let mut title = RichText::new(&card.title).strong().size(15.0);
                        if let Some(c) = crew {
                            let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), egui::Sense::hover());
                            ui.painter().rect_filled(r, 3.0, c.color32());
                        } else if let Some(s) = &card.status {
                            title = title.color(status_color(s));
                        } else {
                            title = title.color(ui.visuals().hyperlink_color);
                        }
                        ui.add(egui::Label::new(title).truncate());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.weak(card.people.len().to_string());
                        });
                    });
                    if !card.sub.is_empty() {
                        ui.add(egui::Label::new(RichText::new(&card.sub).small().weak()).truncate());
                    }
                    if let Some(w) = &card.vehicle_warning {
                        ui.add(
                            egui::Label::new(RichText::new(format!("⚠ {w}")).small().color(Color32::from_rgb(210, 50, 50)))
                                .truncate(),
                        );
                    }
                    ui.separator();
                    if card.people.is_empty() {
                        ui.weak("No one on this crew");
                    }
                    for (id, is_lead) in &card.people {
                        let Some(e) = app.employees.iter().find(|e| e.id == *id) else { continue };
                        ui.horizontal(|ui| {
                            let entry = records.get(&(e.id, day));
                            let chip = match entry {
                                Some(en) => {
                                    let mut t = status_short(&en.status).to_string();
                                    if !en.note.trim().is_empty() {
                                        t.push_str(" •");
                                    }
                                    Button::new(RichText::new(t).small().strong().color(Color32::WHITE))
                                        .fill(status_color(&en.status))
                                }
                                None => Button::new(RichText::new("–").small().weak()),
                            };
                            let tip = match entry {
                                Some(en) if !en.note.trim().is_empty() => format!("{}: {}", en.status, en.note),
                                Some(en) => en.status.clone(),
                                None => "Not recorded — click to record".to_string(),
                            };
                            let chip = ui
                                .add(chip.min_size(vec2(44.0, 20.0)).sense(egui::Sense::click_and_drag()))
                                .on_hover_text(tip);
                            let mut name = RichText::new(&e.name);
                            if *is_lead {
                                name = name.color(lead_red).strong();
                            }
                            if !e.active {
                                name = name.italics().weak();
                            }
                            let label = ui
                                .with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if !e.role.is_empty() {
                                        ui.label(RichText::new(&e.role).small().weak());
                                    }
                                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                        ui.add(egui::Label::new(name).truncate().sense(egui::Sense::click_and_drag()))
                                    })
                                    .inner
                                })
                                .inner
                                .on_hover_text("Click to record attendance · drag to another crew · right-click for more");
                            if chip.clicked() || label.clicked() {
                                acts.push(Act::Edit(e.id, e.name.clone(), day));
                            }
                            // Drag a person (by name or chip) onto another card to move them.
                            if editable && (chip.drag_started() || label.drag_started()) {
                                egui::DragAndDrop::set_payload(
                                    ui.ctx(),
                                    DraggedPerson { id: e.id, name: e.name.clone(), from: card.crew },
                                );
                            }
                            if chip.hovered() || label.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                            }
                            let menu = |ui: &mut egui::Ui| {
                                ui.label(RichText::new("Move to crew").weak());
                                for (cid, cname, color) in rough_crews {
                                    ui.horizontal(|ui| {
                                        let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), egui::Sense::hover());
                                        ui.painter().rect_filled(r, 2.0, *color);
                                        let current = e.crew_id == Some(*cid);
                                        if ui.selectable_label(current, cname.as_str()).clicked() {
                                            if !current {
                                                acts.push(Act::MoveCrew(e.id, Some(*cid)));
                                            }
                                            ui.close_menu();
                                        }
                                    });
                                }
                                ui.separator();
                                if ui.selectable_label(e.crew_id.is_none(), "No crew").clicked() {
                                    if e.crew_id.is_some() {
                                        acts.push(Act::MoveCrew(e.id, None));
                                    }
                                    ui.close_menu();
                                }
                            };
                            label.context_menu(menu);
                        });
                    }
                }))
                .response;

                // Drop target: a crew card puts them on that crew; any other card
                // means "no crew".
                if let Some(p) = card_resp.dnd_hover_payload::<DraggedPerson>() {
                    let color = if p.from == card.crew {
                        ui.visuals().weak_text_color()
                    } else {
                        ui.visuals().selection.stroke.color
                    };
                    ui.painter().rect_stroke(card_resp.rect.expand(1.0), 6.0, egui::Stroke::new(2.5_f32, color));
                }
                if let Some(p) = card_resp.dnd_release_payload::<DraggedPerson>() {
                    if p.from != card.crew {
                        acts.push(Act::MoveCrew(p.id, card.crew));
                    }
                }
            }
        });
        ui.add_space(GAP);
    }

    // While dragging: follow the pointer with the person's name.
    if let Some(p) = egui::DragAndDrop::payload::<DraggedPerson>(ui.ctx()) {
        if let Some(pos) = ui.ctx().pointer_interact_pos() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("att_drag")));
            let galley = painter.layout_no_wrap(
                p.name.clone(),
                egui::FontId::proportional(14.0),
                ui.visuals().strong_text_color(),
            );
            let rect = egui::Rect::from_min_size(pos + vec2(12.0, 8.0), galley.size() + vec2(16.0, 8.0));
            painter.rect(
                rect,
                5.0,
                ui.visuals().window_fill,
                egui::Stroke::new(1.5_f32, ui.visuals().selection.stroke.color),
            );
            painter.galley(rect.min + vec2(8.0, 4.0), galley, Color32::PLACEHOLDER);
        }
    }
}

/// What's being dragged on the Day board.
struct DraggedPerson {
    id: i64,
    name: String,
    /// The crew card they were dragged from (None = a non-crew card).
    from: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emp(id: i64, name: &str, role: &str, crew: Option<i64>) -> Employee {
        Employee {
            id,
            name: name.into(),
            role: role.into(),
            staff: Board::RoughIn,
            crew_id: crew,
            ..Employee::new()
        }
    }

    #[test]
    fn day_board_puts_the_lead_first_and_groups_everyone_else() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 13).unwrap();
        let crew = Crew {
            id: 1,
            name: "Sean Pierce".into(),
            foreman: "Sean Pierce".into(),
            phone: String::new(),
            color: [0, 0, 0],
            active: true,
            notes: "Truck-24\nmore".into(),
            board: Board::RoughIn,
            member_ids: vec![],
            vehicle_id: None,
        };
        let employees = vec![
            emp(1, "Kurt Ames", "Helper", Some(1)),
            emp(2, "Sean Pierce", "Lead", Some(1)),
            emp(3, "Alex Moreno", "Journeyman", Some(1)),
            emp(4, "Erik Bauer", "Field Supervisor", None),
            emp(5, "Seth Navarro", "Helper", None),
            emp(6, "Erik Lund", "Lead", None),
            emp(7, "Noah Fenwick", "Helper", None),
        ];
        let mut records = HashMap::new();
        let mark = |s: &str| AttendanceEntry { status: s.into(), note: String::new() };
        records.insert((5, day), mark("Off"));
        records.insert((6, day), mark("Absent"));
        let cards = day_cards(&employees, &[crew.clone()], &[], &records, day);
        let titles: Vec<&str> = cards.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["Sean Pierce", "Field Supervisors", "Absent", "Off", "Unassigned"]);
        assert_eq!(cards[0].sub, "Truck-24");
        assert_eq!(cards[0].people, [(2, true), (3, false), (1, false)]);
        assert_eq!(cards[1].people, [(4, false)]);
        assert_eq!(cards[2].people, [(6, false)]);
        assert_eq!(cards[3].people, [(5, false)]);
        assert_eq!(cards[4].people, [(7, false)]);
        assert_eq!(cards[0].vehicle_warning, None);

        // With a linked truck, the card shows that vehicle, and warns while it's in the shop.
        let mut truck = Vehicle::new();
        truck.id = 9;
        truck.unit = "24".into();
        truck.year = "2021".into();
        truck.make = "Chevy".into();
        truck.model = "Silverado 2500".into();
        let linked = Crew { vehicle_id: Some(9), ..crew };
        let cards = day_cards(&employees, &[linked.clone()], &[truck.clone()], &records, day);
        assert_eq!(cards[0].sub, "🚚 #24  2021 Chevy Silverado 2500");
        assert_eq!(cards[0].vehicle_warning, None);
        truck.status = "In Shop".into();
        truck.set_info_date(SHOP_PICKUP, NaiveDate::from_ymd_opt(2026, 11, 20));
        let cards = day_cards(&employees, &[linked], &[truck], &records, day);
        assert_eq!(cards[0].vehicle_warning.as_deref(), Some("Truck in shop (pickup Nov 20)"));
    }

    #[test]
    fn not_working_takes_them_off_their_crew() {
        for s in ["PTO", "Off", "Absent", "Sick"] {
            assert!(takes_off_crew(s), "{s}");
        }
        for s in ["Present", "Late"] {
            assert!(!takes_off_crew(s), "{s} keeps them on their crew");
        }
    }
}
