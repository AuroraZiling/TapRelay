use super::support::{open, settle};
use crate::{
    gui::action::Action,
    gui::view::{Command, components::LiveElement},
};
use gpui_kit::base::test_support::snapshots;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Bounds, Element, Point, Role, StatefulInteractiveElement, Task, TestAppContext,
    accesskit,
};

#[gpui_kit::test]
fn sidebar_destinations_are_named_and_keyboard_operable_in_both_states(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        cx.set_reduce_motion(true);
        view.update(cx, |view, _| view._pump = Task::ready(()));
        for collapsed in [false, true] {
            view.update(cx, |view, cx| {
                view.sidebar_collapsed = collapsed;
                view.focus.focus(window, cx);
                cx.notify();
            });
            window.render_frame(cx);
            window.press("tab", cx);
            assert_eq!(window.find("collapse").focused(), Some(true));
            window.press("space", cx);
            assert_eq!(view.read(cx).sidebar_collapsed, !collapsed);
            window.press("enter", cx);
            assert_eq!(view.read(cx).sidebar_collapsed, collapsed);
            for (id, page) in [("0-0", 0), ("0-1", 1), ("0-2", 4), ("0-3", 2)] {
                window.press("tab", cx);
                assert_eq!(window.find(id).focused(), Some(true));
                assert_eq!(window.find(id).role(), Some(Role::Button));
                assert!(!window.find(id).label().unwrap().is_empty());
                window.press("enter", cx);
                assert_eq!(view.read(cx).ui.page, page);
            }
            window.press("tab", cx);
            assert_eq!(window.find(("nav", 3usize)).focused(), Some(true));
            window.press("space", cx);
            assert_eq!(view.read(cx).ui.page, 3);
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn tab_and_reverse_tab_reveal_controls_in_compact_scroll_regions(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    settle(cx, handle, &view);
    for language in ["en", "zh-CN"] {
        for page in [1, 3] {
            cx.update_window(handle, |_, window, cx| {
                view.update(cx, |view, cx| {
                    view._pump = Task::ready(());
                    view.controller.runtime.config.options.language =
                        crate::config::Language::from_name(language).unwrap();
                    view.controller.sync(&mut view.ui);
                    view.ui.page = page;
                    view.content_scroll.set_offset(Point::default());
                    view.focus.focus(window, cx);
                    cx.notify();
                });
                window.render_frame(cx);
            })
            .unwrap();
            let mut visited = std::collections::HashSet::new();
            for key in ["tab", "shift-tab"] {
                for _ in 0..45 {
                    cx.update_window(handle, |_, window, cx| window.press(key, cx))
                        .unwrap();
                    cx.run_until_parked();
                    cx.update_window(handle, |_, window, cx| {
                        window.render_frame(cx);
                        for element in snapshots(window) {
                            if matches!(
                                element.role(),
                                Some(Role::Button | Role::Switch | Role::CheckBox)
                            ) && element.focused() == Some(true)
                            {
                                assert!(
                                    element.visible(),
                                    "{language} page={page} key={key}: {element:?}"
                                );
                                assert!(
                                    element.bounds().is_contained_within(&Bounds::new(
                                        Point::default(),
                                        window.viewport_size()
                                    )),
                                    "{element:?}"
                                );
                                visited.insert(element.label().unwrap().to_owned());
                            }
                        }
                    })
                    .unwrap();
                }
            }
            if page == 3 {
                cx.update(|cx| assert!(visited.contains(&view.read(cx).tr("settings.wizard"))));
            }
        }
    }
}

#[gpui_kit::test]
fn mapping_editor_reveals_manual_controls_and_keeps_escape_available_as_output(
    cx: &mut TestAppContext,
) {
    let (handle, view, _folder) = open(cx);
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view._pump = Task::ready(());
            view.dispatch(Command::App(Action::Navigate, "1".into()), window, cx);
        });
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("custom-mappings", cx);
        window.click("mapping-create", cx);
        window.click(("entry-mode", 1usize), cx);
    })
    .unwrap();
    let mut saw_output = false;
    for key in ["tab", "shift-tab"] {
        for _ in 0..25 {
            cx.update_window(handle, |_, window, cx| window.press(key, cx))
                .unwrap();
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                for element in snapshots(window) {
                    if matches!(
                        element.role(),
                        Some(Role::Button | Role::CheckBox | Role::TextInput)
                    ) && element.focused() == Some(true)
                    {
                        assert!(element.visible(), "{element:?}");
                    }
                }
                saw_output |= window.find("mapping-output-choice").focused() == Some(true);
            })
            .unwrap();
        }
    }
    assert!(saw_output);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.dispatch(
                Command::Mapping("select-output", "key/41".into()),
                window,
                cx,
            )
        });
        assert_eq!(view.read(cx).ui.mapping.selected_output.value, "key/41");
    })
    .unwrap();
}

#[gpui_kit::test]
fn errors_headings_and_live_regions_publish_content_and_urgency(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, _, cx| {
        let view = view.read(cx);
        let status = view
            .text("Saved")
            .role(Role::Status)
            .live(accesskit::Live::Polite);
        let mut node = accesskit::Node::new(status.a11y_role().unwrap());
        status.write_a11y_info(&mut node);
        assert_eq!(node.label(), Some("Saved"));
        assert_eq!(node.live(), Some(accesskit::Live::Polite));
        let error = view.error("Name is required", cx);
        let mut node = accesskit::Node::new(error.a11y_role().unwrap());
        error.write_a11y_info(&mut node);
        assert_eq!(node.role(), Role::Alert);
        assert_eq!(node.label(), Some("Name is required"));
        assert_eq!(node.live(), Some(accesskit::Live::Assertive));
        let heading = view.heading("Settings".into());
        let mut node = accesskit::Node::new(heading.a11y_role().unwrap());
        heading.write_a11y_info(&mut node);
        assert_eq!(node.role(), Role::Heading);
        assert_eq!(node.level(), Some(1));
        assert_eq!(node.label(), Some("Settings"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn discard_dialog_has_a_name_traps_focus_and_returns_to_the_trigger(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        cx.set_reduce_motion(true);
        view.update(cx, |view, cx| {
            view._pump = Task::ready(());
            view.dispatch(Command::App(Action::Navigate, "1".into()), window, cx);
        });
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("custom-mappings", cx);
        window.click("mapping-create", cx);
        window.click("mapping-name", cx);
        window.input("Unsaved", cx);
        window.press("escape", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("mapping-leave-dialog").role(),
            Some(Role::Dialog)
        );
        assert_eq!(
            window.find("mapping-leave-dialog").label(),
            Some(view.read(cx).tr("common.save_changes").as_str())
        );
        for key in ["tab", "shift-tab"] {
            for _ in 0..8 {
                window.press(key, cx);
                assert!(
                    ["dialog-cancel", "dialog-discard", "dialog-save"]
                        .iter()
                        .any(|id| window.find(*id).focused() == Some(true))
                );
            }
        }
        window.press("escape", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("mapping-leave-dialog").is_none());
        assert_eq!(window.find("mapping-name").focused(), Some(true));
    })
    .unwrap();
}
