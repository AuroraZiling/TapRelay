use super::support::open;
use crate::action::Action;
use crate::gui::view::Command;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};

#[gpui_kit::test]
fn renders_every_page_in_both_languages_and_the_minimum_window(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        for language in ["english", "chinese"] {
            for page in 0..5 {
                view.update(cx, |view, cx| {
                    view.dispatch(Command::App(Action::Language, language.into()), window, cx);
                    view.dispatch(Command::App(Action::Navigate, page.to_string()), window, cx);
                });
                window.render_frame(cx);
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
