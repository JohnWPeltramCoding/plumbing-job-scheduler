# Importing Jobs into the Plumbing Job Scheduler

This guide describes how jobs from another system (the source database) are brought into the scheduler. The source system exports a CSV file in the layout below, and the scheduler reads that file.

- `Jobs Import Template.csv`: the header row only. Start every export from this.
- `Jobs Import Example.csv`: the same columns with five sample jobs showing the common cases.

## Why CSV

- **Any database can produce it.** SQL Server, Access, MySQL and Excel all export CSV without add-ons.
- **The two systems stay separate.** The scheduler never needs a login to the source database, and vice versa.
- **People can read it.** Open the file in Excel to check it before importing.
- **It can be automated later.** A scheduled export plus the scheduler's command-line import can run on its own.

## The file

- **Format:** CSV, UTF-8, comma-separated, **one header row** with the column names exactly as shown (case doesn't matter, and column order doesn't matter).
- **One row per job.** A job that runs several days is still one row (use **Days**).
- **Text with commas, quotes or line breaks** must be in double quotes, with any quote inside doubled (`"Stub-outs, see ""P2"""`). Database export tools do this automatically.
- **Extra columns are ignored**, so the export can include fields the scheduler doesn't use.
- **Name:** `Jobs Import YYYY-MM-DD.csv`, saved in `Import Files\Jobs\`.

## Columns

| Column | Required | Format / allowed values | Default if blank | Notes |
|---|---|---|---|---|
| **Source ID** | **Yes** | The source system's unique job ID, e.g. `JOB-10421` | (none) | How the scheduler recognizes the job next time. Must never change or be reused. |
| **Schedule** | **Yes** | `Rough-In` or `Trim` | (none) | Which schedule tab the job goes on. |
| **Project Code** | **Yes** | Text, e.g. `WIL-220` | (none) | |
| Project Name | No | Text | (none) | |
| Lot | No | Text, e.g. `41`, `007`, `Bldg C-12` | (none) | Kept as text, so leading zeros stay. |
| Phase | No | Text. Presets: Rough-In: `Underground`, `Rough-In`, `Top-Out`, `Tub / Shower Set`, `Gas`, `Inspection`. Trim: `Trim / Final`, `Fixture Set`, `Water Heater`, `Gas Final`, `Inspection`, `Service / Warranty` | (none) | Other text is allowed. |
| Status | No | `Scheduled`, `Confirmed`, `In Progress`, `Complete`, `On Hold`, `Cancelled` | `Scheduled` | Anything else is an error. |
| Crew | No | A crew name exactly as it appears in the scheduler, e.g. `Zachary McBride` | Unassigned | An unknown crew name is reported, and the job is left unassigned. |
| Start Date | No | `YYYY-MM-DD` (also accepted: `MM/DD/YYYY`) | Unscheduled | Blank puts the job in the Unscheduled list. |
| Start Time | No | `7:00 AM`, `12:30 PM` or 24-hour `07:00`, `13:30` | `7:00 AM` | Rounded to 15 minutes. |
| Hours Per Day | No | A number of hours, e.g. `8`, `4.5` | `8` | Between 0.25 and 24. |
| Days | No | Whole number from 1 to 60 | `1` | Workdays the job runs. |
| Include Saturday | No | `Yes` / `No` | `No` | A Saturday counts as one of the job's days. |
| Include Sunday | No | `Yes` / `No` | `No` | Same, for Sundays. |
| Water Tank Size | No | Text, e.g. `50 gal`, `Tankless` | (none) | Shown on Trim jobs. |
| Kitchen Sink | No | Text, e.g. `Double Bowl`, a model # | (none) | Shown on Trim jobs. |
| Notes | No | Text | (none) | |
| Comments | No | Text | (none) | |
| **Field: _name_** | No | Text | (none) | Any column whose header starts with `Field: ` becomes an **Additional Field** with that name. For example, `Field: Permit #` becomes *Permit #*. Add as many as you like. |

## How the import behaves

1. **Preview first.** In the scheduler, **Import Jobs…** reads the file and shows what it will do before saving anything:
   - jobs to **add** and to **update**
   - jobs already **up to date**
   - **errors**
   - **warnings**, such as an unknown crew
2. **New or existing** is decided by **Source ID**. A Source ID the scheduler hasn't seen is added. One it has seen updates that job.
3. **Blank means "leave it alone"** on an existing job. A blank cell never erases what's in the scheduler. To clear a value, put `CLEAR` in the cell.
4. **Schedulers' changes are protected.** On existing jobs, the import does **not** change **Crew, Start Date, Start Time, Hours Per Day, Days, or the weekend settings** unless *Update scheduling (dates and crews)* is ticked in the preview. That way, a job a scheduler moved or crewed in the app isn't moved back by the next export. Everything else (names, phase, status, notes, tank, sink, Additional Fields) is updated from the file.
5. **Nothing is ever deleted by an import.** To call a job off, export it with Status `Cancelled`.
6. **Rows with errors are skipped and listed.** The rest of the file still imports. Fix the problem in the source and re-run.
7. **A backup of the scheduler's database is made** before each import.
8. **Running the same file twice changes nothing** the second time.

### Errors (row skipped)
- Missing Source ID, Schedule or Project Code
- The same Source ID twice in one file
- Schedule not `Rough-In` / `Trim`
- Status not in the list
- Start Date, Start Time, Hours Per Day or Days not in a valid format or range
- Include Saturday / Sunday not `Yes` / `No`

### Warnings (row imported)
- Crew name not found: the job is imported as Unassigned, and the name is listed so you can fix it
- The crew is in the other staff group (a Trim crew on a Rough-In job)
- A start date more than a year in the past or future

## Workflow

1. **One-time setup in the source system.** Create a saved query or view that returns exactly the template's columns (sample query below). Test it once by opening the CSV in Excel next to `Jobs Import Example.csv`.
2. **Export.** Run the query and save the result as CSV (UTF-8) with a header row, as `Import Files\Jobs\Jobs Import YYYY-MM-DD.csv`.
3. **Import.** In the scheduler, open the **Rough-In Schedule** or **Trim Schedule** tab and click **Import Jobs…** (either tab; each row's *Schedule* column decides where it goes). Pick the file and review the preview.
4. **Check the report.** Fix any errors or unknown crews in the source, and re-run if needed. Re-running is always safe.
5. **How often:** whenever the source changes, typically daily or weekly. When it's routine, the export and import can be scheduled to run unattended using the command-line version below.

## Command line (for scheduled imports)

The same import, without the window. It's useful for a nightly Windows Task Scheduler job right after the source system's export.

```
plumbing-scheduler.exe --import-jobs "Import Files\Jobs\Jobs Import 2026-11-13.csv"
plumbing-scheduler.exe --import-jobs "Import Files\Jobs\Jobs Import 2026-11-13.csv" --apply
plumbing-scheduler.exe --import-jobs "…csv" --update-scheduling --apply
```

- The first form only reports what it would do: one line per job to add or update, then every error and warning.
- `--apply` saves the changes, after backing up the database.
- `--update-scheduling` also applies crews, dates and times to jobs already in the app. It's the same as ticking that box in the window.
- Backups are saved as `schedule.backup-YYYYMMDD-HHMMSS-before-job-import.db` next to the database (`%APPDATA%\PlumbingScheduler`).

## Sample export query (SQL Server)

Adjust the table and column names on the right side of each line to match the source database. Keep the names in brackets exactly as shown.

```sql
SELECT
    j.JobNumber                                   AS [Source ID],
    CASE WHEN j.PhaseName IN ('Trim', 'Final', 'Water Heater', 'Fixture Set', 'Gas Final', 'Service', 'Warranty')
         THEN 'Trim' ELSE 'Rough-In' END          AS [Schedule],
    p.ProjectCode                                 AS [Project Code],
    p.ProjectName                                 AS [Project Name],
    j.Lot                                         AS [Lot],
    j.PhaseName                                   AS [Phase],
    CASE j.StatusCode
         WHEN 'C'  THEN 'Complete'
         WHEN 'H'  THEN 'On Hold'
         WHEN 'X'  THEN 'Cancelled'
         ELSE 'Scheduled' END                     AS [Status],
    ''                                            AS [Crew],          -- leave blank if crews are assigned in the scheduler
    CONVERT(varchar(10), j.ScheduledDate, 23)     AS [Start Date],    -- 23 = YYYY-MM-DD
    FORMAT(j.ScheduledStart, 'h:mm tt')           AS [Start Time],
    j.EstimatedHours                              AS [Hours Per Day],
    COALESCE(j.EstimatedDays, 1)                  AS [Days],
    'No'                                          AS [Include Saturday],
    'No'                                          AS [Include Sunday],
    j.WaterHeater                                 AS [Water Tank Size],
    j.KitchenSink                                 AS [Kitchen Sink],
    j.Notes                                       AS [Notes],
    ''                                            AS [Comments],
    j.PermitNumber                                AS [Field: Permit #],
    p.Superintendent                              AS [Field: Superintendent],
    p.SitePhone                                   AS [Field: Site Contact Phone],
    p.GateCode                                    AS [Field: Gate / Lockbox Code],
    j.PONumber                                    AS [Field: PO #]
FROM Jobs j
JOIN Projects p ON p.ProjectID = j.ProjectID
WHERE j.ScheduledDate >= DATEADD(day, -7, CAST(GETDATE() AS date))  -- recent and upcoming jobs
   OR j.ScheduledDate IS NULL                                        -- plus anything not yet scheduled
ORDER BY j.ScheduledDate, p.ProjectCode, j.Lot;
```

- **Saving the result as CSV:**
  - **SSMS:** right-click the results, then *Save Results As…* → CSV.
  - **Unattended:** `sqlcmd -S SERVER -d DATABASE -E -i jobs_export.sql -s"," -W -o "Jobs Import.csv"`.
- **Access:** run the query, then *External Data → Text File*, choose *Delimited, comma, include field names*, UTF-8.
- **MySQL:** use `DATE_FORMAT(j.ScheduledDate, '%Y-%m-%d')` instead of `CONVERT(...)` and `DATE_FORMAT(j.ScheduledStart, '%l:%i %p')` for the time.

## Excel tips

If you open or edit the file in Excel before importing:
- **Save with *File → Save As → CSV UTF-8*.** Plain *CSV* can damage names with accents.
- **Excel drops leading zeros** (Lot `007` becomes `7`) and may rewrite dates. Either don't re-save the file in Excel, or format those columns as **Text** before pasting.
- **Keep the header row exactly as in the template.**
