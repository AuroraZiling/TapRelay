use gpui_kit::component::{
    ActiveTheme, Colorize, Theme, ThemeRegistry, ThemeSet, input::CursorBlink,
};
use gpui_kit::{App, Hsla, rgb};
use std::time::Duration;

pub(super) fn warning_signal(cx: &App) -> Hsla {
    if cx.theme().is_dark() {
        cx.theme().warning
    } else {
        rgb(0xBC8A00).into()
    }
}

pub(super) fn star_icon(cx: &App) -> Hsla {
    if cx.theme().is_dark() {
        rgb(0xF5A623).into()
    } else {
        rgb(0xBC7000).into()
    }
}

pub(super) fn star_banner_bg(cx: &App) -> Hsla {
    if cx.theme().is_dark() {
        rgb(0x3D3228).into()
    } else {
        rgb(0xFEF7E8).into()
    }
}

pub(super) fn star_link_foreground(cx: &App) -> Hsla {
    if cx.theme().is_dark() {
        star_icon(cx).lighten(0.35)
    } else {
        star_icon(cx).darken(0.35)
    }
}

pub(super) fn about_link_colors(cx: &App) -> [Hsla; 4] {
    let colors = if cx.theme().is_dark() {
        [0x8AADF4, 0xC6A0F6, 0xA6DA95, 0xEED49F]
    } else {
        [0x295CC4, 0x7C3BB5, 0x356E1A, 0x805600]
    };
    colors.map(|color| rgb(color).into())
}

pub(super) fn star_banner_border(cx: &App) -> Hsla {
    if cx.theme().is_dark() {
        rgb(0x5E4A36).into()
    } else {
        rgb(0xEDD8B4).into()
    }
}

pub(super) fn init(cx: &mut App) {
    let mut themes: ThemeSet =
        serde_json::from_str(include_str!("../../resources/themes/catppuccin.json"))
            .expect("embedded Catppuccin themes must be valid");
    for config in &mut themes.themes {
        let shell_background = match config.name.as_ref() {
            "Catppuccin Latte" => {
                config.colors.background = Some("#EDF1F7".into());
                config.colors.foreground = Some("#30384B".into());
                config.colors.muted_foreground = Some("#5B6578".into());
                config.colors.group_box = Some("#FFFFFF".into());
                config.colors.popover = Some("#FFFFFF".into());
                config.colors.popover_foreground = Some("#30384B".into());
                config.colors.border = Some("#D5DCE7".into());
                config.colors.muted = Some("#E5EAF2".into());
                config.colors.secondary = Some("#E9EEF6".into());
                config.colors.secondary_foreground = Some("#30384B".into());
                config.colors.secondary_hover = Some("#DCE4F0".into());
                config.colors.secondary_active = Some("#D0DAE9".into());
                config.colors.accent = Some("#DEE5F2".into());
                config.colors.accent_foreground = Some("#30384B".into());
                config.colors.danger = Some("#AA2741".into());
                config.colors.success = Some("#356E1A".into());
                config.colors.info = Some("#176A77".into());
                config.colors.warning = Some("#805600".into());
                config.colors.primary = Some("#435AC4".into());
                config.colors.primary_hover = Some("#3B50B0".into());
                config.colors.primary_active = Some("#334699".into());
                config.colors.ring = Some("#435AC4".into());
                config.colors.switch = Some("#777B8E".into());
                "#F7F9FC"
            }
            "Catppuccin Macchiato" => {
                config.colors.secondary_hover = Some("#494D64".into());
                config.colors.switch = Some("#70758D".into());
                "#24273A"
            }
            _ => continue,
        };
        config.colors.sidebar = Some(shell_background.into());
        config.colors.title_bar = Some(shell_background.into());
    }
    let registry = ThemeRegistry::global_mut(cx);
    registry
        .load_themes_from_str(&serde_json::to_string(&themes).expect("serialize app themes"))
        .expect("register app themes");
    let light = registry
        .themes()
        .get("Catppuccin Latte")
        .expect("embedded Catppuccin Latte theme must exist")
        .clone();
    let dark = registry
        .themes()
        .get("Catppuccin Macchiato")
        .expect("embedded Catppuccin Macchiato theme must exist")
        .clone();
    Theme::update(cx, |theme| {
        theme.apply_config(if theme.is_dark() { &dark } else { &light });
        theme.light_theme = light;
        theme.dark_theme = dark;
        theme.cursor_blink = CursorBlink::blinking(Duration::from_millis(500))
            .with_idle_timeout(Duration::from_secs(5));
    });
}
