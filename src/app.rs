use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::{Datelike, Duration, Local, Months, NaiveDate, NaiveDateTime, TimeZone};
use eframe::egui::{
    self, Align, Button, Color32, Context, Key, KeyboardShortcut, Layout, Modifiers, RichText,
    TextEdit,
};

use crate::calendar::{self, DragState, DropTarget};
use crate::crews::{self, CrewsState};
use crate::db::Db;
use crate::attendance::{self, AttendanceState};
use crate::employees::{self, EmployeesState};
use crate::job_editor::{self, EditorOutcome, JobEditor};
use crate::model::*;
use crate::users::{self, AuthState, ChangePassword, UsersState};
use crate::vehicles::{self, VehiclesState};

/// Database timestamps are UTC "YYYY-MM-DD HH:MM:SS"; show them in local time.
pub fn local_time(ts: &str) -> String {
    match NaiveDateTime::parse_from_str(ts, "%Y-%m-%d %H:%M:%S") {
        Ok(n) => Local.from_utc_datetime(&n).format("%b %-d, %Y %-I:%M %p").to_string(),
        Err(_) => ts.to_string(),
    }
}

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Schedule(Board),
    /// Rough-In Staff or Trim Staff, and which sub-tab.
    Staff(Board, StaffPage),
    Vehicles,
    Users,
}

/// Sub-tabs inside a staff tab. Attendance exists only for Rough-In Staff.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StaffPage {
    Crews,
    Employees,
    Attendance,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Month,
    Week,
    /// One day by the hour, full width.
    Day,
    /// Crew rows × hours, for the week or a single day.
    Crew,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CrewFilter {
    All,
    Unassigned,
    Crew(i64),
}

enum Undo {
    /// Put this job back exactly as it was (undoes a move, edit or delete).
    Restore(Job),
    /// Delete a job that was just created.
    Remove(i64),
}

pub struct SchedulerApp {
    pub db: Db,
    pub jobs: Vec<Job>,
    pub crews: Vec<Crew>,
    pub employees: Vec<Employee>,
    pub vehicles: Vec<Vehicle>,
    /// Every vehicle's repairs, newest first.
    pub repairs: Vec<Repair>,
    /// Driver's licenses by employee id.
    pub licenses: HashMap<i64, DriverLicense>,
    /// Accidents, moving violations and parking tickets, newest first.
    pub incidents: Vec<Incident>,
    pub users: Vec<User>,
    /// Users who must choose a new password at their next sign-in.
    pub pending_resets: std::collections::HashSet<i64>,
    /// Created/changed-by info per job id.
    pub audit: HashMap<i64, JobAudit>,
    /// None = nobody signed in (the sign-in screen is shown).
    pub current_user: Option<User>,
    pub auth: AuthState,
    pub users_state: UsersState,
    pub change_pw: Option<ChangePassword>,
    pub tab: Tab,
    /// Last-used sub-tab on each staff tab (indexed by Board::idx).
    pub last_staff_page: [StaffPage; 2],
    pub view: ViewMode,
    /// Month view shows this date's month; week view shows its week.
    pub focus: NaiveDate,
    /// Crew Week view: Some(day) = single-day timeline, None = whole week.
    pub crew_view_day: Option<NaiveDate>,
    /// Crew Week view: also show the other schedule's jobs (faded).
    pub crew_view_all_boards: bool,
    pub crew_filter: CrewFilter,
    pub search: String,
    pub hide_done: bool,
    /// Calendar shows multi-day jobs on their start date only (else on every
    /// day they run). Remembered between sessions.
    pub show_start_only: bool,
    pub editor: Option<JobEditor>,
    /// Crew and employee page state, one per staff group (Board::idx).
    pub crews_state: [CrewsState; 2],
    pub employees_state: [EmployeesState; 2],
    pub attendance_state: AttendanceState,
    pub vehicles_state: VehiclesState,
    /// An employee import waiting in its preview window.
    pub headcount: Option<crate::employee_import::HeadcountImport>,
    /// A job import waiting in its preview window.
    pub job_import: Option<crate::job_import::JobImport>,
    /// Rough-In crews change day to day. The Rough-In Staff tabs show and edit
    /// the crews on this day; `employees[].crew_id` and Rough-In crews'
    /// `member_ids` reflect it.
    pub roster_day: NaiveDate,
    /// Where `roster_day`'s crews come from (itself once changed, else the
    /// latest changed day before it; None = the original crews).
    pub roster_source: Option<NaiveDate>,
    /// Rough-In crews by day, loaded as needed (cleared on reload).
    roster_cache: HashMap<NaiveDate, crate::db::Roster>,
    pub drag: Option<DragState>,
    /// Rebuilt every frame by the calendar views.
    pub drop_targets: Vec<(egui::Rect, DropTarget)>,
    undo: Vec<Undo>,
    pub status: String,
    pub status_is_error: bool,
}

impl SchedulerApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (db, warning) = Db::open_default();
        let mut app = Self {
            db,
            jobs: Vec::new(),
            crews: Vec::new(),
            employees: Vec::new(),
            vehicles: Vec::new(),
            repairs: Vec::new(),
            licenses: HashMap::new(),
            incidents: Vec::new(),
            users: Vec::new(),
            pending_resets: Default::default(),
            audit: HashMap::new(),
            current_user: None,
            auth: AuthState::default(),
            users_state: UsersState::default(),
            change_pw: None,
            tab: Tab::Schedule(Board::RoughIn),
            last_staff_page: [StaffPage::Crews; 2],
            view: ViewMode::Month,
            focus: today(),
            crew_view_day: None,
            crew_view_all_boards: true,
            crew_filter: CrewFilter::All,
            search: String::new(),
            hide_done: false,
            show_start_only: false,
            editor: None,
            crews_state: Default::default(),
            employees_state: Default::default(),
            attendance_state: AttendanceState::default(),
            vehicles_state: VehiclesState::default(),
            headcount: None,
            job_import: None,
            roster_day: today(),
            roster_source: None,
            roster_cache: HashMap::new(),
            drag: None,
            drop_targets: Vec::new(),
            undo: Vec::new(),
            status: "Ready. Drag jobs to reschedule · click a job to edit · double-click a day to add a job.".into(),
            status_is_error: false,
        };
        app.show_start_only = app.db.get_setting("show_start_only").as_deref() == Some("1");
        app.reload();
        // `--sample` loads demo crews/jobs into an empty database.
        if std::env::args().any(|a| a == "--sample") && app.jobs.is_empty() && app.crews.is_empty() {
            if let Err(e) = app.db.seed_sample(today()) {
                app.set_error(format!("Could not load sample data: {e}"));
            }
            app.reload();
        }
        if let Some(w) = warning {
            app.set_error(w);
        }
        app
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status = msg.into();
        self.status_is_error = false;
    }

    pub fn set_error(&mut self, msg: impl Into<String>) {
        self.status = msg.into();
        self.status_is_error = true;
    }

    pub fn reload(&mut self) {
        match self.db.load_crews() {
            Ok(c) => self.crews = c,
            Err(e) => self.set_error(format!("Failed to load crews: {e}")),
        }
        match self.db.load_employees() {
            Ok(e) => self.employees = e,
            Err(e) => self.set_error(format!("Failed to load employees: {e}")),
        }
        match self.db.load_vehicles() {
            Ok(v) => self.vehicles = v,
            Err(e) => self.set_error(format!("Failed to load vehicles: {e}")),
        }
        match self.db.load_repairs() {
            Ok(r) => self.repairs = r,
            Err(e) => self.set_error(format!("Failed to load repairs: {e}")),
        }
        match self.db.load_licenses() {
            Ok(l) => self.licenses = l,
            Err(e) => self.set_error(format!("Failed to load driver's licenses: {e}")),
        }
        match self.db.load_incidents() {
            Ok(i) => self.incidents = i,
            Err(e) => self.set_error(format!("Failed to load driving incidents: {e}")),
        }
        match self.db.load_jobs() {
            Ok(j) => self.jobs = j,
            Err(e) => self.set_error(format!("Failed to load jobs: {e}")),
        }
        match self.db.load_users() {
            Ok(u) => self.users = u,
            Err(e) => self.set_error(format!("Failed to load users: {e}")),
        }
        if let Ok(a) = self.db.load_audit() {
            self.audit = a;
        }
        if let Ok(p) = self.db.pending_resets() {
            self.pending_resets = p;
        }
        self.apply_roster();
    }

    /// Shows the Rough-In crews as they are on `roster_day`.
    fn apply_roster(&mut self) {
        self.roster_cache.clear();
        let day = self.roster_day;
        let roster = match self.db.load_roster(day) {
            Ok(r) => r,
            Err(e) => {
                self.set_error(format!("Failed to load crews for {}: {e}", day.format("%b %-d")));
                Default::default()
            }
        };
        self.roster_source = self.db.roster_source(day).unwrap_or(None);
        for e in self.employees.iter_mut().filter(|e| e.staff == Board::RoughIn) {
            e.crew_id = roster.get(&e.id).map(|x| x.0);
        }
        for c in self.crews.iter_mut().filter(|c| c.board == Board::RoughIn) {
            let mut members: Vec<(i64, i64)> =
                roster.iter().filter(|(_, (crew, _))| *crew == c.id).map(|(emp, (_, pos))| (*pos, *emp)).collect();
            members.sort();
            c.member_ids = members.into_iter().map(|(_, emp)| emp).collect();
        }
        self.roster_cache.insert(day, roster);
    }

    /// Rough-In crew assignments on any day (employee id → (crew id, position)).
    pub fn roster_on(&mut self, day: NaiveDate) -> &crate::db::Roster {
        if !self.roster_cache.contains_key(&day) {
            let r = self.db.load_roster(day).unwrap_or_default();
            self.roster_cache.insert(day, r);
        }
        &self.roster_cache[&day]
    }

    /// The crew an employee is on that day (Rough-In: that day's crews;
    /// Trim: their one standing crew).
    pub fn crew_of_on(&mut self, emp: i64, day: NaiveDate) -> Option<i64> {
        let e = self.employees.iter().find(|e| e.id == emp)?;
        if e.staff == Board::RoughIn {
            self.roster_on(day).get(&emp).map(|x| x.0)
        } else {
            e.crew_id
        }
    }

    /// Names of the people on a crew that day, in crew order.
    pub fn crew_names_on(&mut self, crew_id: i64, day: NaiveDate) -> Vec<String> {
        let Some(crew) = self.crews.iter().find(|c| c.id == crew_id) else { return Vec::new() };
        let ids: Vec<i64> = if crew.board == Board::RoughIn {
            let mut m: Vec<(i64, i64)> = self
                .roster_on(day)
                .iter()
                .filter(|(_, (c, _))| *c == crew_id)
                .map(|(e, (_, p))| (*p, *e))
                .collect();
            m.sort();
            m.into_iter().map(|(_, e)| e).collect()
        } else {
            crew.member_ids.clone()
        };
        ids.iter().filter_map(|id| self.employees.iter().find(|e| e.id == *id)).map(|e| e.name.clone()).collect()
    }

    /// Who is on the job's crew on each day it runs (from the daily crews,
    /// so people marked out that day aren't counted). Empty when the job has
    /// no date or no crew.
    /// For each day the job runs: who is on its crew (from the daily crews,
    /// so people marked out that day aren't counted) and how many jobs the
    /// crew has that day, this one included (the crew's hours are split
    /// between them). Empty when the job has no date or no crew.
    pub fn job_crew_by_day(&mut self, job: &Job) -> Vec<(NaiveDate, Vec<String>, usize)> {
        let Some(crew) = job.crew_id else { return Vec::new() };
        job.work_dates()
            .into_iter()
            .map(|d| (d, self.crew_names_on(crew, d), self.crew_jobs_on(crew, d, job)))
            .collect()
    }

    /// How many jobs a crew has on a day, counting `job` (as edited) once.
    /// Cancelled jobs don't count.
    pub fn crew_jobs_on(&self, crew: i64, day: NaiveDate, job: &Job) -> usize {
        let others = self
            .jobs
            .iter()
            .filter(|j| j.id != job.id || job.id == 0)
            .filter(|j| j.crew_id == Some(crew) && !j.is_cancelled() && j.day_index(day).is_some())
            .count();
        others + 1
    }

    /// Allocated man-hours for a job (8 hours per person per day, split
    /// across the crew's jobs that day), without touching the cache (for
    /// hover cards, which only read).
    pub fn job_allocated_hours(&self, job: &Job) -> f32 {
        let Some(crew_id) = job.crew_id else { return 0.0 };
        let Some(crew) = self.crews.iter().find(|c| c.id == crew_id) else { return 0.0 };
        let days: Vec<(usize, usize)> = job
            .work_dates()
            .into_iter()
            .map(|d| {
                let people = if crew.board != Board::RoughIn {
                    crew.member_ids.len()
                } else {
                    let count = |r: &crate::db::Roster| r.values().filter(|(c, _)| *c == crew_id).count();
                    match self.roster_cache.get(&d) {
                        Some(r) => count(r),
                        None => self.db.load_roster(d).map(|r| count(&r)).unwrap_or(0),
                    }
                };
                (people, self.crew_jobs_on(crew_id, d, job))
            })
            .collect();
        allocated_hours(&days)
    }

    /// Allocated hours for every Rough-In job showing on these days (loads
    /// the daily crews it needs first, so drawing stays fast).
    pub fn allocations_on(&mut self, days: &[NaiveDate]) -> HashMap<i64, f32> {
        let jobs: Vec<Job> = self
            .jobs
            .iter()
            .filter(|j| j.board == Board::RoughIn && j.crew_id.is_some())
            .filter(|j| days.iter().any(|d| self.job_on(j, *d)))
            .cloned()
            .collect();
        for j in &jobs {
            for d in j.work_dates() {
                self.roster_on(d);
            }
        }
        jobs.iter().map(|j| (j.id, self.job_allocated_hours(j))).collect()
    }

    /// True while the Rough-In crew editor has unsaved changes.
    pub fn rough_crew_draft_dirty(&self) -> bool {
        let cs = &self.crews_state[Board::RoughIn.idx()];
        match &cs.draft {
            Some(d) if d.id == 0 => true,
            Some(d) => self.crews.iter().find(|c| c.id == d.id).is_some_and(|c| c != d),
            None => false,
        }
    }

    /// Switches the Rough-In Staff tabs to another day's crews.
    pub fn set_roster_day(&mut self, day: NaiveDate) {
        if day == self.roster_day {
            return;
        }
        if self.rough_crew_draft_dirty() {
            self.set_error("Save or discard your crew changes before switching days.");
            return;
        }
        self.roster_day = day;
        self.reload();
        // The open crew now shows that day's members.
        let cs = &mut self.crews_state[Board::RoughIn.idx()];
        if let Some(d) = cs.draft.as_mut() {
            if let Some(c) = self.crews.iter().find(|c| c.id == d.id) {
                *d = c.clone();
            }
        }
    }

    /// e.g. "Fri Nov 13".
    pub fn short_day(d: NaiveDate) -> String {
        d.format("%a %b %-d").to_string()
    }

    // ------------------------------------------------------------ users

    pub fn user_id(&self) -> Option<i64> {
        self.current_user.as_ref().map(|u| u.id)
    }

    /// Schedulers and admins can change data; viewers can only look.
    pub fn can_edit(&self) -> bool {
        self.current_user.as_ref().is_some_and(|u| u.active && u.role.can_edit())
    }

    pub fn is_admin(&self) -> bool {
        self.current_user.as_ref().is_some_and(|u| u.active && u.role == Role::Admin)
    }

    /// Moves an employee onto a crew (None = no crew) in one step, keeping any
    /// open crew editors in step. Returns a status-bar message.
    /// For Rough-In staff this changes the crews on `roster_day` only.
    pub fn move_employee(&mut self, id: i64, crew: Option<i64>) -> Result<String, String> {
        self.move_employee_on(self.roster_day, id, crew)
    }

    /// Moves an employee onto a crew (None = no crew). Rough-In staff are
    /// moved for `day` only; Trim staff have one standing crew.
    pub fn move_employee_on(&mut self, day: NaiveDate, id: i64, crew: Option<i64>) -> Result<String, String> {
        if !self.can_edit() {
            return Err("Your account is view-only. Ask an admin if you need to make changes.".into());
        }
        let Some(emp) = self.employees.iter().find(|e| e.id == id) else {
            return Err("That employee no longer exists.".into());
        };
        let emp_name = emp.name.clone();
        let rough = emp.staff == Board::RoughIn;
        let old_crew = self.crew_of_on(id, day);
        let result = match (rough, crew) {
            (true, c) => self.db.set_roster_crew(day, id, c),
            (false, Some(c)) => self.db.move_employee_to_crew(id, c),
            (false, None) => self.db.release_employee(id),
        };
        result.map_err(|e| format!("Could not change crew: {e}"))?;
        self.reload();
        // Keep open crew editors in step (Rough-In: only if showing that day).
        for (bi, cs) in self.crews_state.iter_mut().enumerate() {
            if rough && (bi != Board::RoughIn.idx() || day != self.roster_day) {
                continue;
            }
            if let Some(d) = cs.draft.as_mut() {
                if Some(d.id) == crew {
                    if !d.member_ids.contains(&id) {
                        d.member_ids.push(id);
                    }
                } else {
                    d.member_ids.retain(|m| *m != id);
                }
            }
        }
        let name_of = |c: Option<i64>| c.and_then(|c| self.crews.iter().find(|x| x.id == c)).map(|x| x.name.clone());
        let when = if rough { format!(" for {}", Self::short_day(day)) } else { String::new() };
        let mut msg = match (name_of(old_crew), name_of(crew)) {
            (Some(from), Some(to)) => format!("Moved {emp_name} from {from} to {to}{when}."),
            (None, Some(to)) => format!("Added {emp_name} to {to}{when}."),
            (Some(from), None) => format!("Removed {emp_name} from {from}{when}; they're now available."),
            (None, None) => String::new(),
        };
        // Heads-up if they were listed as the old crew's foreman.
        if let Some(oc) = old_crew.and_then(|c| self.crews.iter().find(|x| x.id == c)) {
            if !emp_name.trim().is_empty() && oc.foreman.trim().eq_ignore_ascii_case(emp_name.trim()) {
                msg.push_str(&format!(" They were {}'s foreman, so update that crew's foreman.", oc.name));
            }
        }
        Ok(msg)
    }

    pub fn sign_in(&mut self, user: User) {
        let _ = self.db.set_setting("last_username", &user.username);
        self.set_status(format!("Signed in as {} ({})", user.name, user.role.as_str()));
        self.current_user = Some(user);
        self.auth = AuthState::default();
        self.tab = Tab::Schedule(Board::RoughIn);
    }

    pub fn sign_out(&mut self) {
        self.current_user = None;
        self.editor = None;
        self.drag = None;
        self.undo.clear();
        self.change_pw = None;
        self.crews_state = Default::default();
        self.employees_state = Default::default();
        self.attendance_state = AttendanceState::default();
        self.users_state = UsersState::default();
        self.auth = AuthState::default();
        self.tab = Tab::Schedule(Board::RoughIn);
        self.set_status("Signed out.");
    }

    fn user_name(&self, id: Option<i64>) -> Option<String> {
        id.and_then(|id| self.users.iter().find(|u| u.id == id)).map(|u| u.name.clone())
    }

    /// "Created by Ann on … · Last changed by Bob on …" for the job editor.
    pub fn audit_line(&self, job_id: i64) -> Option<String> {
        let a = self.audit.get(&job_id)?;
        let part = |verb: &str, who: Option<i64>, when: &str| match self.user_name(who) {
            Some(n) => format!("{verb} by {n} on {}", local_time(when)),
            None => format!("{verb} {}", local_time(when)),
        };
        let mut s = part("Created", a.created_by, &a.created_at);
        if a.updated_at != a.created_at || a.updated_by != a.created_by {
            s.push_str(" · ");
            s.push_str(&part("Last changed", a.updated_by, &a.updated_at));
        }
        Some(s)
    }

    pub fn crew(&self, id: Option<i64>) -> Option<&Crew> {
        id.and_then(|id| self.crews.iter().find(|c| c.id == id))
    }

    pub fn job_color(&self, job: &Job) -> Color32 {
        self.crew(job.crew_id).map(|c| c.color32()).unwrap_or(UNASSIGNED_COLOR)
    }

    pub fn crew_name(&self, job: &Job) -> String {
        self.crew(job.crew_id).map(|c| c.name.clone()).unwrap_or_else(|| "Unassigned".into())
    }

    /// The Rough-In / Trim side currently shown (Rough-In on the Users tab).
    pub fn board(&self) -> Board {
        match self.tab {
            Tab::Schedule(b) | Tab::Staff(b, _) => b,
            Tab::Users | Tab::Vehicles => Board::RoughIn,
        }
    }

    /// Whether the calendar shows `job` on `date`: its start date, plus every
    /// later day it runs unless "Show Start Date Only" is checked.
    pub fn job_on(&self, job: &Job, date: NaiveDate) -> bool {
        if self.show_start_only {
            job.date == Some(date)
        } else {
            job.day_index(date).is_some()
        }
    }

    pub fn job_visible(&self, job: &Job) -> bool {
        if let Tab::Schedule(b) = self.tab {
            if job.board != b {
                return false;
            }
        }
        self.job_passes_filters(job)
    }

    /// Crew / search / hide-completed filters, ignoring which schedule page
    /// the job is on (the crew timeline shows both).
    pub fn job_passes_filters(&self, job: &Job) -> bool {
        match self.crew_filter {
            CrewFilter::All => {}
            CrewFilter::Unassigned => {
                if job.crew_id.is_some() {
                    return false;
                }
            }
            CrewFilter::Crew(id) => {
                if job.crew_id != Some(id) {
                    return false;
                }
            }
        }
        if self.hide_done && (job.is_complete() || job.is_cancelled()) {
            return false;
        }
        let q = self.search.trim().to_lowercase();
        if !q.is_empty() {
            let mut hay = format!(
                "{} {} {} {} {} {} {}",
                job.project_code,
                job.project_name,
                job.lot_label(),
                job.phase,
                job.status,
                job.notes,
                job.comments
            );
            if job.board == Board::Trim {
                hay.push(' ');
                hay.push_str(&job.tank_size);
                hay.push(' ');
                hay.push_str(&job.kitchen_sink);
            }
            if let Some(c) = self.crew(job.crew_id) {
                hay.push(' ');
                hay.push_str(&c.name);
            }
            for f in &job.fields {
                hay.push(' ');
                hay.push_str(&f.value);
            }
            if !hay.to_lowercase().contains(&q) {
                return false;
            }
        }
        true
    }

    fn push_undo(&mut self, u: Undo) {
        self.undo.push(u);
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
    }

    /// Saves a new or edited job. Returns the job id on success.
    pub fn save_job(&mut self, mut job: Job) -> Option<i64> {
        if !self.require_edit() {
            return None;
        }
        if self.crew(job.crew_id).is_none() {
            job.crew_id = None;
        }
        let prev = self.jobs.iter().find(|j| job.id > 0 && j.id == job.id).cloned();
        match self.db.save_job(&job, self.current_user.as_ref().map(|u| u.id)) {
            Ok(id) => {
                self.push_undo(match prev {
                    Some(p) => Undo::Restore(p),
                    None => Undo::Remove(id),
                });
                self.reload();
                match self.tab {
                    Tab::Schedule(b) if b != job.board => self.set_status(format!(
                        "Saved {} to the {} (it no longer shows on this page)",
                        job.short_title(),
                        job.board.page_title()
                    )),
                    _ => self.set_status(format!("Saved {}", job.short_title())),
                }
                Some(id)
            }
            Err(e) => {
                self.set_error(format!("Could not save job: {e}"));
                None
            }
        }
    }

    /// Moves a job to a new date/time. `crew` = Some(..) also reassigns the
    /// crew (Some(None) = unassigned); None leaves the crew unchanged.
    pub fn move_job(&mut self, id: i64, date: Option<NaiveDate>, start_min: i32, crew: Option<Option<i64>>) {
        if !self.require_edit() {
            return;
        }
        let Some(prev) = self.jobs.iter().find(|j| j.id == id).cloned() else {
            return;
        };
        let crew = crew.filter(|c| *c != prev.crew_id);
        if prev.date == date && prev.start_min == start_min && crew.is_none() {
            return;
        }
        let user = self.current_user.as_ref().map(|u| u.id);
        // Scheduling an unscheduled job gives it a full 8-hour day.
        let newly_scheduled = prev.date.is_none() && date.is_some();
        let result = if newly_scheduled {
            let mut j = prev.clone();
            j.date = date;
            j.start_min = start_min;
            if let Some(c) = crew {
                j.crew_id = c;
            }
            j.duration_min = HOURS_PER_PERSON_DAY * 60;
            self.db.save_job(&j, user).map(|_| ())
        } else {
            match crew {
                Some(c) => self.db.move_job_to_crew(id, date, start_min, c, user),
                None => self.db.move_job(id, date, start_min, user),
            }
        };
        match result {
            Ok(()) => {
                self.push_undo(Undo::Restore(prev.clone()));
                self.reload();
                let crew_part = match crew {
                    Some(c) => format!(
                        " · {}",
                        self.crew(c).map(|c| c.name.clone()).unwrap_or_else(|| "Unassigned".into())
                    ),
                    None => String::new(),
                };
                let msg = match date {
                    Some(d) if newly_scheduled => format!(
                        "Scheduled {} on {} at {} for {} hrs{crew_part}   (Ctrl+Z to undo)",
                        prev.short_title(),
                        d.format("%a %b %-d"),
                        fmt_time(start_min),
                        HOURS_PER_PERSON_DAY
                    ),
                    Some(d) => format!(
                        "Moved {} to {} at {}{crew_part}   (Ctrl+Z to undo)",
                        prev.short_title(),
                        d.format("%a %b %-d"),
                        fmt_time(start_min)
                    ),
                    None => format!("Moved {} to Unscheduled   (Ctrl+Z to undo)", prev.short_title()),
                };
                self.set_status(msg);
            }
            Err(e) => self.set_error(format!("Could not move job: {e}")),
        }
    }

    pub fn delete_job(&mut self, id: i64) {
        if !self.require_edit() {
            return;
        }
        let Some(prev) = self.jobs.iter().find(|j| j.id == id).cloned() else {
            return;
        };
        match self.db.delete_job(id) {
            Ok(()) => {
                self.push_undo(Undo::Restore(prev.clone()));
                self.reload();
                self.set_status(format!("Deleted {}   (Ctrl+Z to undo)", prev.short_title()));
            }
            Err(e) => self.set_error(format!("Could not delete job: {e}")),
        }
    }

    pub fn undo_last(&mut self) {
        if !self.require_edit() {
            return;
        }
        let Some(u) = self.undo.pop() else {
            self.set_status("Nothing to undo");
            return;
        };
        let result = match u {
            Undo::Restore(mut job) => {
                if self.crew(job.crew_id).is_none() {
                    job.crew_id = None;
                }
                self.db.save_job(&job, self.current_user.as_ref().map(|u| u.id)).map(|_| ())
            }
            Undo::Remove(id) => self.db.delete_job(id),
        };
        match result {
            Ok(()) => {
                self.reload();
                self.set_status("Undone");
            }
            Err(e) => self.set_error(format!("Undo failed: {e}")),
        }
    }

    /// Returns true if the signed-in user may change data; otherwise says why.
    pub fn require_edit(&mut self) -> bool {
        if self.can_edit() {
            true
        } else {
            self.set_error("Your account is view-only. Ask an admin if you need to make changes.");
            false
        }
    }

    pub fn open_new_job(&mut self, date: Option<NaiveDate>, start_min: Option<i32>) {
        if !self.require_edit() {
            return;
        }
        let mut job = Job::new(date, start_min.unwrap_or(7 * 60));
        job.board = self.board();
        if let CrewFilter::Crew(id) = self.crew_filter {
            job.crew_id = Some(id);
        }
        self.editor = Some(JobEditor::new(job, true));
    }

    pub fn open_job(&mut self, id: i64) {
        if let Some(j) = self.jobs.iter().find(|j| j.id == id) {
            self.editor = Some(JobEditor::new(j.clone(), false));
        }
    }

    fn known_projects(&self) -> Vec<(String, String)> {
        let mut map: BTreeMap<String, String> = BTreeMap::new();
        for j in &self.jobs {
            let code = j.project_code.trim();
            if !code.is_empty() {
                let entry = map.entry(code.to_string()).or_default();
                if entry.is_empty() {
                    *entry = j.project_name.trim().to_string();
                }
            }
        }
        map.into_iter().collect()
    }

    fn known_field_labels(&self) -> Vec<String> {
        let mut set: BTreeSet<String> = FIELD_SUGGESTIONS.iter().map(|s| s.to_string()).collect();
        for j in &self.jobs {
            for f in &j.fields {
                if !f.label.trim().is_empty() {
                    set.insert(f.label.trim().to_string());
                }
            }
        }
        set.into_iter().collect()
    }

    fn shift(&mut self, n: i32) {
        self.focus = match self.view {
            ViewMode::Month => {
                let first = self.focus.with_day(1).unwrap_or(self.focus);
                if n >= 0 {
                    first.checked_add_months(Months::new(n as u32)).unwrap_or(first)
                } else {
                    first.checked_sub_months(Months::new((-n) as u32)).unwrap_or(first)
                }
            }
            ViewMode::Week => self.focus + Duration::days(7 * n as i64),
            ViewMode::Day => self.focus + Duration::days(n as i64),
            ViewMode::Crew => match self.crew_view_day {
                // Single-day timeline: step a day at a time.
                Some(d) => {
                    let d = d + Duration::days(n as i64);
                    self.crew_view_day = Some(d);
                    d
                }
                None => self.focus + Duration::days(7 * n as i64),
            },
        };
    }

    fn period_label(&self) -> String {
        match self.view {
            ViewMode::Month => self.focus.format("%B %Y").to_string(),
            ViewMode::Day => self.focus.format("%a, %B %-d, %Y").to_string(),
            ViewMode::Crew if self.crew_view_day.is_some() => {
                self.crew_view_day.unwrap_or(self.focus).format("%a, %B %-d, %Y").to_string()
            }
            ViewMode::Week | ViewMode::Crew => {
                let s = calendar::week_start(self.focus);
                let e = s + Duration::days(6);
                if s.month() == e.month() {
                    format!("{} – {}", s.format("%B %-d"), e.format("%-d, %Y"))
                } else if s.year() == e.year() {
                    format!("{} – {}", s.format("%b %-d"), e.format("%b %-d, %Y"))
                } else {
                    format!("{} – {}", s.format("%b %-d, %Y"), e.format("%b %-d, %Y"))
                }
            }
        }
    }

    fn top_bar(&mut self, ctx: &Context) {
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                let editable = self.can_edit();
                ui.menu_button("File", |ui| {
                    if ui.add_enabled(editable, Button::new("New Job…")).clicked() {
                        self.open_new_job(Some(self.focus), None);
                        ui.close_menu();
                    }
                    let empty = self.jobs.is_empty() && self.crews.is_empty() && self.is_admin();
                    if ui
                        .add_enabled(empty, Button::new("Load sample data"))
                        .on_disabled_hover_text("Admins only, and only while the schedule is empty")
                        .clicked()
                    {
                        match self.db.seed_sample(today()) {
                            Ok(()) => self.set_status("Sample crews and jobs loaded"),
                            Err(e) => self.set_error(format!("Could not load sample data: {e}")),
                        }
                        self.reload();
                        ui.close_menu();
                    }
                    if let Some(dir) = self.db.path.as_ref().and_then(|p| p.parent()) {
                        if ui.button("Open data folder").clicked() {
                            let _ = std::process::Command::new("explorer").arg(dir).spawn();
                            ui.close_menu();
                        }
                    }
                    ui.separator();
                    if ui.button("Exit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Edit", |ui| {
                    if ui
                        .add_enabled(editable && !self.undo.is_empty(), Button::new("Undo").shortcut_text("Ctrl+Z"))
                        .clicked()
                    {
                        self.undo_last();
                        ui.close_menu();
                    }
                });
                ui.separator();
                for b in Board::ALL {
                    let pending = self.jobs.iter().filter(|j| j.board == b && j.date.is_none()).count();
                    let mut label = format!("📅 {}", b.page_title());
                    if pending > 0 {
                        label.push_str(&format!("  ({pending} unscheduled)"));
                    }
                    ui.selectable_value(&mut self.tab, Tab::Schedule(b), RichText::new(label).size(15.0));
                }
                // One staff tab per group; each remembers its last sub-tab.
                for b in Board::ALL {
                    let on_this = matches!(self.tab, Tab::Staff(tb, _) if tb == b);
                    if let Tab::Staff(tb, page) = self.tab {
                        if tb == b {
                            self.last_staff_page[b.idx()] = page;
                        }
                    }
                    let label = format!("👥 {} Staff", b.as_str());
                    if ui.selectable_label(on_this, RichText::new(label).size(15.0)).clicked() && !on_this {
                        self.tab = Tab::Staff(b, self.last_staff_page[b.idx()]);
                    }
                }
                let due = self.vehicles.iter().filter(|v| !v.reminders(today()).is_empty()).count();
                let mut label = "🚚 Vehicles".to_string();
                if due > 0 {
                    label.push_str(&format!("  ({due} due)"));
                }
                ui.selectable_value(&mut self.tab, Tab::Vehicles, RichText::new(label).size(15.0))
                    .on_hover_text("Company vehicles. “Due” counts registration, insurance or service dates that are past or within 30 days.");
                if self.is_admin() {
                    ui.selectable_value(&mut self.tab, Tab::Users, RichText::new("🔑 Users").size(15.0));
                }

                // Account menu on the right.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let Some(me) = self.current_user.clone() else { return };
                    ui.menu_button(RichText::new(format!("👤 {}", me.name)).size(14.0), |ui| {
                        ui.label(RichText::new(&me.name).strong());
                        ui.weak(format!("{} · {}", me.username, me.role.as_str()));
                        ui.separator();
                        if ui.button("Change my password…").clicked() {
                            self.change_pw = Some(ChangePassword::default());
                            ui.close_menu();
                        }
                        if ui.button("Sign out").clicked() {
                            self.sign_out();
                            ui.close_menu();
                        }
                    });
                    if !me.role.can_edit() {
                        ui.label(RichText::new("VIEW ONLY").small().strong().color(Color32::from_rgb(200, 120, 20)));
                    }
                });
            });

            let Tab::Schedule(board) = self.tab else {
                return;
            };
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                egui::Frame::none()
                    .fill(board.accent())
                    .rounding(4.0)
                    .inner_margin(egui::Margin::symmetric(8.0, 3.0))
                    .show(ui, |ui| {
                        ui.label(RichText::new(board.as_str().to_uppercase()).color(Color32::WHITE).strong().size(14.0));
                    });
                ui.add_space(6.0);
                if ui.button(RichText::new("◀").size(16.0)).on_hover_text("Previous").clicked() {
                    self.shift(-1);
                }
                if ui.button("Today").clicked() {
                    self.focus = today();
                    if self.crew_view_day.is_some() {
                        self.crew_view_day = Some(today());
                    }
                }
                if ui.button(RichText::new("▶").size(16.0)).on_hover_text("Next").clicked() {
                    self.shift(1);
                }
                ui.add_space(8.0);
                ui.add_sized([250.0, 26.0], egui::Label::new(RichText::new(self.period_label()).size(20.0).strong()));
                ui.selectable_value(&mut self.view, ViewMode::Month, "Month");
                ui.selectable_value(&mut self.view, ViewMode::Week, "Week");
                ui.selectable_value(&mut self.view, ViewMode::Day, "Day")
                    .on_hover_text("One day by the hour, with room for each job's details");
                ui.selectable_value(&mut self.view, ViewMode::Crew, "Crew Week")
                    .on_hover_text("Each crew's week by the hour: see where every crew is and when");

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let editable = self.can_edit();
                    if ui
                        .add_enabled(editable, Button::new(RichText::new("+ New Job").strong().size(15.0)))
                        .on_disabled_hover_text("Your account is view-only")
                        .clicked()
                    {
                        let date = if self.view == ViewMode::Day {
                            Some(self.focus)
                        } else if self.view == ViewMode::Month && self.focus.month() != today().month() {
                            self.focus.with_day(1)
                        } else {
                            Some(today())
                        };
                        self.open_new_job(date, None);
                    }
                    if ui
                        .add_enabled(editable, Button::new("⬆ Import Jobs…"))
                        .on_hover_text(
                            "Add or update jobs from another system's CSV export (see templates\\Jobs Import Guide.md). \
                             You'll see a preview before anything is saved.",
                        )
                        .clicked()
                    {
                        crate::job_import::start(self);
                    }
                });
            });

            // Filters and display options on their own row, so nothing
            // overlaps on narrower windows.
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.label("Crew:");
                let filter_text = match self.crew_filter {
                    CrewFilter::All => "All crews".to_string(),
                    CrewFilter::Unassigned => "Unassigned".to_string(),
                    CrewFilter::Crew(id) => self
                        .crews
                        .iter()
                        .find(|c| c.id == id)
                        .map(|c| c.name.clone())
                        .unwrap_or_else(|| "All crews".into()),
                };
                egui::ComboBox::from_id_salt("crew_filter")
                    .selected_text(filter_text)
                    .width(170.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.crew_filter, CrewFilter::All, "All crews");
                        ui.selectable_value(&mut self.crew_filter, CrewFilter::Unassigned, "Unassigned");
                        for c in &self.crews {
                            ui.selectable_value(&mut self.crew_filter, CrewFilter::Crew(c.id), c.name.as_str());
                        }
                    });
                ui.label("Search:");
                ui.add(
                    TextEdit::singleline(&mut self.search)
                        .hint_text("code, project, lot, notes…")
                        .desired_width(190.0),
                );
                if !self.search.is_empty() && ui.small_button("✖").on_hover_text("Clear search").clicked() {
                    self.search.clear();
                }
                ui.checkbox(&mut self.hide_done, "Hide completed");
                if ui
                    .checkbox(&mut self.show_start_only, "Show Start Date Only")
                    .on_hover_text(
                        "Checked: a multi-day job shows only on the day it starts.\n\
                         Unchecked: it shows on every day it runs.",
                    )
                    .changed()
                {
                    let _ = self.db.set_setting("show_start_only", if self.show_start_only { "1" } else { "0" });
                }
            });
            ui.add_space(4.0);
        });
    }

    /// "Crews on: ◀ [date] ▶ Today" for the Rough-In Staff tabs.
    pub fn roster_day_picker(&mut self, ui: &mut egui::Ui) {
        let today = today();
        let day = self.roster_day;
        let mut pick = None;
        ui.label(RichText::new("Crews on").strong().size(14.0));
        if ui.button(RichText::new("◀").size(14.0)).on_hover_text("Previous day").clicked() {
            pick = Some(day - chrono::Duration::days(1));
        }
        let mut d = day;
        ui.add(egui_extras::DatePickerButton::new(&mut d).id_salt("roster_day").calendar_week(false));
        if d != day {
            pick = Some(d);
        }
        if ui.button(RichText::new("▶").size(14.0)).on_hover_text("Next day").clicked() {
            pick = Some(day + chrono::Duration::days(1));
        }
        let mut label = RichText::new(day.format("%A").to_string()).size(14.0);
        if day == today {
            label = label.color(ui.visuals().selection.stroke.color);
        }
        ui.label(label);
        if day != today && ui.button("Today").clicked() {
            pick = Some(today);
        }
        let (text, tip) = match self.roster_source {
            Some(s) if s == day => (
                "set for this day".to_string(),
                "These crews were set for this day. Changes here affect only this day.".to_string(),
            ),
            Some(s) => (
                format!("same as {}", Self::short_day(s)),
                format!(
                    "Nothing changed on this day yet, so it uses the crews from {}. \
                     Your first change copies those crews to this day and changes only this day.",
                    Self::short_day(s)
                ),
            ),
            None => (
                "original crews".to_string(),
                "No day has been changed yet up to here, so this uses the crews as first set up. \
                 Your first change copies them to this day and changes only this day."
                    .to_string(),
            ),
        };
        ui.label(RichText::new(text).weak()).on_hover_text(tip);
        if let Some(p) = pick {
            self.set_roster_day(p);
        }
    }

    /// Sub-tab row inside a staff tab: Crews | Employees (| Attendance).
    fn staff_subtabs(&mut self, ctx: &Context, board: Board) {
        egui::TopBottomPanel::top("staff_subtabs").show(ctx, |ui| {
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{} Staff", board.as_str())).strong().size(16.0).color(board.accent()));
                ui.separator();
                ui.selectable_value(&mut self.tab, Tab::Staff(board, StaffPage::Crews), RichText::new("Crews").size(14.0));
                let available = self
                    .employees
                    .iter()
                    .filter(|e| e.staff == board && e.active && e.crew_id.is_none())
                    .count();
                let label = if available > 0 {
                    format!("Employees  ({available} available)")
                } else {
                    "Employees".to_string()
                };
                ui.selectable_value(&mut self.tab, Tab::Staff(board, StaffPage::Employees), RichText::new(label).size(14.0));
                if board == Board::RoughIn {
                    ui.selectable_value(
                        &mut self.tab,
                        Tab::Staff(board, StaffPage::Attendance),
                        RichText::new("Attendance").size(14.0),
                    );
                    // Rough-In crews are set day by day; everything on these
                    // sub-tabs shows and edits the crews on this day.
                    ui.separator();
                    self.roster_day_picker(ui);
                }
            });
            ui.add_space(3.0);
        });
    }

    fn status_bar(&mut self, ctx: &Context) {
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Right-hand items first; the message gets whatever width is left,
                // and is cut short with "…" (full text on hover) instead of overlapping.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    match &self.db.path {
                        Some(p) => ui.weak(format!("Data: {}", p.display())),
                        None => ui.colored_label(Color32::from_rgb(220, 60, 60), "Temporary database — not saved"),
                    };
                    ui.separator();
                    ui.weak(format!("{} jobs · {} crews", self.jobs.len(), self.crews.len()));
                    if let Some(me) = &self.current_user {
                        ui.separator();
                        ui.weak(format!("Signed in as {} ({})", me.name, me.role.as_str()));
                    }
                    ui.separator();
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let mut text = egui::RichText::new(self.status.as_str());
                        if self.status_is_error {
                            text = text.color(Color32::from_rgb(220, 60, 60));
                        }
                        ui.add(egui::Label::new(text).truncate())
                            .on_hover_text(self.status.as_str());
                    });
                });
            });
        });
    }

    fn show_editor(&mut self, ctx: &Context) {
        let Some(mut editor) = self.editor.take() else {
            return;
        };
        let projects = self.known_projects();
        let labels = self.known_field_labels();
        let read_only = !self.can_edit();
        let audit = if editor.job.id > 0 { self.audit_line(editor.job.id) } else { None };
        let crew_day = match (editor.job.date, editor.job.crew_id) {
            (Some(d), Some(c)) => Some((d, self.crew_names_on(c, d))),
            _ => None,
        };
        // Rough-In jobs show allocated man-hours, worked out live from the
        // daily crews (so it follows crew and attendance changes).
        let allocation =
            (editor.job.board == Board::RoughIn).then(|| self.job_crew_by_day(&editor.job.clone()));
        let outcome = job_editor::show(
            ctx,
            &mut editor,
            &self.crews,
            &projects,
            &labels,
            self.focus,
            read_only,
            audit.as_deref(),
            crew_day,
            allocation,
        );
        match outcome {
            EditorOutcome::Open => self.editor = Some(editor),
            EditorOutcome::Close => {}
            EditorOutcome::Save(job) => {
                if self.save_job(job).is_none() {
                    self.editor = Some(editor);
                }
            }
            EditorOutcome::SaveAndDuplicate(job) => {
                if self.save_job(job.clone()).is_some() {
                    let mut copy = job;
                    copy.id = 0;
                    copy.lot.clear();
                    copy.status = "Scheduled".into();
                    for f in &mut copy.fields {
                        f.value.clear();
                    }
                    self.editor = Some(JobEditor::new(copy, true));
                    self.set_status("Saved. Enter the next lot # for the duplicate.");
                } else {
                    self.editor = Some(editor);
                }
            }
            EditorOutcome::Delete(id) => self.delete_job(id),
        }
    }
}

impl eframe::App for SchedulerApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.drop_targets.clear();

        // Nobody signed in: show only the sign-in (or first-admin) screen.
        if self.current_user.is_none() {
            users::auth_screen(self, ctx);
            return;
        }
        users::guard_tab(self);

        if self.editor.is_none()
            && self.drag.is_none()
            && self.can_edit()
            && !ctx.wants_keyboard_input()
            && ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Z)))
        {
            self.undo_last();
        }

        self.top_bar(ctx);
        self.status_bar(ctx);

        match self.tab {
            Tab::Schedule(_) => {
                calendar::unscheduled_panel(self, ctx);
                egui::CentralPanel::default().show(ctx, |ui| match self.view {
                    ViewMode::Month => calendar::month_view(self, ui),
                    ViewMode::Week => calendar::week_view(self, ui),
                    ViewMode::Day => calendar::day_view(self, ui),
                    ViewMode::Crew => calendar::crew_view(self, ui),
                });
            }
            Tab::Staff(board, _) => {
                self.staff_subtabs(ctx, board);
                // Read the page after the sub-tab row, so a click shows at once.
                let page = match self.tab {
                    Tab::Staff(_, p) => p,
                    _ => StaffPage::Crews,
                };
                match page {
                    StaffPage::Crews => crews::show(self, ctx, board),
                    StaffPage::Employees => employees::show(self, ctx, board),
                    // Attendance is a Rough-In Staff page only.
                    StaffPage::Attendance if board == Board::RoughIn => attendance::show(self, ctx),
                    StaffPage::Attendance => self.tab = Tab::Staff(board, StaffPage::Crews),
                }
            }
            Tab::Vehicles => vehicles::show(self, ctx),
            Tab::Users => users::users_page(self, ctx),
        }

        self.show_editor(ctx);
        crate::employee_import::window(self, ctx);
        crate::job_import::window(self, ctx);
        users::change_password_window(self, ctx);
        calendar::finish_drag(self, ctx);
    }
}
