#![cfg_attr(windows, windows_subsystem = "windows")]
mod action;
mod administrator;
#[cfg(windows)]
mod bluetooth_worker;
mod capture_key;
mod config;
mod feedback;
mod gui;
mod i18n;
mod logging;
mod platform;
mod receiver_view;
mod runtime;
mod runtime_worker;
mod startup_checks;
mod version;
#[cfg(windows)]
mod window_rendering;
slint::include_modules!();
#[cfg(test)]
mod ui_tests;
fn main() {
    #[cfg(windows)]
    if std::env::args().nth(1).as_deref() == Some("--bluetooth-worker") {
        let _ = tracing_subscriber::fmt()
            .json()
            .with_writer(std::io::stderr)
            .try_init();
        let code = match bluetooth_worker::run_child() {
            Ok(()) => 0,
            Err(error) => {
                tracing::error!(%error, "Bluetooth worker failed");
                1
            }
        };
        std::process::exit(code);
    }
    logging::install_panic_handler();
    if let Err(e) = gui::run() {
        platform::desktop::show_error(&format!("TapRelay: {e:#}"));
    }
}
