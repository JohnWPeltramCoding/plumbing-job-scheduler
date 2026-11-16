//! Import employees from the payroll "Active Headcount" export (.csv).
//!
//! Only plumbing positions are taken, each going to its staff list:
//! RI PLUMBER / RI PLUMBER SUP -> Rough-In Staff, TRIM PLUMBER / TRIM PLUMBER
//! SUP -> Trim Staff (supervisors get the Field Supervisor role). People under
//! the supervisors ticked in the preview are left out (remembered between
//! imports). Anyone already in the app (same employee ID, or else the same
//! name) is updated instead of added, keeping their crew, phone and notes.

use std::collections::{BTreeMap, BTreeSet};

use eframe::egui::{self, vec2, Button, CollapsingHeader, Color32, Grid, RichText, ScrollArea};

use crate::app::SchedulerApp;
use crate::fleet_import::csv_fields;
use crate::model::*;

/// Position codes that are imported: (code, staff list, role).
pub const POSITIONS: &[(&str, Board, &str)] = &[
    ("RI PLUMBER", Board::RoughIn, ""),
    ("RI PLUMBER SUP", Board::RoughIn, "Field Supervisor"),
    ("TRIM PLUMBER", Board::Trim, ""),
    ("TRIM PLUMBER SUP", Board::Trim, "Field Supervisor"),
];

/// Supervisors whose people are left out until changed in the preview.
pub const DEFAULT_EXCLUDED: &[&str] = &["Reed, Martin"];
const SETTING: &str = "headcount_exclude_supervisors";

#[derive(Default, Debug)]
pub struct HeadcountPlan {
    /// New employees to add.
    pub add: Vec<Employee>,
    /// Existing employees with their updated details.
    pub update: Vec<Employee>,
    /// Already in the app with nothing to change.
    pub unchanged: usize,
    /// Supervisor -> number of people in imported positions under them.
    pub supervisors: BTreeMap<String, usize>,
    /// Left out because of their supervisor: (name, supervisor).
    pub skipped_supervisor: Vec<(String, String)>,
    /// Left out because of their position: (name, position description).
    pub skipped_position: Vec<(String, String)>,
    /// Left out because they have a termination date.
    pub skipped_terminated: Vec<String>,
}

impl HeadcountPlan {
    pub fn count(&self, b: Board) -> usize {
        self.add.iter().chain(&self.update).filter(|e| e.staff == b).count()
    }
}

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Works out who to add or update. `excluded` holds supervisor names
/// ("Last, First") whose people are left out.
pub fn plan(csv: &str, existing: &[Employee], excluded: &BTreeSet<String>) -> Result<HeadcountPlan, String> {
    let mut lines = csv.lines().filter(|l| !l.trim().is_empty());
    let header: Vec<String> = csv_fields(lines.next().ok_or("The file is empty.")?.trim_start_matches('\u{feff}'))
        .into_iter()
        .map(|h| norm(&h))
        .collect();
    let col = |name: &str| header.iter().position(|h| h == name);
    let need = |name: &str| col(name).ok_or_else(|| format!("No “{name}” column in the header row."));
    let (c_id, c_last, c_first) = (need("employee id")?, need("last name")?, need("preferred/first name")?);
    let c_pos = need("position code")?;
    let (c_sup, c_lic, c_desc, c_term) =
        (col("supervisor"), col("license level"), col("position description"), col("termination date"));
    let excluded: BTreeSet<String> = excluded.iter().map(|s| norm(s)).collect();

    let mut plan = HeadcountPlan::default();
    let mut taken: BTreeSet<i64> = BTreeSet::new();
    for line in lines {
        let f = csv_fields(line);
        let get = |i: Option<usize>| i.and_then(|i| f.get(i)).map(|s| s.trim()).unwrap_or("");
        let name = format!("{} {}", get(Some(c_first)), get(Some(c_last))).trim().to_string();
        if name.is_empty() {
            continue;
        }
        let code = norm(get(Some(c_pos)));
        let Some(&(_, staff, role)) = POSITIONS.iter().find(|(p, _, _)| norm(p) == code) else {
            let desc = if get(c_desc).is_empty() { get(Some(c_pos)) } else { get(c_desc) };
            plan.skipped_position.push((name, desc.to_string()));
            continue;
        };
        if !get(c_term).is_empty() {
            plan.skipped_terminated.push(name);
            continue;
        }
        let sup = get(c_sup).to_string();
        if !sup.is_empty() {
            *plan.supervisors.entry(sup.clone()).or_default() += 1;
        }
        if excluded.contains(&norm(&sup)) {
            plan.skipped_supervisor.push((name, sup));
            continue;
        }
        let emp_no = get(Some(c_id)).to_string();
        let license = get(c_lic).to_string();
        // Already here? Same employee ID, or else the same name without an ID yet.
        let found = existing
            .iter()
            .filter(|e| !taken.contains(&e.id))
            .find(|e| !emp_no.is_empty() && e.employee_no.trim() == emp_no)
            .or_else(|| existing.iter().find(|e| !taken.contains(&e.id) && e.employee_no.trim().is_empty() && norm(&e.name) == norm(&name)));
        match found {
            Some(old) => {
                taken.insert(old.id);
                let mut e = old.clone();
                e.name = name;
                e.employee_no = emp_no;
                e.license = license;
                e.staff = staff;
                e.active = true;
                if !role.is_empty() {
                    e.role = role.to_string();
                }
                if e == *old {
                    plan.unchanged += 1;
                } else {
                    plan.update.push(e);
                }
            }
            None => {
                let mut e = Employee::new();
                e.name = name;
                e.employee_no = emp_no;
                e.license = license;
                e.staff = staff;
                e.role = role.to_string();
                plan.add.push(e);
            }
        }
    }
    let by_name = |a: &Employee, b: &Employee| norm(&a.name).cmp(&norm(&b.name));
    plan.add.sort_by(by_name);
    plan.update.sort_by(by_name);
    Ok(plan)
}

/// An import waiting for the user to check the preview.
pub struct HeadcountImport {
    file: String,
    text: String,
    excluded: BTreeSet<String>,
    plan: HeadcountPlan,
}

/// Asks for the headcount file and opens the preview.
pub fn start(app: &mut SchedulerApp) {
    let Some(path) = rfd::FileDialog::new()
        .set_title("Import employees from the Active Headcount report")
        .add_filter("CSV report", &["csv", "txt"])
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
    let excluded: BTreeSet<String> = match app.db.get_setting(SETTING) {
        Some(s) => s.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect(),
        None => DEFAULT_EXCLUDED.iter().map(|s| s.to_string()).collect(),
    };
    match plan(&text, &app.employees, &excluded) {
        Ok(plan) => app.headcount = Some(HeadcountImport { file, text, excluded, plan }),
        Err(e) => app.set_error(format!("{file}: {e}")),
    }
}

fn people_list(ui: &mut egui::Ui, id: &str, list: &[&Employee]) {
    Grid::new(id).striped(true).spacing([16.0, 3.0]).show(ui, |ui| {
        for e in list {
            ui.label(&e.name);
            ui.label(RichText::new(if e.license.is_empty() { "—" } else { &e.license }).weak());
            ui.label(RichText::new(&e.role).weak());
            ui.end_row();
        }
    });
}

/// The preview window: who will be added or updated, who's left out and why.
pub fn window(app: &mut SchedulerApp, ctx: &egui::Context) {
    let Some(imp) = app.headcount.as_mut() else { return };
    let mut open = true;
    let (mut go, mut cancel, mut changed) = (false, false, false);
    egui::Window::new("Import Employees")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(600.0)
        .default_height(620.0)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            let p = &imp.plan;
            ui.label(RichText::new(&imp.file).weak());
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "{} to add, {} to update  ·  Rough-In Staff {}, Trim Staff {}",
                    p.add.len(),
                    p.update.len(),
                    p.count(Board::RoughIn),
                    p.count(Board::Trim)
                ))
                .strong()
                .size(15.0),
            );
            if p.unchanged > 0 {
                ui.weak(format!("{} already here and up to date.", p.unchanged));
            }
            ui.weak("Positions taken: RI Plumber and RI Plumber Supervisor (Rough-In), Trim Plumber and Trim Plumber Supervisor (Trim).");
            ui.add_space(6.0);
            ui.label(RichText::new("Leave out people supervised by:").strong());
            ui.horizontal_wrapped(|ui| {
                for (sup, n) in &p.supervisors {
                    let mut ticked = imp.excluded.iter().any(|x| norm(x) == norm(sup));
                    if ui.checkbox(&mut ticked, format!("{sup} ({n})")).changed() {
                        imp.excluded.retain(|x| norm(x) != norm(sup));
                        if ticked {
                            imp.excluded.insert(sup.clone());
                        }
                        changed = true;
                    }
                }
            });
            ui.separator();
            ScrollArea::vertical().max_height(400.0).auto_shrink([false, true]).show(ui, |ui| {
                for b in Board::ALL {
                    let new: Vec<&Employee> = p.add.iter().filter(|e| e.staff == b).collect();
                    let upd: Vec<&Employee> = p.update.iter().filter(|e| e.staff == b).collect();
                    CollapsingHeader::new(RichText::new(format!("{} Staff: {} new, {} updated", b.as_str(), new.len(), upd.len())).strong())
                        .id_salt(("hc_staff", b.idx()))
                        .default_open(true)
                        .show(ui, |ui| {
                            people_list(ui, &format!("hc_new_{}", b.idx()), &new);
                            if !upd.is_empty() {
                                ui.add_space(4.0);
                                ui.weak("Updated (crew, phone and notes are kept):");
                                people_list(ui, &format!("hc_upd_{}", b.idx()), &upd);
                            }
                        });
                }
                CollapsingHeader::new(format!(
                    "Left out: {} by supervisor, {} other positions, {} terminated",
                    p.skipped_supervisor.len(),
                    p.skipped_position.len(),
                    p.skipped_terminated.len()
                ))
                .id_salt("hc_skipped")
                .show(ui, |ui| {
                    Grid::new("hc_skipped_grid").striped(true).spacing([16.0, 3.0]).show(ui, |ui| {
                        for (n, s) in &p.skipped_supervisor {
                            ui.label(n);
                            ui.label(RichText::new(format!("supervisor {s}")).weak());
                            ui.end_row();
                        }
                        for (n, d) in &p.skipped_position {
                            ui.label(n);
                            ui.label(RichText::new(d).weak());
                            ui.end_row();
                        }
                        for n in &p.skipped_terminated {
                            ui.label(n);
                            ui.label(RichText::new("terminated").weak());
                            ui.end_row();
                        }
                    });
                });
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let n = p.add.len() + p.update.len();
                let label = RichText::new(format!("Import {n} employee{}", if n == 1 { "" } else { "s" })).strong();
                if ui.add_enabled(n > 0, Button::new(label).fill(Color32::from_rgb(210, 235, 215))).clicked() {
                    go = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });

    if changed {
        if let Ok(p) = plan(&imp.text, &app.employees, &imp.excluded) {
            imp.plan = p;
        }
    }
    if go {
        let imp = app.headcount.take().unwrap();
        let all: Vec<Employee> = imp.plan.add.iter().chain(&imp.plan.update).cloned().collect();
        match app.db.save_employees(&all) {
            Ok(()) => {
                let list: Vec<&str> = imp.excluded.iter().map(|s| s.as_str()).collect();
                let _ = app.db.set_setting(SETTING, &list.join("\n"));
                app.reload();
                app.set_status(format!(
                    "Imported from {}: {} added, {} updated (Rough-In {}, Trim {}).",
                    imp.file,
                    imp.plan.add.len(),
                    imp.plan.update.len(),
                    imp.plan.count(Board::RoughIn),
                    imp.plan.count(Board::Trim)
                ));
            }
            Err(e) => app.set_error(format!("Import failed, nothing was changed: {e}")),
        }
    } else if cancel || !open {
        app.headcount = None;
    }
}

/// Command line: `--import-headcount <file.csv> [--apply]`, using the saved
/// (or default) supervisor exclusions. Returns the exit code.
pub fn run(mut db: crate::db::Db, path: &str, apply: bool) -> i32 {
    let text = match std::fs::read(path) {
        Ok(b) => String::from_utf8_lossy(&b).to_string(),
        Err(e) => {
            eprintln!("Can't read {path}: {e}");
            return 1;
        }
    };
    let excluded: BTreeSet<String> = match db.get_setting(SETTING) {
        Some(s) => s.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect(),
        None => DEFAULT_EXCLUDED.iter().map(|s| s.to_string()).collect(),
    };
    let existing = match db.load_employees() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Can't read the database: {e}");
            return 1;
        }
    };
    let p = match plan(&text, &existing, &excluded) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    for b in Board::ALL {
        println!("== {} Staff ({})", b.as_str(), p.count(b));
        for (tag, list) in [("add", &p.add), ("update", &p.update)] {
            for e in list.iter().filter(|e| e.staff == b) {
                println!("  {tag:<6} {:<28} {:<11} {:<17} #{}", e.name, e.license, e.role, e.employee_no);
            }
        }
    }
    println!(
        "\nLeft out: {} supervised by {}, {} other positions, {} terminated. Already up to date: {}.",
        p.skipped_supervisor.len(),
        excluded.iter().cloned().collect::<Vec<_>>().join(" / "),
        p.skipped_position.len(),
        p.skipped_terminated.len(),
        p.unchanged
    );
    println!("{} to add, {} to update.", p.add.len(), p.update.len());
    if !apply {
        println!("Nothing saved. Run again with --apply to save.");
        return 0;
    }
    let all: Vec<Employee> = p.add.iter().chain(&p.update).cloned().collect();
    match db.save_employees(&all) {
        Ok(()) => {
            println!("Saved.");
            0
        }
        Err(e) => {
            eprintln!("Import failed, nothing was saved: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CSV: &str = r#""Company Code","Employee Id","Last Name","Preferred/First Name","Project Description","License Level","UnitLot Description","Work Type Description","Employee Status Code","Hire Date","Rehire Date","Termination Date","Supervisor","Position Code","Position Description"
"100001","2009","Ashby","Daniel","Field Plumbers","Apprentice","NA","Trim","A",06/01/2020,,,"Grant, Alan","TRIM PLUMBER","Trim Plumber"
"100001","201","Hale Jr","Victor","Officers","Journey","NA","NA","A",01/01/1990,,,"Blake, Nora","CEO","CEO"
"100001","2016","Marsh","Isaiah","Field Plumbers","Resident","NA","Rough-In","A",08/01/2020,,,"Prescott, Dustin","RI PLUMBER","Rough-in Plumber"
"100001","2018","Holt","Jeremy","Field Plumbers","Apprentice","NA","NA","A",09/01/2020,,,"Reed, Martin","RI PLUMBER","Rough-in Plumber"
"100001","574","Moreland","Keith","Field Supervisors","Resident","NA","NA","A",09/01/2005,,,"Prescott, Dustin","RI PLUMBER SUP","Rough-in plumber Supervisor"
"100001","652","Varga","Jason","Field Supervisors","Resident","NA","Trim","A",05/01/2007,,,"Grant, Alan","TRIM PLUMBER SUP","Trim Plumber Supervisor"
"100001","799","Gone","Old","Field Plumbers","Apprentice","NA","Rough-In","A",03/01/2020,,01/01/2026,"Prescott, Dustin","RI PLUMBER","Rough-in Plumber"
"#;

    fn foster() -> BTreeSet<String> {
        DEFAULT_EXCLUDED.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn takes_plumbers_sorted_into_staff_lists() {
        let p = plan(CSV, &[], &foster()).unwrap();
        let names: Vec<(&str, Board, &str, &str)> =
            p.add.iter().map(|e| (e.name.as_str(), e.staff, e.license.as_str(), e.role.as_str())).collect();
        assert_eq!(
            names,
            vec![
                ("Daniel Ashby", Board::Trim, "Apprentice", ""),
                ("Isaiah Marsh", Board::RoughIn, "Resident", ""),
                ("Jason Varga", Board::Trim, "Resident", "Field Supervisor"),
                ("Keith Moreland", Board::RoughIn, "Resident", "Field Supervisor"),
            ]
        );
        assert_eq!(p.add[1].employee_no, "2016");
        assert_eq!(p.skipped_supervisor, vec![("Jeremy Holt".to_string(), "Reed, Martin".to_string())]);
        assert_eq!(p.skipped_position, vec![("Victor Hale Jr".to_string(), "CEO".to_string())]);
        assert_eq!(p.skipped_terminated, vec!["Old Gone".to_string()]);
        assert_eq!((p.count(Board::RoughIn), p.count(Board::Trim)), (2, 2));
        assert_eq!(p.supervisors.get("Reed, Martin"), Some(&1));
        // Unticking Foster brings his people in.
        let p = plan(CSV, &[], &BTreeSet::new()).unwrap();
        assert!(p.add.iter().any(|e| e.name == "Jeremy Holt"));
    }

    #[test]
    fn reimport_updates_instead_of_duplicating() {
        let mut db = crate::db::Db::open_in_memory().unwrap();
        // Someone already typed in by hand, with a crew-independent phone number.
        let mut by_hand = Employee::new();
        by_hand.name = "isaiah  marsh".into();
        by_hand.phone = "555-0100".into();
        by_hand.staff = Board::Trim;
        db.save_employee(&by_hand).unwrap();

        let p = plan(CSV, &db.load_employees().unwrap(), &foster()).unwrap();
        assert_eq!((p.add.len(), p.update.len()), (3, 1));
        let all: Vec<Employee> = p.add.iter().chain(&p.update).cloned().collect();
        db.save_employees(&all).unwrap();
        let emps = db.load_employees().unwrap();
        assert_eq!(emps.len(), 4);
        let ib = emps.iter().find(|e| e.employee_no == "2016").unwrap();
        assert_eq!((ib.name.as_str(), ib.staff, ib.phone.as_str(), ib.license.as_str()), ("Isaiah Marsh", Board::RoughIn, "555-0100", "Resident"));

        // Same file again: nothing to add, nothing to change.
        let p = plan(CSV, &emps, &foster()).unwrap();
        assert_eq!((p.add.len(), p.update.len(), p.unchanged), (0, 0, 4));
    }
}
