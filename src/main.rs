// A console window would pop up behind the app on Windows; in a debug build it
// is wanted, because that is where panics and warnings land.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod capture;
mod clipboard;
mod config;
mod editor;
mod hotkeys;
mod icon;
mod library;
mod naming;
mod overlay;
mod platform;
mod thumbs;
mod tray;
mod ui;

use std::sync::Mutex;

/// Handed to the listener thread once the window exists. Holding it here keeps
/// the claim alive between `main` and `App::new`.
pub static INSTANCE: Mutex<Option<platform::InstanceGuard>> = Mutex::new(None);

const HELP: &str = "\
Visura - screenshots for Windows and Linux

Usage:
  visura [options]

Options:
  --shot <region|window|screen>
                   take a screenshot; a running Visura does it, otherwise
                   Visura starts in the background and does it
  --background     start without a window, shortcuts and tray only
  --version        print the version
  --help           this help

Environment:
  VISURA_CONFIG=<path>   use a different configuration file
";

/// The value of `--shot`, as `--shot region` or `--shot=region`.
fn shot_argument(args: &[String]) -> Result<Option<platform::Request>, String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let value = if arg == "--shot" {
            iter.next().map(String::as_str).unwrap_or("")
        } else if let Some(value) = arg.strip_prefix("--shot=") {
            value
        } else {
            continue;
        };
        return match platform::Request::parse(value) {
            Some(platform::Request::Show) | None => Err(format!(
                "--shot takes region, window or screen, not '{value}'"
            )),
            Some(request) => Ok(Some(request)),
        };
    }
    Ok(None)
}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{HELP}");
        return Ok(());
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("visura {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let shot = match shot_argument(&args) {
        Ok(shot) => shot,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };

    let (config, _) = config::Config::load();
    // Asked for a shot only: the window stays out of the way.
    let start_hidden =
        shot.is_some() || args.iter().any(|a| a == "--background") || config.ui.start_hidden;

    // A second copy would fight over the hotkeys, so it hands over instead.
    match platform::acquire_single_instance(shot.unwrap_or(platform::Request::Show)) {
        Some(guard) => {
            if let Ok(mut slot) = INSTANCE.lock() {
                *slot = Some(guard);
            }
        }
        None => return Ok(()),
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Visura")
            .with_inner_size([1000.0, 700.0])
            .with_min_inner_size([620.0, 420.0])
            .with_visible(!start_hidden)
            .with_icon(icon::window_icon().unwrap_or_default()),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };

    eframe::run_native(
        "Visura",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, start_hidden, shot)))),
    )
}
