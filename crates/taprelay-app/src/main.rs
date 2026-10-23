#![cfg_attr(windows, windows_subsystem = "windows")]
rust_i18n::i18n!("locales", fallback = "en");
mod config;
mod constants;
mod gui;
mod i18n;
mod logging;
mod platform;
mod runtime;

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--bluetooth-worker") {
        let args: Vec<_> = std::env::args().skip(2).collect();
        if let Err(error) = logging::init_worker(&args) {
            eprintln!("Bluetooth worker logging: {error:#}");
            std::process::exit(2);
        }
        let code = match platform::bluetooth_worker::run_child() {
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
