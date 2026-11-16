use chrono::{Datelike, NaiveDate};
use eframe::egui::Color32;

pub const STATUSES: &[&str] = &[
    "Scheduled",
    "Confirmed",
    "In Progress",
    "Complete",
    "On Hold",
    "Cancelled",
];

/// Quick picks for the water tank size on Trim jobs (any text is allowed).
pub const TANK_SIZES: &[&str] = &["30 gal", "40 gal", "50 gal", "75 gal", "80 gal", "Tankless"];

/// Quick picks for the kitchen sink on Trim jobs (any text is allowed).
pub const KITCHEN_SINKS: &[&str] = &[
    "Single Bowl",
    "Double Bowl",
    "Farmhouse / Apron",
    "Undermount",
    "Drop-in",
    "Prep / Bar Sink",
];

/// Which schedule page (tab) a job lives on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Board {
    #[default]
    RoughIn,
    Trim,
}

impl Board {
    pub const ALL: [Board; 2] = [Board::RoughIn, Board::Trim];

    /// 0 for Rough-In, 1 for Trim (for per-group state arrays).
    pub fn idx(self) -> usize {
        match self {
            Board::RoughIn => 0,
            Board::Trim => 1,
        }
    }

    pub fn other(self) -> Board {
        match self {
            Board::RoughIn => Board::Trim,
            Board::Trim => Board::RoughIn,
        }
    }

    /// Value stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            Board::RoughIn => "Rough-In",
            Board::Trim => "Trim",
        }
    }

    pub fn from_db(s: &str) -> Board {
        if s.eq_ignore_ascii_case("trim") { Board::Trim } else { Board::RoughIn }
    }

    pub fn page_title(self) -> &'static str {
        match self {
            Board::RoughIn => "Rough-In Schedule",
            Board::Trim => "Trim Schedule",
        }
    }

    /// Phase presets offered in the job editor for this schedule.
    pub fn phases(self) -> &'static [&'static str] {
        match self {
            Board::RoughIn => &["Underground", "Rough-In", "Top-Out", "Tub / Shower Set", "Gas", "Inspection"],
            Board::Trim => &["Trim / Final", "Fixture Set", "Water Heater", "Gas Final", "Inspection", "Service / Warranty"],
        }
    }

    /// Accent color for the page header.
    pub fn accent(self) -> Color32 {
        match self {
            Board::RoughIn => Color32::from_rgb(52, 120, 246),
            Board::Trim => Color32::from_rgb(142, 68, 173),
        }
    }
}

pub const MEMBER_ROLES: &[&str] = &["Lead", "Foreman", "Journeyman", "Apprentice", "Helper", "Driver", "Field Supervisor"];

/// Suggested extra-field names offered in the job editor.
pub const FIELD_SUGGESTIONS: &[&str] = &[
    "Permit #",
    "Inspection Date",
    "Superintendent",
    "Site Contact Phone",
    "Gate / Lockbox Code",
    "Fixture Package",
    "PO #",
];

pub const CREW_PALETTE: &[[u8; 3]] = &[
    [52, 120, 246],
    [230, 126, 34],
    [39, 174, 96],
    [142, 68, 173],
    [231, 76, 60],
    [22, 160, 133],
    [241, 196, 15],
    [52, 73, 94],
    [211, 84, 0],
    [41, 128, 185],
    [192, 57, 43],
    [127, 140, 141],
];

pub const UNASSIGNED_COLOR: Color32 = Color32::from_rgb(140, 140, 140);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CustomField {
    pub label: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    /// 0 = not yet saved.
    pub id: i64,
    pub board: Board,
    pub project_code: String,
    pub project_name: String,
    pub lot: String,
    pub phase: String,
    pub status: String,
    pub crew_id: Option<i64>,
    /// None = unscheduled.
    pub date: Option<NaiveDate>,
    /// Minutes after midnight.
    pub start_min: i32,
    /// Hours worked each day, in minutes.
    pub duration_min: i32,
    /// How many days the job runs (1 = one day). Saturdays and Sundays are
    /// skipped unless `weekend` includes them, and then count as job days.
    pub days: i32,
    pub weekend: Weekend,
    pub notes: String,
    pub comments: String,
    /// Water heater tank size (Trim jobs), e.g. "50 gal" or "Tankless".
    pub tank_size: String,
    /// Kitchen sink (Trim jobs), e.g. "Double Bowl" or a model number.
    pub kitchen_sink: String,
    pub fields: Vec<CustomField>,
    /// The job's ID in the system it was imported from (empty = made here).
    pub source_id: String,
}

impl Job {
    pub fn new(date: Option<NaiveDate>, start_min: i32) -> Self {
        Self {
            id: 0,
            board: Board::RoughIn,
            project_code: String::new(),
            project_name: String::new(),
            lot: String::new(),
            phase: String::new(),
            status: "Scheduled".into(),
            crew_id: None,
            date,
            start_min,
            // New jobs default to a full 8-hour day.
            duration_min: HOURS_PER_PERSON_DAY * 60,
            days: 1,
            weekend: Weekend::SKIP,
            notes: String::new(),
            comments: String::new(),
            tank_size: String::new(),
            kitchen_sink: String::new(),
            fields: Vec::new(),
            source_id: String::new(),
        }
    }

    pub fn end_min(&self) -> i32 {
        self.start_min + self.duration_min
    }

    /// Every day the job runs: its start date, then the following weekdays
    /// until `days` work days are covered. Empty when unscheduled.
    pub fn work_dates(&self) -> Vec<NaiveDate> {
        match self.date {
            Some(d) => work_dates(d, self.days, self.weekend),
            None => Vec::new(),
        }
    }

    /// Which day of the job `d` is (0 = start day), if it runs that day.
    pub fn day_index(&self, d: NaiveDate) -> Option<usize> {
        let start = self.date?;
        if d < start {
            return None;
        }
        if self.days <= 1 {
            return (d == start).then_some(0);
        }
        self.work_dates().iter().position(|x| *x == d)
    }

    pub fn last_date(&self) -> Option<NaiveDate> {
        self.work_dates().last().copied()
    }
}

pub const MAX_JOB_DAYS: i32 = 60;

/// Labor allocated per person per day on a job.
pub const HOURS_PER_PERSON_DAY: i32 = 8;

/// One job day's share of its crew: everyone on the crew counts 8 hours,
/// split evenly across the crew's jobs that day.
pub fn day_hours(people: usize, crew_jobs_that_day: usize) -> f32 {
    (people as i32 * HOURS_PER_PERSON_DAY) as f32 / crew_jobs_that_day.max(1) as f32
}

/// Allocated man-hours for a job: `days` has (people on the crew, jobs the
/// crew has that day) for each day the job runs.
pub fn allocated_hours(days: &[(usize, usize)]) -> f32 {
    days.iter().map(|(p, j)| day_hours(*p, *j)).sum()
}

/// "48", "21.3": whole hours without decimals.
pub fn fmt_hours(h: f32) -> String {
    let r = (h * 10.0).round() / 10.0;
    if r.fract() == 0.0 {
        format!("{}", r as i64)
    } else {
        format!("{r:.1}")
    }
}

/// Which weekend days a job works. Weekends are skipped unless ticked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Weekend {
    pub sat: bool,
    pub sun: bool,
}

impl Weekend {
    pub const SKIP: Weekend = Weekend { sat: false, sun: false };

    /// True for a Saturday/Sunday the job doesn't work.
    fn skips(self, d: NaiveDate) -> bool {
        match d.weekday() {
            chrono::Weekday::Sat => !self.sat,
            chrono::Weekday::Sun => !self.sun,
            _ => false,
        }
    }

    /// e.g. "weekends skipped", "Saturdays included".
    pub fn describe(self) -> &'static str {
        match (self.sat, self.sun) {
            (false, false) => "weekends skipped",
            (true, false) => "Saturdays included, Sundays skipped",
            (false, true) => "Sundays included, Saturdays skipped",
            (true, true) => "weekends included",
        }
    }
}

/// `start`, then the following days the job works (weekdays, plus any
/// weekend days ticked) until `days` days are listed. A job may start on
/// any day; only the days after it are skipped.
pub fn work_dates(start: NaiveDate, days: i32, weekend: Weekend) -> Vec<NaiveDate> {
    let n = days.clamp(1, MAX_JOB_DAYS) as usize;
    let mut out = Vec::with_capacity(n);
    out.push(start);
    let mut d = start;
    while out.len() < n {
        d += chrono::Duration::days(1);
        if !weekend.skips(d) {
            out.push(d);
        }
    }
    out
}

/// The start date that puts work day number `index` (0-based) on `d`:
/// steps back `index` working days.
pub fn start_for_day(d: NaiveDate, index: usize, weekend: Weekend) -> NaiveDate {
    let mut s = d;
    let mut left = index;
    while left > 0 {
        s -= chrono::Duration::days(1);
        if !weekend.skips(s) {
            left -= 1;
        }
    }
    s
}

impl Job {

    pub fn lot_label(&self) -> String {
        let lot = self.lot.trim();
        if lot.is_empty() {
            String::new()
        } else if lot.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            format!("Lot {lot}")
        } else {
            lot.to_string()
        }
    }

    /// e.g. "OAK-101 · Lot 12"
    pub fn short_title(&self) -> String {
        let mut s = self.project_code.trim().to_string();
        if s.is_empty() {
            s = self.project_name.trim().to_string();
        }
        let lot = self.lot_label();
        if !lot.is_empty() {
            if !s.is_empty() {
                s.push_str(" · ");
            }
            s.push_str(&lot);
        }
        if s.is_empty() {
            s = "(untitled job)".into();
        }
        s
    }

    pub fn is_complete(&self) -> bool {
        self.status == "Complete"
    }

    pub fn is_cancelled(&self) -> bool {
        self.status == "Cancelled"
    }

    pub fn status_icon(&self) -> &'static str {
        match self.status.as_str() {
            "Complete" => "✔ ",
            "Cancelled" => "✖ ",
            "On Hold" => "⏸ ",
            _ => "",
        }
    }
}

/// What a signed-in user is allowed to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Admin,
    Scheduler,
    Viewer,
}

impl Role {
    pub const ALL: [Role; 3] = [Role::Admin, Role::Scheduler, Role::Viewer];

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "Admin",
            Role::Scheduler => "Scheduler",
            Role::Viewer => "Viewer",
        }
    }

    pub fn from_db(s: &str) -> Role {
        match s {
            "Admin" => Role::Admin,
            "Scheduler" => Role::Scheduler,
            _ => Role::Viewer,
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Role::Admin => "Everything, including managing users",
            Role::Scheduler => "Create, move and edit jobs, crews and employees",
            Role::Viewer => "View the schedule only; can't change anything",
        }
    }

    pub fn can_edit(self) -> bool {
        !matches!(self, Role::Viewer)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct User {
    /// 0 = not yet saved.
    pub id: i64,
    pub name: String,
    pub username: String,
    pub role: Role,
    pub active: bool,
}

/// Who created / last changed a job, and when (UTC "YYYY-MM-DD HH:MM:SS").
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JobAudit {
    pub created_by: Option<i64>,
    pub created_at: String,
    pub updated_by: Option<i64>,
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Employee {
    /// 0 = not yet saved.
    pub id: i64,
    pub name: String,
    pub role: String,
    pub phone: String,
    pub active: bool,
    pub notes: String,
    /// Which staff list they are on (Rough-In or Trim).
    pub staff: Board,
    /// The one crew this employee is on; None = available.
    pub crew_id: Option<i64>,
    /// Plumbing license level: Apprentice, Resident, Journey (any text).
    pub license: String,
    /// Payroll / HR employee ID, used to match people on later imports.
    pub employee_no: String,
}

/// Quick picks for an employee's license level (any text is allowed).
pub const LICENSE_LEVELS: &[&str] = &["Apprentice", "Resident", "Journey"];

impl Employee {
    pub fn new() -> Self {
        Self {
            id: 0,
            name: String::new(),
            role: String::new(),
            phone: String::new(),
            active: true,
            notes: String::new(),
            staff: Board::default(),
            crew_id: None,
            license: String::new(),
            employee_no: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Crew {
    /// 0 = not yet saved.
    pub id: i64,
    pub name: String,
    pub foreman: String,
    pub phone: String,
    pub color: [u8; 3],
    pub active: bool,
    pub notes: String,
    /// Which staff group the crew belongs to (Rough-In or Trim).
    pub board: Board,
    /// Employee ids on this crew, in display order.
    pub member_ids: Vec<i64>,
    /// The truck/van this crew goes out in (a Vehicles record). Two crews
    /// may share one.
    pub vehicle_id: Option<i64>,
}

impl Crew {
    pub fn new(color: [u8; 3]) -> Self {
        Self {
            id: 0,
            name: String::new(),
            foreman: String::new(),
            phone: String::new(),
            color,
            active: true,
            notes: String::new(),
            board: Board::default(),
            member_ids: Vec::new(),
            vehicle_id: None,
        }
    }

    pub fn color32(&self) -> Color32 {
        Color32::from_rgb(self.color[0], self.color[1], self.color[2])
    }
}

pub fn fmt_time(min: i32) -> String {
    let h = (min / 60).rem_euclid(24);
    let m = min.rem_euclid(60);
    let (h12, ap) = match h {
        0 => (12, "AM"),
        1..=11 => (h, "AM"),
        12 => (12, "PM"),
        _ => (h - 12, "PM"),
    };
    format!("{h12}:{m:02} {ap}")
}

/// Compact time for month cells: "7a", "12:30p".
pub fn fmt_time_compact(min: i32) -> String {
    let h = (min / 60).rem_euclid(24);
    let m = min.rem_euclid(60);
    let (h12, ap) = match h {
        0 => (12, "a"),
        1..=11 => (h, "a"),
        12 => (12, "p"),
        _ => (h - 12, "p"),
    };
    if m == 0 {
        format!("{h12}{ap}")
    } else {
        format!("{h12}:{m:02}{ap}")
    }
}

pub fn fmt_duration(min: i32) -> String {
    let (h, m) = (min / 60, min % 60);
    match (h, m) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

/// Black or white text, whichever reads better on `bg`.
pub fn text_color_for(bg: Color32) -> Color32 {
    let lum = 0.299 * bg.r() as f32 + 0.587 * bg.g() as f32 + 0.114 * bg.b() as f32;
    if lum > 160.0 {
        Color32::from_rgb(20, 20, 20)
    } else {
        Color32::WHITE
    }
}

/// Linear blend between two opaque colors.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Attendance choices: (stored value, short label shown in the grid, color).
pub const ATTENDANCE: &[(&str, &str, [u8; 3])] = &[
    ("Present", "P", [39, 150, 80]),
    ("Late", "Late", [230, 150, 30]),
    ("Absent", "Abs", [220, 60, 60]),
    ("Sick", "Sick", [142, 68, 173]),
    ("PTO", "PTO", [52, 120, 246]),
    ("Off", "Off", [127, 140, 141]),
];

/// One employee's attendance for one day.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AttendanceEntry {
    pub status: String,
    pub note: String,
}

// ---------------------------------------------------------------- vehicles

/// Vehicle statuses, with a badge color.
pub const VEHICLE_STATUSES: &[(&str, [u8; 3])] = &[
    ("Active", [39, 150, 80]),
    ("In Shop", [230, 150, 30]),
    ("Out of Service", [200, 60, 60]),
    ("Spare", [60, 130, 200]),
];

/// Shop visit details (shown while a vehicle is In Shop), kept in `info`.
pub const SHOP_NAME: &str = "shop_name";
pub const SHOP_WORK: &str = "shop_work";
pub const SHOP_DROP_OFF: &str = "shop_drop_off";
pub const SHOP_REPAIR_TIME: &str = "shop_repair_time";
pub const SHOP_COST: &str = "shop_cost";
pub const SHOP_PICKUP: &str = "shop_pickup";
/// What the repair actually cost (entered when it's done).
pub const SHOP_FINAL_COST: &str = "shop_final_cost";
pub const SHOP_KEYS: &[&str] =
    &[SHOP_NAME, SHOP_WORK, SHOP_DROP_OFF, SHOP_REPAIR_TIME, SHOP_COST, SHOP_PICKUP, SHOP_FINAL_COST];

/// Parses a dollar amount typed or stored as text ("$1,234.50").
pub fn parse_money(s: &str) -> Option<f64> {
    s.trim().trim_start_matches('$').replace(',', "").trim().parse().ok()
}

/// "$1,234.50"
pub fn fmt_money(v: f64) -> String {
    let cents = (v * 100.0).round() as i64;
    let whole = (cents / 100).to_string();
    let mut out = String::new();
    for (i, ch) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    format!("${out}.{:02}", (cents % 100).abs())
}

/// One shop visit for a vehicle. A visit is open (returned_on is None) while
/// the vehicle is In Shop; it's closed when the vehicle comes back.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Repair {
    /// 0 = not yet saved.
    pub id: i64,
    pub vehicle_id: i64,
    /// When it was put in the shop in the app: local "YYYY-MM-DD HH:MM".
    pub opened_at: String,
    pub opened_by: Option<i64>,
    pub shop: String,
    pub work: String,
    pub drop_off: Option<NaiveDate>,
    /// Estimated time of repair, as typed ("3 days").
    pub repair_time: String,
    pub est_cost: Option<f64>,
    pub est_pickup: Option<NaiveDate>,
    /// When it came back; None = still in the shop.
    pub returned_on: Option<NaiveDate>,
    pub final_cost: Option<f64>,
    pub notes: String,
}

impl Repair {
    /// First day it was in the shop: the drop-off date, else the stamp date.
    pub fn start(&self) -> Option<NaiveDate> {
        self.drop_off.or_else(|| NaiveDate::parse_from_str(self.opened_at.get(..10).unwrap_or(""), "%Y-%m-%d").ok())
    }

    /// Days in the shop (so far, if it's still there).
    pub fn days(&self, today: NaiveDate) -> Option<i64> {
        let end = self.returned_on.unwrap_or(today);
        self.start().map(|s| (end - s).num_days().max(0))
    }

    /// The final cost, else the estimate.
    pub fn cost(&self) -> Option<f64> {
        self.final_cost.or(self.est_cost)
    }
}

/// Groups of extra vehicle details, in the order the form shows them.
pub const VEHICLE_SECTIONS: &[&str] = &["Registration & IDs", "Driver", "Oil & Service History"];

/// One extra text detail on a vehicle (stored in `vehicle_info`).
pub struct VehicleField {
    pub key: &'static str,
    pub label: &'static str,
    /// Index into VEHICLE_SECTIONS.
    pub section: usize,
    pub multiline: bool,
}

const fn vf(key: &'static str, label: &'static str, section: usize, multiline: bool) -> VehicleField {
    VehicleField { key, label, section, multiline }
}

/// Extra details, matching the columns of the fleet management log.
pub const VEHICLE_FIELDS: &[VehicleField] = &[
    vf("tab_number", "Tab #", 0, false),
    vf("title", "Title", 0, false),
    vf("toll_pass", "Good To Go pass", 0, false),
    vf("gps", "Samsara GPS", 0, false),
    vf("dashcam", "Dash cam SN", 0, false),
    vf("driver", "Driver", 1, false),
    vf("driver_phone", "Driver phone", 1, false),
    vf("supervisor", "Supervisor", 1, false),
    vf("odometer_updated", "Mileage updated", 2, false),
    vf("oil_log", "Oil change log", 2, true),
    vf("oil_due", "Due", 2, false),
    vf("brakes", "Brakes", 2, true),
    vf("tires", "Tires", 2, true),
    vf("tire_size", "Tire size", 2, false),
    vf("tune", "Tune-up / major service", 2, true),
    vf("battery", "Battery", 2, false),
    vf("belts", "Belts", 2, false),
    vf("other", "Other repairs", 2, true),
    vf("recalls", "Recalls", 2, true),
];

/// Quick picks for the vehicle type (any text is allowed).
pub const VEHICLE_KINDS: &[&str] = &["Pickup", "Service Van", "Box Truck", "Flatbed", "Dump Truck", "Trailer", "Car"];

/// Days ahead that a registration / insurance / service date counts as "due soon".
pub const DUE_SOON_DAYS: i64 = 30;

#[derive(Clone, Debug, PartialEq)]
pub struct Vehicle {
    /// 0 = not yet saved.
    pub id: i64,
    /// Unit / truck number, e.g. "Truck 12".
    pub unit: String,
    pub kind: String,
    pub year: String,
    pub make: String,
    pub model: String,
    pub plate: String,
    pub vin: String,
    pub status: String,
    /// Crew the vehicle goes out with (either staff group).
    pub crew_id: Option<i64>,
    /// Employee who usually drives it / takes it home.
    pub driver_id: Option<i64>,
    pub odometer: i64,
    pub registration_due: Option<NaiveDate>,
    pub insurance_due: Option<NaiveDate>,
    pub next_service: Option<NaiveDate>,
    pub purchase_date: Option<NaiveDate>,
    pub last_oil_change: Option<NaiveDate>,
    pub notes: String,
    /// Extra details by VEHICLE_FIELDS key (empty values aren't stored).
    pub info: std::collections::BTreeMap<String, String>,
}

impl Vehicle {
    pub fn new() -> Self {
        Self {
            id: 0,
            unit: String::new(),
            kind: String::new(),
            year: String::new(),
            make: String::new(),
            model: String::new(),
            plate: String::new(),
            vin: String::new(),
            status: VEHICLE_STATUSES[0].0.into(),
            crew_id: None,
            driver_id: None,
            odometer: 0,
            registration_due: None,
            insurance_due: None,
            next_service: None,
            purchase_date: None,
            last_oil_change: None,
            notes: String::new(),
            info: Default::default(),
        }
    }

    pub fn get(&self, key: &str) -> &str {
        self.info.get(key).map(|s| s.as_str()).unwrap_or("")
    }

    /// A date kept in `info` as YYYY-MM-DD.
    pub fn info_date(&self, key: &str) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(self.get(key), "%Y-%m-%d").ok()
    }

    pub fn set_info_date(&mut self, key: &str, d: Option<NaiveDate>) {
        match d {
            Some(d) => {
                self.info.insert(key.to_string(), d.format("%Y-%m-%d").to_string());
            }
            None => {
                self.info.remove(key);
            }
        }
    }

    /// The current shop visit as a Repair (from the Shop fields).
    pub fn shop_visit(&self) -> Repair {
        Repair {
            vehicle_id: self.id,
            shop: self.get(SHOP_NAME).trim().to_string(),
            work: self.get(SHOP_WORK).trim().to_string(),
            drop_off: self.info_date(SHOP_DROP_OFF),
            repair_time: self.get(SHOP_REPAIR_TIME).trim().to_string(),
            est_cost: parse_money(self.get(SHOP_COST)),
            est_pickup: self.info_date(SHOP_PICKUP),
            final_cost: parse_money(self.get(SHOP_FINAL_COST)),
            ..Repair::default()
        }
    }

    /// "#12" for a plain truck number, else the unit as typed.
    pub fn label(&self) -> String {
        let u = self.unit.trim();
        if u.is_empty() {
            "(no unit #)".into()
        } else if u.chars().all(|c| c.is_ascii_digit()) {
            format!("#{u}")
        } else {
            u.to_string()
        }
    }

    /// Sort key: truck numbers in number order, then anything else by name.
    pub fn sort_key(&self) -> (u64, String) {
        let u = self.unit.trim();
        let digits: String = u.chars().take_while(|c| c.is_ascii_digit()).collect();
        (digits.parse().unwrap_or(u64::MAX), u.to_lowercase())
    }

    pub fn status_color(&self) -> Color32 {
        let [r, g, b] = VEHICLE_STATUSES
            .iter()
            .find(|(s, _)| *s == self.status)
            .map(|x| x.1)
            .unwrap_or([140, 140, 140]);
        Color32::from_rgb(r, g, b)
    }

    /// "2022 Ford F-250" (whatever parts are filled in).
    pub fn description(&self) -> String {
        [self.year.trim(), self.make.trim(), self.model.trim()]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Dates that are past or within DUE_SOON_DAYS of `today`, as
    /// (label, date, overdue).
    pub fn reminders(&self, today: NaiveDate) -> Vec<(&'static str, NaiveDate, bool)> {
        [
            ("Tab", self.registration_due),
            ("Insurance", self.insurance_due),
            ("Service", self.next_service),
        ]
        .into_iter()
        .filter_map(|(label, d)| {
            let d = d?;
            let days = (d - today).num_days();
            (days <= DUE_SOON_DAYS).then_some((label, d, days < 0))
        })
        .collect()
    }
}

// ---------------------------------------------------------------- drivers

/// Driver's license statuses, with a badge color.
pub const LICENSE_STATUSES: &[(&str, [u8; 3])] = &[
    ("Valid", [39, 150, 80]),
    ("Restricted", [220, 140, 20]),
    ("Expired", [210, 50, 50]),
    ("Suspended", [210, 50, 50]),
    ("Revoked", [210, 50, 50]),
    ("No license", [140, 140, 140]),
];

/// The three kinds of driving incident, with the section heading for each.
pub const INCIDENT_KINDS: &[(&str, &str)] = &[
    ("Accident", "Accidents"),
    ("Moving Violation", "Moving Violations"),
    ("Parking Ticket", "Parking Tickets"),
];

pub const INCIDENT_STATUSES: &[&str] = &["Open", "Paid", "Contested", "Dismissed", "Closed"];

/// An employee's driver's license (one per employee; None in the app = not entered).
#[derive(Clone, Debug, PartialEq)]
pub struct DriverLicense {
    pub employee_id: i64,
    pub number: String,
    /// Issuing state, e.g. WA.
    pub state: String,
    /// License class / CDL, e.g. "C", "CDL-A".
    pub class: String,
    pub status: String,
    pub expires: Option<NaiveDate>,
    /// Points on the license (as reported by the state / MVR).
    pub points: i32,
    /// When the motor vehicle record was last checked.
    pub last_checked: Option<NaiveDate>,
    pub endorsements: String,
    pub notes: String,
}

impl DriverLicense {
    pub fn new(employee_id: i64) -> Self {
        Self {
            employee_id,
            number: String::new(),
            state: "WA".into(),
            class: String::new(),
            status: "Valid".into(),
            expires: None,
            points: 0,
            last_checked: None,
            endorsements: String::new(),
            notes: String::new(),
        }
    }

    /// A short warning for the employee list, if anything needs attention.
    pub fn warning(&self, today: NaiveDate) -> Option<String> {
        if self.status != "Valid" && self.status != "Restricted" {
            return Some(format!("License {}", self.status.to_lowercase()));
        }
        match self.expires {
            Some(d) if d < today => Some("License expired".into()),
            Some(d) if (d - today).num_days() <= DUE_SOON_DAYS => Some(format!("License expires {}", d.format("%b %-d"))),
            _ => None,
        }
    }
}

/// An accident, moving violation or parking ticket.
#[derive(Clone, Debug, PartialEq)]
pub struct Incident {
    /// 0 = not yet saved.
    pub id: i64,
    pub employee_id: i64,
    /// One of INCIDENT_KINDS.
    pub kind: String,
    pub date: Option<NaiveDate>,
    /// The company vehicle involved, if any.
    pub vehicle_id: Option<i64>,
    pub location: String,
    pub description: String,
    /// Ticket / citation / claim / police report number.
    pub reference: String,
    /// Fine, or the cost of the damage for an accident.
    pub amount: Option<f64>,
    /// Points the violation put on the license.
    pub points: i32,
    /// Accidents: was the driver at fault?
    pub at_fault: bool,
    pub status: String,
    pub notes: String,
}

impl Incident {
    pub fn new(employee_id: i64, kind: &str, today: NaiveDate) -> Self {
        Self {
            id: 0,
            employee_id,
            kind: kind.into(),
            date: Some(today),
            vehicle_id: None,
            location: String::new(),
            description: String::new(),
            reference: String::new(),
            amount: None,
            points: 0,
            at_fault: false,
            status: "Open".into(),
            notes: String::new(),
        }
    }
}
