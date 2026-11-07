#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod account;
mod app;
mod crypto;
mod db;
mod dialogs;
mod export;
mod imap;
mod mime;
mod net;
mod paths;
mod providers;
mod service;
mod smtp;
mod templates;
mod theme;

fn app_icon() -> eframe::egui::IconData {
    eframe::egui::IconData {
        rgba: include_bytes!("../assets/icon.rgba").to_vec(),
        width: 128,
        height: 128,
    }
}

fn start() -> Result<(), String> {
    let dirs = paths::Dirs::new(&paths::Dirs::default_base())?;
    let db = db::Db::open(&dirs.db_file())?;
    let crypto = crypto::Crypto::load_or_create(&dirs.key_file())?;
    let store = account::AccountStore::new(&dirs.accounts, crypto);
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(format!(
                "{} di Lorenzo D'Apolito v{}",
                app::APP_TITLE,
                app::APP_VERSION
            ))
            .with_inner_size([1200.0, 740.0])
            .with_min_inner_size([860.0, 560.0])
            .with_icon(app_icon()),
        ..Default::default()
    };
    eframe::run_native(
        app::APP_TITLE,
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, dirs, db, store, Vec::new())))),
    )
    .map_err(|e| format!("finestra: {e}"))
}

fn main() {
    if let Err(e) = start() {
        eprintln!("DoveClient non è partito: {e}");
        std::process::exit(1);
    }
}
