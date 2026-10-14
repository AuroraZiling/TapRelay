use gpui_kit::App;
use gpui_kit::component::{Theme, ThemeRegistry};

pub(super) fn init(cx: &mut App) {
    let registry = ThemeRegistry::global_mut(cx);
    registry
        .load_themes_from_str(include_str!("../../resources/themes/catppuccin.json"))
        .expect("embedded Catppuccin themes must be valid");
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
    });
}
