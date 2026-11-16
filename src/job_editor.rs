use chrono::NaiveDate;
use eframe::egui::{self, vec2, Align, Button, Color32, ComboBox, DragValue, Grid, Layout, RichText, Sense, TextEdit};
use egui_extras::DatePickerButton;

use crate::model::*;

pub struct JobEditor {
    pub job: Job,
    pub is_new: bool,
    confirm_delete: bool,
    new_field: String,
}

impl JobEditor {
    pub fn new(job: Job, is_new: bool) -> Self {
        Self { job, is_new, confirm_delete: false, new_field: String::new() }
    }
}

pub enum EditorOutcome {
    Open,
    Close,
    Save(Job),
    SaveAndDuplicate(Job),
    Delete(i64),
}

pub fn show(
    ctx: &egui::Context,
    ed: &mut JobEditor,
    crews: &[Crew],
    projects: &[(String, String)],
    field_labels: &[String],
    default_date: NaiveDate,
    read_only: bool,
    audit: Option<&str>,
    // Who is on the chosen crew on the job's date (crews change day to day).
    crew_day: Option<(NaiveDate, Vec<String>)>,
    // Rough-In jobs: who is on the crew on each day the job runs.
    allocation: Option<Vec<(NaiveDate, Vec<String>, usize)>>,
) -> EditorOutcome {
    let mut open = true;
    let mut outcome = EditorOutcome::Open;
    let title = if ed.is_new {
        format!("New {} Job", ed.job.board.as_str())
    } else if read_only {
        format!("{} Job — {}", ed.job.board.as_str(), ed.job.short_title())
    } else {
        format!("Edit {} Job — {}", ed.job.board.as_str(), ed.job.short_title())
    };
    let max_body_h = (ctx.screen_rect().height() - 180.0).max(200.0);

    egui::Window::new(title)
        .id(egui::Id::new("job_editor_window"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(560.0)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.screen_rect().center())
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().max_height(max_body_h).show(ui, |ui| {
                if read_only {
                    ui.disable();
                }
                body(ui, ed, crews, projects, field_labels, default_date, crew_day.as_ref(), allocation.as_deref());
            });
            ui.separator();
            if let Some(a) = audit {
                ui.label(RichText::new(a).small().weak());
                ui.add_space(2.0);
            }
            if read_only {
                ui.horizontal(|ui| {
                    if ui.button("Close").clicked() {
                        outcome = EditorOutcome::Close;
                    }
                    ui.weak("View only. Your account can't change jobs.");
                });
                return;
            }
            ui.horizontal(|ui| {
                let can_save = !ed.job.project_code.trim().is_empty() || !ed.job.project_name.trim().is_empty();
                if ui.add_enabled(can_save, Button::new(RichText::new("💾 Save").strong())).clicked() {
                    outcome = EditorOutcome::Save(ed.job.clone());
                }
                if ui
                    .add_enabled(can_save, Button::new("Save & Duplicate for Next Lot"))
                    .on_hover_text("Saves this job, then opens a copy with the lot # cleared")
                    .clicked()
                {
                    outcome = EditorOutcome::SaveAndDuplicate(ed.job.clone());
                }
                if ui.button("Cancel").clicked() {
                    outcome = EditorOutcome::Close;
                }
                if !can_save {
                    ui.weak("Enter a project code or name");
                }
                if !ed.is_new {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ed.confirm_delete {
                            if ui.button("Keep").clicked() {
                                ed.confirm_delete = false;
                            }
                            if ui
                                .button(RichText::new("Confirm Delete").color(Color32::from_rgb(220, 50, 50)))
                                .clicked()
                            {
                                outcome = EditorOutcome::Delete(ed.job.id);
                            }
                        } else if ui.button("🗑 Delete").clicked() {
                            ed.confirm_delete = true;
                        }
                    });
                }
            });
        });

    if !open {
        outcome = EditorOutcome::Close;
    }
    outcome
}

fn swatch(ui: &mut egui::Ui, color: Color32) {
    let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
    ui.painter().rect_filled(r, 2.0, color);
}

fn body(
    ui: &mut egui::Ui,
    ed: &mut JobEditor,
    crews: &[Crew],
    projects: &[(String, String)],
    field_labels: &[String],
    default_date: NaiveDate,
    crew_day: Option<&(NaiveDate, Vec<String>)>,
    allocation: Option<&[(NaiveDate, Vec<String>, usize)]>,
) {
    let job = &mut ed.job;

    Grid::new("job_form").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        ui.label("Schedule");
        ui.horizontal(|ui| {
            for b in Board::ALL {
                let text = RichText::new(b.as_str()).strong();
                let text = if job.board == b { text.color(Color32::WHITE) } else { text };
                let button = Button::new(text).selected(job.board == b).min_size(vec2(90.0, 24.0));
                let button = if job.board == b { button.fill(b.accent()) } else { button };
                if ui.add(button).clicked() {
                    job.board = b;
                }
            }
        });
        ui.end_row();

        ui.label("Project Code");
        ui.horizontal(|ui| {
            ui.add(TextEdit::singleline(&mut job.project_code).desired_width(140.0).hint_text("e.g. OAK-101"));
            if !projects.is_empty() {
                ComboBox::from_id_salt("pick_project")
                    .selected_text("Pick existing project…")
                    .width(210.0)
                    .show_ui(ui, |ui| {
                        for (code, name) in projects {
                            if ui.selectable_label(job.project_code == *code, format!("{code} — {name}")).clicked() {
                                job.project_code = code.clone();
                                job.project_name = name.clone();
                            }
                        }
                    });
            }
        });
        ui.end_row();

        ui.label("Project Name");
        ui.add(TextEdit::singleline(&mut job.project_name).desired_width(360.0));
        ui.end_row();

        ui.label("Unit / Lot #");
        ui.add(TextEdit::singleline(&mut job.lot).desired_width(140.0).hint_text("e.g. 12 or Unit 4B"));
        ui.end_row();

        ui.label("Phase / Task");
        ui.horizontal(|ui| {
            ui.add(TextEdit::singleline(&mut job.phase).desired_width(180.0).hint_text(format!("e.g. {}", job.board.phases()[0])));
            ComboBox::from_id_salt("phase_pick").selected_text("Presets").show_ui(ui, |ui| {
                for p in job.board.phases() {
                    if ui.selectable_label(job.phase == *p, *p).clicked() {
                        job.phase = p.to_string();
                    }
                }
            });
        });
        ui.end_row();

        // Trim jobs only: water heater tank size.
        if job.board == Board::Trim {
            ui.label("Water Tank Size");
            ui.horizontal(|ui| {
                ui.add(TextEdit::singleline(&mut job.tank_size).desired_width(120.0).hint_text("e.g. 50 gal"));
                ComboBox::from_id_salt("tank_pick").selected_text("Sizes").show_ui(ui, |ui| {
                    for s in TANK_SIZES {
                        if ui.selectable_label(job.tank_size == *s, *s).clicked() {
                            job.tank_size = s.to_string();
                        }
                    }
                });
            });
            ui.end_row();

            ui.label("Kitchen Sink");
            ui.horizontal(|ui| {
                ui.add(
                    TextEdit::singleline(&mut job.kitchen_sink)
                        .desired_width(220.0)
                        .hint_text("e.g. Double bowl undermount, model #"),
                );
                ComboBox::from_id_salt("sink_pick").selected_text("Types").show_ui(ui, |ui| {
                    for s in KITCHEN_SINKS {
                        if ui.selectable_label(job.kitchen_sink == *s, *s).clicked() {
                            job.kitchen_sink = s.to_string();
                        }
                    }
                });
            });
            ui.end_row();
        }

        ui.label("Crew");
        ui.horizontal(|ui| {
            let current = crews.iter().find(|c| Some(c.id) == job.crew_id);
            swatch(ui, current.map(|c| c.color32()).unwrap_or(UNASSIGNED_COLOR));
            let text = current.map(|c| c.name.clone()).unwrap_or_else(|| "— Unassigned —".into());
            ComboBox::from_id_salt("crew_pick").selected_text(text).width(240.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut job.crew_id, None, "— Unassigned —");
                let selected_id = job.crew_id;
                // This job's own staff group first, then the other group's crews.
                for (i, group) in [job.board, if job.board == Board::RoughIn { Board::Trim } else { Board::RoughIn }]
                    .into_iter()
                    .enumerate()
                {
                    let list: Vec<&Crew> = crews
                        .iter()
                        .filter(|c| c.board == group && (c.active || Some(c.id) == selected_id))
                        .collect();
                    if list.is_empty() {
                        continue;
                    }
                    ui.separator();
                    let heading = if i == 0 { format!("{} crews", group.as_str()) } else { format!("Other: {} crews", group.as_str()) };
                    ui.label(RichText::new(heading).small().weak());
                    for c in list {
                        ui.horizontal(|ui| {
                            swatch(ui, c.color32());
                            ui.selectable_value(&mut job.crew_id, Some(c.id), c.name.as_str());
                        });
                    }
                }
            });
            if crews.is_empty() {
                ui.weak("Add crews on the Staff tabs");
            }
        });
        ui.end_row();
        if let Some((day, names)) = crew_day {
            ui.label("");
            let text = if names.is_empty() {
                format!("No one is on this crew on {}.", day.format("%a %b %-d"))
            } else {
                format!("On {}: {}", day.format("%a %b %-d"), names.join(", "))
            };
            ui.add(egui::Label::new(RichText::new(text).small().weak()).wrap());
            ui.end_row();
        }

        ui.label("Status");
        ComboBox::from_id_salt("status_pick").selected_text(job.status.clone()).show_ui(ui, |ui| {
            for s in STATUSES {
                ui.selectable_value(&mut job.status, s.to_string(), *s);
            }
        });
        ui.end_row();

        ui.label("Date");
        ui.horizontal(|ui| {
            let mut scheduled = job.date.is_some();
            if ui.checkbox(&mut scheduled, "Scheduled").changed() {
                job.date = scheduled.then_some(default_date);
                if scheduled {
                    // Newly scheduled: a full 8-hour day (change it if needed).
                    job.duration_min = HOURS_PER_PERSON_DAY * 60;
                }
            }
            if let Some(d) = job.date.as_mut() {
                ui.add(DatePickerButton::new(d));
                ui.label(d.format("%A").to_string());
            } else {
                ui.weak("(shows in the Unscheduled list)");
            }
        });
        ui.end_row();

        ui.label("Start Time");
        ComboBox::from_id_salt("start_pick")
            .selected_text(fmt_time(job.start_min))
            .height(320.0)
            .show_ui(ui, |ui| {
                let mut m = 4 * 60;
                while m <= 21 * 60 {
                    ui.selectable_value(&mut job.start_min, m, fmt_time(m));
                    m += 15;
                }
            });
        ui.end_row();

        ui.label(if job.days > 1 { "Hours per Day" } else { "Duration" });
        ui.horizontal(|ui| {
            let mut hours = job.duration_min as f32 / 60.0;
            if ui
                .add(DragValue::new(&mut hours).speed(0.05).range(0.25..=16.0).fixed_decimals(2).suffix(" hrs"))
                .changed()
            {
                job.duration_min = ((hours * 4.0).round() as i32 * 15).max(15);
            }
            ui.weak(format!("ends {}", fmt_time(job.end_min())));
        });
        ui.end_row();

        ui.label("Days");
        ui.horizontal(|ui| {
            ui.add(
                DragValue::new(&mut job.days)
                    .speed(0.1)
                    .range(1..=MAX_JOB_DAYS)
                    .custom_formatter(|n, _| if n as i32 == 1 { "1 day".into() } else { format!("{} days", n as i32) })
                    .custom_parser(|s| s.trim().trim_end_matches("days").trim_end_matches("day").trim().parse().ok()),
            )
            .on_hover_text(
                "Days the job runs. Saturdays and Sundays are skipped unless ticked below; \
                 a ticked weekend day counts as one of the job's days.",
            );
            if job.days > 1 {
                match job.last_date() {
                    Some(last) => ui.weak(format!("through {} ({})", last.format("%a %b %-d"), job.weekend.describe())),
                    None => ui.weak(job.weekend.describe()),
                };
            }
        });
        ui.end_row();

        ui.label("Weekend");
        ui.horizontal(|ui| {
            ui.checkbox(&mut job.weekend.sat, "Include Saturday")
                .on_hover_text("Work Saturdays on this job (counts as one of its days)");
            ui.checkbox(&mut job.weekend.sun, "Include Sunday")
                .on_hover_text("Work Sundays on this job (counts as one of its days)");
            if job.days <= 1 && (job.weekend.sat || job.weekend.sun) {
                ui.weak("applies when the job runs more than 1 day");
            }
        });
        ui.end_row();

        // Rough-In: man-hours from the daily crews (8 h per person per day).
        if let Some(days) = allocation {
            ui.label("Allocated Hours");
            ui.vertical(|ui| {
                let counts: Vec<(usize, usize)> = days.iter().map(|(_, p, jobs)| (p.len(), *jobs)).collect();
                let total = allocated_hours(&counts);
                let shared = counts.iter().any(|(_, j)| *j > 1);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{} hrs", fmt_hours(total))).strong().size(15.0));
                    if days.is_empty() {
                        ui.weak("Give the job a date and a crew to allocate hours.");
                    } else {
                        let people: usize = counts.iter().map(|(p, _)| p).sum();
                        let mut s = format!(
                            "{people} person-day{} × {HOURS_PER_PERSON_DAY} h",
                            if people == 1 { "" } else { "s" }
                        );
                        if shared {
                            s.push_str(", split with the crew's other jobs");
                        }
                        s.push_str(" · from the daily crews and attendance");
                        ui.weak(s);
                    }
                });
                if !days.is_empty() {
                    egui::CollapsingHeader::new(RichText::new("By day").small())
                        .id_salt("alloc_by_day")
                        .default_open(days.len() > 1 || shared)
                        .show(ui, |ui| {
                            for (d, names, jobs) in days {
                                let n = names.len();
                                let who = if names.is_empty() { "no one on the crew".to_string() } else { names.join(", ") };
                                let split = if *jobs > 1 {
                                    format!(" (crew has {jobs} jobs this day: {} h ÷ {jobs})", n as i32 * HOURS_PER_PERSON_DAY)
                                } else {
                                    String::new()
                                };
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(format!(
                                            "{} · {n} {} · {} h{split} — {who}",
                                            d.format("%a %b %-d"),
                                            if n == 1 { "person" } else { "people" },
                                            fmt_hours(day_hours(n, *jobs))
                                        ))
                                        .small(),
                                    )
                                    .wrap(),
                                );
                            }
                        });
                }
            });
            ui.end_row();
        }
    });

    ui.add_space(8.0);
    ui.label(RichText::new("Notes").strong());
    ui.add(
        TextEdit::multiline(&mut job.notes)
            .desired_rows(3)
            .desired_width(f32::INFINITY)
            .hint_text("Job notes, site access, materials…"),
    );
    ui.add_space(4.0);
    ui.label(RichText::new("Additional Comments").strong());
    ui.add(TextEdit::multiline(&mut job.comments).desired_rows(3).desired_width(f32::INFINITY));

    ui.add_space(8.0);
    ui.label(RichText::new("Additional Fields").strong());
    let mut remove = None;
    if !job.fields.is_empty() {
        Grid::new("custom_fields").num_columns(3).spacing([8.0, 4.0]).show(ui, |ui| {
            for (i, f) in job.fields.iter_mut().enumerate() {
                ui.add_sized([160.0, 22.0], TextEdit::singleline(&mut f.label).hint_text("Field name"));
                ui.add_sized([300.0, 22.0], TextEdit::singleline(&mut f.value).hint_text("Value"));
                if ui.small_button("🗑").on_hover_text("Remove field").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
    }
    if let Some(i) = remove {
        job.fields.remove(i);
    }
    ui.horizontal(|ui| {
        let resp = ui.add(TextEdit::singleline(&mut ed.new_field).desired_width(160.0).hint_text("New field name"));
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.button("+ Add Field").clicked() || (enter && !ed.new_field.trim().is_empty()) {
            let label = ed.new_field.trim();
            job.fields.push(CustomField {
                label: if label.is_empty() { "New field".into() } else { label.to_string() },
                value: String::new(),
            });
            ed.new_field.clear();
        }
        let suggestions: Vec<&String> = field_labels
            .iter()
            .filter(|l| !job.fields.iter().any(|f| f.label.trim() == l.as_str()))
            .collect();
        if !suggestions.is_empty() {
            ComboBox::from_id_salt("field_suggest").selected_text("Quick add…").show_ui(ui, |ui| {
                for l in suggestions {
                    if ui.selectable_label(false, l.as_str()).clicked() {
                        job.fields.push(CustomField { label: l.clone(), value: String::new() });
                    }
                }
            });
        }
    });
}
