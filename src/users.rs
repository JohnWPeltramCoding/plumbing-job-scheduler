//! Sign-in, first-run admin setup, the Users admin page and "change my
//! password".

use eframe::egui::{self, vec2, Align, Button, Color32, Grid, Key, Layout, RichText, ScrollArea, TextEdit};

use crate::app::{SchedulerApp, Tab};
use crate::auth;
use crate::model::*;

const ERROR_RED: Color32 = Color32::from_rgb(220, 60, 60);

// ------------------------------------------------------------- sign-in

#[derive(Default)]
pub struct AuthState {
    name: String,
    username: String,
    password: String,
    confirm: String,
    error: Option<String>,
    focus_done: bool,
    prefilled: bool,
    /// Set when the account's password was reset: they must choose a new one.
    reset_user: Option<User>,
}

/// Full-window sign-in screen, "create the first admin" when no users exist,
/// or "choose a new password" after an admin reset the account's password.
pub fn auth_screen(app: &mut SchedulerApp, ctx: &egui::Context) {
    let first_run = app.users.is_empty();
    if !app.auth.prefilled {
        app.auth.prefilled = true;
        if let Some(u) = app.db.get_setting("last_username") {
            app.auth.username = u;
        }
    }
    let reset = app.auth.reset_user.clone();
    let mut submit = false;
    let mut back = false;

    egui::CentralPanel::default().show(ctx, |ui| {
        let st = &mut app.auth;
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.16).max(20.0));
            ui.label(RichText::new("Plumbing Job Scheduler").size(30.0).strong());
            ui.add_space(6.0);
            ui.label(if first_run {
                "Welcome! Create the administrator account to get started."
            } else if reset.is_some() {
                "Choose a new password"
            } else {
                "Sign in to continue"
            });
            ui.add_space(18.0);
        });

        let width = 360.0;
        ui.horizontal(|ui| {
            ui.add_space(((ui.available_width() - width - 40.0) / 2.0).max(0.0));
            egui::Frame::group(ui.style()).inner_margin(20.0).show(ui, |ui| ui.vertical(|ui| {
                ui.set_width(width);
                if let Some(r) = &reset {
                    ui.label(format!(
                        "The password for {} ({}) was reset. Choose a new password to sign in.",
                        r.name, r.username
                    ));
                    ui.add_space(8.0);
                }
                let mut first_field = None;
                if first_run {
                    ui.label("Your name");
                    first_field = Some(ui.add(TextEdit::singleline(&mut st.name).desired_width(f32::INFINITY)));
                    ui.add_space(6.0);
                }
                let mut u = None;
                if reset.is_none() {
                    ui.label("Username");
                    u = Some(ui.add(TextEdit::singleline(&mut st.username).desired_width(f32::INFINITY)));
                    ui.add_space(6.0);
                }
                ui.label(if reset.is_some() { "New password" } else { "Password" });
                let p = ui.add(TextEdit::singleline(&mut st.password).password(true).desired_width(f32::INFINITY));
                let mut last = p.clone();
                if first_run || reset.is_some() {
                    ui.add_space(6.0);
                    ui.label("Confirm password");
                    last = ui.add(TextEdit::singleline(&mut st.confirm).password(true).desired_width(f32::INFINITY));
                    ui.weak(format!("At least {} characters.", auth::MIN_PASSWORD_LEN));
                }
                if !st.focus_done {
                    st.focus_done = true;
                    match (first_field, u) {
                        (Some(f), _) => f.request_focus(),
                        (None, Some(u)) if st.username.is_empty() => u.request_focus(),
                        _ => p.request_focus(),
                    }
                }
                let enter = last.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                ui.add_space(12.0);
                let label = if first_run {
                    "Create Admin Account"
                } else if reset.is_some() {
                    "Set Password & Sign In"
                } else {
                    "Sign In"
                };
                if ui.add_sized([width, 32.0], Button::new(RichText::new(label).strong().size(15.0))).clicked() || enter {
                    submit = true;
                }
                if let Some(e) = &st.error {
                    ui.add_space(8.0);
                    ui.colored_label(ERROR_RED, e);
                }
                if reset.is_some() {
                    ui.add_space(6.0);
                    if ui.link("Back to sign in").clicked() {
                        back = true;
                    }
                }
            }));
        });
    });

    if back {
        app.auth.reset_user = None;
        app.auth.password.clear();
        app.auth.confirm.clear();
        app.auth.error = None;
        app.auth.focus_done = false;
    } else if submit {
        if first_run {
            create_first_admin(app);
        } else if reset.is_some() {
            complete_password_reset(app);
        } else {
            try_sign_in(app);
        }
    }
}

/// Finishes "choose a new password" for an account whose password was reset.
fn complete_password_reset(app: &mut SchedulerApp) {
    let Some(r) = app.auth.reset_user.clone() else { return };
    let st = &mut app.auth;
    if let Err(e) = auth::check_new_password(&st.password, &st.confirm) {
        st.error = Some(e);
        return;
    }
    // Only allowed while the reset is still pending (password cleared).
    let current = match app.db.find_login(&r.username) {
        Ok(Some((u, hash))) if hash.is_empty() && u.active && u.id == r.id => u,
        _ => {
            *st = AuthState { username: r.username.clone(), prefilled: true, ..Default::default() };
            st.error = Some("That password reset is no longer pending. Sign in normally.".into());
            return;
        }
    };
    let result = auth::hash_password(&st.password).and_then(|h| app.db.set_password(current.id, &h).map_err(|e| e.to_string()));
    match result {
        Ok(()) => {
            app.reload();
            app.sign_in(current);
            app.set_status("New password saved. You're signed in.");
        }
        Err(e) => app.auth.error = Some(format!("Could not save the new password: {e}")),
    }
}

fn try_sign_in(app: &mut SchedulerApp) {
    let username = app.auth.username.trim().to_string();
    let password = std::mem::take(&mut app.auth.password);
    let result = match app.db.find_login(&username) {
        // Password was reset by an admin: they choose a new one now.
        Ok(Some((user, hash))) if hash.is_empty() && user.active => {
            app.auth.reset_user = Some(user);
            app.auth.confirm.clear();
            app.auth.error = None;
            app.auth.focus_done = false;
            return;
        }
        Ok(Some((user, hash))) => {
            if !auth::verify_password(&password, &hash) {
                Err("Incorrect username or password.".to_string())
            } else if !user.active {
                Err("This account has been deactivated. Ask an administrator.".to_string())
            } else {
                Ok(user)
            }
        }
        Ok(None) => Err("Incorrect username or password.".to_string()),
        Err(e) => Err(format!("Could not check sign-in: {e}")),
    };
    match result {
        Ok(user) => app.sign_in(user),
        Err(e) => {
            app.auth.error = Some(e);
            app.auth.focus_done = false;
        }
    }
}

fn create_first_admin(app: &mut SchedulerApp) {
    let st = &mut app.auth;
    let err = if st.name.trim().is_empty() || st.username.trim().is_empty() {
        Some("Enter your name and a username.".to_string())
    } else {
        auth::check_new_password(&st.password, &st.confirm).err()
    };
    if let Some(e) = err {
        st.error = Some(e);
        return;
    }
    let mut user = User { id: 0, name: st.name.trim().into(), username: st.username.trim().into(), role: Role::Admin, active: true };
    let result = auth::hash_password(&st.password)
        .and_then(|h| app.db.create_user(&user, &h).map_err(|e| format!("Could not create the account: {e}")));
    match result {
        Ok(id) => {
            user.id = id;
            app.reload();
            app.sign_in(user);
        }
        Err(e) => app.auth.error = Some(e),
    }
}

// ---------------------------------------------------------- users admin

#[derive(Default)]
pub struct UsersState {
    selected: Option<i64>,
    /// Working copy (id 0 = new user).
    draft: Option<User>,
    new_pw: String,
    confirm_pw: String,
}

enum UserAction {
    Select(i64),
    New,
    Save,
    Revert,
    RequireReset(i64),
}

pub fn users_page(app: &mut SchedulerApp, ctx: &egui::Context) {
    let me = app.user_id();
    let st = &mut app.users_state;
    if st.draft.is_none() && st.selected.is_none() {
        if let Some(u) = app.users.first() {
            st.selected = Some(u.id);
            st.draft = Some(u.clone());
        }
    }
    let original = st.selected.and_then(|id| app.users.iter().find(|u| u.id == id).cloned());
    let dirty = match (&st.draft, &original) {
        (Some(d), Some(o)) => d != o || !st.new_pw.is_empty(),
        (Some(d), None) => d.id == 0,
        _ => false,
    };
    let mut actions = Vec::new();

    egui::SidePanel::left("user_list").resizable(true).default_width(300.0).show(ctx, |ui| {
        let st = &app.users_state;
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.heading("Users");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("+ New User").clicked() {
                    actions.push(UserAction::New);
                }
            });
        });
        ui.weak("People who can sign in to the scheduler.");
        ui.separator();
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for u in &app.users {
                let selected = st.selected == Some(u.id);
                ui.horizontal(|ui| {
                    let mut name = RichText::new(&u.name).size(14.0);
                    if !u.active {
                        name = name.weak().italics();
                    }
                    let resp = ui.selectable_label(selected, name);
                    ui.weak(&u.username);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let tag = if u.active { u.role.as_str() } else { "Inactive" };
                        ui.label(RichText::new(tag).small());
                        if app.pending_resets.contains(&u.id) {
                            ui.label(RichText::new("reset pending").small().color(Color32::from_rgb(200, 120, 20)));
                        }
                    });
                    if resp.clicked() && !selected {
                        actions.push(UserAction::Select(u.id));
                    }
                });
            }
        });
    });

    egui::CentralPanel::default().show(ctx, |ui| {
        let st = &mut app.users_state;
        let Some(draft) = st.draft.as_mut() else {
            ui.centered_and_justified(|ui| ui.label("Select a user on the left, or click “+ New User”."));
            return;
        };
        let is_me = draft.id > 0 && Some(draft.id) == me;

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.heading(if draft.id == 0 { "New User".to_string() } else { draft.name.clone() });
            if is_me {
                ui.weak("(you)");
            }
            if dirty {
                ui.colored_label(Color32::from_rgb(230, 150, 30), "(unsaved changes)");
            }
        });
        ui.add_space(8.0);

        Grid::new("user_form").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
            ui.label("Name");
            ui.add(TextEdit::singleline(&mut draft.name).desired_width(280.0));
            ui.end_row();

            ui.label("Username");
            ui.add(TextEdit::singleline(&mut draft.username).desired_width(200.0).hint_text("used to sign in"));
            ui.end_row();

            ui.label("Role");
            ui.vertical(|ui| {
                ui.add_enabled_ui(!is_me, |ui| {
                    for r in Role::ALL {
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut draft.role, r, RichText::new(r.as_str()).strong());
                            ui.weak(r.description());
                        });
                    }
                });
                if is_me {
                    ui.weak("You can't change your own role.");
                }
            });
            ui.end_row();

            ui.label("Active");
            ui.add_enabled_ui(!is_me, |ui| {
                ui.checkbox(&mut draft.active, "Can sign in (deactivate instead of deleting, to keep job history)");
            });
            ui.end_row();

            ui.label(if draft.id == 0 { "Password" } else { "Reset password" });
            ui.vertical(|ui| {
                ui.add(TextEdit::singleline(&mut st.new_pw).password(true).desired_width(220.0).hint_text(
                    if draft.id == 0 { "new password" } else { "leave blank to keep current" },
                ));
                ui.add(TextEdit::singleline(&mut st.confirm_pw).password(true).desired_width(220.0).hint_text("confirm"));
                ui.weak(format!("At least {} characters.", auth::MIN_PASSWORD_LEN));
                if draft.id > 0 && !is_me {
                    ui.add_space(4.0);
                    if app.pending_resets.contains(&draft.id) {
                        ui.colored_label(
                            Color32::from_rgb(200, 120, 20),
                            "Password reset pending: they'll choose a new password at their next sign-in.",
                        );
                    } else if ui
                        .button("Make them choose a new password")
                        .on_hover_text("Clears their password now. At their next sign-in they pick a new one.")
                        .clicked()
                    {
                        actions.push(UserAction::RequireReset(draft.id));
                    }
                }
            });
            ui.end_row();
        });

        ui.add_space(12.0);
        ui.separator();
        ui.horizontal(|ui| {
            let can_save = dirty && !draft.name.trim().is_empty() && !draft.username.trim().is_empty();
            if ui.add_enabled(can_save, Button::new(RichText::new("💾 Save User").strong())).clicked() {
                actions.push(UserAction::Save);
            }
            if ui.add_enabled(dirty, Button::new("Discard Changes")).clicked() {
                actions.push(UserAction::Revert);
            }
        });
    });

    for a in actions {
        match a {
            UserAction::Select(_) | UserAction::New if dirty => app.set_error("Save or discard your user changes first."),
            UserAction::RequireReset(id) => {
                let name = app.users.iter().find(|u| u.id == id).map(|u| u.name.clone()).unwrap_or_default();
                match app.db.set_password(id, "") {
                    Ok(()) => {
                        app.reload();
                        app.set_status(format!("{name} will choose a new password at their next sign-in."));
                    }
                    Err(e) => app.set_error(format!("Could not reset the password: {e}")),
                }
            }
            UserAction::Select(id) => {
                let st = &mut app.users_state;
                st.selected = Some(id);
                st.draft = app.users.iter().find(|u| u.id == id).cloned();
                st.new_pw.clear();
                st.confirm_pw.clear();
            }
            UserAction::New => {
                app.users_state = UsersState {
                    selected: None,
                    draft: Some(User { id: 0, name: String::new(), username: String::new(), role: Role::Scheduler, active: true }),
                    new_pw: String::new(),
                    confirm_pw: String::new(),
                };
            }
            UserAction::Revert => {
                let st = &mut app.users_state;
                st.draft = original.clone();
                st.new_pw.clear();
                st.confirm_pw.clear();
                if original.is_none() {
                    st.selected = None;
                }
            }
            UserAction::Save => save_user(app),
        }
    }
}

/// Validates and saves the user being edited.
fn save_user(app: &mut SchedulerApp) {
    let Some(d) = app.users_state.draft.clone() else { return };
    let (pw, confirm) = (app.users_state.new_pw.clone(), app.users_state.confirm_pw.clone());
    if let Err(e) = check_user_change(&app.users, &d, app.user_id()) {
        app.set_error(e);
        return;
    }
    if d.id == 0 || !pw.is_empty() {
        if let Err(e) = auth::check_new_password(&pw, &confirm) {
            app.set_error(e);
            return;
        }
    }
    let hash = if pw.is_empty() {
        None
    } else {
        match auth::hash_password(&pw) {
            Ok(h) => Some(h),
            Err(e) => {
                app.set_error(e);
                return;
            }
        }
    };
    let result = if d.id == 0 {
        app.db.create_user(&d, hash.as_deref().unwrap_or_default())
    } else {
        app.db.update_user(&d).and_then(|_| match &hash {
            Some(h) => app.db.set_password(d.id, h),
            None => Ok(()),
        }).map(|_| d.id)
    };
    match result {
        Ok(id) => {
            app.reload();
            let st = &mut app.users_state;
            st.selected = Some(id);
            st.draft = app.users.iter().find(|u| u.id == id).cloned();
            st.new_pw.clear();
            st.confirm_pw.clear();
            if app.user_id() == Some(id) {
                app.current_user = app.users.iter().find(|u| u.id == id).cloned();
            }
            app.set_status(format!("Saved user {}", d.name.trim()));
        }
        Err(e) if e.to_string().contains("UNIQUE") => {
            app.set_error(format!("The username “{}” is already taken.", d.username.trim()))
        }
        Err(e) => app.set_error(format!("Could not save user: {e}")),
    }
}

/// Rules that keep at least one admin able to sign in.
pub fn check_user_change(users: &[User], d: &User, me: Option<i64>) -> Result<(), String> {
    if d.name.trim().is_empty() || d.username.trim().is_empty() {
        return Err("Name and username are required.".into());
    }
    if d.id > 0 && Some(d.id) == me && (d.role != Role::Admin || !d.active) {
        return Err("You can't remove your own admin access or deactivate yourself.".into());
    }
    let was_active_admin = users.iter().any(|u| u.id == d.id && u.role == Role::Admin && u.active);
    let stays_active_admin = d.role == Role::Admin && d.active;
    if was_active_admin && !stays_active_admin {
        let others = users.iter().filter(|u| u.id != d.id && u.role == Role::Admin && u.active).count();
        if others == 0 {
            return Err("There must always be at least one active admin.".into());
        }
    }
    Ok(())
}

// ------------------------------------------------------ change password

#[derive(Default)]
pub struct ChangePassword {
    current: String,
    new: String,
    confirm: String,
    error: Option<String>,
}

pub fn change_password_window(app: &mut SchedulerApp, ctx: &egui::Context) {
    let Some(st) = app.change_pw.as_mut() else { return };
    let mut open = true;
    let mut submit = false;
    let mut cancel = false;
    egui::Window::new("Change My Password")
        .id(egui::Id::new("change_pw"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.screen_rect().center())
        .show(ctx, |ui| {
            Grid::new("pw_grid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                ui.label("Current password");
                ui.add(TextEdit::singleline(&mut st.current).password(true).desired_width(220.0));
                ui.end_row();
                ui.label("New password");
                ui.add(TextEdit::singleline(&mut st.new).password(true).desired_width(220.0));
                ui.end_row();
                ui.label("Confirm new");
                let last = ui.add(TextEdit::singleline(&mut st.confirm).password(true).desired_width(220.0));
                if last.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    submit = true;
                }
                ui.end_row();
            });
            ui.weak(format!("At least {} characters.", auth::MIN_PASSWORD_LEN));
            if let Some(e) = &st.error {
                ui.colored_label(ERROR_RED, e);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.add(Button::new(RichText::new("Change Password").strong()).min_size(vec2(0.0, 26.0))).clicked() {
                    submit = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
    if !open || cancel {
        app.change_pw = None;
        return;
    }
    if !submit {
        return;
    }
    let Some(me) = app.current_user.clone() else { return };
    let st = app.change_pw.as_mut().unwrap();
    let ok_current = matches!(app.db.find_login(&me.username), Ok(Some((_, h))) if auth::verify_password(&st.current, &h));
    let result = if !ok_current {
        Err("Your current password is incorrect.".to_string())
    } else {
        auth::check_new_password(&st.new, &st.confirm)
            .and_then(|_| auth::hash_password(&st.new))
            .and_then(|h| app.db.set_password(me.id, &h).map_err(|e| e.to_string()))
    };
    match result {
        Ok(()) => {
            app.change_pw = None;
            app.set_status("Your password was changed.");
        }
        Err(e) => {
            st.error = Some(e);
            st.current.clear();
        }
    }
}

/// Users tab is admin-only; bounce anyone else back to the schedule.
pub fn guard_tab(app: &mut SchedulerApp) {
    if app.tab == Tab::Users && !app.is_admin() {
        app.tab = Tab::Schedule(Board::RoughIn);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(id: i64, role: Role, active: bool) -> User {
        User { id, name: format!("U{id}"), username: format!("u{id}"), role, active }
    }

    #[test]
    fn always_keeps_an_active_admin() {
        let users = vec![u(1, Role::Admin, true), u(2, Role::Scheduler, true), u(3, Role::Admin, false)];
        // Demoting or deactivating the only active admin is blocked...
        assert!(check_user_change(&users, &u(1, Role::Scheduler, true), Some(2)).is_err());
        assert!(check_user_change(&users, &u(1, Role::Admin, false), Some(2)).is_err());
        // ...and so is changing yourself, even with another admin around.
        let users2 = vec![u(1, Role::Admin, true), u(4, Role::Admin, true)];
        assert!(check_user_change(&users2, &u(1, Role::Viewer, true), Some(1)).is_err());
        // With a second active admin, someone else may demote admin 1.
        assert!(check_user_change(&users2, &u(1, Role::Viewer, true), Some(4)).is_ok());
        // Ordinary edits are fine.
        assert!(check_user_change(&users, &u(2, Role::Viewer, false), Some(1)).is_ok());
        assert!(check_user_change(&users, &User { name: " ".into(), ..u(2, Role::Viewer, true) }, Some(1)).is_err());
    }
}
