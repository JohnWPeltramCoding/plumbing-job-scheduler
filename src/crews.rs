use eframe::egui::{self, vec2, Align, Button, Color32, Grid, Layout, RichText, ScrollArea, Sense, TextEdit};

use crate::app::{today, SchedulerApp, StaffPage, Tab};
use crate::model::*;

#[derive(Default)]
pub struct CrewsState {
    pub selected: Option<i64>,
    /// Working copy being edited (id 0 = new crew).
    pub draft: Option<Crew>,
    confirm_delete: bool,
}

enum CrewAction {
    Select(i64),
    New,
    Save,
    Revert,
    Delete(i64),
    OpenJob(i64),
    OpenEmployees,
}

pub fn show(app: &mut SchedulerApp, ctx: &egui::Context, board: Board) {
    let bi = board.idx();
    // Auto-select the first crew when arriving on the tab.
    if app.crews_state[bi].draft.is_none() && app.crews_state[bi].selected.is_none() {
        if let Some(c) = app.crews.iter().find(|c| c.board == board) {
            app.crews_state[bi].selected = Some(c.id);
            app.crews_state[bi].draft = Some(c.clone());
        }
    }
    let original = app
        .crews_state[bi]
        .selected
        .and_then(|id| app.crews.iter().find(|c| c.id == id).cloned());
    // Drop members that were deleted from the employee list.
    if let Some(d) = app.crews_state[bi].draft.as_mut() {
        d.member_ids.retain(|id| app.employees.iter().any(|e| e.id == *id));
    }
    let dirty = match (&app.crews_state[bi].draft, &original) {
        (Some(d), Some(o)) => d != o,
        (Some(d), None) => d.id == 0,
        _ => false,
    };

    let mut actions: Vec<CrewAction> = Vec::new();
    let editable = app.can_edit();
    let today = today();
    let roster_day = app.roster_day;

    egui::SidePanel::left("crew_list")
        .resizable(true)
        .default_width(280.0)
        .width_range(220.0..=420.0)
        .show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading(format!("{} Crews", board.as_str()));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add_enabled(editable, Button::new("+ New Crew")).clicked() {
                        actions.push(CrewAction::New);
                    }
                });
            });
            ui.separator();
            if !app.crews.iter().any(|c| c.board == board) {
                ui.weak("No crews yet. Click “+ New Crew” to add one.");
            }
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for c in app.crews.iter().filter(|c| c.board == board) {
                    let selected = app.crews_state[bi].selected == Some(c.id);
                    let upcoming = app
                        .jobs
                        .iter()
                        .filter(|j| j.crew_id == Some(c.id) && j.date.is_some_and(|d| d >= today))
                        .count();
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                        ui.painter().rect_filled(r, 3.0, c.color32());
                        let mut text = RichText::new(format!(
                            "{}   ·  {} member{} · {} upcoming",
                            c.name,
                            c.member_ids.len(),
                            if c.member_ids.len() == 1 { "" } else { "s" },
                            upcoming
                        ));
                        if !c.active {
                            text = text.weak().italics();
                        }
                        if ui.selectable_label(selected, text).clicked() && !selected {
                            actions.push(CrewAction::Select(c.id));
                        }
                    });
                }
            });
        });

    egui::CentralPanel::default().show(ctx, |ui| {
        if !editable {
            ui.disable();
        }
        let cs = &mut app.crews_state[bi];
        let Some(draft) = cs.draft.as_mut() else {
            ui.centered_and_justified(|ui| {
                ui.label("Select a crew on the left, or click “+ New Crew”.");
            });
            return;
        };

        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                ui.painter().rect_filled(r, 4.0, draft.color32());
                let title = if draft.name.trim().is_empty() { "New Crew".to_string() } else { draft.name.clone() };
                ui.heading(title);
                if dirty {
                    ui.colored_label(Color32::from_rgb(230, 150, 30), "(unsaved changes)");
                }
            });
            ui.add_space(8.0);

            Grid::new("crew_form").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Crew Name");
                ui.add(TextEdit::singleline(&mut draft.name).desired_width(300.0).hint_text("e.g. Rough-In Crew 1"));
                ui.end_row();
                ui.label("Foreman / Lead");
                ui.horizontal(|ui| {
                    ui.add(TextEdit::singleline(&mut draft.foreman).desired_width(240.0));
                    let members: Vec<&Employee> = draft
                        .member_ids
                        .iter()
                        .filter_map(|id| app.employees.iter().find(|e| e.id == *id))
                        .collect();
                    if !members.is_empty() {
                        ui.menu_button("Pick from crew…", |ui| {
                            for e in members {
                                if ui.button(&e.name).clicked() {
                                    draft.foreman = e.name.clone();
                                    if draft.phone.trim().is_empty() {
                                        draft.phone = e.phone.clone();
                                    }
                                    ui.close_menu();
                                }
                            }
                        });
                    }
                });
                ui.end_row();
                ui.label("Phone");
                ui.add(TextEdit::singleline(&mut draft.phone).desired_width(200.0));
                ui.end_row();
                ui.label("Vehicle").on_hover_text("The truck or van this crew goes out in (from the Vehicles tab)");
                ui.horizontal(|ui| {
                    let current = draft.vehicle_id.and_then(|id| app.vehicles.iter().find(|v| v.id == id));
                    let text = match current {
                        Some(v) => format!("{}  {}", v.label(), v.description()),
                        None => "No vehicle".into(),
                    };
                    egui::ComboBox::from_id_salt("crew_vehicle").selected_text(text).width(300.0).show_ui(ui, |ui| {
                        ui.selectable_value(&mut draft.vehicle_id, None, "No vehicle");
                        ui.separator();
                        for v in &app.vehicles {
                            let driver = v.get("driver");
                            let mut label = format!("{}  {}", v.label(), v.description());
                            if !driver.is_empty() {
                                label.push_str(&format!("  · {driver}"));
                            }
                            if v.status != "Active" {
                                label.push_str(&format!("  · {}", v.status));
                            }
                            ui.selectable_value(&mut draft.vehicle_id, Some(v.id), label);
                        }
                    });
                    if let Some(v) = current {
                        if v.status == "In Shop" || v.status == "Out of Service" {
                            ui.colored_label(Color32::from_rgb(210, 50, 50), format!("⚠ {}", v.status));
                        }
                        let others: Vec<&str> = app
                            .crews
                            .iter()
                            .filter(|c| c.id != draft.id && c.vehicle_id == Some(v.id))
                            .map(|c| c.name.as_str())
                            .collect();
                        if !others.is_empty() {
                            ui.weak(format!("also used by {}", others.join(", ")));
                        }
                    }
                });
                ui.end_row();
                ui.label("Calendar Color");
                ui.horizontal(|ui| {
                    ui.color_edit_button_srgb(&mut draft.color);
                    for c in CREW_PALETTE {
                        let (r, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                        ui.painter().rect_filled(r, 3.0, Color32::from_rgb(c[0], c[1], c[2]));
                        if draft.color == *c {
                            ui.painter().rect_stroke(r.expand(1.5), 3.0, egui::Stroke::new(2.0_f32, ui.visuals().strong_text_color()));
                        }
                        if resp.clicked() {
                            draft.color = *c;
                        }
                    }
                });
                ui.end_row();
                ui.label("Active");
                ui.checkbox(&mut draft.active, "Available for scheduling");
                ui.end_row();
                ui.label("Staff group");
                ui.horizontal(|ui| {
                    for b in Board::ALL {
                        ui.selectable_value(&mut draft.board, b, b.as_str());
                    }
                    if draft.board != board {
                        ui.colored_label(
                            Color32::from_rgb(230, 150, 30),
                            format!("Saving moves this crew and its members to {} Staff.", draft.board.as_str()),
                        );
                    }
                });
                ui.end_row();
                ui.label("Notes");
                ui.add(TextEdit::multiline(&mut draft.notes).desired_rows(2).desired_width(420.0));
                ui.end_row();
            });

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if draft.board == Board::RoughIn && board == Board::RoughIn {
                    ui.label(
                        RichText::new(format!("Crew Members on {}", roster_day.format("%A, %b %-d")))
                            .strong()
                            .size(15.0),
                    );
                    ui.weak("· changes apply to this day only");
                } else {
                    ui.label(RichText::new("Crew Members").strong().size(15.0));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.link("Manage employee list").clicked() {
                        actions.push(CrewAction::OpenEmployees);
                    }
                });
            });
            ui.add_space(4.0);
            let find = |id: i64| app.employees.iter().find(|e| e.id == id);
            let mut remove = None;
            if draft.member_ids.is_empty() {
                ui.weak("No one is on this crew yet.");
            } else {
                Grid::new("crew_members").num_columns(4).spacing([16.0, 6.0]).striped(true).show(ui, |ui| {
                    ui.weak("Name");
                    ui.weak("Role");
                    ui.weak("Phone");
                    ui.label("");
                    ui.end_row();
                    for (i, id) in draft.member_ids.iter().enumerate() {
                        let Some(e) = find(*id) else { continue };
                        let mut name = RichText::new(&e.name).strong();
                        if !e.active {
                            name = name.weak().italics();
                        }
                        ui.horizontal(|ui| {
                            ui.label(name);
                            if !e.active {
                                ui.weak("(inactive)");
                            }
                        });
                        ui.label(&e.role);
                        ui.label(&e.phone);
                        if ui.small_button("Remove").on_hover_text("Take off this crew (they become available)").clicked() {
                            remove = Some(i);
                        }
                        ui.end_row();
                    }
                });
            }
            if let Some(i) = remove {
                draft.member_ids.remove(i);
            }

            // Only employees who are not on any other crew are offered.
            let available: Vec<&Employee> = app
                .employees
                .iter()
                .filter(|e| {
                    e.active
                        && e.staff == draft.board
                        && (e.crew_id.is_none() || (draft.id > 0 && e.crew_id == Some(draft.id)))
                        && !draft.member_ids.contains(&e.id)
                })
                .collect();
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if available.is_empty() {
                    ui.add_enabled(false, Button::new("+ Add Employee"));
                    ui.weak("No available employees. Everyone active is already on a crew.");
                } else {
                    let mut picked = None;
                    egui::ComboBox::from_id_salt("add_member")
                        .selected_text(format!("+ Add Employee ({} available)", available.len()))
                        .width(260.0)
                        .height(360.0)
                        .show_ui(ui, |ui| {
                            for e in &available {
                                let text = if e.role.is_empty() { e.name.clone() } else { format!("{} — {}", e.name, e.role) };
                                if ui.selectable_label(false, text).clicked() {
                                    picked = Some(e.id);
                                }
                            }
                        });
                    if let Some(id) = picked {
                        draft.member_ids.push(id);
                    }
                }
            });
            if dirty {
                if draft.board == Board::RoughIn {
                    ui.weak(format!(
                        "Member changes take effect for {} when you click Save Crew.",
                        roster_day.format("%a %b %-d")
                    ));
                } else {
                    ui.weak("Member changes take effect when you click Save Crew.");
                }
            }

            ui.add_space(12.0);
            ui.separator();
            ui.horizontal(|ui| {
                let can_save = dirty && !draft.name.trim().is_empty();
                if ui.add_enabled(can_save, Button::new(RichText::new("💾 Save Crew").strong())).clicked() {
                    actions.push(CrewAction::Save);
                }
                if ui.add_enabled(dirty, Button::new("Discard Changes")).clicked() {
                    actions.push(CrewAction::Revert);
                }
                if dirty && draft.name.trim().is_empty() {
                    ui.weak("Crew name is required");
                }
                if draft.id > 0 {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if cs.confirm_delete {
                            if ui.button("Keep").clicked() {
                                cs.confirm_delete = false;
                            }
                            if ui
                                .button(RichText::new("Confirm Delete").color(Color32::from_rgb(220, 50, 50)))
                                .clicked()
                            {
                                actions.push(CrewAction::Delete(draft.id));
                            }
                            ui.weak("Its jobs become Unassigned and its members become available.");
                        } else if ui.button("🗑 Delete Crew").clicked() {
                            cs.confirm_delete = true;
                        }
                    });
                }
            });

            // Upcoming schedule for this crew.
            if draft.id > 0 {
                ui.add_space(14.0);
                ui.label(RichText::new("Upcoming Jobs").strong().size(15.0));
                ui.add_space(4.0);
                let mut upcoming: Vec<&Job> = app
                    .jobs
                    .iter()
                    .filter(|j| j.crew_id == Some(draft.id) && j.date.is_some_and(|d| d >= today))
                    .collect();
                upcoming.sort_by_key(|j| (j.date, j.start_min));
                if upcoming.is_empty() {
                    ui.weak("Nothing scheduled.");
                }
                Grid::new("crew_upcoming").num_columns(4).spacing([16.0, 4.0]).striped(true).show(ui, |ui| {
                    for j in upcoming.iter().take(25) {
                        let d = j.date.map(|d| d.format("%a %b %-d").to_string()).unwrap_or_default();
                        ui.label(d);
                        ui.label(fmt_time(j.start_min));
                        if ui.link(j.short_title()).clicked() {
                            actions.push(CrewAction::OpenJob(j.id));
                        }
                        ui.weak(format!("{} · {}  {}", j.board.as_str(), j.phase, j.status));
                        ui.end_row();
                    }
                });
            }
        });
    });

    for a in actions {
        match a {
            CrewAction::Select(_) | CrewAction::New if dirty => {
                app.set_error("Save or discard your crew changes first.");
            }
            CrewAction::Select(id) => {
                app.crews_state[bi].selected = Some(id);
                app.crews_state[bi].draft = app.crews.iter().find(|c| c.id == id).cloned();
                app.crews_state[bi].confirm_delete = false;
            }
            CrewAction::New => {
                let color = CREW_PALETTE[app.crews.len() % CREW_PALETTE.len()];
                app.crews_state[bi].selected = None;
                let mut crew = Crew::new(color);
                crew.board = board;
                app.crews_state[bi].draft = Some(crew);
                app.crews_state[bi].confirm_delete = false;
            }
            CrewAction::Save => {
                let Some(draft) = app.crews_state[bi].draft.clone() else { continue };
                match app.db.save_crew_on(&draft, roster_day) {
                    Ok(id) => {
                        app.reload();
                        if draft.board != board {
                            // It now lives on the other staff tab.
                            app.crews_state[bi] = CrewsState::default();
                            app.set_status(format!(
                                "Moved crew {} and its members to {} Staff.",
                                draft.name.trim(),
                                draft.board.as_str()
                            ));
                        } else {
                            app.crews_state[bi].selected = Some(id);
                            app.crews_state[bi].draft = app.crews.iter().find(|c| c.id == id).cloned();
                            if draft.board == Board::RoughIn {
                                app.set_status(format!(
                                    "Saved crew {} (members for {} only).",
                                    draft.name.trim(),
                                    roster_day.format("%a %b %-d")
                                ));
                            } else {
                                app.set_status(format!("Saved crew {}", draft.name.trim()));
                            }
                        }
                    }
                    Err(e) => app.set_error(format!("Could not save crew: {e}")),
                }
            }
            CrewAction::Revert => {
                app.crews_state[bi].draft = original.clone();
                if original.is_none() {
                    app.crews_state[bi].selected = None;
                }
            }
            CrewAction::Delete(id) => match app.db.delete_crew(id) {
                Ok(()) => {
                    app.reload();
                    app.crews_state[bi] = CrewsState::default();
                    app.set_status("Crew deleted; its jobs are now Unassigned.");
                }
                Err(e) => app.set_error(format!("Could not delete crew: {e}")),
            },
            CrewAction::OpenJob(id) => app.open_job(id),
            CrewAction::OpenEmployees => app.tab = Tab::Staff(board, StaffPage::Employees),
        }
    }
}
