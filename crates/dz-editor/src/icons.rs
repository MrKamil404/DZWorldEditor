//! Icons of the interface: the Phosphor set, drawn as glyphs of a font so that they take the
//! colour and size of the text they stand in.
pub use egui_phosphor::regular::*;

/// The font family of titles.
pub const STRONG: &str = "strong";

/// A font of the titles at the given size.
pub fn strong(size: f32) -> eframe::egui::FontId {
    eframe::egui::FontId::new(size, eframe::egui::FontFamily::Name(STRONG.into()))
}

/// Sets the text font and adds the icon font after it, so that any label can hold icons. The
/// text is in Segoe UI, the font of Windows and of Workbench, read from the system so that it
/// is not shipped with the program; egui's own font stands in where it is missing.
pub fn install(ctx: &eframe::egui::Context) {
    use eframe::egui::{FontData, FontFamily};
    let mut fonts = eframe::egui::FontDefinitions::default();
    let windows = std::env::var_os("WINDIR").map(std::path::PathBuf::from).unwrap_or_else(|| r"C:\Windows".into());
    if let Ok(bytes) = std::fs::read(windows.join("Fonts").join("segoeui.ttf")) {
        fonts.font_data.insert("segoe".into(), std::sync::Arc::new(FontData::from_owned(bytes)));
        fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "segoe".into());
    }
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    // A heavier face for titles of sections and panels; the plain one where it is missing.
    let mut strong = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    if let Ok(bytes) = std::fs::read(windows.join("Fonts").join("seguisb.ttf")) {
        fonts.font_data.insert("segoe_semibold".into(), std::sync::Arc::new(FontData::from_owned(bytes)));
        strong.insert(0, "segoe_semibold".into());
    }
    fonts.families.insert(FontFamily::Name(STRONG.into()), strong);
    ctx.set_fonts(fonts);
}

/// An icon followed by a label.
pub fn with(icon: &str, text: &str) -> String {
    format!("{icon}  {text}")
}

/// The program's icon for its windows, so the taskbar and Alt+Tab show it and not the default
/// one of the window library; the icon of the .exe file does not reach the windows by itself.
pub fn window_icon() -> std::sync::Arc<eframe::egui::IconData> {
    static ICON: std::sync::OnceLock<std::sync::Arc<eframe::egui::IconData>> = std::sync::OnceLock::new();
    ICON.get_or_init(|| {
        let img = app_icon_png(256).and_then(|png| image::load_from_memory(png).ok()).map(|i| i.into_rgba8());
        std::sync::Arc::new(match img {
            Some(i) => eframe::egui::IconData { width: i.width(), height: i.height(), rgba: i.into_raw() },
            None => eframe::egui::IconData::default(),
        })
    }).clone()
}

/// The picture of the program's icon at least `size` pixels wide, from the icon file built into
/// the program (every picture in it is a PNG).
pub fn app_icon_png(size: u32) -> Option<&'static [u8]> {
    static ICO: &[u8] = include_bytes!("../../../assets/app-icon.ico");
    let count = u16::from_le_bytes([*ICO.get(4)?, *ICO.get(5)?]) as usize;
    let mut best: Option<(u32, &[u8])> = None;
    for i in 0..count {
        let e = ICO.get(6 + 16 * i..22 + 16 * i)?;
        let w = if e[0] == 0 { 256 } else { e[0] as u32 };
        let len = u32::from_le_bytes([e[8], e[9], e[10], e[11]]) as usize;
        let at = u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as usize;
        let data = ICO.get(at..at + len)?;
        if !data.starts_with(b"\x89PNG") { continue; }
        // The smallest one big enough, else the biggest.
        let better = match best { None => true, Some((bw, _)) => (w >= size && (bw < size || w < bw)) || (bw < size && w > bw) };
        if better { best = Some((w, data)); }
    }
    best.map(|b| b.1)
}
