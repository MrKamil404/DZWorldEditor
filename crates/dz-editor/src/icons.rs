//! Icons of the interface: the Phosphor set, drawn as glyphs of a font so that they take the
//! colour and size of the text they stand in.
pub use egui_phosphor::regular::*;

/// Adds the icon font after the text font, so that any label can hold icons.
pub fn install(ctx: &eframe::egui::Context) {
    let mut fonts = eframe::egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);
}

/// An icon followed by a label.
pub fn with(icon: &str, text: &str) -> String {
    format!("{icon}  {text}")
}
