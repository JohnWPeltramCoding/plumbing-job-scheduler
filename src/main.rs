// Hide the console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod attendance;
mod auth;
mod calendar;
mod crews;
mod driver;
mod db;
mod employee_import;
mod employees;
mod fleet_import;
mod job_editor;
mod job_import;
mod model;
mod users;
mod vehicles;

use eframe::egui;

fn main() -> eframe::Result<()> {
    // `--import-fleet <file.tsv> [--apply]` / `--import-headcount <file.csv> [--apply]` /
    // `--import-jobs <file.csv> [--update-scheduling] [--apply]`: import, then exit.
    let args: Vec<String> = std::env::args().collect();
    for flag in ["--import-fleet", "--import-headcount", "--import-jobs"] {
        let Some(i) = args.iter().position(|a| a == flag) else { continue };
        let Some(path) = args.get(i + 1) else {
            eprintln!("Usage: plumbing-scheduler {flag} <file> [--apply]");
            std::process::exit(2);
        };
        let (db, warning) = db::Db::open_default();
        if let Some(w) = warning {
            eprintln!("{w}");
            std::process::exit(1);
        }
        println!("Database: {}", db.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default());
        let apply = args.iter().any(|a| a == "--apply");
        std::process::exit(match flag {
            "--import-fleet" => fleet_import::run(db, path, apply),
            "--import-headcount" => employee_import::run(db, path, apply),
            _ => job_import::run(db, path, args.iter().any(|a| a == "--update-scheduling"), apply),
        });
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Plumbing Job Scheduler")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([960.0, 620.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Plumbing Job Scheduler",
        options,
        Box::new(|cc| Ok(Box::new(app::SchedulerApp::new(cc)))),
    )
}
