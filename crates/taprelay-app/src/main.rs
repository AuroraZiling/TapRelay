#![cfg_attr(windows, windows_subsystem = "windows")]
mod action;
mod administrator;
mod capture_key;
mod config;
mod feedback;
mod gui;
mod i18n;
mod logging;
mod platform;
mod receiver_view;
mod runtime;
#[cfg(windows)]
mod window_rendering;
slint::include_modules!();
#[cfg(test)]
mod ui_tests;
fn main() {
    logging::install_panic_handler();
    if let Err(e) = gui::run() {
        // Last-resort native error surface, including failures before the renderer exists.
        platform::desktop::show_error(&format!("TapRelay: {e:#}"));
    }
}
