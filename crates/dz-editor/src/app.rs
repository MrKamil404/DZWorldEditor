//! Editor state, background tasks and file handling.
use crate::camera::Camera;
use crate::i18n::{t, tf};
use crate::models::{self, ModelLoader, Thumbnails};
use crate::render::{Renderer, Shading, TerrainLod};
use crate::settings::{data_dir, Settings};
use crate::theme;
use crate::updater::Updater;
use crate::viewport::ViewCache;
use dz_core::assets::AssetIndex;
use dz_core::edit::{Change, History, MaskStroke, SculptStroke, SculptTool, Touched};
use dz_core::gen;
use dz_core::model_info::ModelDb;
use dz_core::objects::Obj;
use dz_core::project::{self, SaveParts};
use dz_core::terrain::Rect;
use dz_core::shapes::{MapShape, ShapeKind, ShapeLayer};
use dz_core::world::{MapLine, World, WorldMeta};
use dz_core::Job;
use eframe::egui;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Select,
    Landscape,
    Paint,
    Place,
    Lines,
    Generate,
    Shapes,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShapeTool {
    Edit,
    Polygon,
    Polyline,
    Point,
}

pub struct ShapePanel {
    pub tool: ShapeTool,
    /// Vertices of the shape being drawn.
    pub points: Vec<[f32; 2]>,
    /// (layer, shape)
    pub selection: BTreeSet<(usize, usize)>,
    pub vertex: Option<usize>,
    /// Corners of the selection box being dragged, and whether it adds to the selection.
    pub boxing: Option<(egui::Pos2, egui::Pos2, bool)>,
    /// Last ground point of a drag that moves whole shapes.
    pub move_from: Option<[f32; 2]>,
    /// State before the vertex drag in progress, for undo.
    pub drag_before: Option<Vec<ShapeLayer>>,
}

impl Default for ShapePanel {
    fn default() -> Self {
        Self { tool: ShapeTool::Edit, points: Vec::new(), selection: BTreeSet::new(), vertex: None, boxing: None, move_from: None, drag_before: None }
    }
}

pub enum ShapeAction {
    New,
    Active(usize),
    Visible(usize),
    Rename(usize, String),
    Delete(usize),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Gizmo {
    Move,
    Rotate,
    Scale,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewMode {
    Perspective,
    Top,
    Split,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BottomTab {
    Content,
    Generators,
    Presets,
    Output,
    Build,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineAction {
    Road,
    Fence,
    Plain,
    Edit,
}

pub struct SculptSettings {
    pub tool: SculptTool,
    pub radius: f32,
    pub falloff: f32,
    pub strength: f32,
    pub shape: dz_core::edit::BrushShape,
    pub noise: f32,
}

pub struct PaintSettings {
    pub layer: usize,
    pub radius: f32,
    pub scatter: f32,
    pub shape: dz_core::edit::BrushShape,
    /// Paint only over this surface.
    pub only_on: Option<usize>,
    /// The next click takes the surface under the cursor.
    pub picking: bool,
}

pub struct PlaceSettings {
    pub model: Option<String>,
    pub random_yaw: bool,
    pub scale_min: f32,
    pub scale_max: f32,
    pub align: bool,
    /// Places models of this object preset instead of `model`.
    pub preset: Option<u64>,
    pub auto_pad: bool,
    pub pad_falloff: f32,
    /// Extra ground levelled around an object, in metres.
    pub pad_margin: f32,
}

pub struct LineSettings {
    pub points: Vec<[f32; 2]>,
    pub action: LineAction,
    pub family: String,
    pub grade: bool,
    pub fence_model: String,
    pub fence_spacing: f32,
    /// Search the route across the terrain, as Road Builder does.
    pub route: bool,
    pub routing: dayz_road_tool::document::RoutingSettings,
    pub rail_routing: dayz_road_tool::document::RoutingSettings,
    pub grade_width: f32,
    pub grade_falloff: f32,
    pub selected_route: Option<usize>,
    pub selected_point: Option<usize>,
    /// Waypoints before the drag in progress.
    pub point_drag: Option<Vec<[f32; 2]>>,
    /// Shape layer whose polygons the route must avoid.
    pub forbidden: Option<usize>,
}

pub struct GenPanel {
    pub terrain: gen::TerrainParams,
    pub erosion: gen::ErosionParams,
    pub thermal_iters: u32,
    pub thermal_talus: f32,
    pub terrace_step: f32,
    pub terrace_sharp: f32,
    pub rivers: u32,
    pub river_width: f32,
    pub river_depth: f32,
}

impl Default for GenPanel {
    fn default() -> Self {
        Self {
            terrain: Default::default(), erosion: Default::default(), thermal_iters: 20, thermal_talus: 40.0,
            terrace_step: 12.0, terrace_sharp: 0.6, rivers: 3, river_width: 8.0, river_depth: 3.0,
        }
    }
}

pub struct NewProject {
    pub name: String,
    pub preset: usize,
    pub base: f32,
    pub folder: String,
}

/// (terrain vertices, cell size in metres)
pub const SIZE_PRESETS: [(u32, f32); 9] = [(1024, 5.0), (1024, 10.0), (2048, 2.5), (2048, 5.0), (2048, 6.25), (2048, 7.5), (4096, 2.5), (4096, 3.75), (8192, 1.875)];

pub type Finish = Box<dyn FnOnce(&mut App) + Send>;

pub struct Task {
    pub title: String,
    pub job: Job,
    rx: Receiver<Result<Finish, String>>,
}

#[derive(Clone, Default)]
pub struct Dirty {
    pub height: bool,
    pub mask: bool,
    pub satellite: bool,
    pub objects: bool,
}

pub struct Doc {
    pub world: World,
    pub dir: Option<PathBuf>,
    pub history: History,
    pub lod: TerrainLod,
    pub dirty: Dirty,
    pub height_range: [f32; 2],
}

fn layers_len(w: &World) -> usize {
    w.shape_layers.len()
}

fn ctx_shift(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.modifiers.shift)
}

pub struct ImportDialog {
    pub plan: dz_core::import::ImportPlan,
    pub scan: dz_core::import::FolderScan,
}

/// Per-model and per-layer counts, recomputed only when the objects change.
pub struct OutlinerCache {
    pub revision: u64,
    pub layers: usize,
    pub histogram: Vec<(u32, usize)>,
    pub layer_counts: Vec<usize>,
}

pub enum LayerAction {
    New,
    Active(u16),
    Visible(u16),
    Editable(u16),
    Export(u16),
    Solo(u16),
    EditAll,
    ShowAll,
    Select(u16),
    MoveSelection(u16),
    Rename(u16, String),
    Delete(u16),
    DeleteWithObjects(u16),
}

pub enum Dialog {
    None,
    Import(Box<ImportDialog>),
    RoadCatalog,
    NewProject(NewProject),
    Settings,
    ProjectSettings,
    About,
    Confirm { text: String, action: fn(&mut App) },
}

pub struct LogLine {
    pub text: String,
    pub level: u8,
}

pub struct App {
    pub ctx: egui::Context,
    pub settings: Settings,
    pub doc: Option<Doc>,
    pub renderer: Arc<Mutex<Renderer>>,
    pub loader: ModelLoader,
    pub thumbs: Thumbnails,
    pub db: ModelDb,
    pub updater: Updater,
    pub assets: Arc<AssetIndex>,
    assets_rx: Option<Receiver<AssetIndex>>,
    pub surfaces: Vec<String>,
    layer_rx: Option<Receiver<models::LayerTexture>>,
    pub layer_averages: Vec<[u8; 3]>,
    pub road_catalog: Arc<Vec<dayz_road_tool::roads::CatalogPart>>,
    pub road_families: Vec<String>,
    road_rx: Option<Receiver<Vec<dayz_road_tool::roads::CatalogPart>>>,

    pub mode: Mode,
    pub gizmo: Gizmo,
    pub view_mode: ViewMode,
    pub shading3: Shading,
    pub shading2: Shading,
    pub wireframe: bool,
    pub show_objects: bool,
    pub show_grass: bool,
    pub show_shadows: bool,
    pub clutter_db: dz_core::clutter::ClutterDb,
    /// Per layer: (index in `clutter_models`, cumulative probability, scale min, scale max).
    pub clutter_layers: Vec<Vec<(u32, f32, f32, f32)>>,
    pub clutter_models: Vec<String>,
    pub show_water: bool,
    pub show_lines: bool,
    pub show_land_grid: bool,
    pub show_tile_grid: bool,
    pub show_contours: bool,
    pub bottom: BottomTab,

    pub sculpt: SculptSettings,
    pub paint: PaintSettings,
    pub place: PlaceSettings,
    pub line: LineSettings,
    pub gen: GenPanel,
    pub stroke: Option<SculptStroke>,
    pub mask_stroke: Option<MaskStroke>,

    pub camera: Camera,
    pub top_center: [f32; 2],
    pub top_mpp: f32,
    pub view3: ViewCache,
    pub view2: ViewCache,
    pub impostor_fits: HashMap<u32, [f32; 3]>,
    pub detail_center: [f32; 2],

    pub selection: BTreeSet<u32>,
    pub selected_modifier: Option<u64>,
    pub selected_line: Option<usize>,
    pub selection_rev: u64,
    pub drag: Option<crate::viewport::ObjectDrag>,
    pub box_select: Option<(egui::Pos2, egui::Pos2, bool)>,

    pub task: Option<Task>,
    pub log: Vec<LogLine>,
    pub status: String,
    pub dialog: Dialog,
    pub filter: String,
    pub browser_folder: Option<String>,
    pub outliner_filter: String,
    pub outliner_cache: Option<OutlinerCache>,
    pub layer_rename: Option<(u16, String)>,
    pub shape_rename: Option<(usize, String)>,
    /// Tiles found over the surface limit.
    pub tile_issues: Vec<(u32, u32, u32)>,
    /// Triangles of loaded models, for picking what is actually under the cursor.
    pub pick_meshes: HashMap<u32, Arc<crate::viewport::PickMesh>>,
    pub shape: ShapePanel,
    pub show_shapes: bool,
    pub grass_under_objects: bool,
    pub library: dz_core::library::Library,
    pub built_in: Vec<dz_core::library::GeneratorPreset>,
    /// Generator of the project shown in the details panel.
    pub selected_generator: Option<u64>,
    /// Library entries being edited.
    pub edit_generator: Option<u64>,
    pub edit_object: Option<u64>,
    pub gen_folder: Option<String>,
    pub preset_folder: Option<String>,
    pub new_folder: String,
    /// The generator or preset waiting for a satellite colour from the map.
    pub sampling: Option<u64>,
    pub lib_dirty: bool,
    /// Shape layers opened in the outliner.
    pub shape_open: BTreeSet<usize>,
    pub last_autosave: Instant,
    pub session: Option<dz_core::build::Session>,
    pub close_confirmed: bool,
    pub frame_ms: f32,
    pub autoscript: Option<crate::viewport::AutoScript>,
    shot_path: Option<String>,
    /// Input injected by automated checks.
    pub synthetic: Vec<egui::Event>,
    /// Frame times collected by automated checks.
    pub perf: Option<Vec<f32>>,
}

impl App {
    pub fn new(ctx: egui::Context) -> Self {
        let settings = Settings::load();
        crate::i18n::load_user_languages(&data_dir().join("lang"));
        crate::i18n::set_language(&settings.language);
        theme::apply(&ctx, settings.ui_scale);
        let root = settings.tools.work_drive.clone();
        let db = ModelDb::new(root.clone());
        let mut updater = Updater::default();
        if settings.check_updates {
            updater.startup();
        }
        let loader = ModelLoader::new(db.clone(), ctx.clone());
        let thumbs = Thumbnails::new(db.clone(), data_dir().join("thumbnails"), ctx.clone());
        let surfaces = models::scan_surfaces(&root);
        let camera_speed = settings.camera_speed;
        let clutter_db = dz_core::clutter::ClutterDb::load(&root);
        let mut app = Self {
            ctx, db, loader, thumbs, updater, surfaces,
            assets: Arc::new(AssetIndex::default()), assets_rx: None,
            doc: None, renderer: Default::default(), layer_rx: None, layer_averages: Vec::new(),
            road_catalog: Arc::new(Vec::new()), road_families: Vec::new(), road_rx: None,
            mode: Mode::Select, gizmo: Gizmo::Move, view_mode: ViewMode::Perspective,
            shading3: Shading::Lit, shading2: Shading::Lit, wireframe: false, show_objects: true, show_grass: true, show_shadows: true, clutter_db, clutter_layers: Vec::new(), clutter_models: Vec::new(), show_water: true, show_lines: true,
            show_land_grid: false, show_tile_grid: false, show_contours: false, bottom: BottomTab::Content,
            sculpt: SculptSettings { tool: SculptTool::Raise, radius: 60.0, falloff: 0.6, strength: 10.0, shape: Default::default(), noise: 0.0 },
            paint: PaintSettings { layer: 0, radius: 15.0, scatter: 0.3, shape: Default::default(), only_on: None, picking: false },
            place: PlaceSettings { model: None, random_yaw: true, scale_min: 1.0, scale_max: 1.0, align: false, preset: None, auto_pad: false, pad_falloff: 8.0, pad_margin: 1.0 },
            line: LineSettings { points: Vec::new(), action: LineAction::Road, family: "asf2".into(), grade: true, fence_model: String::new(), fence_spacing: 4.0, route: true, routing: Default::default(), rail_routing: dayz_road_tool::document::RoutingSettings::rail(), grade_width: 9.0, grade_falloff: 12.0, selected_route: None, selected_point: None, point_drag: None, forbidden: None },
            gen: GenPanel::default(), stroke: None, mask_stroke: None,
            camera: Camera { pos: [0.0, 300.0, 0.0], yaw: 0.8, pitch: -0.4, fov: settings.fov.clamp(20.0, 120.0).to_radians(), speed: camera_speed },
            top_center: [0.0, 0.0], top_mpp: 8.0, view3: Default::default(), view2: Default::default(),
            impostor_fits: HashMap::new(), detail_center: [f32::MIN, f32::MIN],
            selection: BTreeSet::new(), selected_modifier: None, selected_line: None, selection_rev: 0, drag: None, box_select: None,
            task: None, log: Vec::new(), status: String::new(), dialog: Dialog::None, filter: String::new(), browser_folder: None, outliner_filter: String::new(), outliner_cache: None, layer_rename: None, shape_rename: None, tile_issues: Vec::new(), pick_meshes: HashMap::new(), shape: ShapePanel::default(), show_shapes: true, grass_under_objects: false, library: dz_core::library::Library::load(&crate::library_ui::library_path()), built_in: dz_core::library::built_in_generators(), selected_generator: None, edit_generator: None, edit_object: None, gen_folder: None, preset_folder: None, new_folder: String::new(), sampling: None, lib_dirty: false, shape_open: BTreeSet::new(),
            last_autosave: Instant::now(), session: None, close_confirmed: false, frame_ms: 0.0, autoscript: None, shot_path: None, synthetic: Vec::new(), perf: None,
            settings,
        };
        app.rescan_assets();
        app.scan_roads();
        app
    }

    pub fn info(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.status = text.clone();
        self.log.push(LogLine { text, level: 0 });
    }
    pub fn warn(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.status = text.clone();
        self.log.push(LogLine { text, level: 1 });
    }
    pub fn error(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.status = text.clone();
        self.log.push(LogLine { text, level: 2 });
        self.bottom = BottomTab::Output;
    }

    pub fn rescan_assets(&mut self) {
        let (tx, rx) = channel();
        let root = self.settings.tools.work_drive.clone();
        let tops = self.settings.asset_roots.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let cancel = AtomicBool::new(false);
            let _ = tx.send(dz_core::assets::scan(&root, &tops, &cancel));
            ctx.request_repaint();
        });
        self.assets_rx = Some(rx);
    }

    /// Everything a road or track depends on besides its own waypoints.
    pub fn route_request(&self, family: String, points: Vec<[f32; 2]>, terrain_route: bool, settings: dayz_road_tool::document::RoutingSettings) -> gen::RouteRequest {
        let world = self.doc.as_ref().map(|d| &d.world);
        let forbidden = self.line.forbidden.and_then(|i| world?.shape_layers.get(i)).map(|l| l.shapes.iter().filter(|s| s.kind == ShapeKind::Polygon).map(|s| s.points.clone()).collect()).unwrap_or_default();
        let roads = world.map(|w| w.lines.iter().map(|l| l.points.clone()).chain(w.routes.iter().filter(|r| !dayz_road_tool::rails::is_track_family(&r.family)).map(|r| r.centerline.clone())).collect()).unwrap_or_default();
        gen::RouteRequest { family, points, terrain_route, settings, forbidden, roads }
    }

    /// Makes the terrain modifier of a route match its line and options. Returns what to undo.
    fn sync_route_modifier(&mut self, i: usize) -> Option<Vec<dz_core::world::TerrainModifier>> {
        let doc = self.doc.as_mut()?;
        let route = doc.world.routes.get(i)?.clone();
        let before = doc.world.modifiers.clone();
        let mut area: Option<Rect> = None;
        let at = route.modifier.and_then(|id| doc.world.modifiers.iter().position(|m| m.id == id));
        if let Some(k) = at {
            area = doc.world.modifier_rect(&doc.world.modifiers[k]);
        }
        let id = route.modifier.unwrap_or_else(|| doc.world.new_modifier_id());
        let built = route.grade.then(|| gen::road_modifier(&doc.world.base, id, &route.name, &route.centerline, route.grade_width, route.grade_falloff)).flatten();
        match (built, at) {
            (Some(m), Some(k)) => doc.world.modifiers[k] = m,
            (Some(m), None) => {
                doc.world.modifiers.push(m);
                doc.world.routes[i].modifier = Some(id);
            }
            (None, Some(k)) => {
                doc.world.modifiers.remove(k);
                doc.world.routes[i].modifier = None;
            }
            (None, None) => return None,
        }
        if let Some(m) = doc.world.modifiers.iter().find(|m| Some(m.id) == doc.world.routes[i].modifier) {
            if let Some(r) = doc.world.modifier_rect(m) { area = Some(area.map_or(r, |a| a.union(r))); }
        }
        let r = area?;
        doc.world.recompose(Some(r));
        self.height_changed(r);
        Some(before)
    }

    /// Width, blend or on/off of a route's levelling changed.
    pub fn route_grading_changed(&mut self, i: usize) {
        self.sync_route_modifier(i);
        if let Some(d) = &mut self.doc { d.history.revision += 1; }
    }

    /// Builds a route in the background. `target` is the route being re-fitted, and `before`
    /// the waypoints it goes back to when no fit is found.
    pub fn start_route(&mut self, req: gen::RouteRequest, target: Option<usize>, before: Option<Vec<[f32; 2]>>, grade: bool) {
        // New routes take the option from the tool; existing ones keep their own.
        let Some(doc) = &self.doc else { return };
        let mut lite = doc.world.lite();
        let catalog = self.road_catalog.clone();
        self.spawn(t("Building road"), move |job| {
            let built = gen::build_route(&mut lite, &req, &catalog, job);
            let models: Vec<String> = lite.objects.models.iter().map(|m| m.path.clone()).collect();
            Ok(Box::new(move |app: &mut App| match built {
                Ok(road) => app.store_route(req, road, models, target, before, grade),
                Err(e) => {
                    if let (Some(i), Some(points), Some(d)) = (target, before, &mut app.doc) {
                        if let Some(r) = d.world.routes.get_mut(i) { r.points = points; }
                    }
                    app.error(format!("{e:#}"));
                }
            }) as Finish)
        });
    }

    fn store_route(&mut self, req: gen::RouteRequest, road: gen::RoadResult, models: Vec<String>, target: Option<usize>, before: Option<Vec<[f32; 2]>>, grade: bool) {
        let Some(doc) = &mut self.doc else { return };
        let mut previous = doc.world.routes.clone();
        let previous_lines = doc.world.lines.clone();
        let old = target.and_then(|i| doc.world.routes.get(i)).map(|r| r.objects.clone()).unwrap_or_default();
        let layer = old.first().and_then(|id| doc.world.objects.get(*id)).map(|o| o.layer()).unwrap_or_else(|| doc.world.target_layer());
        let mut change = Vec::with_capacity(old.len() + road.objects.len());
        for id in old {
            if let Some(o) = doc.world.objects.remove(id) {
                change.push((id, Some(o), None));
            }
        }
        let map: Vec<u32> = models.iter().map(|m| doc.world.objects.intern(m)).collect();
        let mut ids = Vec::with_capacity(road.objects.len());
        for mut o in road.objects {
            o.model = map[o.model as usize];
            o.set_layer(layer);
            let id = doc.world.objects.insert(o);
            change.push((id, None, Some(o)));
            ids.push(id);
        }
        let center = road.centerline;
        let index = match target.filter(|i| *i < doc.world.routes.len()) {
            Some(i) => {
                if let (Some(p), Some(points)) = (previous.get_mut(i), before) { p.points = points; }
                let r = &mut doc.world.routes[i];
                (r.points, r.centerline, r.objects, r.family, r.terrain_route, r.settings) = (req.points, center.clone(), ids, req.family, req.terrain_route, req.settings);
                i
            }
            None => {
                let n = doc.world.routes.len() + 1;
                let kind = if dayz_road_tool::rails::is_track_family(&req.family) { "Track" } else { "Road" };
                doc.world.routes.push(dz_core::world::RoadRoute { name: format!("{kind} {n}"), family: req.family, points: req.points, centerline: center.clone(), objects: ids, terrain_route: req.terrain_route, settings: req.settings, grade, grade_width: self.line.grade_width, grade_falloff: self.line.grade_falloff, modifier: None });
                doc.world.routes.len() - 1
            }
        };
        let label = t("Build road");
        doc.history.push(label.clone(), Change::Objects(change));
        doc.history.push(label.clone(), Change::Routes(previous, previous_lines));
        doc.dirty.objects = true;
        self.line.selected_route = Some(index);
        let parts = match self.sync_route_modifier(index) {
            Some(before) => {
                self.doc.as_mut().unwrap().history.push(label.clone(), Change::Modifiers(before));
                3
            }
            None => 2,
        };
        self.doc.as_mut().unwrap().history.group_last(parts, label);
    }

    /// Fits the models of a route again after its waypoints or settings changed.
    pub fn refit_route(&mut self, i: usize, before: Vec<[f32; 2]>) {
        let Some(r) = self.doc.as_ref().and_then(|d| d.world.routes.get(i)).cloned() else { return };
        if self.task.is_some() {
            if let Some(d) = &mut self.doc { d.world.routes[i].points = before; }
            self.warn(t("Another task is still running"));
            return;
        }
        let req = self.route_request(r.family, r.points, r.terrain_route, r.settings);
        self.start_route(req, Some(i), Some(before), false);
    }

    pub fn delete_route(&mut self, i: usize) {
        let Some(doc) = &mut self.doc else { return };
        if i >= doc.world.routes.len() { return; }
        let (previous, previous_lines) = (doc.world.routes.clone(), doc.world.lines.clone());
        let route = doc.world.routes.remove(i);
        let change = route.objects.iter().filter_map(|id| doc.world.objects.remove(*id).map(|o| (*id, Some(o), None))).collect();
        doc.history.push(t("Delete route"), Change::Objects(change));
        doc.history.push(t("Delete route"), Change::Routes(previous, previous_lines));
        let mut parts = 2;
        if let Some(k) = route.modifier.and_then(|id| doc.world.modifiers.iter().position(|m| m.id == id)) {
            let before = doc.world.modifiers.clone();
            let area = doc.world.modifier_rect(&doc.world.modifiers[k]);
            doc.world.modifiers.remove(k);
            doc.history.push(t("Delete route"), Change::Modifiers(before));
            parts = 3;
            if let Some(r) = area {
                doc.world.recompose(Some(r));
                self.height_changed(r);
            }
        }
        let Some(doc) = &mut self.doc else { return };
        doc.history.group_last(parts, t("Delete route"));
        doc.dirty.objects = true;
        self.line.selected_route = None;
        self.line.selected_point = None;
        self.selection.retain(|id| self.doc.as_ref().is_some_and(|d| d.world.objects.get(*id).is_some()));
        self.selection_rev += 1;
    }

    /// Road parts lying on a line that no route owns.
    fn parts_along(world: &World, line: &MapLine) -> Vec<u32> {
        let (mut x0, mut z0, mut x1, mut z1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in &line.points {
            (x0, z0, x1, z1) = (x0.min(p[0]), z0.min(p[1]), x1.max(p[0]), z1.max(p[1]));
        }
        let distance = |x: f32, z: f32| line.points.windows(2).map(|w| {
            let (dx, dz) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
            let k = (((x - w[0][0]) * dx + (z - w[0][1]) * dz) / (dx * dx + dz * dz).max(1e-3)).clamp(0.0, 1.0);
            ((w[0][0] + dx * k - x).powi(2) + (w[0][1] + dz * k - z).powi(2)).sqrt()
        }).fold(f32::MAX, f32::min);
        let taken: BTreeSet<u32> = world.routes.iter().flat_map(|r| r.objects.iter().copied()).collect();
        let mut ids = Vec::new();
        world.objects.for_each_in(x0 - 3.0, z0 - 3.0, x1 + 3.0, z1 + 3.0, |id, o| {
            if !taken.contains(&id) && world.objects.model_path(o).contains("\\roads\\parts\\") && distance(o.x, o.z) < 1.5 {
                ids.push(id);
            }
        });
        ids
    }

    /// Deletes an imported road: its line and the parts lying on it.
    pub fn delete_line(&mut self, i: usize) {
        let Some(doc) = &mut self.doc else { return };
        let Some(line) = doc.world.lines.get(i).cloned() else { return };
        let ids = Self::parts_along(&doc.world, &line);
        let (routes, lines) = (doc.world.routes.clone(), doc.world.lines.clone());
        let change = ids.iter().filter_map(|id| doc.world.objects.remove(*id).map(|o| (*id, Some(o), None))).collect();
        doc.world.lines.remove(i);
        doc.history.push(t("Delete route"), Change::Objects(change));
        doc.history.push(t("Delete route"), Change::Routes(routes, lines));
        doc.history.group_last(2, t("Delete route"));
        doc.dirty.objects = true;
        self.selected_line = None;
        self.selection.retain(|id| !ids.contains(id));
        self.selection_rev += 1;
    }

    /// Turns an imported road line into an editable route that owns the parts lying on it.
    /// The parts stay exactly as they are until a waypoint is changed.
    pub fn line_to_route(&mut self, i: usize) {
        let Some(doc) = &mut self.doc else { return };
        let Some(line) = doc.world.lines.get(i).cloned() else { return };
        let ids = Self::parts_along(&doc.world, &line);
        let world = &doc.world;
        let Some(first) = ids.first().and_then(|id| world.objects.get(*id)) else {
            self.warn(t("No road parts lie along this line"));
            return;
        };
        let family = dayz_road_tool::road_colors::family(world.objects.model_path(first));
        let (previous, previous_lines) = (doc.world.routes.clone(), doc.world.lines.clone());
        // Terrain Builder roads may turn tighter than new ones are allowed to.
        let settings = dayz_road_tool::document::RoutingSettings { min_radius: 10.0, ..Default::default() };
        doc.world.routes.push(dz_core::world::RoadRoute { name: line.name.clone(), family, points: gen::simplify_line(&line.points, 2.0), centerline: line.points, objects: ids, terrain_route: false, settings, grade: false, grade_width: 9.0, grade_falloff: 12.0, modifier: None });
        doc.world.lines.remove(i);
        doc.history.push(t("Make editable route"), Change::Routes(previous, previous_lines));
        doc.dirty.objects = true;
        self.selected_line = None;
        self.line.selected_route = Some(doc.world.routes.len() - 1);
        self.line.selected_point = None;
        self.line.action = LineAction::Edit;
        self.mode = Mode::Lines;
    }

    /// Road types defined by the map's Terrain Builder project, when it has one.
    pub fn load_road_types(&mut self, project: PathBuf) {
        self.spawn(t("Reading road types"), move |_| {
            let mut catalog = gen::road_catalog_from_project(&project, Path::new("P:\\dz\\structures\\roads\\parts"))?;
            anyhow::ensure!(!catalog.is_empty(), "{} defines no usable road types", project.display());
            catalog.extend(gen::rail_catalog());
            Ok(Box::new(move |app: &mut App| {
                let mut catalog = catalog;
                if let Some(d) = &app.doc {
                    gen::apply_catalog_edits(&mut catalog, &d.world.road_removed, &d.world.road_added, Path::new("P:\\dz\\structures\\roads\\parts"));
                }
                app.road_families = gen::road_families(&catalog);
                if !app.road_families.contains(&app.line.family) {
                    app.line.family = app.road_families.first().cloned().unwrap_or_default();
                }
                app.road_catalog = Arc::new(catalog);
                if let Some(d) = &mut app.doc { d.world.road_project = Some(project); d.history.revision += 1; }
            }) as Finish)
        });
    }

    fn scan_roads(&mut self) {
        let (tx, rx) = channel();
        let folder = self.settings.tools.work_drive.join("dz/structures/roads/parts");
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let mut catalog = gen::road_catalog(&folder).unwrap_or_default();
            catalog.extend(gen::rail_catalog());
            let _ = tx.send(catalog);
            ctx.request_repaint();
        });
        self.road_rx = Some(rx);
    }

    /// Starts a background task; `f` returns what to do with its result on the UI thread.
    pub fn spawn(&mut self, title: impl Into<String>, f: impl FnOnce(&Job) -> anyhow::Result<Finish> + Send + 'static) {
        let title = title.into();
        if self.task.is_some() {
            self.warn(t("Another task is still running"));
            return;
        }
        let job = Job::default();
        let (tx, rx) = channel();
        let (j, ctx) = (job.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&j)));
            let r = match r {
                Ok(r) => r.map_err(|e| format!("{e:#}")),
                Err(_) => Err("Internal error".to_string()),
            };
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        self.task = Some(Task { title, job, rx });
    }

    pub fn tick(&mut self) {
        let mut messages = Vec::new();
        let mut finished = None;
        if let Some(task) = &self.task {
            messages = task.job.take_log();
            match task.rx.try_recv() {
                Ok(r) => finished = Some((r, task.title.clone())),
                Err(std::sync::mpsc::TryRecvError::Empty) => self.ctx.request_repaint_after(Duration::from_millis(100)),
                Err(_) => finished = Some((Err("Task stopped".to_string()), task.title.clone())),
            }
        }
        for m in messages {
            self.log.push(LogLine { text: m, level: 0 });
        }
        if let Some((r, title)) = finished {
            self.task = None;
            match r {
                Ok(f) => {
                    f(self);
                    if self.status.is_empty() {
                        self.status = title;
                    }
                }
                Err(e) => self.error(format!("{title}: {e}")),
            }
        }
        if self.assets_rx.as_ref().is_some_and(|r| {
            if let Ok(a) = r.try_recv() {
                self.assets = Arc::new(a);
                true
            } else {
                false
            }
        }) {
            self.assets_rx = None;
        }
        if let Some(r) = &self.road_rx {
            if let Ok(c) = r.try_recv() {
                self.road_families = gen::road_families(&c);
                self.road_catalog = Arc::new(c);
                self.road_rx = None;
            }
        }
        let mut layer_updates = Vec::new();
        if let Some(rx) = &self.layer_rx {
            while let Ok(l) = rx.try_recv() {
                layer_updates.push(l);
            }
        }
        if !layer_updates.is_empty() {
            let mut r = self.renderer.lock().unwrap();
            for l in layer_updates {
                if self.layer_averages.len() <= l.index {
                    self.layer_averages.resize(l.index + 1, [128, 128, 128]);
                }
                self.layer_averages[l.index] = l.average;
                r.set_layer(l.index, 512, l.rgba);
            }
            drop(r);
            self.push_palette();
        }
        // Models arriving from the workers.
        {
            let (renderer, fits, picks) = (self.renderer.clone(), &mut self.impostor_fits, &mut self.pick_meshes);
            self.loader.poll(|m| {
                crate::viewport::remember_pick_mesh(picks, &m);
                if m.impostor.is_some() {
                    fits.insert(m.key.0, m.impostor_fit);
                }
                renderer.lock().unwrap().add_mesh(m);
            });
        }
        self.thumbs.poll(&self.ctx.clone());
        self.updater.tick(&self.ctx.clone());
        if let Some(s) = &mut self.session {
            if let Ok(Some(_)) = s.server.try_wait() {
                self.session = None;
                self.info(t("Test session ended"));
            }
        }
        let every = self.settings.autosave_minutes;
        if every > 0 && self.last_autosave.elapsed() > Duration::from_secs(every as u64 * 60) {
            self.last_autosave = Instant::now();
            if self.doc.as_ref().is_some_and(|d| d.history.dirty() && d.dir.is_some()) && self.stroke.is_none() {
                self.save(None);
            }
        }
    }

    pub fn push_palette(&mut self) {
        let Some(doc) = &self.doc else { return };
        let colors: Vec<[u8; 3]> = doc.world.layers.iter().map(|l| l.color).collect();
        let mut avg = self.layer_averages.clone();
        avg.resize(colors.len().max(avg.len()), [128, 128, 128]);
        self.renderer.lock().unwrap().set_palette(colors, avg);
    }

    pub fn reload_layer_textures(&mut self) {
        let Some(doc) = &self.doc else { return };
        // Ground clutter of each surface, as the game configures it.
        let mut models: Vec<String> = Vec::new();
        self.clutter_layers = doc.world.layers.iter().map(|l| {
            self.clutter_db.for_texture(&l.texture).into_iter().map(|e| {
                let i = models.iter().position(|m| *m == e.model).unwrap_or_else(|| {
                    models.push(e.model.clone());
                    models.len() - 1
                });
                (i as u32, e.threshold, e.scale_min, e.scale_max)
            }).collect()
        }).collect();
        self.clutter_models = models;
        let list: Vec<_> = doc.world.layers.iter().enumerate().take(64).map(|(i, l)| (i, l.texture.clone(), l.color)).collect();
        self.layer_rx = Some(models::load_layer_textures(self.settings.tools.work_drive.clone(), list, self.ctx.clone()));
    }

    /// Makes `world` the open document.
    pub fn open_world(&mut self, world: World, dir: Option<PathBuf>) {
        let lod = TerrainLod::build(&world.height);
        let (lo, hi) = world.height.min_max();
        let side = world.meta.size_m();
        {
            let mut r = self.renderer.lock().unwrap();
            r.reset_terrain(&world.height);
            r.set_mask(&world.mask);
            r.set_satellite(world.satellite.as_ref());
        }
        self.loader.reset();
        self.impostor_fits.clear();
        self.layer_averages.clear();
        self.detail_center = [f32::MIN, f32::MIN];
        self.selection.clear();
        self.selected_modifier = None;
        self.selected_line = None;
        self.selection_rev += 1;
        self.top_center = [side * 0.5, side * 0.5];
        self.top_mpp = side / 900.0;
        let ground = world.height.sample(side * 0.5, side * 0.5).max(0.0);
        self.camera.pos = [side * 0.35, ground + side * 0.09, side * 0.3];
        self.camera.look_at([side * 0.5, ground, side * 0.52]);
        self.line.points.clear();
        self.shape.selection.clear();
        self.shape.points.clear();
        self.line.selected_route = None;
        self.line.selected_point = None;
        self.line.point_drag = None;
        self.pick_meshes.clear();
        self.selected_generator = None;
        self.sampling = None;
        let road_project = world.road_project.clone().filter(|p| p.is_file());
        if let Some(d) = &dir {
            self.settings.remember(d.clone());
            self.settings.save();
        }
        let name = world.meta.name.clone();
        self.paint.layer = 0;
        self.doc = Some(Doc { world, dir, history: History::default(), lod, dirty: Dirty::default(), height_range: [lo, hi] });
        self.view3 = Default::default();
        self.view2 = Default::default();
        self.push_palette();
        self.reload_layer_textures();
        self.info(tf("Opened {0}", &[&name]));
        if let Some(p) = road_project {
            self.load_road_types(p);
        }
    }

    pub fn create_project(&mut self, p: &NewProject) {
        let (n, cell) = SIZE_PRESETS[p.preset];
        let mut meta = WorldMeta::new(&p.name, n, cell);
        meta.author = String::new();
        let world = World::new(meta, p.base);
        let dir = (!p.folder.trim().is_empty()).then(|| PathBuf::from(p.folder.trim()).join(&world.meta.map_id));
        if let Some(d) = &dir {
            if let Err(e) = project::save(&world, d, SaveParts::ALL) {
                self.error(format!("{e:#}"));
                return;
            }
        }
        self.open_world(world, dir);
    }

    pub fn open_project(&mut self, dir: PathBuf) {
        let d2 = dir.clone();
        self.spawn(t("Opening project"), move |_| {
            let w = project::load(&d2)?;
            Ok(Box::new(move |app: &mut App| app.open_world(w, Some(d2))) as Finish)
        });
    }

    pub fn save(&mut self, target: Option<PathBuf>) {
        let Some(doc) = &mut self.doc else { return };
        let same = target.is_none();
        let Some(dir) = target.or_else(|| doc.dir.clone()) else {
            self.save_as();
            return;
        };
        let parts = if same && doc.dir.is_some() {
            SaveParts { height: doc.dirty.height, mask: doc.dirty.mask, satellite: doc.dirty.satellite, objects: doc.dirty.objects }
        } else {
            SaveParts::ALL
        };
        match project::save(&doc.world, &dir, parts) {
            Ok(()) => {
                doc.dirty = Dirty::default();
                doc.dir = Some(dir.clone());
                doc.history.mark_saved();
                self.settings.remember(dir.clone());
                self.settings.save();
                self.info(tf("Saved {0}", &[&dir.display().to_string()]));
            }
            Err(e) => self.error(format!("{e:#}")),
        }
    }

    pub fn save_as(&mut self) {
        if let Some(d) = rfd::FileDialog::new().set_title(t("Choose a folder for the project")).pick_folder() {
            self.save(Some(d));
        }
    }

    pub fn pick_and_open(&mut self) {
        if let Some(f) = rfd::FileDialog::new().add_filter("DZ World Editor", &["dzwe"]).pick_file() {
            self.open_project(project::project_dir_of(&f));
        }
    }

    pub fn undo(&mut self) {
        let Some(doc) = &mut self.doc else { return };
        if let Some(t) = doc.history.undo(&mut doc.world) {
            self.apply_touched(t);
        }
    }
    pub fn redo(&mut self) {
        let Some(doc) = &mut self.doc else { return };
        if let Some(t) = doc.history.redo(&mut doc.world) {
            self.apply_touched(t);
        }
    }

    pub fn apply_touched(&mut self, t: Touched) {
        if let Some(r) = t.height {
            self.height_changed(r);
        }
        if let Some(r) = t.mask {
            self.mask_changed(r);
        }
        if t.objects {
            if let Some(d) = &mut self.doc {
                d.dirty.objects = true;
                let existing: Vec<u32> = self.selection.iter().copied().filter(|id| d.world.objects.get(*id).is_some()).collect();
                self.selection = existing.into_iter().collect();
            }
            self.selection_rev += 1;
        }
    }

    pub fn height_changed(&mut self, r: Rect) {
        let Some(doc) = &mut self.doc else { return };
        doc.lod.update(&doc.world.height, r);
        doc.dirty.height = true;
        self.renderer.lock().unwrap().update_height(&doc.world.height, r);
    }

    pub fn mask_changed(&mut self, r: Rect) {
        let Some(doc) = &mut self.doc else { return };
        doc.dirty.mask = true;
        self.renderer.lock().unwrap().update_mask(&doc.world.mask, r);
    }

    pub fn whole_height_changed(&mut self) {
        let Some(doc) = &mut self.doc else { return };
        doc.lod = TerrainLod::build(&doc.world.height);
        let (a, b) = doc.world.height.min_max();
        doc.height_range = [a, b];
        doc.dirty.height = true;
        let r = doc.world.height.full_rect();
        self.renderer.lock().unwrap().update_height(&doc.world.height, r);
    }

    pub fn whole_mask_changed(&mut self) {
        let Some(doc) = &mut self.doc else { return };
        doc.dirty.mask = true;
        self.renderer.lock().unwrap().set_mask(&doc.world.mask);
    }

    /// Adds objects made by a generator on a `lite` copy of the world.
    pub fn merge_generated(&mut self, label: &str, models: Vec<String>, objects: Vec<Obj>) -> Vec<u32> {
        let Some(doc) = &mut self.doc else { return Vec::new() };
        let map: Vec<u32> = models.iter().map(|m| doc.world.objects.intern(m)).collect();
        let mut change = Vec::with_capacity(objects.len());
        let mut ids = Vec::with_capacity(objects.len());
        let layer = doc.world.target_layer();
        for mut o in objects {
            o.model = map[o.model as usize];
            o.set_layer(layer);
            let id = doc.world.objects.insert(o);
            change.push((id, None, Some(o)));
            ids.push(id);
        }
        doc.history.push(label, Change::Objects(change));
        doc.dirty.objects = true;
        ids
    }

    /// Replaces the base heights with generator output.
    pub fn apply_heights(&mut self, label: &str, data: Vec<f32>, lines: Vec<MapLine>) {
        let Some(doc) = &mut self.doc else { return };
        let mut old = data;
        std::mem::swap(&mut old, &mut doc.world.base.data);
        doc.world.recompose(None);
        doc.world.lines.extend(lines);
        doc.history.push(label, Change::WholeHeight(old));
        self.whole_height_changed();
        self.info(label.to_string());
    }

    pub fn apply_mask(&mut self, label: &str, data: Vec<u8>) {
        let Some(doc) = &mut self.doc else { return };
        let mut old = data;
        std::mem::swap(&mut old, &mut doc.world.mask.data);
        doc.history.push(label, Change::WholeMask(old));
        self.whole_mask_changed();
        self.info(label.to_string());
    }

    pub fn delete_selection(&mut self) {
        let Some(doc) = &mut self.doc else { return };
        let mut change = Vec::new();
        let mut area: Option<Rect> = None;
        for id in std::mem::take(&mut self.selection) {
            if let Some(o) = doc.world.objects.remove(id) {
                change.push((id, Some(o), None));
                // A modifier stays only as long as its object.
                let mut k = 0;
                while k < doc.world.modifiers.len() {
                    if doc.world.modifiers[k].attached == Some(id) {
                        let m = doc.world.modifiers.remove(k);
                        if let Some(r) = doc.world.base.rect_around(m.x, m.z, m.reach()) {
                            area = Some(area.map_or(r, |a| a.union(r)));
                        }
                    } else {
                        k += 1;
                    }
                }
            }
        }
        if !change.is_empty() {
            doc.history.push(t("Delete objects"), Change::Objects(change));
            doc.dirty.objects = true;
        }
        if let Some(r) = area {
            doc.world.recompose(Some(r));
            self.height_changed(r);
        }
        self.selection_rev += 1;
    }

    pub fn duplicate_selection(&mut self) {
        let Some(doc) = &mut self.doc else { return };
        let mut change = Vec::new();
        let mut new = BTreeSet::new();
        for id in &self.selection {
            let Some(mut o) = doc.world.objects.get(*id).copied() else { continue };
            o.x += 2.0;
            o.z += 2.0;
            let n = doc.world.objects.insert(o);
            change.push((n, None, Some(o)));
            new.insert(n);
        }
        if !change.is_empty() {
            doc.history.push(t("Duplicate objects"), Change::Objects(change));
            doc.dirty.objects = true;
            self.selection = new;
            self.selection_rev += 1;
        }
    }

    pub fn set_modifiers(&mut self, label: &str, f: impl FnOnce(&mut Vec<dz_core::world::TerrainModifier>)) {
        let Some(doc) = &mut self.doc else { return };
        let before = doc.world.modifiers.clone();
        f(&mut doc.world.modifiers);
        doc.world.recompose(None);
        doc.history.push(label, Change::Modifiers(before));
        self.whole_height_changed();
    }

    pub fn add_pads_for_selection(&mut self, falloff: f32) {
        let ids: Vec<u32> = self.selection.iter().copied().collect();
        self.add_pads(&ids, falloff);
    }

    /// Applies the margin and blend to the modifiers of the selected objects.
    pub fn update_selected_pads(&mut self) {
        let (margin, falloff) = (self.place.pad_margin, self.place.pad_falloff);
        let Some(doc) = &mut self.doc else { return };
        let mut area: Option<Rect> = None;
        for id in &self.selection {
            let Some(o) = doc.world.objects.get(*id).copied() else { continue };
            let info = self.db.get(doc.world.objects.model_path(&o));
            let (hx, hz) = if info.known { info.footprint() } else { (4.0, 4.0) };
            for k in 0..doc.world.modifiers.len() {
                if doc.world.modifiers[k].attached != Some(*id) { continue; }
                let before = doc.world.modifier_rect(&doc.world.modifiers[k]);
                doc.world.modifiers[k].shape = dz_core::world::ModShape::Rect { half_x: hx * o.scale + margin, half_z: hz * o.scale + margin };
                doc.world.modifiers[k].falloff = falloff;
                let after = doc.world.modifier_rect(&doc.world.modifiers[k]);
                for r in [before, after].into_iter().flatten() { area = Some(area.map_or(r, |a| a.union(r))); }
            }
        }
        if let Some(r) = area {
            doc.world.recompose(Some(r));
            doc.history.revision += 1;
            self.height_changed(r);
        }
    }

    pub fn add_pads(&mut self, ids: &[u32], falloff: f32) {
        let Some(doc) = &mut self.doc else { return };
        let before = doc.world.modifiers.clone();
        let mut added = 0;
        for id in ids {
            if doc.world.modifiers.iter().any(|m| m.attached == Some(*id)) {
                continue;
            }
            let Some(o) = doc.world.objects.get(*id).copied() else { continue };
            let info = self.db.get(doc.world.objects.model_path(&o));
            let (hx, hz) = if info.known { info.footprint() } else { (4.0, 4.0) };
            if let Some(m) = gen::pad_modifier(&mut doc.world, *id, hx * o.scale, hz * o.scale, self.place.pad_margin, falloff) {
                doc.world.modifiers.push(m);
                added += 1;
            }
        }
        if added > 0 {
            doc.world.recompose(None);
            doc.history.push(t("Add terrain modifiers"), Change::Modifiers(before));
            self.whole_height_changed();
        }
    }

    pub fn import_into_project(&mut self, label: String, f: impl FnOnce(&mut World, &AssetIndex, &Job) -> anyhow::Result<Vec<String>> + Send + 'static) {
        let Some(doc) = &self.doc else { return };
        let mut lite = doc.world.lite();
        let assets = self.assets.clone();
        let label2 = label.clone();
        self.spawn(label, move |job| {
            let notes = f(&mut lite, &assets, job)?;
            Ok(Box::new(move |app: &mut App| {
                let models: Vec<String> = lite.objects.models.iter().map(|m| m.path.clone()).collect();
                let objects: Vec<Obj> = lite.objects.iter().map(|(_, o)| *o).collect();
                let count = objects.len();
                app.merge_generated(&label2, models, objects);
                if let Some(d) = &mut app.doc {
                    d.world.lines.extend(lite.lines);
                }
                for n in notes {
                    app.warn(n);
                }
                app.info(tf("Imported {0} objects", &[&count.to_string()]));
            }) as Finish)
        });
    }

    // ---- menu actions

    pub fn import_folder(&mut self) {
        let Some(dir) = rfd::FileDialog::new().set_title(t("Choose a Terrain Builder or built map folder")).pick_folder() else { return };
        self.scan_for_import(dir);
    }

    /// Looks through a folder and lets the user review what was found.
    pub fn scan_for_import(&mut self, dir: PathBuf) {
        self.spawn(t("Looking through the folder"), move |_| {
            let scan = dz_core::import::scan_folder(&dir);
            let plan = dz_core::import::ImportPlan::from_scan(&dir, &scan);
            Ok(Box::new(move |app: &mut App| app.dialog = Dialog::Import(Box::new(ImportDialog { plan, scan }))) as Finish)
        });
    }

    pub fn run_import(&mut self, plan: dz_core::import::ImportPlan) {
        let (assets, db) = (self.assets.clone(), self.db.clone());
        self.spawn(t("Importing map"), move |job| {
            let w = dz_core::import::import_plan(&plan, &assets, &db, job)?;
            Ok(Box::new(move |app: &mut App| app.open_world(w, None)) as Finish)
        });
    }

    pub fn finish_shape(&mut self) {
        let kind = match self.shape.tool {
            ShapeTool::Polygon => ShapeKind::Polygon,
            ShapeTool::Polyline => ShapeKind::Polyline,
            ShapeTool::Point => ShapeKind::Point,
            ShapeTool::Edit => return,
        };
        if self.shape.points.len() < kind.min_points() {
            return;
        }
        let points = std::mem::take(&mut self.shape.points);
        let Some(doc) = &mut self.doc else { return };
        let previous = doc.world.shape_layers.clone();
        let w = &mut doc.world;
        if w.shape_layers.is_empty() {
            w.shape_layers.push(ShapeLayer { name: t("Shapes"), visible: true, shapes: Vec::new() });
        }
        w.active_shape_layer = w.active_shape_layer.min(w.shape_layers.len() - 1);
        let layer = &mut w.shape_layers[w.active_shape_layer];
        layer.visible = true;
        layer.shapes.push(MapShape { kind, points, attributes: Vec::new() });
        self.shape.selection = BTreeSet::from([(w.active_shape_layer, layer.shapes.len() - 1)]);
        doc.history.push(t("Add shape"), Change::Shapes(previous));
    }

    /// Removes the selected vertex, or the selected shapes.
    pub fn delete_shape_part(&mut self) {
        let vertex = self.shape.vertex.take();
        let Some(doc) = &mut self.doc else { return };
        let previous = doc.world.shape_layers.clone();
        if let (1, Some(v)) = (self.shape.selection.len(), vertex) {
            let (l, s) = *self.shape.selection.iter().next().unwrap();
            if let Some(shape) = doc.world.shape_layers.get_mut(l).and_then(|x| x.shapes.get_mut(s)) {
                if v < shape.points.len() && shape.points.len() > shape.kind.min_points() {
                    shape.points.remove(v);
                    doc.history.push(t("Edit shape"), Change::Shapes(previous));
                    return;
                }
            }
        }
        let mut removed = false;
        for (l, s) in std::mem::take(&mut self.shape.selection).into_iter().rev() {
            if let Some(list) = doc.world.shape_layers.get_mut(l).map(|x| &mut x.shapes) {
                if s < list.len() {
                    list.remove(s);
                    removed = true;
                }
            }
        }
        if removed {
            doc.history.push(t("Delete shape"), Change::Shapes(previous));
        }
    }

    pub fn shape_action(&mut self, action: ShapeAction) {
        let Some(doc) = &mut self.doc else { return };
        let previous = doc.world.shape_layers.clone();
        let layers = &mut doc.world.shape_layers;
        match action {
            ShapeAction::New => {
                layers.push(ShapeLayer { name: t("New layer"), visible: true, shapes: Vec::new() });
                let n = layers.len() - 1;
                doc.world.active_shape_layer = n;
                self.shape_rename = Some((n, doc.world.shape_layers[n].name.clone()));
                doc.history.push(t("New layer"), Change::Shapes(previous));
            }
            ShapeAction::Active(i) => {
                doc.world.active_shape_layer = i;
                doc.history.revision += 1;
            }
            ShapeAction::Visible(i) => {
                layers[i].visible ^= true;
                doc.history.revision += 1;
            }
            ShapeAction::Rename(i, name) => {
                let name = name.trim();
                if !name.is_empty() && name != layers[i].name {
                    layers[i].name = name.to_string();
                    doc.history.push(t("Rename"), Change::Shapes(previous));
                }
            }
            ShapeAction::Delete(i) => {
                layers.remove(i);
                doc.world.active_shape_layer = doc.world.active_shape_layer.min(layers_len(&doc.world).saturating_sub(1));
                self.shape.selection.clear();
                doc.history.push(t("Delete layer"), Change::Shapes(previous));
            }
        }
    }

    pub fn import_shapes(&mut self) {
        let Some(files) = rfd::FileDialog::new().add_filter("Shapes", &["shp", "tv4s"]).pick_files() else { return };
        self.load_shape_files(files);
    }

    pub fn load_shape_files(&mut self, files: Vec<PathBuf>) {
        self.spawn(t("Import shapes"), move |_| {
            let mut layers = Vec::new();
            for f in &files {
                layers.push(dz_core::shapes::load(f)?);
            }
            Ok(Box::new(move |app: &mut App| {
                let Some(doc) = &mut app.doc else { return };
                let previous = doc.world.shape_layers.clone();
                let (n, warning) = dz_core::shapes::add_layers(&mut doc.world, layers);
                doc.history.push(t("Import shapes"), Change::Shapes(previous));
                if let Some(w) = warning { app.warn(w); }
                app.info(tf("Imported {0} shapes", &[&n.to_string()]));
            }) as Finish)
        });
    }

    pub fn export_shapes(&mut self) {
        let Some(dir) = rfd::FileDialog::new().set_title(t("Export shapes (SHP)...")).pick_folder() else { return };
        self.write_shapes(&dir);
    }

    /// Writes every visible layer; hidden layers are left out like hidden objects.
    pub fn write_shapes(&mut self, dir: &Path) {
        let Some(doc) = &self.doc else { return };
        let (ex, ey) = (doc.world.meta.easting, doc.world.meta.northing);
        let mut files = 0;
        for l in doc.world.shape_layers.iter().filter(|l| l.visible && !l.shapes.is_empty()) {
            match dz_core::shapes::write_layer(dir, l, ex, ey) {
                Ok(list) => files += list.len(),
                Err(e) => {
                    self.error(format!("{e:#}"));
                    return;
                }
            }
        }
        self.info(tf("Exported {0} shapefiles", &[&files.to_string()]));
    }

    /// Terrain Builder layers into the open project, each as a new object layer.
    pub fn import_tv4l(&mut self) {
        let Some(files) = rfd::FileDialog::new().add_filter("Terrain Builder", &["tv4l"]).pick_files() else { return };
        let Some(doc) = &self.doc else { return };
        let mut lite = doc.world.lite();
        self.spawn(t("Import Terrain Builder layers"), move |job| {
            let layers: Vec<dz_core::import::LayerFile> = files.iter().filter_map(|p| dz_core::tv4l::read_info(p).ok().map(|info| dz_core::import::LayerFile { path: p.clone(), info, import: true })).collect();
            let libraries = files.first().map(|f| dz_core::import::libraries_near(f)).unwrap_or_default();
            anyhow::ensure!(!libraries.is_empty(), "No TemplateLibs folder next to the layers");
            let before = lite.object_layers.len();
            dz_core::import::add_tb_layers(&mut lite, &layers, &libraries, job)?;
            Ok(Box::new(move |app: &mut App| app.merge_layers(lite, before)) as Finish)
        });
    }

    fn merge_layers(&mut self, lite: World, first: usize) {
        let Some(doc) = &mut self.doc else { return };
        let previous = doc.world.object_layers.clone();
        let mut map: HashMap<u16, u16> = HashMap::new();
        for (i, l) in lite.object_layers.iter().enumerate().skip(first) {
            let n = doc.world.add_object_layer(&l.name);
            let target = &mut doc.world.object_layers[n as usize];
            (target.visible, target.editable, target.export) = (l.visible, l.editable, l.export);
            map.insert(i as u16, n);
        }
        let models: Vec<u32> = lite.objects.models.iter().map(|m| doc.world.objects.intern(&m.path)).collect();
        let mut change = Vec::with_capacity(lite.objects.len());
        for (_, o) in lite.objects.iter() {
            let mut o = *o;
            o.model = models[o.model as usize];
            o.set_layer(map.get(&o.layer()).copied().unwrap_or(0));
            let id = doc.world.objects.insert(o);
            change.push((id, None, Some(o)));
        }
        let count = change.len();
        doc.history.push(t("Import Terrain Builder layers"), Change::ObjectLayers(previous));
        doc.history.push(t("Import Terrain Builder layers"), Change::Objects(change));
        doc.history.group_last(2, t("Import Terrain Builder layers"));
        doc.dirty.objects = true;
        self.info(tf("Imported {0} objects", &[&count.to_string()]));
    }

    /// Call after layers change: views refresh and unreachable objects leave the selection.
    fn layers_changed(&mut self) {
        let Some(doc) = &mut self.doc else { return };
        doc.world.objects.revision += 1;
        doc.history.revision += 1;
        let w = &doc.world;
        let before = self.selection.len();
        self.selection.retain(|id| w.objects.get(*id).is_some_and(|o| w.object_interactive(o)));
        if self.selection.len() != before {
            self.selection_rev += 1;
        }
    }

    pub fn layer_action(&mut self, action: LayerAction) {
        let Some(doc) = &mut self.doc else { return };
        let layers = &mut doc.world.object_layers;
        match action {
            LayerAction::New => {
                let previous = layers.clone();
                let n = doc.world.add_object_layer(&t("New layer"));
                doc.world.active_layer = n;
                doc.history.push(t("New layer"), Change::ObjectLayers(previous));
                self.layer_rename = Some((n, doc.world.object_layers[n as usize].name.clone()));
            }
            LayerAction::Active(l) => {
                doc.world.active_layer = l;
                doc.history.revision += 1;
                return;
            }
            LayerAction::Visible(l) => layers[l as usize].visible ^= true,
            LayerAction::Editable(l) => layers[l as usize].editable ^= true,
            LayerAction::Export(l) => layers[l as usize].export ^= true,
            LayerAction::Solo(l) => {
                for (i, x) in layers.iter_mut().enumerate() {
                    x.editable = i == l as usize;
                }
                layers[l as usize].visible = true;
                doc.world.active_layer = l;
            }
            LayerAction::EditAll => layers.iter_mut().for_each(|x| x.editable = true),
            LayerAction::ShowAll => layers.iter_mut().for_each(|x| x.visible = true),
            LayerAction::Select(l) => {
                let w = &doc.world;
                if !ctx_shift(&self.ctx) {
                    self.selection.clear();
                }
                self.selection.extend(w.objects.iter().filter(|(_, o)| o.layer() == l && w.object_interactive(o)).map(|(id, _)| id));
                self.selection_rev += 1;
                return;
            }
            LayerAction::MoveSelection(l) => {
                let mut change = Vec::with_capacity(self.selection.len());
                for id in &self.selection {
                    let Some(before) = doc.world.objects.get(*id).copied() else { continue };
                    let mut after = before;
                    after.set_layer(l);
                    doc.world.objects.update(*id, after);
                    change.push((*id, Some(before), Some(after)));
                }
                if !change.is_empty() {
                    doc.history.push(t("Move to layer"), Change::Objects(change));
                    doc.dirty.objects = true;
                }
            }
            LayerAction::Rename(l, name) => {
                let name = name.trim();
                if !name.is_empty() && name != layers[l as usize].name {
                    let previous = layers.clone();
                    layers[l as usize].name = name.to_string();
                    doc.history.push(t("Rename"), Change::ObjectLayers(previous));
                }
            }
            LayerAction::Delete(l) => {
                if l == 0 { return; }
                let previous = layers.clone();
                let list: Vec<(u32, Obj)> = doc.world.objects.iter().filter(|(_, o)| o.layer() == l).map(|(id, o)| (id, *o)).collect();
                let mut change = Vec::with_capacity(list.len());
                for (id, before) in list {
                    let mut after = before;
                    after.set_layer(0);
                    doc.world.objects.update(id, after);
                    change.push((id, Some(before), Some(after)));
                }
                doc.world.object_layers[l as usize].deleted = true;
                doc.world.active_layer = doc.world.target_layer();
                doc.history.push(t("Delete layer"), Change::Objects(change));
                doc.history.push(t("Delete layer"), Change::ObjectLayers(previous));
                doc.history.group_last(2, t("Delete layer"));
                doc.dirty.objects = true;
            }
            LayerAction::DeleteWithObjects(l) => {
                if l == 0 { return; }
                let previous = layers.clone();
                let ids: BTreeSet<u32> = doc.world.objects.iter().filter(|(_, o)| o.layer() == l).map(|(id, _)| id).collect();
                let any = !ids.is_empty();
                doc.world.object_layers[l as usize].deleted = true;
                doc.world.active_layer = doc.world.target_layer();
                doc.history.push(t("Delete layer"), Change::ObjectLayers(previous));
                if any {
                    self.selection = ids;
                    self.delete_selection();
                    if let Some(d) = &mut self.doc {
                        d.history.group_last(2, t("Delete layer"));
                    }
                }
            }
        }
        self.layers_changed();
    }

    pub fn import_wrp_file(&mut self, f: PathBuf) {
        let db = self.db.clone();
        self.spawn(t("Importing WRP"), move |job| {
            let w = dz_core::import::import_wrp(&f, None, &db, job)?;
            Ok(Box::new(move |app: &mut App| app.open_world(w, None)) as Finish)
        });
    }

    pub fn import_wrp(&mut self) {
        let Some(f) = rfd::FileDialog::new().add_filter("WRP", &["wrp"]).pick_file() else { return };
        let db = self.db.clone();
        self.spawn(t("Importing WRP"), move |job| {
            let w = dz_core::import::import_wrp(&f, None, &db, job)?;
            Ok(Box::new(move |app: &mut App| app.open_world(w, None)) as Finish)
        });
    }

    pub fn import_heightmap(&mut self) {
        if self.doc.is_none() {
            return;
        }
        let Some(f) = rfd::FileDialog::new().add_filter("ASC", &["asc"]).pick_file() else { return };
        self.spawn(t("Importing heightmap"), move |_| {
            let g = dz_core::import::read_asc(&f)?;
            Ok(Box::new(move |app: &mut App| {
                let Some(d) = &app.doc else { return };
                if g.map.cols != d.world.base.cols || g.map.rows != d.world.base.rows {
                    app.error(tf("The heightmap has {0} columns, the project {1}", &[&g.map.cols.to_string(), &d.world.base.cols.to_string()]));
                    return;
                }
                app.apply_heights(&t("Import heightmap"), g.map.data, Vec::new());
            }) as Finish)
        });
    }

    pub fn import_satellite(&mut self) {
        if self.doc.is_none() {
            return;
        }
        let Some(f) = rfd::FileDialog::new().add_filter("Image", &["png", "bmp", "jpg", "tga", "tif"]).pick_file() else { return };
        self.spawn(t("Importing satellite image"), move |_| {
            let img = dz_core::terrain::RgbImage::load(&f)?;
            Ok(Box::new(move |app: &mut App| {
                let Some(d) = &mut app.doc else { return };
                app.renderer.lock().unwrap().set_satellite(Some(&img));
                d.world.satellite = Some(img);
                d.dirty.satellite = true;
                d.history.revision += 1;
                app.detail_center = [f32::MIN, f32::MIN];
            }) as Finish)
        });
    }

    pub fn import_mask(&mut self) {
        let Some(doc) = &self.doc else { return };
        let Some(f) = rfd::FileDialog::new().add_filter("Image", &["png", "bmp", "tga", "tif"]).pick_file() else { return };
        let mut layers = doc.world.layers.clone();
        self.spawn(t("Importing mask"), move |job| {
            let img = dz_core::terrain::RgbImage::load(&f)?;
            let mask = dz_core::import::mask_from_image(&img, &mut layers, job)?;
            Ok(Box::new(move |app: &mut App| {
                let Some(d) = &mut app.doc else { return };
                d.world.layers = layers;
                d.world.mask = mask;
                d.dirty.mask = true;
                d.history.revision += 1;
                app.whole_mask_changed();
                app.push_palette();
                app.reload_layer_textures();
            }) as Finish)
        });
    }

    pub fn import_layers_cfg(&mut self) {
        if self.doc.is_none() {
            return;
        }
        let Some(f) = rfd::FileDialog::new().add_filter("layers.cfg", &["cfg"]).pick_file() else { return };
        match std::fs::read(&f).map_err(anyhow::Error::from).and_then(|b| dz_core::layers_cfg::parse(&String::from_utf8_lossy(&b))) {
            Ok(layers) => {
                if let Some(d) = &mut self.doc {
                    d.world.layers = layers;
                    d.history.revision += 1;
                }
                self.push_palette();
                self.reload_layer_textures();
            }
            Err(e) => self.error(format!("{e:#}")),
        }
    }

    pub fn import_objects_txt(&mut self) {
        let Some(f) = rfd::FileDialog::new().add_filter("Terrain Builder", &["txt"]).pick_file() else { return };
        self.import_into_project(t("Import objects"), move |w, a, _| {
            let text = String::from_utf8_lossy(&std::fs::read(&f)?).into_owned();
            let (list, missing) = dz_core::import::parse_tb_objects(&text, w, a, false);
            for o in list {
                w.objects.insert(o);
            }
            Ok(missing.into_iter().map(|m| format!("? {m}")).collect())
        });
    }

    pub fn import_forest(&mut self) {
        let Some(f) = rfd::FileDialog::new().add_filter("DayZForests", &["json"]).pick_file() else { return };
        self.import_into_project(t("Import DayZForests project"), move |w, a, _| {
            let (_, missing) = dz_core::import::import_forest_project(&f, w, a)?;
            Ok(missing.into_iter().map(|m| format!("? {m}")).collect())
        });
    }

    pub fn import_road_project(&mut self) {
        let Some(f) = rfd::FileDialog::new().add_filter("Road Studio", &["dzroad", "json"]).pick_file() else { return };
        self.import_into_project(t("Import Road Studio project"), move |w, _, _| {
            dz_core::import::import_road_document(&f, w)?;
            Ok(Vec::new())
        });
    }

    pub fn import_tv4p(&mut self) {
        let Some(f) = rfd::FileDialog::new().add_filter("TV4P", &["tv4p"]).pick_file() else { return };
        self.import_into_project(t("Import TV4P roads"), move |w, _, _| {
            dz_core::import::import_tv4p(&f, w, Some(0))?;
            Ok(Vec::new())
        });
    }

    // ---- export and build

    fn export_options(&self, source: bool) -> dz_core::export::ExportOptions {
        dz_core::export::ExportOptions { root: self.settings.tools.work_drive.clone(), layers: true, world: true, config: true, source }
    }

    fn report(&mut self, r: dz_core::export::ExportReport) {
        self.info(tf("Exported to {0}: {1} tiles, {2} materials, {3} objects", &[&r.folder.display().to_string(), &r.tiles.to_string(), &r.rvmats.to_string(), &r.objects.to_string()]));
        if !r.missing_models.is_empty() {
            self.warn(tf("Models not found on the work drive ({0}), placed without height correction", &[&r.missing_models.len().to_string()]));
            for m in r.missing_models.iter().take(30) {
                self.warn(format!("  {m}"));
            }
        }
        for w in r.warnings {
            self.warn(w);
        }
    }

    pub fn export_map(&mut self, source: bool) {
        let Some(doc) = &self.doc else { return };
        let (w, db, opts) = (doc.world.clone(), self.db.clone(), self.export_options(source));
        self.bottom = BottomTab::Build;
        self.spawn(t("Exporting map"), move |job| {
            let r = dz_core::export::run(&w, &db, &opts, job)?;
            Ok(Box::new(move |app: &mut App| app.report(r)) as Finish)
        });
    }

    /// Export, binarise and optionally start the game.
    pub fn build(&mut self, run: bool) {
        let Some(doc) = &self.doc else { return };
        let problems = self.settings.tools.check();
        if !problems.is_empty() {
            for p in problems {
                self.error(p);
            }
            return;
        }
        let (w, db, opts, paths) = (doc.world.clone(), self.db.clone(), self.export_options(false), self.settings.tools.clone());
        let (client, mods) = (self.settings.with_client, self.settings.extra_mods.clone());
        self.bottom = BottomTab::Build;
        self.spawn(if run { t("Building and starting map") } else { t("Building map") }, move |job| {
            let report = dz_core::export::run(&w, &db, &opts, job)?;
            let id = w.meta.map_id.clone();
            dz_core::build::build_pbo(&paths, &id, job)?;
            let session = if run {
                dz_core::build::prepare_mission(&paths, &id, w.meta.spawn, job)?;
                Some(dz_core::build::launch(&paths, &id, &data_dir().join("session").join(&id), client, &mods, job)?)
            } else {
                None
            };
            Ok(Box::new(move |app: &mut App| {
                app.report(report);
                app.session = session;
                app.info(t("Build finished"));
            }) as Finish)
        });
    }

    pub fn stop_session(&mut self) {
        if let Some(mut s) = self.session.take() {
            let _ = s.server.kill();
            if let Some(c) = &mut s.client {
                let _ = c.kill();
            }
            self.info(t("Test session stopped"));
        }
    }

    pub fn generate_navmesh(&mut self) {
        let Some(doc) = &self.doc else { return };
        let paths = self.settings.tools.clone();
        let id = doc.world.meta.map_id.clone();
        let spawn = doc.world.meta.spawn;
        let session = data_dir().join("session").join(&id);
        self.bottom = BottomTab::Build;
        self.spawn(t("Starting navmesh tools"), move |job| {
            dz_core::build::prepare_mission(&paths, &id, spawn, job)?;
            let server = dz_core::build::launch_navmesh(&paths, &id, &session, job)?;
            Ok(Box::new(move |app: &mut App| {
                app.session = Some(dz_core::build::Session { server, client: None });
                app.info(t("Navmesh server started. Generate the mesh in NavMeshGenerator, save it as navmesh.nm in the map's navmesh folder, then build again."));
            }) as Finish)
        });
    }

    /// Saves the next rendered frame as a PNG; used by automated checks.
    pub fn pending_shot(&mut self, path: String, ctx: &egui::Context) {
        self.shot_path = Some(path);
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
    }

    pub fn handle_screenshots(&mut self, ctx: &egui::Context) {
        let images: Vec<_> = ctx.input(|i| i.events.iter().filter_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None }).collect());
        for image in images {
            if let Some(p) = self.shot_path.take() {
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
                let _ = image::save_buffer(&p, &bytes, image.width() as u32, image.height() as u32, image::ColorType::Rgba8);
            }
        }
    }

    pub fn open_folder(&self, p: &Path) {
        let _ = std::process::Command::new("explorer").arg(p).spawn();
    }

    pub fn sync_language(&mut self, code: &str) {
        crate::i18n::set_language(code);
        self.settings.language = code.to_string();
        self.settings.save();
    }
}
