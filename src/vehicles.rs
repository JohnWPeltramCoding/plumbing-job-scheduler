//! Vehicles: the company fleet. A list on the left (with tab, insurance and
//! service reminders) and the selected vehicle's details on the right. The
//! details follow the fleet management log: IDs and passes, driver and
//! supervisor, mileage, oil changes and service history.

use chrono::NaiveDate;
use eframe::egui::{
    self, vec2, Align, Button, CollapsingHeader, Color32, DragValue, Grid, Layout, RichText, ScrollArea,
    Sense, TextEdit,
};
use egui_extras::DatePickerButton;

use crate::app::{today, SchedulerApp};
use crate::fleet_import::{plan_mileage, MileagePlan};
use crate::model::*;

const OVERDUE: Color32 = Color32::from_rgb(210, 50, 50);
const DUE_SOON: Color32 = Color32::from_rgb(220, 140, 20);
const LINKED: Color32 = Color32::from_rgb(39, 150, 80);

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum VehFilter {
    #[default]
    All,
    /// Index into VEHICLE_STATUSES.
    Status(usize),
    Reminders,
}

/// How the vehicle list is ordered (remembered between sessions).
#[derive(Clone, Copy, PartialEq, Eq)]
enum VehSort {
    Truck,
    /// By the driver's last name, then first; vehicles without a driver last.
    Driver,
}

/// Sort key for a driver name: (last, rest), lowercased. "Isaiah Marsh" ->
/// ("marsh", "isaiah"); "Marsh, Isaiah" is understood too.
fn driver_key(name: &str) -> (String, String) {
    let name = name.trim().to_lowercase();
    if let Some((last, first)) = name.split_once(',') {
        return (last.trim().to_string(), first.trim().to_string());
    }
    match name.rsplit_once(' ') {
        Some((first, last)) => (last.trim().to_string(), first.trim().to_string()),
        None => (name, String::new()),
    }
}

#[derive(Default)]
pub struct VehiclesState {
    selected: Option<i64>,
    /// Working copy being edited (id 0 = new vehicle).
    draft: Option<Vehicle>,
    filter: VehFilter,
    /// List order; None until read from the saved setting.
    sort: Option<VehSort>,
    search: String,
    confirm_delete: bool,
    /// Shown after a mileage import: (file name, what happened).
    mileage_summary: Option<(String, MileagePlan)>,
    /// A repair open in the editor window.
    repair_edit: Option<Repair>,
    confirm_delete_repair: bool,
}

enum VehAction {
    Select(i64),
    New,
    Save,
    Revert,
    Delete(i64),
    ImportMileage,
    /// Put a vehicle on a crew (or take it off), saved straight away.
    SetCrewVehicle(i64, Option<i64>),
    NewRepair,
    EditRepair(Repair),
    SaveRepair(Repair),
    DeleteRepair(i64),
    /// Save the repair report: one vehicle, or all (None).
    ExportRepairs(Option<i64>),
}

/// The window listing what a mileage import did, especially trucks that
/// didn't match.
fn mileage_summary_window(ctx: &egui::Context, summary: &mut Option<(String, MileagePlan)>) {
    let Some((file, plan)) = summary.as_ref() else { return };
    let mut open = true;
    let mut close = false;
    egui::Window::new("Mileage Import")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(520.0)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.label(RichText::new(file).weak());
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!("Updated mileage on {} vehicle{}.", plan.updates.len(), plural(plan.updates.len() as i64)))
                    .strong()
                    .size(15.0),
            );
            let not_matched = plan.unmatched.len();
            if not_matched == 0 {
                ui.colored_label(LINKED, "✔ Every vehicle in the report matched a vehicle here.");
            } else {
                ui.colored_label(
                    OVERDUE,
                    RichText::new(format!(
                        "{not_matched} vehicle{} in the report did not match any vehicle here:",
                        plural(not_matched as i64)
                    ))
                    .strong(),
                );
            }
            ScrollArea::vertical().max_height(420.0).auto_shrink([false, true]).show(ui, |ui| {
                for row in &plan.unmatched {
                    ui.label(format!("   •  {row}"));
                }
                if !plan.no_mileage.is_empty() {
                    ui.add_space(6.0);
                    ui.colored_label(DUE_SOON, format!("No mileage in the report ({}), left unchanged:", plan.no_mileage.len()));
                    for row in &plan.no_mileage {
                        ui.label(format!("   •  {row}"));
                    }
                }
                if !plan.duplicates.is_empty() {
                    ui.add_space(6.0);
                    for d in &plan.duplicates {
                        ui.colored_label(DUE_SOON, format!("⚠ {d}"));
                    }
                }
                ui.add_space(6.0);
                let lower = plan.changes.iter().filter(|(_, old, new)| new < old).count();
                CollapsingHeader::new(format!("Updated vehicles ({})", plan.changes.len()))
                    .id_salt("mileage_updated_list")
                    .show(ui, |ui| {
                        if lower > 0 {
                            ui.weak(format!("{lower} reading{} lower than the mileage recorded before (shown in orange).", plural(lower as i64)));
                        }
                        Grid::new("mileage_changes").striped(true).spacing([16.0, 4.0]).show(ui, |ui| {
                            ui.label(RichText::new("Vehicle").strong());
                            ui.label(RichText::new("Before").strong());
                            ui.label(RichText::new("Now (GPS)").strong());
                            ui.end_row();
                            for (name, old, new) in &plan.changes {
                                ui.label(name);
                                ui.label(RichText::new(format!("{} mi", fmt_miles(*old))).weak());
                                let text = RichText::new(format!("{} mi", fmt_miles(*new)));
                                ui.label(if new < old { text.color(DUE_SOON) } else { text });
                                ui.end_row();
                            }
                        });
                    });
            });
            ui.add_space(8.0);
            ui.vertical_centered(|ui| {
                if ui.button("   OK   ").clicked() {
                    close = true;
                }
            });
        });
    if !open || close {
        *summary = None;
    }
}

fn fmt_miles(n: i64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Equal apart from blank extra details (which aren't stored anyway).
fn same(a: &Vehicle, b: &Vehicle) -> bool {
    let strip = |v: &Vehicle| {
        let mut v = v.clone();
        v.info.retain(|_, s| !s.trim().is_empty());
        v
    };
    strip(a) == strip(b)
}

fn status_badge(ui: &mut egui::Ui, v: &Vehicle) {
    let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
    ui.painter().circle_filled(r.center(), 5.0, v.status_color());
}

fn plural(n: i64) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// An optional date: a date picker plus Clear, or "Not set" plus Set.
/// A `deadline` is flagged when it's past or coming up; otherwise the note
/// says how long ago it was.
pub(crate) fn optional_date(ui: &mut egui::Ui, id: &str, value: &mut Option<NaiveDate>, today: NaiveDate, deadline: bool) {
    ui.horizontal(|ui| match value {
        Some(d) => {
            ui.add(DatePickerButton::new(d).id_salt(id));
            let days = (*d - today).num_days();
            if deadline {
                if days < 0 {
                    ui.colored_label(OVERDUE, format!("overdue by {} day{}", -days, plural(-days)));
                } else if days == 0 {
                    ui.colored_label(DUE_SOON, "due today");
                } else if days <= DUE_SOON_DAYS {
                    ui.colored_label(DUE_SOON, format!("due in {days} day{}", plural(days)));
                }
            } else if days < 0 {
                ui.weak(format!("{} day{} ago", -days, plural(-days)));
            }
            if ui.small_button("Clear").clicked() {
                *value = None;
            }
        }
        None => {
            ui.weak("Not set");
            if ui.small_button("Set date").clicked() {
                *value = Some(today);
            }
        }
    });
}

/// The Shop section, shown while a vehicle is In Shop: where it is, what's
/// being done, and the drop-off / repair time / cost / pickup estimates.
fn shop_section(ui: &mut egui::Ui, draft: &mut Vehicle, today: NaiveDate) {
    let frame = egui::Frame::group(ui.style())
        .fill(Color32::from_rgba_unmultiplied(230, 150, 30, 18))
        .stroke(egui::Stroke::new(1.0_f32, DUE_SOON));
    frame.show(ui, |ui| {
        CollapsingHeader::new(RichText::new("🔧 Shop").strong().size(15.0).color(DUE_SOON))
            .id_salt("veh_shop")
            .default_open(true)
            .show(ui, |ui| {
                Grid::new("veh_shop_grid").num_columns(2).spacing([12.0, 8.0]).min_col_width(150.0).show(ui, |ui| {
                    ui.label("Shop");
                    ui.add(
                        TextEdit::singleline(draft.info.entry(SHOP_NAME.into()).or_default())
                            .desired_width(260.0)
                            .hint_text("e.g. Sunset Chevrolet"),
                    );
                    ui.end_row();

                    ui.label("Work being done");
                    ui.add(
                        TextEdit::multiline(draft.info.entry(SHOP_WORK.into()).or_default())
                            .desired_rows(2)
                            .desired_width(520.0)
                            .hint_text("e.g. front brakes and rotors"),
                    );
                    ui.end_row();

                    ui.label("Drop-off date");
                    let mut d = draft.info_date(SHOP_DROP_OFF);
                    optional_date(ui, "veh_shop_drop", &mut d, today, false);
                    draft.set_info_date(SHOP_DROP_OFF, d);
                    ui.end_row();

                    ui.label("Est. time of repair");
                    ui.add(
                        TextEdit::singleline(draft.info.entry(SHOP_REPAIR_TIME.into()).or_default())
                            .desired_width(160.0)
                            .hint_text("e.g. 3 days"),
                    );
                    ui.end_row();

                    ui.label("Est. cost of repair");
                    ui.horizontal(|ui| {
                        let mut cost = parse_money(draft.get(SHOP_COST));
                        if money_field(ui, &mut cost) {
                            match cost {
                                Some(c) => draft.info.insert(SHOP_COST.into(), format!("{c:.2}")),
                                None => draft.info.remove(SHOP_COST),
                            };
                        }
                    });
                    ui.end_row();

                    ui.label("Est. pickup date");
                    ui.horizontal(|ui| {
                        let mut d = draft.info_date(SHOP_PICKUP);
                        optional_date(ui, "veh_shop_pickup", &mut d, today, true);
                        draft.set_info_date(SHOP_PICKUP, d);
                        if let (Some(drop), Some(pick)) = (draft.info_date(SHOP_DROP_OFF), draft.info_date(SHOP_PICKUP)) {
                            let days = (pick - drop).num_days();
                            if days >= 0 {
                                ui.weak(format!("({days} day{} in the shop)", plural(days)));
                            }
                        }
                    });
                    ui.end_row();

                    ui.label("Final cost");
                    ui.horizontal(|ui| {
                        let mut cost = parse_money(draft.get(SHOP_FINAL_COST));
                        if money_field(ui, &mut cost) {
                            match cost {
                                Some(c) => draft.info.insert(SHOP_FINAL_COST.into(), format!("{c:.2}")),
                                None => draft.info.remove(SHOP_FINAL_COST),
                            };
                        }
                        ui.weak("what it actually cost, when you know");
                    });
                    ui.end_row();
                });
                ui.add_space(4.0);
                ui.weak(
                    "This visit is in the Repair History below. When the vehicle comes back, set its status \
                     (e.g. Active) and save: the visit is closed with today's date, and these fields clear for next time.",
                );
                if ui
                    .small_button("Clear shop info")
                    .on_hover_text("Empty these shop fields, e.g. before the vehicle's next shop visit")
                    .clicked()
                {
                    for k in SHOP_KEYS {
                        draft.info.remove(*k);
                    }
                }
            });
    });
}

/// Total cost (final, else estimate) and total days in the shop.
fn repair_totals(list: &[&Repair], today: NaiveDate) -> (f64, i64) {
    (list.iter().filter_map(|r| r.cost()).sum(), list.iter().filter_map(|r| r.days(today)).sum())
}

/// "Nov 13, 2026 2:05 PM" from a stored "2026-11-13 14:05" stamp.
fn fmt_stamp(s: &str) -> String {
    match chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M") {
        Ok(t) if t.format("%H:%M").to_string() == "00:00" => t.format("%b %-d, %Y").to_string(),
        Ok(t) => t.format("%b %-d, %Y %-I:%M %p").to_string(),
        Err(_) => s.to_string(),
    }
}

pub(crate) fn fmt_date(d: Option<NaiveDate>) -> String {
    d.map(|d| d.format("%b %-d, %Y").to_string()).unwrap_or_default()
}

/// The vehicle's Repair History: every shop visit, newest first, with totals.
fn repair_history(ui: &mut egui::Ui, v: &Vehicle, repairs: &[Repair], today: NaiveDate, actions: &mut Vec<VehAction>) {
    let list: Vec<&Repair> = repairs.iter().filter(|r| r.vehicle_id == v.id).collect();
    let (total, days) = repair_totals(&list, today);
    let title = if list.is_empty() {
        "Repair History".to_string()
    } else {
        format!(
            "Repair History  ·  {} repair{}  ·  {}  ·  {} day{} in the shop",
            list.len(),
            plural(list.len() as i64),
            fmt_money(total),
            days,
            plural(days)
        )
    };
    CollapsingHeader::new(RichText::new(title).strong().size(15.0))
        .id_salt("veh_repairs")
        .default_open(true)
        .show(ui, |ui| {
            if v.id == 0 {
                ui.weak("Save the vehicle first.");
                return;
            }
            ui.horizontal(|ui| {
                if ui.button("+ Add past repair").on_hover_text("Record a repair from before the app tracked them").clicked() {
                    actions.push(VehAction::NewRepair);
                }
                if ui.button("⬇ Export this vehicle's repairs…").on_hover_text("Save a report (.csv, opens in Excel)").clicked() {
                    actions.push(VehAction::ExportRepairs(Some(v.id)));
                }
                if ui.button("⬇ Export all vehicles…").on_hover_text("Every vehicle's repairs in one report (.csv)").clicked() {
                    actions.push(VehAction::ExportRepairs(None));
                }
            });
            if list.is_empty() {
                ui.weak("No repairs yet. Setting the status to In Shop and saving starts one.");
                return;
            }
            ui.add_space(4.0);
            ScrollArea::horizontal().id_salt("veh_repairs_scroll").show(ui, |ui| {
                Grid::new("veh_repairs_grid").striped(true).spacing([14.0, 5.0]).show(ui, |ui| {
                    for h in ["Put in shop", "Shop", "Work", "Drop-off", "Back", "Days", "Est. time", "Est. cost", "Final cost", ""] {
                        ui.label(RichText::new(h).strong());
                    }
                    ui.end_row();
                    for r in &list {
                        ui.label(fmt_stamp(&r.opened_at));
                        ui.label(&r.shop);
                        ui.scope(|ui| {
                            ui.set_min_width(180.0);
                            ui.set_max_width(260.0);
                            ui.add(egui::Label::new(&r.work).truncate()).on_hover_text(&r.work);
                        });
                        ui.label(fmt_date(r.drop_off));
                        match r.returned_on {
                            Some(d) => ui.label(fmt_date(Some(d))),
                            None => ui.colored_label(DUE_SOON, "in shop now"),
                        };
                        ui.label(r.days(today).map(|d| d.to_string()).unwrap_or_default());
                        ui.label(&r.repair_time);
                        ui.label(r.est_cost.map(fmt_money).unwrap_or_default());
                        ui.label(RichText::new(r.final_cost.map(fmt_money).unwrap_or_default()).strong());
                        if r.returned_on.is_some() {
                            if ui.small_button("Edit").clicked() {
                                actions.push(VehAction::EditRepair((*r).clone()));
                            }
                        } else {
                            ui.weak("edit in Shop above");
                        }
                        ui.end_row();
                    }
                    let finals: f64 = list.iter().filter_map(|r| r.final_cost).sum();
                    let ests: f64 = list.iter().filter_map(|r| r.est_cost).sum();
                    ui.label(RichText::new("Total").strong());
                    for _ in 0..4 {
                        ui.label("");
                    }
                    ui.label(RichText::new(days.to_string()).strong());
                    ui.label("");
                    ui.label(RichText::new(fmt_money(ests)).strong());
                    ui.label(RichText::new(fmt_money(finals)).strong());
                    ui.end_row();
                });
            });
            ui.weak("Total cost in the heading uses the final cost where there is one, else the estimate.");
        });
}

/// Editing a closed (or hand-added) repair.
fn repair_editor(ctx: &egui::Context, edit: &mut Option<Repair>, confirm_delete: &mut bool, label: &str, actions: &mut Vec<VehAction>) {
    let Some(r) = edit.as_mut() else { return };
    let mut open = true;
    let mut close = false;
    egui::Window::new(if r.id == 0 { format!("Add past repair · {label}") } else { format!("Repair · {label}") })
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            if r.id > 0 {
                ui.weak(format!("Put in shop {}", fmt_stamp(&r.opened_at)));
            }
            Grid::new("repair_edit").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Shop");
                ui.add(TextEdit::singleline(&mut r.shop).desired_width(260.0));
                ui.end_row();
                ui.label("Work done");
                ui.add(TextEdit::multiline(&mut r.work).desired_rows(2).desired_width(360.0));
                ui.end_row();
                let today = today();
                ui.label("Drop-off date");
                optional_date(ui, "rep_drop", &mut r.drop_off, today, false);
                ui.end_row();
                ui.label("Back on");
                ui.horizontal(|ui| {
                    let mut back = r.returned_on.unwrap_or(today);
                    ui.add(DatePickerButton::new(&mut back).id_salt("rep_back"));
                    r.returned_on = Some(back);
                    if let Some(d) = r.days(today) {
                        ui.weak(format!("{d} day{} in the shop", plural(d)));
                    }
                });
                ui.end_row();
                ui.label("Est. time of repair");
                ui.add(TextEdit::singleline(&mut r.repair_time).desired_width(160.0));
                ui.end_row();
                ui.label("Est. cost");
                ui.horizontal(|ui| {
                    money_field(ui, &mut r.est_cost);
                });
                ui.end_row();
                ui.label("Final cost");
                ui.horizontal(|ui| {
                    money_field(ui, &mut r.final_cost);
                });
                ui.end_row();
                ui.label("Notes");
                ui.add(TextEdit::multiline(&mut r.notes).desired_rows(2).desired_width(360.0));
                ui.end_row();
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new("💾 Save repair").strong()).clicked() {
                    actions.push(VehAction::SaveRepair(r.clone()));
                    close = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
                if r.id > 0 {
                    if *confirm_delete {
                        if ui.button(RichText::new("Confirm delete").color(OVERDUE)).clicked() {
                            actions.push(VehAction::DeleteRepair(r.id));
                            close = true;
                        }
                    } else if ui.button("🗑 Delete").clicked() {
                        *confirm_delete = true;
                    }
                }
            });
        });
    if close || !open {
        *edit = None;
        *confirm_delete = false;
    }
}

/// The repair report as CSV: one row per repair, vehicle by vehicle, with a
/// total line per vehicle (and a grand total when there's more than one).
pub fn repairs_csv(vehicles: &[Vehicle], repairs: &[Repair], only: Option<i64>, today: NaiveDate) -> String {
    let q = |s: &str| format!("\"{}\"", s.replace('"', "\"\"").replace(['\r', '\n'], " "));
    let money = |v: Option<f64>| v.map(|v| format!("{v:.2}")).unwrap_or_default();
    let mut out = String::from(
        "Truck #,Vehicle,Plate,Put in shop,Shop,Work,Drop-off,Back,Days in shop,Est. time,Est. cost,Final cost,Cost (final or est.),Notes\r\n",
    );
    let (mut all_cost, mut all_days, mut all_n, mut shown) = (0.0, 0, 0, 0);
    for v in vehicles.iter().filter(|v| only.map_or(true, |id| v.id == id)) {
        let list: Vec<&Repair> = repairs.iter().filter(|r| r.vehicle_id == v.id).collect();
        if list.is_empty() && only.is_none() {
            continue;
        }
        shown += 1;
        for r in list.iter().rev() {
            let back = match r.returned_on {
                Some(d) => d.format("%Y-%m-%d").to_string(),
                None => "in shop".into(),
            };
            out.push_str(
                &[
                    q(&v.label()),
                    q(&v.description()),
                    q(&v.plate),
                    q(&r.opened_at),
                    q(&r.shop),
                    q(&r.work),
                    r.drop_off.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default(),
                    back,
                    r.days(today).map(|d| d.to_string()).unwrap_or_default(),
                    q(&r.repair_time),
                    money(r.est_cost),
                    money(r.final_cost),
                    money(r.cost()),
                    q(&r.notes),
                ]
                .join(","),
            );
            out.push_str("\r\n");
        }
        let (cost, days) = repair_totals(&list, today);
        out.push_str(&format!(
            "{},{},,,,\"Total: {} repair{}\",,,{},,,,{:.2},\r\n",
            q(&v.label()),
            q(&v.description()),
            list.len(),
            plural(list.len() as i64),
            days,
            cost
        ));
        all_cost += cost;
        all_days += days;
        all_n += list.len();
    }
    if shown > 1 {
        out.push_str(&format!(",\"All vehicles\",,,,\"Total: {all_n} repairs\",,,{all_days},,,,{all_cost:.2},\r\n"));
    }
    out
}

/// A dollar amount (None = not entered). Returns true when it changed.
pub(crate) fn money_field(ui: &mut egui::Ui, value: &mut Option<f64>) -> bool {
    let mut v = value.unwrap_or(0.0);
    let resp = ui.add(
        DragValue::new(&mut v)
            .range(0.0..=1_000_000.0)
            .speed(5.0)
            .min_decimals(2)
            .max_decimals(2)
            .custom_formatter(|n, _| fmt_money(n))
            .custom_parser(parse_money),
    );
    if value.is_none() {
        ui.weak("not entered");
    }
    if resp.changed() {
        *value = (v > 0.0).then_some(v);
        true
    } else {
        false
    }
}

/// A text detail from VEHICLE_FIELDS.
fn info_field(ui: &mut egui::Ui, draft: &mut Vehicle, f: &VehicleField) {
    let text = draft.info.entry(f.key.to_string()).or_default();
    if f.multiline {
        ui.add(TextEdit::multiline(text).desired_rows(2).desired_width(520.0));
    } else {
        ui.add(TextEdit::singleline(text).desired_width(260.0));
    }
}

pub fn show(app: &mut SchedulerApp, ctx: &egui::Context) {
    let today = today();
    let state = &mut app.vehicles_state;
    if state.draft.is_none() && state.selected.is_none() {
        if let Some(v) = app.vehicles.first() {
            state.selected = Some(v.id);
            state.draft = Some(v.clone());
        }
    }
    let original = state.selected.and_then(|id| app.vehicles.iter().find(|v| v.id == id).cloned());
    let dirty = match (&state.draft, &original) {
        (Some(d), Some(o)) => !same(d, o),
        (Some(d), None) => d.id == 0,
        _ => false,
    };
    let editable = app.can_edit();
    let mut actions: Vec<VehAction> = Vec::new();
    let crews = &app.crews;
    let employees = &app.employees;
    let crew_of = |id: Option<i64>| id.and_then(|id| crews.iter().find(|c| c.id == id));
    let employee = |id: Option<i64>| id.and_then(|id| employees.iter().find(|e| e.id == id));
    // Who drives it: the linked employee, else the name typed in.
    let driver_of = |v: &Vehicle| employee(v.driver_id).map(|e| e.name.clone()).unwrap_or_else(|| v.get("driver").to_string());
    // Crews that go out in it (set on the crew), else its driver's crew today.
    let crews_using = |vid: i64| crews.iter().filter(move |c| vid > 0 && c.vehicle_id == Some(vid));
    let crew_for = |v: &Vehicle| crews_using(v.id).next().or_else(|| crew_of(employee(v.driver_id).and_then(|e| e.crew_id)));

    egui::SidePanel::left("vehicle_list")
        .resizable(true)
        .default_width(400.0)
        .width_range(300.0..=560.0)
        .show(ctx, |ui| {
            let state = &mut app.vehicles_state;
            let vehicles = &app.vehicles;
            let db = &app.db;
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading("Vehicles");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add_enabled(editable, Button::new("+ New Vehicle")).clicked() {
                        actions.push(VehAction::New);
                    }
                    if ui
                        .add_enabled(editable && !vehicles.is_empty(), Button::new("⬆ Import Mileage…"))
                        .on_hover_text("Update odometers from the GPS company's asset report (.csv). Trucks are matched by their # number.")
                        .clicked()
                    {
                        actions.push(VehAction::ImportMileage);
                    }
                });
            });
            ui.add(
                TextEdit::singleline(&mut state.search)
                    .hint_text("Search #, make, plate, VIN, driver, GPS, notes…")
                    .desired_width(f32::INFINITY),
            );
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut state.filter, VehFilter::All, format!("All ({})", vehicles.len()));
                for (i, (s, _)) in VEHICLE_STATUSES.iter().enumerate() {
                    let n = vehicles.iter().filter(|v| v.status == *s).count();
                    ui.selectable_value(&mut state.filter, VehFilter::Status(i), format!("{s} ({n})"));
                }
                let due = vehicles.iter().filter(|v| !v.reminders(today).is_empty()).count();
                ui.selectable_value(&mut state.filter, VehFilter::Reminders, format!("Due soon ({due})"));
            });
            let sort = *state.sort.get_or_insert_with(|| match db.get_setting("vehicle_sort").as_deref() {
                Some("driver") => VehSort::Driver,
                _ => VehSort::Truck,
            });
            ui.horizontal(|ui| {
                ui.label("Sort by:");
                let mut s = sort;
                ui.selectable_value(&mut s, VehSort::Truck, "Truck #");
                ui.selectable_value(&mut s, VehSort::Driver, "Driver")
                    .on_hover_text("By last name; vehicles without a driver go at the end");
                if s != sort {
                    state.sort = Some(s);
                    let _ = db.set_setting("vehicle_sort", if s == VehSort::Driver { "driver" } else { "truck" });
                }
            });
            ui.separator();

            if vehicles.is_empty() {
                ui.weak("No vehicles yet. Click “+ New Vehicle” to add your first one.");
            }
            // Vehicles load in truck # order; re-sort by driver when asked.
            let mut ordered: Vec<&Vehicle> = vehicles.iter().collect();
            if state.sort == Some(VehSort::Driver) {
                ordered.sort_by_cached_key(|v| {
                    let d = driver_of(v);
                    (d.trim().is_empty(), driver_key(&d), v.sort_key())
                });
            }
            let q = state.search.trim().to_lowercase();
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for v in ordered {
                    let keep = match state.filter {
                        VehFilter::All => true,
                        VehFilter::Status(i) => v.status == VEHICLE_STATUSES[i].0,
                        VehFilter::Reminders => !v.reminders(today).is_empty(),
                    };
                    if !keep {
                        continue;
                    }
                    let crew = crew_for(v);
                    let driver = driver_of(v);
                    if !q.is_empty() {
                        let mut hay = format!(
                            "{} {} {} {} {} {} {} {} {} {}",
                            v.label(),
                            v.unit,
                            v.kind,
                            v.description(),
                            v.plate,
                            v.vin,
                            v.status,
                            crew.map(|c| c.name.as_str()).unwrap_or(""),
                            driver,
                            v.notes
                        );
                        for val in v.info.values() {
                            hay.push(' ');
                            hay.push_str(val);
                        }
                        if !hay.to_lowercase().contains(&q) {
                            continue;
                        }
                    }
                    let selected = state.selected == Some(v.id);
                    ui.horizontal(|ui| {
                        status_badge(ui, v);
                        let resp = ui.selectable_label(selected, RichText::new(v.label()).size(14.0).strong());
                        // Leave room for the driver on the right.
                        let room = (ui.available_width() - 150.0).max(30.0);
                        ui.scope(|ui| {
                            ui.set_max_width(room);
                            ui.add(egui::Label::new(RichText::new(v.description()).weak()).truncate());
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.set_max_width(150.0);
                            if v.status == "In Shop" {
                                match v.info_date(SHOP_PICKUP) {
                                    Some(p) if p < today => {
                                        ui.label(RichText::new(format!("In Shop · pickup was {}", p.format("%b %-d"))).small().color(OVERDUE));
                                    }
                                    Some(p) => {
                                        ui.label(RichText::new(format!("In Shop · pickup {}", p.format("%b %-d"))).small().color(v.status_color()));
                                    }
                                    None => {
                                        ui.label(RichText::new("In Shop").small().color(v.status_color()));
                                    }
                                }
                            } else if v.status != "Active" {
                                ui.label(RichText::new(&v.status).small().color(v.status_color()));
                            }
                            if !driver.is_empty() {
                                ui.add(egui::Label::new(RichText::new(&driver).small()).truncate());
                            }
                            if let Some(c) = crew {
                                let (r, resp) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                                ui.painter().rect_filled(r, 2.0, c.color32());
                                resp.on_hover_text(&c.name);
                            }
                        });
                        if resp.clicked() && !selected {
                            actions.push(VehAction::Select(v.id));
                        }
                    });
                    let reminders = v.reminders(today);
                    if !reminders.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            ui.add_space(18.0);
                            for (label, d, overdue) in reminders {
                                let (text, color) = if overdue {
                                    (format!("⚠ {label} overdue ({})", d.format("%b %-d, %Y")), OVERDUE)
                                } else {
                                    (format!("⏰ {label} due {}", d.format("%b %-d")), DUE_SOON)
                                };
                                ui.label(RichText::new(text).small().color(color));
                            }
                        });
                    }
                }
            });
        });

    egui::CentralPanel::default().show(ctx, |ui| {
        if !editable {
            ui.disable();
        }
        let state = &mut app.vehicles_state;
        let Some(draft) = state.draft.as_mut() else {
            ui.centered_and_justified(|ui| {
                ui.label("Select a vehicle on the left, or click “+ New Vehicle”.");
            });
            return;
        };

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let title = if draft.unit.trim().is_empty() { "New Vehicle".to_string() } else { draft.label() };
            ui.heading(title);
            let desc = draft.description();
            if !desc.is_empty() {
                ui.weak(desc);
            }
            if dirty {
                ui.colored_label(Color32::from_rgb(230, 150, 30), "(unsaved changes)");
            }
        });
        ui.add_space(4.0);

        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            Grid::new("vehicle_form").num_columns(2).spacing([12.0, 8.0]).min_col_width(150.0).show(ui, |ui| {
                ui.label("Truck / unit #");
                ui.add(TextEdit::singleline(&mut draft.unit).desired_width(120.0).hint_text("e.g. 12"));
                ui.end_row();

                ui.label("Body type");
                ui.horizontal(|ui| {
                    ui.add(TextEdit::singleline(&mut draft.kind).desired_width(220.0).hint_text("e.g. Service Body 4 Door"));
                    ui.menu_button("Presets", |ui| {
                        for k in VEHICLE_KINDS {
                            if ui.button(*k).clicked() {
                                draft.kind = k.to_string();
                                ui.close_menu();
                            }
                        }
                    });
                });
                ui.end_row();

                ui.label("Year / Make / Model");
                ui.horizontal(|ui| {
                    ui.add(TextEdit::singleline(&mut draft.year).desired_width(50.0).hint_text("Year"));
                    ui.add(TextEdit::singleline(&mut draft.make).desired_width(110.0).hint_text("Make"));
                    ui.add(TextEdit::singleline(&mut draft.model).desired_width(170.0).hint_text("Model"));
                });
                ui.end_row();

                ui.label("Status");
                ui.horizontal(|ui| {
                    for (s, rgb) in VEHICLE_STATUSES {
                        let text = RichText::new(*s).color(Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                        if ui.selectable_label(draft.status == *s, text).clicked() {
                            draft.status = s.to_string();
                            // Going into the shop: drop-off defaults to today.
                            if *s == "In Shop" && draft.info_date(SHOP_DROP_OFF).is_none() {
                                draft.set_info_date(SHOP_DROP_OFF, Some(today));
                            }
                        }
                    }
                });
                ui.end_row();

                ui.label("License plate");
                ui.add(TextEdit::singleline(&mut draft.plate).desired_width(120.0));
                ui.end_row();

                ui.label("VIN");
                ui.add(TextEdit::singleline(&mut draft.vin).desired_width(220.0).char_limit(17));
                ui.end_row();

                ui.label("Tire size");
                ui.add(
                    TextEdit::singleline(draft.info.entry("tire_size".into()).or_default())
                        .desired_width(160.0)
                        .hint_text("e.g. 245/75/R17"),
                );
                ui.end_row();

                ui.label("Tab expires");
                optional_date(ui, "veh_reg", &mut draft.registration_due, today, true);
                ui.end_row();

                ui.label("Insurance due");
                optional_date(ui, "veh_ins", &mut draft.insurance_due, today, true);
                ui.end_row();

                ui.label("Purchase date");
                optional_date(ui, "veh_buy", &mut draft.purchase_date, today, false);
                ui.end_row();

                ui.label("Crews").on_hover_text(
                    "The crews that go out in this vehicle. This is the truck shown on their Attendance card; \
                     you can also set it on the crew (Staff → Crews → Vehicle).",
                );
                ui.horizontal_wrapped(|ui| {
                    if draft.id == 0 {
                        ui.weak("Save the vehicle first, then assign it to a crew here.");
                        return;
                    }
                    let using: Vec<&Crew> = crews_using(draft.id).collect();
                    for c in &using {
                        egui::Frame::none()
                            .fill(c.color32().gamma_multiply(0.25))
                            .rounding(4.0)
                            .inner_margin(egui::Margin::symmetric(6.0, 2.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(&c.name);
                                    if ui.small_button("x").on_hover_text("Take this vehicle off the crew").clicked() {
                                        actions.push(VehAction::SetCrewVehicle(c.id, None));
                                    }
                                });
                            });
                    }
                    ui.menu_button("Assign to crew…", |ui| {
                        ui.set_max_height(380.0);
                        ScrollArea::vertical().show(ui, |ui| {
                            for b in Board::ALL {
                                ui.weak(format!("{} crews", b.as_str()));
                                for c in crews.iter().filter(|c| c.board == b && c.active && c.vehicle_id != Some(draft.id)) {
                                    let other = c.vehicle_id.and_then(|vid| app.vehicles.iter().find(|v| v.id == vid));
                                    let label = match other {
                                        Some(o) => format!("{}  (now {})", c.name, o.label()),
                                        None => c.name.clone(),
                                    };
                                    if ui.button(label).clicked() {
                                        actions.push(VehAction::SetCrewVehicle(c.id, Some(draft.id)));
                                        ui.close_menu();
                                    }
                                }
                                ui.separator();
                            }
                        });
                    });
                    if using.is_empty() {
                        match crew_of(employee(draft.driver_id).and_then(|e| e.crew_id)) {
                            Some(c) => ui.weak(format!("none (driver is on {})", c.name)),
                            None => ui.weak("none"),
                        };
                    } else if using.len() > 1 {
                        ui.colored_label(DUE_SOON, format!("shared by {} crews", using.len()));
                    }
                });
                ui.end_row();
            });

            if draft.status == "In Shop" {
                ui.add_space(6.0);
                shop_section(ui, draft, today);
            }
            ui.add_space(6.0);
            repair_history(ui, draft, &app.repairs, today, &mut actions);

            for (si, section) in VEHICLE_SECTIONS.iter().enumerate() {
                ui.add_space(6.0);
                CollapsingHeader::new(RichText::new(*section).strong().size(15.0))
                    .id_salt(("veh_section", si))
                    .default_open(true)
                    .show(ui, |ui| {
                        Grid::new(("veh_grid", si)).num_columns(2).spacing([12.0, 8.0]).min_col_width(150.0).show(ui, |ui| {
                            if si == 2 {
                                ui.label("Odometer");
                                ui.horizontal(|ui| {
                                    ui.add(
                                        DragValue::new(&mut draft.odometer)
                                            .range(0..=2_000_000)
                                            .speed(10.0)
                                            .custom_formatter(|n, _| {
                                                let s = (n as i64).to_string();
                                                let mut out = String::new();
                                                for (i, ch) in s.chars().enumerate() {
                                                    if i > 0 && (s.len() - i) % 3 == 0 {
                                                        out.push(',');
                                                    }
                                                    out.push(ch);
                                                }
                                                out
                                            })
                                            .custom_parser(|s| s.replace(',', "").trim().parse::<f64>().ok()),
                                    );
                                    ui.label("miles");
                                });
                                ui.end_row();

                                ui.label("Last oil change");
                                optional_date(ui, "veh_oil", &mut draft.last_oil_change, today, false);
                                ui.end_row();

                                ui.label("Next service");
                                optional_date(ui, "veh_svc", &mut draft.next_service, today, true);
                                ui.end_row();
                            }
                            // Tire size is shown with the main details instead.
                            for f in VEHICLE_FIELDS.iter().filter(|f| f.section == si && f.key != "tire_size") {
                                ui.label(f.label);
                                if f.key == "driver" {
                                    ui.horizontal(|ui| {
                                        info_field(ui, draft, f);
                                        let linked = employee(draft.driver_id);
                                        ui.menu_button("Pick employee…", |ui| {
                                            ui.set_max_height(360.0);
                                            ScrollArea::vertical().show(ui, |ui| {
                                                for b in Board::ALL {
                                                    ui.weak(format!("{} Staff", b.as_str()));
                                                    for e in employees.iter().filter(|e| e.staff == b && e.active) {
                                                        if ui.selectable_label(draft.driver_id == Some(e.id), &e.name).clicked() {
                                                            draft.driver_id = Some(e.id);
                                                            draft.info.insert("driver".into(), e.name.clone());
                                                            if !e.phone.trim().is_empty() {
                                                                draft.info.insert("driver_phone".into(), e.phone.clone());
                                                            }
                                                            ui.close_menu();
                                                        }
                                                    }
                                                    ui.separator();
                                                }
                                            });
                                        })
                                        .response
                                        .on_hover_text("Link the driver to an employee, so the vehicle follows their crew");
                                        if let Some(e) = linked {
                                            ui.colored_label(LINKED, format!("✔ {}", e.name))
                                                .on_hover_text("Linked to this employee");
                                            if ui.small_button("Unlink").clicked() {
                                                draft.driver_id = None;
                                            }
                                        } else if !draft.get("driver").trim().is_empty() {
                                            ui.weak("not linked to an employee");
                                        }
                                    });
                                } else {
                                    info_field(ui, draft, f);
                                }
                                ui.end_row();
                            }
                        });
                    });
            }

            ui.add_space(6.0);
            Grid::new("vehicle_notes").num_columns(2).spacing([12.0, 8.0]).min_col_width(150.0).show(ui, |ui| {
                ui.label("Notes");
                ui.add(TextEdit::multiline(&mut draft.notes).desired_rows(3).desired_width(520.0));
                ui.end_row();
            });

            ui.add_space(12.0);
            ui.separator();
            ui.horizontal(|ui| {
                let can_save = dirty && !draft.unit.trim().is_empty();
                if ui.add_enabled(can_save, Button::new(RichText::new("💾 Save Vehicle").strong())).clicked() {
                    actions.push(VehAction::Save);
                }
                if ui.add_enabled(dirty, Button::new("Discard Changes")).clicked() {
                    actions.push(VehAction::Revert);
                }
                if dirty && draft.unit.trim().is_empty() {
                    ui.weak("Truck / unit # is required");
                }
                if draft.id > 0 {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if state.confirm_delete {
                            if ui.button("Keep").clicked() {
                                state.confirm_delete = false;
                            }
                            if ui.button(RichText::new("Confirm Delete").color(OVERDUE)).clicked() {
                                actions.push(VehAction::Delete(draft.id));
                            }
                            ui.weak("This can't be undone.");
                        } else if ui.button("🗑 Delete Vehicle").clicked() {
                            state.confirm_delete = true;
                        }
                    });
                }
            });
            ui.add_space(12.0);
        });
    });

    for a in actions {
        match a {
            VehAction::Select(_) | VehAction::New | VehAction::ImportMileage if dirty => {
                app.set_error("Save or discard your vehicle changes first.");
            }
            VehAction::Select(id) => {
                let s = &mut app.vehicles_state;
                s.selected = Some(id);
                s.draft = app.vehicles.iter().find(|v| v.id == id).cloned();
                s.confirm_delete = false;
            }
            VehAction::New => {
                let s = &mut app.vehicles_state;
                s.selected = None;
                s.draft = Some(Vehicle::new());
                s.confirm_delete = false;
            }
            VehAction::Save => {
                let Some(draft) = app.vehicles_state.draft.clone() else { continue };
                let was_in_shop = app.repairs.iter().any(|r| r.vehicle_id == draft.id && r.returned_on.is_none());
                match app.db.save_vehicle_by(&draft, app.user_id()) {
                    Ok(id) => {
                        app.reload();
                        app.vehicles_state.selected = Some(id);
                        app.vehicles_state.draft = app.vehicles.iter().find(|v| v.id == id).cloned();
                        let now_in_shop = draft.status == "In Shop";
                        app.set_status(match (was_in_shop, now_in_shop) {
                            (false, true) => format!("{} is in the shop; repair started in its history.", draft.label()),
                            (true, false) => format!("{} is back from the shop; repair closed in its history.", draft.label()),
                            _ => format!("Saved vehicle {}", draft.label()),
                        });
                    }
                    Err(e) => app.set_error(format!("Could not save vehicle: {e}")),
                }
            }
            VehAction::Revert => {
                app.vehicles_state.draft = original.clone();
                if original.is_none() {
                    app.vehicles_state.selected = None;
                }
            }
            VehAction::Delete(id) => match app.db.delete_vehicle(id) {
                Ok(()) => {
                    app.reload();
                    app.vehicles_state = VehiclesState::default();
                    app.set_status("Vehicle deleted.");
                }
                Err(e) => app.set_error(format!("Could not delete vehicle: {e}")),
            },
            VehAction::NewRepair => {
                if let Some(vid) = app.vehicles_state.selected {
                    app.vehicles_state.repair_edit = Some(Repair {
                        vehicle_id: vid,
                        returned_on: Some(today),
                        ..Repair::default()
                    });
                }
            }
            VehAction::EditRepair(r) => {
                app.vehicles_state.repair_edit = Some(r);
                app.vehicles_state.confirm_delete_repair = false;
            }
            VehAction::SaveRepair(_) | VehAction::DeleteRepair(_) => {}
            VehAction::ExportRepairs(only) => {
                let name = match only.and_then(|id| app.vehicles.iter().find(|v| v.id == id)) {
                    Some(v) => format!("Repairs - Truck {} - {}.csv", v.unit.trim(), today.format("%Y-%m-%d")),
                    None => format!("Repairs - All Vehicles - {}.csv", today.format("%Y-%m-%d")),
                };
                let Some(path) = rfd::FileDialog::new()
                    .set_title("Save repair report")
                    .set_file_name(&name)
                    .add_filter("CSV (opens in Excel)", &["csv"])
                    .save_file()
                else {
                    continue;
                };
                let csv = repairs_csv(&app.vehicles, &app.repairs, only, today);
                // A byte-order mark so Excel reads it as UTF-8.
                match std::fs::write(&path, format!("\u{feff}{csv}")) {
                    Ok(()) => app.set_status(format!("Saved repair report to {}", path.display())),
                    Err(e) => app.set_error(format!("Could not save the report: {e}")),
                }
            }
            VehAction::SetCrewVehicle(crew, vehicle) => {
                let crew_name = app.crews.iter().find(|c| c.id == crew).map(|c| c.name.clone()).unwrap_or_default();
                let label = vehicle.and_then(|id| app.vehicles.iter().find(|v| v.id == id)).map(|v| v.label());
                match app.db.set_crew_vehicle(crew, vehicle) {
                    Ok(()) => {
                        app.reload();
                        app.set_status(match label {
                            Some(l) => format!("{crew_name} now goes out in {l}."),
                            None => format!("Took the vehicle off {crew_name}."),
                        });
                    }
                    Err(e) => app.set_error(format!("Could not change the crew's vehicle: {e}")),
                }
            }
            VehAction::ImportMileage => {
                let Some(path) = rfd::FileDialog::new()
                    .set_title("Import mileage from the GPS asset report")
                    .add_filter("CSV report", &["csv", "txt"])
                    .add_filter("All files", &["*"])
                    .pick_file()
                else {
                    continue;
                };
                let file = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                let text = match std::fs::read(&path) {
                    Ok(bytes) => String::from_utf8_lossy(&bytes).to_string(),
                    Err(e) => {
                        app.set_error(format!("Could not read {file}: {e}"));
                        continue;
                    }
                };
                let plan = plan_mileage(&text, &app.vehicles, today);
                if plan.updates.is_empty() && plan.unmatched.is_empty() && plan.no_mileage.is_empty() {
                    app.set_error(format!("{file} has no rows like “#9 - Name, Make, Model, Mileage”. Nothing changed."));
                    continue;
                }
                match app.db.save_vehicles(&plan.updates) {
                    Ok(()) => {
                        app.reload();
                        // Show the selected vehicle's new mileage.
                        let s = &mut app.vehicles_state;
                        s.draft = s.selected.and_then(|id| app.vehicles.iter().find(|v| v.id == id).cloned());
                        let msg = format!(
                            "Mileage updated on {} vehicle{} from {file}; {} not matched.",
                            plan.updates.len(),
                            plural(plan.updates.len() as i64),
                            plan.unmatched.len()
                        );
                        app.vehicles_state.mileage_summary = Some((file, plan));
                        app.set_status(msg);
                    }
                    Err(e) => app.set_error(format!("Mileage import failed, nothing was changed: {e}")),
                }
            }
        }
    }
    mileage_summary_window(ctx, &mut app.vehicles_state.mileage_summary);
    let label = app
        .vehicles_state
        .repair_edit
        .as_ref()
        .and_then(|r| app.vehicles.iter().find(|v| v.id == r.vehicle_id))
        .map(|v| format!("{} {}", v.label(), v.description()))
        .unwrap_or_default();
    let mut repair_actions = Vec::new();
    let s = &mut app.vehicles_state;
    repair_editor(ctx, &mut s.repair_edit, &mut s.confirm_delete_repair, &label, &mut repair_actions);
    for a in repair_actions {
        match a {
            VehAction::SaveRepair(mut r) => {
                if r.opened_at.is_empty() {
                    // A past repair: stamp it with its drop-off (else back) date.
                    let d = r.drop_off.or(r.returned_on).unwrap_or(today);
                    r.opened_at = format!("{} 00:00", d.format("%Y-%m-%d"));
                    r.opened_by = app.user_id();
                }
                match app.db.save_repair(&r) {
                    Ok(_) => {
                        app.reload();
                        app.set_status("Repair saved.");
                    }
                    Err(e) => app.set_error(format!("Could not save the repair: {e}")),
                }
            }
            VehAction::DeleteRepair(id) => match app.db.delete_repair(id) {
                Ok(()) => {
                    app.reload();
                    app.set_status("Repair deleted.");
                }
                Err(e) => app.set_error(format!("Could not delete the repair: {e}")),
            },
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::driver_key;

    #[test]
    fn drivers_sort_by_last_name() {
        assert_eq!(driver_key("Isaiah Marsh"), ("marsh".into(), "isaiah".into()));
        assert_eq!(driver_key("Marsh, Isaiah"), ("marsh".into(), "isaiah".into()));
        let mut names = vec!["Zach McBride", "Chris Gomez", "Adam Sorensen", "Chris Abbott"];
        names.sort_by_key(|n| driver_key(n));
        assert_eq!(names, vec!["Chris Abbott", "Chris Gomez", "Zach McBride", "Adam Sorensen"]);
    }
}

#[cfg(test)]
mod shop_tests {
    use crate::model::*;
    use chrono::NaiveDate;

    #[test]
    fn shop_details_round_trip() {
        let d = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        let db = crate::db::Db::open_in_memory().unwrap();
        let mut v = Vehicle::new();
        v.unit = "7".into();
        v.status = "In Shop".into();
        v.set_info_date(SHOP_DROP_OFF, Some(d("2026-11-13")));
        v.set_info_date(SHOP_PICKUP, Some(d("2026-11-20")));
        v.info.insert(SHOP_NAME.into(), "Sunset Chevrolet".into());
        v.info.insert(SHOP_REPAIR_TIME.into(), "5 days".into());
        v.info.insert(SHOP_COST.into(), "1234.50".into());
        db.save_vehicle(&v).unwrap();
        let got = &db.load_vehicles().unwrap()[0];
        assert_eq!(got.info_date(SHOP_DROP_OFF), Some(d("2026-11-13")));
        assert_eq!(got.info_date(SHOP_PICKUP), Some(d("2026-11-20")));
        assert_eq!(got.get(SHOP_NAME), "Sunset Chevrolet");
        assert_eq!(parse_money(got.get(SHOP_COST)), Some(1234.5));
        let mut c = got.clone();
        c.set_info_date(SHOP_PICKUP, None);
        assert_eq!(c.info_date(SHOP_PICKUP), None);
    }
}

#[cfg(test)]
mod repair_tests {
    use crate::db::Db;
    use crate::model::*;
    use chrono::NaiveDate;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn in_shop_opens_a_repair_and_coming_back_closes_it() {
        let db = Db::open_in_memory().unwrap();
        let mut v = Vehicle::new();
        v.unit = "24".into();
        let id = db.save_vehicle(&v).unwrap();
        assert!(db.load_repairs().unwrap().is_empty(), "nothing until it goes in the shop");

        // In the shop: one open repair, stamped, following the Shop fields.
        let mut v = db.load_vehicles().unwrap().remove(0);
        v.status = "In Shop".into();
        v.info.insert(SHOP_NAME.into(), "Sunset Chevrolet".into());
        v.info.insert(SHOP_WORK.into(), "Front brakes".into());
        v.set_info_date(SHOP_DROP_OFF, Some(d("2026-11-10")));
        v.info.insert(SHOP_COST.into(), "800.00".into());
        db.save_vehicle_by(&v, Some(7)).unwrap();
        v.info.insert(SHOP_REPAIR_TIME.into(), "3 days".into());
        db.save_vehicle(&v).unwrap(); // saving again updates the same repair
        let reps = db.load_repairs().unwrap();
        assert_eq!(reps.len(), 1);
        let r = &reps[0];
        assert_eq!((r.vehicle_id, r.returned_on, r.opened_by), (id, None, Some(7)));
        assert_eq!(r.opened_at.len(), 16, "stamped with date and time: {}", r.opened_at);
        assert_eq!((r.shop.as_str(), r.work.as_str(), r.repair_time.as_str()), ("Sunset Chevrolet", "Front brakes", "3 days"));
        assert_eq!((r.drop_off, r.est_cost), (Some(d("2026-11-10")), Some(800.0)));

        // Back: closed today with the final cost; the Shop fields clear.
        v.info.insert(SHOP_FINAL_COST.into(), "912.40".into());
        v.status = "Active".into();
        db.save_vehicle(&v).unwrap();
        let r = &db.load_repairs().unwrap()[0];
        assert_eq!(r.returned_on, Some(chrono::Local::now().date_naive()));
        assert_eq!(r.final_cost, Some(912.4));
        assert_eq!(r.cost(), Some(912.4));
        let v = db.load_vehicles().unwrap().remove(0);
        assert!(SHOP_KEYS.iter().all(|k| v.get(k).is_empty()));

        // The next visit is a second repair.
        let mut v2 = v.clone();
        v2.status = "In Shop".into();
        db.save_vehicle(&v2).unwrap();
        assert_eq!(db.load_repairs().unwrap().len(), 2);
    }

    #[test]
    fn days_and_report() {
        let mut v = Vehicle::new();
        v.id = 3;
        v.unit = "24".into();
        v.year = "2021".into();
        v.make = "Chevy".into();
        v.model = "Silverado 2500".into();
        let today = d("2026-11-13");
        let done = Repair {
            id: 1,
            vehicle_id: 3,
            opened_at: "2026-10-01 09:30".into(),
            shop: "Les Schwab".into(),
            work: "4 tires, \"alignment\"".into(),
            drop_off: Some(d("2026-10-01")),
            returned_on: Some(d("2026-10-04")),
            est_cost: Some(900.0),
            final_cost: Some(1050.5),
            ..Repair::default()
        };
        let open = Repair { id: 2, vehicle_id: 3, opened_at: "2026-11-10 14:00".into(), est_cost: Some(300.0), ..Repair::default() };
        assert_eq!(done.days(today), Some(3));
        assert_eq!(open.days(today), Some(3), "still in the shop: counts to today from the stamp");
        assert_eq!(open.cost(), Some(300.0));
        assert_eq!(fmt_money(1234567.5), "$1,234,567.50");
        assert_eq!(parse_money("$1,050.50"), Some(1050.5));

        let csv = super::repairs_csv(&[v], &[open, done], None, today);
        let lines: Vec<&str> = csv.lines().collect();
        assert!(lines[0].starts_with("Truck #,Vehicle,Plate,Put in shop"));
        assert!(lines[1].contains("\"2026-10-01 09:30\"") && lines[1].contains("\"4 tires, \"\"alignment\"\"\"") && lines[1].contains("1050.50"));
        assert!(lines[2].contains("in shop") && lines[2].contains("300.00"));
        assert!(lines[3].contains("Total: 2 repairs") && lines[3].ends_with(",6,,,,1350.50,"), "{}", lines[3]);
        assert_eq!(lines.len(), 4, "no grand total for a single vehicle");
    }
}
