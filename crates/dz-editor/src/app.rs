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
use dz_core::world::{World, WorldMeta};
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
    KeyPoints,
    Satellite,
    /// The server mission: economy, spawns, territories and the rest.
    Mission,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShapeTool {
    Edit,
    Polygon,
    Polyline,
    Point,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SelectedWhole {
    Route(u64),
    Generator(u64),
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
    pub keypoint: Option<usize>,
    /// Key points before the drag or edit in progress, for undo.
    pub keypoints_before: Option<Vec<dz_core::keypoints::KeyPoint>>,
    pub keypoint_filter: String,
    pub keypoint_moving: bool,
    /// Clicks on the map add key points.
    pub keypoint_adding: bool,
    /// Water courses from the terrain: how wet a place must be to start one, and the shortest
    /// kept, in metres.
    pub course_wetness: f32,
    pub course_length: f32,
}

impl Default for ShapePanel {
    fn default() -> Self {
        Self { keypoint: None, keypoints_before: None, keypoint_filter: String::new(), keypoint_moving: false, keypoint_adding: false, tool: ShapeTool::Edit, points: Vec::new(), selection: BTreeSet::new(), vertex: None, boxing: None, move_from: None, drag_before: None, course_wetness: 0.62, course_length: 300.0 }
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
    /// Arrows at the ends of the selection lay copies of it in a row.
    Copy,
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
    /// One of the user's brush pictures in place of the shape.
    pub picture: Option<String>,
    pub spray: bool,
    pub spray_density: f32,
    /// The height copied from the map for the set height brush.
    pub height: Option<f32>,
}

pub struct PaintSettings {
    pub layer: usize,
    pub radius: f32,
    pub scatter: f32,
    pub shape: dz_core::edit::BrushShape,
    /// One of the user's brush pictures in place of the shape.
    pub picture: Option<String>,
    pub spray: bool,
    pub spray_density: f32,
    /// Paint only over this surface.
    pub only_on: Option<usize>,
    /// The next click takes the surface under the cursor.
    pub picking: bool,
    /// Strokes go into a mask modifier, as those of the satellite brushes do, rather than
    /// straight onto the mask.
    pub into_modifier: bool,
    /// The modifier the strokes go into; a new one when it is gone.
    pub target: Option<u32>,
}

/// A stroke of the surface brush that goes into a mask modifier.
pub struct MaskModStroke {
    pub id: u32,
    /// The modifiers before the stroke, for undo.
    pub before: Vec<dz_core::maskmod::MaskModifier>,
    /// Takes the modifier's pixels away instead of painting.
    pub erase: bool,
}

pub struct PlaceSettings {
    pub model: Option<String>,
    pub random_yaw: bool,
    pub scale_min: f32,
    pub scale_max: f32,
    pub align: bool,
    /// Paints objects with a brush instead of placing one per click.
    pub brush: bool,
    pub brush_radius: f32,
    /// Least distance between painted objects.
    pub brush_spacing: f32,
    /// Objects the brush puts in 100 m² of the zone it paints; 0 for as many as fit.
    pub brush_density: f32,
    /// The brush leaves out spots that other objects, such as buildings, stand on.
    pub brush_avoid: bool,
    /// Places models of this object preset instead of `model`.
    pub preset: Option<u64>,
    /// Places this prefab of the library instead of `model`.
    pub prefab: Option<u64>,
    /// Models selected together in the browser; one of them is placed at random when
    /// `model` is among them.
    pub pool: Vec<String>,
    pub auto_pad: bool,
    pub pad_falloff: f32,
    /// Extra ground levelled around an object, in metres.
    pub pad_margin: f32,
    /// New pads level the ground to the height of their object and follow it up and down.
    pub pad_follow: bool,
    /// Clicks in 3D place objects on top of other objects (roofs, floors, decks) instead of only terrain.
    pub snap_surface: bool,
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
    pub grading: dz_core::world::RoadGrading,
    /// Clicks add points to this end of the selected route: true for its start.
    pub extend: Option<bool>,
    /// A route lengthened at one end has only the new stretch fitted.
    pub extend_only: bool,
    pub selected_route: Option<usize>,
    pub selected_point: Option<usize>,
    /// Waypoints before the drag in progress.
    pub point_drag: Option<Vec<[f32; 2]>>,
    /// Waypoints before values were typed into the point list; they wait for Fit again.
    pub typed: Option<(usize, Vec<[f32; 2]>)>,
    /// Shape layer whose polygons the route must avoid.
    pub forbidden: Option<usize>,
    /// How clicks lay a road.
    pub draw: crate::road_draw::RoadDraw,
    /// The crossroad put into the selected route by a click on it.
    pub crossroad_model: String,
    /// The next click on the selected route puts the crossroad there.
    pub crossroad_armed: bool,
    /// Diameter of the centre line of roundabouts, in metres.
    pub roundabout_diameter: f32,
    /// Clicks place roundabouts instead of drawing roads.
    pub roundabout_armed: bool,
    /// Drawing live, the pieces laid up to the clicked points stay as they are while the road
    /// goes on; otherwise the whole road is fitted again with every new point.
    pub live_keep: bool,
    /// The ring last worked out, round the origin: (road type, diameter, ring or why none).
    pub roundabout: Option<(String, f32, Result<dayz_road_tool::roundabout::Ring, String>)>,
}

pub struct NewProject {
    pub name: String,
    pub preset: usize,
    pub base: f32,
    pub folder: String,
    /// Made from a real place instead of flat ground.
    pub real: bool,
    pub place: Box<crate::real_terrain_ui::RealPlace>,
}

/// (terrain vertices, cell size in metres)
pub const SIZE_PRESETS: [(u32, f32); 11] = [(1024, 5.0), (1024, 10.0), (2048, 2.5), (2048, 5.0), (2048, 6.25), (2048, 7.5), (4096, 2.5), (4096, 3.75), (8192, 1.875), (4096, 5.0), (8192, 2.5)];

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
    /// Its objects are baked into the satellite image, or left out.
    Satellite(u16),
    Solo(u16),
    EditAll,
    ShowAll,
    Select(u16),
    MoveSelection(u16),
    Rename(u16, String),
    /// Team work: who works on the layer; empty for anyone.
    Owner(u16, String),
    /// Team work: edit a layer of someone else anyway, or leave it again.
    Unlock(u16),
    Delete(u16),
    DeleteWithObjects(u16),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OutlinerRenameTarget {
    TerrainMod(u64),
    MaskMod(u32),
    SatMod(u32),
    Generator(u64),
    Route(usize),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConvertKind {
    Mask,
    Satellite,
}

pub enum Dialog {
    None,
    Import(Box<ImportDialog>),
    RoadCatalog,
    NewProject(NewProject),
    Settings,
    ProjectSettings,
    MapConfig,
    About,
    Shortcuts,
    Export(Option<crate::exporting::Part>),
    Delete(crate::deleting::Plan),
    Confirm { text: String, action: fn(&mut App) },
    /// Removing a surface layer: which, and the layer its ground becomes.
    DeleteLayer { layer: usize, into: usize },
    /// Confirming conversion of a generated/baked modifier into a hand-editable brush layer.
    ConvertToBrush { kind: ConvertKind, id: u32, name: String },
}

pub struct LogLine {
    pub text: String,
    pub level: u8,
}

/// The measuring tool.
#[derive(Default)]
pub struct Measure {
    pub active: bool,
    pub points: Vec<[f32; 2]>,
}

/// State of the tile surface-limit checker and fixer in Project settings.
#[derive(Clone, Debug)]
pub struct TileLimitState {
    pub issues: Vec<dz_core::mask_limit::TileIssue>,
    /// True after `max_tile_surfaces` or `tile_px` changed until the user runs Check tiles.
    pub needs_check: bool,
    /// Highlight excess-surface pixels in the viewport (2D and 3D).
    pub show_pixels: bool,
    /// Tile `(col, row)` selected for inspection or single-tile fixing.
    pub selected: Option<(u32, u32)>,
    pub options: dz_core::mask_limit::FixOptions,
    pub last_report: Option<dz_core::mask_limit::FixReport>,
}

impl Default for TileLimitState {
    fn default() -> Self {
        Self {
            issues: Vec::new(),
            needs_check: false,
            show_pixels: true,
            selected: None,
            options: dz_core::mask_limit::FixOptions::default(),
            last_report: None,
        }
    }
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
    /// The map check and its window.
    pub check: crate::map_check::MapCheck,
    /// Whether the view holds the surfaces as the game shows them.
    shown_game_surfaces: bool,
    pub goto: crate::goto::GoTo,
    /// The measuring tool: on, and the points clicked.
    pub measure: Measure,
    /// Terrain stamps: the user's own, and the one a click on the map lays next.
    pub stamps: crate::stamps_ui::StampTool,
    /// The hour and cloud of the time of day of the lighting.
    pub time_of_day: f32,
    pub overcast: f32,
    /// The terrain the water flow shading was worked out for, and when.
    flow_key: u64,
    flow_at: Option<Instant>,
    density_rev: u64,
    /// Small pictures of the surface textures, for the layer tiles of the Paint panel.
    pub layer_thumbs: Vec<Option<egui::TextureHandle>>,
    pub road_catalog: Arc<Vec<dayz_road_tool::roads::CatalogPart>>,
    pub road_families: Vec<String>,
    road_rx: Option<Receiver<Vec<dayz_road_tool::roads::CatalogPart>>>,

    pub mode: Mode,
    pub gizmo: Gizmo,
    /// The gizmo moves and turns along the axes of the object instead of the world.
    pub gizmo_local: bool,
    pub view_mode: ViewMode,
    pub shading3: Shading,
    pub shading2: Shading,
    pub wireframe: bool,
    pub show_objects: bool,
    pub show_grass: bool,
    /// Furniture and fittings buildings place through the proxies of their visual LOD.
    pub show_proxies: bool,
    pub proxies: crate::proxies::Proxies,
    pub show_shadows: bool,
    /// The surfaces of the game (CfgSurfaces and their grass).
    pub surface_catalog: Arc<dz_core::surfaces::SurfaceCatalog>,
    /// The footsteps of the game's surfaces, read in the background.
    pub step_sounds: Arc<std::sync::OnceLock<dz_core::surfaces::StepSounds>>,
    /// The map surface open in the surface editor.
    pub surface_edit: Option<usize>,
    /// The material of a layer open for editing.
    pub material_edit: Option<crate::material_ui::MaterialEdit>,
    /// Where the last click picked an object, so a click there again picks the one under it.
    pub last_click: Option<eframe::egui::Pos2>,
    /// Per layer: (index in `clutter_models`, cumulative probability, scale min, scale max).
    pub clutter_layers: Vec<Vec<(u32, f32, f32, f32)>>,
    pub clutter_models: Vec<String>,
    pub show_water: bool,
    pub show_lines: bool,
    pub show_land_grid: bool,
    pub show_tile_grid: bool,
    pub show_contours: bool,
    /// Keys held down by the test script.
    pub synthetic_modifiers: egui::Modifiers,
    /// The pointer was over a 3D or top view in the last frame.
    pub view_hovered: bool,
    /// The pointer was over a view in the frame before, for keys that act at the cursor.
    pub view_hovered_last: bool,
    /// The user's own brush pictures.
    pub brushes: Vec<crate::brushes::CustomBrush>,
    /// Metres of height between two contour lines.
    pub contour_interval: f32,
    /// The camera may go below the terrain.
    pub camera_under: bool,
    /// Clicks in the Select tool take whole roads and generators.
    pub select_whole: bool,
    /// The road or generator all of the selection belongs to.
    pub whole: Option<SelectedWhole>,
    /// The selection last checked for loose road pieces: (revision, size).
    pub road_guard: (u64, usize),
    pub hole_tool: bool,
    pub hole_radius: f32,
    /// State before the hole stroke in progress, for undo.
    pub hole_before: Option<dz_core::holes::Holes>,
    pub holes_seen: u32,
    pub bottom: BottomTab,

    pub sculpt: SculptSettings,
    pub paint: PaintSettings,
    /// The satellite tools: baking objects and the clone brush.
    pub sat: crate::satellite::SatTool,
    /// The tool that stamps a surface under objects.
    pub mask_tool: crate::maskmods::MaskTool,
    pub place: PlaceSettings,
    pub line: LineSettings,
    /// The road fitted to the mouse while drawing live.
    pub live: crate::road_draw::LiveFit,
    /// The road being laid piece by piece.
    pub road_parts: crate::road_draw::PartsDraw,
    pub stroke: Option<SculptStroke>,
    pub mask_stroke: Option<MaskStroke>,
    pub mask_mod_stroke: Option<MaskModStroke>,

    pub camera: Camera,
    pub top_center: [f32; 2],
    pub top_mpp: f32,
    pub view3: ViewCache,
    pub view2: ViewCache,
    /// What the minimap draws of the objects.
    pub view_mini: ViewCache,
    /// How road parts that nearly meet are drawn stretched to close the gap.
    pub road_joins: crate::road_joins::RoadJoins,
    pub impostor_fits: HashMap<u32, [f32; 3]>,
    /// Texture size and anisotropy the loaded models were made with.
    applied_graphics: (u32, u32),
    pub detail_center: [f32; 2],

    pub selection: BTreeSet<u32>,
    /// Objects temporarily hidden in the view during this session (not saved to the project).
    pub hidden_objects: std::collections::HashSet<u32>,
    pub selected_modifier: Option<u64>,
    pub selection_rev: u64,
    /// The selection the outliner last scrolled to.
    pub outliner_rev: u64,
    pub drag: Option<crate::viewport::ObjectDrag>,
    /// The box or lasso being drawn: where it began, where the mouse is, and what it does.
    pub box_select: Option<(egui::Pos2, egui::Pos2, crate::viewport::BoxOp)>,
    /// The outline drawn so far while selecting with the lasso, in screen points.
    pub lasso_points: Vec<egui::Pos2>,
    /// The modifier keys held this frame.
    pub modifiers_now: egui::Modifiers,
    /// The ground under the cursor in a view, last frame.
    pub cursor_ground: Option<[f32; 3]>,
    /// For automated checks: a screenshot taken once background work is running.
    pub shot_during_task: Option<String>,
    /// What was selected last, laid out as a prefab to be copied again, and the state it was
    /// taken in.
    pub last_copy: Option<(dz_core::library::Prefab, (u64, u64))>,
    /// How far the minimap is zoomed in (1 shows the whole map) and the middle it shows.
    pub minimap_zoom: f32,
    pub minimap_center: [f32; 2],
    /// The pointer was over the minimap last frame, so the wheel is the minimap's.
    pub minimap_hover: bool,
    /// Where the camera was last frame; the zoomed minimap follows it only when it moves.
    pub minimap_camera: [f32; 3],
    /// The minimap was moved by hand; it stays put until the camera moves.
    pub minimap_panned: bool,
    /// Routes and modifiers before the grading of a route began to change.
    /// A replaced model rests on the ground as the one before it did.
    pub replace_keep_ground: bool,
    pub grading_edit: Option<(Vec<dz_core::world::RoadRoute>, Vec<dz_core::world::TerrainModifier>)>,
    /// The surface layers before a change of a mask colour that is still going on (the picker
    /// dragged); one undo step once the mouse is let go.
    pub layer_colour_edit: Option<Vec<dz_core::world::SurfaceLayer>>,
    /// The view takes the whole screen, with the panels sliding in at the edges.
    pub fullscreen: bool,
    /// Which edge panels of the full screen view are out: top, left, right, bottom.
    pub edge_open: [bool; 4],
    /// The view is in a window of its own.
    pub detached: bool,

    pub task: Option<Task>,
    /// Generators to run one after another, each once the task before it is done.
    pub gen_queue: std::collections::VecDeque<u64>,
    /// Roads to build one after another (road type and points), before the generators.
    pub route_queue: std::collections::VecDeque<(String, Vec<[f32; 2]>)>,
    /// Roads of the queue built and not built so far, while it runs; one that cannot be built
    /// is counted instead of shown as an error each.
    pub route_tally: Option<(usize, usize)>,
    pub log: Vec<LogLine>,
    /// How much of the log was there when the Output Log was last open.
    pub log_seen: usize,
    pub status: String,
    pub dialog: Dialog,
    pub filter: String,
    pub browser: crate::asset_browser::Browser,
    /// The action of the shortcut window waiting for its new key.
    pub key_capture: Option<crate::keys::Action>,
    /// Puts the keyboard into the search of the Content Browser on the next frame.
    pub focus_search: bool,
    /// A wicket or gate of a fence generator waiting for a click on its fence: (generator, opening).
    pub pick_opening: Option<(u64, usize)>,
    /// The next click on an object gives the selection its direction.
    pub pick_direction: bool,
    /// Steps of the history already looked at for changes of what others made in team work.
    pub team_seen: u64,
    /// What of others this person was told about changing, so it is said once.
    pub team_warned: std::collections::HashSet<String>,
    /// Picks the random turns of the next copies laid in a row, so the preview shows them.
    pub row_seed: u64,
    pub outliner_filter: String,
    /// Opens (true) or closes every outliner section in the next frame.
    pub outliner_fold: Option<bool>,
    /// The sections were last opened by the fold button, not closed.
    pub outliner_unfolded: bool,
    pub outliner_cache: Option<OutlinerCache>,
    pub layer_rename: Option<(u16, String)>,
    pub shape_rename: Option<(usize, String)>,
    pub outliner_rename: Option<(OutlinerRenameTarget, String)>,
    /// Tile surface-limit check results and fix options.
    pub tile_limit: TileLimitState,
    /// Triangles of loaded models, for picking what is actually under the cursor.
    pub pick_meshes: HashMap<u32, Arc<crate::viewport::PickMesh>>,
    pub shape: ShapePanel,
    pub show_shapes: bool,
    pub show_keypoints: bool,
    pub grass_under_objects: bool,
    pub library: dz_core::library::Library,
    pub built_in: Vec<dz_core::library::GeneratorPreset>,
    /// Generator of the project shown in the details panel.
    pub selected_generator: Option<u64>,
    /// Library entries being edited.
    pub edit_generator: Option<u64>,
    pub edit_object: Option<u64>,
    /// The prefab of the library shown in the details.
    pub edit_prefab: Option<u64>,
    /// Name of the forest preset being looked at.
    pub edit_forest: Option<String>,
    pub preset_folder: Option<String>,
    /// The generator or preset waiting for a satellite colour from the map.
    pub sampling: Option<u64>,
    pub lib_dirty: bool,
    /// Pictures made for library entries with no model to show.
    pub lib_thumbs: HashMap<String, egui::TextureHandle>,
    /// What the placement brush has done since the button went down.
    pub place_stroke: Vec<(u32, Option<Obj>, Option<Obj>)>,
    pub place_counter: u32,
    /// The centres of the dabs of the brush stroke going on: the zone painted so far.
    pub place_zone: Vec<[f32; 2]>,
    /// The part of an object the zone painted last has not paid yet.
    pub place_carry: f32,
    /// Shape layers opened in the outliner.
    pub shape_open: BTreeSet<usize>,
    pub last_autosave: Instant,
    /// When the project was last saved, shown in the status bar.
    pub saved_at: Option<Instant>,
    pub last_backup: Instant,
    /// Edit count of the project when the last backup was made.
    pub backup_revision: u64,
    pub backup_job: Option<std::sync::mpsc::Receiver<Result<PathBuf, String>>>,
    pub session: Option<dz_core::build::Session>,
    pub close_confirmed: bool,
    pub frame_ms: f32,
    pub autoscript: Option<crate::viewport::AutoScript>,
    shot_path: Option<String>,
    /// Input injected by automated checks.
    pub synthetic: Vec<egui::Event>,
    /// Frame times collected by automated checks.
    pub perf: Option<Vec<f32>>,
    /// The mission editor.
    pub mission: crate::mission_ui::MissionState,
    /// Standalone 3D loot point editor for `mapgroupproto.xml`.
    pub loot_proto: crate::loot_proto_ui::LootProtoUi,
    /// Team work: sending and merging the changes of several people.
    pub team: crate::team_ui::TeamUi,
}

impl App {
    pub fn new(ctx: egui::Context) -> Self {
        let mut settings = Settings::load();
        crate::i18n::load_user_languages(&data_dir().join("lang"));
        // Never chosen: the language of the system, or English.
        if settings.language.is_empty() {
            settings.language = crate::i18n::detect();
        }
        crate::i18n::set_language(&settings.language);
        dayz_road_tool::drive::set(&settings.tools.work_drive);
        crate::icons::install(&ctx);
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
        let surface_catalog = Arc::new(dz_core::surfaces::SurfaceCatalog::load(&root));
        let step_sounds: Arc<std::sync::OnceLock<dz_core::surfaces::StepSounds>> = Default::default();
        {
            let (cell, root) = (step_sounds.clone(), root.clone());
            std::thread::spawn(move || { let _ = cell.set(dz_core::surfaces::StepSounds::load(&root)); });
        }
        let mut app = Self {
            ctx, db, loader, thumbs, updater, surfaces,
            assets: Arc::new(AssetIndex::default()), assets_rx: None,
            doc: None, renderer: Default::default(), layer_rx: None, layer_averages: Vec::new(), check: Default::default(), shown_game_surfaces: false, goto: Default::default(), measure: Default::default(), stamps: Default::default(), time_of_day: 10.0, overcast: 0.2, flow_key: 0, flow_at: None, density_rev: u64::MAX, layer_thumbs: Vec::new(),
            road_catalog: Arc::new(Vec::new()), road_families: Vec::new(), road_rx: None,
            mode: Mode::Select, gizmo: Gizmo::Move, gizmo_local: false, view_mode: ViewMode::Perspective,
            shading3: Shading::Lit, shading2: Shading::Lit, wireframe: false, show_objects: true, show_grass: true, show_proxies: false, proxies: Default::default(), show_shadows: true, surface_catalog, step_sounds, surface_edit: None, material_edit: None, last_click: None, clutter_layers: Vec::new(), clutter_models: Vec::new(), show_water: true, show_lines: true,
            show_land_grid: false, show_tile_grid: false, show_contours: false, synthetic_modifiers: Default::default(), view_hovered: false, view_hovered_last: false, brushes: crate::brushes::load_all(), contour_interval: 10.0, camera_under: false, select_whole: true, whole: None, road_guard: (u64::MAX, 0), hole_tool: false, hole_radius: 2.0, hole_before: None, holes_seen: 0, bottom: BottomTab::Content,
            sculpt: SculptSettings { tool: SculptTool::Raise, radius: 60.0, falloff: 0.6, strength: 10.0, shape: Default::default(), noise: 0.0, picture: None, spray: false, spray_density: 0.08, height: None },
            paint: PaintSettings { layer: 0, radius: 15.0, scatter: 0.3, shape: Default::default(), picture: None, spray: false, spray_density: 0.08, only_on: None, picking: false, into_modifier: true, target: None },
            place: PlaceSettings { model: None, random_yaw: true, scale_min: 1.0, scale_max: 1.0, align: false, brush: false, brush_radius: 20.0, brush_spacing: 4.0, brush_density: 2.0, brush_avoid: false, preset: None, prefab: None, pool: Vec::new(), auto_pad: false, pad_falloff: 8.0, pad_margin: 1.0, pad_follow: false, snap_surface: false },
            line: LineSettings { points: Vec::new(), action: LineAction::Road, family: "asf2".into(), grade: true, fence_model: String::new(), fence_spacing: 4.0, route: true, routing: Default::default(), rail_routing: dayz_road_tool::document::RoutingSettings::rail(), grade_width: 9.0, grade_falloff: 12.0, grading: Default::default(), extend: None, extend_only: true, selected_route: None, selected_point: None, point_drag: None, typed: None, forbidden: None, draw: Default::default(), crossroad_model: String::new(), crossroad_armed: false, roundabout_diameter: 40.0, roundabout_armed: false, roundabout: None, live_keep: true }, live: Default::default(), road_parts: Default::default(),
            stroke: None, mask_stroke: None, mask_mod_stroke: None,
            camera: Camera { pos: [0.0, 300.0, 0.0], yaw: 0.8, pitch: -0.4, fov: settings.fov.clamp(20.0, 120.0).to_radians(), speed: camera_speed },
            top_center: [0.0, 0.0], top_mpp: 8.0, view3: Default::default(), view2: Default::default(), view_mini: Default::default(), road_joins: Default::default(),
            impostor_fits: HashMap::new(), sat: Default::default(), mask_tool: Default::default(), applied_graphics: (0, 0), detail_center: [f32::MIN, f32::MIN],
            selection: BTreeSet::new(), hidden_objects: std::collections::HashSet::new(), selected_modifier: None, selection_rev: 0, outliner_rev: 0, drag: None, box_select: None, lasso_points: Vec::new(), modifiers_now: egui::Modifiers::NONE, cursor_ground: None, shot_during_task: None, last_copy: None, minimap_zoom: 1.0, minimap_center: [f32::NAN, f32::NAN], minimap_hover: false, minimap_camera: [0.0; 3], minimap_panned: false, grading_edit: None, layer_colour_edit: None, replace_keep_ground: true, fullscreen: false, edge_open: [false; 4], detached: false,
            task: None, gen_queue: Default::default(), route_queue: Default::default(), route_tally: None, log: Vec::new(), log_seen: 0, status: String::new(), dialog: Dialog::None, filter: String::new(), browser: Default::default(), key_capture: None, focus_search: false, pick_opening: None, pick_direction: false, team_seen: 0, team_warned: Default::default(), row_seed: 1, outliner_filter: String::new(), outliner_fold: None, outliner_unfolded: true, outliner_cache: None, layer_rename: None, shape_rename: None, outliner_rename: None, tile_limit: TileLimitState::default(), pick_meshes: HashMap::new(), shape: ShapePanel::default(), show_shapes: true, show_keypoints: true, grass_under_objects: false, library: dz_core::library::Library::load(&crate::library_ui::library_path()), built_in: dz_core::library::built_in_generators(), selected_generator: None, edit_generator: None, edit_object: None, edit_prefab: None, edit_forest: None, preset_folder: None, sampling: None, lib_dirty: false, lib_thumbs: HashMap::new(), place_stroke: Vec::new(), place_counter: 0, place_zone: Vec::new(), place_carry: 0.0, shape_open: BTreeSet::new(),
            last_autosave: Instant::now(), saved_at: None, last_backup: Instant::now(), backup_revision: 0, backup_job: None, session: None, close_confirmed: false, frame_ms: 0.0, autoscript: None, shot_path: None, synthetic: Vec::new(), perf: None, mission: Default::default(), loot_proto: Default::default(), team: Default::default(),
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
        let text = crate::i18n::road(&text.into());
        self.status = text.clone();
        self.log.push(LogLine { text, level: 1 });
    }
    pub fn error(&mut self, text: impl Into<String>) {
        let text = crate::i18n::road(&text.into());
        self.status = text.clone();
        self.log.push(LogLine { text, level: 2 });
        self.bottom = BottomTab::Output;
    }

    /// After another work drive was chosen in the settings: models, thumbnails, the list of
    /// models and the road pieces are read from it from now on.
    pub fn work_drive_changed(&mut self) {
        let root = self.settings.tools.work_drive.clone();
        dayz_road_tool::drive::set(&root);
        self.db = ModelDb::new(root);
        self.loader = ModelLoader::new(self.db.clone(), self.ctx.clone());
        self.thumbs = Thumbnails::new(self.db.clone(), data_dir().join("thumbnails"), self.ctx.clone());
        self.rescan_assets();
        self.scan_roads();
    }

    /// Opens the asset browser at the model of an object.
    pub fn show_in_browser(&mut self, path: &str) {
        self.browser.reveal(path);
        self.bottom = BottomTab::Content;
    }

    /// Opens the asset browser where a field of a panel asked it to (see `slots`): at a model,
    /// an object preset or a forest preset, selected.
    pub fn reveal_asked(&mut self, ctx: &egui::Context) {
        use crate::asset_browser::Pick;
        let Some(pick) = crate::slots::take_reveal(ctx) else { return };
        match &pick {
            Pick::Model(m) => self.show_in_browser(&m.clone()),
            Pick::Preset(id) => {
                let Some(folder) = self.library.objects.iter().find(|p| p.id == *id).map(|p| p.folder.clone()) else { return };
                let home = self.library.library_home(&folder, dz_core::library::LibraryItem::Preset).to_string();
                self.browser.show_pick(crate::asset_browser::library_source_of(&home, dz_core::library::PRESETS_ROOT), pick);
                self.bottom = BottomTab::Content;
            }
            Pick::Forest(n) => {
                let own = self.library.forest_presets.iter().any(|p| p.name == *n);
                self.browser.show_pick(crate::asset_browser::forest_source(own), pick);
                self.bottom = BottomTab::Content;
            }
            _ => {}
        }
    }

    /// The model of the first selected object.
    pub fn selected_model(&self) -> Option<String> {
        let d = self.doc.as_ref()?;
        let o = self.selection.iter().next().and_then(|id| d.world.objects.get(*id))?;
        Some(d.world.objects.model_path(o).to_string())
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
        let roads = world.map(|w| w.routes.iter().filter(|r| !dayz_road_tool::rails::is_track_family(&r.family)).map(|r| r.centerline.clone()).collect()).unwrap_or_default();
        gen::RouteRequest { family, points, terrain_route, settings, forbidden, roads, start: None, laid: Vec::new() }
    }

    /// Makes the terrain modifier of a route match its line and options. Returns what to undo.
    pub(crate) fn sync_route_modifier(&mut self, i: usize) -> Option<Vec<dz_core::world::TerrainModifier>> {
        let doc = self.doc.as_mut()?;
        let route = doc.world.routes.get(i)?.clone();
        let before = doc.world.modifiers.clone();
        let mut area: Option<Rect> = None;
        let at = route.modifier.and_then(|id| doc.world.modifiers.iter().position(|m| m.id == id));
        if let Some(k) = at {
            area = doc.world.modifier_rect(&doc.world.modifiers[k]);
        }
        let id = route.modifier.unwrap_or_else(|| doc.world.new_modifier_id());
        let decks = route_decks(&doc.world, i);
        let built = route.grade.then(|| gen::road_modifier(&doc.world.ground(), id, &route.name, &route.centerline, route.grade_width, route.grade_falloff, &decks, &route.grading, &route.profile)).flatten();
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

    /// Grades again the roads that run over a terrain stamp in the square of world metres
    /// `[x0, z0, x1, z1]`, so they follow the ground it shapes.
    pub(crate) fn regrade_routes_in(&mut self, area: [f32; 4]) {
        let Some(doc) = &self.doc else { return };
        let hit: Vec<usize> = doc.world.routes.iter().enumerate().filter(|(_, r)| r.grade && r.centerline.iter().any(|p| p[0] >= area[0] && p[0] <= area[2] && p[1] >= area[1] && p[1] <= area[3])).map(|(i, _)| i).collect();
        for i in hit {
            self.sync_route_modifier(i);
        }
    }

    /// Also makes the roads that cross route `i` meet it, and keeps tracks over road banks.
    fn sync_route_and_crossings(&mut self, i: usize) -> Option<Vec<dz_core::world::TerrainModifier>> {
        use dayz_road_tool::rails::is_track_family as track;
        let mut before = self.sync_route_modifier(i);
        let doc = self.doc.as_ref()?;
        let routes = &doc.world.routes;
        let (rail, line) = (track(&routes.get(i)?.family), routes.get(i)?.centerline.clone());
        let flat: Vec<[f32; 3]> = line.iter().map(|p| [p[0], p[1], 0.0]).collect();
        // Roads give way to tracks, and later roads to earlier ones.
        let crossing: Vec<usize> = (0..routes.len())
            .filter(|j| *j != i && !track(&routes[*j].family) && (rail || *j > i) && !gen::crossing_decks(&routes[*j].centerline, &flat, &[], 0.0, &[]).is_empty())
            .collect();
        for j in crossing {
            let b = self.sync_route_modifier(j);
            before = before.or(b);
        }
        let doc = self.doc.as_mut()?;
        let is_track = |m: &dz_core::world::TerrainModifier| doc.world.routes.iter().any(|r| r.modifier == Some(m.id) && track(&r.family));
        let tracks: Vec<bool> = doc.world.modifiers.iter().map(is_track).collect();
        // Tracks come last, so the banks of a road never bury them.
        let first_track = tracks.iter().position(|t| *t).unwrap_or(tracks.len());
        if tracks[first_track..].iter().any(|t| !*t) {
            before.get_or_insert_with(|| doc.world.modifiers.clone());
            let (rails, others): (Vec<_>, Vec<_>) = std::mem::take(&mut doc.world.modifiers).into_iter().zip(tracks).partition(|(_, t)| *t);
            doc.world.modifiers = others.into_iter().chain(rails).map(|(m, _)| m).collect();
            doc.world.recompose(None);
            self.whole_height_changed();
        }
        before
    }

    /// Width, blend or on/off of a route's levelling changed.
    /// Notes the routes and modifiers as they were before the grading of a route is changed,
    /// once for a whole drag of its sliders or a typed value.
    pub fn begin_grading_edit(&mut self, routes: Vec<dz_core::world::RoadRoute>) {
        if self.grading_edit.is_none() {
            if let Some(d) = &self.doc {
                self.grading_edit = Some((routes, d.world.modifiers.clone()));
            }
        }
    }

    /// Once the sliders are let go and nothing is being typed, the grading change becomes one
    /// step of the history, which brings back both the values and the terrain.
    pub fn finish_grading_edit(&mut self, ctx: &egui::Context) {
        if self.grading_edit.is_none() || ctx.input(|i| i.pointer.any_down()) || ctx.wants_keyboard_input() {
            return;
        }
        let (routes, modifiers) = self.grading_edit.take().unwrap();
        let Some(d) = &mut self.doc else { return };
        let label = t("Terrain under the road");
        d.history.push(label.clone(), Change::Routes(routes));
        d.history.push(label.clone(), Change::Modifiers(modifiers));
        d.history.group_last(2, label);
    }

    /// Sets the mask colour of a surface layer; the change is undone as one step with the rest
    /// of the drag in the picker.
    pub fn set_layer_colour(&mut self, layer: usize, colour: [u8; 3]) {
        let Some(d) = &mut self.doc else { return };
        if d.world.layers.get(layer).is_none_or(|l| l.color == colour) { return; }
        if self.layer_colour_edit.is_none() { self.layer_colour_edit = Some(d.world.layers.clone()); }
        d.world.layers[layer].color = colour;
        d.history.revision += 1;
        self.push_palette();
    }

    /// Puts a finished change of mask colours in the undo history.
    pub fn finish_layer_colour_edit(&mut self, ctx: &egui::Context) {
        if self.layer_colour_edit.is_none() || ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        let before = self.layer_colour_edit.take().unwrap();
        let Some(d) = &mut self.doc else { return };
        if before != d.world.layers { d.history.push(t("Mask colour"), Change::SurfaceLayers(before)); }
    }

    pub fn route_grading_changed(&mut self, i: usize) {
        self.sync_route_and_crossings(i);
        if let Some(d) = &mut self.doc { d.history.revision += 1; }
    }

    /// Builds a route in the background. `target` is the route being re-fitted, and `before`
    /// the waypoints it goes back to when no fit is found.
    pub fn start_route(&mut self, req: gen::RouteRequest, target: Option<usize>, before: Option<Vec<[f32; 2]>>, grade: bool) {
        // New routes take the option from the tool; existing ones keep their own.
        let Some(doc) = &self.doc else { return };
        let mut lite = doc.world.lite();
        let catalog = self.road_catalog.clone();
        let db = self.db.clone();
        self.spawn(t("Building road"), move |job| {
            let built = gen::build_route(&mut lite, &db, &req, &catalog, job);
            let models: Vec<String> = lite.objects.models.iter().map(|m| m.path.clone()).collect();
            Ok(Box::new(move |app: &mut App| match built {
                Ok(road) => app.store_route(req, road, models, target, before, grade),
                Err(e) => {
                    if let (Some(i), Some(points), Some(d)) = (target, before, &mut app.doc) {
                        if let Some(r) = d.world.routes.get_mut(i) { r.points = points; }
                    }
                    match &mut app.route_tally {
                        Some(t) => t.1 += 1,
                        None => app.error(format!("{e:#}")),
                    }
                }
            }) as Finish)
        });
    }

    fn store_route(&mut self, req: gen::RouteRequest, road: gen::RoadResult, models: Vec<String>, target: Option<usize>, before: Option<Vec<[f32; 2]>>, grade: bool) {
        let Some(doc) = &mut self.doc else { return };
        let mut previous = doc.world.routes.clone();
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
        // A road drawn over a track that is already there gets its deck here.
        let mut no_deck = 0;
        if !dayz_road_tool::rails::is_track_family(&req.family) {
            let (decks, skipped) = gen::decks_over_tracks(&doc.world, &self.db, &road.centerline, &road.decks);
            no_deck = skipped;
            for (path, mut o) in decks {
                o.model = doc.world.objects.intern(&path);
                o.set_layer(layer);
                let id = doc.world.objects.insert(o);
                change.push((id, None, Some(o)));
                ids.push(id);
            }
        }
        let center = road.centerline;
        let index = match target.filter(|i| *i < doc.world.routes.len()) {
            Some(i) => {
                if let (Some(p), Some(points)) = (previous.get_mut(i), before) { p.points = points; }
                let r = &mut doc.world.routes[i];
                (r.points, r.centerline, r.objects, r.family, r.terrain_route, r.settings) = (req.points, center.clone(), ids, req.family, req.terrain_route, req.settings);
                r.grade |= !road.decks.is_empty();
                r.decks = road.decks;
                r.profile = road.profile;
                r.parts = road.parts;
                i
            }
            None => {
                let n = doc.world.routes.len() + 1;
                let kind = if dayz_road_tool::rails::is_track_family(&req.family) { "Track" } else { "Road" };
                // Pieces kept while drawing live stay a live road; laid by hand or a ring, they
                // are changed piece by piece.
                let method = if req.laid.is_empty() || (self.line.draw == crate::road_draw::RoadDraw::Live && !self.line.roundabout_armed) { self.line.draw } else { crate::road_draw::RoadDraw::Parts };
                doc.world.routes.push(dz_core::world::RoadRoute { name: format!("{kind} {n}"), family: req.family, points: req.points, centerline: center.clone(), objects: ids, id: dz_core::library::new_id(), grading: self.line.grading, profile: road.profile, decks: road.decks.clone(), terrain_route: req.terrain_route, settings: req.settings, grade: grade || !road.decks.is_empty(), grade_width: self.line.grade_width, grade_falloff: self.line.grade_falloff, modifier: None, parts: road.parts.clone(), fitted: Vec::new(), method });
                doc.world.routes.len() - 1
            }
        };
        // A designed line stands on its earthworks.
        if !doc.world.routes[index].profile.is_empty() { doc.world.routes[index].grade = true; }
        doc.world.routes[index].fitted = doc.world.routes[index].points.clone();
        let label = t("Build road");
        doc.history.push(label.clone(), Change::Objects(change));
        doc.history.push(label.clone(), Change::Routes(previous));
        doc.dirty.objects = true;
        self.line.selected_route = Some(index);
        let parts = match self.sync_route_and_crossings(index) {
            Some(before) => {
                self.doc.as_mut().unwrap().history.push(label.clone(), Change::Modifiers(before));
                3
            }
            None => 2,
        };
        self.doc.as_mut().unwrap().history.group_last(parts, label);
        if no_deck > 0 {
            self.warn(t("The road crosses a track on a bend or too obliquely, so no level crossing was laid there"));
        }
    }

    /// Fits the whole of a route again, bridges and all, as it was built: what "Fit again"
    /// does, whatever is kept when points are moved.
    pub fn fit_route_again(&mut self, i: usize, before: Vec<[f32; 2]>) {
        let Some(r) = self.doc.as_ref().and_then(|d| d.world.routes.get(i)).cloned() else { return };
        if self.task.is_some() {
            if let Some(d) = &mut self.doc { d.world.routes[i].points = before; }
            self.warn(t("Another task is still running"));
            return;
        }
        let req = self.route_request(r.family, r.points, r.terrain_route, r.settings);
        self.start_route(req, Some(i), Some(before), false);
    }

    /// Fits the models of a route again after its waypoints or settings changed.
    pub fn refit_route(&mut self, i: usize, before: Vec<[f32; 2]>) {
        let Some(r) = self.doc.as_ref().and_then(|d| d.world.routes.get(i)).cloned() else { return };
        if self.task.is_some() {
            if let Some(d) = &mut self.doc { d.world.routes[i].points = before; }
            self.warn(t("Another task is still running"));
            return;
        }
        // A route edited live is built as its preview showed it: all of it fitted again.
        if self.line.extend_only && r.method != crate::road_draw::RoadDraw::Live {
            if let Some((at_end, from)) = extension(&r) {
                self.extend_route(i, at_end, from, before);
                return;
            }
        }
        // With pieces kept, only the stretch from the first changed point is fitted again.
        if self.line.live_keep {
            if let Some((kept, covered)) = crate::road_draw::route_kept(&r, &before, &r.points, &self.road_catalog) {
                if let Ok(parts) = crate::road_draw::fit_kept(&kept, covered, &r.points, &r.family, &self.road_catalog, &r.settings, crate::road_draw::cap_tail(self, &r.family)) {
                    let mut req = self.route_request(r.family, r.points, false, r.settings);
                    req.laid = parts;
                    self.start_route(req, Some(i), Some(before), false);
                    return;
                }
            }
        }
        let req = self.route_request(r.family, r.points, r.terrain_route, r.settings);
        self.start_route(req, Some(i), Some(before), false);
    }

    /// Takes waypoint `p` off route `i` and fits the route again without it. A route keeps
    /// at least two points, and one laid piece by piece is changed by its pieces instead.
    pub fn delete_route_point(&mut self, i: usize, p: usize) {
        let Some(r) = self.doc.as_ref().and_then(|d| d.world.routes.get(i)) else { return };
        if r.method == crate::road_draw::RoadDraw::Parts {
            self.warn(t("A road laid piece by piece is changed by its pieces"));
            return;
        }
        if r.points.len() <= 2 || p >= r.points.len() {
            self.warn(t("A route needs at least two points"));
            return;
        }
        if self.task.is_some() {
            self.warn(t("Another task is still running"));
            return;
        }
        let before = r.points.clone();
        crate::viewport::remember_fit(self, i);
        if let Some(d) = &mut self.doc { d.world.routes[i].points.remove(p); }
        self.line.selected_point = None;
        self.live.shown = None;
        self.refit_route(i, before);
    }

    /// Fits only the stretch added at one end of route `i`, from its open end, and joins it
    /// to the models already there. `from` is how many of the new waypoints come first.
    fn extend_route(&mut self, i: usize, at_end: bool, new: Vec<[f32; 2]>, before: Vec<[f32; 2]>) {
        let Some(r) = self.doc.as_ref().and_then(|d| d.world.routes.get(i)).cloned() else { return };
        let catalog = self.road_catalog.clone();
        // The open end, with its end piece taken off: from the pieces the route keeps, or for
        // a route imported or built before it kept them, from its models.
        let mut parts = r.parts.clone();
        let found = if parts.is_empty() {
            self.open_end_from_objects(&r, at_end)
        } else {
            let cap = if at_end { parts.last() } else { parts.first() }.filter(|p| dayz_road_tool::roads::is_cap(p, &catalog)).cloned();
            if cap.is_some() {
                if at_end { parts.pop(); } else { parts.remove(0); }
            }
            let port = if at_end { dayz_road_tool::roads::endpoint(&parts, &catalog) } else { dayz_road_tool::roads::startpoint(&parts, &catalog) };
            port.ok().map(|port| (port, cap.and_then(|c| self.cap_object(&r, &c))))
        };
        let Some((port, cap)) = found else {
            self.warn(t("The open end of the route was not found among its models, so the whole route is fitted again"));
            let req = self.route_request(r.family, r.points, r.terrain_route, r.settings);
            self.start_route(req, Some(i), Some(before), false);
            return;
        };
        // The road leaves the old end along its heading and turns towards the new points on a
        // curve it can take, so a point ahead of the end gives that curve room.
        let mut points = vec![[port.p[0] as f32, port.p[1] as f32]];
        if let Some(&next) = new.first() {
            let (dx, dz) = (next[0] as f64 - port.p[0], next[1] as f64 - port.p[1]);
            let dist = dx.hypot(dz);
            let turn = (port.outward[0] * dz - port.outward[1] * dx).atan2(port.outward[0] * dx + port.outward[1] * dz).abs();
            if dist > 1.0 && turn > 0.05 {
                // The curve's tangent may take at most 45 % of each leg next to it.
                let ahead = (r.settings.min_radius * (turn * 0.55).min(1.3).tan() / 0.4 + 2.0).min(dist * 0.5);
                points.push([(port.p[0] + port.outward[0] * ahead) as f32, (port.p[1] + port.outward[1] * ahead) as f32]);
            }
        }
        points.extend(new);
        let mut req = self.route_request(r.family.clone(), points, r.terrain_route, r.settings.clone());
        req.start = Some(port);
        let Some(doc) = &self.doc else { return };
        let mut lite = doc.world.lite();
        let db = self.db.clone();
        self.spawn(t("Extending road"), move |job| {
            let built = gen::build_route(&mut lite, &db, &req, &catalog, job);
            let models: Vec<String> = lite.objects.models.iter().map(|m| m.path.clone()).collect();
            Ok(Box::new(move |app: &mut App| match built {
                Ok(road) => app.store_extension(i, at_end, cap, parts, road, models),
                Err(e) => {
                    if let Some(d) = &mut app.doc {
                        if let Some(r) = d.world.routes.get_mut(i) { r.points = before; }
                    }
                    app.error(format!("{}: {e:#}", t("The new stretch could not be fitted; Fit again with the option off fits the whole route")));
                }
            }) as Finish)
        });
    }

    /// Joins a stretch fitted from one end of route `i` to it: its end piece goes, the new
    /// models come, and everything else stays as it was.
    /// The object of an end piece the route keeps among its pieces.
    fn cap_object(&self, r: &dz_core::world::RoadRoute, cap: &dayz_road_tool::roads::PlacedPart) -> Option<u32> {
        let w = &self.doc.as_ref()?.world;
        // Where the piece stands as an object, opening or closing the road, set as the editor
        // sets it or as road tools do.
        let mut spots = vec![cap.position, gen::closing_cap(cap.position, cap.rotation).0];
        if let Some((layout, length)) = self.cap_layout(&cap.model) {
            spots.extend([false, true].map(|closing| gen::cap_pose(cap.position, cap.rotation, closing, layout, length).0));
        }
        r.objects.iter().copied().filter(|id| w.objects.get(*id).is_some_and(|o| w.objects.model_path(o).to_ascii_lowercase().ends_with(&cap.model.to_ascii_lowercase()))).min_by(|a, b| {
            let d = |id: &u32| { let o = w.objects.get(*id).unwrap(); spots.iter().map(|p| (o.x as f64 - p[0]).hypot(o.z as f64 - p[1])).fold(f64::MAX, f64::min) };
            d(a).total_cmp(&d(b))
        })
    }

    /// How the road of end piece `model` of the road catalog lies along it, and the span of its ports.
    pub fn cap_layout(&self, model: &str) -> Option<(dz_core::model_info::CapLayout, f64)> {
        gen::cap_layout(&self.db, &self.road_catalog, model, r"dz\structures\roads\parts")
    }

    /// The open end of a route that keeps no pieces, from the model at that end of its centre
    /// line: where the road goes on and which way, and the end piece to take off there.
    fn open_end_from_objects(&self, r: &dz_core::world::RoadRoute, at_end: bool) -> Option<(dayz_road_tool::geometry::Port, Option<u32>)> {
        use dayz_road_tool::geometry::Port;
        use dayz_road_tool::roads::{endpoint, startpoint, PlacedPart};
        let w = &self.doc.as_ref()?.world;
        let line: Vec<[f64; 2]> = r.centerline.iter().map(|p| [p[0] as f64, p[1] as f64]).collect();
        if line.len() < 2 { return None; }
        let end = if at_end { *line.last()? } else { line[0] };
        let to_line = |p: [f64; 2]| line.windows(2).map(|s| {
            let (ab, ap) = ([s[1][0] - s[0][0], s[1][1] - s[0][1]], [p[0] - s[0][0], p[1] - s[0][1]]);
            let t = ((ap[0] * ab[0] + ap[1] * ab[1]) / (ab[0] * ab[0] + ab[1] * ab[1]).max(1e-9)).clamp(0.0, 1.0);
            (p[0] - s[0][0] - ab[0] * t).hypot(p[1] - s[0][1] - ab[1] * t)
        }).fold(f64::MAX, f64::min);
        let mut best: Option<(f64, Port, Option<u32>)> = None;
        for id in &r.objects {
            let Some(o) = w.objects.get(*id) else { continue };
            if (o.x as f64 - end[0]).hypot(o.z as f64 - end[1]) > 40.0 { continue; }
            let path = w.objects.model_path(o);
            let file = path.rsplit('\\').next().unwrap_or(path).to_ascii_lowercase();
            let Some(item) = self.road_catalog.iter().find(|c| c.path.rsplit(['\\', '/']).next().unwrap_or(&c.path).eq_ignore_ascii_case(&file)) else { continue };
            let cap = item.category == 6;
            // The object stands at the piece's origin, or at its middle for pieces fitted to the ground.
            let c = self.db.get(path).center;
            let m = dz_core::objects::rotation(o.yaw, o.pitch, o.roll, 1.0);
            let origins = [[o.x as f64, o.z as f64], [(o.x + m[0][0] * c[0] + m[2][0] * c[2]) as f64, (o.z + m[0][2] * c[0] + m[2][2] * c[2]) as f64]];
            for origin in origins {
                // An end piece may stand as the editor sets it or as road tools do.
                let laid = if cap { self.cap_layout(&item.path) } else { None };
                for (closing, editor) in [(false, false), (true, false), (false, true), (true, true)] {
                    if (closing || editor) && !cap { continue; }
                    let (position, rotation) = match (editor, laid) {
                        (true, Some((layout, length))) => gen::cap_placement(origin, o.yaw as f64, closing, layout, length),
                        (true, None) => continue,
                        (false, _) if closing => gen::closing_cap_placement(origin, o.yaw as f64),
                        (false, _) => (origin, o.yaw as f64),
                    };
                    let part = [PlacedPart { model: item.path.clone(), reverse: false, position, rotation }];
                    let (Ok(entry), Ok(exit)) = (startpoint(&part, &self.road_catalog), endpoint(&part, &self.road_catalog)) else { continue };
                    for (open, inner) in [(exit, entry), (entry, exit)] {
                        let score = (open.p[0] - end[0]).hypot(open.p[1] - end[1]) + to_line(inner.p);
                        // Without its end piece the road goes on from the cap's inner side.
                        let found = if cap { (Port { p: inner.p, outward: [-inner.outward[0], -inner.outward[1]] }, Some(*id)) } else { (open, None) };
                        if best.as_ref().is_none_or(|b| score < b.0) { best = Some((score, found.0, found.1)); }
                    }
                }
            }
        }
        best.filter(|b| b.0 < 1.5).map(|b| (b.1, b.2))
    }

    fn store_extension(&mut self, i: usize, at_end: bool, cap: Option<u32>, kept: Vec<dayz_road_tool::roads::PlacedPart>, road: gen::RoadResult, models: Vec<String>) {
        let Some(doc) = &mut self.doc else { return };
        let Some(route) = doc.world.routes.get(i).cloned() else { return };
        let previous = doc.world.routes.clone();
        let mut change = Vec::new();
        let mut objects = route.objects.clone();
        // The end piece that closed the road where it now goes on.
        if let Some(id) = cap {
            if let Some(o) = doc.world.objects.remove(id) { change.push((id, Some(o), None)); }
            objects.retain(|x| *x != id);
        }
        let layer = route.objects.first().and_then(|id| doc.world.objects.get(*id)).map(|o| o.layer()).unwrap_or_else(|| doc.world.target_layer());
        let map: Vec<u32> = models.iter().map(|m| doc.world.objects.intern(m)).collect();
        for mut o in road.objects {
            o.model = map[o.model as usize];
            o.set_layer(layer);
            let id = doc.world.objects.insert(o);
            change.push((id, None, Some(o)));
            objects.push(id);
        }
        let r = &mut doc.world.routes[i];
        let mut line = road.centerline;
        let whole = !kept.is_empty();
        let nearest = |l: &[[f32; 2]], p: [f32; 2]| (0..l.len()).min_by(|a, b| { let d = |k: &usize| (l[*k][0] - p[0]).hypot(l[*k][1] - p[1]); d(a).total_cmp(&d(b)) }).unwrap_or(0);
        if at_end {
            if let Some(&joint) = line.first() { let k = nearest(&r.centerline, joint); r.centerline.truncate(k + 1); }
            r.centerline.extend(line.into_iter().skip(1));
            r.parts = if whole { kept.into_iter().chain(road.parts).collect() } else { Vec::new() };
            r.profile.extend(road.profile);
        } else {
            if let Some(&joint) = line.first() { let k = nearest(&r.centerline, joint); r.centerline.drain(..k); }
            line.reverse();
            line.pop();
            r.centerline = line.into_iter().chain(r.centerline.drain(..)).collect();
            r.parts = if whole { dayz_road_tool::roads::flip(&road.parts).into_iter().chain(kept).collect() } else { Vec::new() };
            let mut profile = road.profile;
            profile.reverse();
            r.profile = profile.into_iter().chain(r.profile.drain(..)).collect();
        }
        r.decks.extend(road.decks);
        r.objects = objects;
        r.fitted = r.points.clone();
        let label = t("Extend road");
        doc.history.push(label.clone(), Change::Objects(change));
        doc.history.push(label.clone(), Change::Routes(previous));
        doc.dirty.objects = true;
        let parts = match self.sync_route_and_crossings(i) {
            Some(before) => {
                self.doc.as_mut().unwrap().history.push(label.clone(), Change::Modifiers(before));
                3
            }
            None => 2,
        };
        self.doc.as_mut().unwrap().history.group_last(parts, label);
    }

    pub fn delete_route(&mut self, i: usize) {
        let Some(doc) = &mut self.doc else { return };
        if i >= doc.world.routes.len() { return; }
        let previous = doc.world.routes.clone();
        let route = doc.world.routes.remove(i);
        let change = route.objects.iter().filter_map(|id| doc.world.objects.remove(*id).map(|o| (*id, Some(o), None))).collect();
        doc.history.push(t("Delete route"), Change::Objects(change));
        doc.history.push(t("Delete route"), Change::Routes(previous));
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

    /// Road types defined by the map's Terrain Builder project, when it has one.
    pub fn load_road_types(&mut self, project: PathBuf) {
        let drive = self.settings.tools.work_drive.clone();
        self.spawn(t("Reading road types"), move |_| {
            let mut catalog = gen::road_catalog_from_project(&project, &drive.join(gen::VANILLA_ROAD_FOLDERS[0]))?;
            anyhow::ensure!(!catalog.is_empty(), "{} defines no usable road types", project.display());
            gen::add_vanilla_roads(&mut catalog, &drive);
            catalog.extend(gen::rail_catalog());
            Ok(Box::new(move |app: &mut App| {
                let mut catalog = catalog;
                if let Some(d) = &app.doc {
                    gen::apply_catalog_edits(&mut catalog, &d.world.road_removed, &d.world.road_added, &dayz_road_tool::drive::roads_folder());
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

    pub(crate) fn scan_roads(&mut self) {
        let (tx, rx) = channel();
        let drive = self.settings.tools.work_drive.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            // The roads and crossroads of every map that comes with the game.
            let mut catalog = gen::vanilla_road_catalog(&drive);
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

    /// Works out where water flows again when a view shows it and the terrain has changed,
    /// at most every second and a half, so sculpting stays smooth.
    fn keep_flow(&mut self) {
        use crate::render::Shading;
        if self.shading3 != Shading::Flow && self.shading2 != Shading::Flow {
            return;
        }
        let Some(d) = &self.doc else { return };
        let h = &d.world.height;
        let key = {
            use std::hash::{Hash, Hasher};
            let mut s = std::collections::hash_map::DefaultHasher::new();
            (h.cols, h.rows, d.world.meta.sea_level.to_bits()).hash(&mut s);
            let step = (h.data.len() / 8192).max(1);
            for v in h.data.iter().step_by(step) {
                v.to_bits().hash(&mut s);
            }
            s.finish()
        };
        if key == self.flow_key || self.flow_at.is_some_and(|t| t.elapsed() < Duration::from_millis(1500)) {
            if key != self.flow_key {
                self.ctx.request_repaint_after(Duration::from_millis(1500));
            }
            return;
        }
        let small = dz_core::analysis::downsample(h, 1024);
        let wet = dz_core::analysis::wetness(&small, d.world.meta.sea_level);
        let data = wet.iter().map(|v| (v * 255.0).round() as u8).collect();
        self.renderer.lock().unwrap().set_flow(small.cols, small.rows, data);
        self.flow_key = key;
        self.flow_at = Some(Instant::now());
    }

    /// Keeps the object density heatmap texture current while either view shows `Shading::Density`.
    fn keep_density(&mut self) {
        let needed = (self.view_mode != ViewMode::Top && self.shading3 == Shading::Density)
            || (self.view_mode != ViewMode::Perspective && self.shading2 == Shading::Density);
        if !needed {
            return;
        }
        let Some(d) = &self.doc else { return };
        let rev = d.world.objects.revision;
        if rev == self.density_rev {
            return;
        }
        let grid = dz_core::analysis::DensityGrid::build(&d.world, 100.0, false);
        let data = grid.heatmap_u8(600.0);
        self.renderer.lock().unwrap().set_density(grid.cols, grid.rows, data);
        self.density_rev = rev;
    }

    pub fn tick(&mut self) {
        self.keep_flow();
        self.keep_density();
        self.sync_game_surfaces();
        self.warn_foreign_edits();
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
            match m.strip_prefix(dz_core::WARN_MARK) {
                Some(w) => self.log.push(LogLine { text: w.to_string(), level: 1 }),
                None => self.log.push(LogLine { text: m, level: 0 }),
            }
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
                Err(e) => {
                    // A run that was stopped stops those waiting after it too.
                    if e.contains("Cancelled") {
                        self.gen_queue.clear();
                        self.route_queue.clear();
                    }
                    self.error(format!("{title}: {e}"));
                }
            }
        }
        if self.task.is_none() {
            if let Some((family, points)) = self.route_queue.pop_front() {
                // Laid along the line as it is, without searching the terrain, and with bends
                // as tight as the road's own pieces allow.
                let mut settings = if dayz_road_tool::rails::is_track_family(&family) { self.line.rail_routing.clone() } else { self.line.routing.clone() };
                settings.min_radius = settings.min_radius.min(dz_core::real_terrain::ROAD_RADIUS as f64);
                let req = self.route_request(family, points, false, settings);
                self.route_tally.get_or_insert((0, 0)).0 += 1;
                self.start_route(req, None, None, true);
            } else if let Some((tried, failed)) = self.route_tally.take() {
                if failed > 0 {
                    self.warn(tf("{0} of {1} roads could not be fitted and stay as shapes", &[&failed.to_string(), &tried.to_string()]));
                } else {
                    self.info(tf("{0} roads built", &[&tried.to_string()]));
                }
            } else if let Some(id) = self.gen_queue.pop_front() {
                crate::library_ui::run_generator(self, id);
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
                if self.layer_thumbs.len() <= l.index {
                    self.layer_thumbs.resize(l.index + 1, None);
                }
                self.layer_thumbs[l.index] = Some(self.ctx.load_texture(format!("surface_thumb_{}", l.index), layer_thumb(&l.rgba, 512, 64), egui::TextureOptions::LINEAR));
                r.set_layer(l.index, 512, l.rgba, l.normal);
            }
            drop(r);
            self.push_palette();
        }
        // Texture settings apply to what loads from now on, so a change loads the models again.
        let graphics = (self.settings.texture_size, self.settings.anisotropy);
        if self.applied_graphics != graphics {
            let first = self.applied_graphics == (0, 0);
            self.applied_graphics = graphics;
            self.loader.set_texture_limit(graphics.0);
            self.renderer.lock().unwrap().anisotropy = graphics.1 as f32;
            if !first {
                self.loader.reset();
                self.impostor_fits.clear();
                self.renderer.lock().unwrap().clear_models();
            }
        }
        {
            let mut r = self.renderer.lock().unwrap();
            r.shadow_size = self.settings.shadow_map as i32;
            r.model_memory = self.settings.model_memory as usize * (1 << 20);
        }
        self.proxies.poll();
        // Models the renderer let go of may be asked for again.
        let evicted = std::mem::take(&mut self.renderer.lock().unwrap().evicted);
        for key in evicted {
            self.loader.forget(key);
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
        // Not while team work reads or writes the project.
        if every > 0 && !self.team.busy() && self.last_autosave.elapsed() > Duration::from_secs(every as u64 * 60) {
            self.last_autosave = Instant::now();
            if self.doc.as_ref().is_some_and(|d| d.history.dirty() && d.dir.is_some()) && self.stroke.is_none() {
                self.save(None);
            }
        }
        if let Some(rx) = &self.backup_job {
            match rx.try_recv() {
                Ok(Ok(path)) => {
                    self.backup_job = None;
                    self.info(tf("Backup saved: {0}", &[&path.display().to_string()]));
                }
                Ok(Err(e)) => {
                    self.backup_job = None;
                    self.error(tf("Backup failed: {0}", &[&e]));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(_) => self.backup_job = None,
            }
        }
        if self.settings.backup_enabled && self.last_backup.elapsed() > Duration::from_secs(self.settings.backup_minutes.max(1) as u64 * 60) {
            self.last_backup = Instant::now();
            if self.doc.as_ref().is_some_and(|d| d.dir.is_some() && d.history.revision != self.backup_revision) && self.stroke.is_none() {
                self.start_backup(false);
            }
        }
    }

    /// Where the backups of the open project go.
    pub fn backup_root(&self) -> Option<PathBuf> {
        let dir = self.doc.as_ref()?.dir.clone()?;
        Some(if self.settings.backup_folder.is_empty() {
            dir.join("backups")
        } else {
            PathBuf::from(&self.settings.backup_folder).join(dir.file_name().unwrap_or_default())
        })
    }

    /// Copies the project in the background, so that work can go on meanwhile.
    pub fn start_backup(&mut self, by_hand: bool) {
        if self.backup_job.is_some() {
            return;
        }
        let (Some(root), keep) = (self.backup_root(), self.settings.backup_keep.max(1) as usize) else {
            if by_hand { self.warn(t("Save the project first")); }
            return;
        };
        let Some(doc) = &mut self.doc else { return };
        let dir = doc.dir.clone().unwrap_or_default();
        // The satellite image is large and rarely changes: when it is on disk it is not copied.
        let on_disk = dir.join("satellite.rgb");
        let satellite_file = (!doc.dirty.satellite && doc.world.satellite.is_some() && on_disk.is_file()).then_some(on_disk);
        let image = if satellite_file.is_some() { doc.world.satellite.take() } else { None };
        let mut copy = doc.world.clone();
        if let Some(s) = image {
            copy.satellite = Some(dz_core::terrain::RgbImage { width: s.width, height: s.height, data: Vec::new() });
            doc.world.satellite = Some(s);
        }
        self.backup_revision = doc.history.revision;
        let name = crate::settings::timestamp();
        let (tx, rx) = std::sync::mpsc::channel();
        self.backup_job = Some(rx);
        std::thread::spawn(move || {
            let result = project::write_backup(&copy, &root, &name, keep, satellite_file.as_deref()).map_err(|e| format!("{e:#}"));
            let _ = tx.send(result);
        });
    }

    pub fn push_palette(&mut self) {
        let Some(doc) = &self.doc else { return };
        let colors: Vec<[u8; 3]> = doc.world.layers.iter().map(|l| l.color).collect();
        let mut avg = self.layer_averages.clone();
        avg.resize(colors.len().max(avg.len()), [128, 128, 128]);
        self.renderer.lock().unwrap().set_palette(colors, avg);
    }

    /// Ground clutter of each surface layer, as the game would grow it: from the map's own
    /// surfaces, then those of the game.
    pub fn reload_clutter(&mut self) {
        let Some(doc) = &self.doc else { return };
        let cat = &self.surface_catalog;
        let own = &doc.world.meta.config.surfaces;
        // Models keep their index: the loaded meshes are found by it.
        let mut models: Vec<String> = std::mem::take(&mut self.clutter_models);
        self.clutter_layers = doc.world.layers.iter().map(|l| {
            let mut sum = 0.0;
            dz_core::surfaces::clutter_of(&l.texture, own, cat).into_iter().filter_map(|(name, p)| {
                sum += p;
                let kind = cat.clutter_kind(&name)?;
                let i = models.iter().position(|m| *m == kind.model).unwrap_or_else(|| {
                    models.push(kind.model.clone());
                    models.len() - 1
                });
                Some((i as u32, sum, kind.scale[0], kind.scale[1]))
            }).collect()
        }).collect();
        self.clutter_models = models;
        for v in [&mut self.view3, &mut self.view2, &mut self.view_mini] {
            v.invalidate();
        }
    }

    pub fn reload_layer_textures(&mut self) {
        self.reload_clutter();
        let Some(doc) = &self.doc else { return };
        let list: Vec<_> = doc.world.layers.iter().enumerate().take(64).map(|(i, l)| (i, l.texture.clone(), l.material.clone(), l.color)).collect();
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
            r.set_holes(&world.holes, world.height.cols, world.height.rows);
            r.set_satellite(world.satellite.as_ref());
        }
        // The surfaces as the game shows them go over the painted mask once the world is open.
        self.shown_game_surfaces = false;
        self.loader.reset();
        self.impostor_fits.clear();
        self.layer_averages.clear();
        self.detail_center = [f32::MIN, f32::MIN];
        self.selection.clear();
        self.hidden_objects.clear();
        self.density_rev = u64::MAX;
        self.selected_modifier = None;
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
        self.tile_limit = TileLimitState::default();
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

    /// Saves the project; tells whether it was saved.
    pub fn save(&mut self, target: Option<PathBuf>) -> bool {
        let Some(doc) = &mut self.doc else { return false };
        let same = target.is_none();
        let Some(dir) = target.or_else(|| doc.dir.clone()) else {
            self.save_as();
            return false;
        };
        let parts = if same && doc.dir.is_some() {
            SaveParts { height: doc.dirty.height, mask: doc.dirty.mask, satellite: doc.dirty.satellite, objects: doc.dirty.objects }
        } else {
            SaveParts::ALL
        };
        match project::save(&doc.world, &dir, parts).and_then(|_| project::save_mission(&mut doc.world, &dir)) {
            Ok(()) => {
                doc.dirty = Dirty::default();
                doc.dir = Some(dir.clone());
                doc.history.mark_saved();
                self.saved_at = Some(Instant::now());
                self.settings.remember(dir.clone());
                self.settings.save();
                self.info(tf("Saved {0}", &[&dir.display().to_string()]));
                true
            }
            Err(e) => {
                self.error(format!("{e:#}"));
                false
            }
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
        crate::mission_ui::flush(self);
        let Some(doc) = &mut self.doc else { return };
        if let Some(t) = doc.history.undo(&mut doc.world) {
            self.apply_touched(t);
        }
    }
    pub fn redo(&mut self) {
        crate::mission_ui::flush(self);
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
        if let Some(r) = t.satellite {
            self.satellite_changed(r);
        }
        if t.layers {
            self.push_palette();
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
        // What was painted goes under the mask modifiers, which stay on top of it.
        let r = doc.world.mask_painted(r).map_or(r, |m| m.union(r));
        self.upload_mask(Some(r));
    }

    /// Sends the mask to the view: as painted, or as the game will show it.
    pub fn upload_mask(&mut self, area: Option<Rect>) {
        let Some(doc) = &self.doc else { return };
        let m = &doc.world.mask;
        let mut r = self.renderer.lock().unwrap();
        if self.settings.game_surfaces && m.width.max(m.height) <= 16384 {
            let all = Rect { x0: 0, y0: 0, x1: m.width - 1, y1: m.height - 1 };
            let area = area.map_or(all, |a| dz_core::export::game_mask_area(&doc.world, a));
            if r.update_mask_data(m, area, dz_core::export::game_mask(&doc.world, area)) {
                return;
            }
        }
        match area {
            Some(a) => r.update_mask(m, a),
            None => r.set_mask(m),
        }
    }

    /// Uploads the mask again when the way it is shown was changed.
    pub fn sync_game_surfaces(&mut self) {
        if self.shown_game_surfaces != self.settings.game_surfaces {
            self.shown_game_surfaces = self.settings.game_surfaces;
            self.upload_mask(None);
        }
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
        let all = Rect { x0: 0, y0: 0, x1: doc.world.mask.width.saturating_sub(1), y1: doc.world.mask.height.saturating_sub(1) };
        doc.world.mask_painted(all);
        self.upload_mask(None);
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
    pub fn apply_heights(&mut self, label: &str, data: Vec<f32>) {
        let Some(doc) = &mut self.doc else { return };
        let mut old = data;
        std::mem::swap(&mut old, &mut doc.world.base.data);
        doc.world.recompose(None);
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

    /// Sets `flag` on the selected objects, or clears it, as one step to undo. Objects that take
    /// or lose a fixed height stay where they stand: their elevation turns from over the ground
    /// to over the sea and back.
    pub fn set_selection_flag(&mut self, flag: u32, on: bool, label: &str) {
        use dz_core::objects::FLAG_ABSOLUTE;
        let Some(doc) = &mut self.doc else { return };
        let ids: Vec<u32> = self.selection.iter().copied().collect();
        let mut change = Vec::with_capacity(ids.len());
        let mut moved = Vec::new();
        for id in ids {
            let Some(o) = doc.world.objects.get(id).copied() else { continue };
            if (o.flags & flag != 0) == on {
                continue;
            }
            let mut n = o;
            if on { n.flags |= flag } else { n.flags &= !flag }
            if flag == FLAG_ABSOLUTE {
                let ground = doc.world.height.sample(o.x, o.z);
                n.elevation += if on { ground } else { -ground };
            }
            doc.world.objects.update(id, n);
            change.push((id, Some(o), Some(n)));
            moved.extend(doc.world.sync_attached(id));
        }
        if change.is_empty() {
            return;
        }
        let count = change.len().to_string();
        doc.history.push(label, Change::Objects(change));
        doc.dirty.objects = true;
        for r in moved {
            self.height_changed(r);
        }
        self.selection_rev += 1;
        self.status = tf(if on { "{0}: set on {1} objects" } else { "{0}: cleared on {1} objects" }, &[label, &count]);
    }

    /// Sets the underground flag of the selected objects, or clears it when all have it.
    pub fn toggle_underground(&mut self) {
        use dz_core::objects::FLAG_UNDERGROUND;
        let Some(doc) = &mut self.doc else { return };
        let items: Vec<(u32, dz_core::objects::Obj)> = self.selection.iter().filter_map(|id| Some((*id, *doc.world.objects.get(*id)?))).collect();
        if items.is_empty() {
            return;
        }
        let on = items.iter().any(|(_, o)| o.flags & FLAG_UNDERGROUND == 0);
        let mut change = Vec::with_capacity(items.len());
        for (id, o) in items {
            let mut n = o;
            if on { n.flags |= FLAG_UNDERGROUND } else { n.flags &= !FLAG_UNDERGROUND }
            if n != o {
                doc.world.objects.update(id, n);
                change.push((id, Some(o), Some(n)));
            }
        }
        let count = change.len().to_string();
        doc.history.push(t("Underground"), Change::Objects(change));
        doc.dirty.objects = true;
        self.status = tf(if on { "Underground flag set on {0} objects" } else { "Underground flag cleared on {0} objects" }, &[&count]);
    }

    pub fn hide_selection(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        let n = self.selection.len();
        self.hidden_objects.extend(self.selection.iter().copied());
        self.selection.clear();
        self.selection_rev += 1;
        self.info(tf("Hid {0} objects", &[&n.to_string()]));
    }

    pub fn show_hidden_objects(&mut self) {
        if self.hidden_objects.is_empty() {
            return;
        }
        self.hidden_objects.clear();
        self.info(t("All hidden objects are shown again"));
    }

    /// Hides every visible object outside the selection, or restores all hidden objects if
    /// everything outside the selection is already hidden.
    pub fn isolate_selection(&mut self) {
        let Some(doc) = &self.doc else { return };
        if self.selection.is_empty() {
            self.show_hidden_objects();
            return;
        }
        let mut to_hide = Vec::new();
        for (id, o) in doc.world.objects.iter() {
            if doc.world.object_visible(o) && !self.selection.contains(&id) {
                to_hide.push(id);
            }
        }
        let already_isolated = !self.hidden_objects.is_empty() && to_hide.iter().all(|id| self.hidden_objects.contains(id));
        if already_isolated {
            self.show_hidden_objects();
        } else {
            for id in to_hide {
                self.hidden_objects.insert(id);
            }
            for id in &self.selection {
                self.hidden_objects.remove(id);
            }
            self.info(tf("Isolated {0} objects", &[&self.selection.len().to_string()]));
        }
    }

    pub fn delete_selection(&mut self) {
        match self.whole.take() {
            Some(SelectedWhole::Route(id)) => {
                crate::deleting::request(self, crate::deleting::Target::Route(id));
                self.selection.clear();
                self.selection_rev += 1;
                return;
            }
            Some(SelectedWhole::Generator(id)) => {
                crate::deleting::request(self, crate::deleting::Target::Generator(id));
                self.selection.clear();
                self.selection_rev += 1;
                return;
            }
            None => {}
        }
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

    /// Saves the selected objects as a prefab of the library, to be placed again as one.
    pub fn create_prefab(&mut self) -> Option<u64> {
        let doc = self.doc.as_ref()?;
        let objects: Vec<(String, dz_core::objects::Obj)> = self.selection.iter().filter_map(|id| doc.world.objects.get(*id)).map(|o| (doc.world.objects.model_path(o).to_string(), *o)).collect();
        if objects.is_empty() {
            self.warn(t("Select the objects of the prefab first"));
            return None;
        }
        let base = t("Prefab");
        let name = (1..).map(|n| format!("{base} {n}")).find(|n| !self.library.prefabs.iter().any(|p| p.name == *n)).unwrap();
        let id = dz_core::library::new_id();
        let mut prefab = dz_core::library::Prefab::from_objects(id, name.clone(), &objects, &doc.world.height);
        // Made from a folder of prefabs open in the browser, it goes there.
        if let crate::asset_browser::Source::LibFolder(f) = &self.browser.source {
            prefab.folder = self.library.library_home(f, dz_core::library::LibraryItem::Prefab).to_string();
        }
        let shown = if prefab.folder.is_empty() { crate::asset_browser::Source::Group(crate::asset_browser::Group::LibraryPrefabs) } else { crate::asset_browser::Source::LibFolder(prefab.folder.clone()) };
        self.library.prefabs.push(prefab);
        self.lib_dirty = true;
        self.browser.go(shown);
        self.browser.selected = vec![crate::asset_browser::Pick::Prefab(id)];
        self.bottom = BottomTab::Content;
        self.edit_object = None;
        self.edit_generator = None;
        self.edit_forest = None;
        self.edit_prefab = Some(id);
        self.leave_world_details();
        self.info(tf("{0} made of {1} objects", &[&name, &objects.len().to_string()]));
        Some(id)
    }

    /// Places a prefab with its middle at (x, z) and selects what it placed.
    pub fn place_prefab(&mut self, id: u64, x: f32, z: f32) -> Vec<u32> {
        let Some(prefab) = self.library.prefabs.iter().find(|p| p.id == id).cloned() else { return Vec::new() };
        let label = tf("Place {0}", &[&prefab.name]);
        self.place_layout(&prefab, x, z, label)
    }

    /// The selected objects laid out round their middle, as a prefab is.
    pub fn selection_layout(&self) -> Option<dz_core::library::Prefab> {
        let doc = self.doc.as_ref()?;
        let objects: Vec<(String, dz_core::objects::Obj)> = self.selection.iter().take(20_000).filter_map(|id| doc.world.objects.get(*id)).map(|o| (doc.world.objects.model_path(o).to_string(), *o)).collect();
        (!objects.is_empty()).then(|| dz_core::library::Prefab::from_objects(0, String::new(), &objects, &doc.world.height))
    }

    /// After an import: layers whose textures no surface of the game or the map stands for.
    pub fn warn_layers_without_surface(&mut self, missing: &[String]) {
        if !missing.is_empty() {
            self.warn(tf("Layers {0} have no surface of the game or the map; read the map's CfgSurfaces in Surfaces in the game", &[&missing.join(", ")]));
        }
    }

    /// Whether a mode is offered: those of experimental features only when they are turned on.
    pub fn mode_enabled(&self, m: Mode) -> bool {
        m != Mode::Mission || self.settings.experimental
    }

    /// A pick in the view or the outliner takes the details over from the asset browser.
    pub fn leave_browser(&mut self) {
        self.browser.selected.clear();
        self.edit_object = None;
        self.edit_generator = None;
        self.edit_forest = None;
        self.edit_prefab = None;
    }

    /// A pick in the asset browser takes the details over from the generator, route or modifier chosen in the view.
    pub fn leave_world_details(&mut self) {
        self.selected_generator = None;
        self.selected_modifier = None;
        self.whole = None;
    }

    /// Keeps a copy of what is selected, for V to place again once something else is.
    /// Road pieces move only with their road: they are taken out of any selection that is not
    /// their whole road, from a box, a click, the outliner or Select all alike. A selection of
    /// nothing but pieces of one road becomes that whole road.
    pub fn keep_road_parts_whole(&mut self) {
        let key = (self.selection_rev, self.selection.len());
        if self.road_guard == key {
            return;
        }
        self.road_guard = key;
        let Some(doc) = &self.doc else { return };
        if doc.world.routes.is_empty() || self.selection.is_empty() {
            return;
        }
        let owner: HashMap<u32, u64> = doc.world.routes.iter().flat_map(|r| r.objects.iter().map(move |id| (*id, r.id))).collect();
        let whole = match self.whole { Some(SelectedWhole::Route(id)) => Some(id), _ => None };
        let mut dropped = BTreeSet::new();
        self.selection.retain(|id| match owner.get(id) {
            Some(r) if Some(*r) != whole => { dropped.insert(*r); false }
            _ => true,
        });
        if dropped.is_empty() {
            return;
        }
        if self.selection.is_empty() && self.whole.is_none() && dropped.len() == 1 {
            let id = *dropped.first().unwrap();
            if let Some(r) = doc.world.routes.iter().find(|r| r.id == id) {
                self.selection.extend(r.objects.iter().copied());
                self.whole = Some(SelectedWhole::Route(id));
            }
        }
        self.selection_rev += 1;
        self.road_guard = (self.selection_rev, self.selection.len());
    }

    /// Puts other models in place of the selected objects: each keeps where it stands, how
    /// it is turned, its size, layer and flags. With several models, each object takes one
    /// of them at random, the same one every time for the same place. Kept on the ground, the
    /// new model rests on it as the old one did although their bottoms differ. Objects made
    /// by a generator are left as they are, since running it again would bring them back.
    pub fn replace_selection(&mut self, models: &[String]) {
        if models.is_empty() || self.selection.is_empty() {
            return;
        }
        let keep_ground = self.replace_keep_ground;
        let Some(doc) = &mut self.doc else { return };
        let w = &mut doc.world;
        let made: std::collections::HashSet<u32> = w.generators.iter().flat_map(|g| g.objects.iter().copied()).collect();
        let rest = |info: &dz_core::model_info::ModelInfo| if info.contact_count > 0 { info.contact_y } else { 0.0 };
        let mut change = Vec::new();
        let mut skipped = 0;
        for &id in &self.selection {
            let Some(old) = w.objects.get(id).copied() else { continue };
            if made.contains(&id) {
                skipped += 1;
                continue;
            }
            let pick = if models.len() == 1 { 0 } else { ((old.x.to_bits() ^ old.z.to_bits().rotate_left(13)).wrapping_mul(2_654_435_761) >> 7) as usize % models.len() };
            let path = &models[pick];
            let mut new = old;
            new.model = w.objects.intern(path);
            if new.model == old.model {
                continue;
            }
            if keep_ground && old.flags & dz_core::objects::FLAG_ABSOLUTE == 0 {
                let (a, b) = (self.db.get(w.objects.models[old.model as usize].path.as_str()), self.db.get(path));
                new.elevation += (rest(&a) - rest(&b)) * old.scale;
            }
            w.objects.update(id, new);
            change.push((id, Some(old), Some(new)));
        }
        let mut area: Option<Rect> = None;
        for (id, _, _) in &change {
            if let Some(r) = w.sync_attached(*id) { area = Some(area.map_or(r, |a| a.union(r))); }
        }
        let n = change.len();
        if n > 0 {
            doc.history.push(t("Replace objects"), Change::Objects(change));
            doc.dirty.objects = true;
            self.selection_rev += 1;
        }
        if let Some(r) = area { self.height_changed(r); }
        let name = |p: &String| p.rsplit('\\').next().unwrap_or(p).trim_end_matches(".p3d").to_string();
        let what = if models.len() == 1 { name(&models[0]) } else { tf("{0} models", &[&models.len().to_string()]) };
        self.status = tf("Replaced {0} objects with {1}", &[&n.to_string(), &what]);
        if skipped > 0 {
            self.warn(tf("{0} objects made by generators were left as they are; change the generator's models instead.", &[&skipped.to_string()]));
        }
    }

    /// Puts the copy without land contact (`_nolc`) in place of each selected model that the
    /// game stands on the ground, where there is one. Each object stays where it was shown, and
    /// from then on keeps the elevation it is given.
    pub fn use_free_variants(&mut self) {
        let Some(doc) = &mut self.doc else { return };
        let w = &mut doc.world;
        let mut change = Vec::new();
        for &id in &self.selection {
            let Some(old) = w.objects.get(id).copied() else { continue };
            let path = w.objects.models[old.model as usize].path.clone();
            let info = self.db.get(&path);
            if !dz_core::model_info::stands_on_contacts(&info) {
                continue;
            }
            let Some(free) = self.db.free_variant(&path) else { continue };
            let mut new = old;
            new.model = w.objects.intern(&free);
            if old.flags & dz_core::objects::FLAG_ABSOLUTE == 0 {
                new.elevation += dz_core::model_info::shown_lift(&old, &info, &w.height);
            }
            w.objects.update(id, new);
            change.push((id, Some(old), Some(new)));
        }
        let n = change.len();
        if n > 0 {
            for (id, _, _) in &change {
                w.sync_attached(*id);
            }
            doc.history.push(t("Replace objects"), Change::Objects(change));
            doc.dirty.objects = true;
            self.selection_rev += 1;
        }
        self.info(tf("{0} objects now use the copy that can be raised", &[&n.to_string()]));
    }

    pub fn remember_selection(&mut self) {
        let Some(doc) = &self.doc else { return };
        if self.selection.is_empty() {
            return;
        }
        let key = (self.selection_rev, doc.world.objects.revision);
        if self.last_copy.as_ref().is_some_and(|c| c.1 == key) {
            return;
        }
        if let Some(p) = self.selection_layout() {
            self.last_copy = Some((p, key));
        }
    }

    /// Copies of the selection (`last` false) or of the last selection under the cursor.
    pub fn paste_at_cursor(&mut self, last: bool) {
        let Some(at) = self.cursor_ground else { return };
        let layout = if last { self.last_copy.as_ref().map(|c| c.0.clone()) } else { self.selection_layout() };
        let Some(layout) = layout else {
            self.warn(if last { t("Nothing was selected yet") } else { t("Select objects first") });
            return;
        };
        self.place_layout(&layout, at[0], at[2], t("Copy objects"));
    }

    /// Objects laid out as a prefab, with its middle at (x, z), selected once placed.
    pub fn place_layout(&mut self, prefab: &dz_core::library::Prefab, x: f32, z: f32, label: String) -> Vec<u32> {
        let Some(doc) = self.doc.as_mut() else { return Vec::new() };
        let layer = doc.world.target_layer();
        if !doc.world.layer_interactive(layer) {
            let w = layer_warning(&doc.world, layer);
            self.warn(w);
            return Vec::new();
        }
        let mut change = Vec::new();
        let mut ids = Vec::new();
        for (model, mut o) in prefab.objects_at(x, z, 0.0, &doc.world.height) {
            o.model = doc.world.objects.intern(&model);
            o.set_layer(layer);
            let n = doc.world.objects.insert(o);
            change.push((n, None, Some(o)));
            ids.push(n);
        }
        if change.is_empty() {
            return ids;
        }
        doc.history.push(label, Change::Objects(change));
        doc.dirty.objects = true;
        self.whole = None;
        self.selected_generator = None;
        self.selection = ids.iter().copied().collect();
        self.selection_rev += 1;
        ids
    }

    /// Tilts the selected objects to the ground under them; models the game fits to the
    /// ground themselves are left level.
    pub fn align_selection_to_ground(&mut self) {
        let Some(doc) = &mut self.doc else { return };
        let mut change = Vec::new();
        for id in self.selection.iter() {
            let Some(o) = doc.world.objects.get(*id).copied() else { continue };
            let mut n = o;
            if !self.db.get(doc.world.objects.model_path(&o)).placement.skews() {
                (n.pitch, n.roll) = dz_core::gen::slope_angles(&doc.world.height, o.x, o.z, o.yaw);
            }
            if n != o {
                doc.world.objects.update(*id, n);
                change.push((*id, Some(o), Some(n)));
            }
        }
        if !change.is_empty() {
            doc.history.push(t("Align to the ground"), Change::Objects(change));
            doc.dirty.objects = true;
        }
    }

    /// In team work, says once when a step changed or deleted something someone else made: a
    /// modifier, road or generator of theirs, their layer or objects on it.
    fn warn_foreign_edits(&mut self) {
        let Some(doc) = &self.doc else { return };
        let pushed = doc.history.pushed;
        if pushed == self.team_seen {
            return;
        }
        let new = pushed.saturating_sub(self.team_seen).min(16) as usize;
        self.team_seen = pushed;
        if doc.world.team_me.is_empty() {
            return;
        }
        let found: Vec<dz_core::team::Foreign> = doc.history.recent(new).flat_map(|c| dz_core::team::foreign_in(c, &doc.world)).collect();
        for f in found {
            if !self.team_warned.insert(f.key.clone()) {
                continue;
            }
            let text = if f.removed {
                tf("You deleted {0}, which belongs to {1}. Undo (Ctrl+Z) if you did not mean to.", &[&f.name, &f.owner])
            } else {
                tf("You changed {0}, which belongs to {1}. Undo (Ctrl+Z) if you did not mean to.", &[&f.name, &f.owner])
            };
            self.warn(text);
        }
    }

    /// Sets the selected objects back to their own size.
    pub fn reset_selection_scale(&mut self) {
        let Some(doc) = &mut self.doc else { return };
        let mut change = Vec::new();
        for id in self.selection.iter() {
            let Some(o) = doc.world.objects.get(*id).copied() else { continue };
            if o.scale != 1.0 {
                let n = dz_core::objects::Obj { scale: 1.0, ..o };
                doc.world.objects.update(*id, n);
                change.push((*id, Some(o), Some(n)));
            }
        }
        if !change.is_empty() {
            doc.history.push(t("Reset scale"), Change::Objects(change));
            doc.dirty.objects = true;
            self.selection_rev += 1;
        }
    }

    /// Turns the selected objects the way `source` faces. With "Stick to the ground" they take
    /// its heading only and tilt with the ground under them.
    pub fn copy_direction_from(&mut self, source: u32) {
        let Some(doc) = &mut self.doc else { return };
        let Some(s) = doc.world.objects.get(source).copied() else { return };
        let w = &mut doc.world;
        let mut change = Vec::new();
        for id in self.selection.iter().filter(|id| **id != source) {
            let Some(o) = w.objects.get(*id).copied() else { continue };
            let mut n = dz_core::objects::Obj { yaw: s.yaw, pitch: s.pitch, roll: s.roll, ..o };
            if self.settings.align_to_ground && !self.db.get(w.objects.model_path(&o)).placement.skews() {
                (n.pitch, n.roll) = dz_core::gen::slope_angles(&w.height, n.x, n.z, n.yaw);
            }
            if n != o {
                w.objects.update(*id, n);
                change.push((*id, Some(o), Some(n)));
            }
        }
        let mut area: Option<Rect> = None;
        for (id, _, _) in &change {
            if let Some(r) = w.sync_attached(*id) { area = Some(area.map_or(r, |a| a.union(r))); }
        }
        if change.is_empty() {
            self.status = t("The selection already faces that way");
        } else {
            doc.history.push(t("Copy direction"), Change::Objects(change));
            doc.dirty.objects = true;
            self.selection_rev += 1;
            self.status.clear();
        }
        if let Some(r) = area { self.height_changed(r); }
    }

    /// The selected objects with their models' boxes, in the order of the selection.
    pub fn selection_items(&self) -> Vec<(dz_core::objects::Obj, dz_core::model_info::ModelInfo)> {
        let Some(doc) = &self.doc else { return Vec::new() };
        let w = &doc.world;
        self.selection.iter().filter_map(|id| {
            let o = w.objects.get(*id).copied()?;
            Some((o, self.db.get(w.objects.model_path(&o))))
        }).collect()
    }

    /// The row copies of the selection are laid in, ahead or behind, with the gap of the
    /// settings between them.
    pub fn selection_row(&self, items: &[(dz_core::objects::Obj, dz_core::model_info::ModelInfo)], ahead: bool) -> Option<dz_core::objects::Row> {
        let mut row = dz_core::objects::row_of_turned(items, ahead, self.settings.row_angle)?;
        row.step = (row.step + self.settings.row_gap).max(0.1);
        Some(row)
    }

    /// Copies of a row turned at random as the settings say, the same way each time until
    /// a row is laid.
    /// The models of the row's preset, with their weights, as far as they are in the project;
    /// `None` when the copies keep the models of the selection.
    pub fn row_choices(&self) -> Option<Vec<(u32, f32)>> {
        let preset = self.library.objects.iter().find(|p| Some(p.id) == self.settings.row_preset)?;
        let w = &self.doc.as_ref()?.world;
        let list: Vec<(u32, f32)> = preset.items.iter().filter_map(|(m, k)| Some((w.objects.index_of(m)?, *k))).collect();
        (!list.is_empty()).then_some(list)
    }

    /// Puts the models of the row's preset into the project, so copies and their preview can use them.
    pub fn intern_row_preset(&mut self) {
        let Some(preset) = self.library.objects.iter().find(|p| Some(p.id) == self.settings.row_preset) else { return };
        let Some(doc) = self.doc.as_mut() else { return };
        for (m, _) in &preset.items {
            doc.world.objects.intern(m);
        }
    }

    /// The models of the copies of a row: from the row's preset when one is chosen.
    pub fn row_models(&self, copies: &mut [dz_core::objects::Obj]) {
        if let Some(choices) = self.row_choices() {
            dz_core::objects::pick_models(copies, &choices, self.row_seed);
        }
    }

    pub fn row_random_turns(&self, copies: &mut [dz_core::objects::Obj], items: &[(dz_core::objects::Obj, dz_core::model_info::ModelInfo)]) {
        let info: Vec<_> = items.iter().map(|(_, i)| *i).collect();
        dz_core::objects::random_turns(copies, &info, self.settings.row_flip, self.settings.row_turn, self.row_seed);
    }

    /// How the selection moves to join its neighbour, with the gap of the settings between
    /// them, and the neighbour's end it then turns about.
    pub fn join_shift(&self) -> Option<([f32; 2], [f32; 2])> {
        let (mine, theirs, other) = self.selection_link()?;
        // The gap opens along the selection, away from the neighbour.
        let (dx, dz) = (other[0] - mine[0], other[1] - mine[1]);
        let l = dx.hypot(dz).max(1e-4);
        let g = self.settings.row_gap;
        Some(([theirs[0] - mine[0] + dx / l * g, theirs[1] - mine[1] + dz / l * g], theirs))
    }

    /// Where the selection joins a neighbour in a row: its end there, the neighbour's end and
    /// the other end of the selection. Neighbours are objects outside the selection within half
    /// the selection's length.
    pub fn selection_link(&self) -> Option<([f32; 2], [f32; 2], [f32; 2])> {
        let doc = self.doc.as_ref()?;
        let w = &doc.world;
        let items = self.selection_items();
        let angle = self.settings.row_angle;
        let row = dz_core::objects::row_of_turned(&items, true, angle)?;
        let reach = (row.step * 0.5).clamp(0.5, 4.0) + self.settings.row_gap.max(0.0);
        let back = dz_core::objects::row_of_turned(&items, false, angle)?;
        let mut others = Vec::new();
        for end in [row.joint, back.joint] {
            w.objects.for_each_in(end[0] - 12.0, end[1] - 12.0, end[0] + 12.0, end[1] + 12.0, |id, o| {
                if !self.selection.contains(&id) && w.object_interactive(o) && !others.iter().any(|(i, _): &(u32, _)| *i == id) {
                    others.push((id, *o));
                }
            });
        }
        let others: Vec<_> = others.into_iter().map(|(_, o)| (o, self.db.get(w.objects.model_path(&o)))).filter(|(_, i)| i.known).collect();
        dz_core::objects::link_to_turned(&items, &others, reach, angle).map(|(k, mine, theirs)| (mine, theirs, if k == 0 { back.joint } else { row.joint }))
    }

    /// Moves the selection so its end meets the end of the neighbour it joins in a row.
    pub fn join_selection_to_neighbour(&mut self) {
        let Some((shift, _)) = self.join_shift() else {
            self.warn(t("No object ends near the ends of the selection"));
            return;
        };
        let Some(doc) = &mut self.doc else { return };
        let w = &mut doc.world;
        let mut change = Vec::new();
        for id in self.selection.iter() {
            let Some(o) = w.objects.get(*id).copied() else { continue };
            let n = dz_core::objects::Obj { x: o.x + shift[0], z: o.z + shift[1], ..o };
            if n != o {
                w.objects.update(*id, n);
                change.push((*id, Some(o), Some(n)));
            }
        }
        let mut area: Option<Rect> = None;
        for (id, _, _) in &change {
            if let Some(r) = w.sync_attached(*id) { area = Some(area.map_or(r, |a| a.union(r))); }
        }
        if !change.is_empty() {
            doc.history.push(t("Join to the neighbour"), Change::Objects(change));
            doc.dirty.objects = true;
            self.selection_rev += 1;
        }
        if let Some(r) = area { self.height_changed(r); }
    }

    /// Lays a copy of the selection one length further on along it, `ahead` or back, and
    /// selects the copy, so pressing again goes on laying a row.
    pub fn copy_selection_along(&mut self, ahead: bool) {
        self.copy_selection_row(ahead, 1, 0.0);
    }

    /// Lays `count` copies of the selection in a row ahead of it or behind, the row turned by
    /// `turn` degrees about where the selection ends, and selects the last copy, so the row
    /// can go on from there.
    pub fn copy_selection_row(&mut self, ahead: bool, count: usize, turn: f32) {
        if self.whole.is_some() || self.selection.is_empty() || count == 0 {
            return;
        }
        let ids: Vec<u32> = self.selection.iter().copied().collect();
        let Some(doc) = &mut self.doc else { return };
        let w = &mut doc.world;
        let items: Vec<(dz_core::objects::Obj, dz_core::model_info::ModelInfo)> = ids.iter().filter_map(|id| {
            let o = w.objects.get(*id).copied()?;
            Some((o, self.db.get(w.objects.model_path(&o))))
        }).collect();
        if items.len() != ids.len() {
            return;
        }
        let Some(row) = self.selection_row(&items, ahead) else { return };
        let mut moved = dz_core::objects::row_copies(&items, &row, count, turn);
        self.intern_row_preset();
        self.row_models(&mut moved);
        self.row_random_turns(&mut moved, &items);
        // The next row turns its copies another way.
        self.row_seed = self.row_seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let Some(doc) = &mut self.doc else { return };
        let w = &mut doc.world;
        let before_mods = w.modifiers.clone();
        let mut change = Vec::new();
        let mut last = Vec::new();
        let mut area: Option<Rect> = None;
        for (k, to) in moved.iter().enumerate() {
            let (old, id) = (&items[k % ids.len()].0, ids[k % ids.len()]);
            let (c, map) = copy_objects(w, &[id]);
            change.extend(c);
            let Some(&n) = map.get(&id) else { continue };
            if k / ids.len() + 1 == count { last.push(n); }
            let mut o = *to;
            // An object at a height of its own keeps it above the ground where it goes.
            if o.flags & dz_core::objects::FLAG_ABSOLUTE != 0 {
                o.elevation += w.height.sample(o.x, o.z) - w.height.sample(old.x, old.z);
            }
            if self.settings.align_to_ground && !self.db.get(w.objects.model_path(&o)).placement.skews() {
                (o.pitch, o.roll) = dz_core::gen::slope_angles(&w.height, o.x, o.z, o.yaw);
            }
            w.objects.update(n, o);
            if let Some(c) = change.iter_mut().find(|c| c.0 == n) { c.2 = Some(o); }
            if let Some(r) = w.sync_attached(n) { area = Some(area.map_or(r, |a| a.union(r))); }
        }
        if change.is_empty() {
            return;
        }
        let mut steps = vec![Change::Objects(change)];
        if w.modifiers.len() != before_mods.len() {
            steps.push(Change::Modifiers(before_mods));
        }
        doc.history.push(t("Copy on along the selection"), Change::Group(steps));
        doc.dirty.objects = true;
        self.selection = last.into_iter().collect();
        self.selection_rev += 1;
        if let Some(r) = area { self.height_changed(r); }
    }

    /// Copies the selection in place. A whole road or generator is copied as one, with what
    /// belongs to it; objects take the terrain modifiers attached to them along.
    pub fn duplicate_selection(&mut self) {
        match self.whole {
            Some(SelectedWhole::Route(id)) => return self.duplicate_route(id),
            Some(SelectedWhole::Generator(id)) => return self.duplicate_generator(id),
            None => {}
        }
        let ids: Vec<u32> = self.selection.iter().copied().collect();
        let Some(doc) = &mut self.doc else { return };
        let before_mods = doc.world.modifiers.clone();
        let (change, map) = copy_objects(&mut doc.world, &ids);
        if change.is_empty() {
            return;
        }
        let mut steps = vec![Change::Objects(change)];
        if doc.world.modifiers.len() != before_mods.len() {
            steps.push(Change::Modifiers(before_mods));
        }
        doc.history.push(t("Duplicate objects"), Change::Group(steps));
        doc.dirty.objects = true;
        self.selection = map.values().copied().collect();
        self.selection_rev += 1;
    }

    /// Copies a road with its parts, its line and settings, the terrain levelled under it and
    /// the generators that follow it.
    fn duplicate_route(&mut self, id: u64) {
        let Some(doc) = &mut self.doc else { return };
        let Some(route) = doc.world.routes.iter().find(|r| r.id == id).cloned() else { return };
        let (before_routes, before_mods, before_gens) = (doc.world.routes.clone(), doc.world.modifiers.clone(), doc.world.generators.clone());
        let (mut change, map) = copy_objects(&mut doc.world, &route.objects);
        let new_id = dz_core::library::new_id();
        let mut copy = route.clone();
        copy.id = new_id;
        copy.name = copy_name(&route.name, |n| doc.world.routes.iter().any(|r| r.name == n));
        copy.objects = route.objects.iter().filter_map(|o| map.get(o).copied()).collect();
        copy.modifier = None;
        doc.world.routes.push(copy);
        let index = doc.world.routes.len() - 1;
        // Fences, lamps and the like along the road go with it.
        let followers: Vec<dz_core::library::GeneratorInstance> = doc.world.generators.iter().filter(|g| g.target == dz_core::library::GenTarget::Route(id)).cloned().collect();
        for g in followers {
            let (c, m) = copy_objects(&mut doc.world, &g.objects);
            change.extend(c);
            let mut n = g.clone();
            n.id = dz_core::library::new_id();
            n.target = dz_core::library::GenTarget::Route(new_id);
            n.objects = g.objects.iter().filter_map(|o| m.get(o).copied()).collect();
            n.modifier = None;
            n.mask_modifier = None;
            n.removed.clear();
            doc.world.generators.push(n);
        }
        let selection: BTreeSet<u32> = doc.world.routes[index].objects.iter().copied().collect();
        doc.dirty.objects = true;
        self.sync_route_modifier(index);
        let Some(doc) = &mut self.doc else { return };
        doc.history.push(t("Duplicate road"), Change::Group(vec![Change::Objects(change), Change::Routes(before_routes), Change::Modifiers(before_mods), Change::Generators(before_gens)]));
        self.selection = selection;
        self.whole = Some(SelectedWhole::Route(new_id));
        self.line.selected_route = Some(index);
        self.selection_rev += 1;
    }

    /// Copies a generator with its objects, its terrain modifier and the shape it works on.
    fn duplicate_generator(&mut self, id: u64) {
        use dz_core::library::GenTarget;
        let Some(doc) = &mut self.doc else { return };
        let Some(g) = doc.world.generators.iter().find(|g| g.id == id).cloned() else { return };
        let (before_gens, before_mods, before_shapes) = (doc.world.generators.clone(), doc.world.modifiers.clone(), doc.world.shape_layers.clone());
        let (change, map) = copy_objects(&mut doc.world, &g.objects);
        let mut n = g.clone();
        n.id = dz_core::library::new_id();
        n.name = copy_name(&g.name, |x| doc.world.generators.iter().any(|g| g.name == x));
        n.objects = g.objects.iter().filter_map(|o| map.get(o).copied()).collect();
        // What it took off the map is gone already, and its painted ground is a picture.
        n.removed.clear();
        n.mask_modifier = None;
        n.modifier = g.modifier.and_then(|m| doc.world.modifiers.iter().find(|x| x.id == m).cloned()).map(|mut m| {
            m.id = doc.world.new_modifier_id();
            m.name = copy_name(&m.name, |_| false);
            let id = m.id;
            doc.world.modifiers.push(m);
            id
        });
        let mut shapes_changed = false;
        if let GenTarget::Shape { layer, index } = &g.target {
            if let Some(l) = doc.world.shape_layers.iter_mut().find(|l| l.name == *layer) {
                if let Some(s) = l.shapes.get(*index).cloned() {
                    l.shapes.push(s);
                    n.target = GenTarget::Shape { layer: layer.clone(), index: l.shapes.len() - 1 };
                    shapes_changed = true;
                }
            }
        }
        let new_id = n.id;
        doc.world.generators.push(n);
        let mut steps = vec![Change::Objects(change), Change::Generators(before_gens)];
        if doc.world.modifiers.len() != before_mods.len() {
            steps.push(Change::Modifiers(before_mods));
        }
        if shapes_changed {
            steps.push(Change::Shapes(before_shapes));
        }
        doc.history.push(t("Duplicate generator"), Change::Group(steps));
        doc.dirty.objects = true;
        self.selection = map.values().copied().collect();
        self.whole = Some(SelectedWhole::Generator(new_id));
        self.selected_generator = Some(new_id);
        self.selection_rev += 1;
    }

    /// Makes what belongs to a whole road or generator follow the move of its objects: the
    /// line and levelled terrain of a road, the modifier and the shape of a generator.
    /// Returns the states to undo to.
    pub fn move_whole(&mut self, m: &dz_core::world::Motion) -> Vec<Change> {
        use dz_core::library::GenTarget;
        let mut steps = Vec::new();
        let Some(whole) = self.whole else { return steps };
        let Some(doc) = &mut self.doc else { return steps };
        match whole {
            SelectedWhole::Route(id) => {
                let Some(i) = doc.world.routes.iter().position(|r| r.id == id) else { return steps };
                steps.push(Change::Routes(doc.world.routes.clone()));
                doc.world.routes[i].moved(m);
                if let Some(before) = self.sync_route_and_crossings(i) {
                    steps.push(Change::Modifiers(before));
                }
            }
            SelectedWhole::Generator(id) => {
                let Some(g) = doc.world.generators.iter().find(|g| g.id == id).cloned() else { return steps };
                if let Some(k) = g.modifier.and_then(|mid| doc.world.modifiers.iter().position(|x| x.id == mid)) {
                    steps.push(Change::Modifiers(doc.world.modifiers.clone()));
                    let old = doc.world.modifier_rect(&doc.world.modifiers[k]);
                    doc.world.modifiers[k].moved(m);
                    let new = doc.world.modifier_rect(&doc.world.modifiers[k]);
                    let area = match (old, new) { (Some(a), Some(b)) => Some(a.union(b)), (a, b) => a.or(b) };
                    if let Some(r) = area {
                        doc.world.recompose(Some(r));
                        self.height_changed(r);
                    }
                }
                let Some(doc) = &mut self.doc else { return steps };
                match &g.target {
                    GenTarget::Shape { layer, index } => {
                        if let Some(s) = doc.world.shape_layers.iter().position(|l| l.name == *layer).filter(|l| doc.world.shape_layers[*l].shapes.len() > *index) {
                            steps.push(Change::Shapes(doc.world.shape_layers.clone()));
                            for p in &mut doc.world.shape_layers[s].shapes[*index].points {
                                *p = m.apply(*p);
                            }
                        }
                    }
                    GenTarget::Point(p) => {
                        steps.push(Change::Generators(doc.world.generators.clone()));
                        let moved = m.apply(*p);
                        if let Some(x) = doc.world.generators.iter_mut().find(|x| x.id == id) {
                            x.target = GenTarget::Point(moved);
                        }
                    }
                    _ => {}
                }
            }
        }
        steps
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
            if let Some(mut m) = gen::pad_modifier(&mut doc.world, *id, hx * o.scale, hz * o.scale, self.place.pad_margin, falloff) {
                m.follow = self.place.pad_follow;
                doc.world.modifiers.push(m);
                added += 1;
            }
        }
        if added > 0 {
            doc.world.recompose(None);
            doc.history.push(t("Add terrain modifiers"), Change::Modifiers(before));
            self.whole_height_changed();
            if self.place.pad_follow {
                self.set_pad_follow(ids, true);
            }
        }
    }

    /// Makes the pads under these objects level the ground to the height the object stands
    /// at, or to the ground at its middle again. An object followed this way gets a height of
    /// its own, where it stands now, so that the ground it raises does not lift it as well.
    pub fn set_pad_follow(&mut self, ids: &[u32], on: bool) {
        let Some(doc) = &mut self.doc else { return };
        let (mods_before, mut objects) = (doc.world.modifiers.clone(), Vec::new());
        let mut changed = false;
        for id in ids {
            let has = doc.world.modifiers.iter().any(|m| m.attached == Some(*id));
            // Only an object its ground followed goes back down when that ends.
            let was = doc.world.modifiers.iter().any(|m| m.attached == Some(*id) && m.follow);
            if !has || (!on && !was) { continue; }
            for m in doc.world.modifiers.iter_mut().filter(|m| m.attached == Some(*id)) {
                if m.follow != on {
                    m.follow = on;
                    if !on { m.target = None; }
                    changed = true;
                }
            }
            if let Some(o) = doc.world.objects.get(*id).copied() {
                // Followed, the object keeps the height it stands at; no longer followed, it goes
                // back down onto the terrain, which no longer comes up to it.
                let now = if on {
                    if o.flags & dz_core::objects::FLAG_ABSOLUTE != 0 { o } else {
                        dz_core::objects::Obj { elevation: dz_core::objects::ObjectStore::origin_y(&o, &doc.world.height), flags: o.flags | dz_core::objects::FLAG_ABSOLUTE, ..o }
                    }
                } else {
                    on_ground(&self.db, doc.world.objects.model_path(&o), o)
                };
                if now != o {
                    doc.world.objects.update(*id, now);
                    objects.push((*id, Some(o), Some(now)));
                }
            }
        }
        if !changed && objects.is_empty() { return; }
        let label = t("Ground follows the object");
        let mut steps = 1;
        if !objects.is_empty() {
            doc.history.push(label.clone(), Change::Objects(objects));
            doc.dirty.objects = true;
            steps += 1;
        }
        doc.history.push(label.clone(), Change::Modifiers(mods_before));
        doc.history.group_last(steps, label);
        for id in ids { doc.world.sync_attached(*id); }
        doc.world.recompose(None);
        self.whole_height_changed();
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
                let lite_ids: Vec<u32> = lite.objects.iter().map(|(id, _)| id).collect();
                let count = objects.len();
                let ids = app.merge_generated(&label2, models, objects);
                let map: HashMap<u32, u32> = lite_ids.into_iter().zip(ids).collect();
                if let Some(d) = &mut app.doc {
                    for mut r in lite.routes {
                        r.objects = r.objects.iter().filter_map(|i| map.get(i).copied()).collect();
                        d.world.routes.push(r);
                    }
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
            let mut w = dz_core::import::import_plan(&plan, &assets, &db, job)?;
            translate_object_groups(&mut w);
            let missing = dz_core::import::layers_without_surface(&w, db.root());
            Ok(Box::new(move |app: &mut App| {
                // Tiles over the surface limit are marked in the view straight away.
                let over = dz_core::mask_limit::scan_detailed(&w, &dz_core::mask_limit::FixOptions::default());
                app.open_world(w, None);
                app.warn_layers_without_surface(&missing);
                if !over.is_empty() {
                    app.warn(tf("{0} texture tiles use more than the surfaces allowed; they are marked in the 2D view and Project settings can fix them", &[&over.len().to_string()]));
                    app.tile_limit.issues = over;
                }
            }) as Finish)
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
        layer.shapes.push(MapShape { holes: Vec::new(), kind, points, attributes: Vec::new() });
        self.shape.selection = BTreeSet::from([(w.active_shape_layer, layer.shapes.len() - 1)]);
        doc.history.push(t("Add shape"), Change::Shapes(previous));
    }

    /// Adds the water courses of the terrain as lines of a new shape layer, as one undo step.
    pub fn find_water_courses(&mut self) {
        let Some(d) = &self.doc else { return };
        let small = dz_core::analysis::downsample(&d.world.height, dz_core::analysis::MAX_SIDE);
        let sea = d.world.meta.sea_level;
        let (wetness, length) = (self.shape.course_wetness, self.shape.course_length);
        self.spawn(t("Water courses from the terrain"), move |job| {
            job.stage("Working out where water runs");
            let wet = dz_core::analysis::wetness(&small, sea);
            let lines = dz_core::analysis::water_courses(&small, &wet, sea, wetness, length);
            Ok(Box::new(move |app: &mut App| {
                if lines.is_empty() {
                    app.warn(t("No water course is long enough: lower the wetness or the shortest length"));
                    return;
                }
                let Some(doc) = &mut app.doc else { return };
                let previous = doc.world.shape_layers.clone();
                let shapes: Vec<MapShape> = lines.into_iter().map(|points| MapShape { holes: Vec::new(), kind: dz_core::shapes::ShapeKind::Polyline, points, attributes: Vec::new() }).collect();
                let n = shapes.len();
                doc.world.shape_layers.push(ShapeLayer { name: t("Water courses"), visible: true, shapes });
                doc.world.active_shape_layer = doc.world.shape_layers.len() - 1;
                app.shape.selection.clear();
                doc.history.push(t("Water courses from the terrain"), Change::Shapes(previous));
                app.info(tf("{0} water courses added as lines", &[&n.to_string()]));
            }) as Finish)
        });
    }

    /// The selected shapes that are closed areas, which objects can lie inside.
    pub fn selected_areas(&self) -> usize {
        let Some(doc) = &self.doc else { return 0 };
        self.shape.selection.iter().filter(|(l, s)| doc.world.shape_layers.get(*l).and_then(|l| l.shapes.get(*s)).is_some_and(|s| s.kind == dz_core::shapes::ShapeKind::Polygon)).count()
    }

    /// Selects the objects inside the selected area shapes and goes to the Select tool, where they
    /// can be moved or deleted. With `add` they join what is already selected.
    pub fn select_inside_shapes(&mut self, add: bool) {
        let Some(doc) = &self.doc else { return };
        let world = &doc.world;
        let mut found = Vec::new();
        for (l, s) in &self.shape.selection {
            if let Some(shape) = world.shape_layers.get(*l).and_then(|l| l.shapes.get(*s)).filter(|s| s.kind == dz_core::shapes::ShapeKind::Polygon) {
                found.extend(world.objects_inside(&shape.points));
            }
        }
        if !add {
            self.selection.clear();
            self.selected_modifier = None;
            self.selected_generator = None;
            self.whole = None;
        }
        self.selection.extend(found);
        self.selection_rev += 1;
        self.mode = Mode::Select;
        let n = self.selection.len().to_string();
        self.info(tf("{0} objects selected", &[&n]));
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

    /// Key points from Terrain Builder: a project or an exported names file.
    pub fn import_keypoints(&mut self) {
        let Some(file) = rfd::FileDialog::new().add_filter("Terrain Builder", &["tv4p", "hpp"]).pick_file() else { return };
        let Some(doc) = &mut self.doc else { return };
        let is_project = file.extension().is_some_and(|e| e.eq_ignore_ascii_case("tv4p"));
        let points = match std::fs::read(&file) {
            Ok(b) if is_project => dz_core::keypoints::parse_tv4p(&b, [doc.world.meta.easting, doc.world.meta.northing]),
            Ok(b) => dz_core::keypoints::parse_hpp(&String::from_utf8_lossy(&b)),
            Err(e) => { self.error(format!("{e:#}")); return }
        };
        if points.is_empty() {
            self.warn(t("No key points in this file"));
            return;
        }
        let before = doc.world.keypoints.clone();
        let n = dz_core::keypoints::merge(&mut doc.world, points);
        doc.history.push(t("Import key points"), Change::KeyPoints(before));
        self.info(tf("{0} key points read", &[&n.to_string()]));
    }

    pub fn add_keypoint(&mut self, x: f32, z: f32) {
        let Some(doc) = &mut self.doc else { return };
        let before = doc.world.keypoints.clone();
        let mut n = doc.world.keypoints.len() + 1;
        while doc.world.keypoints.iter().any(|k| k.name == format!("KeyPoint{n}")) { n += 1; }
        let name = format!("KeyPoint{n}");
        doc.world.keypoints.push(dz_core::keypoints::KeyPoint { text: name.clone(), name, x, z, ..Default::default() });
        doc.history.push(t("Add key point"), Change::KeyPoints(before));
        self.shape.keypoint = Some(doc.world.keypoints.len() - 1);
    }

    pub fn delete_keypoint(&mut self, i: usize) {
        let Some(doc) = &mut self.doc else { return };
        if i >= doc.world.keypoints.len() { return; }
        let before = doc.world.keypoints.clone();
        doc.world.keypoints.remove(i);
        doc.history.push(t("Delete key point"), Change::KeyPoints(before));
        self.shape.keypoint = None;
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
        let db = self.db.clone();
        self.spawn(t("Import Terrain Builder layers"), move |job| {
            let layers: Vec<dz_core::import::LayerFile> = files.iter().filter_map(|p| dz_core::tv4l::read_info(p).ok().map(|info| dz_core::import::LayerFile { path: p.clone(), info, import: true })).collect();
            let libraries = files.first().map(|f| dz_core::import::libraries_near(f)).unwrap_or_default();
            anyhow::ensure!(!libraries.is_empty(), "No TemplateLibs folder next to the layers");
            let before = lite.object_layers.len();
            dz_core::import::add_tb_layers(&mut lite, &layers, &libraries, &db, job)?;
            Ok(Box::new(move |app: &mut App| {
                // The libraries stay with the project, for the content browser to show.
                if let Some(d) = app.doc.as_mut() {
                    for l in libraries {
                        if !d.world.template_libraries.contains(&l) { d.world.template_libraries.push(l); }
                    }
                }
                app.merge_layers(lite, before)
            }) as Finish)
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
            LayerAction::Satellite(l) => layers[l as usize].satellite ^= true,
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
            LayerAction::Owner(l, owner) => {
                if layers[l as usize].owner != owner {
                    let previous = layers.clone();
                    layers[l as usize].owner = owner;
                    doc.history.push(t("Owner of the layer"), Change::ObjectLayers(previous));
                }
            }
            LayerAction::Unlock(l) => {
                let name = layers[l as usize].name.clone();
                if !doc.world.team_unlocked.remove(&name) {
                    doc.world.team_unlocked.insert(name);
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
            let mut w = dz_core::import::import_wrp(&f, None, &db, job)?;
            translate_object_groups(&mut w);
            // The map's own surfaces, from its folder (the one holding `world`).
            if let Some(cfg) = f.parent().and_then(|p| p.parent()).filter(|m| m.parent().is_some()).and_then(|m| dz_core::import::find_surfaces_cfg(m).into_iter().next()) {
                match dz_core::import::read_surfaces_cfg(&mut w, &cfg, db.root()) {
                    Ok(n) => job.log(format!("{}: {n} surfaces of the map", cfg.display())),
                    Err(e) => job.log(format!("{}: {e:#}", cfg.display())),
                }
            }
            let missing = dz_core::import::layers_without_surface(&w, db.root());
            Ok(Box::new(move |app: &mut App| {
                app.open_world(w, None);
                app.warn_layers_without_surface(&missing);
            }) as Finish)
        });
    }

    pub fn import_wrp(&mut self) {
        let Some(f) = rfd::FileDialog::new().add_filter("WRP", &["wrp"]).pick_file() else { return };
        let db = self.db.clone();
        self.spawn(t("Importing WRP"), move |job| {
            let mut w = dz_core::import::import_wrp(&f, None, &db, job)?;
            translate_object_groups(&mut w);
            // The map's own surfaces, from its folder (the one holding `world`).
            if let Some(cfg) = f.parent().and_then(|p| p.parent()).filter(|m| m.parent().is_some()).and_then(|m| dz_core::import::find_surfaces_cfg(m).into_iter().next()) {
                match dz_core::import::read_surfaces_cfg(&mut w, &cfg, db.root()) {
                    Ok(n) => job.log(format!("{}: {n} surfaces of the map", cfg.display())),
                    Err(e) => job.log(format!("{}: {e:#}", cfg.display())),
                }
            }
            let missing = dz_core::import::layers_without_surface(&w, db.root());
            Ok(Box::new(move |app: &mut App| {
                app.open_world(w, None);
                app.warn_layers_without_surface(&missing);
            }) as Finish)
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
                app.apply_heights(&t("Import heightmap"), g.map.data);
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
                d.world.satellite = Some(img);
                // The new picture is the original the satellite modifiers now lie on.
                d.world.sat_image_replaced();
                app.renderer.lock().unwrap().set_satellite(d.world.satellite.as_ref());
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
        let db = self.db.clone();
        self.import_into_project(t("Import objects"), move |w, a, _| {
            let text = String::from_utf8_lossy(&std::fs::read(&f)?).into_owned();
            let (list, missing) = dz_core::import::parse_tb_objects(&text, w, a, &db, false);
            for o in list {
                w.objects.insert(o);
            }
            Ok(missing.into_iter().map(|m| format!("? {m}")).collect())
        });
    }

    pub fn import_dayz_objects(&mut self) {
        let Some(f) = rfd::FileDialog::new().add_filter("DayZ Editor / Object Spawner", &["dze", "json"]).pick_file() else { return };
        let classes = crate::mission_ui::class_index(self);
        let work_drive = self.settings.tools.work_drive.clone();
        let layer_name = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "DayZ Editor".into());
        self.import_into_project(t("Import DayZ Editor / Object Spawner"), move |w, a, _| {
            let loaded = if classes.is_none() {
                let root = PathBuf::from(&work_drive);
                root.is_dir().then(|| dz_core::mission::classes::ClassIndex::scan_work_drive(&root))
            } else {
                None
            };
            let cls = classes.as_deref().or(loaded.as_ref());
            let bytes = std::fs::read(&f)?;
            let (list, missing) = dz_core::dayz_editor::parse_dayz_objects(&bytes, w, a, cls)?;
            if !list.is_empty() {
                let layer = w.add_object_layer(&layer_name);
                for mut o in list {
                    o.set_layer(layer);
                    w.objects.insert(o);
                }
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
            Ok(dz_core::import::import_tv4p_report(&f, w, Some(0))?.1)
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
    /// Builds the map (and plays it), after the map check when it is turned on.
    pub fn build(&mut self, run: bool) {
        if self.settings.check_before_build {
            crate::map_check::run(self, Some(run));
        } else {
            self.build_checked(run);
        }
    }

    /// Builds the map (and plays it) without checking it first.
    pub fn build_checked(&mut self, run: bool) {
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
            dz_core::build::build_pbo(&paths, &id, &w.meta.build_folders, job)?;
            let session = if run {
                match &w.mission {
                    // The mission of the project, when it has one.
                    Some(m) => {
                        let target = paths.dayz.join("mpmissions").join(format!("dayzOffline.{id}"));
                        job.log(format!("Mission: {}", target.display()));
                        let mut m = (**m).clone();
                        m.write(&target, false, false)?;
                    }
                    None => {
                        dz_core::build::prepare_mission(&paths, &id, w.meta.spawn, job)?;
                    }
                }
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

    pub fn shot_path_set(&mut self, path: String) {
        self.shot_path = Some(path);
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

/// Bridges of a road, and the points where it has to meet what it crosses.
fn route_decks(world: &dz_core::world::World, i: usize) -> Vec<dz_core::world::BridgeDeck> {
    use dayz_road_tool::rails::is_track_family as track;
    use dz_core::world::ModShape;
    let route = &world.routes[i];
    let mut decks = route.decks.clone();
    if track(&route.family) {
        return decks;
    }
    for (j, other) in world.routes.iter().enumerate() {
        let rail = track(&other.family);
        if j == i || (!rail && j > i) {
            continue;
        }
        let shaped = other.modifier.and_then(|id| world.modifiers.iter().find(|m| m.id == id)).and_then(|m| match &m.shape {
            ModShape::Line { points, heights, gaps, .. } => Some((points.iter().zip(heights).map(|(p, h)| [p[0], p[1], *h]).collect::<Vec<_>>(), gaps.clone())),
            _ => None,
        });
        let graded = shaped.is_some();
        let ground = world.ground();
        let (line, gaps) = shaped.unwrap_or_else(|| (other.centerline.iter().map(|p| [p[0], p[1], ground.sample(p[0], p[1])]).collect(), Vec::new()));
        let lift = match (rail, graded) {
            (true, true) => gen::RAIL_CROSSING_LIFT,
            (true, false) => gen::RAIL_CROSSING_LIFT_UNGRADED,
            _ => 0.0,
        };
        decks.extend(gen::crossing_decks(&route.centerline, &line, &gaps, lift, &route.decks));
    }
    decks
}

/// When a route was only lengthened at one end since its last fit: whether at its end, and
/// its new waypoints beyond the old end, nearest first.
/// A `side` x `side` picture of a square RGBA texture `size` pixels wide, each pixel the
/// average of the block it stands for.
fn layer_thumb(rgba: &[u8], size: usize, side: usize) -> egui::ColorImage {
    let step = (size / side).max(1);
    let mut out = Vec::with_capacity(side * side * 4);
    for y in 0..side {
        for x in 0..side {
            let mut sum = [0u32; 3];
            for dy in 0..step {
                for dx in 0..step {
                    let k = ((y * step + dy).min(size - 1) * size + (x * step + dx).min(size - 1)) * 4;
                    for c in 0..3 {
                        sum[c] += rgba.get(k + c).copied().unwrap_or(0) as u32;
                    }
                }
            }
            let n = (step * step) as u32;
            out.extend_from_slice(&[(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8, 255]);
        }
    }
    egui::ColorImage::from_rgba_unmultiplied([side, side], &out)
}

/// The layers an import sorts objects into, named in the language of the interface.
fn translate_object_groups(w: &mut dz_core::world::World) {
    for l in &mut w.object_layers {
        if dz_core::world::OBJECT_GROUPS.contains(&l.name.as_str()) {
            l.name = t(&l.name);
        }
    }
}

fn extension(r: &dz_core::world::RoadRoute) -> Option<(bool, Vec<[f32; 2]>)> {
    let (old, now) = (&r.fitted, &r.points);
    // The waypoints of the last fit tell it best; without them it is guessed from the line.
    if old.len() < 2 {
        return beyond_centerline(r);
    }
    if now.len() <= old.len() {
        return None;
    }
    let extra = now.len() - old.len();
    if now[..old.len()] == old[..] {
        return Some((true, now[old.len()..].to_vec()));
    }
    if now[extra..] == old[..] {
        return Some((false, now[..extra].iter().rev().copied().collect()));
    }
    None
}

/// For a route without a record of its last fit: the waypoints past either end of its centre
/// line are the new ones, when there are some at one end only.
fn beyond_centerline(r: &dz_core::world::RoadRoute) -> Option<(bool, Vec<[f32; 2]>)> {
    let (line, points) = (&r.centerline, &r.points);
    let (Some(&first), Some(&last)) = (line.first(), line.last()) else { return None };
    let d = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
    let near = |end: [f32; 2]| (0..points.len()).min_by(|a, b| d(points[*a], end).total_cmp(&d(points[*b], end))).filter(|k| d(points[*k], end) < 15.0);
    let (at_start, at_end) = (near(first)?, near(last)?);
    match (at_start > 0, at_end + 1 < points.len()) {
        (false, true) => Some((true, points[at_end + 1..].to_vec())),
        (true, false) => Some((false, points[..at_start].iter().rev().copied().collect())),
        _ => None,
    }
}

#[cfg(test)]
mod extension_tests {
    #[test]
    fn a_route_lengthened_at_one_end_is_told_apart() {
        let mut r: dz_core::world::RoadRoute = serde_json::from_value(serde_json::json!({
            "name": "r", "family": "f", "points": [[0.0, 0.0], [10.0, 0.0]], "centerline": [], "objects": [],
            "terrain_route": false, "settings": dayz_road_tool::document::RoutingSettings::default(),
        })).unwrap();
        r.fitted = r.points.clone();
        r.parts = vec![dayz_road_tool::roads::PlacedPart { model: "a".into(), reverse: false, position: [0.0, 0.0], rotation: 0.0 }];
        r.points.push([20.0, 5.0]);
        assert_eq!(super::extension(&r), Some((true, vec![[20.0, 5.0]])));
        r.points = vec![[-20.0, 0.0], [-10.0, 0.0], [0.0, 0.0], [10.0, 0.0]];
        assert_eq!(super::extension(&r), Some((false, vec![[-10.0, 0.0], [-20.0, 0.0]])));
        // A point moved inside the route needs the whole fit.
        r.points = vec![[0.0, 1.0], [10.0, 0.0], [20.0, 0.0]];
        assert_eq!(super::extension(&r), None);
        // A route that keeps no record is told from its centre line.
        r.parts.clear();
        r.fitted.clear();
        r.centerline = vec![[0.0, 0.0], [5.0, 0.0], [10.0, 0.0]];
        r.points = vec![[0.0, 0.0], [10.0, 0.0], [30.0, 0.0], [40.0, 5.0]];
        assert_eq!(super::extension(&r), Some((true, vec![[30.0, 0.0], [40.0, 5.0]])));
        r.points = vec![[-30.0, 0.0], [0.0, 0.0], [10.0, 0.0]];
        assert_eq!(super::extension(&r), Some((false, vec![[-30.0, 0.0]])));
    }
}

/// An object standing on the terrain: its height counted from the ground again, with the
/// model resting on its land contact points.
/// Why objects cannot go to a layer: it is hidden, locked, or someone else's in team work.
pub fn layer_warning(w: &World, layer: u16) -> String {
    match w.object_layers.get(layer as usize) {
        Some(l) if w.layer_foreign(layer) => tf("The active layer belongs to {0}. Unlock it in the outliner to work on it anyway.", &[&l.owner]),
        _ => t("The active layer is hidden or locked"),
    }
}

pub fn on_ground(db: &dz_core::model_info::ModelDb, path: &str, o: dz_core::objects::Obj) -> dz_core::objects::Obj {
    let info = db.get(path);
    let elevation = if info.contact_count > 0 { -info.contact_y * o.scale } else { 0.0 };
    dz_core::objects::Obj { elevation, flags: o.flags & !dz_core::objects::FLAG_ABSOLUTE, ..o }
}

/// "name (copy)", or with a number when that is taken.
fn copy_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    let copy = t("copy");
    (1..).map(|n| if n == 1 { format!("{base} ({copy})") } else { format!("{base} ({copy} {n})") }).find(|n| !taken(n)).unwrap()
}

/// Copies objects in place with the terrain modifiers attached to them. Returns the change to
/// undo and the copy of each object.
fn copy_objects(w: &mut dz_core::world::World, ids: &[u32]) -> (Vec<(u32, Option<dz_core::objects::Obj>, Option<dz_core::objects::Obj>)>, std::collections::HashMap<u32, u32>) {
    let mut change = Vec::new();
    let mut map = std::collections::HashMap::new();
    for id in ids {
        let Some(o) = w.objects.get(*id).copied() else { continue };
        let n = w.objects.insert(o);
        change.push((n, None, Some(o)));
        map.insert(*id, n);
        let attached: Vec<dz_core::world::TerrainModifier> = w.modifiers.iter().filter(|m| m.attached == Some(*id)).cloned().collect();
        for mut m in attached {
            m.id = w.new_modifier_id();
            m.attached = Some(n);
            w.modifiers.push(m);
        }
    }
    (change, map)
}
