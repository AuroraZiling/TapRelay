use crate::gui::view::AppView;
use crate::{
    config::{Config, Geometry},
    gui::controller::{Controller, Shell},
    runtime::Runtime,
    runtime::worker::RuntimeHandle,
};
use gpui_kit::component::TitleBar;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, TestAppContext, WindowBounds, WindowOptions, px,
    size,
};
use std::time::{Duration, Instant};

pub(super) mod receiver;

pub(super) fn open(
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<AppView>, tempfile::TempDir) {
    let geometry = Geometry::default();
    open_sized(cx, geometry.width, geometry.height)
}

pub(super) fn open_sized(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (AnyWindowHandle, Entity<AppView>, tempfile::TempDir) {
    open_with_transport(cx, width, height, None)
}

pub(super) fn open_with_transport(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
    transport: Option<Box<dyn crate::platform::Transport + Send>>,
) -> (AnyWindowHandle, Entity<AppView>, tempfile::TempDir) {
    let mut config = Config::default();
    config.options.language = crate::config::Language::from_name("en").unwrap();
    config.options.notifications = false;
    config.wizard.dismissed = true;
    config
        .functions
        .get_mut(&taprelay_core::function::FunctionId::MediaPlayPause)
        .unwrap()
        .enabled = true;
    open_with_config(cx, width, height, config, transport)
}

pub(super) fn open_with_config(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
    config: Config,
    transport: Option<Box<dyn crate::platform::Transport + Send>>,
) -> (AnyWindowHandle, Entity<AppView>, tempfile::TempDir) {
    cx.update(gpui_kit::init);
    let folder = tempfile::tempdir().unwrap();
    let runtime = RuntimeHandle::spawn(
        config,
        move |config| {
            let mut runtime = Runtime::new(config);
            runtime.state.adapter = true;
            runtime.state.peripheral = true;
            runtime.state.service = true;
            runtime.state.broadcasting = true;
            if let Some(transport) = transport {
                runtime.set_headless_transport(transport);
            }
            runtime.headless_tick();
            runtime
        },
        Runtime::headless_tick,
        64,
    )
    .unwrap();
    let mut controller = Controller::new(
        runtime,
        folder.path().join("config.json"),
        None,
        Shell::Headless {
            autostart: false,
            running_apps: Vec::new(),
            notifications: Default::default(),
        },
        Default::default(),
        None,
    )
    .unwrap();
    controller.passed = true;
    controller.hidden = false;
    controller.startup_pending = false;
    let (handle, view) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Default::default(),
                    size(px(width), px(height)),
                ))),
                ..TitleBar::window_options()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    let mut view = AppView::new(controller, Default::default(), window, cx);
                    view.ui.mode = 2;
                    view
                })
            },
        )
        .unwrap()
    });
    (handle, view, folder)
}

pub(super) fn settle(cx: &mut TestAppContext, handle: AnyWindowHandle, view: &Entity<AppView>) {
    wait_until(cx, handle, view, |view| !view.controller.runtime.busy());
}

pub(super) fn wait_until(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    view: &Entity<AppView>,
    done: impl Fn(&AppView) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let finished = cx
            .update_window(handle, |_, window, cx| {
                view.update(cx, |view, cx| {
                    view.controller.runtime.poll();
                    view.controller.sync(&mut view.ui);
                    view.effects(window, cx);
                    cx.notify();
                    let finished = done(view);
                    if !finished && !view.controller.runtime.busy() {
                        view.controller.runtime.tick();
                    }
                    finished
                })
            })
            .unwrap();
        if finished {
            break;
        }
        assert!(Instant::now() < deadline, "worker did not settle");
        std::thread::sleep(Duration::from_millis(5));
    }
}
