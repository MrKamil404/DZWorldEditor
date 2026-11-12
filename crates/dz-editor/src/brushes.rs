//! Brush tips of the user's own: grey pictures kept in `%LOCALAPPDATA%\DZWorldEditor\brushes`.
use crate::i18n::{t, tf};
use eframe::egui;
use crate::theme::tool_button;
use dz_core::edit::{BrushShape, Stamp};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Pictures larger than this are scaled down; a brush needs no more detail.
const MAX_SIDE: u32 = 256;
const EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "bmp", "tga", "webp"];

pub struct CustomBrush {
    pub name: String,
    pub path: PathBuf,
    pub stamp: Arc<Stamp>,
    preview: image::RgbaImage,
    thumb: Option<egui::TextureHandle>,
}

pub fn folder() -> PathBuf {
    crate::settings::data_dir().join("brushes")
}

pub fn load(path: &Path) -> Result<CustomBrush, String> {
    let img = image::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let img = if img.width().max(img.height()) > MAX_SIDE { img.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Triangle) } else { img };
    let la = img.to_luma_alpha8();
    let pixels: Vec<[u8; 2]> = la.pixels().map(|p| p.0).collect();
    let stamp = Stamp::from_luma_alpha(la.width() as usize, la.height() as usize, &pixels);
    // White on a dark tile, the way it works on the ground.
    let preview = image::RgbaImage::from_fn(la.width(), la.height(), |x, y| {
        let v = (stamp.data[(y * la.width() + x) as usize] * 255.0) as u8;
        image::Rgba([v.max(28), v.max(30), v.max(34), 255])
    });
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(CustomBrush { name, path: path.to_path_buf(), stamp: Arc::new(stamp), preview, thumb: None })
}

/// Every picture in the brush folder, by name.
pub fn load_all() -> Vec<CustomBrush> {
    let Ok(rd) = std::fs::read_dir(folder()) else { return Vec::new() };
    let mut out: Vec<CustomBrush> = rd.flatten().map(|e| e.path()).filter(|p| is_picture(p)).filter_map(|p| load(&p).ok()).collect();
    out.sort_by_key(|b| b.name.to_lowercase());
    out
}

fn is_picture(p: &Path) -> bool {
    p.extension().is_some_and(|x| EXTENSIONS.contains(&x.to_string_lossy().to_lowercase().as_str()))
}

/// The picture of a brush chosen by name.
pub fn stamp(brushes: &[CustomBrush], name: &Option<String>) -> Option<Arc<Stamp>> {
    let name = name.as_ref()?;
    brushes.iter().find(|b| &b.name == name).map(|b| b.stamp.clone())
}

/// Copies pictures into the brush folder; returns the name of the last one and any problems.
fn add(brushes: &mut Vec<CustomBrush>, files: Vec<PathBuf>) -> (Option<String>, Vec<String>) {
    let dir = folder();
    let mut problems = Vec::new();
    let mut last = None;
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return (None, vec![format!("{}: {e}", dir.display())]);
    }
    for f in files {
        let Some(file) = f.file_name() else { continue };
        let mut target = dir.join(file);
        // A brush of the same name is kept; the new one gets a number.
        let mut n = 2;
        while target.exists() && target != f {
            let stem = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let ext = f.extension().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            target = dir.join(format!("{stem} {n}.{ext}"));
            n += 1;
        }
        // Read first, so a file that is no picture is not copied.
        match load(&f) {
            Ok(_) => {}
            Err(e) => {
                problems.push(e);
                continue;
            }
        }
        if let Err(e) = std::fs::copy(&f, &target) {
            problems.push(format!("{}: {e}", target.display()));
            continue;
        }
        match load(&target) {
            Ok(b) => {
                last = Some(b.name.clone());
                brushes.retain(|x| x.name != b.name);
                brushes.push(b);
            }
            Err(e) => problems.push(e),
        }
    }
    brushes.sort_by_key(|b| b.name.to_lowercase());
    (last, problems)
}

/// What the user did in the row of brush tips.
pub enum TipAction {
    None,
    Problems(Vec<String>),
}

/// The built-in outlines and the user's pictures, with a button to add pictures.
pub fn tip_row(ui: &mut egui::Ui, brushes: &mut Vec<CustomBrush>, shape: &mut BrushShape, picture: &mut Option<String>) -> TipAction {
    use dz_core::edit::BrushShape as B;
    let mut action = TipAction::None;
    ui.horizontal_wrapped(|ui| {
        for (s, name) in [(B::Circle, "Round"), (B::Square, "Square"), (B::Diamond, "Diamond")] {
            if tool_button(ui, picture.is_none() && *shape == s, &t(name)).clicked() {
                *shape = s;
                *picture = None;
            }
        }
        let mut remove = None;
        for (i, b) in brushes.iter_mut().enumerate() {
            let tex = b.thumb.get_or_insert_with(|| {
                let size = [b.preview.width() as usize, b.preview.height() as usize];
                ui.ctx().load_texture(format!("brush-{}", b.path.display()), egui::ColorImage::from_rgba_unmultiplied(size, b.preview.as_raw()), egui::TextureOptions::LINEAR)
            });
            let image = egui::Image::new(egui::load::SizedTexture::new(tex.id(), egui::vec2(22.0, 22.0)));
            let chosen = picture.as_deref() == Some(b.name.as_str());
            let r = ui.add(egui::Button::image(image).selected(chosen)).on_hover_text(&b.name);
            if r.clicked() {
                *picture = Some(b.name.clone());
            }
            r.context_menu(|ui| {
                if ui.button(t("Remove brush")).on_hover_text(b.path.display().to_string()).clicked() {
                    remove = Some(i);
                    ui.close_menu();
                }
            });
        }
        if let Some(i) = remove {
            let b = brushes.remove(i);
            if picture.as_deref() == Some(b.name.as_str()) {
                *picture = None;
            }
            if let Err(e) = std::fs::remove_file(&b.path) {
                action = TipAction::Problems(vec![format!("{}: {e}", b.path.display())]);
            }
        }
        let add_tip = tf("Add your own brushes: grey pictures, white works fully, black not at all. They are copied to {0}", &[&folder().display().to_string()]);
        if ui.button(crate::icons::PLUS).on_hover_text(add_tip).clicked() {
            if let Some(files) = rfd::FileDialog::new().set_title(t("Choose brush pictures")).add_filter(t("Pictures"), &EXTENSIONS).pick_files() {
                let (last, problems) = add(brushes, files);
                if last.is_some() {
                    *picture = last;
                }
                if !problems.is_empty() {
                    action = TipAction::Problems(problems);
                }
            }
        }
    });
    action
}

/// The spray switch and its density.
pub fn spray_row(ui: &mut egui::Ui, on: &mut bool, density: &mut f32) {
    ui.horizontal(|ui| {
        ui.checkbox(on, t("Spray")).on_hover_text(t("Sprays scattered dots that fill in while the button is held, instead of covering the whole brush."));
        if *on {
            ui.add(egui::Slider::new(density, 0.01..=0.6).logarithmic(true).text(t("Density")));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_are_added_and_read_back() {
        let dir = std::env::temp_dir().join(format!("dzwe-brush-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("spot.png");
        image::GrayImage::from_fn(8, 8, |x, _| image::Luma([if x < 4 { 255 } else { 0 }])).save(&src).unwrap();
        let b = load(&src).unwrap();
        assert_eq!((b.stamp.width, b.stamp.height), (8, 8));
        assert!(b.stamp.sample(-0.5, 0.0) > 0.9 && b.stamp.sample(0.5, 0.0) < 0.1);
        assert!(!is_picture(&dir.join("notes.txt")));
        std::fs::remove_dir_all(&dir).ok();
    }
}
