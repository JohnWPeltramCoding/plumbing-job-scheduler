//! Employee list. Employees are created here; crews are built from them on
//! the Crews tab. An employee is on at most one crew at a time.

use eframe::egui::{self, vec2, Align, Button, Color32, Grid, Layout, RichText, ScrollArea, Sense, TextEdit};

use crate::app::{SchedulerApp, StaffPage, Tab};
use crate::model::*;

const AVAILABLE_GREEN: Color32 = Color32::from_rgb(39, 150, 80);

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum EmpFilter {
    #[default]
    All,
    Available,
    OnCrew,
    Inactive,
}

#[derive(Default)]
pub struct EmployeesState {
    pub selected: Option<i64>,
    /// Working copy being edited (id 0 = new employee).
    pub draft: Option<Employee>,
    filter: EmpFilter,
    search: String,
    confirm_delete: bool,
    /// Details or Driver screen for the selected employee.
    page: EmpPage,
    pub driver: crate::driver::DriverState,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum EmpPage {
    #[default]
    Details,
    Driver,
}

enum EmpAction {
    Select(i64),
    New,
    Save,
    Revert,
    Delete(i64),
    /// Move an employee to a crew (None = no crew) in one step.
    MoveTo(i64, Option<i64>),
    OpenCrew(i64),
    ImportHeadcount,
}

/// True when the editable details differ (crew membership is managed elsewhere).
fn details_differ(a: &Employee, b: &Employee) -> bool {
    a.name != b.name
        || a.role != b.role
        || a.phone != b.phone
        || a.active != b.active
        || a.notes != b.notes
        || a.staff != b.staff
        || a.license != b.license
        || a.employee_no != b.employee_no
}

pub fn show(app: &mut SchedulerApp, ctx: &egui::Context, board: Board) {
    let bi = board.idx();
    let state = &mut app.employees_state[bi];
    if state.draft.is_none() && state.selected.is_none() {
        if let Some(e) = app.employees.iter().find(|e| e.staff == board) {
            state.selected = Some(e.id);
            state.draft = Some(e.clone());
        }
    }
    let original = state.selected.and_then(|id| app.employees.iter().find(|e| e.id == id).cloned());
    // Crew membership can change from the Crews tab; keep the draft in sync.
    if let (Some(d), Some(o)) = (state.draft.as_mut(), original.as_ref()) {
        d.crew_id = o.crew_id;
    }
    let dirty = match (&state.draft, &original) {
        (Some(d), Some(o)) => details_differ(d, o),
        (Some(d), None) => d.id == 0,
        _ => false,
    };

    let crews = &app.crews;
    let crew_of = |id: Option<i64>| id.and_then(|id| crews.iter().find(|c| c.id == id));
    let mut actions: Vec<EmpAction> = Vec::new();
    let editable = app.can_edit();
    let roster_day = app.roster_day;
    let today = crate::app::today();
    let mut driver_actions: Vec<crate::driver::DriverAction> = Vec::new();

    egui::SidePanel::left("employee_list")
        .resizable(true)
        .default_width(330.0)
        .width_range(260.0..=480.0)
        .show(ctx, |ui| {
            let state = &mut app.employees_state[bi];
            ui.add_space(6.0);
            ui.heading(format!("{} Employees", board.as_str()));
            ui.horizontal(|ui| {
                if ui.add_enabled(editable, Button::new("+ New Employee")).clicked() {
                    actions.push(EmpAction::New);
                }
                if ui
                    .add_enabled(editable, Button::new("⬆ Import Headcount…"))
                    .on_hover_text("Add or update plumbers from the payroll Active Headcount report (.csv). You'll see a preview before anything is saved.")
                    .clicked()
                {
                    actions.push(EmpAction::ImportHeadcount);
                }
            });
            ui.add(
                TextEdit::singleline(&mut state.search)
                    .hint_text("Search name, role, license, phone, ID…")
                    .desired_width(f32::INFINITY),
            );
            let mine = || app.employees.iter().filter(|e| e.staff == board);
            let count = |f: &dyn Fn(&Employee) -> bool| mine().filter(|e| f(e)).count();
            ui.horizontal_wrapped(|ui| {
                let filters = [
                    (EmpFilter::All, format!("All ({})", mine().count())),
                    (EmpFilter::Available, format!("Available ({})", count(&|e| e.active && e.crew_id.is_none()))),
                    (EmpFilter::OnCrew, format!("On a crew ({})", count(&|e| e.crew_id.is_some()))),
                    (EmpFilter::Inactive, format!("Inactive ({})", count(&|e| !e.active))),
                ];
                for (f, label) in filters {
                    ui.selectable_value(&mut state.filter, f, label);
                }
            });
            ui.separator();

            if mine().next().is_none() {
                ui.weak("No employees yet. Click “+ New Employee” to add your first one.");
            }
            let q = state.search.trim().to_lowercase();
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for e in mine() {
                    let keep = match state.filter {
                        EmpFilter::All => true,
                        EmpFilter::Available => e.active && e.crew_id.is_none(),
                        EmpFilter::OnCrew => e.crew_id.is_some(),
                        EmpFilter::Inactive => !e.active,
                    };
                    if !keep {
                        continue;
                    }
                    if !q.is_empty()
                        && !format!("{} {} {} {} {}", e.name, e.role, e.phone, e.license, e.employee_no).to_lowercase().contains(&q)
                    {
                        continue;
                    }
                    let selected = state.selected == Some(e.id);
                    let crew = crew_of(e.crew_id);
                    ui.horizontal(|ui| {
                        let mut name = RichText::new(&e.name).size(14.0);
                        if !e.active {
                            name = name.weak().italics();
                        }
                        let resp = ui.selectable_label(selected, name);
                        let detail = [e.role.as_str(), e.license.as_str()]
                            .iter()
                            .filter(|s| !s.is_empty())
                            .copied()
                            .collect::<Vec<_>>()
                            .join(" · ");
                        if !detail.is_empty() {
                            ui.weak(detail);
                        }
                        if let Some(w) = app.licenses.get(&e.id).and_then(|l| l.warning(today)) {
                            ui.label(RichText::new("⚠").color(Color32::from_rgb(210, 50, 50))).on_hover_text(w);
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            match crew {
                                Some(c) => {
                                    ui.label(RichText::new(&c.name).small());
                                    let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                                    ui.painter().rect_filled(r, 2.0, c.color32());
                                }
                                None if e.active => {
                                    ui.label(RichText::new("Available").small().color(AVAILABLE_GREEN));
                                }
                                None => {
                                    ui.label(RichText::new("Inactive").small().weak());
                                }
                            }
                        });
                        if resp.clicked() && !selected {
                            actions.push(EmpAction::Select(e.id));
                        }
                    });
                }
            });
        });

    egui::CentralPanel::default().show(ctx, |ui| {
        if !editable {
            ui.disable();
        }
        let state = &mut app.employees_state[bi];
        let Some(draft) = state.draft.as_mut() else {
            ui.centered_and_justified(|ui| {
                ui.label("Select an employee on the left, or click “+ New Employee”.");
            });
            return;
        };

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let title = if draft.name.trim().is_empty() { "New Employee".to_string() } else { draft.name.clone() };
            ui.heading(title);
            if dirty {
                ui.colored_label(Color32::from_rgb(230, 150, 30), "(unsaved changes)");
            }
        });
        // Two screens per employee: their details, and driver information.
        ui.horizontal(|ui| {
            ui.selectable_value(&mut state.page, EmpPage::Details, RichText::new("Details").size(15.0));
            let driver_tab = ui.add_enabled(
                draft.id > 0,
                egui::SelectableLabel::new(state.page == EmpPage::Driver, RichText::new("Driver").size(15.0)),
            );
            if driver_tab.clicked() {
                state.page = EmpPage::Driver;
            }
            driver_tab.on_disabled_hover_text("Save the employee first");
            if let Some(w) = app.licenses.get(&draft.id).and_then(|l| l.warning(today)) {
                ui.colored_label(Color32::from_rgb(210, 50, 50), format!("⚠ {w}"));
            }
        });
        ui.separator();
        if state.page == EmpPage::Driver && draft.id > 0 {
            crate::driver::page(
                ui,
                draft,
                &app.licenses,
                &app.incidents,
                &app.vehicles,
                &mut state.driver,
                editable,
                &mut driver_actions,
            );
            return;
        }
        ui.add_space(8.0);

        Grid::new("employee_form").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Name");
            ui.add(TextEdit::singleline(&mut draft.name).desired_width(280.0).hint_text("First and last name"));
            ui.end_row();

            ui.label("Role");
            ui.horizontal(|ui| {
                ui.add(TextEdit::singleline(&mut draft.role).desired_width(160.0).hint_text("e.g. Journeyman"));
                ui.menu_button("Presets", |ui| {
                    for role in MEMBER_ROLES {
                        if ui.button(*role).clicked() {
                            draft.role = role.to_string();
                            ui.close_menu();
                        }
                    }
                });
            });
            ui.end_row();

            ui.label("License level");
            ui.horizontal(|ui| {
                for level in LICENSE_LEVELS {
                    if ui.selectable_label(draft.license == *level, *level).clicked() {
                        draft.license = if draft.license == *level { String::new() } else { level.to_string() };
                    }
                }
                if !draft.license.is_empty() && !LICENSE_LEVELS.contains(&draft.license.as_str()) {
                    ui.weak(format!("({})", draft.license));
                }
            });
            ui.end_row();

            ui.label("Employee ID");
            ui.add(TextEdit::singleline(&mut draft.employee_no).desired_width(120.0).hint_text("payroll ID"))
                .on_hover_text("Used to match this person when the headcount report is imported again");
            ui.end_row();

            ui.label("Phone");
            ui.add(TextEdit::singleline(&mut draft.phone).desired_width(200.0));
            ui.end_row();

            ui.label("Active");
            ui.checkbox(&mut draft.active, "Currently employed (inactive employees can't be added to crews)");
            ui.end_row();

            ui.label("Staff list");
            ui.horizontal(|ui| {
                for b in Board::ALL {
                    ui.selectable_value(&mut draft.staff, b, b.as_str());
                }
                if draft.staff != board {
                    let note = if draft.crew_id.is_some() {
                        format!("Saving moves them to {} Staff and takes them off their current crew.", draft.staff.as_str())
                    } else {
                        format!("Saving moves them to the {} Staff list.", draft.staff.as_str())
                    };
                    ui.colored_label(Color32::from_rgb(230, 150, 30), note);
                }
            });
            ui.end_row();

            if draft.staff == Board::RoughIn {
                ui.label(format!("Crew on {}", roster_day.format("%a %b %-d")))
                    .on_hover_text("Rough-In crews are set day by day. Pick the day at the top of the page.");
            } else {
                ui.label("Crew");
            }
            ui.horizontal(|ui| {
                if draft.id == 0 {
                    ui.weak("Save the employee first, then pick their crew here.");
                    return;
                }
                let current = crew_of(draft.crew_id);
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                ui.painter().rect_filled(r, 3.0, current.map(|c| c.color32()).unwrap_or(AVAILABLE_GREEN));
                let selected = match current {
                    Some(c) => RichText::new(&c.name).strong(),
                    None => RichText::new("No crew (available)").color(AVAILABLE_GREEN).strong(),
                };
                // Picking a crew moves them there in one step (off their old crew).
                ui.add_enabled_ui(draft.active || draft.crew_id.is_some(), |ui| {
                    egui::ComboBox::from_id_salt("emp_crew_pick")
                        .selected_text(selected)
                        .width(230.0)
                        .show_ui(ui, |ui| {
                            // Only crews from this employee's staff group.
                            for c in crews
                                .iter()
                                .filter(|c| c.board == draft.staff && (c.active || Some(c.id) == draft.crew_id))
                            {
                                ui.horizontal(|ui| {
                                    let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                                    ui.painter().rect_filled(r, 2.0, c.color32());
                                    let is_current = draft.crew_id == Some(c.id);
                                    if ui.selectable_label(is_current, c.name.as_str()).clicked() && !is_current {
                                        actions.push(EmpAction::MoveTo(draft.id, Some(c.id)));
                                    }
                                });
                            }
                            ui.separator();
                            let none = draft.crew_id.is_none();
                            if ui.selectable_label(none, "No crew (available)").clicked() && !none {
                                actions.push(EmpAction::MoveTo(draft.id, None));
                            }
                        })
                        .response
                        .on_hover_text("Choose a crew to move them there right away");
                });
                if let Some(c) = current {
                    if ui.button("Open crew").clicked() {
                        actions.push(EmpAction::OpenCrew(c.id));
                    }
                }
                if !draft.active && draft.crew_id.is_none() {
                    ui.weak("Inactive employees can't be put on a crew.");
                }
            });
            ui.end_row();

            ui.label("Notes");
            ui.add(TextEdit::multiline(&mut draft.notes).desired_rows(3).desired_width(420.0));
            ui.end_row();
        });

        ui.add_space(12.0);
        ui.separator();
        ui.horizontal(|ui| {
            let can_save = dirty && !draft.name.trim().is_empty();
            if ui.add_enabled(can_save, Button::new(RichText::new("💾 Save Employee").strong())).clicked() {
                actions.push(EmpAction::Save);
            }
            if ui.add_enabled(dirty, Button::new("Discard Changes")).clicked() {
                actions.push(EmpAction::Revert);
            }
            if dirty && draft.name.trim().is_empty() {
                ui.weak("Name is required");
            }
            if draft.id > 0 {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if state.confirm_delete {
                        if ui.button("Keep").clicked() {
                            state.confirm_delete = false;
                        }
                        if ui
                            .button(RichText::new("Confirm Delete").color(Color32::from_rgb(220, 50, 50)))
                            .clicked()
                        {
                            actions.push(EmpAction::Delete(draft.id));
                        }
                        ui.weak("This also removes them from their crew.");
                    } else if ui.button("🗑 Delete Employee").clicked() {
                        state.confirm_delete = true;
                    }
                });
            }
        });
    });

    let license_dirty = app.employees_state[bi].driver.license_dirty(&app.licenses);
    for a in actions {
        match a {
            EmpAction::Select(_) | EmpAction::New if license_dirty => {
                app.set_error("Save or discard the driver's license changes first.");
            }
            EmpAction::ImportHeadcount if dirty => {
                app.set_error("Save or discard your employee changes first.");
            }
            EmpAction::ImportHeadcount => crate::employee_import::start(app),
            EmpAction::Select(_) | EmpAction::New if dirty => {
                app.set_error("Save or discard your employee changes first.");
            }
            EmpAction::Select(id) => {
                let s = &mut app.employees_state[bi];
                s.selected = Some(id);
                s.draft = app.employees.iter().find(|e| e.id == id).cloned();
                s.confirm_delete = false;
            }
            EmpAction::New => {
                let s = &mut app.employees_state[bi];
                s.selected = None;
                let mut e = Employee::new();
                e.staff = board;
                s.draft = Some(e);
                s.confirm_delete = false;
            }
            EmpAction::Save => {
                let Some(draft) = app.employees_state[bi].draft.clone() else { continue };
                match app.db.save_employee(&draft) {
                    Ok(id) => {
                        app.reload();
                        if draft.staff != board {
                            // They now appear on the other staff tab.
                            app.employees_state[bi] = EmployeesState::default();
                            app.set_status(format!(
                                "Moved {} to the {} Staff list.",
                                draft.name.trim(),
                                draft.staff.as_str()
                            ));
                        } else {
                            app.employees_state[bi].selected = Some(id);
                            app.employees_state[bi].draft = app.employees.iter().find(|e| e.id == id).cloned();
                            app.set_status(format!("Saved employee {}", draft.name.trim()));
                        }
                    }
                    Err(e) => app.set_error(format!("Could not save employee: {e}")),
                }
            }
            EmpAction::Revert => {
                app.employees_state[bi].draft = original.clone();
                if original.is_none() {
                    app.employees_state[bi].selected = None;
                }
            }
            EmpAction::MoveTo(id, crew) => match app.move_employee(id, crew) {
                Ok(msg) => app.set_status(msg),
                Err(e) => app.set_error(e),
            },
            EmpAction::Delete(id) => match app.db.delete_employee(id) {
                Ok(()) => {
                    app.reload();
                    app.employees_state[bi] = EmployeesState::default();
                    app.set_status("Employee deleted.");
                }
                Err(e) => app.set_error(format!("Could not delete employee: {e}")),
            },
            EmpAction::OpenCrew(crew_id) => {
                let crew_board = app.crews.iter().find(|c| c.id == crew_id).map(|c| c.board).unwrap_or(board);
                app.tab = Tab::Staff(crew_board, StaffPage::Crews);
                let cs = &mut app.crews_state[crew_board.idx()];
                let crew_dirty = match (&cs.draft, cs.selected.and_then(|id| app.crews.iter().find(|c| c.id == id))) {
                    (Some(d), Some(o)) => d != o,
                    (Some(d), None) => d.id == 0,
                    _ => false,
                };
                if !crew_dirty {
                    cs.selected = Some(crew_id);
                    cs.draft = app.crews.iter().find(|c| c.id == crew_id).cloned();
                }
            }
        }
    }

    // Driver screen: the incident editor window, then whatever it asked for.
    let emp_id = app.employees_state[bi].selected.unwrap_or(0);
    let who = app.employees.iter().find(|e| e.id == emp_id).map(|e| e.name.clone()).unwrap_or_default();
    crate::driver::incident_window(ctx, &mut app.employees_state[bi].driver, &app.vehicles, &who, &mut driver_actions);
    if !driver_actions.is_empty() {
        crate::driver::apply(app, bi, emp_id, driver_actions);
    }
}
