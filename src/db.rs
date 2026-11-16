use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{Datelike, Duration, NaiveDate, Weekday};
use rusqlite::{params, Connection, OptionalExtension, Result};

use crate::model::*;

const SCHEMA: &str = r#"
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS crews (
    id       INTEGER PRIMARY KEY,
    name     TEXT    NOT NULL,
    foreman  TEXT    NOT NULL DEFAULT '',
    phone    TEXT    NOT NULL DEFAULT '',
    color    INTEGER NOT NULL DEFAULT 3438838,
    active   INTEGER NOT NULL DEFAULT 1,
    notes    TEXT    NOT NULL DEFAULT '',
    board    TEXT    NOT NULL DEFAULT 'Trim',     -- staff group: 'Rough-In' or 'Trim'
    vehicle_id INTEGER REFERENCES vehicles(id) ON DELETE SET NULL   -- v13: the crew's truck
);

-- People who sign in to the app. Users are deactivated, never deleted, so the
-- "created by / changed by" history on jobs stays meaningful.
CREATE TABLE IF NOT EXISTS users (
    id             INTEGER PRIMARY KEY,
    name           TEXT    NOT NULL,
    username       TEXT    NOT NULL UNIQUE COLLATE NOCASE,
    password_hash  TEXT    NOT NULL,           -- Argon2id; never the password itself
    role           TEXT    NOT NULL DEFAULT 'Viewer',
    active         INTEGER NOT NULL DEFAULT 1,
    created_at     TEXT    NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS app_settings (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
);

-- One row per employee. crew_id holds the single crew they belong to (NULL =
-- available), so an employee can never be on two crews at once.
CREATE TABLE IF NOT EXISTS employees (
    id        INTEGER PRIMARY KEY,
    name      TEXT    NOT NULL,
    role      TEXT    NOT NULL DEFAULT '',
    phone     TEXT    NOT NULL DEFAULT '',
    active    INTEGER NOT NULL DEFAULT 1,
    notes     TEXT    NOT NULL DEFAULT '',
    crew_id   INTEGER REFERENCES crews(id) ON DELETE SET NULL,
    position  INTEGER NOT NULL DEFAULT 0,
    staff     TEXT    NOT NULL DEFAULT 'Trim',    -- staff list: 'Rough-In' or 'Trim'
    license      TEXT NOT NULL DEFAULT '',        -- v12: Apprentice / Resident / Journey
    employee_no  TEXT NOT NULL DEFAULT ''         -- v12: payroll employee ID
);
CREATE INDEX IF NOT EXISTS idx_employees_crew ON employees(crew_id);

-- Daily attendance, one row per employee per day.
CREATE TABLE IF NOT EXISTS attendance (
    employee_id  INTEGER NOT NULL REFERENCES employees(id) ON DELETE CASCADE,
    date         TEXT    NOT NULL,             -- YYYY-MM-DD
    status       TEXT    NOT NULL,             -- Present, Late, Absent, Sick, PTO, Off
    note         TEXT    NOT NULL DEFAULT '',
    updated_by   INTEGER,                      -- users.id
    updated_at   TEXT    NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (employee_id, date)
);

-- Rough-In crews are recorded per day. A day listed in roster_days has its
-- own crew list in roster (anyone without a row is on no crew that day). A
-- day that isn't listed follows the latest listed day before it; before the
-- first listed day, employees.crew_id is used.
CREATE TABLE IF NOT EXISTS roster_days (
    date  TEXT PRIMARY KEY                     -- YYYY-MM-DD
);
CREATE TABLE IF NOT EXISTS roster (
    date         TEXT    NOT NULL REFERENCES roster_days(date) ON DELETE CASCADE,
    employee_id  INTEGER NOT NULL REFERENCES employees(id) ON DELETE CASCADE,
    crew_id      INTEGER REFERENCES crews(id) ON DELETE SET NULL,
    position     INTEGER NOT NULL DEFAULT 0,
    -- 1 = out that day (PTO/Off/Absent/Sick): off the crew for this day only,
    -- but the next day's copy puts them back on it.
    out          INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (date, employee_id)
);
CREATE INDEX IF NOT EXISTS idx_roster_crew ON roster(date, crew_id);

CREATE TABLE IF NOT EXISTS jobs (
    id            INTEGER PRIMARY KEY,
    project_code  TEXT    NOT NULL DEFAULT '',
    project_name  TEXT    NOT NULL DEFAULT '',
    lot           TEXT    NOT NULL DEFAULT '',
    phase         TEXT    NOT NULL DEFAULT '',
    status        TEXT    NOT NULL DEFAULT 'Scheduled',
    crew_id       INTEGER REFERENCES crews(id) ON DELETE SET NULL,
    date          TEXT,              -- YYYY-MM-DD, NULL = unscheduled
    start_min     INTEGER NOT NULL DEFAULT 420,
    duration_min  INTEGER NOT NULL DEFAULT 240,
    notes         TEXT    NOT NULL DEFAULT '',
    comments      TEXT    NOT NULL DEFAULT '',
    board         TEXT    NOT NULL DEFAULT 'Rough-In',   -- 'Rough-In' or 'Trim'
    tank_size     TEXT    NOT NULL DEFAULT '',           -- water heater size (Trim jobs)
    kitchen_sink  TEXT    NOT NULL DEFAULT '',           -- kitchen sink (Trim jobs)
    source_id     TEXT    NOT NULL DEFAULT '',           -- v16: ID in the system it was imported from
    days          INTEGER NOT NULL DEFAULT 1,            -- days the job runs
    work_sat      INTEGER NOT NULL DEFAULT 0,            -- 1 = Saturdays count as job days
    work_sun      INTEGER NOT NULL DEFAULT 0,            -- 1 = Sundays count as job days
    created_by    INTEGER,                               -- users.id
    updated_by    INTEGER,                               -- users.id
    created_at    TEXT    NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at    TEXT    NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_jobs_date ON jobs(date);
CREATE INDEX IF NOT EXISTS idx_jobs_crew ON jobs(crew_id);

CREATE TABLE IF NOT EXISTS job_fields (
    id        INTEGER PRIMARY KEY,
    job_id    INTEGER NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    position  INTEGER NOT NULL,
    label     TEXT    NOT NULL,
    value     TEXT    NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_job_fields_job ON job_fields(job_id);

-- Company vehicles (v10).
CREATE TABLE IF NOT EXISTS vehicles (
    id                INTEGER PRIMARY KEY,
    unit              TEXT    NOT NULL DEFAULT '',   -- truck / unit number
    kind              TEXT    NOT NULL DEFAULT '',   -- Pickup, Service Van, ...
    year              TEXT    NOT NULL DEFAULT '',
    make              TEXT    NOT NULL DEFAULT '',
    model             TEXT    NOT NULL DEFAULT '',
    plate             TEXT    NOT NULL DEFAULT '',
    vin               TEXT    NOT NULL DEFAULT '',
    status            TEXT    NOT NULL DEFAULT 'Active',
    crew_id           INTEGER REFERENCES crews(id) ON DELETE SET NULL,
    driver_id         INTEGER REFERENCES employees(id) ON DELETE SET NULL,
    odometer          INTEGER NOT NULL DEFAULT 0,
    registration_due  TEXT,                          -- YYYY-MM-DD
    insurance_due     TEXT,
    next_service      TEXT,
    notes             TEXT    NOT NULL DEFAULT '',
    purchase_date     TEXT,                          -- v11
    last_oil_change   TEXT                           -- v11
);
-- Driver information (v15): one license row per employee, and their
-- accidents / moving violations / parking tickets.
CREATE TABLE IF NOT EXISTS driver_licenses (
    employee_id   INTEGER PRIMARY KEY REFERENCES employees(id) ON DELETE CASCADE,
    number        TEXT    NOT NULL DEFAULT '',
    state         TEXT    NOT NULL DEFAULT '',
    class         TEXT    NOT NULL DEFAULT '',
    status        TEXT    NOT NULL DEFAULT 'Valid',
    expires       TEXT,                            -- YYYY-MM-DD
    points        INTEGER NOT NULL DEFAULT 0,
    last_checked  TEXT,                            -- last MVR check
    endorsements  TEXT    NOT NULL DEFAULT '',
    notes         TEXT    NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS driver_incidents (
    id           INTEGER PRIMARY KEY,
    employee_id  INTEGER NOT NULL REFERENCES employees(id) ON DELETE CASCADE,
    kind         TEXT    NOT NULL,                 -- Accident / Moving Violation / Parking Ticket
    date         TEXT,
    vehicle_id   INTEGER REFERENCES vehicles(id) ON DELETE SET NULL,
    location     TEXT    NOT NULL DEFAULT '',
    description  TEXT    NOT NULL DEFAULT '',
    reference    TEXT    NOT NULL DEFAULT '',      -- ticket / citation / claim #
    amount       REAL,                             -- fine, or damage cost
    points       INTEGER NOT NULL DEFAULT 0,
    at_fault     INTEGER NOT NULL DEFAULT 0,
    status       TEXT    NOT NULL DEFAULT 'Open',
    notes        TEXT    NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_incidents_employee ON driver_incidents(employee_id);

-- Shop visits (v14): one row per repair. Opened (stamped) when a vehicle is
-- saved as In Shop; closed (returned_on set) when it's saved as anything else.
CREATE TABLE IF NOT EXISTS vehicle_repairs (
    id           INTEGER PRIMARY KEY,
    vehicle_id   INTEGER NOT NULL REFERENCES vehicles(id) ON DELETE CASCADE,
    opened_at    TEXT    NOT NULL,                 -- local 'YYYY-MM-DD HH:MM'
    opened_by    INTEGER,                          -- users.id
    shop         TEXT    NOT NULL DEFAULT '',
    work         TEXT    NOT NULL DEFAULT '',
    drop_off     TEXT,                             -- YYYY-MM-DD
    repair_time  TEXT    NOT NULL DEFAULT '',      -- estimate as typed
    est_cost     REAL,
    est_pickup   TEXT,
    returned_on  TEXT,                             -- NULL = still in the shop
    final_cost   REAL,
    notes        TEXT    NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_repairs_vehicle ON vehicle_repairs(vehicle_id);

-- Extra vehicle details (tab #, toll pass, GPS, driver, service history...),
-- one row per filled-in field; keys are listed in model::VEHICLE_FIELDS.
CREATE TABLE IF NOT EXISTS vehicle_info (
    vehicle_id  INTEGER NOT NULL REFERENCES vehicles(id) ON DELETE CASCADE,
    key         TEXT    NOT NULL,
    value       TEXT    NOT NULL,
    PRIMARY KEY (vehicle_id, key)
);
"#;

pub struct Db {
    conn: Connection,
    /// None when running on a temporary in-memory database.
    pub path: Option<PathBuf>,
}

fn data_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("PlumbingScheduler"))
}

fn date_str(d: Option<NaiveDate>) -> Option<String> {
    d.map(|d| d.format("%Y-%m-%d").to_string())
}

/// An error carrying a plain-language message for the status bar.
fn constraint_error(msg: &str) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT), Some(msg.to_string()))
}

/// Rough-In crew assignments for one day: employee id → (crew id, position).
/// People not listed are on no crew that day.
pub type Roster = HashMap<i64, (i64, i64)>;

fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

/// The latest day on or before `d` whose Rough-In crews were recorded.
fn roster_source_c(conn: &Connection, d: NaiveDate) -> Result<Option<NaiveDate>> {
    let s: Option<String> =
        conn.query_row("SELECT MAX(date) FROM roster_days WHERE date <= ?1", [date_str(Some(d))], |r| r.get(0))?;
    Ok(s.as_deref().and_then(parse_date))
}

/// Rough-In crews in effect on `d` (recorded that day, copied forward from the
/// latest recorded day before it, or the original crew list). People marked
/// out on `d` itself are left off; out on an earlier day doesn't carry over.
fn load_roster_c(conn: &Connection, d: NaiveDate) -> Result<Roster> {
    load_roster_inner(conn, d, false)
}

fn load_roster_inner(conn: &Connection, d: NaiveDate, include_out: bool) -> Result<Roster> {
    let ri = Board::RoughIn.as_str();
    let mut out = Roster::new();
    let mut collect = |sql: &str, p: &[&dyn rusqlite::ToSql]| -> Result<()> {
        let mut st = conn.prepare(sql)?;
        let rows = st.query_map(p, |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))?;
        for row in rows {
            let (emp, crew, pos) = row?;
            out.insert(emp, (crew, pos));
        }
        Ok(())
    };
    match roster_source_c(conn, d)? {
        Some(src) => {
            // Out-that-day rows only count on that very day.
            let skip_out = !include_out && src == d;
            collect(
                "SELECT r.employee_id, r.crew_id, r.position FROM roster r
                   JOIN employees e ON e.id = r.employee_id
                   JOIN crews c ON c.id = r.crew_id
                  WHERE r.date = ?1 AND e.staff = ?2 AND c.board = ?2 AND (r.out = 0 OR ?3 = 0)",
                &[&date_str(Some(src)), &ri, &skip_out],
            )?
        }
        None => collect(
            "SELECT e.id, e.crew_id, e.position FROM employees e
               JOIN crews c ON c.id = e.crew_id
              WHERE e.staff = ?1 AND c.board = ?1",
            &[&ri],
        )?,
    }
    Ok(out)
}

/// Gives `d` its own crew list (a copy of what was in effect) if it doesn't
/// have one yet, so a change to `d` touches no other day.
fn ensure_roster_day_c(conn: &Connection, d: NaiveDate) -> Result<()> {
    let ds = date_str(Some(d));
    if conn.prepare("SELECT 1 FROM roster_days WHERE date = ?1")?.exists([&ds])? {
        return Ok(());
    }
    // Everyone's usual crew, including anyone who was out on the day copied.
    let current = load_roster_inner(conn, d, true)?;
    conn.execute("INSERT INTO roster_days (date) VALUES (?1)", [&ds])?;
    let mut st = conn.prepare("INSERT INTO roster (date, employee_id, crew_id, position) VALUES (?1, ?2, ?3, ?4)")?;
    for (emp, (crew, pos)) in current {
        st.execute(params![ds, emp, crew, pos])?;
    }
    Ok(())
}

fn employee_name_c(conn: &Connection, id: i64) -> String {
    conn.query_row("SELECT name FROM employees WHERE id=?1", [id], |r| r.get(0))
        .unwrap_or_else(|_| format!("Employee #{id}"))
}

/// Sets a Rough-In crew's members for one day only.
fn set_roster_members_c(conn: &Connection, d: NaiveDate, crew: i64, members: &[i64]) -> Result<()> {
    ensure_roster_day_c(conn, d)?;
    let ds = date_str(Some(d));
    // People out that day keep their link to the crew.
    conn.execute("DELETE FROM roster WHERE date = ?1 AND crew_id = ?2 AND out = 0", params![ds, crew])?;
    for (i, emp) in members.iter().enumerate() {
        let other: Option<(Option<i64>, bool)> = conn
            .query_row(
                "SELECT crew_id, out FROM roster WHERE date = ?1 AND employee_id = ?2",
                params![ds, emp],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if matches!(other, Some((Some(_), false))) {
            return Err(constraint_error(&format!(
                "{} is already on another crew on {}",
                employee_name_c(conn, *emp),
                d.format("%a %b %-d")
            )));
        }
        let rough = conn
            .prepare("SELECT 1 FROM employees WHERE id = ?1 AND staff = ?2")?
            .exists(params![emp, Board::RoughIn.as_str()])?;
        if !rough {
            return Err(constraint_error(&format!(
                "{} isn't on the Rough-In staff list (or was deleted)",
                employee_name_c(conn, *emp)
            )));
        }
        conn.execute(
            "INSERT INTO roster (date, employee_id, crew_id, position, out) VALUES (?1, ?2, ?3, ?4, 0)
             ON CONFLICT(date, employee_id) DO UPDATE SET crew_id = excluded.crew_id, position = excluded.position, out = 0",
            params![ds, emp, crew, i as i64],
        )?;
    }
    Ok(())
}

/// Inserts or updates a crew's own details (not its members).
fn upsert_crew_c(conn: &Connection, crew: &Crew) -> Result<i64> {
    let p = params![
        crew.name.trim(),
        crew.foreman.trim(),
        crew.phone.trim(),
        rgb_to_int(crew.color),
        crew.active,
        crew.notes,
        crew.board.as_str(),
        crew.vehicle_id,
        crew.id
    ];
    if crew.id > 0 {
        conn.execute(
            "UPDATE crews SET name=?1, foreman=?2, phone=?3, color=?4, active=?5, notes=?6, board=?7, vehicle_id=?8 WHERE id=?9",
            p,
        )?;
        Ok(crew.id)
    } else {
        conn.execute(
            "INSERT INTO crews (name, foreman, phone, color, active, notes, board, vehicle_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            &p[..8],
        )?;
        Ok(conn.last_insert_rowid())
    }
}

fn rgb_to_int(c: [u8; 3]) -> i64 {
    ((c[0] as i64) << 16) | ((c[1] as i64) << 8) | c[2] as i64
}

fn int_to_rgb(v: i64) -> [u8; 3] {
    [((v >> 16) & 0xFF) as u8, ((v >> 8) & 0xFF) as u8, (v & 0xFF) as u8]
}

impl Db {
    /// Opens %APPDATA%\PlumbingScheduler\schedule.db. Falls back to an
    /// in-memory database (with a warning) if that fails.
    pub fn open_default() -> (Db, Option<String>) {
        // Optional override, e.g. to keep a test database separate.
        if let Some(p) = std::env::var_os("PLUMBING_SCHEDULER_DB") {
            let path = PathBuf::from(p);
            match Connection::open(&path).and_then(|c| Db::init(c, Some(path.clone()))) {
                Ok(db) => return (db, None),
                Err(e) => eprintln!("PLUMBING_SCHEDULER_DB {}: {e}", path.display()),
            }
        }
        let warning = match data_dir() {
            Some(dir) => match std::fs::create_dir_all(&dir) {
                Ok(()) => {
                    let path = dir.join("schedule.db");
                    match Connection::open(&path).and_then(|c| Db::init(c, Some(path.clone()))) {
                        Ok(db) => return (db, None),
                        Err(e) => format!("Could not open {}: {e}", path.display()),
                    }
                }
                Err(e) => format!("Could not create {}: {e}", dir.display()),
            },
            None => "APPDATA is not set".to_string(),
        };
        let db = Connection::open_in_memory()
            .and_then(|c| Db::init(c, None))
            .expect("in-memory SQLite database");
        (
            db,
            Some(format!(
                "{warning} — using a temporary database; changes will NOT be saved."
            )),
        )
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Db> {
        Db::init(Connection::open_in_memory()?, None)
    }

    fn init(conn: Connection, path: Option<PathBuf>) -> Result<Db> {
        conn.execute_batch(SCHEMA)?;
        Self::migrate(&conn)?;
        let db = Db { conn, path };
        db.open_missing_repairs()?;
        Ok(db)
    }

    /// v14: a vehicle already In Shop from before repairs were tracked gets
    /// its open repair now (stamped with its drop-off date when it has one).
    fn open_missing_repairs(&self) -> Result<()> {
        let missing: Vec<i64> = self
            .conn
            .prepare(
                "SELECT id FROM vehicles WHERE status='In Shop'
                   AND NOT EXISTS (SELECT 1 FROM vehicle_repairs r WHERE r.vehicle_id=vehicles.id AND r.returned_on IS NULL)",
            )?
            .query_map([], |r| r.get(0))?
            .collect::<Result<_>>()?;
        if missing.is_empty() {
            return Ok(());
        }
        let now = chrono::Local::now().naive_local();
        for v in self.load_vehicles()?.into_iter().filter(|v| missing.contains(&v.id)) {
            let stamp = v.info_date(SHOP_DROP_OFF).map(|d| d.and_hms_opt(0, 0, 0).unwrap()).unwrap_or(now);
            self.sync_repair(v.id, &v, None, stamp)?;
        }
        Ok(())
    }

    /// Upgrades databases created by older versions of the app.
    fn migrate(conn: &Connection) -> Result<()> {
        let has_board = conn
            .prepare("SELECT 1 FROM pragma_table_info('jobs') WHERE name = 'board'")?
            .exists([])?;
        if !has_board {
            // v2: separate Rough-In and Trim schedules. Sort existing jobs by phase.
            conn.execute_batch(
                "ALTER TABLE jobs ADD COLUMN board TEXT NOT NULL DEFAULT 'Rough-In';
                 UPDATE jobs SET board = 'Trim'
                   WHERE phase LIKE '%trim%' OR phase LIKE '%final%'
                      OR phase LIKE '%service%' OR phase LIKE '%warranty%';",
            )?;
        }
        conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_jobs_board ON jobs(board);")?;

        // v4: remember who created / last changed each job.
        for col in ["created_by", "updated_by"] {
            let exists = conn
                .prepare("SELECT 1 FROM pragma_table_info('jobs') WHERE name = ?1")?
                .exists([col])?;
            if !exists {
                conn.execute_batch(&format!("ALTER TABLE jobs ADD COLUMN {col} INTEGER;"))?;
            }
        }

        // v8: multi-day jobs (number of work days).
        let has_days = conn.prepare("SELECT 1 FROM pragma_table_info('jobs') WHERE name = 'days'")?.exists([])?;
        if !has_days {
            conn.execute_batch("ALTER TABLE jobs ADD COLUMN days INTEGER NOT NULL DEFAULT 1;")?;
        }
        // v9: optional Saturday / Sunday work on multi-day jobs.
        for col in ["work_sat", "work_sun"] {
            let exists = conn
                .prepare("SELECT 1 FROM pragma_table_info('jobs') WHERE name = ?1")?
                .exists([col])?;
            if !exists {
                conn.execute_batch(&format!("ALTER TABLE jobs ADD COLUMN {col} INTEGER NOT NULL DEFAULT 0;"))?;
            }
        }

        // v13: a crew's vehicle lives on the crew (two crews can share a
        // truck). Carry over any crew picked on a vehicle record, then clear
        // that old link so there's only one place it's kept.
        let has_vehicle = conn
            .prepare("SELECT 1 FROM pragma_table_info('crews') WHERE name = 'vehicle_id'")?
            .exists([])?;
        if !has_vehicle {
            conn.execute_batch(
                "ALTER TABLE crews ADD COLUMN vehicle_id INTEGER REFERENCES vehicles(id) ON DELETE SET NULL;",
            )?;
        }
        let has_vehicles_table = conn
            .prepare("SELECT 1 FROM pragma_table_info('vehicles') WHERE name = 'crew_id'")?
            .exists([])?;
        if has_vehicles_table {
            conn.execute_batch(
                "UPDATE crews SET vehicle_id = (SELECT v.id FROM vehicles v WHERE v.crew_id = crews.id ORDER BY v.id LIMIT 1)
                   WHERE vehicle_id IS NULL AND EXISTS (SELECT 1 FROM vehicles v WHERE v.crew_id = crews.id);
                 UPDATE vehicles SET crew_id = NULL WHERE crew_id IS NOT NULL;",
            )?;
        }

        // v12: employee license level and payroll employee ID.
        for col in ["license", "employee_no"] {
            let exists = conn
                .prepare("SELECT 1 FROM pragma_table_info('employees') WHERE name = ?1")?
                .exists([col])?;
            if !exists {
                conn.execute_batch(&format!("ALTER TABLE employees ADD COLUMN {col} TEXT NOT NULL DEFAULT '';"))?;
            }
        }

        // v11: vehicle purchase date and last oil change.
        for col in ["purchase_date", "last_oil_change"] {
            let exists = conn
                .prepare("SELECT 1 FROM pragma_table_info('vehicles') WHERE name = ?1")?
                .exists([col])?;
            if !exists {
                conn.execute_batch(&format!("ALTER TABLE vehicles ADD COLUMN {col} TEXT;"))?;
            }
        }

        // v5: water tank size on Trim jobs. v6: kitchen sink on Trim jobs.
        // v16: imported jobs remember their ID in the source system.
        let has_source = conn.prepare("SELECT 1 FROM pragma_table_info('jobs') WHERE name = 'source_id'")?.exists([])?;
        if !has_source {
            conn.execute_batch("ALTER TABLE jobs ADD COLUMN source_id TEXT NOT NULL DEFAULT '';")?;
        }
        conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_jobs_source ON jobs(source_id);")?;

        for col in ["tank_size", "kitchen_sink"] {
            let exists = conn
                .prepare("SELECT 1 FROM pragma_table_info('jobs') WHERE name = ?1")?
                .exists([col])?;
            if !exists {
                conn.execute_batch(&format!("ALTER TABLE jobs ADD COLUMN {col} TEXT NOT NULL DEFAULT '';"))?;
            }
        }

        // v7: separate Rough-In and Trim staff. Everything that exists today
        // was managed under "Trim Staff", so it starts out as Trim.
        for (table, col) in [("crews", "board"), ("employees", "staff")] {
            let exists = conn
                .prepare(&format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1"))?
                .exists([col])?;
            if !exists {
                conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} TEXT NOT NULL DEFAULT 'Trim';"))?;
            }
        }

        // v3: crew members become employees on a shared employee list.
        let has_old_members = conn
            .prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'crew_members'")?
            .exists([])?;
        if has_old_members {
            conn.execute_batch(
                "BEGIN;
                 INSERT INTO employees (name, role, phone, crew_id, position)
                   SELECT name, role, phone, crew_id, position FROM crew_members
                   WHERE crew_id IN (SELECT id FROM crews) ORDER BY crew_id, position;
                 DROP TABLE crew_members;
                 COMMIT;",
            )?;
        }
        Ok(())
    }

    // ---------------------------------------------------------------- users

    pub fn load_users(&self) -> Result<Vec<User>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, username, role, active FROM users ORDER BY name COLLATE NOCASE, id")?;
        let rows = stmt.query_map([], |r| {
            let role: String = r.get(3)?;
            Ok(User {
                id: r.get(0)?,
                name: r.get(1)?,
                username: r.get(2)?,
                role: Role::from_db(&role),
                active: r.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// Looks up a user by username (case-insensitive) for signing in.
    /// Returns the user and their stored password hash.
    pub fn find_login(&self, username: &str) -> Result<Option<(User, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, username, role, active, password_hash FROM users WHERE username = ?1",
        )?;
        let mut rows = stmt.query([username.trim()])?;
        match rows.next()? {
            Some(r) => {
                let role: String = r.get(3)?;
                Ok(Some((
                    User { id: r.get(0)?, name: r.get(1)?, username: r.get(2)?, role: Role::from_db(&role), active: r.get(4)? },
                    r.get(5)?,
                )))
            }
            None => Ok(None),
        }
    }

    pub fn create_user(&self, u: &User, password_hash: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO users (name, username, password_hash, role, active) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![u.name.trim(), u.username.trim(), password_hash, u.role.as_str(), u.active],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_user(&self, u: &User) -> Result<()> {
        self.conn.execute(
            "UPDATE users SET name=?2, username=?3, role=?4, active=?5 WHERE id=?1",
            params![u.id, u.name.trim(), u.username.trim(), u.role.as_str(), u.active],
        )?;
        Ok(())
    }

    pub fn set_password(&self, user_id: i64, password_hash: &str) -> Result<()> {
        self.conn.execute("UPDATE users SET password_hash=?2 WHERE id=?1", params![user_id, password_hash])?;
        Ok(())
    }

    /// Users whose password was reset (cleared) and who must choose a new
    /// one at their next sign-in.
    pub fn pending_resets(&self) -> Result<std::collections::HashSet<i64>> {
        let mut stmt = self.conn.prepare("SELECT id FROM users WHERE password_hash = ''")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect()
    }

    pub fn get_setting(&self, key: &str) -> Option<String> {
        self.conn
            .query_row("SELECT value FROM app_settings WHERE key=?1", [key], |r| r.get(0))
            .ok()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Created/changed-by info for every job.
    pub fn load_audit(&self) -> Result<HashMap<i64, JobAudit>> {
        let mut stmt = self.conn.prepare("SELECT id, created_by, created_at, updated_by, updated_at FROM jobs")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                JobAudit { created_by: r.get(1)?, created_at: r.get(2)?, updated_by: r.get(3)?, updated_at: r.get(4)? },
            ))
        })?;
        rows.collect()
    }

    // ------------------------------------------------------------ employees

    pub fn load_employees(&self) -> Result<Vec<Employee>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, role, phone, active, notes, crew_id, staff, license, employee_no
             FROM employees ORDER BY name COLLATE NOCASE, id",
        )?;
        let rows = stmt.query_map([], |r| {
            let staff: String = r.get(7)?;
            Ok(Employee {
                id: r.get(0)?,
                name: r.get(1)?,
                role: r.get(2)?,
                phone: r.get(3)?,
                active: r.get(4)?,
                notes: r.get(5)?,
                crew_id: r.get(6)?,
                staff: Board::from_db(&staff),
                license: r.get(8)?,
                employee_no: r.get(9)?,
            })
        })?;
        rows.collect()
    }

    /// Saves several employees at once (an import); if any fails, none are saved.
    pub fn save_employees(&mut self, list: &[Employee]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for e in list {
            self.save_employee(e)?;
        }
        tx.commit()
    }

    /// Saves an employee's details and staff list. Crew membership is changed
    /// through `save_crew` / `move_employee_to_crew` / `release_employee`,
    /// except that switching someone to the other staff list takes them off
    /// a crew from their old list.
    pub fn save_employee(&self, e: &Employee) -> Result<i64> {
        if e.id > 0 {
            self.conn.execute(
                "UPDATE employees
                    SET name=?2, role=?3, phone=?4, active=?5, notes=?6, staff=?7, license=?8, employee_no=?9,
                        crew_id = CASE WHEN (SELECT board FROM crews WHERE id = employees.crew_id) = ?7
                                       THEN crew_id ELSE NULL END
                  WHERE id=?1",
                params![
                    e.id, e.name.trim(), e.role.trim(), e.phone.trim(), e.active, e.notes, e.staff.as_str(),
                    e.license.trim(), e.employee_no.trim()
                ],
            )?;
            Ok(e.id)
        } else {
            self.conn.execute(
                "INSERT INTO employees (name, role, phone, active, notes, staff, license, employee_no)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    e.name.trim(), e.role.trim(), e.phone.trim(), e.active, e.notes, e.staff.as_str(),
                    e.license.trim(), e.employee_no.trim()
                ],
            )?;
            Ok(self.conn.last_insert_rowid())
        }
    }

    /// Moves an employee straight onto a crew (off whatever crew they were
    /// on), placing them last in that crew's member list. Because each
    /// employee has exactly one crew_id, this can never double-assign.
    /// Refused if the crew is in the other staff group.
    pub fn move_employee_to_crew(&self, id: i64, crew_id: i64) -> Result<()> {
        let n = self.conn.execute(
            "UPDATE employees
                SET crew_id = ?2,
                    position = (SELECT COALESCE(MAX(position), -1) + 1 FROM employees WHERE crew_id = ?2 AND id <> ?1)
              WHERE id = ?1 AND staff = (SELECT board FROM crews WHERE id = ?2)",
            params![id, crew_id],
        )?;
        if n == 0 {
            return Err(constraint_error("That crew isn't in this employee's staff group (or no longer exists)."));
        }
        Ok(())
    }

    /// Removes an employee from whatever crew they are on.
    pub fn release_employee(&self, id: i64) -> Result<()> {
        self.conn.execute("UPDATE employees SET crew_id=NULL, position=0 WHERE id=?1", [id])?;
        Ok(())
    }

    pub fn delete_employee(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM employees WHERE id=?1", [id])?;
        Ok(())
    }

    // ---------------------------------------------------------------- vehicles

    pub fn load_vehicles(&self) -> Result<Vec<Vehicle>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, unit, kind, year, make, model, plate, vin, status, crew_id, driver_id,
                    odometer, registration_due, insurance_due, next_service, notes,
                    purchase_date, last_oil_change
             FROM vehicles ORDER BY unit COLLATE NOCASE, id",
        )?;
        let rows = stmt.query_map([], |r| {
            let date = |i: usize| -> Result<Option<NaiveDate>> {
                Ok(r.get::<_, Option<String>>(i)?.as_deref().and_then(parse_date))
            };
            Ok(Vehicle {
                id: r.get(0)?,
                unit: r.get(1)?,
                kind: r.get(2)?,
                year: r.get(3)?,
                make: r.get(4)?,
                model: r.get(5)?,
                plate: r.get(6)?,
                vin: r.get(7)?,
                status: r.get(8)?,
                crew_id: r.get(9)?,
                driver_id: r.get(10)?,
                odometer: r.get(11)?,
                registration_due: date(12)?,
                insurance_due: date(13)?,
                next_service: date(14)?,
                notes: r.get(15)?,
                purchase_date: date(16)?,
                last_oil_change: date(17)?,
                info: Default::default(),
            })
        })?;
        let mut vehicles = rows.collect::<Result<Vec<_>>>()?;
        let index: HashMap<i64, usize> = vehicles.iter().enumerate().map(|(i, v)| (v.id, i)).collect();
        let mut st = self.conn.prepare("SELECT vehicle_id, key, value FROM vehicle_info")?;
        let info = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        for row in info {
            let (id, k, v) = row?;
            if let Some(&i) = index.get(&id) {
                vehicles[i].info.insert(k, v);
            }
        }
        vehicles.sort_by_key(|v| v.sort_key());
        Ok(vehicles)
    }

    fn save_vehicle_info(&self, id: i64, v: &Vehicle) -> Result<()> {
        self.conn.execute("DELETE FROM vehicle_info WHERE vehicle_id=?1", [id])?;
        let mut st = self.conn.prepare("INSERT INTO vehicle_info (vehicle_id, key, value) VALUES (?1, ?2, ?3)")?;
        for (k, val) in &v.info {
            let val = val.trim();
            if !val.is_empty() {
                st.execute(params![id, k, val])?;
            }
        }
        Ok(())
    }

    pub fn save_vehicle(&self, v: &Vehicle) -> Result<i64> {
        self.save_vehicle_by(v, None)
    }

    /// Saves a vehicle; `user` is who's saving (stamped on a new repair).
    pub fn save_vehicle_by(&self, v: &Vehicle, user: Option<i64>) -> Result<i64> {
        let tx = self.conn.unchecked_transaction()?;
        let id = self.save_vehicle_inner(v)?;
        self.sync_repair(id, v, user, chrono::Local::now().naive_local())?;
        tx.commit()?;
        Ok(id)
    }

    /// Keeps the repair history in step with the vehicle's status:
    /// - In Shop: the open repair takes the Shop fields (one is opened and
    ///   stamped with `now` if there isn't one).
    /// - Anything else: an open repair is closed (returned today, with the
    ///   latest Shop fields), and the Shop fields are cleared for next time.
    fn sync_repair(&self, id: i64, v: &Vehicle, user: Option<i64>, now: chrono::NaiveDateTime) -> Result<()> {
        let open: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM vehicle_repairs WHERE vehicle_id=?1 AND returned_on IS NULL ORDER BY id DESC LIMIT 1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        let visit = v.shop_visit();
        let in_shop = v.status == "In Shop";
        if !in_shop && open.is_none() {
            return Ok(());
        }
        let rid = match open {
            Some(rid) => rid,
            None => {
                self.conn.execute(
                    "INSERT INTO vehicle_repairs (vehicle_id, opened_at, opened_by) VALUES (?1, ?2, ?3)",
                    params![id, now.format("%Y-%m-%d %H:%M").to_string(), user],
                )?;
                self.conn.last_insert_rowid()
            }
        };
        self.conn.execute(
            "UPDATE vehicle_repairs SET shop=?2, work=?3, drop_off=?4, repair_time=?5, est_cost=?6, est_pickup=?7,
                    final_cost=?8, returned_on=?9 WHERE id=?1",
            params![
                rid,
                visit.shop,
                visit.work,
                date_str(visit.drop_off),
                visit.repair_time,
                visit.est_cost,
                date_str(visit.est_pickup),
                visit.final_cost,
                if in_shop { None } else { Some(now.date().format("%Y-%m-%d").to_string()) },
            ],
        )?;
        if !in_shop {
            for k in SHOP_KEYS {
                self.conn.execute("DELETE FROM vehicle_info WHERE vehicle_id=?1 AND key=?2", params![id, k])?;
            }
        }
        Ok(())
    }

    pub fn load_repairs(&self) -> Result<Vec<Repair>> {
        let mut st = self.conn.prepare(
            "SELECT id, vehicle_id, opened_at, opened_by, shop, work, drop_off, repair_time, est_cost, est_pickup,
                    returned_on, final_cost, notes
             FROM vehicle_repairs ORDER BY opened_at DESC, id DESC",
        )?;
        let rows = st.query_map([], |r| {
            let date = |i: usize| -> Result<Option<NaiveDate>> { Ok(r.get::<_, Option<String>>(i)?.as_deref().and_then(parse_date)) };
            Ok(Repair {
                id: r.get(0)?,
                vehicle_id: r.get(1)?,
                opened_at: r.get(2)?,
                opened_by: r.get(3)?,
                shop: r.get(4)?,
                work: r.get(5)?,
                drop_off: date(6)?,
                repair_time: r.get(7)?,
                est_cost: r.get(8)?,
                est_pickup: date(9)?,
                returned_on: date(10)?,
                final_cost: r.get(11)?,
                notes: r.get(12)?,
            })
        })?;
        rows.collect()
    }

    /// Saves a repair edited in the history (or a past one added by hand).
    pub fn save_repair(&self, r: &Repair) -> Result<i64> {
        let p = params![
            r.vehicle_id,
            r.opened_at,
            r.opened_by,
            r.shop.trim(),
            r.work.trim(),
            date_str(r.drop_off),
            r.repair_time.trim(),
            r.est_cost,
            date_str(r.est_pickup),
            date_str(r.returned_on),
            r.final_cost,
            r.notes,
            r.id
        ];
        if r.id > 0 {
            self.conn.execute(
                "UPDATE vehicle_repairs SET vehicle_id=?1, opened_at=?2, opened_by=?3, shop=?4, work=?5, drop_off=?6,
                        repair_time=?7, est_cost=?8, est_pickup=?9, returned_on=?10, final_cost=?11, notes=?12
                  WHERE id=?13",
                p,
            )?;
            Ok(r.id)
        } else {
            self.conn.execute(
                "INSERT INTO vehicle_repairs (vehicle_id, opened_at, opened_by, shop, work, drop_off, repair_time,
                        est_cost, est_pickup, returned_on, final_cost, notes)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                &p[..12],
            )?;
            Ok(self.conn.last_insert_rowid())
        }
    }

    // ---------------------------------------------------------------- drivers

    pub fn load_licenses(&self) -> Result<HashMap<i64, DriverLicense>> {
        let mut st = self.conn.prepare(
            "SELECT employee_id, number, state, class, status, expires, points, last_checked, endorsements, notes
             FROM driver_licenses",
        )?;
        let rows = st.query_map([], |r| {
            let date = |i: usize| -> Result<Option<NaiveDate>> { Ok(r.get::<_, Option<String>>(i)?.as_deref().and_then(parse_date)) };
            Ok(DriverLicense {
                employee_id: r.get(0)?,
                number: r.get(1)?,
                state: r.get(2)?,
                class: r.get(3)?,
                status: r.get(4)?,
                expires: date(5)?,
                points: r.get(6)?,
                last_checked: date(7)?,
                endorsements: r.get(8)?,
                notes: r.get(9)?,
            })
        })?;
        rows.map(|r| r.map(|l| (l.employee_id, l))).collect()
    }

    pub fn save_license(&self, l: &DriverLicense) -> Result<()> {
        self.conn.execute(
            "INSERT INTO driver_licenses (employee_id, number, state, class, status, expires, points, last_checked, endorsements, notes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(employee_id) DO UPDATE SET number=?2, state=?3, class=?4, status=?5, expires=?6, points=?7,
                    last_checked=?8, endorsements=?9, notes=?10",
            params![
                l.employee_id,
                l.number.trim(),
                l.state.trim().to_uppercase(),
                l.class.trim(),
                l.status,
                date_str(l.expires),
                l.points.max(0),
                date_str(l.last_checked),
                l.endorsements.trim(),
                l.notes
            ],
        )?;
        Ok(())
    }

    pub fn load_incidents(&self) -> Result<Vec<Incident>> {
        let mut st = self.conn.prepare(
            "SELECT id, employee_id, kind, date, vehicle_id, location, description, reference, amount, points, at_fault,
                    status, notes
             FROM driver_incidents ORDER BY date DESC, id DESC",
        )?;
        let rows = st.query_map([], |r| {
            Ok(Incident {
                id: r.get(0)?,
                employee_id: r.get(1)?,
                kind: r.get(2)?,
                date: r.get::<_, Option<String>>(3)?.as_deref().and_then(parse_date),
                vehicle_id: r.get(4)?,
                location: r.get(5)?,
                description: r.get(6)?,
                reference: r.get(7)?,
                amount: r.get(8)?,
                points: r.get(9)?,
                at_fault: r.get(10)?,
                status: r.get(11)?,
                notes: r.get(12)?,
            })
        })?;
        rows.collect()
    }

    pub fn save_incident(&self, i: &Incident) -> Result<i64> {
        let p = params![
            i.employee_id,
            i.kind,
            date_str(i.date),
            i.vehicle_id,
            i.location.trim(),
            i.description.trim(),
            i.reference.trim(),
            i.amount,
            i.points.max(0),
            i.at_fault,
            i.status,
            i.notes,
            i.id
        ];
        if i.id > 0 {
            self.conn.execute(
                "UPDATE driver_incidents SET employee_id=?1, kind=?2, date=?3, vehicle_id=?4, location=?5, description=?6,
                        reference=?7, amount=?8, points=?9, at_fault=?10, status=?11, notes=?12 WHERE id=?13",
                p,
            )?;
            Ok(i.id)
        } else {
            self.conn.execute(
                "INSERT INTO driver_incidents (employee_id, kind, date, vehicle_id, location, description, reference, amount,
                        points, at_fault, status, notes)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                &p[..12],
            )?;
            Ok(self.conn.last_insert_rowid())
        }
    }

    pub fn delete_incident(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM driver_incidents WHERE id=?1", [id])?;
        Ok(())
    }

    pub fn delete_repair(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM vehicle_repairs WHERE id=?1", [id])?;
        Ok(())
    }

    /// Saves several vehicles at once; if any fails, none are saved.
    pub fn save_vehicles(&mut self, vehicles: &[Vehicle]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let now = chrono::Local::now().naive_local();
        for v in vehicles {
            let id = self.save_vehicle_inner(v)?;
            self.sync_repair(id, v, None, now)?;
        }
        tx.commit()
    }

    fn save_vehicle_inner(&self, v: &Vehicle) -> Result<i64> {
        let vin = v.vin.trim().to_uppercase();
        let (reg, ins, svc) = (date_str(v.registration_due), date_str(v.insurance_due), date_str(v.next_service));
        let id = if v.id > 0 {
            v.id
        } else {
            self.conn.execute("INSERT INTO vehicles (unit) VALUES ('')", [])?;
            self.conn.last_insert_rowid()
        };
        self.conn.execute(
            "UPDATE vehicles SET unit=?1, kind=?2, year=?3, make=?4, model=?5, plate=?6, vin=?7, status=?8,
                    crew_id=?9, driver_id=?10, odometer=?11, registration_due=?12, insurance_due=?13,
                    next_service=?14, notes=?15, purchase_date=?16, last_oil_change=?17
              WHERE id=?18",
            params![
                v.unit.trim(), v.kind.trim(), v.year.trim(), v.make.trim(), v.model.trim(), v.plate.trim(),
                vin, v.status, v.crew_id, v.driver_id, v.odometer.max(0), reg, ins, svc, v.notes,
                date_str(v.purchase_date), date_str(v.last_oil_change), id
            ],
        )?;
        self.save_vehicle_info(id, v)?;
        Ok(id)
    }

    pub fn delete_vehicle(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM vehicles WHERE id=?1", [id])?;
        Ok(())
    }

    // ---------------------------------------------------------------- crews

    pub fn load_crews(&self) -> Result<Vec<Crew>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, foreman, phone, color, active, notes, board, vehicle_id
             FROM crews ORDER BY name COLLATE NOCASE",
        )?;
        let mut crews = stmt
            .query_map([], |r| {
                let board: String = r.get(7)?;
                Ok(Crew {
                    board: Board::from_db(&board),
                    id: r.get(0)?,
                    name: r.get(1)?,
                    foreman: r.get(2)?,
                    phone: r.get(3)?,
                    color: int_to_rgb(r.get(4)?),
                    active: r.get(5)?,
                    notes: r.get(6)?,
                    member_ids: Vec::new(),
                    vehicle_id: r.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>>>()?;

        let index: HashMap<i64, usize> =
            crews.iter().enumerate().map(|(i, c)| (c.id, i)).collect();
        let mut mstmt = self.conn.prepare(
            "SELECT crew_id, id FROM employees WHERE crew_id IS NOT NULL ORDER BY crew_id, position, id",
        )?;
        let rows = mstmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
        for row in rows {
            let (crew_id, emp_id) = row?;
            if let Some(&i) = index.get(&crew_id) {
                crews[i].member_ids.push(emp_id);
            }
        }
        Ok(crews)
    }

    /// Saves the crew and sets its members. Fails (and changes nothing) if any
    /// listed employee already belongs to a different crew or is on the other
    /// staff list. Moving a crew to the other staff group brings its current
    /// members along to that group's employee list.
    pub fn save_crew(&mut self, crew: &Crew) -> Result<i64> {
        let tx = self.conn.transaction()?;
        let board = crew.board.as_str();
        let id = upsert_crew_c(&tx, crew)?;
        // Members that stay on the crew follow it to its staff group.
        for emp_id in &crew.member_ids {
            tx.execute("UPDATE employees SET staff=?2 WHERE id=?1 AND crew_id=?3", params![emp_id, board, id])?;
        }
        tx.execute("UPDATE employees SET crew_id=NULL, position=0 WHERE crew_id=?1", [id])?;
        {
            let mut st = tx.prepare(
                "UPDATE employees SET crew_id=?1, position=?2 WHERE id=?3 AND crew_id IS NULL AND staff=?4",
            )?;
            for (i, emp_id) in crew.member_ids.iter().enumerate() {
                if st.execute(params![id, i as i64, emp_id, board])? == 0 {
                    let who: String = tx
                        .query_row("SELECT name FROM employees WHERE id=?1", [emp_id], |r| r.get(0))
                        .unwrap_or_else(|_| format!("Employee #{emp_id}"));
                    return Err(constraint_error(&format!(
                        "{who} is already on another crew, is on the other staff list, or was deleted"
                    )));
                }
            }
        }
        tx.commit()?;
        Ok(id)
    }

    /// Saves a crew. Rough-In crews get their members set for `day` only;
    /// Trim crews keep one standing member list (`save_crew`).
    pub fn save_crew_on(&mut self, crew: &Crew, day: NaiveDate) -> Result<i64> {
        if crew.board != Board::RoughIn {
            return self.save_crew(crew);
        }
        let tx = self.conn.transaction()?;
        let id = upsert_crew_c(&tx, crew)?;
        // A crew coming over from Trim brings its standing members along.
        for emp_id in &crew.member_ids {
            tx.execute(
                "UPDATE employees SET staff=?2 WHERE id=?1 AND crew_id=?3",
                params![emp_id, Board::RoughIn.as_str(), id],
            )?;
        }
        set_roster_members_c(&tx, day, id, &crew.member_ids)?;
        tx.commit()?;
        Ok(id)
    }

    // ---------------------------------------------------- daily crews (Rough-In)

    /// Rough-In crew assignments in effect on `d`.
    pub fn load_roster(&self, d: NaiveDate) -> Result<Roster> {
        load_roster_c(&self.conn, d)
    }

    /// The day whose crew list `d` uses: `d` itself once it has been changed,
    /// else the latest changed day before it (None = the original crews).
    pub fn roster_source(&self, d: NaiveDate) -> Result<Option<NaiveDate>> {
        roster_source_c(&self.conn, d)
    }

    /// Puts a Rough-In employee on a crew (None = no crew) for one day only.
    pub fn set_roster_crew(&mut self, d: NaiveDate, emp: i64, crew: Option<i64>) -> Result<()> {
        let tx = self.conn.transaction()?;
        ensure_roster_day_c(&tx, d)?;
        let ds = date_str(Some(d));
        match crew {
            Some(c) => {
                let ok = tx
                    .prepare("SELECT 1 FROM employees e, crews c WHERE e.id = ?1 AND c.id = ?2 AND e.staff = ?3 AND c.board = ?3")?
                    .exists(params![emp, c, Board::RoughIn.as_str()])?;
                if !ok {
                    return Err(constraint_error("That crew isn't a Rough-In crew (or no longer exists)."));
                }
                let current: Option<(Option<i64>, bool)> = tx
                    .query_row(
                        "SELECT crew_id, out FROM roster WHERE date = ?1 AND employee_id = ?2",
                        params![ds, emp],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;
                match current {
                    Some((Some(x), false)) if x == c => {}
                    // Back on their own crew (they were marked out): keep their place.
                    Some((Some(x), true)) if x == c => {
                        tx.execute("UPDATE roster SET out = 0 WHERE date = ?1 AND employee_id = ?2", params![ds, emp])?;
                    }
                    _ => {
                        tx.execute(
                            "INSERT INTO roster (date, employee_id, crew_id, position, out)
                             VALUES (?1, ?2, ?3, (SELECT COALESCE(MAX(position), -1) + 1 FROM roster WHERE date = ?1 AND crew_id = ?3), 0)
                             ON CONFLICT(date, employee_id) DO UPDATE SET crew_id = excluded.crew_id, position = excluded.position, out = 0",
                            params![ds, emp, c],
                        )?;
                    }
                }
            }
            None => {
                tx.execute("DELETE FROM roster WHERE date = ?1 AND employee_id = ?2", params![ds, emp])?;
            }
        }
        tx.commit()
    }

    /// Marks a Rough-In employee out (or back in) on one day. Out takes them
    /// off their crew that day only; they stay linked to it, so the next day
    /// and "back in" put them back. Returns their crew if they have one.
    pub fn set_roster_out(&mut self, d: NaiveDate, emp: i64, out: bool) -> Result<Option<i64>> {
        let tx = self.conn.transaction()?;
        let ds = date_str(Some(d));
        let recorded = tx.prepare("SELECT 1 FROM roster_days WHERE date = ?1")?.exists([&ds])?;
        if !recorded {
            // Nothing to undo on a day that was never changed.
            if !out {
                return Ok(load_roster_c(&tx, d)?.get(&emp).map(|x| x.0));
            }
            ensure_roster_day_c(&tx, d)?;
        }
        let crew: Option<Option<i64>> = tx
            .query_row("SELECT crew_id FROM roster WHERE date = ?1 AND employee_id = ?2", params![ds, emp], |r| r.get(0))
            .optional()?;
        tx.execute("UPDATE roster SET out = ?3 WHERE date = ?1 AND employee_id = ?2", params![ds, emp, out])?;
        tx.commit()?;
        Ok(crew.flatten())
    }

    // ----------------------------------------------------------- attendance

    /// Attendance for every employee between two dates (inclusive).
    pub fn load_attendance(&self, from: NaiveDate, to: NaiveDate) -> Result<HashMap<(i64, NaiveDate), AttendanceEntry>> {
        let mut stmt = self
            .conn
            .prepare("SELECT employee_id, date, status, note FROM attendance WHERE date BETWEEN ?1 AND ?2")?;
        let rows = stmt.query_map(params![date_str(Some(from)), date_str(Some(to))], |r| {
            let d: String = r.get(1)?;
            Ok((r.get::<_, i64>(0)?, d, AttendanceEntry { status: r.get(2)?, note: r.get(3)? }))
        })?;
        let mut out = HashMap::new();
        for row in rows {
            let (emp, d, entry) = row?;
            if let Ok(d) = NaiveDate::parse_from_str(&d, "%Y-%m-%d") {
                out.insert((emp, d), entry);
            }
        }
        Ok(out)
    }

    /// Records (or replaces) one employee's attendance for one day.
    pub fn set_attendance(&self, employee_id: i64, date: NaiveDate, entry: &AttendanceEntry, user: Option<i64>) -> Result<()> {
        self.conn.execute(
            "INSERT INTO attendance (employee_id, date, status, note, updated_by, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, CURRENT_TIMESTAMP)
             ON CONFLICT(employee_id, date) DO UPDATE SET
                status=excluded.status, note=excluded.note,
                updated_by=excluded.updated_by, updated_at=CURRENT_TIMESTAMP",
            params![employee_id, date_str(Some(date)), entry.status, entry.note.trim(), user],
        )?;
        Ok(())
    }

    pub fn clear_attendance(&self, employee_id: i64, date: NaiveDate) -> Result<()> {
        self.conn.execute(
            "DELETE FROM attendance WHERE employee_id=?1 AND date=?2",
            params![employee_id, date_str(Some(date))],
        )?;
        Ok(())
    }

    /// Sets (or clears) the vehicle a crew goes out in.
    pub fn set_crew_vehicle(&self, crew_id: i64, vehicle_id: Option<i64>) -> Result<()> {
        self.conn.execute("UPDATE crews SET vehicle_id=?2 WHERE id=?1", params![crew_id, vehicle_id])?;
        Ok(())
    }

    pub fn delete_crew(&self, id: i64) -> Result<()> {
        // Jobs keep existing; their crew_id becomes NULL (ON DELETE SET NULL).
        self.conn.execute("DELETE FROM crews WHERE id=?1", [id])?;
        Ok(())
    }

    // ----------------------------------------------------------------- jobs

    pub fn load_jobs(&self) -> Result<Vec<Job>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_code, project_name, lot, phase, status, crew_id, date,
                    start_min, duration_min, notes, comments, board, tank_size, kitchen_sink, days, work_sat, work_sun,
                    source_id
             FROM jobs ORDER BY date, start_min, id",
        )?;
        let mut jobs = stmt
            .query_map([], |r| {
                let date: Option<String> = r.get(7)?;
                let board: String = r.get(12)?;
                Ok(Job {
                    id: r.get(0)?,
                    board: Board::from_db(&board),
                    project_code: r.get(1)?,
                    project_name: r.get(2)?,
                    lot: r.get(3)?,
                    phase: r.get(4)?,
                    status: r.get(5)?,
                    crew_id: r.get(6)?,
                    date: date.and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok()),
                    start_min: r.get(8)?,
                    duration_min: r.get(9)?,
                    notes: r.get(10)?,
                    comments: r.get(11)?,
                    tank_size: r.get(13)?,
                    kitchen_sink: r.get(14)?,
                    days: r.get::<_, i32>(15)?.clamp(1, MAX_JOB_DAYS),
                    weekend: Weekend { sat: r.get(16)?, sun: r.get(17)? },
                    fields: Vec::new(),
                    source_id: r.get(18)?,
                })
            })?
            .collect::<Result<Vec<_>>>()?;

        let index: HashMap<i64, usize> = jobs.iter().enumerate().map(|(i, j)| (j.id, i)).collect();
        let mut fstmt = self
            .conn
            .prepare("SELECT job_id, label, value FROM job_fields ORDER BY job_id, position")?;
        let rows = fstmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                CustomField {
                    label: r.get(1)?,
                    value: r.get(2)?,
                },
            ))
        })?;
        for row in rows {
            let (job_id, field) = row?;
            if let Some(&i) = index.get(&job_id) {
                jobs[i].fields.push(field);
            }
        }
        Ok(jobs)
    }

    /// Inserts (id == 0) or upserts the job and its extra fields. Upsert also
    /// lets "undo delete" re-create a job with its original id.
    /// `user` is recorded as the creator (new jobs) and last editor.
    pub fn save_job(&mut self, job: &Job, user: Option<i64>) -> Result<i64> {
        let tx = self.conn.transaction()?;
        let id = save_job_c(&tx, job, user)?;
        tx.commit()?;
        Ok(id)
    }

    /// Saves many jobs at once (an import); if any fails, none are saved.
    pub fn save_jobs(&mut self, jobs: &[Job], user: Option<i64>) -> Result<()> {
        let tx = self.conn.transaction()?;
        for j in jobs {
            save_job_c(&tx, j, user)?;
        }
        tx.commit()
    }

    /// Copies the whole database to `path` (a consistent snapshot, safe
    /// while the app is running). Returns false for an in-memory database.
    pub fn backup_to(&self, path: &std::path::Path) -> Result<bool> {
        if self.path.is_none() {
            return Ok(false);
        }
        self.conn.execute("VACUUM INTO ?1", [path.to_string_lossy()])?;
        Ok(true)
    }

    /// The folder the database file lives in (for backups).
    pub fn folder(&self) -> Option<PathBuf> {
        self.path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf()))
    }
}

/// Saves one job (and its extra fields) inside the caller's transaction.
fn save_job_c(tx: &Connection, job: &Job, user: Option<i64>) -> Result<i64> {
    {
        let id_param: Option<i64> = (job.id > 0).then_some(job.id);
        tx.execute(
            "INSERT INTO jobs (id, project_code, project_name, lot, phase, status, crew_id, date,
                               start_min, duration_min, notes, comments, board, created_by, updated_by,
                               tank_size, kitchen_sink, days, work_sat, work_sun, source_id, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14, ?15, ?16, ?17, ?18, ?19, ?20, CURRENT_TIMESTAMP)
             ON CONFLICT(id) DO UPDATE SET
                board=excluded.board, updated_by=excluded.updated_by, tank_size=excluded.tank_size,
                kitchen_sink=excluded.kitchen_sink, days=excluded.days,
                work_sat=excluded.work_sat, work_sun=excluded.work_sun,
                project_code=excluded.project_code, project_name=excluded.project_name,
                lot=excluded.lot, phase=excluded.phase, status=excluded.status,
                crew_id=excluded.crew_id, date=excluded.date, start_min=excluded.start_min,
                duration_min=excluded.duration_min, notes=excluded.notes,
                comments=excluded.comments, source_id=excluded.source_id, updated_at=CURRENT_TIMESTAMP",
            params![
                id_param,
                job.project_code.trim(),
                job.project_name.trim(),
                job.lot.trim(),
                job.phase.trim(),
                job.status,
                job.crew_id,
                date_str(job.date),
                job.start_min,
                job.duration_min,
                job.notes,
                job.comments,
                job.board.as_str(),
                user,
                job.tank_size.trim(),
                job.kitchen_sink.trim(),
                job.days.clamp(1, MAX_JOB_DAYS),
                job.weekend.sat,
                job.weekend.sun,
                job.source_id.trim()
            ],
        )?;
    }
    let id = if job.id > 0 { job.id } else { tx.last_insert_rowid() };
    tx.execute("DELETE FROM job_fields WHERE job_id=?1", [id])?;
    let mut st = tx.prepare("INSERT INTO job_fields (job_id, position, label, value) VALUES (?1, ?2, ?3, ?4)")?;
    for (i, f) in job.fields.iter().enumerate() {
        if f.label.trim().is_empty() && f.value.trim().is_empty() {
            continue;
        }
        st.execute(params![id, i as i64, f.label.trim(), f.value])?;
    }
    Ok(id)
}

impl Db {
    /// Moves a job to a new day/time and crew (crew timeline drag).
    pub fn move_job_to_crew(
        &self,
        id: i64,
        date: Option<NaiveDate>,
        start_min: i32,
        crew_id: Option<i64>,
        user: Option<i64>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET date=?2, start_min=?3, crew_id=?4, updated_by=?5, updated_at=CURRENT_TIMESTAMP WHERE id=?1",
            params![id, date_str(date), start_min, crew_id, user],
        )?;
        Ok(())
    }

    pub fn move_job(&self, id: i64, date: Option<NaiveDate>, start_min: i32, user: Option<i64>) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET date=?2, start_min=?3, updated_by=?4, updated_at=CURRENT_TIMESTAMP WHERE id=?1",
            params![id, date_str(date), start_min, user],
        )?;
        Ok(())
    }

    pub fn delete_job(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM jobs WHERE id=?1", [id])?;
        Ok(())
    }

    // ---------------------------------------------------------- sample data

    pub fn seed_sample(&mut self, today: NaiveDate) -> Result<()> {
        let crews: [(&str, &str, &str, [u8; 3], &[(&str, &str)]); 5] = [
            ("Underground Crew", "Mike R.", "555-0101", [52, 120, 246],
             &[("Mike R.", "Foreman"), ("Luis G.", "Journeyman"), ("Tyler B.", "Apprentice")]),
            ("Rough-In Crew 1", "Dave S.", "555-0102", [230, 126, 34],
             &[("Dave S.", "Foreman"), ("Jake P.", "Journeyman"), ("Omar H.", "Helper")]),
            ("Rough-In Crew 2", "Carlos M.", "555-0103", [39, 174, 96],
             &[("Carlos M.", "Foreman"), ("Ben W.", "Apprentice")]),
            ("Trim Crew", "Steve K.", "555-0104", [142, 68, 173],
             &[("Steve K.", "Foreman"), ("Nate L.", "Journeyman")]),
            ("Service / Warranty", "Rob T.", "555-0105", [231, 76, 60],
             &[("Rob T.", "Journeyman")]),
        ];
        let mut crew_ids = Vec::new();
        let mut rough_in_staff = Vec::new();
        for (i, (name, foreman, phone, color, members)) in crews.into_iter().enumerate() {
            // The first three crews are Rough-In staff; Trim and Service are Trim staff.
            let group = if i < 3 { Board::RoughIn } else { Board::Trim };
            let mut c = Crew::new(color);
            c.board = group;
            c.name = name.into();
            c.foreman = foreman.into();
            c.phone = phone.into();
            for (n, r) in members {
                let mut e = Employee::new();
                e.name = (*n).into();
                e.role = (*r).into();
                e.staff = group;
                let id = self.save_employee(&e)?;
                if group == Board::RoughIn {
                    rough_in_staff.push(id);
                }
                c.member_ids.push(id);
            }
            crew_ids.push(self.save_crew(&c)?);
        }
        // A few employees not yet on any crew.
        for (n, r, group) in [
            ("Marcus J.", "Journeyman", Board::RoughIn),
            ("Eli R.", "Apprentice", Board::RoughIn),
            ("Sam D.", "Helper", Board::Trim),
        ] {
            let mut e = Employee::new();
            e.name = n.into();
            e.role = r.into();
            e.staff = group;
            let id = self.save_employee(&e)?;
            if group == Board::RoughIn {
                rough_in_staff.push(id);
            }
        }

        // Attendance for Rough-In staff, this week up to today.
        let week_start = today - Duration::days(today.weekday().num_days_from_sunday() as i64);
        for d in 1..6 {
            let date = week_start + Duration::days(d);
            if date > today {
                break;
            }
            for (k, emp) in rough_in_staff.iter().enumerate() {
                let (status, note) = match (k + d as usize) % 11 {
                    3 => ("Late", "Arrived 7:40, truck trouble"),
                    7 => ("Sick", ""),
                    9 if d == 1 => ("PTO", "Approved vacation day"),
                    _ => ("Present", ""),
                };
                self.set_attendance(*emp, date, &AttendanceEntry { status: status.into(), note: note.into() }, None)?;
            }
        }

        let projects = [
            ("OAK-101", "Oak Ridge Estates"),
            ("WIL-220", "Willow Creek Townhomes"),
            ("SUM-305", "Summit Park Phase 2"),
        ];
        let phase_for_crew = ["Underground", "Rough-In", "Top-Out", "Trim / Final", "Service / Warranty"];
        let first = today.with_day(1).unwrap_or(today);
        let mut lot = 1;
        for day in 0..31 {
            let date = first + Duration::days(day);
            if date.month() != first.month() || matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
                continue;
            }
            for slot in 0..3i64 {
                let ci = ((day + slot * 2) % 5) as usize;
                let (code, name) = projects[((day + slot) % 3) as usize];
                let mut job = Job::new(Some(date), [7 * 60, 7 * 60 + 30, 12 * 60 + 30][slot as usize]);
                job.project_code = code.into();
                job.project_name = name.into();
                job.lot = lot.to_string();
                lot += 1;
                job.crew_id = Some(crew_ids[ci]);
                job.phase = phase_for_crew[ci].into();
                job.board = if ci >= 3 { Board::Trim } else { Board::RoughIn };
                if job.board == Board::Trim {
                    job.tank_size = ["40 gal", "50 gal", "75 gal", "Tankless"][(day % 4) as usize].into();
                    job.kitchen_sink = ["Double Bowl", "Single Bowl", "Farmhouse / Apron"][(day % 3) as usize].into();
                }
                job.duration_min = if slot == 2 { 4 * 60 } else { 5 * 60 };
                if date < today {
                    job.status = "Complete".into();
                }
                job.fields.push(CustomField { label: "Permit #".into(), value: format!("BP-26-{:04}", 1200 + lot) });
                if ci == 1 || ci == 2 {
                    let value = if day % 2 == 0 { (date + Duration::days(3)).format("%b %-d").to_string() } else { String::new() };
                    job.fields.push(CustomField { label: "Inspection Date".into(), value });
                }
                if day % 4 == 0 {
                    job.notes = "Gate code at the trailer. Confirm fixture package with super.".into();
                }
                if day % 3 == 0 {
                    job.comments = "Homeowner walk-through at 3 PM. Bring extra supply lines and wax rings; \
                                    confirm stub-out heights with the framer before setting anything."
                        .into();
                }
                if day % 6 == 2 {
                    job.notes = "Long note: lot backs onto the retention pond, so park on the street side. \
                                 Water is on at the meter but the yard hydrant is capped. Super wants the \
                                 tub boxes left in place until drywall is done, and the city inspector \
                                 has asked for photos of every nail plate before cover."
                        .into();
                    job.fields.push(CustomField { label: "Superintendent".into(), value: "Dana R. 555-0188".into() });
                }
                self.save_job(&job, None)?;
            }
        }
        for (i, (code, name)) in projects.iter().enumerate() {
            let mut job = Job::new(None, 7 * 60);
            job.project_code = (*code).into();
            job.project_name = (*name).into();
            job.lot = (40 + i).to_string();
            job.phase = "Rough-In".into();
            job.notes = "Waiting on framing sign-off.".into();
            self.save_job(&job, None)?;
        }
        for (i, (code, name)) in projects.iter().enumerate().take(2) {
            let mut job = Job::new(None, 7 * 60);
            job.board = Board::Trim;
            job.project_code = (*code).into();
            job.project_name = (*name).into();
            job.lot = (10 + i).to_string();
            job.phase = "Trim / Final".into();
            job.tank_size = "50 gal".into();
            job.notes = "Waiting on countertops.".into();
            self.save_job(&job, None)?;
        }
        // A multi-day job (3 work days from today), not yet given a crew.
        let mut job = Job::new(Some(today), 7 * 60);
        job.project_code = "MAP-410".into();
        job.project_name = "Maple Grove Clubhouse".into();
        job.lot = "1".into();
        job.phase = "Underground".into();
        job.duration_min = 8 * 60;
        job.days = 3;
        job.notes = "Commercial underground; three days of trenching and sleeves.".into();
        self.save_job(&job, None)?;
        // A few company vehicles.
        let crews = self.load_crews()?;
        let crew = |name: &str| crews.iter().find(|c| c.name == name).map(|c| c.id);
        let vehicles = [
            ("Truck 1", "Pickup", "2021", "Ford", "F-250", "PLB-1021", Some("Underground Crew"), 48_200, "Active", 45),
            ("Truck 2", "Pickup", "2019", "Chevrolet", "Silverado 2500", "PLB-7719", Some("Rough-In Crew 1"), 91_350, "Active", 12),
            ("Van 3", "Service Van", "2022", "Ford", "Transit 250", "PLB-3322", Some("Trim Crew"), 30_900, "Active", 120),
            ("Van 4", "Service Van", "2018", "Ram", "ProMaster 2500", "PLB-4818", None, 133_400, "In Shop", -5),
        ];
        for (unit, kind, year, make, model, plate, crew_name, miles, status, reg_days) in vehicles {
            let mut v = Vehicle::new();
            v.unit = unit.into();
            v.kind = kind.into();
            v.year = year.into();
            v.make = make.into();
            v.model = model.into();
            v.plate = plate.into();
            v.odometer = miles;
            v.status = status.into();
            v.registration_due = Some(today + Duration::days(reg_days));
            v.insurance_due = Some(today + Duration::days(200));
            v.next_service = Some(today + Duration::days(reg_days + 20));
            let vid = self.save_vehicle(&v)?;
            if let Some(cid) = crew_name.and_then(crew) {
                self.conn.execute("UPDATE crews SET vehicle_id=?1 WHERE id=?2", params![vid, cid])?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn job_round_trip_move_delete_restore() {
        let mut db = Db::open_in_memory().unwrap();
        let mut crew = Crew::new([1, 2, 3]);
        crew.name = "Rough-In 1".into();
        let crew_id = db.save_crew(&crew).unwrap();

        let mut job = Job::new(Some(d("2026-11-12")), 7 * 60);
        job.project_code = "OAK-101".into();
        job.project_name = "Oak Ridge".into();
        job.lot = "12".into();
        job.crew_id = Some(crew_id);
        job.notes = "gate code 1234".into();
        job.comments = "call super".into();
        job.fields.push(CustomField { label: "Permit #".into(), value: "P-55".into() });
        let id = db.save_job(&job, None).unwrap();

        let loaded = db.load_jobs().unwrap();
        assert_eq!(loaded.len(), 1);
        let mut expected = job.clone();
        expected.id = id;
        assert_eq!(loaded[0], expected);

        // Drag-and-drop move.
        db.move_job(id, Some(d("2026-11-14")), 9 * 60 + 30, None).unwrap();
        let moved = &db.load_jobs().unwrap()[0];
        assert_eq!(moved.date, Some(d("2026-11-14")));
        assert_eq!(moved.start_min, 570);
        assert_eq!(moved.fields, expected.fields);

        // Unschedule.
        db.move_job(id, None, 570, None).unwrap();
        assert_eq!(db.load_jobs().unwrap()[0].date, None);

        // Delete then undo (upsert with original id restores fields too).
        db.delete_job(id).unwrap();
        assert!(db.load_jobs().unwrap().is_empty());
        db.save_job(&expected, None).unwrap();
        assert_eq!(db.load_jobs().unwrap(), vec![expected]);
    }

    #[test]
    fn crews_link_to_vehicles() {
        let mut db = Db::open_in_memory().unwrap();
        let mut truck = Vehicle::new();
        truck.unit = "24".into();
        let tid = db.save_vehicle(&truck).unwrap();
        let mut a = Crew::new([1, 2, 3]);
        a.name = "Sean Pierce".into();
        a.board = Board::RoughIn;
        a.vehicle_id = Some(tid);
        let a_id = db.save_crew(&a).unwrap();
        let mut b = Crew::new([1, 2, 3]);
        b.name = "Cameron Tate".into();
        b.board = Board::RoughIn;
        let b_id = db.save_crew(&b).unwrap();
        // Two crews can share one truck.
        db.set_crew_vehicle(b_id, Some(tid)).unwrap();
        let crews = db.load_crews().unwrap();
        assert!(crews.iter().all(|c| c.vehicle_id == Some(tid)));
        db.set_crew_vehicle(a_id, None).unwrap();
        assert_eq!(db.load_crews().unwrap().iter().find(|c| c.id == a_id).unwrap().vehicle_id, None);
        // Deleting the vehicle just unlinks it.
        db.delete_vehicle(tid).unwrap();
        assert!(db.load_crews().unwrap().iter().all(|c| c.vehicle_id.is_none()));
    }

    #[test]
    fn vehicles_round_trip() {
        let mut db = Db::open_in_memory().unwrap();
        let mut crew = Crew::new([1, 2, 3]);
        crew.name = "Crew A".into();
        let crew_id = db.save_crew(&crew).unwrap();
        let driver = emp(&db, "Pat D.");
        let mut v = Vehicle::new();
        v.unit = "Truck 7".into();
        v.make = "Ford".into();
        v.vin = " 1ftbf2b6xmed12345 ".into();
        v.crew_id = Some(crew_id);
        v.driver_id = Some(driver);
        v.odometer = 1234;
        v.registration_due = Some(d("2026-12-01"));
        v.purchase_date = Some(d("2024-06-02"));
        v.info.insert("toll_pass".into(), "7700000000003".into());
        v.info.insert("tires".into(), "  ".into()); // blank details aren't stored
        let id = db.save_vehicle(&v).unwrap();
        let got = db.load_vehicles().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].purchase_date, Some(d("2024-06-02")));
        assert_eq!(got[0].get("toll_pass"), "7700000000003");
        assert_eq!(got[0].info.len(), 1);
        assert_eq!(got[0].id, id);
        assert_eq!(got[0].vin, "1FTBF2B6XMED12345");
        assert_eq!(got[0].registration_due, Some(d("2026-12-01")));
        assert_eq!(got[0].insurance_due, None);
        // Edit; deleting the crew and the driver just unassigns them.
        let mut e = got[0].clone();
        e.status = "In Shop".into();
        db.save_vehicle(&e).unwrap();
        db.delete_crew(crew_id).unwrap();
        db.delete_employee(driver).unwrap();
        let got = db.load_vehicles().unwrap();
        assert_eq!(got[0].status, "In Shop");
        assert_eq!((got[0].crew_id, got[0].driver_id), (None, None));
        // Reminders: overdue and due soon, but not far off.
        let today = d("2026-11-13");
        let mut r = Vehicle::new();
        r.registration_due = Some(d("2026-11-01"));
        r.insurance_due = Some(d("2026-12-01"));
        r.next_service = Some(d("2027-06-01"));
        let rem = r.reminders(today);
        assert_eq!(rem.len(), 2);
        assert!(rem[0].2 && !rem[1].2);
        db.delete_vehicle(id).unwrap();
        assert!(db.load_vehicles().unwrap().is_empty());
    }

    fn emp(db: &Db, name: &str) -> i64 {
        let mut e = Employee::new();
        e.name = name.into();
        db.save_employee(&e).unwrap()
    }

    #[test]
    fn deleting_crew_unassigns_jobs_and_frees_employees() {
        let mut db = Db::open_in_memory().unwrap();
        let steve = emp(&db, "Steve");
        let mut crew = Crew::new([9, 9, 9]);
        crew.name = "Trim".into();
        crew.member_ids.push(steve);
        let crew_id = db.save_crew(&crew).unwrap();
        let loaded = db.load_crews().unwrap();
        assert_eq!(loaded[0].member_ids, vec![steve]);
        assert_eq!(loaded[0].color, [9, 9, 9]);
        assert_eq!(db.load_employees().unwrap()[0].crew_id, Some(crew_id));

        let mut job = Job::new(None, 420);
        job.project_code = "X".into();
        job.crew_id = Some(crew_id);
        db.save_job(&job, None).unwrap();

        db.delete_crew(crew_id).unwrap();
        assert!(db.load_crews().unwrap().is_empty());
        assert_eq!(db.load_jobs().unwrap()[0].crew_id, None);
        // The employee still exists and is available again.
        let emps = db.load_employees().unwrap();
        assert_eq!((emps.len(), emps[0].crew_id), (1, None));
    }

    #[test]
    fn employee_cannot_be_on_two_crews() {
        let mut db = Db::open_in_memory().unwrap();
        let ana = emp(&db, "Ana");
        let ben = emp(&db, "Ben");
        let mut a = Crew::new([1, 1, 1]);
        a.name = "A".into();
        a.member_ids = vec![ana];
        let a_id = db.save_crew(&a).unwrap();

        // Crew B tries to take Ana: rejected, and nothing about B is saved.
        let mut b = Crew::new([2, 2, 2]);
        b.name = "B".into();
        b.member_ids = vec![ben, ana];
        let err = db.save_crew(&b).unwrap_err().to_string();
        assert!(err.contains("Ana"), "{err}");
        assert_eq!(db.load_crews().unwrap().len(), 1);
        assert!(db.load_employees().unwrap().iter().all(|e| e.id == ana || e.crew_id.is_none()));

        // Removing Ana from A (re-saving A without her) makes her available to B.
        a.id = a_id;
        a.member_ids.clear();
        db.save_crew(&a).unwrap();
        let b_id = db.save_crew(&b).unwrap();
        let crews = db.load_crews().unwrap();
        let b_loaded = crews.iter().find(|c| c.id == b_id).unwrap();
        assert_eq!(b_loaded.member_ids, vec![ben, ana], "member order is kept");

        // Moving straight from crew B to crew A in one step (no remove/re-add):
        // Ana leaves B, lands last on A, and is on exactly one crew.
        db.move_employee_to_crew(ana, a_id).unwrap();
        let crews = db.load_crews().unwrap();
        let members = |id: i64| crews.iter().find(|c| c.id == id).unwrap().member_ids.clone();
        assert_eq!(members(a_id), vec![ana]);
        assert_eq!(members(b_id), vec![ben]);
        db.move_employee_to_crew(ben, a_id).unwrap();
        let crews = db.load_crews().unwrap();
        assert_eq!(crews.iter().find(|c| c.id == a_id).unwrap().member_ids, vec![ana, ben], "appended last");
        // Moving to a crew that doesn't exist is refused.
        assert!(db.move_employee_to_crew(ana, 9999).is_err());
        db.move_employee_to_crew(ana, b_id).unwrap();
        db.move_employee_to_crew(ben, b_id).unwrap();

        // Releasing and deleting employees.
        db.release_employee(ben).unwrap();
        db.delete_employee(ana).unwrap();
        assert!(db.load_crews().unwrap().iter().all(|c| c.member_ids.is_empty()));
    }

    #[test]
    fn old_crew_members_become_employees() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE crews (id INTEGER PRIMARY KEY, name TEXT NOT NULL, foreman TEXT NOT NULL DEFAULT '',
                phone TEXT NOT NULL DEFAULT '', color INTEGER NOT NULL DEFAULT 0, active INTEGER NOT NULL DEFAULT 1,
                notes TEXT NOT NULL DEFAULT '');
             CREATE TABLE crew_members (id INTEGER PRIMARY KEY, crew_id INTEGER NOT NULL, position INTEGER NOT NULL,
                name TEXT NOT NULL, role TEXT NOT NULL DEFAULT '', phone TEXT NOT NULL DEFAULT '');
             INSERT INTO crews (id, name) VALUES (1, 'Rough 1'), (2, 'Trim');
             INSERT INTO crew_members (crew_id, position, name, role) VALUES
                (1, 0, 'Dave', 'Foreman'), (1, 1, 'Omar', 'Helper'), (2, 0, 'Steve', 'Foreman');",
        )
        .unwrap();
        let db = Db::init(conn, None).unwrap();
        let emps = db.load_employees().unwrap();
        assert_eq!(emps.len(), 3);
        // Everything that existed before staff groups starts out as Trim.
        assert!(emps.iter().all(|e| e.staff == Board::Trim));
        assert!(db.load_crews().unwrap().iter().all(|c| c.board == Board::Trim));
        let crews = db.load_crews().unwrap();
        let name = |id: i64| emps.iter().find(|e| e.id == id).unwrap().name.clone();
        let rough = crews.iter().find(|c| c.name == "Rough 1").unwrap();
        assert_eq!(rough.member_ids.iter().map(|&i| name(i)).collect::<Vec<_>>(), vec!["Dave", "Omar"]);
        // Old table is gone; opening again is a no-op.
        let gone: bool = db
            .conn
            .prepare("SELECT 1 FROM sqlite_master WHERE name='crew_members'")
            .unwrap()
            .exists([])
            .unwrap();
        assert!(!gone);
    }

    #[test]
    fn board_round_trips() {
        let mut db = Db::open_in_memory().unwrap();
        let mut job = Job::new(None, 420);
        job.project_code = "T".into();
        job.board = Board::Trim;
        job.tank_size = " 50 gal ".into();
        let id = db.save_job(&job, None).unwrap();
        assert_eq!(db.load_jobs().unwrap()[0].board, Board::Trim);
        assert_eq!(db.load_jobs().unwrap()[0].tank_size, "50 gal", "stored trimmed");
        job.tank_size = "Tankless".into();
        job.kitchen_sink = " Double Bowl ".into();
        job.id = id;
        db.save_job(&job, None).unwrap();
        let loaded = &db.load_jobs().unwrap()[0];
        assert_eq!(loaded.tank_size, "Tankless", "updates on edit");
        assert_eq!(loaded.kitchen_sink, "Double Bowl", "stored trimmed");
        job.id = id;
        job.board = Board::RoughIn;
        db.save_job(&job, None).unwrap();
        assert_eq!(db.load_jobs().unwrap()[0].board, Board::RoughIn);
        // Multi-day length is saved (new jobs default to 1 day).
        assert_eq!(db.load_jobs().unwrap()[0].days, 1);
        job.days = 4;
        db.save_job(&job, None).unwrap();
        assert_eq!(db.load_jobs().unwrap()[0].days, 4);
        // Weekend days are skipped unless ticked, and are saved.
        assert_eq!(db.load_jobs().unwrap()[0].weekend, Weekend::SKIP);
        job.weekend = Weekend { sat: true, sun: false };
        db.save_job(&job, None).unwrap();
        assert_eq!(db.load_jobs().unwrap()[0].weekend, Weekend { sat: true, sun: false });
    }

    #[test]
    fn old_database_is_migrated_and_sorted_by_phase() {
        // A jobs table as created by the first version (no board column).
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE jobs (
                id INTEGER PRIMARY KEY, project_code TEXT NOT NULL DEFAULT '',
                project_name TEXT NOT NULL DEFAULT '', lot TEXT NOT NULL DEFAULT '',
                phase TEXT NOT NULL DEFAULT '', status TEXT NOT NULL DEFAULT 'Scheduled',
                crew_id INTEGER, date TEXT, start_min INTEGER NOT NULL DEFAULT 420,
                duration_min INTEGER NOT NULL DEFAULT 240, notes TEXT NOT NULL DEFAULT '',
                comments TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
             INSERT INTO jobs (project_code, phase) VALUES ('A', 'Rough-In'), ('B', 'Trim / Final'), ('C', 'Underground');",
        )
        .unwrap();
        let db = Db::init(conn, None).unwrap();
        let boards: Vec<(String, Board)> =
            db.load_jobs().unwrap().into_iter().map(|j| (j.project_code, j.board)).collect();
        assert_eq!(
            boards,
            vec![("A".into(), Board::RoughIn), ("B".into(), Board::Trim), ("C".into(), Board::RoughIn)]
        );
    }

    #[test]
    fn users_and_audit_trail() {
        let mut db = Db::open_in_memory().unwrap();
        assert!(db.load_users().unwrap().is_empty());
        let mut ann = User { id: 0, name: "Ann Admin".into(), username: "ann".into(), role: Role::Admin, active: true };
        ann.id = db.create_user(&ann, "hash-a").unwrap();
        let mut bob = User { id: 0, name: "Bob".into(), username: "bob".into(), role: Role::Scheduler, active: true };
        bob.id = db.create_user(&bob, "hash-b").unwrap();

        // Usernames are unique regardless of case; lookups ignore case and spaces.
        let dup = User { id: 0, name: "Other".into(), username: "BOB".into(), role: Role::Viewer, active: true };
        assert!(db.create_user(&dup, "x").is_err());
        let (found, hash) = db.find_login("  Bob ").unwrap().unwrap();
        assert_eq!((found.id, hash.as_str(), found.role), (bob.id, "hash-b", Role::Scheduler));
        assert!(db.find_login("nobody").unwrap().is_none());

        bob.role = Role::Viewer;
        bob.active = false;
        db.update_user(&bob).unwrap();
        db.set_password(bob.id, "hash-b2").unwrap();
        let (found, hash) = db.find_login("bob").unwrap().unwrap();
        assert_eq!((found.role, found.active, hash.as_str()), (Role::Viewer, false, "hash-b2"));

        // Ann creates a job; Bob moves it; Ann edits it again.
        let mut job = Job::new(None, 420);
        job.project_code = "A".into();
        let id = db.save_job(&job, Some(ann.id)).unwrap();
        let a = &db.load_audit().unwrap()[&id];
        assert_eq!((a.created_by, a.updated_by), (Some(ann.id), Some(ann.id)));
        db.move_job(id, Some(d("2026-11-20")), 480, Some(bob.id)).unwrap();
        let a = &db.load_audit().unwrap()[&id];
        assert_eq!((a.created_by, a.updated_by), (Some(ann.id), Some(bob.id)));
        job.id = id;
        job.notes = "x".into();
        db.save_job(&job, Some(ann.id)).unwrap();
        let a = &db.load_audit().unwrap()[&id];
        assert_eq!((a.created_by, a.updated_by), (Some(ann.id), Some(ann.id)), "creator is kept");
        assert!(!a.created_at.is_empty());

        // Admin-triggered reset clears the hash; the user shows as pending.
        assert!(db.pending_resets().unwrap().is_empty());
        db.set_password(bob.id, "").unwrap();
        assert_eq!(db.pending_resets().unwrap(), std::collections::HashSet::from([bob.id]));
        db.set_password(bob.id, "hash-new").unwrap();
        assert!(db.pending_resets().unwrap().is_empty());

        db.set_setting("last_username", "ann").unwrap();
        db.set_setting("last_username", "bob").unwrap();
        assert_eq!(db.get_setting("last_username").as_deref(), Some("bob"));
    }

    #[test]
    fn sample_data_loads() {
        let mut db = Db::open_in_memory().unwrap();
        db.seed_sample(d("2026-11-12")).unwrap();
        assert_eq!(db.load_crews().unwrap().len(), 5);
        let emps = db.load_employees().unwrap();
        assert_eq!(emps.iter().filter(|e| e.crew_id.is_none()).count(), 3);
        assert!(db.load_jobs().unwrap().len() > 50);
        // Three Rough-In crews and two Trim crews; members match their crew's group.
        let crews = db.load_crews().unwrap();
        assert_eq!(crews.iter().filter(|c| c.board == Board::RoughIn).count(), 3);
        for c in &crews {
            for m in &c.member_ids {
                assert_eq!(emps.iter().find(|e| e.id == *m).unwrap().staff, c.board);
            }
        }
        // Attendance exists for Rough-In staff only (Mon–Thu of that week).
        let att = db.load_attendance(d("2026-11-08"), d("2026-11-14")).unwrap();
        assert!(!att.is_empty());
        assert!(att.keys().all(|(id, _)| emps.iter().find(|e| e.id == *id).unwrap().staff == Board::RoughIn));
        assert!(att.keys().all(|(_, day)| *day <= d("2026-11-12")));
    }

    fn emp_in(db: &Db, name: &str, staff: Board) -> i64 {
        let mut e = Employee::new();
        e.name = name.into();
        e.staff = staff;
        db.save_employee(&e).unwrap()
    }

    #[test]
    fn crews_only_take_their_own_staff_group() {
        let mut db = Db::open_in_memory().unwrap();
        let rick = emp_in(&db, "Rick", Board::RoughIn);
        let tina = emp_in(&db, "Tina", Board::Trim);

        let mut rough = Crew::new([1, 1, 1]);
        rough.name = "RI 1".into();
        rough.board = Board::RoughIn;
        rough.member_ids = vec![rick, tina];
        // Tina is Trim staff, so a Rough-In crew can't take her; nothing is saved.
        let err = db.save_crew(&rough).unwrap_err().to_string();
        assert!(err.contains("Tina"), "{err}");
        assert!(db.load_crews().unwrap().is_empty());

        rough.member_ids = vec![rick];
        rough.id = db.save_crew(&rough).unwrap();
        let mut trim = Crew::new([2, 2, 2]);
        trim.name = "Trim 1".into();
        trim.board = Board::Trim;
        trim.id = db.save_crew(&trim).unwrap();

        // One-step moves are also limited to the employee's own group.
        assert!(db.move_employee_to_crew(tina, rough.id).is_err());
        db.move_employee_to_crew(tina, trim.id).unwrap();

        // Switching Rick to the Trim list takes him off his Rough-In crew.
        let mut r = db.load_employees().unwrap().into_iter().find(|e| e.id == rick).unwrap();
        assert_eq!(r.crew_id, Some(rough.id));
        r.staff = Board::Trim;
        db.save_employee(&r).unwrap();
        let r = db.load_employees().unwrap().into_iter().find(|e| e.id == rick).unwrap();
        assert_eq!((r.staff, r.crew_id), (Board::Trim, None));

        // Moving a whole crew to the other group brings its members along.
        trim.member_ids = vec![tina, rick];
        db.save_crew(&trim).unwrap();
        trim.board = Board::RoughIn;
        db.save_crew(&trim).unwrap();
        let emps = db.load_employees().unwrap();
        assert!(emps.iter().all(|e| e.staff == Board::RoughIn && e.crew_id == Some(trim.id)));
        let crews = db.load_crews().unwrap();
        assert_eq!(crews.iter().find(|c| c.id == trim.id).unwrap().member_ids, vec![tina, rick]);
    }

    #[test]
    fn attendance_save_replace_clear() {
        let db = Db::open_in_memory().unwrap();
        let rick = emp_in(&db, "Rick", Board::RoughIn);
        let day = d("2026-11-12");
        let late = AttendanceEntry { status: "Late".into(), note: " 7:40, truck ".into() };
        db.set_attendance(rick, day, &late, None).unwrap();
        let got = db.load_attendance(day, day).unwrap();
        assert_eq!(got[&(rick, day)], AttendanceEntry { status: "Late".into(), note: "7:40, truck".into() });

        // Saving the same day again replaces it; other days are separate.
        db.set_attendance(rick, day, &AttendanceEntry { status: "Present".into(), note: String::new() }, None).unwrap();
        db.set_attendance(rick, d("2026-11-13"), &AttendanceEntry { status: "Sick".into(), note: String::new() }, None)
            .unwrap();
        let week = db.load_attendance(d("2026-11-08"), d("2026-11-14")).unwrap();
        assert_eq!(week.len(), 2);
        assert_eq!(week[&(rick, day)].status, "Present");
        // Date range is respected.
        assert_eq!(db.load_attendance(day, day).unwrap().len(), 1);

        db.clear_attendance(rick, day).unwrap();
        assert_eq!(db.load_attendance(day, day).unwrap().len(), 0);
        // Deleting the employee removes their attendance too.
        db.delete_employee(rick).unwrap();
        assert!(db.load_attendance(d("2026-11-08"), d("2026-11-14")).unwrap().is_empty());
    }

    #[test]
    fn allocated_hours_follow_daily_crews() {
        let mut db = Db::open_in_memory().unwrap();
        let ids: Vec<i64> = ["Ann", "Bo", "Cy", "Dee"].iter().map(|n| emp_in(&db, n, Board::RoughIn)).collect();
        let mut crew = Crew::new([1, 1, 1]);
        crew.name = "A".into();
        crew.board = Board::RoughIn;
        crew.member_ids = ids[..3].to_vec();
        crew.id = db.save_crew(&crew).unwrap();
        // Two-day job starting Fri Nov 13 → runs Fri 13 and Mon 16.
        let mut job = Job::new(Some(d("2026-11-13")), 420);
        job.crew_id = Some(crew.id);
        job.days = 2;
        let hours = |db: &Db, job: &Job| {
            // (people on the crew, 1 job that day) for each day.
            let counts: Vec<(usize, usize)> = job
                .work_dates()
                .iter()
                .map(|day| (db.load_roster(*day).unwrap().values().filter(|(c, _)| Some(*c) == job.crew_id).count(), 1))
                .collect();
            allocated_hours(&counts)
        };
        assert_eq!(hours(&db, &job), 48.0, "2 days × 3 people × 8 h");
        // Someone out sick Monday: 8 hours fewer.
        db.set_roster_out(d("2026-11-16"), ids[0], true).unwrap();
        assert_eq!(hours(&db, &job), 40.0);
        // A fourth person added on Friday only.
        db.set_roster_crew(d("2026-11-13"), ids[3], Some(crew.id)).unwrap();
        assert_eq!(hours(&db, &job), 48.0);
        // No crew or no date: nothing allocated.
        job.crew_id = None;
        assert_eq!(hours(&db, &job), 0.0);

        // A crew with two jobs on a day splits that day's hours evenly.
        assert_eq!(day_hours(3, 2), 12.0);
        // 2-day job: day 1 shared with another job, day 2 alone → 12 + 24.
        assert_eq!(allocated_hours(&[(3, 2), (3, 1)]), 36.0);
        assert_eq!(fmt_hours(day_hours(1, 3)), "2.7");
        assert_eq!(fmt_hours(48.0), "48");
    }

    #[test]
    fn rough_in_crews_are_recorded_per_day() {
        let mut db = Db::open_in_memory().unwrap();
        let rick = emp_in(&db, "Rick", Board::RoughIn);
        let sam = emp_in(&db, "Sam", Board::RoughIn);
        let mut a = Crew::new([1, 1, 1]);
        a.name = "A".into();
        a.board = Board::RoughIn;
        a.member_ids = vec![rick, sam];
        a.id = db.save_crew(&a).unwrap();
        let mut b = Crew::new([2, 2, 2]);
        b.name = "B".into();
        b.board = Board::RoughIn;
        b.id = db.save_crew(&b).unwrap();
        let crew_of = |db: &Db, day: &str, who: i64| db.load_roster(d(day)).unwrap().get(&who).map(|x| x.0);

        // Nothing recorded yet: every day uses the original crews.
        assert_eq!(crew_of(&db, "2026-11-01", rick), Some(a.id));
        assert_eq!(db.roster_source(d("2026-11-13")).unwrap(), None);

        // Move Rick to B on the 13th: only that day (and unrecorded days after it) change.
        db.set_roster_crew(d("2026-11-13"), rick, Some(b.id)).unwrap();
        assert_eq!(crew_of(&db, "2026-11-12", rick), Some(a.id));
        assert_eq!(crew_of(&db, "2026-11-13", rick), Some(b.id));
        assert_eq!(crew_of(&db, "2026-11-16", rick), Some(b.id), "an unrecorded later day copies the 13th");
        assert_eq!(crew_of(&db, "2026-11-13", sam), Some(a.id), "the rest of the day was copied");
        assert_eq!(db.roster_source(d("2026-11-16")).unwrap(), Some(d("2026-11-13")));

        // Change the 16th: the 13th keeps its own list.
        db.set_roster_crew(d("2026-11-16"), rick, None).unwrap();
        assert_eq!(crew_of(&db, "2026-11-16", rick), None);
        assert_eq!(crew_of(&db, "2026-11-13", rick), Some(b.id));

        // Changing an earlier day afterwards leaves later recorded days alone.
        db.set_roster_crew(d("2026-11-10"), sam, Some(b.id)).unwrap();
        assert_eq!(crew_of(&db, "2026-11-10", sam), Some(b.id));
        assert_eq!(crew_of(&db, "2026-11-13", sam), Some(a.id));
        // The standing list is untouched.
        assert!(db.load_employees().unwrap().iter().all(|e| e.crew_id == Some(a.id)));

        // Saving a crew's members applies to that day only, and refuses doubles.
        b.member_ids = vec![rick, sam];
        let err = db.save_crew_on(&b, d("2026-11-13")).unwrap_err().to_string();
        assert!(err.contains("Sam") && err.contains("another crew"), "{err}");
        a.member_ids = vec![];
        db.save_crew_on(&a, d("2026-11-13")).unwrap();
        db.save_crew_on(&b, d("2026-11-13")).unwrap();
        let day = db.load_roster(d("2026-11-13")).unwrap();
        assert_eq!((day[&rick], day[&sam]), ((b.id, 0), (b.id, 1)));
        assert_eq!(crew_of(&db, "2026-11-16", sam), Some(a.id));

        // Out sick on the 20th: off the crew that day only, back on the next.
        assert_eq!(db.set_roster_out(d("2026-11-20"), sam, true).unwrap(), Some(a.id));
        assert_eq!(crew_of(&db, "2026-11-20", sam), None);
        assert_eq!(crew_of(&db, "2026-11-21", sam), Some(a.id), "out doesn't carry to the next day");
        assert_eq!(crew_of(&db, "2026-11-16", sam), Some(a.id));
        // Marked back in the same day: back on the crew, in the same place.
        db.set_roster_out(d("2026-11-20"), sam, false).unwrap();
        assert_eq!(crew_of(&db, "2026-11-20", sam), Some(a.id));
        // Saving the crew's members while someone is out keeps their link.
        db.set_roster_out(d("2026-11-20"), sam, true).unwrap();
        a.member_ids = vec![];
        db.save_crew_on(&a, d("2026-11-20")).unwrap();
        db.set_roster_out(d("2026-11-20"), sam, false).unwrap();
        assert_eq!(crew_of(&db, "2026-11-20", sam), Some(a.id));
        // "Back in" on a day that was never changed changes nothing.
        db.set_roster_out(d("2026-11-25"), sam, false).unwrap();
        assert_eq!(db.roster_source(d("2026-11-25")).unwrap(), Some(d("2026-11-20")));

        // Deleting a crew frees its people on every day.
        db.delete_crew(b.id).unwrap();
        assert!(db.load_roster(d("2026-11-13")).unwrap().is_empty());
    }
}
