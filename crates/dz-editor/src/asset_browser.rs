//! The content browser, laid out like the one of Unreal Engine: sources on the left, the
//! contents of a folder in a grid with breadcrumbs above it, and the details of what is
//! selected on the right. Besides the models of the work drive it holds folders made by the
//! user, the library and the built-in generators, presets and modifiers, which are locked.
use crate::app::{App, Dialog, LineAction, Mode};
use crate::i18n::{t, tf};
use crate::library_ui::LibDrag;
use crate::theme;
use dz_core::library::{in_folder, parent_folder, BUILT_IN};
use dz_core::world::{ModMode, ModShape};
use eframe::egui::{self, pos2, vec2, Color32, Rect, Stroke};
use std::collections::BTreeMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// Something the browser shows that can be selected, dragged and placed.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Pick {
    Model(String),
    /// An object preset of the library.
    Preset(u64),
    /// A generator, built in or of the library.
    Generator(u64),
    /// A forest preset, built in or of the library.
    Forest(String),
    /// A road type of the road catalog.
    Road(String),
    /// One of `MODIFIERS`.
    Modifier(usize),
}

/// Several things dragged out of the browser at once.
#[derive(Clone, Debug)]
pub struct Dragged(pub Vec<Pick>);

/// Groups of the browser that are not folders of the work drive or of the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    BuiltIn,
    BuiltInGenerators,
    BuiltInPresets,
    BuiltInModifiers,
    Library,
    LibraryGenerators,
    LibraryPresets,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A folder of the work drive; "" is its top.
    Drive(String),
    /// A folder made by the user; "" is the top of them.
    Folder(String),
    Group(Group),
    /// Every model with a surface assigned to it.
    Surfaced,
    /// The Terrain Builder template libraries of the project, or the models of one of them.
    Templates(Option<usize>),
}

impl Source {
    fn parent(&self) -> Option<Source> {
        match self {
            Source::Drive(p) if !p.is_empty() => Some(Source::Drive(p.rsplit_once('\\').map_or("", |x| x.0).to_string())),
            Source::Folder(n) if !n.is_empty() => Some(Source::Folder(parent_folder(n).to_string())),
            Source::Group(Group::BuiltInGenerators | Group::BuiltInPresets | Group::BuiltInModifiers) => Some(Source::Group(Group::BuiltIn)),
            Source::Group(Group::LibraryGenerators | Group::LibraryPresets) | Source::Templates(None) => Some(Source::Group(Group::Library)),
            Source::Templates(Some(_)) => Some(Source::Templates(None)),
            _ => None,
        }
    }
    /// The path shown above the grid, as (label, source) from the top.
    fn crumbs(&self) -> Vec<(String, Source)> {
        let mut out = Vec::new();
        let mut at = Some(self.clone());
        while let Some(s) = at {
            out.push((s.label(), s.clone()));
            at = s.parent();
        }
        out.reverse();
        out
    }
    fn label(&self) -> String {
        match self {
            Source::Drive(p) if p.is_empty() => t("Work drive"),
            Source::Drive(p) => p.rsplit('\\').next().unwrap_or(p).to_string(),
            Source::Folder(n) if n.is_empty() => t("My folders"),
            Source::Folder(n) => n.rsplit('/').next().unwrap_or(n).to_string(),
            Source::Group(g) => t(match g {
                Group::BuiltIn => "Built-in",
                Group::BuiltInGenerators | Group::LibraryGenerators => "Generators",
                Group::BuiltInPresets | Group::LibraryPresets => "Presets",
                Group::BuiltInModifiers => "Modifiers",
                Group::Library => "Library",
            }),
            Source::Surfaced => t("Models with a surface"),
            Source::Templates(None) => t("Project templates"),
            Source::Templates(Some(i)) => TEMPLATE_NAMES.with(|n| n.borrow().get(*i).cloned()).unwrap_or_default(),
        }
    }
    fn locked(&self) -> bool {
        matches!(self, Source::Group(Group::BuiltIn | Group::BuiltInGenerators | Group::BuiltInPresets | Group::BuiltInModifiers))
    }
}

thread_local! {
    /// Names of the template libraries shown, for the labels of their folders.
    static TEMPLATE_NAMES: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The template libraries of the project, read again when their list changes.
#[derive(Default)]
struct Templates {
    /// What the project had when the list was made.
    key: Option<(Vec<std::path::PathBuf>, Option<std::path::PathBuf>)>,
    files: Vec<std::path::PathBuf>,
    /// (name, models)
    libraries: Vec<(String, Vec<String>)>,
}

impl Templates {
    fn update(&mut self, files: Vec<std::path::PathBuf>) {
        if files == self.files {
            return;
        }
        self.libraries = files.iter().map(|f| {
            let name = f.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let text = std::fs::read(f).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
            (name, dz_core::tv4l::library_models(&text))
        }).collect();
        self.files = files;
        TEMPLATE_NAMES.with(|n| *n.borrow_mut() = self.libraries.iter().map(|l| l.0.clone()).collect());
    }
}

/// The template libraries of a project; one imported before they were kept takes those next
/// to its Terrain Builder project.
fn project_templates(app: &App) -> Vec<std::path::PathBuf> {
    let Some(d) = &app.doc else { return Vec::new() };
    if !d.world.template_libraries.is_empty() {
        return d.world.template_libraries.clone();
    }
    d.world.road_project.as_deref().map(dz_core::import::libraries_near).unwrap_or_default()
}

/// A terrain modifier that comes with the program, placed where it is dropped.
pub struct ModTemplate {
    pub name: &'static str,
    pub mode: ModMode,
    pub shape: fn() -> ModShape,
    pub amount: f32,
    pub falloff: f32,
}

pub const MODIFIERS: [ModTemplate; 6] = [
    ModTemplate { name: "Flatten (circle)", mode: ModMode::Flatten, shape: || ModShape::Circle { radius: 12.0 }, amount: 0.0, falloff: 10.0 },
    ModTemplate { name: "Flatten (rectangle)", mode: ModMode::Flatten, shape: || ModShape::Rect { half_x: 10.0, half_z: 10.0 }, amount: 0.0, falloff: 10.0 },
    ModTemplate { name: "Raise", mode: ModMode::Offset, shape: || ModShape::Circle { radius: 15.0 }, amount: 3.0, falloff: 15.0 },
    ModTemplate { name: "Lower", mode: ModMode::Offset, shape: || ModShape::Circle { radius: 15.0 }, amount: -3.0, falloff: 15.0 },
    ModTemplate { name: "Fill up to the level", mode: ModMode::Fill, shape: || ModShape::Circle { radius: 15.0 }, amount: 0.0, falloff: 12.0 },
    ModTemplate { name: "Cut down to the level", mode: ModMode::Cut, shape: || ModShape::Circle { radius: 15.0 }, amount: 0.0, falloff: 12.0 },
];

/// The folders of the work drive as a tree, made once for each scan.
#[derive(Default)]
struct DriveTree {
    key: usize,
    children: BTreeMap<String, Vec<String>>,
}

impl DriveTree {
    fn build(assets: &dz_core::assets::AssetIndex) -> Self {
        let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for folder in assets.by_folder.keys() {
            let mut path = folder.as_str();
            while !path.is_empty() {
                let parent = path.rsplit_once('\\').map_or("", |x| x.0);
                let list = children.entry(parent.to_string()).or_default();
                if list.iter().any(|c| c == path) {
                    break;
                }
                list.push(path.to_string());
                path = parent;
            }
        }
        for list in children.values_mut() {
            list.sort();
        }
        Self { key: 0, children }
    }
}

/// A large model view that turns when dragged, rendered in the background.
pub struct Preview3d {
    shown: Option<(String, i32, i32)>,
    texture: Option<egui::TextureHandle>,
    busy: bool,
    wanted: Option<(String, i32, i32)>,
    failed: Option<String>,
    yaw: f32,
    pitch: f32,
    mesh: Arc<Mutex<Option<(String, Arc<forest_core::model3d::ModelMesh>)>>>,
    tx: Sender<(String, i32, i32, Option<Vec<u8>>)>,
    rx: Receiver<(String, i32, i32, Option<Vec<u8>>)>,
}

const PREVIEW: u32 = 320;

impl Default for Preview3d {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self { shown: None, texture: None, busy: false, wanted: None, failed: None, yaw: 35.0, pitch: 12.0, mesh: Default::default(), tx, rx }
    }
}

impl Preview3d {
    fn poll(&mut self, ctx: &egui::Context, db: &dz_core::model_info::ModelDb) {
        while let Ok((model, yaw, pitch, px)) = self.rx.try_recv() {
            self.busy = false;
            match px {
                Some(px) => {
                    let image = egui::ColorImage::from_rgba_unmultiplied([PREVIEW as usize; 2], &px);
                    match &mut self.texture {
                        Some(t) => t.set(image, egui::TextureOptions::LINEAR),
                        None => self.texture = Some(ctx.load_texture("asset_preview", image, egui::TextureOptions::LINEAR)),
                    }
                    self.failed = None;
                }
                None => self.failed = Some(model.clone()),
            }
            self.shown = Some((model, yaw, pitch));
        }
        if !self.busy {
            if let Some(w) = self.wanted.take() {
                self.start(w, ctx, db);
            }
        }
    }
    fn request(&mut self, model: &str, ctx: &egui::Context, db: &dz_core::model_info::ModelDb) {
        let want = (model.to_string(), self.yaw.round() as i32, self.pitch.round() as i32);
        if self.shown.as_ref() == Some(&want) || self.wanted.as_ref() == Some(&want) {
            return;
        }
        if self.busy {
            self.wanted = Some(want);
        } else {
            self.start(want, ctx, db);
        }
    }
    fn start(&mut self, (model, yaw, pitch): (String, i32, i32), ctx: &egui::Context, db: &dz_core::model_info::ModelDb) {
        self.busy = true;
        let (tx, cache, db, ctx) = (self.tx.clone(), self.mesh.clone(), db.clone(), ctx.clone());
        std::thread::spawn(move || {
            let cached = cache.lock().unwrap().as_ref().filter(|c| c.0 == model).map(|c| c.1.clone());
            let mesh = cached.or_else(|| {
                let file = db.file(&model);
                let mesh = forest_core::model3d::load_model_lod(&file, db.root(), 512, 0).ok().or_else(|| crate::models::thumbnail_mesh(&file, &Default::default()))?;
                let mesh = Arc::new(mesh);
                *cache.lock().unwrap() = Some((model.clone(), mesh.clone()));
                Some(mesh)
            });
            let px = mesh.map(|m| forest_core::model3d::render_preview(&m, PREVIEW, (yaw as f32).to_radians(), (pitch as f32).to_radians()));
            let _ = tx.send((model, yaw, pitch, px));
            ctx.request_repaint();
        });
    }
}

/// State of the browser.
pub struct Browser {
    pub source: Source,
    back: Vec<Source>,
    forward: Vec<Source>,
    pub selected: Vec<Pick>,
    anchor: Option<Pick>,
    /// 0 everything, 1 trees and bushes, 2 other models.
    kind: u8,
    tree: DriveTree,
    preview: Preview3d,
    /// The model of a preset shown in the preview instead of its first one.
    preview_item: Option<String>,
    /// (folder, new name) while a folder is being renamed.
    rename: Option<(String, String)>,
    templates: Templates,
    /// What follows the cursor while something is dragged out of the browser.
    ghost: Option<(Vec<egui::TextureId>, String, Color32, &'static str)>,
}

impl Default for Browser {
    fn default() -> Self {
        Self { source: Source::Drive(String::new()), back: Vec::new(), forward: Vec::new(), selected: Vec::new(), anchor: None, kind: 0, tree: DriveTree::default(), preview: Preview3d::default(), preview_item: None, rename: None, templates: Templates::default(), ghost: None }
    }
}

impl Browser {
    pub fn go(&mut self, to: Source) {
        if to != self.source {
            self.back.push(std::mem::replace(&mut self.source, to));
            self.forward.clear();
        }
    }
    pub fn models(&self) -> Vec<String> {
        self.selected.iter().filter_map(|p| if let Pick::Model(m) = p { Some(m.clone()) } else { None }).collect()
    }
    pub fn presets(&self) -> Vec<u64> {
        self.selected.iter().filter_map(|p| if let Pick::Preset(id) = p { Some(*id) } else { None }).collect()
    }
}

/// One tile of the grid.
#[derive(Clone)]
enum Item {
    Folder(Source),
    Asset(Pick),
}

/// What a tile is drawn with.
struct Look {
    name: String,
    images: Vec<egui::TextureId>,
    letter: &'static str,
    color: Color32,
    stripe: Color32,
    locked: bool,
    surface: Option<Color32>,
    count: Option<usize>,
    tooltip: String,
    /// A terrain modifier, drawn as the profile it gives the ground.
    profile: Option<usize>,
}

/// The ground before and after a modifier of `MODIFIERS`, as heights 0..1 across 0..1.
fn profile(i: usize, x: f32) -> (f32, f32) {
    let m = &MODIFIERS[i];
    let d = (x - 0.5).abs();
    let w = 1.0 - ((d - 0.18) / 0.16).clamp(0.0, 1.0);
    let w = w * w * (3.0 - 2.0 * w);
    let bell = (-(d * d) / 0.02).exp();
    let level = 0.5;
    let base = 0.48 + 0.07 * (x * 11.0).sin() + match m.mode {
        ModMode::Fill => -0.22 * bell,
        ModMode::Cut => 0.22 * bell,
        _ => 0.0,
    };
    let to_level = base + (level - base) * w;
    let after = match m.mode {
        ModMode::Flatten => to_level,
        ModMode::Offset => base + m.amount.signum() * 0.2 * w,
        ModMode::Fill => base.max(to_level),
        ModMode::Cut => base.min(to_level),
    };
    (base, after)
}

fn draw_profile(p: &egui::Painter, art: Rect, i: usize) {
    p.rect_filled(art, 2.0, Color32::from_rgb(52, 62, 74));
    const N: usize = 48;
    let y = |v: f32| art.bottom() - (0.1 + v * 0.75) * art.height();
    let step = art.width() / N as f32;
    let (mut before, mut after) = (Vec::new(), Vec::new());
    for k in 0..=N {
        let x = k as f32 / N as f32;
        let (b, a) = profile(i, x);
        before.push(pos2(art.left() + x * art.width(), y(b)));
        after.push(pos2(art.left() + x * art.width(), y(a)));
        if k < N {
            // Ground under the new surface; what was added or taken away shows in its own colour.
            let px = art.left() + k as f32 * step;
            let (top, low) = (y(a).min(y(b)), y(a).max(y(b)));
            p.rect_filled(Rect::from_min_max(pos2(px, low), pos2(px + step + 0.5, art.bottom())), 0.0, Color32::from_rgb(112, 98, 66));
            if low - top > 0.5 {
                let c = if a > b { Color32::from_rgb(150, 190, 90) } else { Color32::from_rgb(200, 110, 80) };
                p.rect_filled(Rect::from_min_max(pos2(px, top), pos2(px + step + 0.5, low)), 0.0, if a > b { c } else { c.gamma_multiply(0.35) });
            }
        }
    }
    p.add(egui::Shape::dashed_line(&before, Stroke::new(1.0f32, Color32::from_white_alpha(120)), 3.0, 3.0));
    p.add(egui::Shape::line(after, Stroke::new(2.0f32, Color32::from_rgb(235, 215, 140))));
}

const STRIPE_MODEL: Color32 = Color32::from_rgb(70, 140, 200);
const STRIPE_PRESET: Color32 = Color32::from_rgb(230, 140, 40);
const STRIPE_GENERATOR: Color32 = Color32::from_rgb(150, 95, 205);
const STRIPE_FOREST: Color32 = Color32::from_rgb(80, 165, 85);
const STRIPE_ROAD: Color32 = Color32::from_rgb(150, 150, 150);
const STRIPE_MODIFIER: Color32 = Color32::from_rgb(205, 175, 60);

/// A small padlock for what cannot be changed.
fn lock_icon(p: &egui::Painter, at: egui::Pos2, size: f32) {
    let r = Rect::from_min_size(at, vec2(size, size));
    p.circle_filled(r.center(), size * 0.62, Color32::from_black_alpha(140));
    p.text(r.center(), egui::Align2::CENTER_CENTER, crate::icons::LOCK_SIMPLE, egui::FontId::proportional(size * 1.05), Color32::from_rgb(240, 200, 90));
}

fn folder_icon(p: &egui::Painter, r: Rect, locked: bool) {
    let c = if locked { Color32::from_rgb(150, 132, 96) } else { Color32::from_rgb(196, 160, 88) };
    let tab = Rect::from_min_size(r.min + vec2(r.width() * 0.12, r.height() * 0.2), vec2(r.width() * 0.32, r.height() * 0.12));
    p.rect_filled(tab, 3.0, c.gamma_multiply(0.85));
    let body = Rect::from_min_max(r.min + vec2(r.width() * 0.1, r.height() * 0.28), r.max - vec2(r.width() * 0.1, r.height() * 0.16));
    p.rect_filled(body, 4.0, c);
    if locked {
        lock_icon(p, body.center() - vec2(9.0, 9.0), 18.0);
    }
}

fn draw_tile(ui: &egui::Ui, rect: Rect, side: f32, look: &Look, selected: bool, hovered: bool, folder: bool) {
    let p = ui.painter();
    p.rect_filled(rect, 3.0, if selected { theme::ACCENT_SOFT } else if hovered { theme::HOVER } else { Color32::from_rgb(42, 42, 44) });
    if selected {
        p.rect_stroke(rect, 3.0, Stroke::new(1.0f32, theme::ACCENT), egui::StrokeKind::Inside);
    }
    let art = Rect::from_min_size(rect.min + vec2(4.0, 4.0), vec2(side, side));
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    if folder {
        folder_icon(p, art, look.locked);
    } else {
        match look.images.len() {
            _ if look.profile.is_some() => draw_profile(p, art, look.profile.unwrap()),
            0 => {
                p.rect_filled(art, 2.0, look.color);
                p.text(art.center(), egui::Align2::CENTER_CENTER, look.letter, egui::FontId::proportional(side * 0.42), Color32::from_white_alpha(210));
            }
            1 => {
                p.rect_filled(art, 2.0, Color32::from_rgb(52, 52, 52));
                p.image(look.images[0], art, uv, Color32::WHITE);
            }
            n => {
                // A preset shows the models in it.
                p.rect_filled(art, 2.0, Color32::from_rgb(52, 52, 52));
                let half = side * 0.5;
                for (k, img) in look.images.iter().take(4).enumerate() {
                    let cell = Rect::from_min_size(art.min + vec2((k % 2) as f32 * half, (k / 2) as f32 * half), vec2(half, half)).shrink(1.0);
                    let cell = if n == 2 { Rect::from_min_size(art.min + vec2(k as f32 * half, half * 0.5), vec2(half, half)).shrink(1.0) } else if n == 3 && k == 2 { Rect::from_min_size(art.min + vec2(half * 0.5, half), vec2(half, half)).shrink(1.0) } else { cell };
                    p.image(*img, cell, uv, Color32::WHITE);
                }
            }
        }
        // The colour strip of the kind of asset.
        p.rect_filled(Rect::from_min_size(pos2(art.left(), art.bottom() + 1.0), vec2(side, 3.0)), 0.0, look.stripe);
        if let Some(n) = look.count {
            let badge = Rect::from_min_size(pos2(art.left() + 2.0, art.bottom() - 16.0), vec2(22.0, 14.0));
            p.rect_filled(badge, 3.0, Color32::from_black_alpha(170));
            p.text(badge.center(), egui::Align2::CENTER_CENTER, n.to_string(), egui::FontId::proportional(10.0), Color32::WHITE);
        }
        if look.locked {
            lock_icon(p, art.min + vec2(3.0, 3.0), 13.0);
        }
    }
    if let Some(c) = look.surface {
        // The mask colour assigned to the model.
        let mark = Rect::from_min_size(pos2(art.right() - 14.0, art.top() + 2.0), vec2(12.0, 12.0));
        p.rect_filled(mark, 2.0, c);
        p.rect_stroke(mark, 2.0, Stroke::new(1.0f32, Color32::BLACK), egui::StrokeKind::Outside);
    }
    let short: String = if look.name.chars().count() > 16 { look.name.chars().take(15).chain(['…']).collect() } else { look.name.clone() };
    p.text(pos2(rect.center().x, rect.bottom() - 10.0), egui::Align2::CENTER_CENTER, short, egui::FontId::proportional(11.0), theme::TEXT);
}

/// Models of a forest preset, the heaviest first.
pub fn forest_models(app: &App, name: &str) -> Vec<String> {
    use forest_core::user_presets::UserPreset;
    let own = app.library.all_forest_presets();
    let Some(p) = own.iter().find(|p| p.name == name).cloned().or_else(|| forest_core::species::zone_presets().iter().find(|p| p.name == name).map(UserPreset::from_builtin)) else { return Vec::new() };
    let mut w = p.weights.clone();
    w.sort_by(|a, b| b.1.total_cmp(&a.1));
    w.iter().filter_map(|(m, _)| app.assets.resolve(m).map(str::to_string)).collect()
}

/// The models a pick is shown with.
fn models_of(app: &App, pick: &Pick) -> Vec<String> {
    match pick {
        Pick::Model(m) => vec![m.clone()],
        Pick::Preset(id) => app.library.objects.iter().find(|p| p.id == *id).map(|p| p.items.iter().map(|i| dz_core::objects::ObjectStore::normalize_path(&i.0)).collect()).unwrap_or_default(),
        Pick::Forest(n) => forest_models(app, n),
        Pick::Road(f) => crate::library_ui::road_model(app, f).into_iter().collect(),
        Pick::Generator(_) | Pick::Modifier(_) => Vec::new(),
    }
}

fn generator(app: &App, id: u64) -> Option<&dz_core::library::GeneratorPreset> {
    app.built_in.iter().chain(app.library.generators.iter()).find(|g| g.id == id)
}

fn is_locked(app: &App, pick: &Pick) -> bool {
    match pick {
        Pick::Generator(id) => app.built_in.iter().any(|g| g.id == *id),
        Pick::Forest(n) => !app.library.forest_presets.iter().any(|p| p.name == *n),
        Pick::Road(_) | Pick::Modifier(_) => true,
        Pick::Model(_) | Pick::Preset(_) => false,
    }
}

fn name_of(app: &App, pick: &Pick) -> String {
    match pick {
        Pick::Model(m) => m.rsplit('\\').next().unwrap_or(m).trim_end_matches(".p3d").to_string(),
        Pick::Preset(id) => app.library.objects.iter().find(|p| p.id == *id).map(|p| p.name.clone()).unwrap_or_default(),
        Pick::Generator(id) => generator(app, *id).map(|g| if g.folder == BUILT_IN { t(&g.name) } else { g.name.clone() }).unwrap_or_default(),
        Pick::Forest(n) | Pick::Road(n) => n.clone(),
        Pick::Modifier(i) => t(MODIFIERS[*i].name),
    }
}

fn look_of(app: &mut App, ctx: &egui::Context, pick: &Pick, layers: &[(String, [u8; 3])]) -> Look {
    let name = name_of(app, pick);
    let locked = is_locked(app, pick);
    let models = models_of(app, pick);
    let mut images: Vec<egui::TextureId> = models.iter().take(4).filter_map(|m| app.thumbs.get(m).map(|t| t.id())).collect();
    let (mut letter, mut color, stripe, mut count, mut tooltip) = ("?", Color32::from_rgb(64, 64, 64), STRIPE_MODEL, None, name.clone());
    let mut surface = None;
    let mut profile = None;
    let stripe = match pick {
        Pick::Model(m) => {
            if let Some(s) = app.library.surface_of(m) {
                surface = Some(layers.iter().find(|l| l.0.eq_ignore_ascii_case(s)).map_or(Color32::GRAY, |l| Color32::from_rgb(l.1[0], l.1[1], l.1[2])));
                tooltip = format!("{m}\n{}", tf("Surface under it: {0}", &[s]));
            } else {
                tooltip = m.clone();
            }
            stripe
        }
        Pick::Preset(_) => {
            (letter, color) = ("O", Color32::from_rgb(64, 78, 92));
            count = Some(models.len());
            tooltip = format!("{name}\n{}", tf("{0} models", &[&models.len().to_string()]));
            STRIPE_PRESET
        }
        Pick::Forest(_) => {
            (letter, color) = ("F", Color32::from_rgb(44, 96, 56));
            count = Some(models.len());
            STRIPE_FOREST
        }
        Pick::Road(_) => {
            (letter, color) = ("R", Color32::from_rgb(70, 72, 78));
            images.truncate(1);
            STRIPE_ROAD
        }
        Pick::Generator(id) => {
            if let Some(kind) = generator(app, *id).map(|g| g.kind.clone()) {
                (letter, color) = crate::library_ui::kind_look(&kind);
                let preview = crate::library_ui::preview_of(app, &kind);
                images = crate::library_ui::preview_texture(app, ctx, &preview).into_iter().collect();
                tooltip = format!("{name}\n{}", t(kind.type_name()));
            }
            STRIPE_GENERATOR
        }
        Pick::Modifier(i) => {
            profile = Some(*i);
            color = Color32::from_rgb(110, 98, 62);
            tooltip = format!("{name}\n{}", t("Drop it on the map to add a terrain modifier there."));
            STRIPE_MODIFIER
        }
    };
    if locked {
        tooltip = format!("{tooltip}\n{}", t("Built in: it cannot be changed, only duplicated."));
    }
    Look { name, images, letter, color, stripe, locked, surface, count, tooltip, profile }
}

/// What lies in a source, folders first.
fn items_of(app: &App, source: &Source, tree: &DriveTree) -> Vec<Item> {
    let mut out = Vec::new();
    let assets = &app.assets;
    let search = app.filter.trim().to_ascii_lowercase();
    let searching = search.len() >= 2;
    let model_ok = |path: &str| -> bool {
        let plant = crate::viewport::is_plant(path);
        match app.browser.kind {
            1 => plant,
            2 => !plant,
            _ => true,
        }
    };
    let named = |n: &str| !searching || n.to_ascii_lowercase().contains(&search);
    match source {
        Source::Drive(p) => {
            if searching {
                let prefix = if p.is_empty() { String::new() } else { format!("{p}\\") };
                out.extend(assets.assets.iter().filter(|a| a.path.starts_with(&prefix) && a.path.contains(&search) && model_ok(&a.path)).take(3000).map(|a| Item::Asset(Pick::Model(a.path.clone()))));
            } else {
                out.extend(tree.children.get(p).into_iter().flatten().map(|c| Item::Folder(Source::Drive(c.clone()))));
                out.extend(assets.by_folder.get(p).into_iter().flatten().map(|&i| &assets.assets[i as usize]).filter(|a| model_ok(&a.path)).map(|a| Item::Asset(Pick::Model(a.path.clone()))));
            }
        }
        Source::Folder(n) => {
            let folders = &app.library.asset_folders;
            let inside: Vec<&dz_core::library::AssetFolder> = folders.iter().filter(|f| if searching { n.is_empty() || in_folder(&f.name, n) } else { f.name == *n }).collect();
            if !searching {
                out.extend(folders.iter().filter(|f| parent_folder(&f.name) == n && !f.name.is_empty()).map(|f| Item::Folder(Source::Folder(f.name.clone()))));
            }
            let mut seen = std::collections::HashSet::new();
            for f in inside {
                for m in &f.models {
                    if model_ok(m) && named(m) && seen.insert(Pick::Model(m.clone())) {
                        out.push(Item::Asset(Pick::Model(m.clone())));
                    }
                }
                for id in &f.presets {
                    let name = app.library.objects.iter().find(|p| p.id == *id).map(|p| p.name.as_str());
                    if name.is_some_and(named) && seen.insert(Pick::Preset(*id)) {
                        out.push(Item::Asset(Pick::Preset(*id)));
                    }
                }
            }
        }
        Source::Group(g) => {
            let sub = |gs: &[Group]| gs.iter().map(|g| Item::Folder(Source::Group(*g))).collect::<Vec<_>>();
            match g {
                Group::BuiltIn => out.extend(sub(&[Group::BuiltInGenerators, Group::BuiltInPresets, Group::BuiltInModifiers])),
                Group::Library => {
                    out.extend(sub(&[Group::LibraryGenerators, Group::LibraryPresets]));
                    out.push(Item::Folder(Source::Templates(None)));
                }
                Group::BuiltInGenerators => out.extend(app.built_in.iter().filter(|x| named(&t(&x.name))).map(|x| Item::Asset(Pick::Generator(x.id)))),
                Group::LibraryGenerators => out.extend(app.library.generators.iter().filter(|x| named(&x.name)).map(|x| Item::Asset(Pick::Generator(x.id)))),
                Group::BuiltInPresets => {
                    let own: Vec<&str> = app.library.forest_presets.iter().map(|p| p.name.as_str()).collect();
                    out.extend(dz_core::gen::forest_preset_names().into_iter().filter(|n| !own.contains(&n.as_str()) && named(n)).map(|n| Item::Asset(Pick::Forest(n))));
                    out.extend(app.road_families.iter().filter(|f| named(f)).map(|f| Item::Asset(Pick::Road(f.clone()))));
                }
                Group::LibraryPresets => {
                    out.extend(app.library.objects.iter().filter(|p| named(&p.name)).map(|p| Item::Asset(Pick::Preset(p.id))));
                    out.extend(app.library.forest_presets.iter().filter(|p| named(&p.name)).map(|p| Item::Asset(Pick::Forest(p.name.clone()))));
                }
                Group::BuiltInModifiers => out.extend((0..MODIFIERS.len()).filter(|i| named(&t(MODIFIERS[*i].name))).map(|i| Item::Asset(Pick::Modifier(i)))),
            }
        }
        Source::Templates(which) => {
            let libs = &app.browser.templates.libraries;
            match which {
                None if !searching => out.extend((0..libs.len()).map(|i| Item::Folder(Source::Templates(Some(i))))),
                _ => {
                    let mut seen = std::collections::HashSet::new();
                    for (i, (_, models)) in libs.iter().enumerate() {
                        if which.is_some_and(|w| w != i) { continue; }
                        for m in models {
                            if model_ok(m) && named(m) && seen.insert(m.as_str()) {
                                out.push(Item::Asset(Pick::Model(m.clone())));
                            }
                        }
                    }
                }
            }
        }
        Source::Surfaced => out.extend(app.library.model_surfaces.keys().filter(|m| model_ok(m) && named(m)).map(|m| Item::Asset(Pick::Model(m.clone())))),
    }
    out
}

/// What the user did this frame, done after drawing.
enum Act {
    Go(Source),
    Click(Pick, egui::Modifiers),
    Open(Pick),
    SelectAll,
    Clear,
    Surface(Vec<String>, Option<String>),
    AddTo(String, Vec<String>, Vec<u64>),
    RemoveFrom(String, Vec<String>, Vec<u64>),
    NewFolder(String),
    Rename(String),
    DeleteFolder(String),
    MakePreset(Vec<String>),
    Duplicate(Pick),
    Delete(Pick),
    IntoPreset(u64, Vec<String>),
    /// A new generator of the library, of the kind of a built-in one.
    NewGenerator(u64),
    NewPreset,
    LoadTemplates,
    /// A new template library; the models are put into it.
    NewTemplateLibrary(Vec<String>),
    AddTemplates(usize, Vec<String>),
    RemoveTemplates(usize, Vec<String>),
}

/// Menu that assigns the surface painted under models.
fn surface_menu(ui: &mut egui::Ui, layers: &[(String, [u8; 3])], current: Option<&str>) -> Option<Option<String>> {
    let mut chosen = None;
    ui.set_max_width(260.0);
    ui.weak(t("The mask colour painted under the model, by trees as their forest ground and by the tool that paints under objects."));
    if ui.selectable_label(false, t("(none)")).clicked() {
        chosen = Some(None);
        ui.close_menu();
    }
    for (name, c) in layers {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), egui::Sense::hover());
            ui.painter().rect_filled(r, 2.0, Color32::from_rgb(c[0], c[1], c[2]));
            if ui.selectable_label(current.is_some_and(|x| x.eq_ignore_ascii_case(name)), name).clicked() {
                chosen = Some(Some(name.clone()));
                ui.close_menu();
            }
        });
    }
    if layers.is_empty() {
        ui.weak(t("Open a project with surface layers to choose from them."));
    }
    chosen
}

/// Menu of the user's folders to put things into.
fn folder_menu(ui: &mut egui::Ui, folders: &[String], models: &[String], presets: &[u64], acts: &mut Vec<Act>) {
    ui.set_max_width(260.0);
    for f in folders {
        if ui.button(f.replace('/', " › ")).clicked() {
            acts.push(Act::AddTo(f.clone(), models.to_vec(), presets.to_vec()));
            ui.close_menu();
        }
    }
    if !folders.is_empty() {
        ui.separator();
    }
    if ui.button(format!("+ {}", t("New folder"))).clicked() {
        let name = t("New folder");
        acts.push(Act::AddTo(name, models.to_vec(), presets.to_vec()));
        ui.close_menu();
    }
}

/// Menu of the project's template libraries to add models to.
fn template_menu(ui: &mut egui::Ui, libraries: &[String], models: &[String], acts: &mut Vec<Act>) {
    ui.set_max_width(260.0);
    for (i, name) in libraries.iter().enumerate() {
        if ui.button(name).clicked() {
            acts.push(Act::AddTemplates(i, models.to_vec()));
            ui.close_menu();
        }
    }
    if !libraries.is_empty() {
        ui.separator();
    }
    if ui.button(format!("+ {}", t("New template library..."))).clicked() {
        acts.push(Act::NewTemplateLibrary(models.to_vec()));
        ui.close_menu();
    }
}

/// Takes models dropped on a template library.
fn take_template_drop(resp: &egui::Response, library: usize, acts: &mut Vec<Act>) {
    theme::drop_zone(resp, theme::dragging_models(&resp.ctx));
    let mut models: Vec<String> = crate::theme::take::<String>(&resp).map(|m| vec![(*m).clone()]).unwrap_or_default();
    if let Some(d) = crate::theme::take::<Dragged>(&resp) {
        models.extend(d.0.iter().filter_map(|p| if let Pick::Model(m) = p { Some(m.clone()) } else { None }));
    }
    if !models.is_empty() {
        acts.push(Act::AddTemplates(library, models));
    }
}

/// Takes what was dropped on a folder of the user.
fn take_drop(resp: &egui::Response, folder: &str, acts: &mut Vec<Act>) {
    let preset = egui::DragAndDrop::payload::<LibDrag>(&resp.ctx).is_some_and(|d| matches!(*d, LibDrag::ObjectPreset(_)));
    theme::drop_zone(resp, theme::dragging_models(&resp.ctx) || preset);
    if let Some(m) = crate::theme::take::<String>(&resp) {
        acts.push(Act::AddTo(folder.to_string(), vec![(*m).clone()], Vec::new()));
    }
    if let Some(d) = crate::theme::take::<LibDrag>(&resp) {
        if let LibDrag::ObjectPreset(id) = *d {
            acts.push(Act::AddTo(folder.to_string(), Vec::new(), vec![id]));
        }
    }
    if let Some(d) = crate::theme::take::<Dragged>(&resp) {
        let models = d.0.iter().filter_map(|p| if let Pick::Model(m) = p { Some(m.clone()) } else { None }).collect();
        let presets = d.0.iter().filter_map(|p| if let Pick::Preset(id) = p { Some(*id) } else { None }).collect();
        acts.push(Act::AddTo(folder.to_string(), models, presets));
    }
}

fn source_row(ui: &mut egui::Ui, current: &Source, s: Source, text: String, acts: &mut Vec<Act>) -> egui::Response {
    let icon = match &s {
        Source::Folder(n) if n.is_empty() => Some(crate::icons::STAR),
        Source::Templates(None) => Some(crate::icons::BOOKS),
        Source::Templates(Some(_)) => Some(crate::icons::BOOK_OPEN),
        Source::Drive(p) if p.is_empty() => Some(crate::icons::HARD_DRIVES),
        Source::Surfaced => Some(crate::icons::PALETTE),
        Source::Drive(_) | Source::Folder(_) => Some(if *current == s { crate::icons::FOLDER_OPEN } else { crate::icons::FOLDER }),
        Source::Group(Group::Library) => Some(crate::icons::BOOKS),
        Source::Group(Group::LibraryGenerators) => Some(crate::icons::MAGIC_WAND),
        Source::Group(Group::LibraryPresets) => Some(crate::icons::SQUARES_FOUR),
        _ => None,
    };
    if let Some(icon) = icon {
        ui.label(egui::RichText::new(icon).color(Color32::from_rgb(214, 178, 100)));
    }
    let width = ui.fonts(|f| f.layout_no_wrap(text.clone(), egui::TextStyle::Button.resolve(ui.style()), Color32::WHITE).size().x);
    let cut = width + ui.spacing().button_padding.x * 2.0 > ui.available_width();
    let r = ui.selectable_label(*current == s, text.clone());
    let r = if cut { r.on_hover_text(text) } else { r };
    if r.clicked() {
        acts.push(Act::Go(s));
    }
    r
}

/// A row of the sources, with a padlock before what is built in.
fn locked_row(ui: &mut egui::Ui, current: &Source, s: Source, text: String, acts: &mut Vec<Act>) -> egui::Response {
    if s.locked() {
        let (r, _) = ui.allocate_exact_size(vec2(12.0, 14.0), egui::Sense::hover());
        lock_icon(ui.painter(), r.min + vec2(0.0, 1.0), 11.0);
    }
    source_row(ui, current, s, text, acts)
}

fn drive_node(ui: &mut egui::Ui, app: &App, tree: &DriveTree, path: &str, acts: &mut Vec<Act>) {
    let label = |p: &str| {
        let n = app.assets.by_folder.get(p).map_or(0, |v| v.len());
        let leaf = p.rsplit('\\').next().unwrap_or(p);
        if n > 0 { format!("{leaf}  ({n})") } else { leaf.to_string() }
    };
    for child in tree.children.get(path).into_iter().flatten() {
        let s = Source::Drive(child.clone());
        if tree.children.contains_key(child) {
            let id = ui.make_persistent_id(("drive", child));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
                .show_header(ui, |ui| { source_row(ui, &app.browser.source, s, label(child), acts); })
                .body(|ui| drive_node(ui, app, tree, child, acts));
        } else {
            ui.horizontal(|ui| {
                ui.add_space(18.0);
                source_row(ui, &app.browser.source, s, label(child), acts);
            });
        }
    }
}

fn user_folder_row(ui: &mut egui::Ui, app: &App, name: &str, acts: &mut Vec<Act>) -> Option<String> {
    let mut renamed = None;
    if let Some((old, text)) = &app.browser.rename {
        if old == name {
            let mut text = text.clone();
            let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(140.0));
            r.request_focus();
            if r.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                acts.push(Act::Rename(text.clone()));
            }
            renamed = Some(text);
            return renamed;
        }
    }
    let leaf = name.rsplit('/').next().unwrap_or(name);
    let n = app.library.folder(name).map_or(0, |f| f.models.len() + f.presets.len());
    let r = source_row(ui, &app.browser.source, Source::Folder(name.to_string()), if n > 0 { format!("{leaf}  ({n})") } else { leaf.to_string() }, acts);
    take_drop(&r, name, acts);
    r.context_menu(|ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        if ui.button(t("New folder inside")).clicked() { acts.push(Act::NewFolder(name.to_string())); ui.close_menu(); }
        if ui.button(t("Rename")).clicked() { acts.push(Act::Rename(String::from("\0") + name)); ui.close_menu(); }
        if ui.button(t("Delete folder")).on_hover_text(t("The models and presets in it stay where they are.")).clicked() { acts.push(Act::DeleteFolder(name.to_string())); ui.close_menu(); }
    });
    renamed
}

fn user_folders(ui: &mut egui::Ui, app: &App, parent: &str, acts: &mut Vec<Act>, renamed: &mut Option<String>) {
    let names: Vec<String> = app.library.asset_folders.iter().filter(|f| parent_folder(&f.name) == parent).map(|f| f.name.clone()).collect();
    for name in names {
        let has_children = app.library.asset_folders.iter().any(|f| parent_folder(&f.name) == name);
        if has_children {
            let id = ui.make_persistent_id(("userfolder", &name));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
                .show_header(ui, |ui| { if let Some(r) = user_folder_row(ui, app, &name, acts) { *renamed = Some(r); } })
                .body(|ui| user_folders(ui, app, &name, acts, renamed));
        } else {
            ui.horizontal(|ui| {
                ui.add_space(18.0);
                if let Some(r) = user_folder_row(ui, app, &name, acts) { *renamed = Some(r); }
            });
        }
    }
}

fn group_section(ui: &mut egui::Ui, app: &App, root: Group, children: &[Group], acts: &mut Vec<Act>, extra: impl FnOnce(&mut egui::Ui, &mut Vec<Act>)) {
    let id = ui.make_persistent_id(("group", root as u8));
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
        .show_header(ui, |ui| {
            locked_row(ui, &app.browser.source, Source::Group(root), Source::Group(root).label(), acts);
        })
        .body(|ui| {
            for g in children {
                ui.horizontal(|ui| {
                    ui.add_space(18.0);
                    locked_row(ui, &app.browser.source, Source::Group(*g), Source::Group(*g).label(), acts);
                });
            }
            extra(ui, acts);
        });
}

pub fn ui(app: &mut App, ui: &mut egui::Ui) {
    let key = Arc::as_ptr(&app.assets) as usize ^ app.assets.assets.len();
    if app.browser.tree.key != key {
        app.browser.tree = DriveTree::build(&app.assets);
        app.browser.tree.key = key;
    }
    let tree = std::mem::take(&mut app.browser.tree);
    let key = app.doc.as_ref().map(|d| (d.world.template_libraries.clone(), d.world.road_project.clone()));
    if app.browser.templates.key != key {
        app.browser.templates.key = key;
        let files = project_templates(app);
        app.browser.templates.update(files);
    }
    let mut acts: Vec<Act> = Vec::new();
    let layers: Vec<(String, [u8; 3])> = app.doc.as_ref().map(|d| d.world.layers.iter().map(|l| (l.name.clone(), l.color)).collect()).unwrap_or_default();
    let mut renamed = None;

    egui::SidePanel::left("asset_sources").resizable(true).default_width(240.0).width_range(120.0..=600.0).frame(egui::Frame::new()).show_inside(ui, |ui| {
        // Long names are cut to the width of the panel, so that it can always be made narrower.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        ui.horizontal(|ui| {
            ui.menu_button(egui::RichText::new(format!("{} {}", crate::icons::PLUS, t("Add"))).color(Color32::WHITE), |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                if ui.button(t("New folder")).on_hover_text(t("A folder of your own; models and presets are put into it without moving their files.")).clicked() {
                    let parent = if let Source::Folder(n) = &app.browser.source { n.clone() } else { String::new() };
                    acts.push(Act::NewFolder(parent));
                    ui.close_menu();
                }
                if ui.button(t("New object preset")).on_hover_text(t("With the models selected in the browser.")).clicked() { acts.push(Act::NewPreset); ui.close_menu(); }
                ui.menu_button(t("New generator"), |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    for b in &app.built_in {
                        if ui.button(t(b.kind.type_name())).clicked() { acts.push(Act::NewGenerator(b.id)); ui.close_menu(); }
                    }
                });
                ui.separator();
                if ui.button(t("Road catalog...")).clicked() { app.dialog = Dialog::RoadCatalog; ui.close_menu(); }
                if ui.add_enabled(app.doc.is_some(), egui::Button::new(t("New template library..."))).on_hover_text(t("A Terrain Builder template library (.tml) of the project, with the models selected in the browser.")).clicked() { acts.push(Act::NewTemplateLibrary(app.browser.models())); ui.close_menu(); }
                if ui.add_enabled(app.doc.is_some(), egui::Button::new(t("Load template libraries..."))).clicked() { acts.push(Act::LoadTemplates); ui.close_menu(); }
            });
            if ui.button(crate::icons::ARROWS_CLOCKWISE).on_hover_text(t("Scan the work drive again")).clicked() { app.rescan_assets(); }
        });
        egui::ScrollArea::vertical().id_salt("sources").auto_shrink([false, false]).show(ui, |ui| {
            let id = ui.make_persistent_id("my_folders");
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
                .show_header(ui, |ui| {
                    let r = source_row(ui, &app.browser.source, Source::Folder(String::new()), t("My folders"), &mut acts);
                    r.context_menu(|ui| { if ui.button(t("New folder")).clicked() { acts.push(Act::NewFolder(String::new())); ui.close_menu(); } });
                })
                .body(|ui| user_folders(ui, app, "", &mut acts, &mut renamed));
            group_section(ui, app, Group::BuiltIn, &[Group::BuiltInGenerators, Group::BuiltInPresets, Group::BuiltInModifiers], &mut acts, |_, _| {});
            group_section(ui, app, Group::Library, &[Group::LibraryGenerators, Group::LibraryPresets], &mut acts, |ui, acts| {
                let libs: Vec<(String, usize)> = app.browser.templates.libraries.iter().map(|l| (l.0.clone(), l.1.len())).collect();
                let id = ui.make_persistent_id("templates");
                let header = |ui: &mut egui::Ui, acts: &mut Vec<Act>| {
                    let r = source_row(ui, &app.browser.source, Source::Templates(None), t("Project templates"), acts)
                        .on_hover_text(t("The Terrain Builder template libraries read into the project."));
                    r.context_menu(|ui| {
                        if ui.button(t("New template library...")).clicked() { acts.push(Act::NewTemplateLibrary(Vec::new())); ui.close_menu(); }
                        if ui.button(t("Load template libraries...")).clicked() { acts.push(Act::LoadTemplates); ui.close_menu(); }
                    });
                };
                if libs.is_empty() {
                    ui.horizontal(|ui| { ui.add_space(18.0); header(ui, acts); });
                } else {
                    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
                        .show_header(ui, |ui| header(ui, acts))
                        .body(|ui| {
                            for (i, (name, n)) in libs.iter().enumerate() {
                                ui.horizontal(|ui| {
                                    ui.add_space(18.0);
                                    let r = source_row(ui, &app.browser.source, Source::Templates(Some(i)), format!("{name}  ({n})"), acts);
                                    take_template_drop(&r, i, acts);
                                });
                            }
                        });
                }
            });
            source_row(ui, &app.browser.source, Source::Surfaced, t("Models with a surface"), &mut acts);
            let id = ui.make_persistent_id("work_drive");
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
                .show_header(ui, |ui| { source_row(ui, &app.browser.source, Source::Drive(String::new()), format!("{}  ({})", t("Work drive"), app.assets.assets.len()), &mut acts); })
                .body(|ui| drive_node(ui, app, &tree, "", &mut acts));
        });
    });
    if let (Some(text), Some((_, t))) = (renamed, app.browser.rename.as_mut()) {
        *t = text;
    }

    egui::SidePanel::right("asset_details").resizable(true).default_width(290.0).width_range(180.0..=520.0).frame(egui::Frame::new().inner_margin(egui::Margin { left: 6, ..Default::default() })).show_inside(ui, |ui| {
        // Long names wrap, so that they never push the panel wider than it is.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        egui::ScrollArea::vertical().id_salt("asset_details_scroll").auto_shrink([false, false]).show(ui, |ui| details(app, ui, &layers, &mut acts));
    });

    // The bar with the way back, the path and the search.
    let source = app.browser.source.clone();
    ui.horizontal(|ui| {
        // The search and the filter keep their place at the right; the path is cut short before them.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                egui::ComboBox::from_id_salt("asset_kind").width(110.0).selected_text(t(["All models", "Trees and bushes", "Other models"][app.browser.kind as usize])).show_ui(ui, |ui| {
                    for (k, n) in ["All models", "Trees and bushes", "Other models"].iter().enumerate() {
                        ui.selectable_value(&mut app.browser.kind, k as u8, t(n));
                    }
                });
                let search = ui.add(egui::TextEdit::singleline(&mut app.filter).hint_text(format!("{} {}", crate::icons::MAGNIFYING_GLASS, tf("Search in {0}...", &[&source.label()]))).desired_width((ui.available_width() * 0.35).clamp(70.0, 200.0)));
                if std::mem::take(&mut app.focus_search) { search.request_focus(); }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                if ui.add_enabled(!app.browser.back.is_empty(), egui::Button::new(crate::icons::CARET_LEFT).small()).on_hover_text(t("Back")).clicked() {
                    if let Some(s) = app.browser.back.pop() { app.browser.forward.push(std::mem::replace(&mut app.browser.source, s)); }
                }
                if ui.add_enabled(!app.browser.forward.is_empty(), egui::Button::new(crate::icons::CARET_RIGHT).small()).on_hover_text(t("Forward")).clicked() {
                    if let Some(s) = app.browser.forward.pop() { app.browser.back.push(std::mem::replace(&mut app.browser.source, s)); }
                }
                if ui.add_enabled(source.parent().is_some(), egui::Button::new(crate::icons::ARROW_UP).small()).on_hover_text(t("Up")).clicked() {
                    if let Some(p) = source.parent() { acts.push(Act::Go(p)); }
                }
                let crumbs = source.crumbs();
                let n = crumbs.len();
                for (k, (label, s)) in crumbs.into_iter().enumerate() {
                    if s.locked() && k == 0 {
                        let (r, _) = ui.allocate_exact_size(vec2(12.0, 14.0), egui::Sense::hover());
                        lock_icon(ui.painter(), r.min + vec2(0.0, 1.0), 11.0);
                    }
                    let r = ui.add(egui::Button::new(egui::RichText::new(label).strong()).frame(false));
                    if r.clicked() { acts.push(Act::Go(s.clone())); }
                    if let Source::Folder(f) = &s { if !f.is_empty() { take_drop(&r, f, &mut acts); } }
                    if k + 1 < n { ui.weak("›"); }
                }
            });
        });
    });
    ui.separator();

    let items = items_of(app, &source, &tree);
    app.browser.tree = tree;
    let picks: Vec<Pick> = items.iter().filter_map(|i| if let Item::Asset(p) = i { Some(p.clone()) } else { None }).collect();
    let user_folders: Vec<String> = app.library.asset_folders.iter().map(|f| f.name.clone()).collect();
    let template_names: Vec<String> = app.browser.templates.libraries.iter().map(|l| l.0.clone()).collect();
    let has_doc = app.doc.is_some();
    let side = app.settings.library_tile;
    let cell = vec2(side + 8.0, side + 26.0);
    let columns = (ui.available_width() / (cell.x + 4.0)).floor().max(1.0) as usize;
    let rows = items.len().div_ceil(columns);
    let area = ui.available_rect_before_wrap();
    if items.is_empty() {
        let msg = if matches!(source, Source::Templates(Some(_))) { t("Empty. Drag models here to make templates of them.") } else if matches!(source, Source::Templates(_)) { t("No template libraries in the project. Load them with + Add, or import Terrain Builder layers.") } else if matches!(source, Source::Folder(_)) { t("Empty. Drag models or presets here, or add them with the right mouse button.") } else { t("Nothing here") };
        let r = ui.centered_and_justified(|ui| ui.weak(msg)).inner;
        if let Source::Folder(f) = &source { if !f.is_empty() { take_drop(&r, f, &mut acts); } }
        if let Source::Templates(Some(i)) = &source { take_template_drop(&r, *i, &mut acts); }
    } else {
        // A click on empty space clears the selection; the menu there makes folders. It is
        // made before the tiles so that they lie over it and get their clicks.
        let bg = ui.interact(area, ui.id().with("asset_grid_bg"), egui::Sense::click());
        if bg.clicked() { acts.push(Act::Clear); }
        if let Source::Folder(f) = &source {
            let f = f.clone();
            bg.context_menu(|ui| { if ui.button(t("New folder")).clicked() { acts.push(Act::NewFolder(f.clone())); ui.close_menu(); } });
        }
        if let Source::Templates(which) = &source {
            let which = *which;
            if let Some(i) = which { take_template_drop(&bg, i, &mut acts); }
            bg.context_menu(|ui| { if ui.button(t("New template library...")).clicked() { acts.push(Act::NewTemplateLibrary(Vec::new())); ui.close_menu(); } });
        }
        egui::ScrollArea::vertical().id_salt("asset_grid").auto_shrink([false, false]).show_rows(ui, cell.y + 4.0, rows, |ui, range| {
            for r in range {
                ui.horizontal(|ui| {
                    for c in 0..columns {
                        let Some(item) = items.get(r * columns + c) else { continue };
                        let (rect, resp) = ui.allocate_exact_size(cell, egui::Sense::click_and_drag());
                        if !ui.is_rect_visible(rect) { continue; }
                        match item {
                            Item::Folder(s) => {
                                let look = Look { name: s.label(), images: Vec::new(), letter: "", color: Color32::TRANSPARENT, stripe: Color32::TRANSPARENT, locked: s.locked(), surface: None, count: None, tooltip: String::new(), profile: None };
                                draw_tile(ui, rect, side, &look, false, resp.hovered(), true);
                                if resp.double_clicked() { acts.push(Act::Go(s.clone())); }
                                if let Source::Templates(Some(i)) = s {
                                    take_template_drop(&resp, *i, &mut acts);
                                }
                                if let Source::Folder(f) = s {
                                    take_drop(&resp, f, &mut acts);
                                    let f = f.clone();
                                    resp.context_menu(|ui| {
                                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                                        if ui.button(t("Open")).clicked() { acts.push(Act::Go(Source::Folder(f.clone()))); ui.close_menu(); }
                                        if ui.button(t("Delete folder")).clicked() { acts.push(Act::DeleteFolder(f.clone())); ui.close_menu(); }
                                    });
                                }
                                let _ = resp.on_hover_text(t("Double-click to open"));
                            }
                            Item::Asset(pick) => {
                                let look = look_of(app, ui.ctx(), pick, &layers);
                                let selected = app.browser.selected.contains(pick);
                                draw_tile(ui, rect, side, &look, selected, resp.hovered(), false);
                                let resp = resp.on_hover_text(&look.tooltip);
                                if let Pick::Preset(id) = pick {
                                    theme::drop_zone(&resp, theme::dragging_models(ui.ctx()) && !selected);
                                    let mut dropped: Vec<String> = crate::theme::take::<String>(&resp).map(|m| vec![(*m).clone()]).unwrap_or_default();
                                    if let Some(d) = crate::theme::take::<Dragged>(&resp) {
                                        dropped.extend(d.0.iter().filter_map(|p| if let Pick::Model(m) = p { Some(m.clone()) } else { None }));
                                    }
                                    if !dropped.is_empty() { acts.push(Act::IntoPreset(*id, dropped)); }
                                }
                                if resp.clicked() { acts.push(Act::Click(pick.clone(), ui.input(|i| i.modifiers))); }
                                if resp.double_clicked() { acts.push(Act::Open(pick.clone())); }
                                if resp.secondary_clicked() && !selected { acts.push(Act::Click(pick.clone(), egui::Modifiers::NONE)); }
                                // Dragging a selected tile takes the whole selection along.
                                let group: Vec<Pick> = if selected && app.browser.selected.len() > 1 { app.browser.selected.clone() } else { vec![pick.clone()] };
                                if group.len() > 1 {
                                    resp.dnd_set_drag_payload(Dragged(group.clone()));
                                } else {
                                    match pick {
                                        Pick::Model(m) => resp.dnd_set_drag_payload(m.clone()),
                                        Pick::Preset(id) => resp.dnd_set_drag_payload(LibDrag::ObjectPreset(*id)),
                                        Pick::Generator(id) => resp.dnd_set_drag_payload(LibDrag::Generator(*id)),
                                        Pick::Forest(n) => resp.dnd_set_drag_payload(LibDrag::ForestPreset(n.clone())),
                                        Pick::Road(_) | Pick::Modifier(_) => resp.dnd_set_drag_payload(Dragged(group.clone())),
                                    }
                                }
                                if resp.drag_started() {
                                    // What follows the cursor: the pictures of what is dragged.
                                    let mut images = Vec::new();
                                    for p in group.iter().take(3) {
                                        images.extend(look_of(app, ui.ctx(), p, &layers).images.into_iter().take(1));
                                    }
                                    let text = if group.len() > 1 { tf("{0} items", &[&group.len().to_string()]) } else { look.name.clone() };
                                    app.browser.ghost = Some((images, text, look.color, look.letter));
                                }
                                let models: Vec<String> = group.iter().filter_map(|p| if let Pick::Model(m) = p { Some(m.clone()) } else { None }).collect();
                                let presets: Vec<u64> = group.iter().filter_map(|p| if let Pick::Preset(id) = p { Some(*id) } else { None }).collect();
                                let locked = look.locked;
                                resp.context_menu(|ui| {
                                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                                    if group.len() > 1 { ui.weak(tf("{0} selected", &[&group.len().to_string()])); }
                                    if !models.is_empty() {
                                        ui.menu_button(t("Surface under the models"), |ui| {
                                            let current = if models.len() == 1 { app.library.surface_of(&models[0]).map(str::to_string) } else { None };
                                            if let Some(s) = surface_menu(ui, &layers, current.as_deref()) { acts.push(Act::Surface(models.clone(), s)); }
                                        });
                                    }
                                    if !models.is_empty() || !presets.is_empty() {
                                        ui.menu_button(t("Add to folder"), |ui| folder_menu(ui, &user_folders, &models, &presets, &mut acts));
                                        if let Source::Folder(f) = &source {
                                            if !f.is_empty() && ui.button(t("Remove from this folder")).clicked() { acts.push(Act::RemoveFrom(f.clone(), models.clone(), presets.clone())); ui.close_menu(); }
                                        }
                                    }
                                    if !models.is_empty() && has_doc {
                                        ui.menu_button(t("Add to template library"), |ui| template_menu(ui, &template_names, &models, &mut acts));
                                        if let Source::Templates(Some(i)) = &source {
                                            if ui.button(t("Remove from this template library")).on_hover_text(t("Objects of Terrain Builder layers placed with these templates lose their model there.")).clicked() { acts.push(Act::RemoveTemplates(*i, models.clone())); ui.close_menu(); }
                                        }
                                    }
                                    if models.len() > 1 && ui.button(t("Make a preset of them")).clicked() { acts.push(Act::MakePreset(models.clone())); ui.close_menu(); }
                                    if group.len() == 1 {
                                        if let Pick::Model(m) = pick {
                                            if ui.button(t("Show in its folder")).clicked() { acts.push(Act::Go(Source::Drive(m.rsplit_once('\\').map_or("", |x| x.0).to_string()))); ui.close_menu(); }
                                            if ui.button(t("Copy the path")).clicked() { ui.ctx().copy_text(m.clone()); ui.close_menu(); }
                                        }
                                        if matches!(pick, Pick::Generator(_) | Pick::Forest(_) | Pick::Preset(_)) {
                                            if ui.button(if locked { t("Edit a copy") } else { t("Edit") }).clicked() { acts.push(if locked { Act::Duplicate(pick.clone()) } else { Act::Open(pick.clone()) }); ui.close_menu(); }
                                            if ui.button(t("Duplicate")).clicked() { acts.push(Act::Duplicate(pick.clone())); ui.close_menu(); }
                                        }
                                        if !locked && matches!(pick, Pick::Generator(_) | Pick::Forest(_) | Pick::Preset(_)) && ui.button(t("Delete")).clicked() { acts.push(Act::Delete(pick.clone())); ui.close_menu(); }
                                        if locked { locked_note(ui); }
                                    }
                                    ui.separator();
                                    if ui.button(t("Select all")).clicked() { acts.push(Act::SelectAll); ui.close_menu(); }
                                });
                            }
                        }
                    }
                });
            }
        });
    }
    if ui.rect_contains_pointer(area) && !ui.ctx().wants_keyboard_input() {
        if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::A)) { acts.push(Act::SelectAll); }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) { acts.push(Act::Clear); }
    }
    apply(app, acts, &picks);
}

fn apply(app: &mut App, acts: Vec<Act>, picks: &[Pick]) {
    for act in acts {
        match act {
            Act::Go(s) => {
                app.browser.go(s);
                app.filter.clear();
            }
            Act::Click(pick, m) => {
                let b = &mut app.browser;
                if m.shift {
                    let from = b.anchor.as_ref().and_then(|a| picks.iter().position(|p| p == a)).unwrap_or(0);
                    let to = picks.iter().position(|p| *p == pick).unwrap_or(from);
                    let (lo, hi) = (from.min(to), from.max(to));
                    if !m.command { b.selected.clear(); }
                    for p in &picks[lo..=hi] {
                        if !b.selected.contains(p) { b.selected.push(p.clone()); }
                    }
                } else if m.command {
                    if let Some(i) = b.selected.iter().position(|p| *p == pick) { b.selected.remove(i); } else { b.selected.push(pick.clone()); }
                    b.anchor = Some(pick.clone());
                } else {
                    b.selected = vec![pick.clone()];
                    b.anchor = Some(pick.clone());
                }
                b.preview_item = None;
                activate(app, &pick, !m.shift && !m.command);
            }
            Act::Open(pick) => open(app, &pick),
            Act::SelectAll => app.browser.selected = picks.to_vec(),
            Act::Clear => app.browser.selected.clear(),
            Act::Surface(models, s) => {
                for m in &models { app.library.set_surface(m, s.as_deref()); }
                app.lib_dirty = true;
                app.status = match s {
                    Some(s) => tf("Surface {0} assigned to {1} models", &[&s, &models.len().to_string()]),
                    None => tf("Surface taken from {0} models", &[&models.len().to_string()]),
                };
            }
            Act::AddTo(folder, models, presets) => {
                let folder = if app.library.folder(&folder).is_none() && folder == t("New folder") { app.library.new_folder("", &folder) } else { folder };
                app.library.add_to_folder(&folder, &models, &presets);
                app.lib_dirty = true;
                app.status = tf("Added {0} to {1}", &[&(models.len() + presets.len()).to_string(), &folder]);
            }
            Act::RemoveFrom(folder, models, presets) => {
                app.library.remove_from_folder(&folder, &models, &presets);
                app.browser.selected.clear();
                app.lib_dirty = true;
            }
            Act::NewFolder(parent) => {
                let name = app.library.new_folder(&parent, &t("New folder"));
                let leaf = name.rsplit('/').next().unwrap_or(&name).to_string();
                app.browser.rename = Some((name, leaf));
                app.lib_dirty = true;
            }
            Act::Rename(text) => {
                if let Some(old) = text.strip_prefix('\0') {
                    let leaf = old.rsplit('/').next().unwrap_or(old).to_string();
                    app.browser.rename = Some((old.to_string(), leaf));
                } else if let Some((old, _)) = app.browser.rename.take() {
                    if app.library.rename_folder(&old, &text) {
                        let parent = parent_folder(&old);
                        let new = if parent.is_empty() { text.trim().replace('/', "-") } else { format!("{parent}/{}", text.trim().replace('/', "-")) };
                        if let Source::Folder(f) = &app.browser.source {
                            if in_folder(f, &old) { app.browser.source = Source::Folder(format!("{new}{}", &f[old.len()..])); }
                        }
                        app.lib_dirty = true;
                    } else {
                        app.warn(t("A folder of that name is already there"));
                    }
                }
            }
            Act::DeleteFolder(name) => {
                app.library.delete_folder(&name);
                if let Source::Folder(f) = &app.browser.source {
                    if in_folder(f, &name) { app.browser.source = Source::Folder(parent_folder(&name).to_string()); }
                }
                app.lib_dirty = true;
            }
            Act::MakePreset(models) => {
                let id = dz_core::library::new_id();
                let folder = app.preset_folder.clone().unwrap_or_default();
                app.library.objects.push(dz_core::library::ObjectPreset { id, name: t("New preset"), folder, items: models.iter().map(|m| (m.clone(), 1.0)).collect(), scale_min: 1.0, scale_max: 1.0, random_yaw: true, align: false });
                if let Source::Folder(f) = &app.browser.source {
                    if !f.is_empty() { let f = f.clone(); app.library.add_to_folder(&f, &[], &[id]); }
                }
                app.edit_object = Some(id);
                app.browser.selected = vec![Pick::Preset(id)];
                app.lib_dirty = true;
            }
            Act::Duplicate(pick) => duplicate(app, &pick),
            Act::Delete(pick) => {
                match &pick {
                    Pick::Generator(id) => {
                        app.library.generators.retain(|g| g.id != *id);
                        if app.edit_generator == Some(*id) { app.edit_generator = None; }
                    }
                    Pick::Preset(id) => {
                        app.library.objects.retain(|p| p.id != *id);
                        for f in &mut app.library.asset_folders { f.presets.retain(|p| p != id); }
                        if app.edit_object == Some(*id) { app.edit_object = None; }
                        if app.place.preset == Some(*id) { app.place.preset = None; }
                    }
                    Pick::Forest(n) => crate::library_ui::remove_forest_preset(app, n),
                    _ => {}
                }
                app.browser.selected.retain(|p| *p != pick);
                app.lib_dirty = true;
            }
            Act::IntoPreset(id, models) => {
                if let Some(p) = app.library.objects.iter_mut().find(|p| p.id == id) {
                    for m in models {
                        if !p.items.iter().any(|i| i.0 == m) { p.items.push((m, 1.0)); }
                    }
                    app.lib_dirty = true;
                }
            }
            Act::NewGenerator(from) => {
                if let Some(b) = app.built_in.iter().find(|g| g.id == from).cloned() {
                    let id = dz_core::library::new_id();
                    app.library.generators.push(dz_core::library::GeneratorPreset { id, name: t(b.kind.type_name()), folder: String::new(), kind: b.kind });
                    app.edit_generator = Some(id);
                    app.edit_object = None;
                    app.edit_forest = None;
                    app.selected_generator = None;
                    app.browser.go(Source::Group(Group::LibraryGenerators));
                    app.browser.selected = vec![Pick::Generator(id)];
                    app.lib_dirty = true;
                }
            }
            Act::NewPreset => {
                let id = dz_core::library::new_id();
                let items = app.browser.models().into_iter().chain(app.place.model.clone()).take(64).map(|m| (m, 1.0)).collect::<Vec<_>>();
                let mut seen = std::collections::HashSet::new();
                let items = items.into_iter().filter(|i| seen.insert(i.0.clone())).collect();
                app.library.objects.push(dz_core::library::ObjectPreset { id, name: t("New preset"), folder: String::new(), items, scale_min: 1.0, scale_max: 1.0, random_yaw: true, align: false });
                if let Source::Folder(f) = &app.browser.source {
                    if !f.is_empty() { let f = f.clone(); app.library.add_to_folder(&f, &[], &[id]); }
                }
                app.edit_object = Some(id);
                app.edit_generator = None;
                app.edit_forest = None;
                app.browser.selected = vec![Pick::Preset(id)];
                app.lib_dirty = true;
            }
            Act::NewTemplateLibrary(models) => {
                let start = app.browser.templates.files.first().and_then(|f| f.parent()).map(|d| d.to_path_buf())
                    .or_else(|| app.doc.as_ref().and_then(|d| d.world.road_project.as_ref()).and_then(|p| p.parent()).map(|d| d.join("TemplateLibs")));
                let mut dialog = rfd::FileDialog::new().set_title(t("New template library")).add_filter("Terrain Builder", &["tml"]).set_file_name("New_Templates.tml");
                if let Some(d) = start.filter(|d| d.is_dir()) { dialog = dialog.set_directory(d); }
                let Some(mut path) = dialog.save_file() else { continue };
                if path.extension().is_none() { path.set_extension("tml"); }
                if path.exists() {
                    app.warn(t("A file of that name is already there; choose another name or load it."));
                    continue;
                }
                let name = path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                if let Err(e) = dz_core::fsutil::atomic_write(&path, dz_core::tv4l::new_library(&name).as_bytes()) {
                    app.error(format!("{e:#}"));
                    continue;
                }
                // The libraries shown so far, found next to the Terrain Builder project, stay too.
                let shown = app.browser.templates.files.clone();
                if let Some(d) = app.doc.as_mut() {
                    for f in shown.into_iter().chain([path.clone()]) {
                        if !d.world.template_libraries.contains(&f) { d.world.template_libraries.push(f); }
                    }
                    d.history.revision += 1;
                }
                refresh_templates(app);
                let at = app.browser.templates.files.iter().position(|f| *f == path);
                app.browser.go(Source::Templates(at));
                match at {
                    Some(i) if !models.is_empty() => change_templates(app, i, &models, true),
                    _ => app.status = tf("Template library {0} made", &[&name]),
                }
            }
            Act::AddTemplates(i, models) => change_templates(app, i, &models, true),
            Act::RemoveTemplates(i, models) => {
                change_templates(app, i, &models, false);
                app.browser.selected.clear();
            }
            Act::LoadTemplates => {
                if let Some(dir) = rfd::FileDialog::new().set_title(t("Folder with Terrain Builder template libraries")).pick_folder() {
                    let found = dz_core::tv4l::libraries_in(&dir);
                    if found.is_empty() {
                        app.warn(t("No template libraries (.tml) in that folder"));
                    } else if let Some(d) = app.doc.as_mut() {
                        for f in found {
                            if !d.world.template_libraries.contains(&f) { d.world.template_libraries.push(f); }
                        }
                        d.world.template_libraries.sort();
                        d.history.revision += 1;
                        app.browser.go(Source::Templates(None));
                    }
                }
            }
        }
    }
}

/// Reads the template libraries of the project again.
fn refresh_templates(app: &mut App) {
    app.browser.templates.key = app.doc.as_ref().map(|d| (d.world.template_libraries.clone(), d.world.road_project.clone()));
    app.browser.templates.files.clear();
    let files = project_templates(app);
    app.browser.templates.update(files);
}

/// Adds templates of models to library `i` of the project, or takes them out of it, in its file.
fn change_templates(app: &mut App, i: usize, models: &[String], add: bool) {
    let Some(path) = app.browser.templates.files.get(i).cloned() else { return };
    let text = match std::fs::read(&path) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) => {
            app.error(format!("{}: {e}", path.display()));
            return;
        }
    };
    let name = path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let (text, n) = if add {
        // Template ids are shared by every library of the project, so new ones must not repeat any.
        let mut taken: std::collections::HashSet<u32> = dz_core::tv4l::read_libraries(&app.browser.templates.files).into_keys().collect();
        let items: Vec<dz_core::tv4l::NewTemplate> = models.iter().map(|m| {
            let info = app.db.get(m);
            dz_core::tv4l::NewTemplate { file: m.clone(), bounds: info.known.then_some((info.min, info.max, info.center)) }
        }).collect();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        dz_core::tv4l::add_templates(&text, &items, &mut taken, now)
    } else {
        dz_core::tv4l::remove_templates(&text, models)
    };
    if n == 0 {
        app.status = if add { tf("The models are already in {0}", &[&name]) } else { String::new() };
        return;
    }
    if let Err(e) = dz_core::fsutil::atomic_write(&path, text.as_bytes()) {
        app.error(format!("{e:#}"));
        return;
    }
    refresh_templates(app);
    app.info(if add { tf("{0} templates added to {1}", &[&n.to_string(), &name]) } else { tf("{0} templates taken out of {1}", &[&n.to_string(), &name]) });
}

/// A plain click: what it stands for becomes the thing to place.
fn activate(app: &mut App, pick: &Pick, single: bool) {
    let models = app.browser.models();
    match pick {
        Pick::Model(m) => {
            app.place.model = Some(m.clone());
            app.place.preset = None;
            app.place.pool = if models.len() > 1 { models } else { Vec::new() };
            if single && app.mode == Mode::Select { app.mode = Mode::Place; }
        }
        Pick::Preset(id) => {
            app.place.preset = Some(*id);
            if single && app.mode == Mode::Select { app.mode = Mode::Place; }
        }
        Pick::Road(f) => {
            app.line.family = f.clone();
            app.line.action = LineAction::Road;
            if single { app.mode = Mode::Lines; }
        }
        Pick::Generator(_) | Pick::Forest(_) | Pick::Modifier(_) => {}
    }
}

/// A double click opens a preset or generator for editing; built-in ones are added to the project.
fn open(app: &mut App, pick: &Pick) {
    let clear = |app: &mut App| {
        app.edit_object = None;
        app.edit_generator = None;
        app.edit_forest = None;
        app.selected_generator = None;
    };
    match pick {
        Pick::Preset(id) => { clear(app); app.edit_object = Some(*id); }
        Pick::Generator(id) => { clear(app); app.edit_generator = Some(*id); }
        Pick::Forest(n) => { clear(app); app.edit_forest = Some(n.clone()); }
        Pick::Road(f) => { app.line.family = f.clone(); app.dialog = Dialog::RoadCatalog; }
        Pick::Model(_) | Pick::Modifier(_) => {}
    }
}

fn duplicate(app: &mut App, pick: &Pick) {
    match pick {
        Pick::Generator(id) => {
            if let Some(g) = generator(app, *id).cloned() {
                let new = dz_core::library::new_id();
                let name = format!("{} 2", if g.folder == BUILT_IN { t(&g.name) } else { g.name.clone() });
                app.library.generators.push(dz_core::library::GeneratorPreset { id: new, name, folder: if g.folder == BUILT_IN { String::new() } else { g.folder }, kind: g.kind });
                app.edit_generator = Some(new);
                app.edit_object = None;
                app.edit_forest = None;
                app.browser.selected = vec![Pick::Generator(new)];
                app.lib_dirty = true;
            }
        }
        Pick::Forest(n) => crate::library_ui::duplicate_forest_preset(app, n),
        Pick::Preset(id) => crate::library_ui::duplicate_object_preset(app, *id),
        _ => {}
    }
}

fn info_rows(ui: &mut egui::Ui, rows: &[(String, String)]) {
    for (k, v) in rows {
        ui.horizontal_wrapped(|ui| {
            ui.weak(k);
            ui.label(v);
        });
    }
}

/// The big turning view of a model.
fn preview(app: &mut App, ui: &mut egui::Ui, model: &str) {
    let ctx = ui.ctx().clone();
    let db = app.db.clone();
    let p = &mut app.browser.preview;
    p.poll(&ctx, &db);
    p.request(model, &ctx, &db);
    let w = ui.available_width().clamp(120.0, PREVIEW as f32);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, w), egui::Sense::click_and_drag());
    ui.painter().rect_filled(rect, 4.0, Color32::from_rgb(30, 32, 36));
    if resp.dragged() {
        let d = resp.drag_delta();
        p.yaw = (p.yaw - d.x * 0.6).rem_euclid(360.0);
        p.pitch = (p.pitch + d.y * 0.4).clamp(-10.0, 89.0);
    }
    if resp.double_clicked() {
        (p.yaw, p.pitch) = (35.0, 12.0);
    }
    let showing = p.shown.as_ref().is_some_and(|s| s.0 == model);
    match (&p.texture, showing) {
        (Some(tex), true) if p.failed.as_deref() != Some(model) => {
            ui.painter().image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        _ if p.failed.as_deref() == Some(model) => {
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, t("The model could not be read"), egui::FontId::proportional(12.0), theme::TEXT_DIM);
        }
        _ => {
            if let Some(tex) = app.thumbs.get(model) {
                ui.painter().image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::from_white_alpha(140));
            }
            ui.put(Rect::from_center_size(rect.center(), vec2(24.0, 24.0)), egui::Spinner::new());
        }
    }
    if p.busy { ctx.request_repaint(); }
    let _ = resp.on_hover_text(t("Drag to turn the model, double-click to reset the view."));
}

fn model_details(app: &mut App, ui: &mut egui::Ui, m: &str, layers: &[(String, [u8; 3])], acts: &mut Vec<Act>) {
    preview(app, ui, m);
    ui.add_space(4.0);
    ui.strong(m.rsplit('\\').next().unwrap_or(m).trim_end_matches(".p3d"));
    ui.add(egui::Label::new(egui::RichText::new(m).small().weak()).wrap());
    let info = app.db.get(m);
    let mut rows = Vec::new();
    if info.known {
        let (w, d, h) = (info.max[0] - info.min[0], info.max[2] - info.min[2], info.max[1] - info.min[1]);
        rows.push((t("Size"), format!("{w:.1} × {d:.1} × {h:.1} m")));
    }
    rows.push((t("Kind"), if crate::viewport::is_plant(m) { t("Tree or bush") } else { t("Object") }));
    let folders: Vec<String> = app.library.asset_folders.iter().filter(|f| f.models.iter().any(|x| x == &dz_core::library::model_key(m))).map(|f| f.name.replace('/', " › ")).collect();
    if !folders.is_empty() { rows.push((t("In folders"), folders.join(", "))); }
    info_rows(ui, &rows);
    surface_row(app, ui, &[m.to_string()], layers, acts);
    ui.horizontal_wrapped(|ui| {
        if ui.button(t("Place")).clicked() { app.place.model = Some(m.to_string()); app.place.preset = None; app.place.pool.clear(); app.mode = Mode::Place; }
        if ui.button(t("Show in its folder")).clicked() { acts.push(Act::Go(Source::Drive(m.rsplit_once('\\').map_or("", |x| x.0).to_string()))); }
    });
}

fn surface_row(app: &App, ui: &mut egui::Ui, models: &[String], layers: &[(String, [u8; 3])], acts: &mut Vec<Act>) {
    let current: Vec<Option<&str>> = models.iter().map(|m| app.library.surface_of(m)).collect();
    let same = current.windows(2).all(|w| w[0].map(str::to_ascii_lowercase) == w[1].map(str::to_ascii_lowercase));
    let shown = match (same, current.first().copied().flatten()) {
        (false, _) => t("(several)"),
        (true, Some(s)) => s.to_string(),
        (true, None) => t("(none)"),
    };
    theme::row(ui, &t("Surface under it"), |ui| {
        ui.menu_button(shown, |ui| {
            if let Some(s) = surface_menu(ui, layers, if same { current.first().copied().flatten() } else { None }) { acts.push(Act::Surface(models.to_vec(), s)); }
        }).response.on_hover_text(t("The mask colour painted under the model, by trees as their forest ground and by the tool that paints under objects."));
    });
}

/// Small pictures of models, which pick the one shown in the preview.
fn model_strip(app: &mut App, ui: &mut egui::Ui, models: &[(String, Option<f32>)]) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
        for (m, weight) in models.iter().take(60) {
            let (rect, resp) = ui.allocate_exact_size(vec2(54.0, 54.0), egui::Sense::click());
            let chosen = app.browser.preview_item.as_deref() == Some(m.as_str());
            ui.painter().rect_filled(rect, 3.0, if chosen { theme::ACCENT_SOFT } else { Color32::from_rgb(46, 46, 48) });
            if let Some(tex) = app.thumbs.get(m) {
                ui.painter().image(tex.id(), rect.shrink(2.0), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            if let Some(w) = weight {
                ui.painter().text(rect.right_bottom() - vec2(3.0, 2.0), egui::Align2::RIGHT_BOTTOM, format!("{w:.0}"), egui::FontId::proportional(10.0), Color32::WHITE);
            }
            let name = m.rsplit('\\').next().unwrap_or(m).trim_end_matches(".p3d");
            if resp.on_hover_text(name).clicked() { app.browser.preview_item = Some(m.clone()); }
        }
    });
}

fn details(app: &mut App, ui: &mut egui::Ui, layers: &[(String, [u8; 3])], acts: &mut Vec<Act>) {
    let selected = app.browser.selected.clone();
    match selected.as_slice() {
        [] => {
            ui.add_space(8.0);
            ui.weak(t("Select models or presets to see them here. Ctrl and Shift select several; drag them onto the map to place them all."));
        }
        [Pick::Model(m)] => model_details(app, ui, m, layers, acts),
        [one] => {
            let models = models_of(app, one);
            let locked = is_locked(app, one);
            let shown = app.browser.preview_item.clone().filter(|m| models.contains(m)).or_else(|| models.first().cloned());
            if let Some(m) = &shown { preview(app, ui, m); } else {
                let look = look_of(app, ui.ctx(), one, layers);
                let w = ui.available_width().clamp(120.0, 200.0);
                let (rect, _) = ui.allocate_exact_size(vec2(w, w * 0.6), egui::Sense::hover());
                draw_tile(ui, rect, w * 0.6 - 30.0, &Look { name: String::new(), ..look }, false, false, false);
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if locked { lock_icon(ui.painter(), ui.cursor().min + vec2(0.0, 2.0), 13.0); ui.add_space(16.0); }
                ui.strong(name_of(app, one));
            });
            let mut rows = Vec::new();
            match one {
                Pick::Preset(id) => {
                    if let Some(p) = app.library.objects.iter().find(|p| p.id == *id) {
                        rows.push((t("Kind"), t("Object preset")));
                        rows.push((t("Scale"), format!("{:.2} – {:.2}", p.scale_min, p.scale_max)));
                        rows.push((t("Random rotation"), if p.random_yaw { t("Yes") } else { t("No") }));
                        let items: Vec<(String, Option<f32>)> = p.items.iter().map(|i| (dz_core::objects::ObjectStore::normalize_path(&i.0), Some(i.1))).collect();
                        info_rows(ui, &rows);
                        ui.label(tf("{0} models", &[&items.len().to_string()]));
                        model_strip(app, ui, &items);
                        let paths: Vec<String> = items.into_iter().map(|i| i.0).collect();
                        if !paths.is_empty() { surface_row(app, ui, &paths, layers, acts); }
                    }
                }
                Pick::Forest(n) => {
                    rows.push((t("Kind"), t("Forest preset")));
                    info_rows(ui, &rows);
                    let items: Vec<(String, Option<f32>)> = models.iter().map(|m| (m.clone(), None)).collect();
                    ui.label(tf("{0} models", &[&items.len().to_string()]));
                    model_strip(app, ui, &items);
                    let _ = n;
                }
                Pick::Road(_) => {
                    rows.push((t("Kind"), t("Road type")));
                    info_rows(ui, &rows);
                }
                Pick::Generator(id) => {
                    if let Some(g) = generator(app, *id) {
                        rows.push((t("Kind"), t(g.kind.type_name())));
                        if g.folder != BUILT_IN && !g.folder.is_empty() { rows.push((t("Folder"), g.folder.clone())); }
                    }
                    info_rows(ui, &rows);
                    ui.weak(t("Drop it on the map or on a shape to add it to the project."));
                }
                Pick::Modifier(i) => {
                    let m = &MODIFIERS[*i];
                    rows.push((t("Kind"), t("Terrain modifier")));
                    rows.push((t("Blend"), format!("{:.0} m", m.falloff)));
                    info_rows(ui, &rows);
                    ui.weak(t("Drop it on the map to add a terrain modifier there."));
                }
                Pick::Model(_) => {}
            }
            if locked { locked_note(ui); }
            ui.horizontal_wrapped(|ui| {
                match one {
                    Pick::Preset(_) => {
                        if ui.button(t("Place")).clicked() { activate(app, one, true); app.mode = Mode::Place; }
                        if ui.button(t("Edit")).clicked() { open(app, one); }
                    }
                    Pick::Generator(id) => {
                        if ui.button(t("Add to the project")).clicked() { crate::library_ui::add_instance(app, *id, None); }
                        if ui.button(if locked { t("Edit a copy") } else { t("Edit") }).clicked() { if locked { duplicate(app, one) } else { open(app, one) } }
                    }
                    Pick::Forest(n) => {
                        if ui.button(t("Add to the project")).clicked() { crate::library_ui::add_forest(app, n, None); }
                        if ui.button(if locked { t("Edit a copy") } else { t("Edit") }).clicked() { if locked { duplicate(app, one) } else { open(app, one) } }
                    }
                    Pick::Road(_) => {
                        if ui.button(t("Draw the road")).clicked() { activate(app, one, true); }
                        if ui.button(t("Road catalog...")).clicked() { open(app, one); }
                    }
                    _ => {}
                }
            });
        }
        many => {
            let models = app.browser.models();
            let presets = app.browser.presets();
            ui.strong(tf("{0} selected", &[&many.len().to_string()]));
            ui.weak(tf("{0} models, {1} other", &[&models.len().to_string(), &(many.len() - models.len()).to_string()]));
            let shown = app.browser.preview_item.clone().filter(|m| models.contains(m));
            if let Some(m) = shown { preview(app, ui, &m); }
            let strip: Vec<(String, Option<f32>)> = models.iter().map(|m| (m.clone(), None)).collect();
            model_strip(app, ui, &strip);
            ui.add_space(4.0);
            if !models.is_empty() { surface_row(app, ui, &models, layers, acts); }
            let folders: Vec<String> = app.library.asset_folders.iter().map(|f| f.name.clone()).collect();
            ui.horizontal_wrapped(|ui| {
                if !models.is_empty() || !presets.is_empty() {
                    ui.menu_button(t("Add to folder"), |ui| folder_menu(ui, &folders, &models, &presets, acts));
                }
                if models.len() > 1 && ui.button(t("Make a preset of them")).clicked() { acts.push(Act::MakePreset(models.clone())); }
                if !models.is_empty() && app.doc.is_some() {
                    let names: Vec<String> = app.browser.templates.libraries.iter().map(|l| l.0.clone()).collect();
                    ui.menu_button(t("Add to template library"), |ui| template_menu(ui, &names, &models, acts));
                }
                if ui.button(t("Clear the selection")).clicked() { acts.push(Act::Clear); }
            });
            ui.weak(t("Drag them onto the map to place them all side by side. In place mode a click places one of the selected models at random."));
        }
    }
}

/// Draws what is being dragged next to the cursor, until it is dropped.
pub fn drag_ghost(app: &mut App, ctx: &egui::Context) {
    if !egui::DragAndDrop::has_any_payload(ctx) {
        app.browser.ghost = None;
        return;
    }
    let (Some((images, text, color, letter)), Some(p)) = (app.browser.ghost.as_ref(), ctx.pointer_latest_pos()) else { return };
    egui::Area::new(egui::Id::new("asset_drag_ghost")).fixed_pos(p + vec2(16.0, 12.0)).order(egui::Order::Tooltip).interactable(false).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
            ui.horizontal(|ui| {
                const SIDE: f32 = 56.0;
                let n = images.len().max(1);
                // Several pictures lie over each other like a fanned stack.
                let (rect, _) = ui.allocate_exact_size(vec2(SIDE + (n - 1) as f32 * 10.0, SIDE + (n - 1) as f32 * 6.0), egui::Sense::hover());
                let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                if images.is_empty() {
                    let r = Rect::from_min_size(rect.min, vec2(SIDE, SIDE));
                    ui.painter().rect_filled(r, 3.0, *color);
                    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, *letter, egui::FontId::proportional(24.0), Color32::WHITE);
                }
                for (k, img) in images.iter().enumerate().rev() {
                    let r = Rect::from_min_size(rect.min + vec2(k as f32 * 10.0, k as f32 * 6.0), vec2(SIDE, SIDE));
                    ui.painter().rect_filled(r, 3.0, Color32::from_rgb(46, 46, 50));
                    ui.painter().image(*img, r, uv, Color32::WHITE);
                    ui.painter().rect_stroke(r, 3.0, Stroke::new(1.0f32, Color32::from_gray(90)), egui::StrokeKind::Inside);
                }
                ui.label(egui::RichText::new(text).strong());
            });
        });
    });
}

/// What is being dragged, as the browser's picks.
pub fn dragged_picks(ctx: &egui::Context) -> Vec<Pick> {
    if let Some(d) = egui::DragAndDrop::payload::<Dragged>(ctx) {
        return d.0.clone();
    }
    if let Some(m) = egui::DragAndDrop::payload::<String>(ctx) {
        return vec![Pick::Model((*m).clone())];
    }
    match egui::DragAndDrop::payload::<LibDrag>(ctx).map(|d| (*d).clone()) {
        Some(LibDrag::ObjectPreset(id)) => vec![Pick::Preset(id)],
        Some(LibDrag::Generator(id)) => vec![Pick::Generator(id)],
        Some(LibDrag::ForestPreset(n)) => vec![Pick::Forest(n)],
        None => Vec::new(),
    }
}

/// Where the models of a drop stand round its point: (model, x offset, z offset).
fn drop_layout(app: &App, models: &[(String, f32)]) -> Vec<(String, f32, f32)> {
    if models.is_empty() {
        return Vec::new();
    }
    let cell = models.iter().map(|(m, scale)| { let i = app.db.get(m); let (hx, hz) = if i.known { i.footprint() } else { (2.0, 2.0) }; hx.max(hz) * 2.0 * scale.max(1.0) }).fold(1.0f32, f32::max) + 1.0;
    let columns = (models.len() as f32).sqrt().ceil() as usize;
    let rows = models.len().div_ceil(columns);
    models.iter().enumerate().map(|(k, (m, _))| {
        let (c, r) = (k % columns, k / columns);
        (m.clone(), (c as f32 - (columns - 1) as f32 * 0.5) * cell, -(r as f32 - (rows - 1) as f32 * 0.5) * cell)
    }).collect()
}

/// The outline of what is dragged over the map, where it would land.
pub fn drop_preview(app: &App, painter: &egui::Painter, v: &crate::camera::View, at: [f32; 3]) {
    let picks = dragged_picks(painter.ctx());
    let Some(d) = &app.doc else { return };
    let h = &d.world.height;
    let color = Color32::from_rgb(90, 190, 255);
    let line = |a: [f32; 3], b: [f32; 3], w: f32| {
        if let (Some(p), Some(q)) = (v.project(a), v.project(b)) {
            painter.line_segment([p, q], Stroke::new(w, color));
        }
    };
    let ring = |x: f32, z: f32, r: f32| {
        let pts: Vec<egui::Pos2> = (0..=40).filter_map(|k| {
            let a = k as f32 / 40.0 * std::f32::consts::TAU;
            let (px, pz) = (x + a.cos() * r, z + a.sin() * r);
            v.project([px, h.sample(px, pz) + 0.2, pz])
        }).collect();
        painter.add(egui::Shape::line(pts, Stroke::new(1.5f32, color)));
    };
    let models: Vec<(String, f32)> = picks.iter().filter_map(|p| match p {
        Pick::Model(m) => Some((m.clone(), app.place.scale_max)),
        Pick::Preset(id) => app.library.objects.iter().find(|x| x.id == *id).and_then(|x| x.items.first().map(|i| (i.0.clone(), x.scale_max))),
        _ => None,
    }).collect();
    // A mark at the point itself, seen also from far away where the outline is tiny.
    if let Some(c) = v.project([at[0], at[1] + 0.2, at[2]]) {
        painter.circle_stroke(c, 9.0, Stroke::new(2.0f32, color));
        painter.circle_filled(c, 2.5, color);
    }
    for (m, dx, dz) in drop_layout(app, &models) {
        let (x, z) = (at[0] + dx, at[2] + dz);
        let info = app.db.get(&m);
        let (min, max) = if info.known { (info.min, info.max) } else { ([-2.0, 0.0, -2.0], [2.0, 4.0, 2.0]) };
        let y = h.sample(x, z);
        let c = |i: usize| [x + if i & 1 == 0 { min[0] } else { max[0] }, y + if i & 2 == 0 { min[1].max(0.0) } else { max[1] }, z + if i & 4 == 0 { min[2] } else { max[2] }];
        // The footprint on the ground, filled, and the box above it.
        let foot: Vec<egui::Pos2> = [0usize, 1, 5, 4].iter().filter_map(|&i| { let p = c(i); v.project([p[0], h.sample(p[0], p[2]) + 0.1, p[2]]) }).collect();
        if foot.len() == 4 {
            painter.add(egui::Shape::convex_polygon(foot, Color32::from_rgba_unmultiplied(90, 190, 255, 45), Stroke::new(2.0f32, color)));
        }
        for (a, b) in [(0, 1), (1, 5), (5, 4), (4, 0), (2, 3), (3, 7), (7, 6), (6, 2), (0, 2), (1, 3), (4, 6), (5, 7)] {
            line(c(a), c(b), 1.0);
        }
    }
    for (k, p) in picks.iter().enumerate() {
        match p {
            Pick::Modifier(i) => {
                let tpl = &MODIFIERS[*i];
                let r = match (tpl.shape)() { ModShape::Circle { radius } => radius, ModShape::Rect { half_x, half_z } => half_x.max(half_z), _ => 10.0 };
                let x = at[0] + k as f32 * 35.0;
                ring(x, at[2], r);
                ring(x, at[2], r + tpl.falloff);
            }
            Pick::Generator(_) | Pick::Forest(_) => {
                ring(at[0], at[2], 12.0);
                line([at[0] - 16.0, at[1] + 0.2, at[2]], [at[0] + 16.0, at[1] + 0.2, at[2]], 1.5);
                line([at[0], at[1] + 0.2, at[2] - 16.0], [at[0], at[1] + 0.2, at[2] + 16.0], 1.5);
            }
            _ => {}
        }
    }
}

/// Places what was dragged out of the browser together, side by side round the point.
pub fn drop_many(app: &mut App, picks: &[Pick], x: f32, z: f32) {
    use dz_core::edit::Change;
    let mut placed: Vec<(String, (bool, bool, f32, f32))> = Vec::new();
    let defaults = (app.place.random_yaw, app.place.align, app.place.scale_min, app.place.scale_max);
    let mut modifiers = Vec::new();
    for (k, p) in picks.iter().enumerate() {
        match p {
            Pick::Model(m) => placed.push((m.clone(), defaults)),
            Pick::Preset(id) => {
                if let Some(x) = crate::library_ui::pick_from_preset(app, *id, (k as u32 + 1).wrapping_mul(2654435761)) { placed.push(x); }
            }
            Pick::Modifier(i) => modifiers.push(*i),
            Pick::Generator(id) => crate::library_ui::add_instance(app, *id, Some(crate::library_ui::Dropped { shape: None, route: None, at: [x, z] })),
            Pick::Forest(n) => crate::library_ui::add_forest(app, n, Some(crate::library_ui::Dropped { shape: None, route: None, at: [x, z] })),
            Pick::Road(f) => {
                app.line.family = f.clone();
                app.line.action = LineAction::Road;
                app.mode = Mode::Lines;
            }
        }
    }
    if !placed.is_empty() {
        let layout = drop_layout(app, &placed.iter().map(|(m, s)| (m.clone(), s.3)).collect::<Vec<_>>());
        let mut change = Vec::new();
        let mut ids = Vec::new();
        for ((m, options), (_, dx, dz)) in placed.iter().zip(layout) {
            let (px, pz) = (x + dx, z + dz);
            (app.place.random_yaw, app.place.align, app.place.scale_min, app.place.scale_max) = *options;
            if let Some((id, o)) = crate::viewport::insert_placed(app, m, px, pz) {
                change.push((id, None, Some(o)));
                ids.push(id);
            }
        }
        (app.place.random_yaw, app.place.align, app.place.scale_min, app.place.scale_max) = defaults;
        if let Some(doc) = &mut app.doc {
            if !change.is_empty() {
                doc.history.push(tf("Place {0} objects", &[&change.len().to_string()]), Change::Objects(change));
                doc.dirty.objects = true;
            }
        }
        if app.place.auto_pad { app.add_pads(&ids, app.place.pad_falloff); }
        app.selection = ids.into_iter().collect();
        app.selection_rev += 1;
    }
    for (k, i) in modifiers.iter().enumerate() {
        let tpl = &MODIFIERS[*i];
        let Some(doc) = app.doc.as_mut() else { return };
        let id = doc.world.new_modifier_id();
        let m = dz_core::world::TerrainModifier { id, name: t(tpl.name), enabled: true, x: x + k as f32 * 35.0, z, yaw: 0.0, shape: (tpl.shape)(), mode: tpl.mode, target: None, amount: tpl.amount, falloff: tpl.falloff, attached: None };
        app.set_modifiers(&t("Add terrain modifier"), |list| list.push(m));
        app.selected_modifier = Some(id);
    }
}

/// Steps of the test script: `browser drive <path>`, `browser builtin|generators|presets|modifiers`
/// `browsersel <n>`, which selects the first n things shown, and `browseradd <folder>`.
pub fn script(app: &mut App, cmd: &str, arg: &str) {
    if cmd == "browseradd" {
        let (models, presets) = (app.browser.models(), app.browser.presets());
        app.library.add_to_folder(arg, &models, &presets);
        eprintln!("GENSTATS browseradd models={} presets={} assets={}", models.len(), presets.len(), app.assets.assets.len());
        app.browser.go(Source::Folder(arg.to_string()));
        return;
    }
    if cmd == "browsertemplates" {
        if let Some(d) = app.doc.as_mut() { d.world.template_libraries = dz_core::tv4l::libraries_in(std::path::Path::new(arg)); }
        app.browser.go(Source::Templates(None));
        return;
    }
    if cmd == "browsertpladd" {
        let models = app.browser.models();
        change_templates(app, arg.parse().unwrap_or(0), &models, true);
        eprintln!("GENSTATS templates {:?}", app.browser.templates.libraries.iter().map(|l| (l.0.clone(), l.1.len())).collect::<Vec<_>>());
        return;
    }
    if cmd == "browserdrop" {
        let n: Vec<f32> = arg.split_whitespace().filter_map(|v| v.parse().ok()).collect();
        let picks = app.browser.selected.clone();
        if n.len() >= 2 { drop_many(app, &picks, n[0], n[1]); }
        return;
    }
    if cmd == "browsersel" {
        let tree = DriveTree::build(&app.assets);
        let n: usize = arg.parse().unwrap_or(1);
        let picks: Vec<Pick> = items_of(app, &app.browser.source.clone(), &tree).into_iter().filter_map(|i| if let Item::Asset(p) = i { Some(p) } else { None }).take(n).collect();
        if let Some(first) = picks.first().cloned() {
            app.browser.selected = picks;
            activate(app, &first, false);
        }
        return;
    }
    let (what, rest) = arg.split_once(' ').unwrap_or((arg, ""));
    app.browser.go(match what {
        "drive" => Source::Drive(rest.to_string()),
        "folder" => Source::Folder(rest.to_string()),
        "generators" => Source::Group(Group::BuiltInGenerators),
        "presets" => Source::Group(Group::BuiltInPresets),
        "modifiers" => Source::Group(Group::BuiltInModifiers),
        "library" => Source::Group(Group::LibraryPresets),
        "templates" => Source::Templates(rest.parse().ok()),
        _ => Source::Group(Group::BuiltIn),
    });
}

fn locked_note(ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(12.0, 14.0), egui::Sense::hover());
        lock_icon(ui.painter(), r.min + vec2(0.0, 1.0), 11.0);
        ui.weak(t("Built in: it cannot be changed, only duplicated."));
    });
}
