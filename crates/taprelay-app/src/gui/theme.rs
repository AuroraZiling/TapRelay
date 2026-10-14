use gpui_kit::App;
use gpui_kit::component::{Theme, ThemeRegistry, ThemeSet};

pub(super) fn init(cx: &mut App) {
    let mut themes: ThemeSet =
        serde_json::from_str(include_str!("../../resources/themes/catppuccin.json"))
            .expect("embedded Catppuccin themes must be valid");
    for config in &mut themes.themes {
        let shell_background = match config.name.as_ref() {
            "Catppuccin Latte" => "#EFF1F5",
            "Catppuccin Macchiato" => {
                config.colors.secondary_hover = Some("#494D64".into());
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
    });
}
