mod options;

use super::controller::{Controller, Shell};
use super::view::AppView;
use crate::{
    config::{self, Config, Language},
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
    logging::configure_level(startup.log_level)?;
    if let Some(pid) = startup.handoff {
        administrator::wait_for_parent(pid)?;
    } else if desktop::activate_existing() {
        return Ok(());
    }
    let path = config::path()?;
    let loaded = Config::load(&path);
    let status = administrator::status()?;
    let needs_admin = should_request_admin(
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
        Language::System => i18n::system_locale(),
        selected => selected.locale(),
    };
    let desktop = Desktop::new(i18n::tray_labels(locale))?;
    let geometry = config.window.clone();
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
    let error = Rc::new(RefCell::new(None));
    let startup_result = error.clone();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            let size = size(px(geometry.width.max(1000.)), px(geometry.height.max(600.)));
            let mut bounds = Bounds::centered(None, size, cx);
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
                window_min_size: Some(size_for_minimum()),
                show: false,
                focus: false,
                app_id: Some("TapRelay".into()),
                ..TitleBar::window_options()
            };
            match gpui_kit::open_window(options, cx, |window, cx| {
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

fn size_for_minimum() -> gpui_kit::Size<gpui_kit::Pixels> {
    size(px(900.), px(500.))
}
