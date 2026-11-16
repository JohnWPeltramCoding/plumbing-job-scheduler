//! Import the fleet management log ("Truck List" sheet, saved as tab-separated
//! text with its header row) into the Vehicles tab. Columns are found by their
//! header names, so their order doesn't matter. A truck # that's already in
//! the app is updated in place: the sheet's columns overwrite, and anything the
//! sheet doesn't have (crew, insurance, next service, notes) is kept.
//!
//!   plumbing-scheduler --import-fleet <file.tsv>          shows what would change
//!   plumbing-scheduler --import-fleet <file.tsv> --apply  saves it

use std::collections::HashMap;

use chrono::NaiveDate;

use crate::db::Db;
use crate::model::*;

/// One vehicle from the sheet, and how it lines up with the app.
pub struct Planned {
    pub vehicle: Vehicle,
    /// Existing vehicle with the same truck # (updated rather than added).
    pub existing: Option<i64>,
    /// Things worth a look (driver not matched, columns swapped, ...).
    pub notes: Vec<String>,
}

/// "CHEVY SILVERADO 2500" -> "Chevy Silverado 2500"; words with digits stay as typed.
fn title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            if w.chars().any(|c| c.is_ascii_digit()) || w.len() <= 1 {
                w.to_string()
            } else {
                let lower = w.to_lowercase();
                let mut cs = lower.chars();
                cs.next().map(|f| f.to_uppercase().collect::<String>() + cs.as_str()).unwrap_or_default()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A date cell: "2026-01-31" (exported date), or text such as "07/15/26 @7648",
/// "09.26.26" or "1/26/2023". None if it doesn't start with a date.
pub fn parse_date(s: &str) -> Option<NaiveDate> {
    let first = s.trim().split(|c: char| c.is_whitespace() || c == '@' || c == ',').next()?;
    if let Ok(d) = NaiveDate::parse_from_str(first, "%Y-%m-%d") {
        return Some(d);
    }
    let parts: Vec<&str> = first.split(|c| c == '/' || c == '.' || c == '-').collect();
    if parts.len() != 3 {
        return None;
    }
    let (m, d, y): (u32, u32, i32) = (parts[0].parse().ok()?, parts[1].parse().ok()?, parts[2].parse().ok()?);
    let y = match y {
        0..=69 => 2000 + y,
        70..=99 => 1900 + y,
        _ => y,
    };
    NaiveDate::from_ymd_opt(y, m, d)
}

/// "Marsh, Isaiah" -> "Isaiah Marsh" (names without a comma are left alone);
/// all-lowercase words get a capital ("larry" -> "Larry").
fn first_last(name: &str) -> String {
    let n = match name.split_once(',') {
        Some((last, first)) => format!("{} {}", first.trim(), last.trim()),
        None => name.trim().to_string(),
    };
    n.split_whitespace()
        .map(|w| {
            if w.chars().all(|c| !c.is_uppercase()) {
                let mut cs = w.chars();
                cs.next().map(|f| f.to_uppercase().collect::<String>() + cs.as_str()).unwrap_or_default()
            } else {
                w.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Edit distance, counting a swap of two neighbouring letters as one edit.
fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in 0..=a.len() {
        d[i][0] = i;
    }
    for j in 0..=b.len() {
        d[0][j] = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

/// Finds the employee a driver name refers to: the same full name; else the
/// only employee with the same last name and first initial; else the only one
/// with the same first name and a last name off by one letter (or two swapped).
pub fn match_driver(name: &str, employees: &[Employee]) -> Option<i64> {
    let full = first_last(name).to_lowercase();
    let full = full.split_whitespace().collect::<Vec<_>>().join(" ");
    if full.is_empty() {
        return None;
    }
    if let Some(e) = employees.iter().find(|e| e.name.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ") == full) {
        return Some(e.id);
    }
    let words: Vec<&str> = full.split_whitespace().collect();
    let (first, last) = (words.first()?, words.last()?);
    if words.len() < 2 {
        return None;
    }
    let initial = first.chars().next()?;
    let hits: Vec<&Employee> = employees
        .iter()
        .filter(|e| {
            let n = e.name.to_lowercase();
            let w: Vec<&str> = n.split_whitespace().collect();
            w.len() >= 2 && w.last() == Some(last) && w[0].starts_with(initial)
        })
        .collect();
    if hits.len() == 1 {
        return Some(hits[0].id);
    }
    let close: Vec<&Employee> = employees
        .iter()
        .filter(|e| {
            let n = e.name.to_lowercase();
            let w: Vec<&str> = n.split_whitespace().collect();
            w.len() >= 2 && w[0] == *first && w.last().map_or(false, |l| l.len() >= 4 && distance(l, last) <= 1)
        })
        .collect();
    (close.len() == 1).then(|| close[0].id)
}

/// Reads the sheet and lines each truck up with the app's vehicles and employees.
pub fn plan(tsv: &str, existing: &[Vehicle], employees: &[Employee]) -> Result<Vec<Planned>, String> {
    let mut lines = tsv.lines();
    let header: Vec<String> = lines
        .next()
        .ok_or("The file is empty.")?
        .trim_start_matches('\u{feff}')
        .split('\t')
        .map(|h| h.trim().to_lowercase())
        .collect();
    let col = |names: &[&str]| names.iter().find_map(|n| header.iter().position(|h| h == n));
    let need = |names: &[&str]| col(names).ok_or_else(|| format!("No “{}” column in the header row.", names[0]));
    let c_truck = need(&["truck#", "truck #", "number"])?;
    let c = |names: &[&str]| col(names);
    let (c_buy, c_year, c_make, c_body) = (c(&["purchase date"]), c(&["year"]), c(&["make", "model"]), c(&["body type"]));
    let (c_plate, c_tab_exp, c_tab, c_vin, c_title) =
        (c(&["plate"]), c(&["tab exp", "tab exp date"]), c(&["tab#", "tab #"]), c(&["vin#", "vin number", "vin"]), c(&["title"]));
    let (c_g2g, c_g2g2, c_gps, c_cam) = (c(&["g2g pass info"]), c(&["l"]), c(&["samsara gps"]), c(&["dash cam sn"]));
    let (c_driver, c_phone, c_sup) =
        (c(&["driver name"]), c(&["driver phone #", "driver  phone number", "driver phone number"]), c(&["supervisor"]));
    let (c_miles, c_oil, c_log) = (c(&["milage", "mileage"]), c(&["last fluid date"]), c(&["fluid log", "fluids"]));
    let detail_cols: Vec<(&str, Option<usize>)> = vec![
        ("oil_due", c(&["due"])),
        ("brakes", c(&["brakes"])),
        ("tires", c(&["tires"])),
        ("tire_size", c(&["tire size"])),
        ("tune", c(&["tune"])),
        ("battery", c(&["battery"])),
        ("belts", c(&["belts", "belt"])),
        ("other", c(&["other"])),
        ("recalls", c(&["recalls"])),
    ];

    let by_unit: HashMap<String, &Vehicle> =
        existing.iter().map(|v| (v.unit.trim().to_lowercase(), v)).collect();
    let mut out = Vec::new();
    for line in lines {
        let cells: Vec<&str> = line.split('\t').collect();
        let get = |i: Option<usize>| i.and_then(|i| cells.get(i)).map(|s| s.trim()).unwrap_or("");
        let unit = get(Some(c_truck)).trim_start_matches('#').to_string();
        // Rows with only a truck # are unused numbers.
        let filled = cells.iter().enumerate().filter(|(i, s)| *i != c_truck && !s.trim().is_empty()).count();
        if unit.is_empty() || filled == 0 {
            continue;
        }
        let prior = by_unit.get(&unit.to_lowercase()).copied();
        let mut v = prior.cloned().unwrap_or_else(Vehicle::new);
        let mut notes = Vec::new();
        v.unit = unit.clone();
        v.year = get(c_year).to_string();
        let make_model = title_case(get(c_make));
        let (make, model) = make_model.split_once(' ').unwrap_or((make_model.as_str(), ""));
        v.make = make.to_string();
        v.model = model.to_string();
        v.kind = title_case(get(c_body));
        let (mut plate, mut tab) = (get(c_plate).to_string(), get(c_tab).to_string());
        // A tab # ends in "PF"; if the two look swapped, swap them back.
        if plate.to_uppercase().ends_with("PF") && !tab.to_uppercase().ends_with("PF") && !tab.is_empty() {
            std::mem::swap(&mut plate, &mut tab);
            notes.push(format!("plate and tab # looked swapped in the sheet; using plate {plate}, tab # {tab}"));
        }
        v.plate = plate;
        v.vin = get(c_vin).to_uppercase();
        v.purchase_date = parse_date(get(c_buy)).or(v.purchase_date);
        v.registration_due = parse_date(get(c_tab_exp)).or(v.registration_due);
        if let Ok(m) = get(c_miles).replace(',', "").parse::<i64>() {
            v.odometer = m;
        }
        v.last_oil_change = parse_date(get(c_oil)).or(v.last_oil_change);
        if !get(c_oil).is_empty() && parse_date(get(c_oil)).is_none() {
            notes.push(format!("last fluid date “{}” isn't a date; left blank", get(c_oil)));
        }

        let mut set = |key: &str, val: &str| {
            let val = val.trim();
            if val.is_empty() {
                v.info.remove(key);
            } else {
                v.info.insert(key.to_string(), val.to_string());
            }
        };
        set("tab_number", &tab);
        let title = get(c_title);
        set("title", if title.eq_ignore_ascii_case("yes") { "Yes" } else { title });
        set("toll_pass", if get(c_g2g).is_empty() { get(c_g2g2) } else { get(c_g2g) });
        set("gps", get(c_gps));
        set("dashcam", get(c_cam));
        set("driver_phone", get(c_phone));
        set("supervisor", &first_last(get(c_sup)));
        set("oil_log", get(c_log));
        for (key, ci) in &detail_cols {
            set(key, get(*ci));
        }

        // Drivers: "Spare" marks a spare vehicle; real names link to employees when they match.
        let driver = get(c_driver);
        if driver.to_lowercase().starts_with("spare") {
            v.status = "Spare".into();
            v.driver_id = None;
            v.info.remove("driver");
            if !driver.eq_ignore_ascii_case("spare") && !v.notes.contains(driver) {
                v.notes = if v.notes.trim().is_empty() { driver.to_string() } else { format!("{}\n{driver}", v.notes) };
            }
        } else {
            if v.status == "Spare" {
                v.status = "Active".into();
            }
            if driver.is_empty() {
                v.info.remove("driver");
                v.driver_id = None;
            } else {
                match match_driver(driver, employees) {
                    Some(id) => {
                        let e = employees.iter().find(|e| e.id == id).unwrap();
                        v.driver_id = Some(id);
                        v.info.insert("driver".into(), e.name.clone());
                        if !first_last(driver).eq_ignore_ascii_case(e.name.trim()) {
                            notes.push(format!("driver “{driver}” matched to employee {}", e.name));
                        }
                    }
                    None => {
                        v.driver_id = None;
                        v.info.insert("driver".into(), first_last(driver));
                        notes.push(format!("driver “{driver}” isn't an employee in the app; kept as text"));
                    }
                }
            }
        }
        out.push(Planned { vehicle: v, existing: prior.map(|p| p.id), notes });
    }
    Ok(out)
}

// ------------------------------------------------------------ GPS mileage

/// Result of reading a GPS asset report (`#9 - Adam Sorensen,CHEVROLET,SILVERADO HD,23653.8`).
#[derive(Default, Debug)]
pub struct MileagePlan {
    /// Vehicles with their new odometer reading set, ready to save.
    pub updates: Vec<Vehicle>,
    /// (vehicle, old miles, new miles) for every match.
    pub changes: Vec<(String, i64, i64)>,
    /// Report rows whose truck # isn't a vehicle in the app.
    pub unmatched: Vec<String>,
    /// Report rows with a truck # but no mileage.
    pub no_mileage: Vec<String>,
    /// Truck #s listed more than once (the highest mileage is used).
    pub duplicates: Vec<String>,
}

/// Splits one CSV line, honouring "quoted, fields".
pub fn csv_fields(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out.into_iter().map(|s| s.trim().to_string()).collect()
}

/// The truck number in "#9 - Adam Sorensen" (None if there's no "#digits").
fn truck_number(s: &str) -> Option<String> {
    let after = &s[s.find('#')? + 1..];
    let digits: String = after.trim_start().chars().take_while(|c| c.is_ascii_digit()).collect();
    (!digits.is_empty()).then(|| digits.trim_start_matches('0').to_string()).map(|d| if d.is_empty() { "0".into() } else { d })
}

/// Matches each report row to a vehicle by truck # and sets its mileage
/// (rounded to whole miles). `as_of` is recorded as when it was updated.
pub fn plan_mileage(csv: &str, vehicles: &[Vehicle], as_of: NaiveDate) -> MileagePlan {
    let mut plan = MileagePlan::default();
    // Truck # -> (row label, miles), keeping the highest reading per truck.
    let mut best: Vec<(String, String, i64)> = Vec::new();
    for line in csv.lines() {
        let f = csv_fields(line.trim_start_matches('\u{feff}'));
        let label = f.first().cloned().unwrap_or_default();
        if f.iter().all(|s| s.is_empty()) {
            continue;
        }
        // Mileage is the last number on the row.
        let miles = f.iter().skip(1).rev().find_map(|s| s.replace(',', "").parse::<f64>().ok());
        let Some(num) = truck_number(&label) else {
            // A header row has no number and no mileage; anything else is worth reporting.
            if miles.is_some() {
                plan.unmatched.push(format!("{label} (no truck #)"));
            }
            continue;
        };
        let Some(miles) = miles else {
            plan.no_mileage.push(label);
            continue;
        };
        let miles = miles.round() as i64;
        match best.iter_mut().find(|(n, _, _)| *n == num) {
            Some(entry) => {
                plan.duplicates.push(format!("#{num} is listed more than once; used {} mi", miles.max(entry.2)));
                if miles > entry.2 {
                    entry.1 = label;
                    entry.2 = miles;
                }
            }
            None => best.push((num, label, miles)),
        }
    }
    for (num, label, miles) in best {
        match vehicles.iter().find(|v| v.unit.trim().trim_start_matches('#').trim_start_matches('0') == num) {
            Some(v) => {
                plan.changes.push((format!("{} {}", v.label(), v.get("driver")).trim().to_string(), v.odometer, miles));
                let mut v = v.clone();
                v.odometer = miles;
                v.info.insert("odometer_updated".into(), format!("{} (GPS report)", as_of.format("%m/%d/%Y")));
                plan.updates.push(v);
            }
            None => plan.unmatched.push(label),
        }
    }
    plan
}

/// Runs the command-line import (see the module docs). Returns the exit code.
pub fn run(mut db: Db, path: &str, apply: bool) -> i32 {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Can't read {path}: {e}");
            return 1;
        }
    };
    let (vehicles, employees) = match (db.load_vehicles(), db.load_employees()) {
        (Ok(v), Ok(e)) => (v, e),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("Can't read the database: {e}");
            return 1;
        }
    };
    let planned = match plan(&text, &vehicles, &employees) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let (mut added, mut updated, mut linked) = (0, 0, 0);
    for p in &planned {
        let v = &p.vehicle;
        if p.existing.is_some() {
            updated += 1;
        } else {
            added += 1;
        }
        if v.driver_id.is_some() {
            linked += 1;
        }
        println!(
            "{:<5} {:<6} {:<4} {:<26} {:<9} {:<22} {:>7} mi  tab {}  {}",
            v.label(),
            if p.existing.is_some() { "update" } else { "add" },
            v.year,
            format!("{} {}", v.make, v.model),
            v.plate,
            v.get("driver"),
            v.odometer,
            v.registration_due.map(|d| d.to_string()).unwrap_or_default(),
            if v.status == "Active" { String::new() } else { v.status.clone() },
        );
        for n in &p.notes {
            println!("        - {n}");
        }
    }
    println!("\n{} vehicles: {added} to add, {updated} to update; {linked} drivers linked to employees.", planned.len());
    if !apply {
        println!("Nothing saved. Run again with --apply to save.");
        return 0;
    }
    match db.save_vehicles(&planned.iter().map(|p| p.vehicle.clone()).collect::<Vec<_>>()) {
        Ok(()) => {
            println!("Saved.");
            0
        }
        Err(e) => {
            eprintln!("Import failed, nothing was saved: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn dates_in_many_shapes() {
        assert_eq!(parse_date("2026-01-31"), Some(d("2026-01-31")));
        assert_eq!(parse_date("07/15/26 @7648"), Some(d("2026-07-15")));
        assert_eq!(parse_date("09.26.26"), Some(d("2026-09-26")));
        assert_eq!(parse_date("1/26/2023"), Some(d("2023-01-26")));
        assert_eq!(parse_date("New battery 1/26/23"), None);
        assert_eq!(parse_date(""), None);
    }

    fn emp(id: i64, name: &str) -> Employee {
        let mut e = Employee::new();
        e.id = id;
        e.name = name.into();
        e
    }

    #[test]
    fn drivers_match_employees() {
        let emps = vec![emp(1, "Isaiah Marsh"), emp(2, "Darrel Whitfield"), emp(3, "Chris Gomez"), emp(4, "Chris Abbott")];
        assert_eq!(match_driver("Marsh, Isaiah", &emps), Some(1));
        assert_eq!(match_driver("marsh,  isaiah ", &emps), Some(1));
        // Spelled differently, but the only Whitfield starting with D.
        assert_eq!(match_driver("Whitfield, Darryl", &emps), Some(2));
        assert_eq!(match_driver("Peasron, Chris", &emps), None);
        assert_eq!(match_driver("", &emps), None);
        // Same first name, last name off by one letter or two swapped letters.
        let emps = vec![emp(5, "Ravi Castellano"), emp(6, "Chris Pearson"), emp(8, "Chris Abbott")];
        assert_eq!(match_driver("Castelano, Ravi", &emps), Some(5));
        assert_eq!(match_driver("Peasron, Chris", &emps), Some(6));
        assert_eq!(match_driver("Parsons, Chris", &emps), None);
        assert_eq!(first_last("Kessler, larry"), "Larry Kessler");
        assert_eq!(first_last("Mcbride, Zach"), "Zach Mcbride");
    }

    const SHEET: &str = "Purchase Date\tYear\tMake\tBody Type\tTruck#\tPlate\tTab Exp\tTab#\tVin#\tTitle\tG2G pass info\tL\tSamsara GPS\tDash Cam SN\tDriver Name\tDriver phone #\tSupervisor\tMilage\tLast Fluid Date\tFluid Log\tDue\tBrakes\tTires\tTire Size\tTune\tBattery\tBelts\tOther\tRecalls
2025-06-02\t2024\tCHEVY SILVERADO 2500\tSERVICE BODY 2 DOOR\t1\tTST1001\t2026-01-31\t900001PF\t1gctest0vin000001\tYES\t\t77 0000000001\tGABC\tGDEF\tMarsh, Isaiah\t\tPrescott, Dustin\t57863\t07/15/26 @7648\t04/25/24 @ 1420\t\t\t4 new tires\t245/75/R17\t\t\t\tlatch recall\t
\t\t\t\t31\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t
\t2023\tFORD TRANSIT 250\tCARGO VAN\t55\t900002PF\t2026-01-31\tTST2002\tVIN55\tYes\t770000000002\t\t\t\tSpare 5 OLD\t\t\t77688\t11.03.26\t\t\t\t\t\t\t\t\t\t";

    #[test]
    fn plans_the_truck_list() {
        let emps = vec![emp(7, "Isaiah Marsh")];
        let mut old = Vehicle::new();
        old.id = 9;
        old.unit = "55".into();
        old.crew_id = Some(3);
        old.notes = "keep me".into();
        let p = plan(SHEET, &[old], &emps).unwrap();
        assert_eq!(p.len(), 2, "the empty #31 row is skipped");

        let v = &p[0].vehicle;
        assert_eq!(p[0].existing, None);
        assert_eq!((v.unit.as_str(), v.year.as_str(), v.make.as_str(), v.model.as_str()), ("1", "2024", "Chevy", "Silverado 2500"));
        assert_eq!(v.kind, "Service Body 2 Door");
        assert_eq!(v.vin, "1GCTEST0VIN000001");
        assert_eq!(v.purchase_date, Some(d("2025-06-02")));
        assert_eq!(v.registration_due, Some(d("2026-01-31")));
        assert_eq!(v.last_oil_change, Some(d("2026-07-15")));
        assert_eq!(v.odometer, 57863);
        assert_eq!(v.driver_id, Some(7));
        assert_eq!(v.get("driver"), "Isaiah Marsh");
        assert_eq!(v.get("supervisor"), "Dustin Prescott");
        assert_eq!(v.get("title"), "Yes");
        assert_eq!(v.get("toll_pass"), "77 0000000001", "falls back to the second pass column");
        assert_eq!(v.get("tab_number"), "900001PF");
        assert_eq!((v.get("tires"), v.get("tire_size"), v.get("other")), ("4 new tires", "245/75/R17", "latch recall"));
        assert_eq!(v.status, "Active");

        // #55 updates the existing vehicle: plate/tab un-swapped, spare, crew and notes kept.
        let v = &p[1].vehicle;
        assert_eq!(p[1].existing, Some(9));
        assert_eq!(v.id, 9);
        assert_eq!((v.plate.as_str(), v.get("tab_number")), ("TST2002", "900002PF"));
        assert_eq!(v.status, "Spare");
        assert_eq!(v.get("driver"), "");
        assert_eq!(v.crew_id, Some(3));
        assert_eq!(v.notes, "keep me\nSpare 5 OLD");
        assert_eq!(v.last_oil_change, Some(d("2026-11-03")));
        assert!(p[1].notes.iter().any(|n| n.contains("swapped")));
    }

    #[test]
    fn gps_mileage_matches_by_truck_number() {
        let mk = |id: i64, unit: &str, miles: i64| {
            let mut v = Vehicle::new();
            v.id = id;
            v.unit = unit.into();
            v.odometer = miles;
            v
        };
        let vehicles = vec![mk(1, "9", 20_000), mk(2, "2", 10_000), mk(3, "8", 80_000), mk(4, "46", 5)];
        let csv = "Asset,Make,Model,Odometer
#9 - Adam Sorensen,CHEVROLET,SILVERADO HD,23653.8
#8 -Taylor S,CHEVROLET,SILVERADO HD,73366.48
#2 - Mason Deluca,FORD,TRANSIT,12953.73
#2 - Mason Deluca,FORD,TRANSIT,22642.02
#63 - Spare 7,,,225360.15
#46 - Spare - OBD PORT NOT WORKING,CHEVROLET,EXPRESS,
\"#40 - Ward, Clyde\",CHEVROLET,SILVERADO,\"176,957.82\"

";
        let p = plan_mileage(csv, &vehicles, d("2026-11-13"));
        let miles: Vec<(i64, i64)> = p.updates.iter().map(|v| (v.id, v.odometer)).collect();
        assert_eq!(miles, vec![(1, 23654), (3, 73366), (2, 22642)]);
        assert_eq!(p.updates[0].get("odometer_updated"), "11/13/2026 (GPS report)");
        assert_eq!(p.unmatched, vec!["#63 - Spare 7", "#40 - Ward, Clyde"]);
        assert_eq!(p.no_mileage, vec!["#46 - Spare - OBD PORT NOT WORKING"]);
        assert_eq!(p.duplicates.len(), 1);
        // #8's reading went down; the change list shows old and new.
        assert!(p.changes.contains(&("#8".to_string(), 80_000, 73366)));
    }

    #[test]
    fn import_saves_all_vehicles() {
        let mut db = Db::open_in_memory().unwrap();
        let p = plan(SHEET, &[], &[]).unwrap();
        db.save_vehicles(&p.iter().map(|p| p.vehicle.clone()).collect::<Vec<_>>()).unwrap();
        let got = db.load_vehicles().unwrap();
        assert_eq!(got.iter().map(|v| v.label()).collect::<Vec<_>>(), vec!["#1", "#55"]);
        assert_eq!(got[0].get("driver"), "Isaiah Marsh");
        // Importing again updates rather than duplicating.
        let p = plan(SHEET, &got, &[]).unwrap();
        assert!(p.iter().all(|p| p.existing.is_some()));
        db.save_vehicles(&p.iter().map(|p| p.vehicle.clone()).collect::<Vec<_>>()).unwrap();
        assert_eq!(db.load_vehicles().unwrap().len(), 2);
    }
}
