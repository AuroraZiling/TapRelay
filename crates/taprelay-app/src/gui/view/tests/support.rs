use crate::gui::view::AppView;
use crate::{
    config::Config,
    gui::{Controller, Shell},
    runtime::Runtime,
    runtime_worker::RuntimeHandle,
};
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, TestAppContext, WindowBounds, WindowOptions, px,
    size,
};
use std::time::{Duration, Instant};

pub(super) fn open(
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<AppView>, tempfile::TempDir) {
    open_sized(cx, 900., 500.)
}

pub(super) fn open_sized(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (AnyWindowHandle, Entity<AppView>, tempfile::TempDir) {
    cx.update(gpui_kit::init);
    let folder = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.options.language = crate::config::Language::from_name("english").unwrap();
    config.options.notifications = false;
    config.wizard.dismissed = true;
    config
        .functions
        .get_mut(&taprelay_core::function::FunctionId::MediaPlayPause)
        .unwrap()
        .enabled = true;
    let runtime = RuntimeHandle::spawn(
        config,
        |config| {
            let mut runtime = Runtime::new(config);
            runtime.state.adapter = true;
            runtime.state.peripheral = true;
            runtime.state.service = true;
            runtime.state.broadcasting = true;
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
        Shell::Headless,
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
                ..Default::default()
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
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let busy = cx
            .update_window(handle, |_, window, cx| {
                view.update(cx, |view, cx| {
                    view.controller.runtime.poll();
                    view.controller.sync(&mut view.ui);
                    view.effects(window, cx);
                    cx.notify();
                    view.controller.runtime.busy()
                })
            })
            .unwrap();
        if !busy {
            break;
        }
        assert!(Instant::now() < deadline, "worker did not settle");
        std::thread::sleep(Duration::from_millis(5));
    }
}
