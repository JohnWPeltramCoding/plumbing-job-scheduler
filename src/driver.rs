//! The Driver screen on an employee: their driver's license (status, points,
//! expiry, last MVR check) and their accidents, moving violations and parking
//! tickets.

use std::collections::HashMap;

use chrono::{Duration, NaiveDate};
use eframe::egui::{self, vec2, Button, CollapsingHeader, Color32, ComboBox, DragValue, Grid, RichText, ScrollArea, TextEdit};

use crate::app::{today, SchedulerApp};
use crate::model::*;
use crate::vehicles::{fmt_date, money_field, optional_date};

const WARN: Color32 = Color32::from_rgb(210, 50, 50);
const UNSAVED: Color32 = Color32::from_rgb(230, 150, 30);

/// How far back moving-violation points are added up for the summary.
const POINT_YEARS: i64 = 3;

#[derive(Default)]
pub struct DriverState {
    /// License being edited (for the selected employee).
    license: Option<DriverLicense>,
    /// Incident open in the editor window.
    incident: Option<Incident>,
    confirm_delete: bool,
}

impl DriverState {
    /// True when the license form has changes that aren't saved.
    pub fn license_dirty(&self, licenses: &HashMap<i64, DriverLicense>) -> bool {
        match &self.license {
            Some(l) => *l != licenses.get(&l.employee_id).cloned().unwrap_or_else(|| DriverLicense::new(l.employee_id)),
            None => false,
        }
    }
}

pub enum DriverAction {
    SaveLicense(DriverLicense),
    DiscardLicense,
    NewIncident(&'static str),
    EditIncident(Incident),
    SaveIncident(Incident),
    DeleteIncident(i64),
}

/// "Fine" for tickets, "Damage cost" for accidents.
fn amount_label(kind: &str) -> &'static str {
    if kind == "Accident" {
        "Damage cost"
    } else {
        "Fine"
    }
}

fn singular(kind: &str) -> String {
    kind.to_lowercase()
}

/// Moving-violation points from the last POINT_YEARS years.
pub fn recent_points(incidents: &[Incident], employee: i64, today: NaiveDate) -> i32 {
    let since = today - Duration::days(365 * POINT_YEARS);
    incidents
        .iter()
        .filter(|i| i.employee_id == employee && i.kind == "Moving Violation" && i.date.is_some_and(|d| d >= since))
        .map(|i| i.points)
        .sum()
}

/// The Driver screen for one employee.
#[allow(clippy::too_many_arguments)]
pub fn page(
    ui: &mut egui::Ui,
    emp: &Employee,
    licenses: &HashMap<i64, DriverLicense>,
    incidents: &[Incident],
    vehicles: &[Vehicle],
    st: &mut DriverState,
    editable: bool,
    actions: &mut Vec<DriverAction>,
) {
    let today = today();
    if emp.id == 0 {
        ui.weak("Save the employee first, then add their driver information here.");
        return;
    }
    if st.license.as_ref().map(|l| l.employee_id) != Some(emp.id) {
        st.license = Some(licenses.get(&emp.id).cloned().unwrap_or_else(|| DriverLicense::new(emp.id)));
    }
    let saved = licenses.get(&emp.id).cloned().unwrap_or_else(|| DriverLicense::new(emp.id));
    let has_record = licenses.contains_key(&emp.id);
    let mine: Vec<&Incident> = incidents.iter().filter(|i| i.employee_id == emp.id).collect();

    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        // Summary line.
        let count = |k: &str| mine.iter().filter(|i| i.kind == k).count();
        let at_fault = mine.iter().filter(|i| i.kind == "Accident" && i.at_fault).count();
        let fines: f64 = mine.iter().filter(|i| i.kind != "Accident").filter_map(|i| i.amount).sum();
        ui.label(
            RichText::new(format!(
                "{} accident{} ({} at fault)  ·  {} moving violation{}  ·  {} parking ticket{}  ·  {} in fines",
                count("Accident"),
                if count("Accident") == 1 { "" } else { "s" },
                at_fault,
                count("Moving Violation"),
                if count("Moving Violation") == 1 { "" } else { "s" },
                count("Parking Ticket"),
                if count("Parking Ticket") == 1 { "" } else { "s" },
                fmt_money(fines)
            ))
            .weak(),
        );
        ui.add_space(6.0);

        // ---------------------------------------------------------- license
        let draft = st.license.as_mut().unwrap();
        let dirty = *draft != saved;
        let mut header = RichText::new("Driver's License").strong().size(15.0);
        if !has_record && !dirty {
            header = header.italics();
        }
        CollapsingHeader::new(header).id_salt("drv_license").default_open(true).show(ui, |ui| {
            ui.add_enabled_ui(editable, |ui| {
                Grid::new("drv_license_grid").num_columns(2).spacing([12.0, 8.0]).min_col_width(150.0).show(ui, |ui| {
                    ui.label("Status");
                    ui.horizontal(|ui| {
                        for (s, rgb) in LICENSE_STATUSES {
                            let text = RichText::new(*s).color(Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                            if ui.selectable_label(draft.status == *s, text).clicked() {
                                draft.status = s.to_string();
                            }
                        }
                    });
                    ui.end_row();

                    ui.label("License #");
                    ui.add(TextEdit::singleline(&mut draft.number).desired_width(200.0));
                    ui.end_row();

                    ui.label("State");
                    ui.add(TextEdit::singleline(&mut draft.state).desired_width(50.0).char_limit(2));
                    ui.end_row();

                    ui.label("Class / CDL");
                    ui.horizontal(|ui| {
                        ui.add(TextEdit::singleline(&mut draft.class).desired_width(90.0).hint_text("e.g. C, CDL-A"));
                        ui.label("Endorsements");
                        ui.add(TextEdit::singleline(&mut draft.endorsements).desired_width(160.0));
                    });
                    ui.end_row();

                    ui.label("Expires");
                    optional_date(ui, "drv_expires", &mut draft.expires, today, true);
                    ui.end_row();

                    ui.label("Points");
                    ui.horizontal(|ui| {
                        ui.add(DragValue::new(&mut draft.points).range(0..=99).speed(0.1));
                        ui.weak("on the license, per the state / MVR");
                        let recent = recent_points(incidents, emp.id, today);
                        if recent > 0 {
                            ui.label(format!(
                                "·  {recent} point{} from moving violations recorded here in the last {POINT_YEARS} years",
                                if recent == 1 { "" } else { "s" }
                            ));
                        }
                    });
                    ui.end_row();

                    ui.label("Last MVR check");
                    optional_date(ui, "drv_mvr", &mut draft.last_checked, today, false);
                    ui.end_row();

                    ui.label("Notes");
                    ui.add(TextEdit::multiline(&mut draft.notes).desired_rows(2).desired_width(440.0));
                    ui.end_row();
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.add_enabled(dirty, Button::new(RichText::new("💾 Save License").strong())).clicked() {
                        actions.push(DriverAction::SaveLicense(draft.clone()));
                    }
                    if ui.add_enabled(dirty, Button::new("Discard Changes")).clicked() {
                        actions.push(DriverAction::DiscardLicense);
                    }
                    if dirty {
                        ui.colored_label(UNSAVED, "(unsaved license changes)");
                    } else if !has_record {
                        ui.weak("No license on file yet.");
                    }
                });
            });
        });

        // ---------------------------------------------------- incidents
        for (kind, heading) in INCIDENT_KINDS {
            let list: Vec<&&Incident> = mine.iter().filter(|i| i.kind == *kind).collect();
            ui.add_space(8.0);
            let total: f64 = list.iter().filter_map(|i| i.amount).sum();
            let mut title = format!("{heading} ({})", list.len());
            if total > 0.0 {
                title.push_str(&format!("  ·  {}", fmt_money(total)));
            }
            if *kind == "Moving Violation" {
                let pts: i32 = list.iter().map(|i| i.points).sum();
                if pts > 0 {
                    title.push_str(&format!("  ·  {pts} point{}", if pts == 1 { "" } else { "s" }));
                }
            }
            CollapsingHeader::new(RichText::new(title).strong().size(15.0))
                .id_salt(("drv_kind", *kind))
                .default_open(true)
                .show(ui, |ui| {
                    if ui.add_enabled(editable, Button::new(format!("+ Add {}", singular(kind)))).clicked() {
                        actions.push(DriverAction::NewIncident(kind));
                    }
                    if list.is_empty() {
                        ui.weak(format!("No {} recorded.", heading.to_lowercase()));
                        return;
                    }
                    ScrollArea::horizontal().id_salt(("drv_scroll", *kind)).show(ui, |ui| {
                        Grid::new(("drv_grid", *kind)).striped(true).spacing([14.0, 5.0]).show(ui, |ui| {
                            let mut heads = vec!["Date", "Vehicle", "Location", "What happened", "Ref #", amount_label(kind)];
                            if *kind == "Moving Violation" {
                                heads.push("Points");
                            }
                            if *kind == "Accident" {
                                heads.push("At fault");
                            }
                            heads.extend(["Status", ""]);
                            for h in heads {
                                ui.label(RichText::new(h).strong());
                            }
                            ui.end_row();
                            for i in &list {
                                ui.label(fmt_date(i.date));
                                let veh = i.vehicle_id.and_then(|id| vehicles.iter().find(|v| v.id == id));
                                ui.label(veh.map(|v| v.label()).unwrap_or_default());
                                ui.label(&i.location);
                                ui.scope(|ui| {
                                    ui.set_min_width(160.0);
                                    ui.set_max_width(260.0);
                                    ui.add(egui::Label::new(&i.description).truncate()).on_hover_text(&i.description);
                                });
                                ui.label(&i.reference);
                                ui.label(i.amount.map(fmt_money).unwrap_or_default());
                                if *kind == "Moving Violation" {
                                    ui.label(i.points.to_string());
                                }
                                if *kind == "Accident" {
                                    if i.at_fault {
                                        ui.colored_label(WARN, "Yes");
                                    } else {
                                        ui.label("No");
                                    }
                                }
                                if i.status == "Open" {
                                    ui.colored_label(UNSAVED, &i.status);
                                } else {
                                    ui.label(&i.status);
                                }
                                if ui.add_enabled(editable, Button::new("Edit").small()).clicked() {
                                    actions.push(DriverAction::EditIncident((**i).clone()));
                                }
                                ui.end_row();
                            }
                        });
                    });
                });
        }
    });
}

/// The editor window for one accident / violation / ticket.
pub fn incident_window(ctx: &egui::Context, st: &mut DriverState, vehicles: &[Vehicle], who: &str, actions: &mut Vec<DriverAction>) {
    let Some(i) = st.incident.as_mut() else { return };
    let today = today();
    let (mut open, mut close) = (true, false);
    let title = format!("{} {} · {who}", if i.id == 0 { "New" } else { "Edit" }, singular(&i.kind));
    egui::Window::new(title)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            Grid::new("incident_edit").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Date");
                optional_date(ui, "inc_date", &mut i.date, today, false);
                ui.end_row();

                ui.label("Vehicle");
                let current = i.vehicle_id.and_then(|id| vehicles.iter().find(|v| v.id == id));
                let text = current.map(|v| format!("{}  {}", v.label(), v.description())).unwrap_or_else(|| "None / personal".into());
                ComboBox::from_id_salt("inc_vehicle").selected_text(text).width(280.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut i.vehicle_id, None, "None / personal");
                    for v in vehicles {
                        ui.selectable_value(&mut i.vehicle_id, Some(v.id), format!("{}  {}", v.label(), v.description()));
                    }
                });
                ui.end_row();

                ui.label("Location");
                ui.add(TextEdit::singleline(&mut i.location).desired_width(300.0));
                ui.end_row();

                ui.label("What happened");
                ui.add(TextEdit::multiline(&mut i.description).desired_rows(3).desired_width(360.0));
                ui.end_row();

                ui.label(match i.kind.as_str() {
                    "Accident" => "Claim / report #",
                    _ => "Citation #",
                });
                ui.add(TextEdit::singleline(&mut i.reference).desired_width(200.0));
                ui.end_row();

                ui.label(amount_label(&i.kind));
                ui.horizontal(|ui| {
                    money_field(ui, &mut i.amount);
                });
                ui.end_row();

                if i.kind == "Moving Violation" {
                    ui.label("Points");
                    ui.add(DragValue::new(&mut i.points).range(0..=20).speed(0.1));
                    ui.end_row();
                }
                if i.kind == "Accident" {
                    ui.label("At fault");
                    ui.checkbox(&mut i.at_fault, "Our driver was at fault");
                    ui.end_row();
                }

                ui.label("Status");
                ui.horizontal(|ui| {
                    for s in INCIDENT_STATUSES {
                        ui.selectable_value(&mut i.status, s.to_string(), *s);
                    }
                });
                ui.end_row();

                ui.label("Notes");
                ui.add(TextEdit::multiline(&mut i.notes).desired_rows(2).desired_width(360.0));
                ui.end_row();
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new("💾 Save").strong()).clicked() {
                    actions.push(DriverAction::SaveIncident(i.clone()));
                    close = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
                if i.id > 0 {
                    if st.confirm_delete {
                        if ui.button(RichText::new("Confirm delete").color(WARN)).clicked() {
                            actions.push(DriverAction::DeleteIncident(i.id));
                            close = true;
                        }
                    } else if ui.button("🗑 Delete").clicked() {
                        st.confirm_delete = true;
                    }
                }
            });
        });
    if close || !open {
        st.incident = None;
        st.confirm_delete = false;
    }
}

/// Carries out the Driver screen's actions for the employee `emp_id`.
pub fn apply(app: &mut SchedulerApp, staff: usize, emp_id: i64, actions: Vec<DriverAction>) {
    let today = today();
    for a in actions {
        match a {
            DriverAction::SaveLicense(l) => match app.db.save_license(&l) {
                Ok(()) => {
                    app.reload();
                    app.employees_state[staff].driver.license = None;
                    app.set_status("Driver's license saved.");
                }
                Err(e) => app.set_error(format!("Could not save the license: {e}")),
            },
            DriverAction::DiscardLicense => app.employees_state[staff].driver.license = None,
            DriverAction::NewIncident(kind) => {
                let st = &mut app.employees_state[staff].driver;
                st.incident = Some(Incident::new(emp_id, kind, today));
                st.confirm_delete = false;
            }
            DriverAction::EditIncident(i) => {
                let st = &mut app.employees_state[staff].driver;
                st.incident = Some(i);
                st.confirm_delete = false;
            }
            DriverAction::SaveIncident(i) => match app.db.save_incident(&i) {
                Ok(_) => {
                    app.reload();
                    app.set_status(format!("Saved {}.", singular(&i.kind)));
                }
                Err(e) => app.set_error(format!("Could not save: {e}")),
            },
            DriverAction::DeleteIncident(id) => match app.db.delete_incident(id) {
                Ok(()) => {
                    app.reload();
                    app.set_status("Deleted.");
                }
                Err(e) => app.set_error(format!("Could not delete: {e}")),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn license_and_incidents_round_trip() {
        let db = Db::open_in_memory().unwrap();
        let mut e = Employee::new();
        e.name = "Isaiah Marsh".into();
        let id = db.save_employee(&e).unwrap();

        let mut l = DriverLicense::new(id);
        l.number = "BONTEIJ123".into();
        l.state = "wa".into();
        l.points = 3;
        l.expires = Some(d("2027-02-01"));
        db.save_license(&l).unwrap();
        l.status = "Suspended".into();
        db.save_license(&l).unwrap(); // saving again updates the same record
        let got = db.load_licenses().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!((got[&id].state.as_str(), got[&id].points, got[&id].status.as_str()), ("WA", 3, "Suspended"));
        assert_eq!(got[&id].warning(d("2026-11-13")).as_deref(), Some("License suspended"));
        let mut ok = got[&id].clone();
        ok.status = "Valid".into();
        assert_eq!(ok.warning(d("2027-01-20")).as_deref(), Some("License expires Feb 1"));
        assert_eq!(ok.warning(d("2026-11-13")), None);

        let mut v = Incident::new(id, "Moving Violation", d("2026-03-02"));
        v.points = 2;
        v.amount = Some(136.0);
        let vid = db.save_incident(&v).unwrap();
        let mut old = Incident::new(id, "Moving Violation", d("2021-01-05"));
        old.points = 4;
        db.save_incident(&old).unwrap();
        let mut acc = Incident::new(id, "Accident", d("2026-06-10"));
        acc.at_fault = true;
        db.save_incident(&acc).unwrap();
        let all = db.load_incidents().unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].kind, "Accident", "newest first");
        // Only violations from the last 3 years count toward recent points.
        assert_eq!(recent_points(&all, id, d("2026-11-13")), 2);

        db.delete_incident(vid).unwrap();
        assert_eq!(db.load_incidents().unwrap().len(), 2);
        // Deleting the employee removes their driver records.
        db.delete_employee(id).unwrap();
        assert!(db.load_licenses().unwrap().is_empty());
        assert!(db.load_incidents().unwrap().is_empty());
    }
}
