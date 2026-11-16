# Plumbing Job Scheduler

A native Windows app (Rust + egui) for scheduling plumbing jobs across projects, lots and crews.

## Running

```
cargo run --release
```

The finished program is `target\release\plumbing-scheduler.exe`. It's a single file, so you can copy it anywhere.
To try it with demo crews and jobs, run it once with `--sample` on an empty database, or use **File → Load sample data**.

## Features

- **Users and sign-in:** everyone signs in with their own username and password. The first time the app opens, it asks you to create the administrator account. Admins manage people on the **Users** tab:
  - **Admin:** everything, including managing users.
  - **Scheduler:** create, move and edit jobs, crews and employees.
  - **Viewer:** look only. Jobs open read-only and nothing can be dragged or changed.

  Users are deactivated rather than deleted, and there is always at least one active admin. Anyone can change their own password from the account menu (top right). If someone forgets theirs, an admin clicks **Make them choose a new password** on the Users tab, and that person sets a new one at their next sign-in. Passwords are stored only as salted Argon2id hashes.
- **Who changed what:** each job records who created it and who last changed it, and when. This is shown at the bottom of the job window.
- **Two schedules:** separate **Rough-In Schedule** and **Trim Schedule** tabs. Each has its own calendar and its own Unscheduled list. A job's *Schedule* setting (in the job editor) decides which page it appears on. Crews are shared, so a crew booked on both schedules at the same time still gets the double-booking warning.
- **Month view:** every scheduled job appears as a card colored by crew. **Drag a card to another day** to reschedule it. The start time stays the same.
- **Week view:** an hourly grid from 5 AM to 8 PM. **Drag a block to a different day *and* time.** Times snap to 15 minutes, and overlapping jobs sit side by side. Each block also shows the job's notes, comments and additional fields, wrapped to fit; anything that doesn't fit is cut off with "…" (hover for the full details).
- **Day view** (Rough-In and Trim): one day by the hour, full width, so each job has room for its details. Click a day's heading in the Week view to open it; ◀ / ▶ step a day at a time. Drag, double-click to add, and the 15-minute snapping work as in the Week view.
- **Allocated hours on the calendar:** in the Week and Day views, Rough-In job cards show their allocated hours (e.g. *24 hrs allocated*) when there's room; hover any card for the number.
- **8-hour default:** new jobs, and unscheduled jobs when you schedule them (dragging onto the calendar or ticking *Scheduled*), start as 8 hours per day. Change it in the job window if needed.
- **Crew Week view:** one row per crew with time running left to right by the hour. **Whole week** shows a column per day. Click a day (or its header) for that day full-width with hourly gridlines. A red line marks the current time. **Drag a job to another crew's row** to reassign the crew and the time in one move; drop it on the *Unassigned* row to unassign it. Jobs from the other schedule are shown faded, so you can see everywhere a crew will be (toggle with *Include … jobs*). Double-click an empty spot to add a job for that crew at that time.
- **Unscheduled panel:** jobs with no date yet. Drag one onto the calendar to schedule it, or drop a scheduled job here to unschedule it.
- **Undo:** Ctrl+Z (or Edit → Undo) reverses moves, edits, creates and deletes.
- **Double-booking warning:** a red outline and ⚠ appear when a crew has overlapping jobs on the same day.
- **Job editor** (click any job, double-click a day, or **+ New Job**):
  - Project Code, Project Name, Unit / Lot #, Phase / Task, Crew, Status, Date, Start Time, Duration (hours per day), Days
  - **Water Tank Size** (Trim jobs only), with quick picks such as 40 gal, 50 gal and Tankless
  - **Kitchen Sink** (Trim jobs only): any text (style, model #), with quick picks such as Single Bowl, Double Bowl and Farmhouse
  - Notes and Additional Comments
  - **Additional Fields:** add any number of custom name/value fields per job (Permit #, Inspection Date, Gate Code, …)
  - **Save & Duplicate for Next Lot** saves the job and opens a copy with the lot # cleared, for quickly scheduling a run of lots
- **Rough-In Staff and Trim Staff tabs:** two separate staff groups, each with its own **Crews** and **Employees** sub-tabs (Rough-In Staff also has **Attendance**). A crew only takes employees from its own group (the database enforces this). Use the **Staff group** switch on a crew to move it, with its members, to the other group, or the **Staff list** switch on an employee to move just them (this takes them off their current crew). The job window lists the job's own group's crews first.
  - **Rough-In crews are set day by day.** The **Crews on** date picker in the Rough-In Staff sub-tab row (◀ / date / ▶ / Today) picks the day that Crews, Employees and Attendance show and edit. Changing a crew's members on a day changes **only that day**. A day nobody has changed yet starts as a copy of the most recent changed day before it (shown as *same as …*); before any changes, the original crew list is used. Days already changed are never touched by changes to other days. The schedule follows these daily crews: the job window lists who is on the job's crew on the job's date, and the Crew Week view lists each crew's people for the day shown. Trim crews still have one standing member list.
  - **Employees:** one list of everyone (name, role, **license level** (Apprentice / Resident / Journey), payroll **employee ID**, phone, active, notes), showing which crew each person is on or **Available**.
  - **Driver screen:** each employee has two screens, **Details** and **Driver**.
    - **Driver's License:** status (Valid, Restricted, Expired, Suspended, Revoked, No license), license #, state, class / CDL, endorsements, expiry date (flagged when due soon or past), **points** on the license, last MVR check and notes. Saved with **Save License**.
    - Next to the points, the screen shows how many points the moving violations recorded in the app add up to over the last 3 years.
    - **Accidents**, **Moving Violations** and **Parking Tickets** each have their own section. Each one records the date, the company vehicle involved, location, what happened, citation or claim #, fine or damage cost, points (moving violations), at fault (accidents), status (Open, Paid, Contested, Dismissed, Closed) and notes. **+ Add …** and **Edit** open a small window; Delete is in that window.
    - A summary line counts accidents (and how many were at fault), violations, tickets and total fines.
    - A license that's expired, suspended, revoked or expiring within 30 days shows a red ⚠ in the employee list (hover for why) and next to the Driver switch.
    - Deleting an employee deletes their driver records.
  - **Import Headcount…** (Employees sub-tab): pick the payroll *Active Headcount* report (.csv).
    - Only plumbing positions are taken, each into its staff list: **RI Plumber** and **RI Plumber Supervisor** into Rough-In Staff, **Trim Plumber** and **Trim Plumber Supervisor** into Trim Staff. Supervisors get the *Field Supervisor* role.
    - Each person's license level and employee ID come along.
    - A preview lists who will be added or updated and who's left out, and why. In it you can tick supervisors whose people should be left out (Reed, Martin to start); the app remembers the ticks.
    - People already in the app (same employee ID, or same name) are updated, not duplicated, keeping their crew, phone and notes. Terminated people are skipped.
    - The same import runs from the command line: `plumbing-scheduler --import-headcount <file.csv>`, adding `--apply` to save. Filter by All, Available, On a crew or Inactive. Change anyone's crew right from their page with the **Crew** dropdown; picking a crew moves them there in one step (or choose *No crew* to make them available).
  - **Crews:** create crews, set the foreman, phone, calendar color and active status, and see each crew's upcoming jobs. Crew members are picked from the employee list, and **only available employees are offered**. An employee can belong to just one crew; the database enforces this. Removing someone from a crew, or deleting the crew, makes them available again.
  - **Attendance** (Rough-In Staff): a week grid of Rough-In employees. Click a day to mark Present, Late, Absent, Sick, PTO or Off, with an optional note (shown as •). **All present** fills every empty cell for that day. A weekly total shows for each person. The **Crew** column shows (and changes) crews on the chosen day; click a day's heading to pick it. Marking someone **PTO, Off, Absent or Sick** takes them off their crew **for that day only**; the window warns before saving. They stay linked to the crew, so the next day starts with them back on it, and marking them Present or Late again (or clearing the entry) puts them straight back.
  - **Day view** (Week / Day switch on Attendance): one day laid out like the paper attendance sheet, one card per crew with the lead first in red, then the crew. Each name shows that day's status; click a name to record attendance. **Drag a name** (or its status chip) onto another crew's card to move them there; the card under the pointer lights up. Dropping on Unassigned or any other non-crew card takes them off their crew. Right-clicking a name also offers a Move-to-crew menu. Field Supervisors, people out that day (Off, PTO, Sick, Absent) and anyone else without a crew get their own cards at the end. ◀ / Today / ▶ step a day at a time; crew moves made here apply to that day only.
- **Multi-day jobs:** set **Days** in the job window (e.g. 3). The job runs that many days from its date, skipping Saturdays and Sundays unless you tick **Include Saturday** and/or **Include Sunday** (a ticked weekend day counts as one of the job's days, so a 2-day job starting Friday with Saturday ticked runs Fri and Sat). The window shows the end date, at the same start time and hours each day. Calendar cards are tagged 1/3, 2/3, 3/3. Dragging a later day's card moves the whole job so that day lands where you drop it. Double-booking warnings check every day the job runs.
- **Allocated Hours** (Rough-In jobs): the job window shows the man-hours allocated to the job: **8 hours per person per day**, counting everyone on the job's crew on each day it runs, taken from the daily Rough-In crews (so people marked out that day aren't counted). It updates by itself as people are added to or taken off the crew on any of those days, and *By day* lists who counts each day. It's also on the job's hover card. A 2-day job with 3 people each day shows 48 hrs. When a crew has more than one job on a day (either schedule; cancelled jobs don't count), that day's hours are **split evenly** between its jobs, e.g. 3 people with 2 jobs gives each job 12 hrs; *By day* shows the split.
- **Show Start Date Only** (filter row): checked, multi-day jobs appear only on the day they start; unchecked, they appear on every day they run. The app remembers the setting.
- **Filters:** by crew, by text search (code, project, lot, notes, custom fields), and a "hide completed" toggle.
- **Import Jobs…** (both schedule tabs, next to + New Job): bring jobs in from another system's CSV export. The file layout, rules and a sample export query are in [templates/Jobs Import Guide.md](templates/Jobs%20Import%20Guide.md), with a blank template and an example file next to it.
  - Jobs are matched by **Source ID**, so the same export can be imported again and again without duplicates.
  - Blank cells leave existing values alone; `CLEAR` erases one.
  - Crews and dates on existing jobs only change if you tick *Update scheduling*, so moves made in the app are kept.
  - A preview shows new and updated jobs, errors (those rows are skipped) and warnings before anything is saved.
  - The database is backed up first, and nothing is ever deleted.
  - The command-line version, `--import-jobs <file.csv> [--update-scheduling] [--apply]`, is for scheduled imports.
- **Vehicles tab:** the company fleet, laid out like the fleet management log. Each vehicle has:
  - Truck #, body type, year / make / model, status (**Active**, **In Shop**, **Out of Service** or **Spare**), license plate, VIN and **tire size**
  - **🔧 Shop** (shown while the status is **In Shop**): the shop, work being done, **drop-off date** (filled in with today when you choose In Shop), **estimated time of repair**, **estimated cost of repair** and **estimated pickup date**.
    - The pickup date is flagged when it's due soon or past.
    - The list shows *In Shop · pickup Nov 20*, in red once the pickup date has passed.
    - **Final cost:** what the repair actually cost, entered when you know it.
  - **Repair History:** every shop visit is kept.
    - Saving a vehicle as **In Shop** starts a repair stamped with the date, time and who saved it. The Shop fields fill it in as they change.
    - Saving it with any other status (e.g. back to Active) closes the repair with today as the *Back* date and clears the Shop fields for next time.
    - Each vehicle lists its repairs (put in shop, shop, work, drop-off, back, days in the shop, est. time, est. cost, final cost), with totals in the heading: number of repairs, total cost (final, else estimate) and total days in the shop.
    - **Edit** fixes a finished repair (e.g. the final cost when the invoice arrives) or deletes it. **+ Add past repair** records one from before the app tracked them.
  - **Repair report:** **Export this vehicle's repairs…** or **Export all vehicles…** saves a CSV that opens in Excel. It has one row per repair with every field and a total line per vehicle (and for the whole fleet).
  - **Tab expires**, **Insurance due** and **Purchase date**
  - **Crews:** the crews that go out in this vehicle (two crews can share one truck). **Assign to crew…** puts it on a crew and **x** takes it off; both save straight away. The same link is the **Vehicle** setting on each crew (Staff → Crews), which lists every truck with its driver and status and says if another crew also uses it.
  - **On Attendance:** each crew's card on the Attendance day board shows its truck from the vehicle records, e.g. *🚚 #24 2021 Chevy Silverado 2500*. If that truck is **In Shop** or **Out of Service**, the card warns *⚠ Truck in shop (pickup Nov 20)*. A crew with no truck linked shows the first line of its notes instead.
  - **Registration & IDs:** tab #, title, Good To Go pass, Samsara GPS, dash cam SN
  - **Driver:** name, phone and supervisor. **Pick employee…** links the driver to an employee (✔ shows the link).
  - **Oil & Service History:** odometer, last oil change, next service, oil change log, due, brakes, tires, tune-up, battery, belts, other repairs, recalls
  - Notes

  The list on the left is in truck-number order, or **Sort by: Driver** puts it in order of the driver's last name, with spare and unassigned vehicles at the end. The app remembers your choice. Each row shows the status dot, the driver, and the crew color. Dates that are overdue (red) or within 30 days (orange) are flagged, and the tab title counts them, e.g. *Vehicles (2 due)*. Filter by status or Due soon, or search across everything (truck #, plate, VIN, driver, GPS, toll pass, service notes…). Deleting a crew or employee just clears it from any vehicle. Viewers can look but not change anything.
- **Import Mileage… (Vehicles tab):** pick the GPS company's asset report (.csv, rows like `#9 - Adam Sorensen,CHEVROLET,SILVERADO HD,23653.8`).
  - Each row is matched to a vehicle by its truck # (driver names in the report are ignored, so spelling differences don't matter).
  - Matching vehicles get the report's mileage, rounded to whole miles, and *Mileage updated* shows the date.
  - Afterwards a summary lists:
    - every vehicle in the report that **didn't match** a vehicle here
    - rows with no mileage (left unchanged)
    - any truck # listed twice (the higher reading is used)
    - each updated vehicle's mileage before and after, with readings lower than before in orange
  - It's all-or-nothing: if saving fails, nothing changes.
- **Importing the fleet log:** save the *Truck List* sheet of the fleet management log as tab-separated text (Excel: *Save As → Text (Tab delimited)*). Then run `plumbing-scheduler --import-fleet <file.txt>` to see what would change, and add `--apply` to save it.
  - Columns are found by their header names.
  - A truck # already in the app is updated, keeping its crew, insurance, next service and notes.
  - Drivers are linked to employees when the names match, allowing for *Last, First* order and a one-letter misspelling.
  - "Spare" drivers mark the vehicle as Spare.
  - A plate and tab # that look swapped are put back.

## Data

Everything is stored in a SQLite database at `%APPDATA%\PlumbingScheduler\schedule.db` (**File → Open data folder**).
Back it up by copying that file. To use a different file, set the `PLUMBING_SCHEDULER_DB` environment variable to its path.

## Development

```
cargo test     # drop logic, lane layout, conflict detection, database round-trips
```

| File | Purpose |
| --- | --- |
| `src/model.rs` | Job / Crew types, phase and status lists, formatting helpers |
| `src/db.rs` | SQLite schema, load/save, sample data |
| `src/app.rs` | App state, toolbar, filters, undo, editor wiring |
| `src/calendar.rs` | Month and week views, Unscheduled panel, drag and drop |
| `src/job_editor.rs` | Job edit window |
| `src/crews.rs` | Crew management screen |
| `src/employees.rs` | Employee list screen |
| `src/users.rs` | Sign-in, first-admin setup, Users admin page, change password |
| `src/auth.rs` | Password hashing and rules |
| `src/attendance.rs` | Rough-In attendance week grid and day board |
