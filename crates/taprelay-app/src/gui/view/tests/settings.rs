use super::support::open;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};
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
            (1, true, "Catppuccin Latte", 0xe5e9ef, 0x7287fd),
            (0, true, "Catppuccin Macchiato", 0x1e2030, 0x8aadf4),
            (0, false, "Catppuccin Latte", 0xe5e9ef, 0x7287fd),
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
