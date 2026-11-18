//! The look LumaLink's windows share: the rounded font with the system's for Korean and Chinese, light colours and
//! rounded controls, and the icon.

use crate::{sky, words};
use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, Stroke, Vec2};

/// The window's id on Linux (Wayland's app id, X11's class): the name of the desktop entry LumaLink Setup writes, so
/// the menu, the dock and "Open with" know the window as LumaLink's.
pub const APP_ID: &str = "dev.nebula.lumalink";

fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("rounded".to_string(), std::sync::Arc::new(FontData::from_static(include_bytes!("../assets/MPLUSRounded1c-Medium.ttf"))));
    fonts.font_data.insert("rounded-bold".to_string(), std::sync::Arc::new(FontData::from_static(include_bytes!("../assets/MPLUSRounded1c-ExtraBold.ttf"))));
    fonts.families.insert(FontFamily::Name("bold".into()), vec!["rounded-bold".to_string(), "rounded".to_string()]);
    if let Some(body) = fonts.families.get_mut(&FontFamily::Proportional) {
        body.insert(0, "rounded".to_string());
    }
    // Korean and Chinese are not in the rounded font: the system's stand behind it, each weight behind its own family.
    for (family, weight, bold) in [(FontFamily::Name("bold".into()), "system-bold", true), (FontFamily::Proportional, "system", false)] {
        for (i, (path, face)) in words::system_fonts(bold).into_iter().enumerate() {
            let Ok(bytes) = std::fs::read(&path) else { continue };
            let name = format!("{weight}{i}");
            let mut data = FontData::from_owned(bytes);
            data.index = face;
            fonts.font_data.insert(name.clone(), std::sync::Arc::new(data));
            if let Some(list) = fonts.families.get_mut(&family) {
                list.push(name);
            }
        }
    }
    if words::system_font_first() {
        // Every character of a line from one font, Latin letters too: the rounded font holds some Chinese characters.
        for family in [FontFamily::Proportional, FontFamily::Name("bold".into())] {
            if let Some(list) = fonts.families.get_mut(&family) {
                list.sort_by_key(|name| !name.starts_with("system"));
            }
        }
    }
    fonts
}

/// LumaLink's icon for a window's title bar and taskbar button (build.rs draws it from icon.rs).
pub fn icon() -> egui::IconData {
    egui::IconData { rgba: include_bytes!(concat!(env!("OUT_DIR"), "/icon.rgba")).to_vec(), width: 256, height: 256 }
}

/// Gives `ctx` the fonts and look of LumaLink's windows.
pub fn setup(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.set_visuals(egui::Visuals::light());
    let mut style = (*ctx.style()).clone();
    style.visuals.panel_fill = Color32::TRANSPARENT;
    style.visuals.override_text_color = Some(sky::INK);
    style.visuals.window_fill = Color32::WHITE;
    style.visuals.window_corner_radius = egui::CornerRadius::same(14);
    style.visuals.widgets.inactive.weak_bg_fill = Color32::WHITE;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.5, Color32::from_rgb(196, 202, 209));
    style.visuals.widgets.hovered.weak_bg_fill = Color32::WHITE;
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(2.0, sky::BLUE);
    style.visuals.widgets.active.weak_bg_fill = Color32::from_rgb(232, 246, 253);
    style.visuals.widgets.active.bg_stroke = Stroke::new(2.0, sky::BLUE);
    style.visuals.widgets.open.weak_bg_fill = Color32::WHITE;
    style.visuals.widgets.open.bg_stroke = Stroke::new(2.0, sky::BLUE);
    style.visuals.extreme_bg_color = Color32::from_rgb(246, 248, 250);
    style.visuals.selection.bg_fill = Color32::from_rgb(200, 236, 250);
    style.visuals.selection.stroke = Stroke::new(1.5, sky::BLUE_INK);
    style.visuals.hyperlink_color = sky::BLUE_INK;
    style.visuals.menu_corner_radius = egui::CornerRadius::same(12);
    for widget in [&mut style.visuals.widgets.inactive, &mut style.visuals.widgets.hovered, &mut style.visuals.widgets.active, &mut style.visuals.widgets.open] {
        widget.corner_radius = egui::CornerRadius::same(18);
    }
    style.spacing.button_padding = Vec2::new(16.0, 8.0);
    style.spacing.item_spacing = Vec2::new(10.0, 10.0);
    style.spacing.interact_size.y = 34.0;
    ctx.set_style(style);
}
