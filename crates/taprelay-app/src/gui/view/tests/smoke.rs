use super::support::open;
use crate::gui::action::Action;
use crate::gui::view::Command;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, ElementId, TestAppContext};

#[gpui_kit::test]
fn renders_every_page_in_both_languages_in_a_compact_window(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        for language in ["en", "zh-CN"] {
            for (page, landmark) in [
                (0, ElementId::from("status-card")),
                (1, "custom-mappings".into()),
                (2, "receiver-refresh".into()),
                (3, ("theme", 0usize).into()),
                (4, "group-create".into()),
            ] {
                view.update(cx, |view, cx| {
                    view.dispatch(Command::App(Action::Language, language.into()), window, cx);
                    view.dispatch(Command::App(Action::Navigate, page.to_string()), window, cx);
                });
                window.render_frame(cx);
                assert_eq!(view.read(cx).ui.page, page);
                assert!(window.find("page-header").visible());
                assert!(
                    window.find(landmark).visible(),
                    "missing page {page} in {language}"
                );
            }
        }
        for step in [0, 2, 3] {
            view.update(cx, |view, cx| {
                view.ui.mode = 1;
                view.ui.wizard_page = step;
                cx.notify();
            });
            window.render_frame(cx);
            assert!(window.find("wizard-next").visible());
            assert!(
                window
                    .find(match step {
                        0 => "binding-enabled-media.play-pause",
                        2 => "receiver-refresh",
                        _ => "test-receiver",
                    })
                    .visible()
            );
        }
        for mode in [0, 3] {
            view.update(cx, |view, cx| {
                view.ui.mode = mode;
                cx.notify();
            });
            window.render_frame(cx);
            assert!(window.find("startup-quit").visible());
        }
    })
    .unwrap();
}
