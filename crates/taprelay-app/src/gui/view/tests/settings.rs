use super::support::{open, open_sized, settle};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Role, Task, TestAppContext};
use std::time::Instant;

#[gpui_kit::test]
fn settings_switch_between_catppuccin_themes_and_follow_system(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        let theme = gpui_kit::component::Theme::global(cx);
        assert_eq!(
            theme.theme_name().as_ref(),
            if view.read(cx).ui.dark {
                "Catppuccin Macchiato"
            } else {
                "Catppuccin Latte"
            }
        );
        window.render_frame(cx);
        window.click(("nav", 3usize), cx);
        for (choice, system_dark, name, background, primary) in [
            (2, false, "Catppuccin Macchiato", 0x1e2030, 0x8aadf4),
            (1, true, "Catppuccin Latte", 0xe5e9ef, 0x435ac4),
            (0, true, "Catppuccin Macchiato", 0x1e2030, 0x8aadf4),
            (0, false, "Catppuccin Latte", 0xe5e9ef, 0x435ac4),
        ] {
            view.update(cx, |view, _| {
                view.controller.system_theme = system_dark;
                view.controller.last_system_poll = Instant::now();
            });
            window.click(("theme", choice as usize), cx);
            let theme = gpui_kit::component::Theme::global(cx);
            assert_eq!(view.read(cx).ui.theme_choice, choice);
            assert_eq!(theme.theme_name().as_ref(), name);
            assert_eq!(theme.background, gpui_kit::rgb(background).into());
            assert_eq!(theme.primary, gpui_kit::rgb(primary).into());
            assert_eq!(theme.tokens.background, theme.background.into());
            assert_eq!(theme.button_primary, theme.primary);
            assert_eq!(theme.title_bar, theme.sidebar);
            assert!(theme.sidebar.l > theme.background.l);
            for background in [theme.title_bar, theme.secondary] {
                let hovered = background.blend(theme.secondary_hover);
                assert!(
                    (hovered.l - background.l).abs() > 0.02,
                    "secondary hover must remain visible on title bars and buttons in {name}"
                );
            }
            let color = theme.group_box.to_rgb();
            assert_eq!(
                view.read(cx).ui.overlay_background,
                [
                    (color.r * 255.) as u8,
                    (color.g * 255.) as u8,
                    (color.b * 255.) as u8,
                ]
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn settings_controls_update_their_own_preferences_and_persist(cx: &mut TestAppContext) {
    let (handle, view, folder) = open_sized(cx, 900., 2200.);
    cx.update_window(handle, |_, window, cx| {
        cx.set_reduce_motion(true);
        window.render_frame(cx);
        window.click(("nav", 3usize), cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    let mut expected = cx.update(|cx| {
        serde_json::to_value(&view.read(cx).controller.runtime.config.options).unwrap()
    });
    for (id, field) in [
        ("overlay", "passthrough_overlay"),
        ("reverse-scroll", "passthrough_reverse_scroll"),
        ("autostart", "autostart"),
        ("start-hidden", "start_hidden"),
        ("auto-listen", "auto_listen"),
        ("close-to-tray", "close_to_tray"),
        ("always-admin", "always_admin"),
        ("wait-warning", "connection_wait_warning"),
        ("notifications", "notifications"),
    ] {
        let before = expected[field].as_bool().unwrap();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(window.find(id).checked(), Some(before));
            assert!(
                window.find(id).visible(),
                "{id} is outside the settings page"
            );
            assert!(window.find(id).bounds().right() <= window.viewport_size().width);
            window.click(id, cx);
        })
        .unwrap();
        settle(cx, handle, &view);
        expected[field] = (!before).into();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(window.find(id).checked(), Some(!before));
            assert_eq!(
                serde_json::to_value(&view.read(cx).controller.runtime.config.options).unwrap(),
                expected,
                "{id} changed another preference"
            );
            view.update(cx, |view, _| view.controller.flush(true));
        })
        .unwrap();
        let saved = crate::config::Config::load(&folder.path().join("config.json")).unwrap();
        assert_eq!(serde_json::to_value(saved.options).unwrap(), expected);
    }
    let report_rate = cx.update(|cx| view.read(cx).report_rate.clone());
    let combobox_id = ("multi-combo-box", report_rate.entity_id());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let control = window.find("report-rate");
        assert_eq!(window.find(combobox_id).role(), Some(Role::ComboBox));
        assert_eq!(
            control.bounds().right(),
            window.find("overlay").bounds().right()
        );
        assert_eq!(
            control.label(),
            Some(
                view.read(cx)
                    .tr("settings.passthrough.mouse.report.rate")
                    .as_str()
            )
        );
        report_rate.update(cx, |state, cx| state.focus(window, cx));
    })
    .unwrap();
    for (index, rate) in [60usize, 125, 250, 500, 1000, 0].into_iter().enumerate() {
        cx.update_window(handle, |_, window, cx| {
            window.press("enter", cx);
            assert_eq!(window.find(combobox_id).expanded(), Some(true));
            window.press("down", cx);
            if index == 0 {
                window.press("down", cx);
            }
            window.press("enter", cx);
        })
        .unwrap();
        settle(cx, handle, &view);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(window.find(combobox_id).expanded(), Some(false));
            assert_eq!(window.find(combobox_id).focused(), Some(true));
            assert_eq!(report_rate.read(cx).selected_values(), [rate.to_string()]);
            assert_eq!(view.read(cx).ui.passthrough_mouse_report_rate, rate as i32);
            view.update(cx, |view, _| view.controller.flush(true));
        })
        .unwrap();
        let saved = crate::config::Config::load(&folder.path().join("config.json")).unwrap();
        assert_eq!(saved.options.passthrough_mouse_report_rate, rate as u16);
    }
    cx.update_window(handle, |_, window, cx| {
        window.click(("language", 2usize), cx);
        assert_eq!(view.read(cx).ui.locale, "zh-CN");
        assert_eq!(report_rate.read(cx).selection()[0].1.label, "自动");
        assert!(window.find("overlay").visible());
        assert!(window.find("overlay").bounds().right() <= window.viewport_size().width);
        assert_eq!(
            window.find("overlay").label(),
            Some(view.read(cx).tr("settings.passthrough_overlay").as_str())
        );
        view.update(cx, |view, _| view.controller.flush(true));
    })
    .unwrap();
    let saved = crate::config::Config::load(&folder.path().join("config.json")).unwrap();
    assert_eq!(saved.options.language.locale(), "zh-CN");
    assert_eq!(saved.options.passthrough_mouse_report_rate, 0);
}

#[gpui_kit::test]
fn report_rate_combobox_syncs_existing_preferences_and_escape_keeps_selection(
    cx: &mut TestAppContext,
) {
    let (handle, view, _folder) = open_sized(cx, 900., 1200.);
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        cx.set_reduce_motion(true);
        let report_rate = view.read(cx).report_rate.clone();
        view.update(cx, |view, cx| {
            view._pump = Task::ready(());
            view.controller
                .runtime
                .config
                .options
                .passthrough_mouse_report_rate = 500;
            view.controller.sync(&mut view.ui);
            view.effects(window, cx);
        });
        window.render_frame(cx);
        window.click(("nav", 3usize), cx);
        assert_eq!(report_rate.read(cx).selected_values(), ["500"]);
        assert_eq!(report_rate.read(cx).selection()[0].1.label, "500 Hz");
        window.click("report-rate", cx);
        let id = ("multi-combo-box", report_rate.entity_id());
        assert_eq!(window.find(id).expanded(), Some(true));
        view.update(cx, |view, cx| view.effects(window, cx));
        window.press("down", cx);
        window.press("escape", cx);
        assert_eq!(window.find(id).expanded(), Some(false));
        assert_eq!(window.find(id).focused(), Some(true));
        assert_eq!(report_rate.read(cx).selected_values(), ["500"]);
        assert_eq!(view.read(cx).ui.passthrough_mouse_report_rate, 500);
        assert_eq!(
            view.read(cx).report_rate.entity_id(),
            report_rate.entity_id()
        );
    })
    .unwrap();
}
