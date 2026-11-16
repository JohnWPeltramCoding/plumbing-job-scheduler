# Plumbing Job Scheduler – roadmap

Ideas collected on 2026-11-16 from building the app so far and the follow-ups noted along the way. These are not
commitments, just a list to come back to. Items marked ✅ are done and 🚧 are in progress.

## Where it stands

A Windows desktop app (Rust, egui) with its data in a local SQLite file, used on one PC at a time.

- ✅ **Scheduling:** Rough-In and Trim schedules; Month, Week, Day and Crew Week views; drag and drop; multi-day
  jobs with optional Saturday/Sunday; allocated man-hours from the daily crews; double-booking warnings; undo.
- ✅ **Staff:** Rough-In and Trim crews and employees. Rough-In crews are recorded per day; attendance drives who's on
  each crew. Employees come from the payroll headcount import (license level, payroll ID). Each employee has a
  Driver screen: license status, points, MVR check, accidents, moving violations, parking tickets.
- ✅ **Vehicles:** the fleet from the fleet log; crews linked to their truck (shown on Attendance, with a warning when
  the truck is in the shop); repair history with costs and a CSV report; GPS mileage import; sort by truck or driver.
- ✅ **Imports:** jobs from another system's CSV (matched by Source ID, preview, backup first), payroll headcount,
  fleet log, GPS mileage.
- ✅ **Users and roles:** Admin / Scheduler / Viewer, with "who changed what" on jobs.
- ✅ **Code on GitHub:** private repo with the full history, plus a public copy with made-up data.

## Top recommendations

In order (decided 2026-11-16): the shared database comes first; see [Plan: shared database](#plan-shared-database).

1. **Next up: shared PostgreSQL database, same platform as HR Toolbox and AR Tracker** – the one change that lets more than one
   person use the app at once. Move from the local SQLite file to PostgreSQL in its own `scheduler` schema, sharing
   the development server (`..\postgres_portable`) and, later, the AWS managed PostgreSQL planned for the other two
   apps. The storage code is all in `db.rs`, so the screens don't change. Includes refreshing to show other people's
   changes and a warning when two people edit the same job.
2. **Sign-in and permissions on the server (`platform_auth` / Cognito)** – follow HR Toolbox's remote mode: the
   desktop app talks to a small API, the API holds the only database login, and roles are enforced there. One
   company sign-in with MFA instead of separate app passwords.
3. **Employees from the HR Toolbox census** – one list of people for both apps, matched by employee number (already
   stored in the scheduler). Replaces the payroll CSV import, and new hires and terminations flow through on their
   own.
4. **Crews on jobs, and a daily sheet to hand out** – the 68 current jobs are unassigned since the crews were rebuilt.
   Add a printable / PDF daily schedule per crew (job, address, start time, truck) and an attendance sheet in the
   same layout as today's paper one.
5. **Automatic job feed** – once the source system's export query is set up, run `--import-jobs` on a schedule
   (Windows Task Scheduler, later on the server), then consider a direct connection to the source system.

## Plan: shared database

Matches how HR Toolbox and AR Tracker already work: **sqlx 0.8** with **tokio**, PostgreSQL only, tables in their own
schema (`scheduler`), migrations as SQL files run on start-up, and a settings file pointing at the server. Development
uses the shared portable server (`..\postgres_portable`, `localhost:5433`, started with
`..\hr_toolbox\scripts\pg.ps1 start`). Production is the AWS managed PostgreSQL planned for the other two apps.

**Phase 1 – PostgreSQL storage (still one PC at a time)**
- Settings file `scheduler.toml` (git-ignored), with a `[database]` section (`url`, `schema`) like `hr_toolbox.toml`.
  The password comes from `PGPASSWORD`, never the file; `?sslmode=require` for a server reached over the network.
- Schema rewritten as PostgreSQL migrations with native types: `DATE`, `TIMESTAMPTZ`, `BOOLEAN`, `NUMERIC(12,2)` for
  money, identity keys.
- `db.rs` moved to sqlx behind the same functions, so screens and imports don't change.
- Tests run against the development server, each in its own temporary schema, as in the other two apps.
- **One-time copy of today's data** from `schedule.db` into PostgreSQL, keeping every ID so all the links survive (crews,
  daily rosters, attendance, vehicles, repairs, driver records, jobs and their history). Verified by comparing counts
  and spot checks, with the SQLite file kept as a backup.
- Backups move from copying a file to `pg_dump` (scheduled), plus the managed service's own backups later.

**Phase 2 – several PCs on the office network**
- Every PC points at the same server (office server or the AWS instance through a VPN), using its own database login.
- **Seeing other people's changes:** refresh when someone else saves (PostgreSQL `LISTEN/NOTIFY`), not just on restart.
- **Two people editing the same thing:** each record carries a version; saving over someone else's newer change is
  stopped with a message and a reload instead of silently overwriting it.
- App passwords still work as today during this phase.

**Phase 3 – remote use, company sign-in (HR Toolbox's remote mode)**
- A small scheduler API (or endpoints alongside HR Toolbox's), holding the only database login.
- Sign-in through `platform_auth` / Cognito with MFA; Admin / Scheduler / Viewer become Cognito groups enforced by the API.
- The desktop app switches to talking to the API; the same app works in the office and in the field.

**Phase 4 – one list of employees**
- With all three apps on one server, the scheduler reads employees from the HR Toolbox census (matched by employee
  number) instead of the payroll CSV import.

**Decisions to make along the way**
- Whether to share `platform_auth` as a common crate (a git dependency) or copy it into this repo.
- One database with a schema per app, or a database per app on the same server.
- Where the office-network server lives in Phase 2 (office server vs going straight to AWS).

## Data cleanup (quick wins)

- **Vehicle tabs:** 63 of 64 vehicles show tabs expired on Jan 31, 2026 (as entered in the fleet log). Update them
  from DOL so the reminders mean something.
- **Drivers:** re-link vehicle drivers to the new employee records (25 were linked before the staff was rebuilt;
  some names now differ, e.g. a nickname in the fleet log vs the legal name in payroll). Needs a match list to review.
- **Crews:** three crews have no truck; 9 Rough-In employees are on no crew (one isn't on the attendance sheet at
  all).
- **Fleet log vs attendance sheet:** they sometimes disagree on who drives which truck.
- **GPS mileage:** every GPS reading was lower than the fleet log. Decide which is the true odometer before relying
  on mileage-based maintenance.

## Scheduling

- **Trim crews by day** – Rough-In crews already change day to day; Trim still has one standing list.
- **Job completion** – actual hours, completed by, sign-off notes and photos (S3), compared with allocated hours.
- **Capacity view** – people and hours available per day vs hours scheduled, by crew and for each schedule.
- **Crew rules** – warn when a crew has no Journey/Resident, or a lead has no valid driver's license.
- **Notifications** – text or email each lead their next-day jobs and truck.
- **Read-only phone view for leads** – today's jobs and crew, once there's a server.

## Staff and attendance

- **Hours export for payroll** – attendance (Present / Late / Off / PTO) per pay period, in the payroll system's layout.
- **Attendance history and trends** – absences per person, per crew, per month.

## Vehicles

- **Samsara API** – pull odometer and location automatically instead of the CSV export.
- **Maintenance by mileage** – oil change due every N miles (from the oil log), with reminders, alongside the date ones.
- **Repairs** – attach invoices (S3), cost per vehicle per year, and cost per mile from the mileage.
- **Recalls** – open recalls as structured entries (number, date, completed) instead of free text.
- **Fuel** – fuel card spend per vehicle, if the fuel program can export it.

## Drivers

- **Restrict driver records to HR / admins** – license numbers, points and violations visible only to those roles;
  license numbers encrypted and shown as the last 4 digits.
- **Annual MVR check reminders** – flag drivers whose last MVR check is over a year old.
- **Points and violations policy** – a configurable threshold (e.g. points in 3 years) that flags a driver.
- **Accidents to HR Toolbox** – send a vehicle accident to HR Toolbox as an incident (and workers comp case if someone
  was hurt) instead of entering it twice.

## Reports

- Allocated vs actual hours by job, project and crew.
- Vehicle repair costs by vehicle, month and shop; days out of service.
- Driver summary: license status, expirations, points, incidents.
- Export any list to Excel.

## Before go-live (security)

- Sign-in through `platform_auth` (Cognito) with MFA required; roles enforced on the server.
- Database reachable only from the API; HTTPS only.
- Driver license numbers encrypted; driver records limited by role.
- Audit log for employees, licenses, incidents and vehicles (jobs already have one).
- Encrypted, versioned backups with tested restores.
- Dependency scanning (`cargo audit`).

## Housekeeping

- **Installer and updates** – a proper Windows installer and a way to push new versions to every PC.
- **Public repo** – choose a license (or keep "all rights reserved"); a script to refresh the public copy from the
  private one with the data scrubbed.
