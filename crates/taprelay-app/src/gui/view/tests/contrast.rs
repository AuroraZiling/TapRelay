use super::support::open;
use gpui_kit::component::{ActiveTheme, Colorize};
use gpui_kit::{AppContext, Hsla, TestAppContext, transparent_white};

fn contrast(a: Hsla, b: Hsla) -> f32 {
    fn luminance(color: Hsla) -> f32 {
        let color = color.to_rgb();
        let linear = |c: f32| {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
    }
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[gpui_kit::test]
fn app_theme_text_boundaries_and_focus_meet_contrast_thresholds(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        for dark in [false, true] {
            view.update(cx, |view, cx| {
                view.ui.dark = dark;
                view.effects(window, cx);
            });
            let (star_banner_bg, star_icon) = (
                crate::gui::theme::star_banner_bg(cx),
                crate::gui::theme::star_icon(cx),
            );
            let theme = cx.theme();
            for surface in [
                theme.background,
                theme.group_box,
                theme.sidebar,
                theme.popover,
            ] {
                for (name, color) in [
                    ("foreground", theme.foreground),
                    ("muted", theme.muted_foreground),
                    ("danger", theme.danger),
                    ("success", theme.success),
                    ("warning", theme.warning),
                    ("info", theme.info),
                ] {
                    assert!(
                        contrast(color, surface) >= 4.5,
                        "dark={dark} {name}: {}",
                        contrast(color, surface)
                    );
                    let tint = color.mix_oklab(transparent_white(), 0.04);
                    assert!(
                        contrast(color, surface.blend(tint)) >= 4.5,
                        "dark={dark} alert {name}: {}",
                        contrast(color, surface.blend(tint))
                    );
                }
                for (name, color) in [("focus", theme.ring), ("switch", theme.switch)] {
                    assert!(
                        contrast(color, surface) >= 3.,
                        "dark={dark} {name}: {}",
                        contrast(color, surface)
                    );
                }
            }
            for (name, color) in [
                ("foreground", theme.foreground),
                ("muted", theme.muted_foreground),
            ] {
                assert!(
                    contrast(color, star_banner_bg) >= 4.5,
                    "dark={dark} star_banner {name}: {}",
                    contrast(color, star_banner_bg)
                );
            }
            for color in crate::gui::theme::about_link_colors(cx) {
                let hover = if dark {
                    theme.accent.opacity(0.5)
                } else {
                    theme.accent
                };
                for surface in [theme.group_box, hover, theme.tokens.button_active.into()] {
                    let surface = theme.group_box.blend(surface);
                    assert!(
                        contrast(color, surface) >= 3.,
                        "dark={dark} about link: {}",
                        contrast(color, surface)
                    );
                }
            }
            let star_foreground = crate::gui::theme::star_link_foreground(cx);
            assert!(
                contrast(star_foreground, star_banner_bg) >= 4.5,
                "dark={dark} star_link: {}",
                contrast(star_foreground, star_banner_bg)
            );
            assert!(
                contrast(star_icon, star_banner_bg) >= 3.,
                "dark={dark} star_icon: {}",
                contrast(star_icon, star_banner_bg)
            );
            assert!(
                contrast(theme.ring, star_banner_bg) >= 3.,
                "dark={dark} star_banner focus: {}",
                contrast(theme.ring, star_banner_bg)
            );
            assert!(
                contrast(theme.secondary_foreground, theme.secondary) >= 4.5,
                "dark={dark} secondary: {}",
                contrast(theme.secondary_foreground, theme.secondary)
            );
            assert!(
                contrast(theme.foreground, theme.secondary) >= 4.5,
                "dark={dark} secondary foreground: {}",
                contrast(theme.foreground, theme.secondary)
            );
            assert!(contrast(theme.primary_foreground, theme.primary) >= 4.5);
        }
    })
    .unwrap();
}
