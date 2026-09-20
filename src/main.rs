// A console window would pop up behind the app on Windows; in a debug build it
// is wanted, because that is where panics and warnings land.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod capture;
mod clipboard;
mod config;
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
Visura – Screenshots für Windows und Linux

Aufruf:
  visura [Optionen]

Optionen:
  --background     ohne Fenster starten, nur Tastenkürzel und Taskleiste
  --version        Version ausgeben
  --help           diese Hilfe

Konfiguration:
  VISURA_CONFIG=<Pfad>   eine andere Konfigurationsdatei verwenden
";

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

    let (config, _) = config::Config::load();
    let start_hidden = args.iter().any(|a| a == "--background") || config.ui.start_hidden;

    // A second copy would fight over the hotkeys, so it hands over instead.
    match platform::acquire_single_instance() {
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
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, start_hidden)))),
    )
}
