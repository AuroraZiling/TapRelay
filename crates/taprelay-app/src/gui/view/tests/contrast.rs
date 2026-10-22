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
            assert!(contrast(theme.primary_foreground, theme.primary) >= 4.5);
        }
    })
    .unwrap();
}
