mod options;

use super::controller::{Controller, Shell};
use super::view::AppView;
use crate::i18n::{locale, resolve};
use crate::{
    config::{self, Config, Geometry, Language},
    i18n, logging,
    platform::{
        administrator,
        desktop::{self, Desktop},
    },
    runtime::worker::RuntimeHandle,
};
use anyhow::{Result, bail};
use gpui_kit::component::TitleBar;
use gpui_kit::{AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use options::{PrivilegeMode, parse_startup_options, should_request_admin};
use std::{cell::RefCell, rc::Rc};

pub fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let startup = parse_startup_options(&args)?;
    #[cfg(target_os = "macos")]
    if startup.privilege != PrivilegeMode::Config || startup.handoff.is_some() {
        bail!("--admin, --user and --handoff are Windows-only options");
    }
    logging::configure_level(startup.log_level)?;
    if let Some(pid) = startup.handoff {
        administrator::wait_for_parent(pid)?;
    } else if desktop::activate_existing() {
        return Ok(());
    }
    let path = config::path()?;
    let loaded = Config::load(&path);
    let status = administrator::status()?;
    let needs_admin = cfg!(windows)
        && should_request_admin(
            startup.privilege,
            loaded.as_ref().is_ok_and(|c| c.options.always_admin),
        );
    if startup.handoff.is_none() && needs_admin && !status.elevated {
        match administrator::restart() {
            Ok(true) => return Ok(()),
            Ok(false) if startup.privilege == PrivilegeMode::Admin => {
                bail!("Administrator privileges are required when starting with --admin")
            }
            Ok(false) => {}
            Err(error) => desktop::show_error(&format!(
                "Administrator request failed; continuing normally.\n{error:#}"
            )),
        }
    }
    let _instance = match crate::platform::InstanceLock::acquire() {
        Ok(instance) => instance,
        Err(error) => {
            if desktop::activate_existing() {
                return Ok(());
            }
            return Err(error.into());
        }
    };
    let mut startup_error = loaded.as_ref().err().map(|e| format!("{e:#}"));
    if let Err(error) = config::check_writable(&path) {
        startup_error = Some(format!("{error:#}"));
    }
    let mut config = loaded.unwrap_or_default();
    config.options.autostart = desktop::autostart_enabled();
    let (logs, _guard) = match logging::init(&path) {
        Ok((logs, guard)) => (Some(logs), Some(guard)),
        Err(error) => {
            startup_error = Some(format!("Cannot open logs: {error:#}"));
            (None, None)
        }
    };
    tracing::info!(
        version = crate::constants::VERSION,
        elevated = status.elevated,
        account_admin = status.account_admin,
        "TapRelay started"
    );
    let locale = match &config.options.language {
        Language::System => desktop::system_locale_id()
            .as_deref()
            .and_then(resolve)
            .unwrap_or(locale::SOURCE),
        selected => selected.locale(),
    };
    let geometry = config.window.clone();
    // GPUI creates its NSApplication subclass before native menus/status items.
    // Creating a tray item earlier would instantiate plain NSApplication.
    let make_controller = move || -> Result<Controller> {
        let desktop = Desktop::new(i18n::tray_labels(locale))?;
        let mut controller = Controller::new(
            RuntimeHandle::new(config)?,
            path,
            logs,
            Shell::Native(desktop),
            status,
            startup_error,
        )?;
        if controller.fatal.is_none() {
            controller.runtime.start_bluetooth()?;
        }
        Ok(controller)
    };
    let error = Rc::new(RefCell::new(None));
    let startup_result = error.clone();
    let application = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets);
    #[cfg(target_os = "macos")]
    application.on_reopen(|_| desktop::request_show());
    application.run(move |cx| {
        gpui_kit::init(cx);
        #[cfg(target_os = "macos")]
        install_macos_menu(locale, cx);
        #[cfg(target_os = "macos")]
        cx.on_system_notification_response(|response, _| {
            if response.tag.as_ref() == "taprelay-status" && response.action_id.is_none() {
                desktop::request_show();
            }
        });
        let controller = match make_controller() {
            Ok(controller) => controller,
            Err(error) => {
                *startup_result.borrow_mut() = Some(error);
                cx.quit();
                return;
            }
        };
        #[cfg_attr(not(windows), allow(unused_mut))]
        let mut bounds = Bounds::centered(None, window_size(&geometry), cx);
        #[cfg_attr(not(windows), allow(unused_mut))]
        let mut display_id = None;
        #[cfg(windows)]
        if let (Some(x), Some(y)) = (geometry.x, geometry.y)
            && let Some((monitor, scale)) = desktop::display_at_position(x, y)
        {
            display_id = Some(gpui_kit::DisplayId::new(monitor));
            bounds.origin = gpui_kit::point(px(x as f32 / scale), px(y as f32 / scale));
        }
        let options = WindowOptions {
            display_id,
            window_bounds: Some(if geometry.maximized {
                WindowBounds::Maximized(bounds)
            } else {
                WindowBounds::Windowed(bounds)
            }),
            titlebar: Some(TitlebarOptions {
                title: Some("TapRelay".into()),
                ..TitleBar::title_bar_options()
            }),
            window_min_size: Some(size(px(Geometry::MIN_WIDTH), px(Geometry::MIN_HEIGHT))),
            show: false,
            focus: false,
            app_id: Some("TapRelay".into()),
            ..TitleBar::window_options()
        };
        match gpui_kit::open_window(options, cx, |window, cx| {
            match window.gpu_specs() {
                Some(gpu) => tracing::info!(
                    device = %gpu.device_name,
                    software_rendering = gpu.is_software_emulated,
                    driver = %gpu.driver_name,
                    driver_info = %gpu.driver_info,
                    "Window graphics device"
                ),
                None => tracing::warn!("Window graphics device information unavailable"),
            }
            cx.new(|cx| AppView::new(controller, geometry, window, cx))
        }) {
            Ok(_) => {}
            Err(error) => {
                *startup_result.borrow_mut() = Some(error);
                cx.quit();
            }
        }
    });
    if let Some(error) = error.borrow_mut().take() {
        return Err(error);
    }
    Ok(())
}

fn window_size(geometry: &Geometry) -> gpui_kit::Size<gpui_kit::Pixels> {
    size(
        px(geometry.width.max(Geometry::MIN_WIDTH)),
        px(geometry.height.max(Geometry::MIN_HEIGHT)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_window_size_is_preserved_above_the_minimum() {
        let geometry = Geometry {
            width: 950.,
            height: 550.,
            ..Default::default()
        };

        assert_eq!(
            window_size(&geometry),
            size(px(geometry.width), px(geometry.height))
        );
    }
}

#[cfg(target_os = "macos")]
gpui_kit::actions!(
    taprelay,
    [
        MacShow,
        MacSettings,
        MacQuit,
        MacClose,
        MacHide,
        MacHideOthers,
        MacShowAll,
        MacMinimize
    ]
);
#[cfg(target_os = "macos")]
fn install_macos_menu(locale: &str, cx: &mut gpui_kit::App) {
    use gpui_kit::KeyBinding;
    cx.on_action(|_: &MacShow, _| desktop::request_show());
    cx.on_action(|_: &MacSettings, _| desktop::request_settings());
    cx.on_action(|_: &MacQuit, _| desktop::request_quit());
    cx.on_action(|_: &MacClose, _| desktop::request_close());
    cx.on_action(|_: &MacHide, cx| cx.hide());
    cx.on_action(|_: &MacHideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &MacShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &MacMinimize, cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, _| window.minimize_window());
        }
    });
    cx.bind_keys([
        KeyBinding::new("cmd-,", MacSettings, None),
        KeyBinding::new("cmd-q", MacQuit, None),
        KeyBinding::new("cmd-w", MacClose, None),
        KeyBinding::new("cmd-h", MacHide, None),
        KeyBinding::new("alt-cmd-h", MacHideOthers, None),
        KeyBinding::new("cmd-m", MacMinimize, None),
    ]);
    update_macos_menu(locale, cx);
}

#[cfg(target_os = "macos")]
pub(super) fn update_macos_menu(locale: &str, cx: &mut gpui_kit::App) {
    use gpui_kit::component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
    use gpui_kit::{Menu, MenuItem, OsAction};
    let label = |key| rust_i18n::t!(key, locale = locale).into_owned();
    cx.set_menus(vec![
        Menu::new("TapRelay").items([
            MenuItem::action(label("tray.open"), MacShow),
            MenuItem::action(label("nav.settings"), MacSettings),
            MenuItem::separator(),
            MenuItem::action(label("menu.mac.hide"), MacHide),
            MenuItem::action(label("menu.mac.hide_others"), MacHideOthers),
            MenuItem::action(label("menu.mac.show_all"), MacShowAll),
            MenuItem::separator(),
            MenuItem::action(label("common.quit"), MacQuit),
        ]),
        Menu::new(label("common.edit")).items([
            MenuItem::os_action(label("menu.mac.undo"), Undo, OsAction::Undo),
            MenuItem::os_action(label("menu.mac.redo"), Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action(label("menu.mac.cut"), Cut, OsAction::Cut),
            MenuItem::os_action(label("menu.mac.copy"), Copy, OsAction::Copy),
            MenuItem::os_action(label("menu.mac.paste"), Paste, OsAction::Paste),
            MenuItem::os_action(label("menu.mac.select_all"), SelectAll, OsAction::SelectAll),
        ]),
        Menu::new(label("menu.mac.window")).items([
            MenuItem::action(label("menu.mac.minimize"), MacMinimize),
            MenuItem::action(label("menu.mac.close"), MacClose),
            MenuItem::action(label("tray.open"), MacShow),
        ]),
    ]);
}
