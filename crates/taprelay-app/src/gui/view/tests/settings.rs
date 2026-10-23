use super::support::{open, open_sized, settle};
use crate::config::Theme;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Role, Task, TestAppContext};
use std::time::Instant;

#[cfg(target_os = "macos")]
#[gpui_kit::test]
fn notification_authorization_is_separate_from_the_preference_and_can_recover(
    cx: &mut TestAppContext,
) {
    use crate::gui::controller::Shell;
    use crate::platform::desktop::NotificationPermission;
    let (handle, view, folder) = open_sized(cx, 900., 2500.);
    cx.update_window(handle, |_, window, cx| {
        cx.set_reduce_motion(true);
        window.render_frame(cx);
        window.click(("nav", 3usize), cx);
        view.update(cx, |view, _| view._pump = Task::ready(()));
        window.click("notifications", cx);
        for (permission, key) in [
            (
                NotificationPermission::NotDetermined,
                "settings.notifications.mac.not_determined",
            ),
            (NotificationPermission::Denied, "permissions.mac.denied"),
            (NotificationPermission::Allowed, "permissions.mac.allowed"),
            (
                NotificationPermission::Quiet,
                "settings.notifications.mac.quiet",
            ),
            (NotificationPermission::Unknown, "common.status_unavailable"),
            (
                NotificationPermission::Unavailable,
                "settings.notifications.mac.unavailable",
            ),
        ] {
            view.update(cx, |view, cx| {
                if let Shell::Headless {
                    notification_permission,
                    ..
                } = &mut view.controller.desktop
                {
                    *notification_permission = permission;
                }
                view.controller.sync(&mut view.ui);
                view.controller.tick(&mut view.ui);
                view.effects(window, cx);
            });
            window.render_frame(cx);
            assert_eq!(
                window.find("notification-permission").role(),
                Some(Role::Status)
            );
            assert_eq!(
                window.find("notification-permission").label(),
                Some(view.read(cx).tr(key).as_str())
            );
            // OS rejection must not silently erase the user's preference.
            assert_eq!(window.find("notifications").checked(), Some(true));
            let saved = crate::config::Config::load(&folder.path().join("config.json")).unwrap();
            assert!(saved.options.notifications);
        }
        window.click("notification-settings", cx);
        assert!(matches!(
            view.read(cx).controller.desktop,
            Shell::Headless {
                notification_settings_opened: true,
                ..
            }
        ));
        if let Shell::Headless { notifications, .. } = &view.read(cx).controller.desktop {
            notifications.borrow_mut().clear();
        }
        window.click("test-notification", cx);
        let count = |cx: &gpui_kit::App| {
            let Shell::Headless { notifications, .. } = &view.read(cx).controller.desktop else {
                unreachable!()
            };
            notifications.borrow().len()
        };
        assert_eq!(count(cx), 1);
        if let Shell::Headless { notifications, .. } = &view.read(cx).controller.desktop {
            assert_eq!(
                notifications.borrow()[0].1,
                view.read(cx).tr("settings.notifications.mac.test_body")
            );
        }
        window.click("notifications", cx);
        window.render_frame(cx);
        assert_eq!(window.find("notifications").checked(), Some(false));
        window.click("test-notification", cx);
        assert_eq!(count(cx), 1);
        // The controller also protects against a stale enabled button.
        view.update(cx, |view, _| {
            view.controller
                .action(
                    &mut view.ui,
                    crate::gui::action::Action::TestNotification,
                    "",
                )
                .unwrap();
        });
        assert_eq!(count(cx), 1);
        window.click(("language", 2usize), cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("notification-permission").label(),
            Some("系统通知需要从 TapRelay 应用包运行。")
        );
    })
    .unwrap();
}

#[cfg(target_os = "macos")]
#[gpui_kit::test]
fn login_approval_refreshes_from_the_os_and_pending_registration_can_be_cancelled(
    cx: &mut TestAppContext,
) {
    use crate::gui::controller::Shell;
    use crate::platform::desktop::LoginStatus;
    use std::time::Duration;
    let (handle, view, folder) = open_sized(cx, 900., 2200.);
    cx.update_window(handle, |_, window, cx| {
        cx.set_reduce_motion(true);
        window.render_frame(cx);
        window.click(("nav", 3usize), cx);
        view.update(cx, |view, _| view._pump = Task::ready(()));
        for (status, checked, saved_enabled, key) in [
            (
                LoginStatus::RequiresApproval,
                true,
                false,
                "settings.autostart.mac.approval",
            ),
            (
                LoginStatus::Enabled,
                true,
                true,
                "settings.autostart.mac.enabled",
            ),
            (
                LoginStatus::Unknown,
                true,
                true,
                "common.status_unavailable",
            ),
            (
                LoginStatus::Disabled,
                false,
                false,
                "settings.autostart.mac.disabled",
            ),
            (
                LoginStatus::NotFound,
                false,
                false,
                "settings.autostart.mac.not_found",
            ),
            (
                LoginStatus::RequiresApproval,
                true,
                false,
                "settings.autostart.mac.approval",
            ),
        ] {
            view.update(cx, |view, cx| {
                if let Shell::Headless { login_status, .. } = &mut view.controller.desktop {
                    *login_status = Some(status);
                }
                view.controller.last_system_poll = Instant::now() - Duration::from_secs(2);
                view.controller.sync(&mut view.ui);
                view.controller.tick(&mut view.ui);
                view.effects(window, cx);
            });
            window.render_frame(cx);
            assert_eq!(window.find("autostart").checked(), Some(checked));
            assert_eq!(window.find("autostart-status").role(), Some(Role::Status));
            assert_eq!(
                window.find("autostart-status").label(),
                Some(view.read(cx).tr(key).as_str())
            );
            let saved = crate::config::Config::load(&folder.path().join("config.json")).unwrap();
            assert_eq!(saved.options.autostart, saved_enabled);
        }
        window.click("login-settings", cx);
        assert!(matches!(
            view.read(cx).controller.desktop,
            Shell::Headless {
                login_settings_opened: true,
                ..
            }
        ));
        window.click("autostart", cx);
        window.render_frame(cx);
        assert_eq!(window.find("autostart").checked(), Some(false));
        assert_eq!(
            view.read(cx).ui.auto_start_status,
            Some("settings.autostart.mac.disabled")
        );
        let saved = crate::config::Config::load(&folder.path().join("config.json")).unwrap();
        assert!(!saved.options.autostart);
        window.click(("language", 2usize), cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("autostart-status").label(),
            Some("登录时启动已关闭。")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn settings_switch_between_light_dark_and_system_themes_and_persist(cx: &mut TestAppContext) {
    let (handle, view, folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("nav", 3usize), cx);
        for (choice, system_dark, dark, preference) in [
            (2, false, true, Theme::Dark),
            (1, true, false, Theme::Light),
            (0, true, true, Theme::System),
            (0, false, false, Theme::System),
        ] {
            view.update(cx, |view, _| {
                view.controller.system_theme = system_dark;
                view.controller.last_system_poll = Instant::now();
            });
            window.click(("theme", choice as usize), cx);
            let theme = gpui_kit::component::Theme::global(cx);
            assert_eq!(view.read(cx).ui.theme_choice, choice);
            assert_eq!(view.read(cx).ui.dark, dark);
            assert_eq!(theme.is_dark(), dark);
            let saved = crate::config::Config::load(&folder.path().join("config.json")).unwrap();
            assert_eq!(saved.options.theme, preference);
            assert_eq!(theme.tokens.background, theme.background.into());
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
        #[cfg(windows)]
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
        assert_eq!(
            window.find("overlay").label(),
            Some(view.read(cx).tr("settings.passthrough_overlay").as_str())
        );
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

#[gpui_kit::test]
fn windows_capabilities_preserve_settings_controls_and_hide_mac_approval(cx: &mut TestAppContext) {
    let (handle, view, folder) = open_sized(cx, 900., 2200.);
    cx.update_window(handle, |_, window, cx| {
        cx.set_reduce_motion(true);
        view.update(cx, |view, _| {
            view._pump = Task::ready(());
            view.ui.capabilities = crate::platform::desktop::Capabilities {
                privacy_permissions: false,
                login_approval: false,
                notification_permission: false,
                elevation: true,
                subscription_receivers: false,
            };
            view.controller.sync(&mut view.ui);
        });
        window.render_frame(cx);
        window.click(("nav", 3usize), cx);
        window.render_frame(cx);
        for id in [
            "autostart-status",
            "login-settings",
            "notification-settings",
            "notification-permission",
            "test-notification",
        ] {
            assert!(
                window.try_find(id).is_none(),
                "Windows must not acquire {id}"
            );
        }
        assert!(window.find("settings-elevate").visible());
        let before = view.read(cx).ui.always_admin;
        window.click("always-admin", cx);
        window.render_frame(cx);
        assert_eq!(window.find("always-admin").checked(), Some(!before));
        let saved = crate::config::Config::load(&folder.path().join("config.json")).unwrap();
        assert_eq!(saved.options.always_admin, !before);
    })
    .unwrap();
}
