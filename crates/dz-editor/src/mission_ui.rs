//! The mission editor: the server mission of the map (economy, spawns, territories, weather and
//! the rest). The side panel of the Mission mode holds the map tools; the mission window holds
//! the tables and settings of every file. Every change is a step of undo.
use crate::app::{App, Mode};
use crate::i18n::{t, tf};
use crate::icons;
use crate::theme::{self, row, section, tool_button};
use dz_core::edit::Change;
use dz_core::mission::places::{self, Place, PlaceKind, PlaceRef};
use dz_core::mission::xml::{Element, Node};
use dz_core::mission::{self, FlagLayer, Mission, MissionEdit};
use eframe::egui::{self, vec2, Color32, Pos2, RichText, Stroke};
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tab {
    #[default]
    Overview,
    Types,
    Events,
    Spawnable,
    Players,
    Territories,
    Areas,
    Weather,
    Economy,
    Gameplay,
    Messages,
    MapGroups,
    Flags,
    Init,
    Files,
    Check,
}

impl Tab {
    pub const ALL: [Tab; 16] = [Tab::Overview, Tab::Types, Tab::Events, Tab::Spawnable, Tab::Players, Tab::Territories, Tab::Areas, Tab::Weather, Tab::Economy, Tab::Gameplay, Tab::Messages, Tab::MapGroups, Tab::Flags, Tab::Init, Tab::Files, Tab::Check];
    pub fn title(self) -> String {
        match self {
            Tab::Overview => t("Overview"),
            Tab::Types => t("Loot (types)"),
            Tab::Events => t("Events"),
            Tab::Spawnable => t("Spawnable types"),
            Tab::Players => t("Player spawns"),
            Tab::Territories => t("Territories"),
            Tab::Areas => t("Effect areas"),
            Tab::Weather => t("Weather"),
            Tab::Economy => t("Economy"),
            Tab::Gameplay => t("Gameplay"),
            Tab::Messages => t("Messages"),
            Tab::MapGroups => t("Map groups"),
            Tab::Flags => t("Area flags"),
            Tab::Init => "init.c".into(),
            Tab::Files => t("Files"),
            Tab::Check => t("Check"),
        }
    }
    fn icon(self) -> &'static str {
        match self {
            Tab::Overview => icons::FLAG_BANNER,
            Tab::Types => icons::PACKAGE,
            Tab::Events => icons::LIGHTNING,
            Tab::Spawnable => icons::BACKPACK,
            Tab::Players => icons::PERSON_SIMPLE_WALK,
            Tab::Territories => icons::PAW_PRINT,
            Tab::Areas => icons::RADIOACTIVE,
            Tab::Weather => icons::CLOUD_SUN,
            Tab::Economy => icons::COINS,
            Tab::Gameplay => icons::SLIDERS,
            Tab::Messages => icons::CHAT_TEXT,
            Tab::MapGroups => icons::BUILDINGS,
            Tab::Flags => icons::GRID_FOUR,
            Tab::Init => icons::FILE_CODE,
            Tab::Files => icons::FOLDER_OPEN,
            Tab::Check => icons::CHECK_CIRCLE,
        }
    }
    pub fn from_name(n: &str) -> Option<Tab> {
        Some(match n {
            "overview" => Tab::Overview,
            "types" => Tab::Types,
            "events" => Tab::Events,
            "spawnable" => Tab::Spawnable,
            "players" => Tab::Players,
            "territories" => Tab::Territories,
            "areas" => Tab::Areas,
            "weather" => Tab::Weather,
            "economy" => Tab::Economy,
            "gameplay" => Tab::Gameplay,
            "messages" => Tab::Messages,
            "mapgroups" => Tab::MapGroups,
            "flags" => Tab::Flags,
            "init" => Tab::Init,
            "files" => Tab::Files,
            "check" => Tab::Check,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MapTool {
    Select,
    EventSpawn,
    PlayerSpawn,
    Zone,
    Area,
    SafePosition,
    Trigger,
    Flags,
}

impl MapTool {
    const ALL: [MapTool; 8] = [MapTool::Select, MapTool::EventSpawn, MapTool::PlayerSpawn, MapTool::Zone, MapTool::Area, MapTool::SafePosition, MapTool::Trigger, MapTool::Flags];
    fn title(self) -> String {
        match self {
            MapTool::Select => t("Select and move"),
            MapTool::EventSpawn => t("Event spawn"),
            MapTool::PlayerSpawn => t("Player spawn"),
            MapTool::Zone => t("Territory zone"),
            MapTool::Area => t("Effect area"),
            MapTool::SafePosition => t("Safe position"),
            MapTool::Trigger => t("Underground trigger"),
            MapTool::Flags => t("Paint area flags"),
        }
    }
}

/// Places of the mission as last gathered, kept until the mission changes.
#[derive(Default)]
struct PlaceCache {
    rev: u64,
    kinds: Vec<PlaceKind>,
    list: Vec<Place>,
    /// The groups of cfgeventgroups.xml, for the spawns that place them.
    groups: HashMap<String, (String, usize, Vec<places::GroupChild>)>,
    valid: bool,
}

/// Cells touched by a stroke of the flag brush, as they were before it, by blocks of cells.
struct FlagStroke {
    layer: FlagLayer,
    blocks: HashMap<(u32, u32), Vec<u32>>,
}
const FLAG_BLOCK: u32 = 64;

/// What the mission editor remembers between frames.
pub struct MissionState {
    pub open: bool,
    pub tab: Tab,
    pub tool: MapTool,
    pub shown: BTreeSet<PlaceKind>,
    pub show_flags: bool,
    pub selected: Option<PlaceRef>,
    drag: Option<(PlaceRef, Option<MissionEdit>)>,
    cache: PlaceCache,
    /// An element edit waiting for the gesture to end, merged into one undo step.
    pending: Option<(String, MissionEdit)>,
    // Tools.
    pub event: String,
    /// The group of cfgeventgroups.xml new event spawns place; empty for none.
    pub event_group: String,
    /// The event group open in the Events tab, and the settings of making one from props.
    pub group_sel: Option<String>,
    group_name: String,
    group_de: bool,
    group_loot: (i64, i64),
    group_with_spawn: bool,
    /// The configs of the work drive, read once, and the objects of the event groups.
    pub classes: Option<std::sync::Arc<mission::classes::ClassIndex>>,
    classes_job: Option<std::sync::mpsc::Receiver<mission::classes::ClassIndex>>,
    pub ghosts: Ghosts,
    /// The objects of event groups show as models.
    pub show_models: bool,
    /// The mission editor is a window of its own, outside the main window.
    pub own_window: bool,
    pub spawn_section: String,
    pub spawn_group: String,
    pub zone_file: String,
    pub zone_territory: usize,
    pub zone_name: String,
    pub zone_radius: f32,
    pub zone_static: [i64; 2],
    pub zone_dynamic: [i64; 2],
    pub area_radius: f32,
    pub area_name: String,
    pub trigger_size: [f32; 3],
    pub flag_layer: FlagLayer,
    pub flag_bit: u32,
    pub flag_radius: f32,
    pub flag_erase: bool,
    flag_stroke: Option<FlagStroke>,
    flags_tex: Option<egui::TextureHandle>,
    flags_key: Option<(u64, FlagLayer, u32)>,
    flag_counts: Option<(u64, FlagLayer, Vec<usize>)>,
    // Tables.
    pub type_filter: String,
    type_list_filter: [String; 4],
    type_file: Option<String>,
    type_sel: BTreeSet<(String, usize)>,
    type_focus: Option<(String, usize)>,
    type_anchor: Option<usize>,
    type_rows: Option<(u64, Vec<(String, usize, mission::ce::TypeRow)>)>,
    bulk_factor: f32,
    bulk_field: usize,
    bulk_value: i64,
    bulk_list: usize,
    bulk_name: String,
    new_name: String,
    event_filter: String,
    event_sel: Option<(String, usize)>,
    list_filter: String,
    list_sel: HashMap<String, usize>,
    file_sel: Option<String>,
    section: String,
    player_group: Option<usize>,
    issues: Option<(u64, Vec<mission::validate::Issue>)>,
    issue_levels: [bool; 3],
    last_report: Option<mission::gen::GroupReport>,
    keypoint_kinds: BTreeSet<String>,
    with_clusters: bool,
}

impl Default for MissionState {
    fn default() -> Self {
        Self {
            open: false,
            tab: Tab::Overview,
            tool: MapTool::Select,
            shown: [PlaceKind::EventSpawn, PlaceKind::PlayerSpawn, PlaceKind::Territory, PlaceKind::EffectArea, PlaceKind::SafePosition, PlaceKind::Trigger, PlaceKind::Breadcrumb].into_iter().collect(),
            show_flags: false,
            selected: None,
            drag: None,
            cache: PlaceCache::default(),
            pending: None,
            event: String::new(),
            event_group: String::new(),
            group_sel: None,
            group_name: String::new(),
            group_de: true,
            group_loot: (2, 3),
            group_with_spawn: true,
            classes: None,
            classes_job: None,
            ghosts: Ghosts::default(),
            show_models: true,
            own_window: false,
            spawn_section: "fresh".into(),
            spawn_group: "Spawn".into(),
            zone_file: String::new(),
            zone_territory: 0,
            zone_name: "InfectedVillageTier1".into(),
            zone_radius: 60.0,
            zone_static: [0, 0],
            zone_dynamic: [3, 5],
            area_radius: 100.0,
            area_name: "Contaminated".into(),
            trigger_size: [10.0, 5.0, 10.0],
            flag_layer: FlagLayer::Usage,
            flag_bit: 0,
            flag_radius: 50.0,
            flag_erase: false,
            flag_stroke: None,
            flags_tex: None,
            flags_key: None,
            flag_counts: None,
            type_filter: String::new(),
            type_list_filter: Default::default(),
            type_file: None,
            type_sel: BTreeSet::new(),
            type_focus: None,
            type_anchor: None,
            type_rows: None,
            bulk_factor: 1.0,
            bulk_field: 0,
            bulk_value: 0,
            bulk_list: 0,
            bulk_name: String::new(),
            new_name: String::new(),
            event_filter: String::new(),
            event_sel: None,
            list_filter: String::new(),
            list_sel: HashMap::new(),
            file_sel: None,
            section: "fresh".into(),
            player_group: None,
            issues: None,
            issue_levels: [true, true, false],
            last_report: None,
            keypoint_kinds: ["NameCity", "NameCityCapital", "NameVillage"].into_iter().map(String::from).collect(),
            with_clusters: true,
        }
    }
}

// ---------------------------------------------------------------- access and undo

pub fn mission(app: &App) -> Option<&Mission> {
    app.doc.as_ref()?.world.mission.as_deref()
}

pub fn mission_mut(app: &mut App) -> Option<&mut Mission> {
    app.doc.as_mut()?.world.mission.as_deref_mut()
}

fn rev(app: &App) -> u64 {
    mission(app).map(|m| m.rev).unwrap_or(0)
}

/// Records finished edits (in the order they were made) as one step of undo.
pub fn push(app: &mut App, label: &str, mut edits: Vec<MissionEdit>) {
    flush(app);
    if edits.is_empty() {
        return;
    }
    edits.reverse();
    if let Some(doc) = &mut app.doc {
        doc.history.push(label.to_string(), Change::Mission(edits));
    }
}

/// Keeps an edit of a gesture in progress; edits of the same element or file join it.
fn pend(app: &mut App, label: &str, edit: MissionEdit) {
    let same = match (&app.mission.pending, &edit) {
        (Some((_, MissionEdit::Element(a, _))), MissionEdit::Element(b, _)) => a == b,
        (Some((_, MissionEdit::File(a, _))), MissionEdit::File(b, _)) => a == b,
        _ => false,
    };
    if same {
        return;
    }
    flush(app);
    app.mission.pending = Some((label.to_string(), edit));
}

/// Ends the gesture in progress.
pub fn flush(app: &mut App) {
    if let Some((label, e)) = app.mission.pending.take() {
        if let Some(doc) = &mut app.doc {
            doc.history.push(label, Change::Mission(vec![e]));
        }
    }
}

/// Changes one element with undo; `f` changes it in place.
fn edit_el(app: &mut App, label: &str, file: &str, path: &[usize], f: impl FnOnce(&mut Element)) {
    let Some(m) = mission_mut(app) else { return };
    if let Some(e) = mission::element_edit(m, file, path, f) {
        pend(app, label, e);
    }
}

/// Changes a whole file with undo, joined while the gesture lasts.
fn edit_file(app: &mut App, label: &str, file: &str, f: impl FnOnce(&mut Mission)) {
    let Some(m) = mission_mut(app) else { return };
    let e = mission::file_edit(m, file, f);
    pend(app, label, e);
}

/// Ends the gesture when the mouse is up and no field is typed into.
fn settle(app: &mut App, ctx: &egui::Context) {
    if app.mission.pending.is_some() && !ctx.input(|i| i.pointer.any_down()) && !ctx.wants_keyboard_input() {
        flush(app);
    }
}

// ---------------------------------------------------------------- creating, importing, exporting

/// Gives the project a new mission made for its map.
pub fn new_mission(app: &mut App) {
    let Some(doc) = &app.doc else { return };
    let m = mission::templates::new_mission(&doc.world.meta);
    set_mission(app, Some(m), &t("New mission"));
    app.info(t("A new mission was made for the map."));
}

fn set_mission(app: &mut App, m: Option<Mission>, label: &str) {
    flush(app);
    let Some(doc) = &mut app.doc else { return };
    let before = std::mem::replace(&mut doc.world.mission, m.map(Box::new));
    doc.history.push(label.to_string(), Change::Mission(vec![MissionEdit::Whole(before)]));
    app.mission.selected = None;
    app.mission.cache.valid = false;
    app.mission.type_rows = None;
    app.mission.issues = None;
    app.mission.flags_key = None;
}

/// Reads a mission folder into the project; `adapt` makes it the mission of this map.
pub fn import_from(app: &mut App, dir: std::path::PathBuf, adapt: bool) {
    match Mission::load(&dir) {
        Ok(m) => {
            let Some(doc) = &app.doc else { return };
            let m = if adapt { mission::templates::adapt(m, &doc.world.meta) } else { m };
            let name = m.name.clone();
            set_mission(app, Some(m), &t("Import mission"));
            app.info(tf("Mission {0} read from {1}", &[&name, &dir.display().to_string()]));
        }
        Err(e) => app.error(format!("{e:#}")),
    }
}

fn pick_folder(title: &str) -> Option<std::path::PathBuf> {
    rfd::FileDialog::new().set_title(title).pick_folder()
}

pub fn import(app: &mut App, adapt: bool) {
    if let Some(dir) = pick_folder(&t("Choose a mission folder")) {
        if !Mission::is_mission_dir(&dir) {
            app.warn(tf("{0} does not look like a mission folder (no init.c, db/types.xml or cfgeconomycore.xml)", &[&dir.display().to_string()]));
        }
        import_from(app, dir, adapt);
    }
}

/// Writes the mission into `dir\<mission name>`.
pub fn export_to(app: &mut App, dir: &std::path::Path, storage: bool) {
    let Some(m) = mission(app) else { return };
    let mut copy = m.clone();
    let target = dir.join(&copy.name);
    match copy.write(&target, false, storage) {
        Ok(files) => app.info(tf("Exported {0} files to {1}", &[&files.len().to_string(), &target.display().to_string()])),
        Err(e) => app.error(format!("{e:#}")),
    }
}

pub fn export(app: &mut App) {
    if let Some(dir) = pick_folder(&t("Choose the mpmissions folder")) {
        export_to(app, &dir, false);
    }
}

// ---------------------------------------------------------------- the side panel

pub fn panel(app: &mut App, ui: &mut egui::Ui) {
    settle(app, ui.ctx());
    if app.doc.is_none() {
        return;
    }
    if mission(app).is_none() {
        section(ui, &icons::with(icons::FLAG_BANNER, &t("Mission")), true, |ui| {
            ui.label(t("The project has no server mission yet."));
            ui.weak(t("A mission holds the economy, spawns, territories and settings the server reads from mpmissions."));
            ui.add_space(4.0);
            if ui.button(icons::with(icons::FILE_PLUS, &t("New mission for this map"))).clicked() { new_mission(app); }
            if ui.button(icons::with(icons::FOLDER_OPEN, &t("Import a mission..."))).on_hover_text(t("Reads a mission folder as it is, for this map or another.")).clicked() { import(app, false); }
            if ui.button(icons::with(icons::ARROWS_LEFT_RIGHT, &t("Adapt a mission of another map..."))).on_hover_text(t("Keeps the economy of the mission and drops what was placed on the other map: spawns, zones, areas, map groups and area flags.")).clicked() { import(app, true); }
        });
        return;
    }
    let (name, files, dirty) = mission(app).map(|m| (m.name.clone(), m.files.len(), m.files.values().filter(|f| f.dirty).count())).unwrap();
    section(ui, &icons::with(icons::FLAG_BANNER, &t("Mission")), true, |ui| {
        ui.label(RichText::new(&name).strong());
        ui.weak(tf("{0} files, {1} changed since saved", &[&files.to_string(), &dirty.to_string()]));
        ui.horizontal_wrapped(|ui| {
            if ui.button(icons::with(icons::APP_WINDOW, &t("Mission editor"))).clicked() { app.mission.open = true; }
            if ui.button(icons::with(icons::EXPORT, &t("Export..."))).clicked() { export(app); }
        });
    });
    section(ui, &icons::with(icons::STACK, &t("Show on the map")), true, |ui| {
        for k in PlaceKind::ALL {
            let mut on = app.mission.shown.contains(&k);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().circle_filled(r.center(), 4.5, kind_color(k));
                if ui.checkbox(&mut on, t(k.label())).changed() {
                    if on { app.mission.shown.insert(k); } else { app.mission.shown.remove(&k); }
                }
            });
        }
        ui.checkbox(&mut app.mission.show_flags, t("Area flags (2D view)"));
        ui.checkbox(&mut app.mission.show_models, t("Objects of event groups as models")).on_hover_text(t("Every object a static event places is drawn as its model at each spawn; the class gives the model through the configs of the work drive."));
        if app.mission.show_models {
            if app.mission.classes_job.is_some() {
                ui.weak(t("Reading the classes of the configs..."));
            }
            let unknown: Vec<String> = app.mission.ghosts.unknown.iter().cloned().collect();
            if !unknown.is_empty() {
                egui::CollapsingHeader::new(RichText::new(tf("No model found for {0} classes", &[&unknown.len().to_string()])).color(theme::WARN)).id_salt("ghost_unknown").show(ui, |ui| {
                    for u in unknown.iter().take(100) { ui.label(RichText::new(u).monospace().small()); }
                });
            }
        }
    });
    section(ui, &icons::with(icons::CURSOR_CLICK, &t("Tool")), true, |ui| {
        for tool in MapTool::ALL {
            if ui.selectable_label(app.mission.tool == tool, tool.title()).clicked() {
                app.mission.tool = tool;
                if tool == MapTool::Flags { app.mission.show_flags = true; }
            }
        }
        ui.separator();
        tool_settings(app, ui);
    });
    section(ui, &icons::with(icons::MAP_PIN, &t("Selected")), true, |ui| selected_ui(app, ui));
}

fn tool_settings(app: &mut App, ui: &mut egui::Ui) {
    let Some(m) = app.doc.as_ref().and_then(|d| d.world.mission.as_deref()) else { return };
    match app.mission.tool {
        MapTool::Select => {
            ui.weak(t("Click picks a place, drag moves it, Delete removes it."));
        }
        MapTool::EventSpawn => {
            let events = mission::ce::event_names(m);
            row(ui, &t("Event"), |ui| {
                egui::ComboBox::from_id_salt("ms_event").selected_text(app.mission.event.clone()).height(400.0).show_ui(ui, |ui| {
                    for e in &events { ui.selectable_value(&mut app.mission.event, e.clone(), e); }
                });
            });
            ui.text_edit_singleline(&mut app.mission.event);
            let groups = mission::ce::event_group_names(m);
            row(ui, &t("Group"), |ui| {
                let shown = if app.mission.event_group.is_empty() { t("(none)") } else { app.mission.event_group.clone() };
                egui::ComboBox::from_id_salt("ms_event_group").selected_text(shown).height(400.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut app.mission.event_group, String::new(), t("(none)"));
                    for g in &groups { ui.selectable_value(&mut app.mission.event_group, g.clone(), g); }
                });
            });
            ui.weak(t("Click on the map adds a spawn of the event."));
            ui.weak(t("Static events (convoys, trains, police) have no children; each spawn places a group of cfgeventgroups.xml around it, at the height of the ground."));
        }
        MapTool::PlayerSpawn => {
            row(ui, &t("Section"), |ui| {
                for s in ["fresh", "hop", "travel"] { ui.selectable_value(&mut app.mission.spawn_section, s.to_string(), s); }
            });
            row(ui, &t("Group"), |ui| { ui.text_edit_singleline(&mut app.mission.spawn_group); });
            ui.weak(t("Click on the map adds a spawn position to the group."));
        }
        MapTool::Zone => {
            let files = m.territory_files();
            if app.mission.zone_file.is_empty() || !files.contains(&app.mission.zone_file) {
                app.mission.zone_file = files.iter().find(|f| f.contains("zombie")).or(files.first()).cloned().unwrap_or_else(|| "env/zombie_territories.xml".into());
            }
            let territories = m.xml(&app.mission.zone_file).map(|d| d.root.children_named("territory").count()).unwrap_or(0);
            row(ui, &t("File"), |ui| {
                egui::ComboBox::from_id_salt("ms_zone_file").selected_text(app.mission.zone_file.clone()).show_ui(ui, |ui| {
                    for f in &files { ui.selectable_value(&mut app.mission.zone_file, f.clone(), f); }
                });
            });
            row(ui, &t("Territory"), |ui| {
                ui.add(egui::DragValue::new(&mut app.mission.zone_territory).range(0..=territories.max(1) - 1).prefix("#"));
                ui.weak(tf("of {0}", &[&territories.to_string()]));
            });
            row(ui, &t("Name"), |ui| { ui.text_edit_singleline(&mut app.mission.zone_name); });
            row(ui, &t("Radius"), |ui| { ui.add(egui::DragValue::new(&mut app.mission.zone_radius).range(1.0..=5000.0).suffix(" m")); });
            row(ui, "smin / smax", |ui| { ui.add(egui::DragValue::new(&mut app.mission.zone_static[0])); ui.add(egui::DragValue::new(&mut app.mission.zone_static[1])); });
            row(ui, "dmin / dmax", |ui| { ui.add(egui::DragValue::new(&mut app.mission.zone_dynamic[0])); ui.add(egui::DragValue::new(&mut app.mission.zone_dynamic[1])); });
            ui.weak(t("Click on the map adds a zone."));
        }
        MapTool::Area => {
            row(ui, &t("Name"), |ui| { ui.text_edit_singleline(&mut app.mission.area_name); });
            row(ui, &t("Radius"), |ui| { ui.add(egui::DragValue::new(&mut app.mission.area_radius).range(1.0..=5000.0).suffix(" m")); });
            ui.weak(t("Click on the map adds a contaminated area."));
        }
        MapTool::SafePosition => {
            ui.weak(t("Click on the map adds a position players are moved to out of an area."));
        }
        MapTool::Trigger => {
            row(ui, &t("Size"), |ui| {
                for v in &mut app.mission.trigger_size { ui.add(egui::DragValue::new(v).range(0.1..=500.0).speed(0.1)); }
            });
            ui.weak(t("Click on the map adds a trigger box on the ground."));
        }
        MapTool::Flags => flag_tool_ui(app, ui),
    }
}

fn flag_names(m: &Mission, l: FlagLayer) -> Vec<String> {
    m.limits(if l == FlagLayer::Usage { "usageflags" } else { "valueflags" })
}

fn flag_tool_ui(app: &mut App, ui: &mut egui::Ui) {
    let Some(m) = app.doc.as_ref().and_then(|d| d.world.mission.as_deref()) else { return };
    if m.areaflags().is_none() {
        ui.label(t("The mission has no areaflags.map."));
        if ui.button(t("Make one for the map")).clicked() { make_areaflags(app); }
        return;
    }
    row(ui, &t("Layer"), |ui| {
        ui.selectable_value(&mut app.mission.flag_layer, FlagLayer::Usage, t("Usage"));
        ui.selectable_value(&mut app.mission.flag_layer, FlagLayer::Value, t("Value"));
    });
    let names = flag_names(m, app.mission.flag_layer);
    let bits = m.areaflags().map(|a| a.bits(app.mission.flag_layer)).unwrap_or(0);
    for b in 0..bits.min(32) {
        let name = names.get(b as usize).cloned().unwrap_or_else(|| tf("bit {0}", &[&b.to_string()]));
        if (b as usize) >= names.len() && b > 0 && (b as usize) > names.len() + 1 { continue; }
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), egui::Sense::hover());
            ui.painter().rect_filled(r, 2.0, flag_color(b));
            if ui.selectable_label(app.mission.flag_bit == b, name).clicked() { app.mission.flag_bit = b; }
        });
    }
    row(ui, &t("Radius"), |ui| { ui.add(egui::DragValue::new(&mut app.mission.flag_radius).range(1.0..=2000.0).suffix(" m")); });
    ui.checkbox(&mut app.mission.flag_erase, t("Erase"));
    ui.weak(t("The map shows every flag of the layer; the chosen one is the one painted and stands out."));
    ui.weak(t("Drag on the map paints the flag; with Shift it erases."));
}

fn selected_ui(app: &mut App, ui: &mut egui::Ui) {
    let Some(at) = app.mission.selected.clone() else {
        ui.weak(t("Nothing selected"));
        return;
    };
    let Some(p) = places_now(app).iter().find(|p| p.at == at).cloned() else {
        app.mission.selected = None;
        return;
    };
    ui.label(RichText::new(t(p.kind.label())).strong());
    if !p.label.is_empty() { ui.label(&p.label); }
    if p.kind == PlaceKind::EventSpawn {
        let groups = mission(app).map(mission::ce::event_group_names).unwrap_or_default();
        let n = p.sub.as_ref().and_then(|g| app.mission.cache.groups.get(&g.to_ascii_lowercase())).map(|g| g.2.len());
        let mut pick = p.sub.clone();
        row(ui, &t("Group"), |ui| {
            egui::ComboBox::from_id_salt("sel_group").selected_text(pick.clone().unwrap_or_else(|| t("(none)"))).height(400.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut pick, None, t("(none)"));
                for g in &groups { ui.selectable_value(&mut pick, Some(g.clone()), g); }
            });
        });
        if let Some(n) = n { ui.weak(tf("{0} objects in the group", &[&n.to_string()])); }
        if pick != p.sub {
            let y = app.doc.as_ref().map(|d| d.world.height.sample(p.x, p.z) - d.world.meta.sea_level).unwrap_or(0.0);
            let e = mission_mut(app).and_then(|m| places::set_spawn_group(m, &at, pick.as_deref(), y));
            push(app, &t("Event group"), e.into_iter().collect());
        }
        if let Some(g) = &p.sub {
            if ui.button(icons::with(icons::PENCIL_SIMPLE, &t("Edit the group"))).clicked() {
                app.mission.group_sel = Some(g.clone());
                app.mission.event_filter = p.group.clone();
                let ev = mission(app).and_then(|m| m.ce_files("events").into_iter().find_map(|f| {
                    let d = m.xml(&f)?;
                    d.root.children.iter().position(|n| matches!(n, Node::Element(e) if e.name == "event" && e.attr("name") == Some(p.group.as_str()))).map(|i| (f.clone(), i))
                }));
                app.mission.event_sel = ev;
                app.mission.tab = Tab::Events;
                app.mission.open = true;
            }
        }
    }
    let (mut x, mut z, mut r) = (p.x, p.z, p.radius);
    let mut changed = false;
    row(ui, "X", |ui| { changed |= ui.add(egui::DragValue::new(&mut x).speed(0.5).max_decimals(3)).changed(); });
    row(ui, "Z", |ui| { changed |= ui.add(egui::DragValue::new(&mut z).speed(0.5).max_decimals(3)).changed(); });
    if changed {
        if let Some(e) = mission_mut(app).and_then(|m| places::move_to(m, &at, x, z, None)) { pend(app, &t("Move"), e); }
    }
    if matches!(p.kind, PlaceKind::Territory | PlaceKind::EffectArea | PlaceKind::Breadcrumb) || (p.kind == PlaceKind::EventSpawn && p.radius > 0.0) {
        if p.kind != PlaceKind::EventSpawn {
            let mut rc = false;
            row(ui, &t("Radius"), |ui| { rc = ui.add(egui::DragValue::new(&mut r).range(0.0..=10000.0).suffix(" m")).changed(); });
            if rc {
                if let Some(e) = mission_mut(app).and_then(|m| places::set_radius(m, &at, r)) { pend(app, &t("Radius"), e); }
            }
        }
    }
    // The rest of an XML place: its attributes.
    if let PlaceRef::Xml { file, path } = &at {
        if let Some(el) = mission(app).and_then(|m| places::element(m, &at)).cloned() {
            let mut copy = el.clone();
            ui.separator();
            attrs_ui(ui, &mut copy, &format!("sel{file}{path:?}"));
            if copy != el {
                let (f, p) = (file.clone(), path.clone());
                edit_el(app, &t("Edit"), &f, &p, |e| *e = copy);
            }
        }
    }
    if let PlaceRef::Json { .. } = &at {
        if let Some(v) = mission(app).and_then(|m| places::json_value(m, &at)).cloned() {
            let mut copy = v.clone();
            ui.separator();
            egui::ScrollArea::vertical().id_salt("sel_json").max_height(260.0).show(ui, |ui| { json_ui(ui, &mut copy, "seljson"); });
            if copy != v {
                if let Some(e) = mission_mut(app).and_then(|m| places::edit_json(m, &at, |x| *x = copy)) { pend(app, &t("Edit"), e); }
            }
        }
    }
    ui.horizontal(|ui| {
        if ui.button(icons::with(icons::TRASH, &t("Delete"))).clicked() {
            delete_selected(app);
        }
        if ui.button(icons::with(icons::CROSSHAIR, &t("Go to"))).clicked() {
            app.top_center = [p.x, p.z];
            app.camera.pos[0] = p.x;
            app.camera.pos[2] = p.z - 60.0;
        }
    });
}

fn delete_selected(app: &mut App) {
    let Some(at) = app.mission.selected.take() else { return };
    let edits: Vec<MissionEdit> = mission_mut(app).and_then(|m| places::remove(m, &at)).into_iter().collect();
    push(app, &t("Delete"), edits);
}

// ---------------------------------------------------------------- places on the map

pub fn kind_color(k: PlaceKind) -> Color32 {
    match k {
        PlaceKind::EventSpawn => Color32::from_rgb(255, 170, 60),
        PlaceKind::PlayerSpawn => Color32::from_rgb(90, 220, 110),
        PlaceKind::Territory => Color32::from_rgb(220, 90, 90),
        PlaceKind::EffectArea => Color32::from_rgb(190, 230, 60),
        PlaceKind::SafePosition => Color32::from_rgb(110, 200, 255),
        PlaceKind::Trigger => Color32::from_rgb(200, 140, 255),
        PlaceKind::Breadcrumb => Color32::from_rgb(160, 120, 220),
        PlaceKind::MapGroup => Color32::from_rgb(150, 150, 160),
    }
}

fn flag_color(bit: u32) -> Color32 {
    const C: [[u8; 3]; 12] = [[230, 60, 60], [60, 120, 230], [240, 240, 240], [240, 130, 30], [150, 110, 70], [120, 200, 60], [60, 200, 220], [230, 200, 60], [170, 230, 120], [40, 140, 60], [200, 120, 220], [250, 160, 190]];
    let c = C[bit as usize % C.len()];
    Color32::from_rgb(c[0], c[1], c[2])
}

/// The places of the mission shown on the map, gathered again when the mission changed.
fn places_now(app: &mut App) -> &[Place] {
    let r = rev(app);
    let mut kinds: Vec<PlaceKind> = app.mission.shown.iter().copied().collect();
    if let Some(PlaceRef::Xml { .. } | PlaceRef::Json { .. }) = &app.mission.selected {
        // The selected place stays even when its kind is hidden.
        kinds = PlaceKind::ALL.to_vec();
        kinds.retain(|k| app.mission.shown.contains(k) || *k != PlaceKind::MapGroup);
    }
    let c = &app.mission.cache;
    if !(c.valid && c.rev == r && c.kinds == kinds) {
        let list = mission(app).map(|m| places::collect(m, &kinds)).unwrap_or_default();
        let groups = mission(app).map(places::event_groups).unwrap_or_default();
        app.mission.cache = PlaceCache { rev: r, kinds, list, groups, valid: true };
    }
    &app.mission.cache.list
}

/// Brings the caches the drawing needs up to date; called before the views draw.
/// An object of an event group, drawn as its model at a spawn of the group.
pub struct Ghost {
    /// Index into `Ghosts::models`.
    pub model: u32,
    pub x: f32,
    pub z: f32,
    /// Height of the model origin in the editor, when the group gives one; else on the ground.
    pub y: Option<f32>,
    /// Direction, from the north towards the east.
    pub yaw: f32,
}

/// The objects of all event groups at their spawns, with the models they show.
#[derive(Default)]
pub struct Ghosts {
    key: (u64, usize, usize),
    pub models: Vec<String>,
    pub infos: Vec<dz_core::model_info::ModelInfo>,
    pub list: Vec<Ghost>,
    /// Classes no config or model was found for.
    pub unknown: BTreeSet<String>,
}

impl Ghosts {
    /// Changes whenever what is drawn changes; for the caches of the views.
    pub fn version(&self) -> u64 {
        self.key.0.wrapping_mul(31).wrapping_add(self.key.1 as u64).wrapping_mul(31).wrapping_add(self.key.2 as u64)
    }
}

/// The classes of the configs of the work drive, read in the background the first time.
pub fn class_index(app: &mut App) -> Option<std::sync::Arc<mission::classes::ClassIndex>> {
    if let Some(rx) = &app.mission.classes_job {
        if let Ok(idx) = rx.try_recv() {
            app.mission.classes = Some(std::sync::Arc::new(idx));
            app.mission.classes_job = None;
        }
    }
    if app.mission.classes.is_none() && app.mission.classes_job.is_none() {
        let root = app.db.root().to_path_buf();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(mission::classes::ClassIndex::scan_work_drive(&root));
        });
        app.mission.classes_job = Some(rx);
    }
    app.mission.classes.clone()
}

/// The model of a class: from the configs, else a model of the same name.
fn model_of_class(classes: Option<&mission::classes::ClassIndex>, assets: &dz_core::assets::AssetIndex, class: &str) -> Option<String> {
    classes.and_then(|c| c.model_of(class)).or_else(|| assets.resolve(&mission::classes::guess_stem(class)).map(str::to_string))
}

/// The class of a prop for an event group: one the configs give the model, else a guess.
pub fn class_of_model(app: &mut App, model: &str, de: bool) -> String {
    let idx = class_index(app);
    idx.and_then(|c| c.classes_for_model(model, de).into_iter().next()).unwrap_or_else(|| mission::gen::prop_class(model, de))
}

fn update_ghosts(app: &mut App) {
    let classes = class_index(app);
    let key = (rev(app), classes.as_ref().map_or(0, |c| c.len()), app.assets.assets.len());
    if app.mission.ghosts.key == key {
        return;
    }
    // Model numbers stay as they were: the renderer keeps the meshes it loaded by them.
    let old = std::mem::take(&mut app.mission.ghosts);
    let mut g = Ghosts { key, models: old.models, infos: old.infos, ..Default::default() };
    let Some(doc) = &app.doc else { return };
    let Some(m) = doc.world.mission.as_deref() else { return };
    let sea = doc.world.meta.sea_level;
    let groups = places::event_groups(m);
    let mut index: HashMap<String, u32> = g.models.iter().enumerate().map(|(i, m)| (m.clone(), i as u32)).collect();
    let mut by_class: HashMap<String, Option<u32>> = HashMap::new();
    for p in places::collect(m, &[PlaceKind::EventSpawn]) {
        let Some((_, _, children)) = p.sub.as_ref().and_then(|s| groups.get(&s.to_ascii_lowercase())) else { continue };
        for c in children {
            let model = *by_class.entry(c.kind.to_ascii_lowercase()).or_insert_with(|| {
                let path = model_of_class(classes.as_deref(), &app.assets, &c.kind)?;
                let n = index.len() as u32;
                Some(*index.entry(path.clone()).or_insert_with(|| {
                    g.models.push(path.clone());
                    g.infos.push(app.db.get(&path));
                    n
                }))
            });
            let Some(model) = model else {
                g.unknown.insert(c.kind.clone());
                continue;
            };
            let [x, z] = c.world(p.x, p.z);
            let y = c.y.and_then(|cy| p.y.map(|py| py + cy + sea));
            g.list.push(Ghost { model, x, z, y, yaw: c.a });
        }
    }
    app.mission.ghosts = g;
}

/// The objects of the event groups to draw in a view, while the Mission mode shows the spawns.
pub fn ghosts_of(s: &MissionState, mode: Mode) -> Option<&Ghosts> {
    (mode == Mode::Mission && s.shown.contains(&PlaceKind::EventSpawn) && s.show_models && !s.ghosts.list.is_empty()).then_some(&s.ghosts)
}

pub fn prepare(app: &mut App, ctx: &egui::Context) {
    if app.doc.as_ref().is_none_or(|d| d.world.mission.is_none()) {
        return;
    }
    if app.mode == Mode::Mission && app.mission.show_models {
        update_ghosts(app);
    }
    if app.mode == Mode::Mission || app.mission.open {
        let _ = places_now(app);
    }
    if app.mission.show_flags && app.mode == Mode::Mission {
        update_flags_texture(app, ctx);
    }
}

fn update_flags_texture(app: &mut App, ctx: &egui::Context) {
    let key = (rev(app), app.mission.flag_layer, app.mission.flag_bit);
    if app.mission.flags_key == Some(key) && app.mission.flags_tex.is_some() {
        return;
    }
    // While a stroke is painted the texture follows a few times a second.
    if app.mission.flag_stroke.is_some() && app.mission.flags_tex.is_some() && ctx.input(|i| i.time) % 0.2 > 0.05 {
        return;
    }
    let Some(a) = mission(app).and_then(|m| m.areaflags()) else { return };
    let step = a.width.div_ceil(1024).max(1);
    let (w, h) = (a.width.div_ceil(step), a.height.div_ceil(step));
    // Every flag of the layer shows in its colour; the one being painted stands out.
    let edited = 1u32 << app.mission.flag_bit.min(31);
    let palette: Vec<Color32> = (0..32).map(flag_color).collect();
    let mut img = egui::ColorImage::new([w as usize, h as usize], Color32::TRANSPARENT);
    let layer = app.mission.flag_layer;
    let mut colors: HashMap<u32, Color32> = HashMap::new();
    for py in 0..h {
        for px in 0..w {
            let mut v = 0u32;
            for dy in 0..step {
                for dx in 0..step {
                    v |= a.get(layer, px * step + dx, py * step + dy);
                }
            }
            if v == 0 {
                continue;
            }
            let c = *colors.entry(v).or_insert_with(|| {
                let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
                for (bit, col) in palette.iter().enumerate() {
                    if v >> bit & 1 != 0 {
                        r += col.r() as u32;
                        g += col.g() as u32;
                        b += col.b() as u32;
                        n += 1;
                    }
                }
                let (r, g, b) = ((r / n) as u8, (g / n) as u8, (b / n) as u8);
                if v & edited != 0 {
                    let e = palette[app.mission.flag_bit.min(31) as usize];
                    let mix = |x: u8, y: u8| ((x as u16 + y as u16 * 3) / 4) as u8;
                    Color32::from_rgba_unmultiplied(mix(r, e.r()), mix(g, e.g()), mix(b, e.b()), 200)
                } else {
                    Color32::from_rgba_unmultiplied(r, g, b, 95)
                }
            });
            // Image rows go from the north down.
            img.pixels[((h - 1 - py) * w + px) as usize] = c;
        }
    }
    let opts = egui::TextureOptions::NEAREST;
    match &mut app.mission.flags_tex {
        Some(tex) => tex.set(img, opts),
        None => app.mission.flags_tex = Some(ctx.load_texture("mission_flags", img, opts)),
    }
    app.mission.flags_key = Some(key);
}

fn ground(world: &dz_core::world::World, x: f32, z: f32) -> [f32; 3] {
    [x, world.height.sample(x, z) + 0.5, z]
}

/// A circle on the ground, as screen points.
fn ring(world: &dz_core::world::World, v: &crate::camera::View, x: f32, z: f32, r: f32) -> Vec<Pos2> {
    let n = if v.ortho { 40 } else { 28 };
    (0..=n).filter_map(|i| {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        let (px, pz) = (x + r * a.cos(), z + r * a.sin());
        v.project(ground(world, px, pz))
    }).collect()
}

/// Draws the places and the area flags over a view.
/// The objects of an event group around its spawn: a box turned the way the object looks,
/// filled when the object holds loot of the event, a line back to the spawn.
fn draw_group(painter: &egui::Painter, v: &crate::camera::View, world: &dz_core::world::World, x: f32, z: f32, children: &[places::GroupChild], selected: bool) {
    let Some(center) = v.project(ground(world, x, z)) else { return };
    for c in children {
        let [cx, cz] = c.world(x, z);
        let Some(s) = v.project(ground(world, cx, cz)) else { continue };
        let loot = c.deloot || c.loot.is_some_and(|(_, hi)| hi > 0);
        let col = if loot { Color32::from_rgb(255, 200, 110) } else { Color32::from_rgb(170, 170, 170) };
        painter.line_segment([center, s], Stroke::new(0.8_f32, Color32::from_rgba_unmultiplied(255, 200, 110, 70)));
        // About two by four metres, the size of a car, so the direction shows.
        let (sn, cs) = c.a.to_radians().sin_cos();
        let corner = |dx: f32, dz: f32| v.project(ground(world, cx + dx * cs + dz * sn, cz - dx * sn + dz * cs));
        let pts: Vec<Pos2> = [(-1.0, -2.0), (1.0, -2.0), (1.0, 2.0), (-1.0, 2.0)].iter().filter_map(|(dx, dz)| corner(*dx, *dz)).collect();
        if pts.len() == 4 && pts[0].distance(pts[2]) > 3.0 {
            if loot {
                painter.add(egui::Shape::convex_polygon(pts.clone(), Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), 110), Stroke::new(1.0_f32, col)));
            } else {
                painter.add(egui::Shape::closed_line(pts, Stroke::new(1.0_f32, col)));
            }
            if let Some(f) = corner(0.0, 3.0) {
                painter.line_segment([s, f], Stroke::new(1.2_f32, col));
            }
        } else {
            painter.circle_filled(s, 2.0, col);
        }
        if selected {
            painter.text(s + vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, &c.kind, egui::FontId::proportional(10.0), Color32::from_gray(225));
        }
    }
}

pub fn draw_map(app: &App, painter: &egui::Painter, v: &crate::camera::View, resp: &egui::Response) {
    let Some(doc) = &app.doc else { return };
    let world = &doc.world;
    if world.mission.is_none() {
        return;
    }
    let active = app.mode == Mode::Mission;
    if app.mission.show_flags && active && v.ortho {
        if let (Some(tex), Some(a)) = (&app.mission.flags_tex, world.mission.as_ref().and_then(|m| m.areaflags())) {
            if let (Some(p0), Some(p1)) = (v.project([0.0, 0.0, a.world_height as f32]), v.project([a.world_width as f32, 0.0, 0.0])) {
                painter.image(tex.id(), egui::Rect::from_two_pos(p0, p1), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
            }
        }
    }
    if !active && !app.mission.open {
        return;
    }
    let c = &app.mission.cache;
    if !c.valid {
        return;
    }
    let clip = v.rect.expand(40.0);
    let detail = !v.ortho || v.mpp < 4.0;
    for p in &c.list {
        if !app.mission.shown.contains(&p.kind) && app.mission.selected.as_ref() != Some(&p.at) {
            continue;
        }
        let Some(s) = v.project(ground(world, p.x, p.z)) else { continue };
        let selected = app.mission.selected.as_ref() == Some(&p.at);
        let r_px = if p.radius > 0.0 { v.project(ground(world, p.x + p.radius, p.z)).map(|e| e.distance(s)).unwrap_or(0.0) } else { 0.0 };
        if !clip.expand(r_px).contains(s) {
            continue;
        }
        let col = if selected { Color32::WHITE } else { kind_color(p.kind) };
        if p.kind == PlaceKind::MapGroup && !selected {
            if detail { painter.circle_filled(s, 1.8, col); }
            continue;
        }
        if p.radius > 0.0 && r_px > 3.0 {
            let pts = ring(world, v, p.x, p.z, p.radius);
            painter.add(egui::Shape::line(pts, Stroke::new(if selected { 2.0_f32 } else { 1.2 }, col)));
        }
        if let Some(size) = p.size {
            let (hx, hz) = (size[0] * 0.5, size[2] * 0.5);
            let a = p.angle.unwrap_or(0.0).to_radians();
            let (sn, cs) = a.sin_cos();
            let corner = |dx: f32, dz: f32| v.project(ground(world, p.x + dx * cs + dz * sn, p.z - dx * sn + dz * cs));
            let pts: Vec<Pos2> = [(-hx, -hz), (hx, -hz), (hx, hz), (-hx, hz), (-hx, -hz)].iter().filter_map(|(dx, dz)| corner(*dx, *dz)).collect();
            painter.add(egui::Shape::line(pts, Stroke::new(1.4_f32, col)));
        }
        // The objects the group of a static event spawn places around it.
        if let Some((_, _, children)) = p.sub.as_ref().and_then(|g| c.groups.get(&g.to_ascii_lowercase())) {
            if (detail || selected) && (v.ortho || !app.mission.show_models) {
                draw_group(painter, v, world, p.x, p.z, children, selected);
            }
        }
        painter.circle_filled(s, if selected { 5.5 } else { 3.5 }, col);
        painter.circle_stroke(s, if selected { 5.5 } else { 3.5 }, Stroke::new(1.0_f32, Color32::from_black_alpha(160)));
        if let Some(a) = p.angle.filter(|_| p.kind == PlaceKind::EventSpawn && detail) {
            let (sn, cs) = a.to_radians().sin_cos();
            if let Some(e) = v.project(ground(world, p.x + sn * 8.0, p.z + cs * 8.0)) {
                painter.line_segment([s, e], Stroke::new(1.2_f32, col));
            }
        }
        if selected || (detail && (v.ortho && v.mpp < 1.5 || !v.ortho && s.distance(v.rect.center()) < 250.0)) {
            painter.text(s + vec2(7.0, -2.0), egui::Align2::LEFT_CENTER, &p.label, egui::FontId::proportional(11.0), Color32::WHITE);
        }
    }
    // The flag brush.
    if active && app.mission.tool == MapTool::Flags {
        if let Some(m) = resp.hover_pos() {
            let (o, d) = v.ray(m);
            if let Some(h) = crate::viewport::terrain_hit(&world.height, o, d, v.ortho) {
                let pts = ring(world, v, h[0], h[2], app.mission.flag_radius);
                painter.add(egui::Shape::line(pts, Stroke::new(1.5_f32, flag_color(app.mission.flag_bit))));
            }
        }
    }
}

/// Mouse and keys on the map in the Mission mode.
pub fn map_input(app: &mut App, ctx: &egui::Context, resp: &egui::Response, v: &crate::camera::View, hit: Option<[f32; 3]>, mouse: Pos2) {
    if mission(app).is_none() {
        return;
    }
    let (pressed, down, del, esc, shift) = ctx.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.key_pressed(egui::Key::Delete), i.key_pressed(egui::Key::Escape), i.modifiers.shift));
    let typing = ctx.wants_keyboard_input();
    if esc && !typing {
        app.mission.selected = None;
    }
    if del && !typing && app.mission.selected.is_some() {
        delete_selected(app);
        return;
    }
    let world_hit = hit.map(|h| [h[0], h[2]]);
    match app.mission.tool {
        MapTool::Flags => {
            let on_view = resp.is_pointer_button_down_on() || app.mission.flag_stroke.is_some();
            if down && on_view {
                if let Some([x, z]) = world_hit {
                    paint_flags(app, x, z, shift || app.mission.flag_erase);
                }
            } else if !down && app.mission.flag_stroke.is_some() {
                finish_flag_stroke(app);
            }
        }
        MapTool::Select => select_input(app, resp, v, world_hit, mouse, pressed, down),
        tool => {
            if resp.clicked() {
                if let Some([x, z]) = world_hit {
                    add_place(app, tool, x, z);
                }
            }
        }
    }
}

fn select_input(app: &mut App, resp: &egui::Response, v: &crate::camera::View, hit: Option<[f32; 2]>, mouse: Pos2, pressed: bool, down: bool) {
    if let Some((at, first)) = app.mission.drag.take() {
        if down {
            if let Some([x, z]) = hit {
                let e = mission_mut(app).and_then(|m| places::move_to(m, &at, x, z, None));
                // The first edit holds the state before the drag.
                app.mission.drag = Some((at, first.or(e)));
            } else {
                app.mission.drag = Some((at, first));
            }
            return;
        }
        push(app, &t("Move"), first.into_iter().collect());
        return;
    }
    if pressed && resp.hovered() {
        let world = &app.doc.as_ref().unwrap().world;
        let shown = app.mission.shown.clone();
        let picked = app.mission.cache.list.iter().filter(|p| shown.contains(&p.kind)).filter_map(|p| {
            let s = v.project(ground(world, p.x, p.z))?;
            let d = s.distance(mouse);
            (d < 10.0).then_some((p.at.clone(), d))
        }).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(at, _)| at);
        match picked {
            Some(at) => {
                app.mission.selected = Some(at.clone());
                app.mission.drag = Some((at, None));
            }
            None => app.mission.selected = None,
        }
    }
}

fn add_place(app: &mut App, tool: MapTool, x: f32, z: f32) {
    let s = &app.mission;
    let (event, section, group) = (s.event.trim().to_string(), s.spawn_section.clone(), s.spawn_group.trim().to_string());
    let event_group = s.event_group.trim().to_string();
    let (zfile, zt, zname, zr, zs, zd) = (s.zone_file.clone(), s.zone_territory, s.zone_name.clone(), s.zone_radius, s.zone_static, s.zone_dynamic);
    let (aname, ar, tsize) = (s.area_name.clone(), s.area_radius, s.trigger_size);
    let y = app.doc.as_ref().map(|d| d.world.height.sample(x, z) - d.world.meta.sea_level).unwrap_or(0.0);
    let Some(m) = mission_mut(app) else { return };
    let (label, edits) = match tool {
        MapTool::EventSpawn => {
            if event.is_empty() {
                app.warn(t("Choose the event first."));
                return;
            }
            if event_group.is_empty() { (t("Add event spawn"), places::add_event_spawn(m, &event, x, z, 0.0)) } else { (t("Add event spawn"), places::add_group_spawn(m, &event, &event_group, x, z, y)) }
        }
        MapTool::PlayerSpawn => (t("Add player spawn"), places::add_player_spawn(m, &section, if group.is_empty() { "Spawn" } else { &group }, x, z)),
        MapTool::Zone => (t("Add zone"), places::add_zone(m, &zfile, zt, places::zone(&zname, x, z, zr, zs[0], zs[1], zd[0], zd[1]))),
        MapTool::Area => (t("Add effect area"), vec![places::add_json(m, "cfgeffectarea.json", "Areas", places::effect_area(&aname, x, z, ar))]),
        MapTool::SafePosition => (t("Add safe position"), vec![places::add_json(m, "cfgeffectarea.json", "SafePositions", serde_json::json!([ (x * 10.0).round() / 10.0, (z * 10.0).round() / 10.0 ]))]),
        MapTool::Trigger => (t("Add trigger"), vec![places::add_json(m, "cfgundergroundtriggers.json", "Triggers", places::trigger(x, y + tsize[1] * 0.5, z, tsize))]),
        _ => return,
    };
    push(app, &label, edits);
    // Select what was added: the nearest place of that kind.
    let kind = match tool {
        MapTool::EventSpawn => PlaceKind::EventSpawn,
        MapTool::PlayerSpawn => PlaceKind::PlayerSpawn,
        MapTool::Zone => PlaceKind::Territory,
        MapTool::Area => PlaceKind::EffectArea,
        MapTool::SafePosition => PlaceKind::SafePosition,
        _ => PlaceKind::Trigger,
    };
    app.mission.shown.insert(kind);
    let added = places_now(app).iter().filter(|p| p.kind == kind).min_by(|a, b| (a.x - x).hypot(a.z - z).total_cmp(&(b.x - x).hypot(b.z - z))).map(|p| p.at.clone());
    app.mission.selected = added;
}

fn make_areaflags(app: &mut App) {
    let Some(doc) = &app.doc else { return };
    let meta = doc.world.meta.clone();
    let Some(m) = mission_mut(app) else { return };
    let (u, v) = (m.limits("usageflags").len(), m.limits("valueflags").len());
    let e = mission::file_edit(m, "areaflags.map", |m| m.put("areaflags.map", mission::FileData::AreaFlags(mission::templates::blank_areaflags(&meta, u, v))));
    push(app, &t("Area flags"), vec![e]);
}

fn paint_flags(app: &mut App, x: f32, z: f32, erase: bool) {
    let (layer, bit, radius) = (app.mission.flag_layer, app.mission.flag_bit, app.mission.flag_radius);
    let stroke = app.mission.flag_stroke.get_or_insert_with(|| FlagStroke { layer, blocks: HashMap::new() });
    let Some(a) = app.doc.as_mut().and_then(|d| d.world.mission.as_deref_mut()).and_then(|m| m.areaflags_mut()) else { return };
    let (cw, ch) = a.cell_size();
    let x0 = ((x - radius) / cw).floor().max(0.0) as u32;
    let y0 = ((z - radius) / ch).floor().max(0.0) as u32;
    let x1 = (((x + radius) / cw).ceil().max(0.0) as u32).min(a.width);
    let y1 = (((z + radius) / ch).ceil().max(0.0) as u32).min(a.height);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    // Keep the blocks as they were before the stroke first touches them.
    for by in y0 / FLAG_BLOCK..=(y1 - 1) / FLAG_BLOCK {
        for bx in x0 / FLAG_BLOCK..=(x1 - 1) / FLAG_BLOCK {
            stroke.blocks.entry((bx, by)).or_insert_with(|| {
                let (bx0, by0) = (bx * FLAG_BLOCK, by * FLAG_BLOCK);
                a.copy_rect(layer, bx0, by0, (bx0 + FLAG_BLOCK).min(a.width), (by0 + FLAG_BLOCK).min(a.height))
            });
        }
    }
    let r2 = radius * radius;
    let n = a.paint(layer, bit, !erase, x0, y0, x1, y1, |cx, cz| (cx - x).powi(2) + (cz - z).powi(2) <= r2);
    if n > 0 {
        if let Some(m) = mission_mut(app) { m.touch("areaflags.map"); }
    }
}

fn finish_flag_stroke(app: &mut App) {
    let Some(stroke) = app.mission.flag_stroke.take() else { return };
    let Some(a) = mission(app).and_then(|m| m.areaflags()) else { return };
    let mut edits = Vec::new();
    for ((bx, by), values) in stroke.blocks {
        let (x0, y0) = (bx * FLAG_BLOCK, by * FLAG_BLOCK);
        let (x1, y1) = ((x0 + FLAG_BLOCK).min(a.width), (y0 + FLAG_BLOCK).min(a.height));
        if a.copy_rect(stroke.layer, x0, y0, x1, y1) != values {
            edits.push(MissionEdit::Flags { layer: stroke.layer, rect: [x0, y0, x1, y1], values });
        }
    }
    app.mission.flags_key = None;
    push(app, &t("Paint area flags"), edits);
}

/// Shows the brush of the flag tool in the 3D view.
pub fn brush(app: &App, h: [f32; 3]) -> Option<crate::render::BrushView> {
    (app.mode == Mode::Mission && app.mission.tool == MapTool::Flags).then(|| {
        let c = flag_color(app.mission.flag_bit);
        crate::render::BrushView { x: h[0], z: h[2], radius: app.mission.flag_radius, falloff: 0.0, color: [c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0], shape: 0 }
    })
}

// ---------------------------------------------------------------- generic editors

/// The attributes (and the text) of an element as editable rows. Returns whether it changed.
pub fn attrs_ui(ui: &mut egui::Ui, el: &mut Element, id: &str) -> bool {
    let before = el.clone();
    let mut remove = None;
    egui::Grid::new(("attrs", id)).num_columns(3).spacing(vec2(6.0, 2.0)).show(ui, |ui| {
        for (i, (k, v)) in el.attrs.iter_mut().enumerate() {
            ui.label(RichText::new(k.as_str()).monospace().color(theme::TEXT_DIM));
            ui.add(egui::TextEdit::singleline(v).desired_width(180.0));
            if ui.small_button(icons::X).on_hover_text(t("Remove")).clicked() { remove = Some(i); }
            ui.end_row();
        }
    });
    if let Some(i) = remove { el.attrs.remove(i); }
    let leaf = !el.children.iter().any(|n| matches!(n, Node::Element(_)));
    if leaf && (el.children.iter().any(|n| matches!(n, Node::Text(_))) || el.attrs.is_empty()) {
        let mut text = el.text();
        if ui.add(egui::TextEdit::singleline(&mut text).hint_text(t("Text")).desired_width(240.0)).changed() { el.set_text(text); }
    }
    let key = egui::Id::new(("new_attr", id));
    let mut name: String = ui.data_mut(|d| d.get_temp(key).unwrap_or_default());
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut name).hint_text(t("New attribute")).desired_width(110.0));
        if ui.add_enabled(!name.trim().is_empty(), egui::Button::new(icons::PLUS).small()).clicked() {
            el.set_attr(name.trim(), "");
            name.clear();
        }
    });
    ui.data_mut(|d| d.insert_temp(key, name));
    *el != before
}

fn element_title(e: &Element) -> String {
    let mut s = format!("<{}", e.name);
    for (k, v) in e.attrs.iter().take(3) {
        s += &format!(" {k}=\"{v}\"");
    }
    if e.attrs.len() > 3 { s += " ..."; }
    let leaf = !e.children.iter().any(|n| matches!(n, Node::Element(_)));
    if leaf {
        let text = e.text();
        if !text.trim().is_empty() { s += &format!(">{}", text.trim()); }
    }
    s + ">"
}

/// An element with all below it as a tree: attributes, text and children, each to change,
/// copy, remove or add to. Returns whether it changed.
pub fn tree_ui(ui: &mut egui::Ui, el: &mut Element, id: &str) -> bool {
    let before = el.clone();
    attrs_ui(ui, el, id);
    let mut action: Option<(usize, u8)> = None;
    for (i, n) in el.children.iter_mut().enumerate() {
        match n {
            Node::Element(c) => {
                let cid = format!("{id}/{i}");
                ui.horizontal(|ui| {
                    if ui.small_button(icons::COPY).on_hover_text(t("Duplicate")).clicked() { action = Some((i, 1)); }
                    if ui.small_button(icons::TRASH).on_hover_text(t("Remove")).clicked() { action = Some((i, 2)); }
                    egui::CollapsingHeader::new(RichText::new(element_title(c)).monospace().size(11.5)).id_salt(&cid).show(ui, |ui| { tree_ui(ui, c, &cid); });
                });
            }
            Node::Comment(c) => {
                ui.label(RichText::new(format!("<!--{}-->", c.trim().lines().next().unwrap_or(""))).small().color(Color32::from_rgb(110, 150, 110)));
            }
            _ => {}
        }
    }
    match action {
        Some((i, 1)) => { let c = el.children[i].clone(); el.children.insert(i + 1, c); }
        Some((i, 2)) => { el.children.remove(i); }
        _ => {}
    }
    let key = egui::Id::new(("new_child", id));
    let mut name: String = ui.data_mut(|d| d.get_temp(key).unwrap_or_default());
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut name).hint_text(t("New element")).desired_width(110.0));
        if ui.add_enabled(!name.trim().is_empty() && !name.contains(' '), egui::Button::new(icons::PLUS).small()).clicked() {
            el.push(Element::new(name.trim()));
            name.clear();
        }
    });
    ui.data_mut(|d| d.insert_temp(key, name));
    *el != before
}

/// A JSON value to edit: switches, numbers, text, lists and objects. Returns whether it changed.
pub fn json_ui(ui: &mut egui::Ui, v: &mut serde_json::Value, id: &str) -> bool {
    use serde_json::Value;
    let mut changed = false;
    match v {
        Value::Object(o) => {
            egui::Grid::new(("json", id)).num_columns(2).spacing(vec2(8.0, 3.0)).striped(true).show(ui, |ui| {
                for (k, x) in o.iter_mut() {
                    ui.label(RichText::new(k.as_str()).color(theme::TEXT_DIM));
                    let cid = format!("{id}/{k}");
                    match x {
                        Value::Object(_) | Value::Array(_) => {
                            ui.vertical(|ui| {
                                egui::CollapsingHeader::new(summary(x)).id_salt(&cid).show(ui, |ui| { changed |= json_ui(ui, x, &cid); });
                            });
                        }
                        _ => changed |= json_leaf(ui, x),
                    }
                    ui.end_row();
                }
            });
        }
        Value::Array(a) => {
            let mut remove = None;
            let simple = a.iter().all(|x| !x.is_object() && !x.is_array());
            if simple {
                ui.horizontal_wrapped(|ui| {
                    for (i, x) in a.iter_mut().enumerate() {
                        changed |= json_leaf(ui, x);
                        if ui.small_button(icons::X).clicked() { remove = Some(i); }
                    }
                    if ui.small_button(icons::PLUS).clicked() {
                        let next = a.last().cloned().unwrap_or(Value::from(0));
                        a.push(next);
                        changed = true;
                    }
                });
            } else {
                for (i, x) in a.iter_mut().enumerate() {
                    let cid = format!("{id}/{i}");
                    ui.horizontal(|ui| {
                        if ui.small_button(icons::TRASH).clicked() { remove = Some(i); }
                        egui::CollapsingHeader::new(format!("#{} {}", i + 1, summary(x))).id_salt(&cid).show(ui, |ui| { changed |= json_ui(ui, x, &cid); });
                    });
                }
                if ui.small_button(icons::with(icons::PLUS, &t("Add"))).clicked() {
                    let next = a.last().cloned().unwrap_or(Value::Object(Default::default()));
                    a.push(next);
                    changed = true;
                }
            }
            if let Some(i) = remove { a.remove(i); changed = true; }
        }
        _ => changed |= json_leaf(ui, v),
    }
    changed
}

fn summary(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Object(o) => {
            let name = ["AreaName", "name", "Name", "Type"].iter().find_map(|k| o.get(*k).and_then(|x| x.as_str())).unwrap_or("");
            format!("{{{}}} {name}", o.len())
        }
        serde_json::Value::Array(a) => format!("[{}]", a.len()),
        other => other.to_string(),
    }
}

fn json_leaf(ui: &mut egui::Ui, x: &mut serde_json::Value) -> bool {
    use serde_json::Value;
    match x {
        Value::Bool(b) => ui.checkbox(b, "").changed(),
        Value::Number(n) => {
            let float = n.is_f64();
            let mut f = n.as_f64().unwrap_or(0.0);
            let r = ui.add(egui::DragValue::new(&mut f).speed(if float { 0.01 } else { 1.0 }).max_decimals(6));
            if r.changed() {
                mission::json::set_number(x, if float { f } else { f.round() });
                true
            } else {
                false
            }
        }
        Value::String(s) => ui.add(egui::TextEdit::singleline(s).desired_width(220.0)).changed(),
        Value::Null => {
            ui.weak("null");
            false
        }
        _ => false,
    }
}

/// A number held in an attribute, as a drag field that keeps the text when unchanged.
fn num_attr(ui: &mut egui::Ui, el: &mut Element, attr: &str, speed: f64) -> bool {
    let Some(mut v) = el.attr_f64(attr) else {
        ui.weak("-");
        return false;
    };
    let decimals = el.attr(attr).and_then(|s| s.split('.').nth(1)).map(|d| d.len()).unwrap_or(0).max(if speed < 1.0 { 2 } else { 0 });
    if ui.add(egui::DragValue::new(&mut v).speed(speed).max_decimals(decimals.max(1))).changed() {
        let text = if decimals == 0 { format!("{}", v.round() as i64) } else { format!("{v:.decimals$}") };
        el.set_attr(attr, text);
        true
    } else {
        false
    }
}

/// A number held as the text of a child element.
fn num_text(ui: &mut egui::Ui, el: &mut Element, child: &str, speed: f64) -> bool {
    let Some(c) = el.child_mut(child) else {
        ui.weak("-");
        return false;
    };
    let text = c.text();
    let Some(mut v) = mission::xml::parse_num(&text) else {
        let mut s = text.clone();
        if ui.add(egui::TextEdit::singleline(&mut s).desired_width(80.0)).changed() { c.set_text(s); return true; }
        return false;
    };
    let decimals = text.trim().split('.').nth(1).map(|d| d.len()).unwrap_or(0);
    if ui.add(egui::DragValue::new(&mut v).speed(speed).max_decimals(decimals.max(1))).changed() {
        c.set_text(if decimals == 0 { format!("{}", v.round() as i64) } else { format!("{v:.decimals$}") });
        true
    } else {
        false
    }
}

/// Indexes of the element children of `e` that `keep` accepts, with the elements.
fn element_indexes(e: &Element, keep: impl Fn(&Element) -> bool) -> Vec<usize> {
    e.children.iter().enumerate().filter(|(_, n)| matches!(n, Node::Element(c) if keep(c))).map(|(i, _)| i).collect()
}

fn el_name(e: &Element) -> String {
    e.attr("name").or(e.attr("type")).or(e.attr("path")).map(str::to_string).unwrap_or_else(|| element_title(e))
}

/// The children of the root of a file as a list to search and pick from, with the picked one
/// below as a tree to edit. `names` limits the list to elements of those names.
fn element_list(app: &mut App, ui: &mut egui::Ui, file: &str, names: &[&str], height: f32) {
    let Some(doc) = app.doc.as_ref().and_then(|d| d.world.mission.as_deref()).and_then(|m| m.xml(file)) else {
        ui.weak(tf("The mission has no {0}", &[file]));
        return;
    };
    let filter = app.mission.list_filter.to_lowercase();
    let all = element_indexes(&doc.root, |e| names.is_empty() || names.contains(&e.name.as_str()));
    let shown: Vec<(usize, String)> = all.iter().filter_map(|&i| {
        let Node::Element(e) = &doc.root.children[i] else { return None };
        let n = el_name(e);
        (filter.is_empty() || n.to_lowercase().contains(&filter)).then_some((i, n))
    }).collect();
    let mut sel = app.mission.list_sel.get(file).copied().filter(|i| all.contains(i));
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut app.mission.list_filter).hint_text(format!("{} {}", icons::MAGNIFYING_GLASS, t("Search..."))).desired_width(200.0));
        ui.weak(tf("{0} of {1}", &[&shown.len().to_string(), &all.len().to_string()]));
    });
    let mut add: Option<Element> = None;
    let mut remove = None;
    ui.horizontal(|ui| {
        if let Some(i) = sel {
            if ui.button(icons::with(icons::COPY, &t("Duplicate"))).clicked() {
                if let Node::Element(e) = &doc.root.children[i] { add = Some(e.clone()); }
            }
            if ui.button(icons::with(icons::TRASH, &t("Delete"))).clicked() { remove = Some(i); }
        }
        if let Some(n) = names.first() {
            if ui.button(icons::with(icons::PLUS, &t("New"))).clicked() { add = Some(Element::new(*n).with_attr("name", "New")); }
        }
    });
    egui::ScrollArea::vertical().id_salt(("list", file)).max_height(height).auto_shrink([false, true]).show_rows(ui, 18.0, shown.len(), |ui, range| {
        for (i, n) in &shown[range] {
            if ui.selectable_label(sel == Some(*i), n).clicked() { sel = Some(*i); }
        }
    });
    if let Some(s) = sel { app.mission.list_sel.insert(file.to_string(), s); }
    ui.separator();
    if let Some(i) = sel {
        if let Some(Node::Element(e)) = mission(app).and_then(|m| m.xml(file)).and_then(|d| d.root.children.get(i)) {
            let mut copy = e.clone();
            let original = e.clone();
            egui::ScrollArea::vertical().id_salt(("tree", file)).auto_shrink([false, false]).show(ui, |ui| {
                ui.label(RichText::new(element_title(&copy)).monospace().strong());
                tree_ui(ui, &mut copy, &format!("{file}#{i}"));
            });
            if copy != original {
                edit_el(app, &t("Edit"), file, &[i], |e| *e = copy);
            }
        }
    }
    if let Some(e) = add {
        let at = sel.map(|i| i + 1);
        let edits: Vec<_> = mission_mut(app).and_then(|m| mission::insert_child(m, file, &[], at, e)).into_iter().collect();
        push(app, &t("Add"), edits);
        if let Some(i) = at { app.mission.list_sel.insert(file.to_string(), i); }
    }
    if let Some(i) = remove {
        let edits: Vec<_> = mission_mut(app).and_then(|m| mission::remove_child(m, file, &[], i)).into_iter().collect();
        push(app, &t("Delete"), edits);
    }
}

// ---------------------------------------------------------------- the window

pub fn window(app: &mut App, ctx: &egui::Context) {
    if !app.mission.open || app.doc.is_none() {
        return;
    }
    settle(app, ctx);
    if app.mission.own_window {
        // A window of the system, which may stand anywhere, on another screen too.
        let id = egui::ViewportId::from_hash_of("dzwe_mission");
        let builder = egui::ViewportBuilder::default().with_title(format!("DZ World Editor – {}", t("Mission editor"))).with_inner_size([1120.0, 760.0]).with_min_inner_size([520.0, 320.0]);
        ctx.show_viewport_immediate(id, builder, |ctx, _class| {
            if ctx.input(|i| i.viewport().close_requested()) {
                app.mission.open = false;
                return;
            }
            settle(app, ctx);
            egui::CentralPanel::default().show(ctx, |ui| window_body(app, ui));
        });
        return;
    }
    let mut open = true;
    egui::Window::new(icons::with(icons::FLAG_BANNER, &t("Mission editor"))).id(egui::Id::new("mission_window")).open(&mut open).resizable(true).default_size([1040.0, 680.0]).show(ctx, |ui| window_body(app, ui));
    if !open {
        app.mission.open = false;
    }
}

fn window_body(app: &mut App, ui: &mut egui::Ui) {
    {
        ui.horizontal_wrapped(|ui| {
            let (icon, tip) = if app.mission.own_window { (icons::ARROW_SQUARE_IN, t("Back into the main window")) } else { (icons::ARROW_SQUARE_OUT, t("A window of its own, which can go outside the program and onto another screen")) };
            if ui.button(icon).on_hover_text(tip).clicked() { app.mission.own_window = !app.mission.own_window; }
            for tab in Tab::ALL {
                if tool_button(ui, app.mission.tab == tab, &icons::with(tab.icon(), &tab.title())).clicked() {
                    app.mission.tab = tab;
                    app.mission.list_filter.clear();
                }
            }
        });
        ui.separator();
        if mission(app).is_none() && app.mission.tab != Tab::Overview {
            app.mission.tab = Tab::Overview;
        }
        match app.mission.tab {
            Tab::Overview => overview_tab(app, ui),
            Tab::Types => types_tab(app, ui),
            Tab::Events => events_tab(app, ui),
            Tab::Spawnable => spawnable_tab(app, ui),
            Tab::Players => players_tab(app, ui),
            Tab::Territories => territories_tab(app, ui),
            Tab::Areas => areas_tab(app, ui),
            Tab::Weather => weather_tab(app, ui),
            Tab::Economy => economy_tab(app, ui),
            Tab::Gameplay => json_file_ui(app, ui, "cfggameplay.json"),
            Tab::Messages => messages_tab(app, ui),
            Tab::MapGroups => map_groups_tab(app, ui),
            Tab::Flags => flags_tab(app, ui),
            Tab::Init => init_tab(app, ui),
            Tab::Files => files_tab(app, ui),
            Tab::Check => check_tab(app, ui),
        }
    }
}

/// What each file of a mission is for.
pub const FILES: [(&str, &str); 27] = [
    ("init.c", "Script the server runs: starts the economy, sets the date, gives new characters their items."),
    ("cfgeconomycore.xml", "Root classes of the economy, its defaults and logging, and the extra files it reads (ce folders)."),
    ("cfgenvironment.xml", "Animal and infected territories: which files in env each kind of animal uses."),
    ("cfgeventgroups.xml", "Groups of objects spawned together by an event, as a train with its wagons."),
    ("cfgeventspawns.xml", "Where on the map each event may spawn, with optional zones and groups."),
    ("cfgplayerspawnpoints.xml", "Where new players spawn (fresh), after changing servers (hop) and after travel."),
    ("cfgrandompresets.xml", "Named sets of cargo and attachments, with chances, used by spawnable types."),
    ("cfgspawnabletypes.xml", "Cargo, attachments and damage items spawn with; hoarder containers."),
    ("cfgweather.xml", "Weather: overcast, fog, rain, wind, snowfall and storms, with their limits."),
    ("cfgignorelist.xml", "Types the economy leaves alone at cleanup."),
    ("cfglimitsdefinition.xml", "Names of the categories, tags, usage and value flags the other files may use."),
    ("cfglimitsdefinitionuser.xml", "Named groups of usage and value flags for types.xml."),
    ("cfggameplay.json", "Gameplay settings: stamina, base building, map, UI and more (needs enableCfgGameplayFile)."),
    ("cfgeffectarea.json", "Contaminated and other effect areas, and the safe positions players are moved to."),
    ("cfgundergroundtriggers.json", "Trigger boxes that darken the view underground, with breadcrumbs."),
    ("mapgroupproto.xml", "Loot points of each building class: containers, categories, tags and usage."),
    ("mapgrouppos.xml", "Every building of the map that can hold loot, with its position and angle."),
    ("mapclusterproto.xml", "Loot of clusters such as fruit trees and stones, and the models that are clusters."),
    ("mapgroupcluster*.xml", "Every cluster object of the map, made by the economy tools."),
    ("mapgroupdirt.xml", "Positions of dirt piles; usually empty."),
    ("db/types.xml", "Every item the economy spawns: nominal, min, lifetime, restock, flags, category, usage, value and tags."),
    ("db/events.xml", "Dynamic events: vehicles, wrecks, animals, infected and their children."),
    ("db/globals.xml", "Global numbers of the economy: counts, cleanup times, login and logout times."),
    ("db/economy.xml", "Which parts of the economy start, load, respawn and save."),
    ("db/messages.xml", "Messages shown to players: welcome, repeated and shutdown countdowns."),
    ("env/*.xml", "Territory zones of animals and infected."),
    ("areaflags.map", "Usage and value flags painted over the map; the economy uses them for loot without a building."),
];

fn file_help(rel: &str) -> Option<&'static str> {
    let l = rel.to_ascii_lowercase();
    if l.starts_with("storage_") {
        return Some("Persistence of the server: what is on the map now. Deleting it starts the economy afresh.");
    }
    FILES.iter().find(|(p, _)| {
        if let Some(prefix) = p.strip_suffix("*.xml") { l.starts_with(prefix) && l.ends_with(".xml") } else if p.contains('*') { let pre = p.split('*').next().unwrap(); l.starts_with(pre) } else { l == *p }
    }).map(|(_, d)| *d)
}

fn overview_tab(app: &mut App, ui: &mut egui::Ui) {
    if mission(app).is_none() {
        ui.label(t("The project has no server mission yet."));
        ui.horizontal(|ui| {
            if ui.button(icons::with(icons::FILE_PLUS, &t("New mission for this map"))).clicked() { new_mission(app); }
            if ui.button(icons::with(icons::FOLDER_OPEN, &t("Import a mission..."))).clicked() { import(app, false); }
            if ui.button(icons::with(icons::ARROWS_LEFT_RIGHT, &t("Adapt a mission of another map..."))).clicked() { import(app, true); }
        });
        return;
    }
    let m = mission(app).unwrap();
    let name = m.name.clone();
    let types = mission::ce::all_types(m).len();
    let events = mission::ce::event_names(m).len();
    let pl = places::collect(m, &[PlaceKind::EventSpawn, PlaceKind::PlayerSpawn, PlaceKind::Territory, PlaceKind::EffectArea, PlaceKind::Trigger]);
    let count = |k: PlaceKind| pl.iter().filter(|p| p.kind == k).count();
    let groups = m.xml("mapgrouppos.xml").map(|d| d.root.children_named("group").count()).unwrap_or(0);
    let (st_n, st_b) = m.storage_size();
    let mut files: Vec<(String, bool, &'static str)> = m.files.iter().filter(|(k, _)| !mission::is_storage(k)).map(|(k, f)| (k.clone(), f.dirty, file_help(k).unwrap_or(""))).collect();
    files.sort();
    let mut rename = name.clone();
    row(ui, &t("Folder name"), |ui| { ui.text_edit_singleline(&mut rename); });
    if rename != name && !rename.trim().is_empty() {
        if let Some(m) = mission_mut(app) { m.name = rename.trim().to_string(); m.rev += 1; }
    }
    ui.weak(t("The server finds the mission by this name in mpmissions: dayzOffline.<world>."));
    egui::Grid::new("ms_counts").num_columns(2).show(ui, |ui| {
        for (k, v) in [(t("Types"), types), (t("Events"), events), (t("Event spawns"), count(PlaceKind::EventSpawn)), (t("Player spawns"), count(PlaceKind::PlayerSpawn)), (t("Territory zones"), count(PlaceKind::Territory)), (t("Effect areas"), count(PlaceKind::EffectArea)), (t("Underground triggers"), count(PlaceKind::Trigger)), (t("Map groups"), groups)] {
            ui.label(k);
            ui.label(RichText::new(v.to_string()).strong());
            ui.end_row();
        }
        ui.label(t("Persistence"));
        ui.label(tf("{0} files, {1} MB", &[&st_n.to_string(), &format!("{:.1}", st_b as f64 / 1048576.0)]));
        ui.end_row();
    });
    ui.horizontal_wrapped(|ui| {
        if ui.button(icons::with(icons::EXPORT, &t("Export..."))).clicked() { export(app); }
        if ui.button(icons::with(icons::FOLDER_OPEN, &t("Import a mission..."))).clicked() { import(app, false); }
        if ui.button(icons::with(icons::ARROWS_LEFT_RIGHT, &t("Adapt a mission of another map..."))).clicked() { import(app, true); }
        if st_n > 0 && ui.button(icons::with(icons::ERASER, &t("Wipe persistence"))).clicked() {
            let e = mission_mut(app).map(|m| { let before = Box::new(m.clone()); m.wipe_storage(); MissionEdit::Whole(Some(before)) });
            // Whole swaps with what is there; undo brings the files back.
            if let Some(MissionEdit::Whole(Some(b))) = e { push_whole(app, &t("Wipe persistence"), *b); }
        }
        if ui.button(icons::with(icons::TRASH, &t("Remove the mission"))).on_hover_text(t("Undo brings it back; the files in the project folder stay until you delete them.")).clicked() {
            set_mission(app, None, &t("Remove the mission"));
            return;
        }
    });
    ui.weak(t("Play starts the server with this mission. Export writes it into a folder of your choice."));
    ui.separator();
    ui.label(RichText::new(t("Files")).strong());
    egui::ScrollArea::vertical().id_salt("ms_files_over").auto_shrink([false, false]).show(ui, |ui| {
        egui::Grid::new("ms_inv").num_columns(2).striped(true).show(ui, |ui| {
            for (k, dirty, help) in files {
                let label = if dirty { format!("{k} *") } else { k.clone() };
                if ui.link(RichText::new(label).monospace()).clicked() {
                    app.mission.file_sel = Some(k.clone());
                    app.mission.tab = Tab::Files;
                }
                ui.label(RichText::new(t(help)).small().color(theme::TEXT_DIM));
                ui.end_row();
            }
        });
    });
}

/// Records a change of the whole mission whose state before is `before`.
fn push_whole(app: &mut App, label: &str, before: Mission) {
    flush(app);
    if let Some(doc) = &mut app.doc {
        doc.history.push(label.to_string(), Change::Mission(vec![MissionEdit::Whole(Some(Box::new(before)))]));
    }
    app.mission.cache.valid = false;
}

// ---------------------------------------------------------------- loot

const NUM_FIELDS: [&str; 7] = ["nominal", "min", "lifetime", "restock", "quantmin", "quantmax", "cost"];
const LISTS: [&str; 4] = ["category", "usage", "value", "tag"];

fn type_rows(app: &mut App) -> Vec<(String, usize, mission::ce::TypeRow)> {
    let r = rev(app);
    if app.mission.type_rows.as_ref().is_none_or(|(rv, _)| *rv != r) {
        let rows = mission(app).map(mission::ce::all_types).unwrap_or_default();
        app.mission.type_rows = Some((r, rows));
    }
    app.mission.type_rows.as_ref().unwrap().1.clone()
}

fn list_names(m: &Mission, list: &str) -> Vec<String> {
    match list {
        "category" => m.limits("categories"),
        "usage" => m.limits("usageflags"),
        "value" => m.limits("valueflags"),
        _ => m.limits("tags"),
    }
}

fn types_tab(app: &mut App, ui: &mut egui::Ui) {
    let rows = type_rows(app);
    let m = mission(app).unwrap();
    let files = m.ce_files("types");
    let lists: Vec<Vec<String>> = LISTS.iter().map(|l| list_names(m, l)).collect();
    let user_usage: Vec<String> = m.user_limits("usageflags").into_iter().map(|(n, _)| n).collect();
    let user_value: Vec<String> = m.user_limits("valueflags").into_iter().map(|(n, _)| n).collect();
    // Filters.
    ui.horizontal_wrapped(|ui| {
        ui.add(egui::TextEdit::singleline(&mut app.mission.type_filter).hint_text(format!("{} {}", icons::MAGNIFYING_GLASS, t("Search..."))).desired_width(180.0));
        for (i, l) in LISTS.iter().enumerate() {
            let f = &mut app.mission.type_list_filter[i];
            egui::ComboBox::from_id_salt(("tf", i)).width(110.0).selected_text(if f.is_empty() { format!("{}: {}", t(l), t("all")) } else { f.clone() }).show_ui(ui, |ui| {
                ui.selectable_value(f, String::new(), t("all"));
                ui.selectable_value(f, "-".to_string(), t("(none)"));
                for n in &lists[i] { ui.selectable_value(f, n.clone(), n); }
            });
        }
        if files.len() > 1 {
            egui::ComboBox::from_id_salt("tfile").selected_text(app.mission.type_file.clone().unwrap_or_else(|| t("All files"))).show_ui(ui, |ui| {
                ui.selectable_value(&mut app.mission.type_file, None, t("All files"));
                for f in &files { ui.selectable_value(&mut app.mission.type_file, Some(f.clone()), f); }
            });
        }
    });
    let filter = app.mission.type_filter.to_lowercase();
    let lf = app.mission.type_list_filter.clone();
    let shown: Vec<usize> = rows.iter().enumerate().filter(|(_, (f, _, r))| {
        if !filter.is_empty() && !r.name.to_lowercase().contains(&filter) { return false; }
        if app.mission.type_file.as_ref().is_some_and(|x| x != f) { return false; }
        for (i, want) in lf.iter().enumerate() {
            if want.is_empty() { continue; }
            let have = match i { 0 => &r.category, 1 => &r.usage, 2 => &r.value, _ => &r.tag };
            if want == "-" { if !have.is_empty() { return false; } } else if !have.iter().any(|h| h.eq_ignore_ascii_case(want)) { return false; }
        }
        true
    }).map(|(i, _)| i).collect();
    let key = |i: usize| (rows[i].0.clone(), rows[i].1);
    app.mission.type_sel.retain(|k| rows.iter().any(|(f, i, _)| (f, i) == (&k.0, &k.1)));
    let sel_n = app.mission.type_sel.len();
    ui.horizontal(|ui| {
        ui.weak(tf("{0} of {1} types, {2} selected", &[&shown.len().to_string(), &rows.len().to_string(), &sel_n.to_string()]));
        if ui.small_button(t("Select shown")).clicked() { for &i in &shown { app.mission.type_sel.insert(key(i)); } }
        if ui.small_button(t("Select none")).clicked() { app.mission.type_sel.clear(); }
    });
    let mut edits: Vec<(String, usize, mission::ce::TypeRow)> = Vec::new();
    ui.columns(2, |cols| {
        // The table.
        let ui = &mut cols[0];
        let w = [170.0, 46.0, 40.0, 62.0, 52.0, 90.0, 110.0];
        ui.horizontal(|ui| {
            for (i, h) in ["Name", "Nominal", "Min", "Lifetime", "Restock", "Category", "Usage"].iter().enumerate() {
                ui.add_sized([w[i], 16.0], egui::Label::new(RichText::new(t(h)).small().strong()));
            }
        });
        let shift = ui.input(|i| i.modifiers.shift);
        let ctrl = ui.input(|i| i.modifiers.command);
        egui::ScrollArea::vertical().id_salt("types").auto_shrink([false, false]).max_height(ui.available_height() - 6.0).show_rows(ui, 20.0, shown.len(), |ui, range| {
            for pos in range {
                let i = shown[pos];
                let (f, idx, r) = &rows[i];
                let k = (f.clone(), *idx);
                let selected = app.mission.type_sel.contains(&k);
                let mut row = r.clone();
                ui.horizontal(|ui| {
                    let resp = ui.add_sized([w[0], 18.0], egui::SelectableLabel::new(selected || app.mission.type_focus.as_ref() == Some(&k), RichText::new(&r.name).size(12.0)));
                    if resp.clicked() {
                        if shift {
                            if let Some(a) = app.mission.type_anchor {
                                let (lo, hi) = (a.min(pos), a.max(pos));
                                for p in lo..=hi { if let Some(&j) = shown.get(p) { app.mission.type_sel.insert(key(j)); } }
                            }
                        } else if ctrl {
                            if selected { app.mission.type_sel.remove(&k); } else { app.mission.type_sel.insert(k.clone()); }
                        } else {
                            app.mission.type_sel.clear();
                            app.mission.type_sel.insert(k.clone());
                        }
                        app.mission.type_anchor = Some(pos);
                        app.mission.type_focus = Some(k.clone());
                    }
                    for (n, slot) in [&mut row.nominal, &mut row.min, &mut row.lifetime, &mut row.restock].into_iter().enumerate() {
                        match slot {
                            Some(v) => { ui.add_sized([w[n + 1], 18.0], egui::DragValue::new(v).speed(if n >= 2 { 10.0 } else { 0.2 })); }
                            None => { ui.add_sized([w[n + 1], 18.0], egui::Label::new("-")); }
                        }
                    }
                    ui.add_sized([w[5], 18.0], egui::Label::new(RichText::new(r.category.join(", ")).small()).truncate());
                    ui.add_sized([w[6], 18.0], egui::Label::new(RichText::new(r.usage.iter().chain(&r.usage_user).cloned().collect::<Vec<_>>().join(", ")).small()).truncate());
                });
                if row != *r { edits.push((f.clone(), *idx, row)); }
            }
        });
        // The type in focus and the bulk edit.
        let ui = &mut cols[1];
        egui::ScrollArea::vertical().id_salt("type_detail").auto_shrink([false, false]).show(ui, |ui| {
            let focus = app.mission.type_focus.clone().and_then(|k| rows.iter().find(|(f, i, _)| (f, i) == (&k.0, &k.1)).cloned());
            section(ui, &t("Type"), true, |ui| {
                let Some((f, idx, r)) = focus else { ui.weak(t("Click a type in the table.")); return };
                let mut row = r.clone();
                ui.weak(&f);
                ui.horizontal(|ui| { ui.label(t("Name")); ui.text_edit_singleline(&mut row.name); });
                egui::Grid::new("type_nums").num_columns(4).show(ui, |ui| {
                    let slots = [&mut row.nominal, &mut row.min, &mut row.lifetime, &mut row.restock, &mut row.quantmin, &mut row.quantmax, &mut row.cost];
                    for (n, s) in slots.into_iter().enumerate() {
                        let mut on = s.is_some();
                        if ui.checkbox(&mut on, NUM_FIELDS[n]).changed() { *s = if on { Some(0) } else { None }; }
                        if let Some(v) = s { ui.add(egui::DragValue::new(v)); } else { ui.label(""); }
                        if n % 2 == 1 { ui.end_row(); }
                    }
                });
                ui.label(RichText::new(t("Flags")).strong());
                ui.horizontal_wrapped(|ui| {
                    for (n, name) in mission::ce::TYPE_FLAGS.iter().enumerate() {
                        let mut on = row.flags[n].unwrap_or(false);
                        if ui.checkbox(&mut on, *name).changed() { row.flags[n] = Some(on); }
                    }
                });
                for (li, l) in LISTS.iter().enumerate() {
                    ui.label(RichText::new(t(l)).strong());
                    let have = match li { 0 => &mut row.category, 1 => &mut row.usage, 2 => &mut row.value, _ => &mut row.tag };
                    ui.horizontal_wrapped(|ui| {
                        for n in &lists[li] {
                            let mut on = have.iter().any(|h| h.eq_ignore_ascii_case(n));
                            if ui.checkbox(&mut on, n).changed() {
                                if on { if li == 0 { have.clear(); } have.push(n.clone()); } else { have.retain(|h| !h.eq_ignore_ascii_case(n)); }
                            }
                        }
                        for h in have.clone() {
                            if !lists[li].iter().any(|n| n.eq_ignore_ascii_case(&h)) {
                                ui.label(RichText::new(format!("{h} ?")).color(theme::ERROR)).on_hover_text(t("Not in cfglimitsdefinition.xml"));
                            }
                        }
                    });
                    let groups = match li { 1 => Some((&user_usage, &mut row.usage_user)), 2 => Some((&user_value, &mut row.value_user)), _ => None };
                    if let Some((all, have)) = groups {
                        if !all.is_empty() {
                            ui.horizontal_wrapped(|ui| {
                                ui.weak(t("Groups:"));
                                for n in all {
                                    let mut on = have.contains(n);
                                    if ui.checkbox(&mut on, n).changed() { if on { have.push(n.clone()); } else { have.retain(|h| h != n); } }
                                }
                            });
                        }
                    }
                }
                if row != r { edits.push((f.clone(), idx, row)); }
            });
            section(ui, &tf("Change the selected types ({0})", &[&sel_n.to_string()]), true, |ui| bulk_ui(app, ui, &lists));
            section(ui, &t("Add and remove"), true, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut app.mission.new_name).hint_text(t("Class name")).desired_width(160.0));
                    let target = app.mission.type_file.clone().or_else(|| files.first().cloned()).unwrap_or_else(|| "db/types.xml".into());
                    if ui.add_enabled(!app.mission.new_name.trim().is_empty(), egui::Button::new(icons::with(icons::PLUS, &t("New type")))).on_hover_text(target.clone()).clicked() {
                        let name = app.mission.new_name.trim().to_string();
                        if let Some(m) = mission_mut(app) {
                            m.ensure_xml(&target, "types");
                        }
                        let edits: Vec<_> = mission_mut(app).and_then(|m| mission::insert_child(m, &target, &[], None, mission::ce::TypeRow::new_element(&name))).into_iter().collect();
                        push(app, &t("New type"), edits);
                        app.mission.type_filter = name;
                        app.mission.new_name.clear();
                    }
                });
                if ui.add_enabled(sel_n > 0, egui::Button::new(icons::with(icons::TRASH, &tf("Delete the selected types ({0})", &[&sel_n.to_string()])))).clicked() {
                    let mut sel: Vec<(String, usize)> = app.mission.type_sel.iter().cloned().collect();
                    // Last first, so the indexes of the others stay right.
                    sel.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
                    let mut edits = Vec::new();
                    for (f, i) in sel {
                        edits.extend(mission_mut(app).and_then(|m| mission::remove_child(m, &f, &[], i)));
                    }
                    app.mission.type_sel.clear();
                    app.mission.type_focus = None;
                    push(app, &t("Delete types"), edits);
                }
                ui.horizontal(|ui| {
                    let key = egui::Id::new("new_types_file");
                    let mut name: String = ui.data_mut(|d| d.get_temp(key).unwrap_or_default());
                    ui.add(egui::TextEdit::singleline(&mut name).hint_text("types_custom.xml").desired_width(160.0));
                    if ui.add_enabled(name.trim().ends_with(".xml"), egui::Button::new(t("New types file"))).on_hover_text(t("Makes the file in the folder custom and adds it to cfgeconomycore.xml.")).clicked() {
                        let n = name.trim().to_string();
                        let edits: Vec<MissionEdit> = mission_mut(app).map(|m| mission::ce::add_ce_file(m, "custom", &n, "types", "types")).unwrap_or_default();
                        push(app, &t("New types file"), edits);
                        name.clear();
                    }
                    ui.data_mut(|d| d.insert_temp(key, name));
                });
            });
        });
    });
    for (f, idx, row) in edits {
        edit_el(app, &t("Edit type"), &f, &[idx], |e| row.write(e));
    }
}

fn bulk_ui(app: &mut App, ui: &mut egui::Ui, lists: &[Vec<String>]) {
    use mission::ce::BulkOp;
    let s = &mut app.mission;
    let mut op: Option<BulkOp> = None;
    ui.horizontal(|ui| {
        ui.label(t("Nominal and min ×"));
        ui.add(egui::DragValue::new(&mut s.bulk_factor).range(0.0..=100.0).speed(0.05).max_decimals(2));
        if ui.button(t("Apply")).clicked() { op = Some(BulkOp::Scale("nominal", s.bulk_factor as f64)); }
    });
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("bulk_field").selected_text(NUM_FIELDS[s.bulk_field]).show_ui(ui, |ui| {
            for (i, f) in NUM_FIELDS.iter().enumerate() { ui.selectable_value(&mut s.bulk_field, i, *f); }
        });
        ui.label("=");
        ui.add(egui::DragValue::new(&mut s.bulk_value));
        if ui.button(t("Set")).clicked() { op = Some(BulkOp::Set(NUM_FIELDS[s.bulk_field], s.bulk_value)); }
    });
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("bulk_list").selected_text(LISTS[s.bulk_list]).show_ui(ui, |ui| {
            for (i, f) in LISTS.iter().enumerate() { ui.selectable_value(&mut s.bulk_list, i, *f); }
        });
        egui::ComboBox::from_id_salt("bulk_name").selected_text(s.bulk_name.clone()).show_ui(ui, |ui| {
            for n in &lists[s.bulk_list] { ui.selectable_value(&mut s.bulk_name, n.clone(), n); }
        });
        let ok = !s.bulk_name.is_empty();
        if ui.add_enabled(ok, egui::Button::new(t("Add"))).clicked() { op = Some(BulkOp::Add(LISTS[s.bulk_list], s.bulk_name.clone())); }
        if ui.add_enabled(ok, egui::Button::new(t("Remove"))).clicked() { op = Some(BulkOp::Remove(LISTS[s.bulk_list], s.bulk_name.clone())); }
    });
    ui.horizontal_wrapped(|ui| {
        for (i, f) in mission::ce::TYPE_FLAGS.iter().enumerate() {
            ui.menu_button(*f, |ui| {
                if ui.button(t("On")).clicked() { op = Some(BulkOp::Flag(i, true)); ui.close_menu(); }
                if ui.button(t("Off")).clicked() { op = Some(BulkOp::Flag(i, false)); ui.close_menu(); }
            });
        }
    });
    let Some(op) = op else { return };
    let sel: Vec<(String, usize)> = app.mission.type_sel.iter().cloned().collect();
    let mut edits = Vec::new();
    for (f, i) in sel {
        let op = op.clone();
        edits.extend(mission_mut(app).and_then(|m| mission::element_edit(m, &f, &[i], |e| {
            let mut r = mission::ce::TypeRow::read(e);
            op.apply(&mut r);
            r.write(e);
        })));
    }
    let n = edits.len();
    push(app, &t("Change types"), edits);
    app.info(tf("{0} types changed", &[&n.to_string()]));
}

// ---------------------------------------------------------------- events

fn events_tab(app: &mut App, ui: &mut egui::Ui) {
    let m = mission(app).unwrap();
    let files = m.ce_files("events");
    let type_names: Vec<String> = { let mut v: Vec<String> = mission::ce::all_types(m).into_iter().map(|(_, _, r)| r.name).collect(); v.sort(); v.dedup(); v };
    let spawns = places::collect(m, &[PlaceKind::EventSpawn]);
    let mut list: Vec<(String, usize, String)> = Vec::new();
    for f in &files {
        if let Some(d) = m.xml(f) {
            for i in element_indexes(&d.root, |e| e.name == "event") {
                if let Node::Element(e) = &d.root.children[i] { list.push((f.clone(), i, e.attr("name").unwrap_or("").to_string())); }
            }
        }
    }
    let focus = app.mission.event_sel.clone().and_then(|(f, i)| m.xml(&f).and_then(|d| d.root.children.get(i)).and_then(|n| if let Node::Element(e) = n { Some((f, i, e.clone())) } else { None }));
    let mut edit: Option<(String, usize, mission::ce::EventRow)> = None;
    let mut add: Option<(String, Element)> = None;
    let mut remove: Option<(String, usize)> = None;
    ui.columns(2, |cols| {
        let ui = &mut cols[0];
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut app.mission.event_filter).hint_text(format!("{} {}", icons::MAGNIFYING_GLASS, t("Search..."))).desired_width(180.0));
            if ui.button(icons::with(icons::PLUS, &t("New event"))).clicked() {
                add = Some((files.first().cloned().unwrap_or_else(|| "db/events.xml".into()), mission::ce::EventRow::new_element("NewEvent")));
            }
        });
        let filter = app.mission.event_filter.to_lowercase();
        let shown: Vec<&(String, usize, String)> = list.iter().filter(|(_, _, n)| filter.is_empty() || n.to_lowercase().contains(&filter)).collect();
        egui::ScrollArea::vertical().id_salt("events").auto_shrink([false, false]).show_rows(ui, 18.0, shown.len(), |ui, range| {
            for (f, i, n) in &shown[range] {
                let count = spawns.iter().filter(|p| &p.group == n).count();
                let text = if count > 0 { format!("{n}  ({count})") } else { n.clone() };
                if ui.selectable_label(app.mission.event_sel.as_ref() == Some(&(f.clone(), *i)), text).clicked() {
                    app.mission.event_sel = Some((f.clone(), *i));
                }
            }
        });
        let ui = &mut cols[1];
        let Some((f, i, e)) = focus else { ui.weak(t("Pick an event.")); return };
        let r = mission::ce::EventRow::read(&e);
        let mut row = r.clone();
        egui::ScrollArea::vertical().id_salt("event_detail").auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal(|ui| { ui.label(t("Name")); ui.text_edit_singleline(&mut row.name); });
            egui::Grid::new("ev_nums").num_columns(4).show(ui, |ui| {
                let names = ["nominal", "min", "max", "lifetime", "restock", "saferadius", "distanceradius", "cleanupradius"];
                let slots = [&mut row.nominal, &mut row.min, &mut row.max, &mut row.lifetime, &mut row.restock, &mut row.saferadius, &mut row.distanceradius, &mut row.cleanupradius];
                for (n, s) in slots.into_iter().enumerate() {
                    ui.label(names[n]);
                    match s { Some(v) => { ui.add(egui::DragValue::new(v)); } None => { if ui.small_button(icons::PLUS).clicked() { *s = Some(0); } } }
                    if n % 2 == 1 { ui.end_row(); }
                }
            });
            ui.horizontal(|ui| {
                ui.label("position");
                let mut p = row.position.clone().unwrap_or_default();
                for o in ["fixed", "player", "uniform"] { ui.selectable_value(&mut p, o.to_string(), o); }
                row.position = Some(p);
            });
            ui.horizontal(|ui| {
                ui.label("limit");
                let mut p = row.limit.clone().unwrap_or_default();
                for o in ["child", "mixed", "custom", "parent"] { ui.selectable_value(&mut p, o.to_string(), o); }
                row.limit = Some(p);
            });
            ui.horizontal(|ui| {
                let mut a = row.active.unwrap_or(false);
                if ui.checkbox(&mut a, "active").changed() { row.active = Some(a); }
                for (n, name) in mission::ce::EVENT_FLAGS.iter().enumerate() {
                    let mut on = row.flags[n].unwrap_or(false);
                    if ui.checkbox(&mut on, *name).changed() { row.flags[n] = Some(on); }
                }
            });
            ui.horizontal(|ui| {
                let mut s = row.secondary.clone().unwrap_or_default();
                ui.label("secondary");
                if ui.text_edit_singleline(&mut s).changed() { row.secondary = if s.is_empty() { None } else { Some(s) }; }
            });
            ui.label(RichText::new(t("Children")).strong());
            let mut del = None;
            egui::Grid::new("ev_children").num_columns(6).striped(true).show(ui, |ui| {
                for h in ["type", "min", "max", "lootmin", "lootmax", ""] { ui.label(RichText::new(h).small()); }
                ui.end_row();
                for (n, c) in row.children.iter_mut().enumerate() {
                    egui::ComboBox::from_id_salt(("evc", n)).width(170.0).selected_text(c.kind.clone()).height(300.0).show_ui(ui, |ui| {
                        ui.text_edit_singleline(&mut c.kind);
                        let f = c.kind.to_lowercase();
                        for tn in type_names.iter().filter(|x| x.to_lowercase().contains(&f)).take(200) { ui.selectable_value(&mut c.kind, tn.clone(), tn); }
                    });
                    for v in [&mut c.min, &mut c.max, &mut c.lootmin, &mut c.lootmax] { ui.add(egui::DragValue::new(v)); }
                    if ui.small_button(icons::TRASH).clicked() { del = Some(n); }
                    ui.end_row();
                }
            });
            if let Some(n) = del { row.children.remove(n); }
            if ui.small_button(icons::with(icons::PLUS, &t("Add child"))).clicked() {
                row.children.push(row.children.last().cloned().unwrap_or(mission::ce::EventChild { kind: String::new(), min: 1, max: 1, lootmin: 0, lootmax: 0 }));
            }
            ui.separator();
            let n = spawns.iter().filter(|p| p.group == r.name).count();
            ui.label(tf("{0} spawns on the map", &[&n.to_string()]));
            ui.horizontal(|ui| {
                if ui.button(icons::with(icons::MAP_PIN, &t("Place spawns on the map"))).clicked() {
                    app.mission.event = r.name.clone();
                    app.mission.tool = MapTool::EventSpawn;
                    app.mission.shown.insert(PlaceKind::EventSpawn);
                    app.mode = Mode::Mission;
                }
                if ui.button(icons::with(icons::COPY, &t("Duplicate"))).clicked() {
                    let mut c = e.clone();
                    c.set_attr("name", format!("{}_copy", r.name));
                    add = Some((f.clone(), c));
                }
                if ui.button(icons::with(icons::TRASH, &t("Delete"))).clicked() { remove = Some((f.clone(), i)); }
            });
            ui.separator();
            event_groups_ui(app, ui, &r.name);
            if row != r { edit = Some((f.clone(), i, row)); }
        });
    });
    if let Some((f, i, row)) = edit {
        edit_el(app, &t("Edit event"), &f, &[i], |e| row.write(e));
    }
    if let Some((f, e)) = add {
        if let Some(m) = mission_mut(app) { m.ensure_xml(&f, "events"); }
        let edits: Vec<_> = mission_mut(app).and_then(|m| mission::insert_child(m, &f, &[], None, e)).into_iter().collect();
        push(app, &t("New event"), edits);
        let n = mission(app).and_then(|m| m.xml(&f)).map(|d| d.root.children.len() - 1);
        app.mission.event_sel = n.map(|n| (f, n));
    }
    if let Some((f, i)) = remove {
        let edits: Vec<_> = mission_mut(app).and_then(|m| mission::remove_child(m, &f, &[], i)).into_iter().collect();
        push(app, &t("Delete event"), edits);
        app.mission.event_sel = None;
    }
}

fn spawnable_tab(app: &mut App, ui: &mut egui::Ui) {
    let m = mission(app).unwrap();
    let mut files = m.ce_files("spawnabletypes");
    files.extend(m.ce_files("randompresets"));
    if m.xml("cfgeventgroups.xml").is_some() { files.push(m.key("cfgeventgroups.xml").unwrap()); }
    if m.xml("cfgignorelist.xml").is_some() { files.push(m.key("cfgignorelist.xml").unwrap()); }
    let mut sel = app.mission.file_sel.clone().filter(|f| files.contains(f)).unwrap_or_else(|| files.first().cloned().unwrap_or_default());
    ui.horizontal_wrapped(|ui| {
        for f in &files { ui.selectable_value(&mut sel, f.clone(), f); }
    });
    ui.weak(t(file_help(&sel).unwrap_or("")));
    app.mission.file_sel = Some(sel.clone());
    let names: &[&str] = match sel.to_ascii_lowercase().as_str() {
        s if s.contains("randompresets") => &[],
        s if s.contains("eventgroups") => &["group"],
        _ => &["type"],
    };
    element_list(app, ui, &sel, names, 220.0);
}

// ---------------------------------------------------------------- players

fn players_tab(app: &mut App, ui: &mut egui::Ui) {
    let Some(file) = mission(app).and_then(|m| m.key("cfgplayerspawnpoints.xml")) else {
        ui.weak(tf("The mission has no {0}", &["cfgplayerspawnpoints.xml"]));
        if ui.button(t("Make it")).clicked() {
            let meta = app.doc.as_ref().unwrap().world.meta.clone();
            edit_file(app, &t("Player spawns"), "cfgplayerspawnpoints.xml", |m| {
                let g = Some(("Spawn", meta.spawn));
                m.put("cfgplayerspawnpoints.xml", mission::FileData::Xml(mission::xml::XmlDoc::new(Element::new("playerspawnpoints").with_child(mission::templates::spawn_section("fresh", g)).with_child(mission::templates::spawn_section("hop", g)).with_child(mission::templates::spawn_section("travel", g)))));
            });
            flush(app);
        }
        return;
    };
    let root = mission(app).unwrap().xml(&file).unwrap().root.clone();
    let sections: Vec<(usize, String)> = element_indexes(&root, |_| true).into_iter().map(|i| (i, if let Node::Element(e) = &root.children[i] { e.name.clone() } else { String::new() })).collect();
    ui.horizontal(|ui| {
        for (_, s) in &sections { ui.selectable_value(&mut app.mission.section, s.clone(), s); }
    });
    let Some((si, _)) = sections.iter().find(|(_, s)| *s == app.mission.section).cloned() else { return };
    let Node::Element(sec) = &root.children[si] else { return };
    ui.columns(2, |cols| {
        let ui = &mut cols[0];
        egui::ScrollArea::vertical().id_salt("ps_params").auto_shrink([false, false]).show(ui, |ui| {
            for (pi, p) in sec.children.iter().enumerate() {
                let Node::Element(p) = p else { continue };
                if p.name == "generator_posbubbles" { continue; }
                let mut copy = p.clone();
                section(ui, &p.name, true, |ui| {
                    egui::Grid::new(("ps", &p.name)).num_columns(2).show(ui, |ui| {
                        let names: Vec<String> = copy.elements().map(|e| e.name.clone()).collect();
                        for n in names {
                            ui.label(&n);
                            let text = copy.child_text(&n).unwrap_or_default();
                            if matches!(text.trim(), "true" | "false") {
                                let mut b = text.trim() == "true";
                                if ui.checkbox(&mut b, "").changed() { copy.child_mut(&n).unwrap().set_text(if b { "true" } else { "false" }); }
                            } else {
                                num_text(ui, &mut copy, &n, 1.0);
                            }
                            ui.end_row();
                        }
                    });
                });
                if copy != *p { edit_el(app, &t("Player spawns"), &file, &[si, pi], |e| *e = copy); }
            }
        });
        let ui = &mut cols[1];
        let Some(bi) = sec.children.iter().position(|n| matches!(n, Node::Element(e) if e.name == "generator_posbubbles")) else { return };
        let Node::Element(bubbles) = &sec.children[bi] else { return };
        let groups = element_indexes(bubbles, |e| e.name == "group");
        ui.label(RichText::new(t("Groups")).strong());
        egui::ScrollArea::vertical().id_salt("ps_groups").max_height(200.0).auto_shrink([false, true]).show(ui, |ui| {
            for gi in &groups {
                let Node::Element(g) = &bubbles.children[*gi] else { continue };
                let n = g.children_named("pos").count();
                if ui.selectable_label(app.mission.player_group == Some(*gi), format!("{}  ({n})", g.attr("name").unwrap_or(""))).clicked() {
                    app.mission.player_group = Some(*gi);
                    app.mission.spawn_group = g.attr("name").unwrap_or("").to_string();
                    app.mission.spawn_section = app.mission.section.clone();
                }
            }
        });
        let loose = bubbles.children_named("pos").count();
        if loose > 0 { ui.weak(tf("{0} positions outside groups", &[&loose.to_string()])); }
        ui.horizontal(|ui| {
            if ui.button(icons::with(icons::MAP_PIN, &t("Place on the map"))).clicked() {
                app.mission.tool = MapTool::PlayerSpawn;
                app.mission.spawn_section = app.mission.section.clone();
                app.mission.shown.insert(PlaceKind::PlayerSpawn);
                app.mode = Mode::Mission;
            }
            if ui.button(icons::with(icons::MAP_TRIFOLD, &t("Groups from key points"))).on_hover_text(t("One group at each town and village key point of the map.")).clicked() {
                spawns_from_keypoints(app);
            }
        });
        if let Some(gi) = app.mission.player_group.filter(|g| groups.contains(g)) {
            let Node::Element(g) = &bubbles.children[gi] else { return };
            let mut copy = g.clone();
            ui.separator();
            egui::ScrollArea::vertical().id_salt("ps_group").auto_shrink([false, false]).show(ui, |ui| { tree_ui(ui, &mut copy, "ps_group"); });
            if copy != *g { edit_el(app, &t("Player spawns"), &file, &[si, bi, gi], |e| *e = copy); }
        }
    });
}

fn spawns_from_keypoints(app: &mut App) {
    let Some(doc) = &app.doc else { return };
    let points: Vec<_> = doc.world.keypoints.iter().filter(|k| matches!(k.kind.as_str(), "NameCity" | "NameCityCapital" | "NameVillage")).cloned().collect();
    if points.is_empty() {
        app.warn(t("The map has no town or village key points."));
        return;
    }
    let section = app.mission.section.clone();
    let mut edits = Vec::new();
    for k in &points {
        let name: String = k.name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        if let Some(m) = mission_mut(app) { edits.extend(places::add_player_spawn(m, &section, &name, k.x, k.z)); }
    }
    push(app, &t("Groups from key points"), edits);
    app.info(tf("{0} spawn groups added", &[&points.len().to_string()]));
}

// ---------------------------------------------------------------- territories

fn territories_tab(app: &mut App, ui: &mut egui::Ui) {
    let m = mission(app).unwrap();
    let files = m.territory_files();
    let zones = places::collect(m, &[PlaceKind::Territory]);
    let mut sel = app.mission.file_sel.clone().filter(|f| files.contains(f) || f.eq_ignore_ascii_case("cfgenvironment.xml")).unwrap_or_else(|| files.first().cloned().unwrap_or_else(|| "cfgenvironment.xml".into()));
    ui.columns(2, |cols| {
        let ui = &mut cols[0];
        if ui.selectable_label(sel.eq_ignore_ascii_case("cfgenvironment.xml"), RichText::new("cfgenvironment.xml").strong()).clicked() { sel = "cfgenvironment.xml".into(); }
        ui.weak(t(file_help("cfgenvironment.xml").unwrap()));
        ui.separator();
        egui::ScrollArea::vertical().id_salt("terr_files").max_height(260.0).show(ui, |ui| {
            for f in &files {
                let n = zones.iter().filter(|p| matches!(&p.at, PlaceRef::Xml { file, .. } if file == f)).count();
                if ui.selectable_label(sel == *f, format!("{f}  ({n})")).clicked() { sel = f.clone(); }
            }
        });
        ui.horizontal(|ui| {
            let key = egui::Id::new("new_terr");
            let mut name: String = ui.data_mut(|d| d.get_temp(key).unwrap_or_default());
            ui.add(egui::TextEdit::singleline(&mut name).hint_text("wolf_territories").desired_width(150.0));
            if ui.add_enabled(!name.trim().is_empty(), egui::Button::new(t("New territory file"))).on_hover_text(t("Makes env/<name>.xml and adds it to cfgenvironment.xml.")).clicked() {
                let n = name.trim().trim_end_matches(".xml").to_string();
                let edits = mission_mut(app).map(|m| mission::ce::add_territory_file(m, &n)).unwrap_or_default();
                push(app, &t("New territory file"), edits);
                sel = format!("env/{n}.xml");
                name.clear();
            }
            ui.data_mut(|d| d.insert_temp(key, name));
        });
        ui.separator();
        section(ui, &t("Zones from key points"), false, |ui| {
            ui.weak(t("A zone at each key point of the kinds ticked, in the file and territory of the zone tool."));
            ui.horizontal_wrapped(|ui| {
                for k in dz_core::keypoints::TYPES {
                    let mut on = app.mission.keypoint_kinds.contains(k);
                    if ui.checkbox(&mut on, k).changed() { if on { app.mission.keypoint_kinds.insert(k.to_string()); } else { app.mission.keypoint_kinds.remove(k); } }
                }
            });
            row(ui, &t("Name"), |ui| { ui.text_edit_singleline(&mut app.mission.zone_name); });
            row(ui, "dmin / dmax", |ui| { ui.add(egui::DragValue::new(&mut app.mission.zone_dynamic[0])); ui.add(egui::DragValue::new(&mut app.mission.zone_dynamic[1])); });
            if ui.button(t("Add zones")).clicked() {
                let kinds: Vec<String> = app.mission.keypoint_kinds.iter().cloned().collect();
                let kinds: Vec<&str> = kinds.iter().map(String::as_str).collect();
                let points = app.doc.as_ref().unwrap().world.keypoints.clone();
                let made = mission::gen::zones_from_keypoints(&points, &kinds, &app.mission.zone_name, app.mission.zone_dynamic[0], app.mission.zone_dynamic[1]);
                let target = if sel.starts_with("env/") { sel.clone() } else { "env/zombie_territories.xml".to_string() };
                let n = made.len();
                let mut edits = Vec::new();
                for z in made {
                    let territory = app.mission.zone_territory;
                    if let Some(m) = mission_mut(app) { edits.extend(places::add_zone(m, &target, territory, z)); }
                }
                push(app, &t("Zones from key points"), edits);
                app.info(tf("{0} zones added", &[&n.to_string()]));
            }
        });
        if ui.button(icons::with(icons::MAP_PIN, &t("Place zones on the map"))).clicked() {
            if sel.starts_with("env/") { app.mission.zone_file = sel.clone(); }
            app.mission.tool = MapTool::Zone;
            app.mission.shown.insert(PlaceKind::Territory);
            app.mode = Mode::Mission;
        }
        let ui = &mut cols[1];
        element_list(app, ui, &sel, &[], 200.0);
    });
    app.mission.file_sel = Some(sel);
}

// ---------------------------------------------------------------- effect areas and triggers

fn areas_tab(app: &mut App, ui: &mut egui::Ui) {
    ui.columns(2, |cols| {
        let ui = &mut cols[0];
        ui.horizontal(|ui| {
            if ui.button(icons::with(icons::MAP_PIN, &t("Place areas on the map"))).clicked() {
                app.mission.tool = MapTool::Area;
                app.mode = Mode::Mission;
            }
            if ui.button(icons::with(icons::POLYGON, &t("Areas from the selected shapes"))).on_hover_text(t("A contaminated area around each selected shape of the Shapes mode.")).clicked() {
                areas_from_shapes(app);
            }
        });
        json_file_ui(app, ui, "cfgeffectarea.json");
        let ui = &mut cols[1];
        if ui.button(icons::with(icons::MAP_PIN, &t("Place triggers on the map"))).clicked() {
            app.mission.tool = MapTool::Trigger;
            app.mode = Mode::Mission;
        }
        json_file_ui(app, ui, "cfgundergroundtriggers.json");
    });
}

fn areas_from_shapes(app: &mut App) {
    let Some(doc) = &app.doc else { return };
    let w = &doc.world;
    let mut made = Vec::new();
    for (li, si) in app.shape.selection.iter() {
        let Some(s) = w.shape_layers.get(*li).and_then(|l| l.shapes.get(*si)) else { continue };
        if s.points.is_empty() { continue; }
        let n = s.points.len() as f32;
        let (cx, cz) = (s.points.iter().map(|p| p[0]).sum::<f32>() / n, s.points.iter().map(|p| p[1]).sum::<f32>() / n);
        let r = s.points.iter().map(|p| (p[0] - cx).hypot(p[1] - cz)).fold(10.0f32, f32::max);
        made.push((cx, cz, r));
    }
    if made.is_empty() {
        app.warn(t("Select shapes in the Shapes mode first."));
        return;
    }
    let name = app.mission.area_name.clone();
    let mut edits = Vec::new();
    for (i, (x, z, r)) in made.iter().enumerate() {
        if let Some(m) = mission_mut(app) { edits.push(places::add_json(m, "cfgeffectarea.json", "Areas", places::effect_area(&format!("{name}-{}", i + 1), *x, *z, *r))); }
    }
    push(app, &t("Areas from the selected shapes"), edits);
}

/// A JSON file of the mission as a tree to edit.
fn json_file_ui(app: &mut App, ui: &mut egui::Ui, file: &str) {
    let Some(doc) = mission(app).and_then(|m| m.json(file)) else {
        ui.weak(tf("The mission has no {0}", &[file]));
        if ui.button(t("Make it")).clicked() {
            let v = match file {
                "cfggameplay.json" => mission::templates::gameplay_json(),
                "cfgeffectarea.json" => serde_json::json!({ "Areas": [], "SafePositions": [] }),
                _ => serde_json::json!({ "Triggers": [] }),
            };
            edit_file(app, file, file, |m| m.put(file, mission::FileData::Json(mission::JsonDoc::new(v))));
            flush(app);
        }
        return;
    };
    ui.weak(t(file_help(file).unwrap_or("")));
    let original = doc.value.clone();
    let mut v = original.clone();
    egui::ScrollArea::vertical().id_salt(("json", file)).auto_shrink([false, false]).show(ui, |ui| { json_ui(ui, &mut v, file); });
    if v != original {
        let key = mission(app).and_then(|m| m.key(file)).unwrap_or_else(|| file.to_string());
        edit_file(app, file, &key.clone(), |m| { if let Some(d) = m.json_mut(&key) { d.value = v; } });
    }
}

// ---------------------------------------------------------------- weather

fn weather_tab(app: &mut App, ui: &mut egui::Ui) {
    let Some(file) = mission(app).and_then(|m| m.key("cfgweather.xml")) else {
        ui.weak(tf("The mission has no {0}", &["cfgweather.xml"]));
        if ui.button(t("Make it")).clicked() {
            edit_file(app, &t("Weather"), "cfgweather.xml", |m| m.put("cfgweather.xml", mission::FileData::Xml(mission::templates::weather_doc())));
            flush(app);
        }
        return;
    };
    let root = mission(app).unwrap().xml(&file).unwrap().root.clone();
    let mut copy = root.clone();
    egui::ScrollArea::vertical().id_salt("weather").auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal(|ui| {
            for (a, help) in [("enable", "Off leaves the weather to the game."), ("reset", "On ignores the weather saved in the persistence.")] {
                let mut on = copy.attr(a).is_some_and(mission::ce::is_true);
                if ui.checkbox(&mut on, a).on_hover_text(t(help)).changed() { copy.set_attr(a, if on { "1" } else { "0" }); }
            }
        });
        let parts: Vec<String> = copy.elements().filter(|e| e.name != "storm").map(|e| e.name.clone()).collect();
        egui::Grid::new("weather_grid").num_columns(5).striped(true).spacing(vec2(10.0, 4.0)).show(ui, |ui| {
            for h in ["", "current: actual / time / duration", "limits: min / max", "timelimits: min / max", "changelimits: min / max"] { ui.label(RichText::new(h).small().strong()); }
            ui.end_row();
            for p in &parts {
                let el = copy.child_mut(p).unwrap();
                ui.label(RichText::new(p).strong());
                let speed = if p.starts_with("wind") { 0.05 } else { 0.01 };
                for (c, attrs, sp) in [("current", ["actual", "time", "duration"].as_slice(), speed), ("limits", &["min", "max"], speed), ("timelimits", &["min", "max"], 10.0), ("changelimits", &["min", "max"], speed)] {
                    ui.horizontal(|ui| {
                        if let Some(ce) = el.child_mut(c) {
                            for (n, a) in attrs.iter().enumerate() { num_attr(ui, ce, a, if c == "current" && n > 0 { 10.0 } else { sp }); }
                        } else { ui.weak("-"); }
                    });
                }
                ui.end_row();
                if let Some(th) = el.child_mut("thresholds") {
                    ui.label("");
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("thresholds min / max / end").small());
                        num_attr(ui, th, "min", 0.01);
                        num_attr(ui, th, "max", 0.01);
                        num_attr(ui, th, "end", 10.0);
                    });
                    ui.end_row();
                }
            }
        });
        if let Some(s) = copy.child_mut("storm") {
            ui.horizontal(|ui| {
                ui.label(RichText::new("storm").strong());
                for (a, sp) in [("density", 0.01), ("threshold", 0.01), ("timeout", 1.0)] { ui.label(a); num_attr(ui, s, a, sp); }
            });
        }
        ui.weak(t("Overcast, fog, rain and snowfall go from 0 to 1, wind in m/s, its direction in radians, times in seconds."));
    });
    if copy != root { edit_el(app, &t("Weather"), &file, &[], |e| *e = copy); }
}

// ---------------------------------------------------------------- economy

fn economy_tab(app: &mut App, ui: &mut egui::Ui) {
    let m = mission(app).unwrap();
    let globals = m.key("db/globals.xml");
    let economy = m.key("db/economy.xml");
    let groot = globals.as_ref().and_then(|g| m.xml(g)).map(|d| d.root.clone());
    let eroot = economy.as_ref().and_then(|g| m.xml(g)).map(|d| d.root.clone());
    let mut sub = app.mission.file_sel.clone().filter(|s| ["globals", "economy", "limits", "user", "core"].contains(&s.as_str())).unwrap_or_else(|| "globals".into());
    ui.horizontal(|ui| {
        for (k, l) in [("globals", "db/globals.xml"), ("economy", "db/economy.xml"), ("limits", "cfglimitsdefinition.xml"), ("user", "cfglimitsdefinitionuser.xml"), ("core", "cfgeconomycore.xml")] {
            ui.selectable_value(&mut sub, k.to_string(), l);
        }
    });
    app.mission.file_sel = Some(sub.clone());
    match sub.as_str() {
        "globals" => {
            let (Some(file), Some(root)) = (globals, groot) else { ui.weak(tf("The mission has no {0}", &["db/globals.xml"])); return };
            let mut copy = root.clone();
            egui::ScrollArea::vertical().id_salt("globals").auto_shrink([false, false]).show(ui, |ui| {
                egui::Grid::new("globals_grid").num_columns(3).striped(true).show(ui, |ui| {
                    for v in copy.children_named_mut("var") {
                        let name = v.attr("name").unwrap_or("").to_string();
                        let help = mission::ce::GLOBALS_HELP.iter().find(|(n, _)| *n == name).map(|(_, h)| t(h)).unwrap_or_default();
                        ui.label(&name).on_hover_text(&help);
                        let float = v.attr("type") == Some("1");
                        num_attr(ui, v, "value", if float { 0.01 } else { 1.0 });
                        ui.label(RichText::new(help).small().color(theme::TEXT_DIM));
                        ui.end_row();
                    }
                });
            });
            if copy != root { edit_el(app, &t("Globals"), &file, &[], |e| *e = copy); }
        }
        "economy" => {
            let (Some(file), Some(root)) = (economy, eroot) else { ui.weak(tf("The mission has no {0}", &["db/economy.xml"])); return };
            let mut copy = root.clone();
            egui::Grid::new("eco_grid").num_columns(5).striped(true).show(ui, |ui| {
                for h in ["", "init", "load", "respawn", "save"] { ui.label(RichText::new(h).strong()); }
                ui.end_row();
                for e in copy.elements_mut() {
                    let help = mission::ce::ECONOMY_SECTIONS.iter().find(|(n, _)| *n == e.name).map(|(_, h)| t(h)).unwrap_or_default();
                    ui.label(&e.name).on_hover_text(help);
                    for a in ["init", "load", "respawn", "save"] {
                        let mut on = e.attr(a).is_some_and(mission::ce::is_true);
                        if ui.checkbox(&mut on, "").changed() { e.set_attr(a, if on { "1" } else { "0" }); }
                    }
                    ui.end_row();
                }
            });
            ui.weak(t("init: spawn at start; load: read from the persistence; respawn: spawn again over time; save: write to the persistence."));
            if copy != root { edit_el(app, &t("Economy"), &file, &[], |e| *e = copy); }
        }
        "limits" => limits_ui(app, ui),
        "user" => element_list(app, ui, "cfglimitsdefinitionuser.xml", &[], 200.0),
        _ => element_list(app, ui, "cfgeconomycore.xml", &[], 200.0),
    }
}

/// The lists of `cfglimitsdefinition.xml`, each with names to add and remove.
fn limits_ui(app: &mut App, ui: &mut egui::Ui) {
    let Some(file) = mission(app).and_then(|m| m.key("cfglimitsdefinition.xml")) else { ui.weak(tf("The mission has no {0}", &["cfglimitsdefinition.xml"])); return };
    let root = mission(app).unwrap().xml(&file).unwrap().root.clone();
    let mut copy = root.clone();
    ui.weak(t(file_help("cfglimitsdefinition.xml").unwrap()));
    ui.columns(4, |cols| {
        for (i, (list, item)) in [("categories", "category"), ("tags", "tag"), ("usageflags", "usage"), ("valueflags", "value")].into_iter().enumerate() {
            let ui = &mut cols[i];
            ui.label(RichText::new(list).strong());
            let l = copy.ensure_child(list);
            let mut remove = None;
            for (n, e) in l.children.iter().enumerate() {
                if let Node::Element(e) = e {
                    ui.horizontal(|ui| {
                        if ui.small_button(icons::X).clicked() { remove = Some(n); }
                        ui.label(e.attr("name").unwrap_or(""));
                    });
                }
            }
            if let Some(n) = remove { l.children.remove(n); }
            let key = egui::Id::new(("lim_new", list));
            let mut name: String = ui.data_mut(|d| d.get_temp(key).unwrap_or_default());
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut name).desired_width(100.0));
                if ui.add_enabled(!name.trim().is_empty(), egui::Button::new(icons::PLUS).small()).clicked() {
                    l.push(Element::new(item).with_attr("name", name.trim()));
                    name.clear();
                }
            });
            ui.data_mut(|d| d.insert_temp(key, name));
        }
    });
    ui.weak(t("The order of the usage and value flags is the order of the bits of areaflags.map."));
    if copy != root { edit_el(app, &t("Limits"), &file, &[], |e| *e = copy); }
}

// ---------------------------------------------------------------- messages

fn messages_tab(app: &mut App, ui: &mut egui::Ui) {
    let Some(file) = mission(app).and_then(|m| m.key("db/messages.xml")) else {
        ui.weak(tf("The mission has no {0}", &["db/messages.xml"]));
        if ui.button(t("Make it")).clicked() {
            edit_file(app, &t("Messages"), "db/messages.xml", |m| m.put("db/messages.xml", mission::FileData::Xml(mission::xml::XmlDoc::new(Element::new("messages")))));
            flush(app);
        }
        return;
    };
    let root = mission(app).unwrap().xml(&file).unwrap().root.clone();
    let mut copy = root.clone();
    ui.weak(t("#name is the server name, #tmin the minutes left to the shutdown. Times are in minutes."));
    let mut remove = None;
    egui::ScrollArea::vertical().id_salt("messages").auto_shrink([false, false]).show(ui, |ui| {
        for (i, n) in copy.children.iter_mut().enumerate() {
            let Node::Element(msg) = n else { continue };
            if msg.name != "message" { continue; }
            egui::Frame::group(ui.style()).show(ui, |ui| {
                let mut text = msg.child_text("text").unwrap_or_default();
                if ui.add(egui::TextEdit::multiline(&mut text).desired_rows(1).desired_width(f32::INFINITY)).changed() {
                    msg.set_child_text("text", &text, &["delay", "repeat", "deadline", "onconnect", "shutdown", "text"]);
                }
                ui.horizontal(|ui| {
                    for k in ["delay", "repeat", "deadline"] {
                        let mut v = msg.child_i64(k).unwrap_or(0);
                        ui.label(k);
                        if ui.add(egui::DragValue::new(&mut v).range(0..=100000)).changed() {
                            if v == 0 { msg.remove_children(k); } else { msg.set_child_num(k, v as f64, &["delay", "repeat", "deadline", "onconnect", "shutdown", "text"]); }
                        }
                    }
                    for k in ["onconnect", "shutdown"] {
                        let mut on = msg.child_i64(k).unwrap_or(0) != 0;
                        if ui.checkbox(&mut on, k).changed() {
                            if on { msg.set_child_num(k, 1.0, &["delay", "repeat", "deadline", "onconnect", "shutdown", "text"]); } else { msg.remove_children(k); }
                        }
                    }
                    if ui.small_button(icons::TRASH).clicked() { remove = Some(i); }
                });
            });
        }
        if ui.button(icons::with(icons::PLUS, &t("New message"))).clicked() {
            copy.push(Element::new("message").with_child(Element::new("repeat").with_text("15")).with_child(Element::new("text").with_text("Welcome to #name")));
        }
    });
    if let Some(i) = remove { copy.children.remove(i); }
    if copy != root { edit_el(app, &t("Messages"), &file, &[], |e| *e = copy); }
}

// ---------------------------------------------------------------- map groups

fn map_groups_tab(app: &mut App, ui: &mut egui::Ui) {
    let m = mission(app).unwrap();
    let protos = m.xml("mapgroupproto.xml").map(|d| d.root.children_named("group").count()).unwrap_or(0);
    let groups = m.xml("mapgrouppos.xml").map(|d| d.root.children_named("group").count()).unwrap_or(0);
    let exports = mission::gen::cluster_exports(m).len();
    let objects = app.doc.as_ref().unwrap().world.objects.len();
    ui.label(tf("{0} prototypes, {1} buildings in mapgrouppos.xml, {2} cluster models; {3} objects on the map.", &[&protos.to_string(), &groups.to_string(), &exports.to_string(), &objects.to_string()]));
    ui.horizontal(|ui| {
        ui.checkbox(&mut app.mission.with_clusters, t("Clusters too (mapgroupcluster.xml)"));
        if ui.button(icons::with(icons::BUILDINGS, &t("Make map groups from the placed objects"))).on_hover_text(t("Every placed building whose class has a prototype goes into mapgrouppos.xml, with the position and angles the game would write.")).clicked() {
            make_groups(app);
        }
        let mut show = app.mission.shown.contains(&PlaceKind::MapGroup);
        if ui.checkbox(&mut show, t("Show on the map")).changed() { if show { app.mission.shown.insert(PlaceKind::MapGroup); } else { app.mission.shown.remove(&PlaceKind::MapGroup); } }
    });
    if let Some(r) = app.mission.last_report.clone() {
        ui.label(tf("{0} buildings and {1} clusters written.", &[&r.groups.to_string(), &r.clusters.to_string()]));
        if !r.without_proto.is_empty() {
            egui::CollapsingHeader::new(tf("{0} placed classes have no prototype", &[&r.without_proto.len().to_string()])).show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("noproto").max_height(140.0).show(ui, |ui| {
                    for (c, n) in &r.without_proto { ui.label(format!("{c}  ×{n}")); }
                });
                ui.horizontal(|ui| {
                    if ui.button(t("Copy prototypes from another mapgroupproto.xml...")).on_hover_text(t("For example from a vanilla mission, for buildings of the game.")).clicked() {
                        copy_protos(app, r.without_proto.iter().map(|(c, _)| c.to_ascii_lowercase()).collect());
                    }
                    if ui.button(t("Add empty prototypes")).on_hover_text(t("One container without loot points for each; ExportProxyProto in init.c makes the points in the game.")).clicked() {
                        let file = mission(app).and_then(|m| m.key("mapgroupproto.xml")).unwrap_or_else(|| "mapgroupproto.xml".into());
                        let classes: Vec<String> = r.without_proto.iter().map(|(c, _)| c.clone()).collect();
                        edit_file(app, &t("Add empty prototypes"), &file.clone(), |m| {
                            let d = m.ensure_xml(&file, "prototype");
                            for c in classes { d.root.push(mission::gen::empty_prototype(&c)); }
                        });
                        flush(app);
                    }
                });
            });
        }
    }
    ui.separator();
    let mut sel = app.mission.file_sel.clone().filter(|s| ["mapgroupproto.xml", "mapclusterproto.xml", "mapgrouppos.xml"].contains(&s.as_str())).unwrap_or_else(|| "mapgroupproto.xml".into());
    ui.horizontal(|ui| {
        for f in ["mapgroupproto.xml", "mapclusterproto.xml", "mapgrouppos.xml"] { ui.selectable_value(&mut sel, f.to_string(), f); }
    });
    ui.weak(t(file_help(&sel).unwrap_or("")));
    app.mission.file_sel = Some(sel.clone());
    element_list(app, ui, &sel, &[], 200.0);
}

fn make_groups(app: &mut App) {
    let Some(doc) = &app.doc else { return };
    let (objects, _) = dz_core::export::wrp_objects(&doc.world, &app.db);
    let clusters = app.mission.with_clusters;
    let Some(m) = mission_mut(app) else { return };
    let (edits, report) = mission::gen::map_groups(m, &objects, clusters);
    push(app, &t("Make map groups"), edits);
    app.info(tf("{0} buildings and {1} clusters written.", &[&report.groups.to_string(), &report.clusters.to_string()]));
    app.mission.last_report = Some(report);
}

fn copy_protos(app: &mut App, classes: std::collections::HashSet<String>) {
    let Some(path) = rfd::FileDialog::new().add_filter("mapgroupproto", &["xml"]).set_title(t("Choose a mapgroupproto.xml")).pick_file() else { return };
    let doc = match std::fs::read(&path).map_err(anyhow::Error::from).and_then(|b| mission::xml::parse(&b)) {
        Ok(d) => d,
        Err(e) => { app.error(format!("{}: {e:#}", path.display())); return; }
    };
    let Some(m) = mission_mut(app) else { return };
    let (edit, n) = mission::gen::copy_prototypes(m, &doc, &classes);
    push(app, &t("Copy prototypes"), edit.into_iter().collect());
    app.info(tf("{0} prototypes copied", &[&n.to_string()]));
}

// ---------------------------------------------------------------- area flags

fn flags_tab(app: &mut App, ui: &mut egui::Ui) {
    let meta = app.doc.as_ref().unwrap().world.meta.clone();
    let m = mission(app).unwrap();
    let Some(a) = m.areaflags() else {
        ui.label(t("The mission has no areaflags.map."));
        if ui.button(t("Make one for the map")).clicked() { make_areaflags(app); }
        return;
    };
    let (w, h, ww, wh, ub, vb) = (a.width, a.height, a.world_width, a.world_height, a.usage_bits, a.value_bits);
    ui.label(tf("{0} × {1} cells over {2} × {3} m; {4} bits of usage, {5} bits of value.", &[&w.to_string(), &h.to_string(), &ww.to_string(), &wh.to_string(), &ub.to_string(), &vb.to_string()]));
    let size = meta.size_m().round() as u32;
    if ww != size || wh != size {
        ui.colored_label(theme::WARN, tf("The map is {0} m; the flags cover {1} m.", &[&size.to_string(), &ww.to_string()]));
        if ui.button(t("Fit the flags to the map")).clicked() {
            let a2 = a.resized(w, size);
            edit_file(app, &t("Area flags"), "areaflags.map", |m| { if let Some(x) = m.areaflags_mut() { *x = a2; } });
            flush(app);
            return;
        }
    }
    let r = rev(app);
    let layer = app.mission.flag_layer;
    ui.horizontal(|ui| {
        ui.selectable_value(&mut app.mission.flag_layer, FlagLayer::Usage, t("Usage"));
        ui.selectable_value(&mut app.mission.flag_layer, FlagLayer::Value, t("Value"));
        if ui.button(t("Count cells")).clicked() {
            let c = mission(app).and_then(|m| m.areaflags()).map(|a| a.counts(layer)).unwrap_or_default();
            app.mission.flag_counts = Some((r, layer, c));
        }
    });
    let names = flag_names(mission(app).unwrap(), layer);
    let counts = app.mission.flag_counts.as_ref().filter(|(rv, l, _)| *rv == r && *l == layer).map(|(_, _, c)| c.clone());
    let bits = if layer == FlagLayer::Usage { ub } else { vb };
    egui::Grid::new("flag_bits").num_columns(4).striped(true).show(ui, |ui| {
        for b in 0..bits.min(32) {
            if (b as usize) >= names.len() && counts.as_ref().is_none_or(|c| c.get(b as usize).copied().unwrap_or(0) == 0) { continue; }
            let (rc, _) = ui.allocate_exact_size(vec2(12.0, 12.0), egui::Sense::hover());
            ui.painter().rect_filled(rc, 2.0, flag_color(b));
            if ui.selectable_label(app.mission.flag_bit == b, names.get(b as usize).cloned().unwrap_or_else(|| tf("bit {0}", &[&b.to_string()]))).clicked() { app.mission.flag_bit = b; }
            ui.label(counts.as_ref().and_then(|c| c.get(b as usize)).map(|n| tf("{0} cells", &[&n.to_string()])).unwrap_or_default());
            ui.end_row();
        }
    });
    if names.len() > bits as usize {
        ui.colored_label(theme::ERROR, tf("cfglimitsdefinition.xml has {0} flags; the layer holds {1}.", &[&names.len().to_string(), &bits.to_string()]));
        if ui.button(t("Widen the layer")).clicked() {
            let a2 = mission(app).unwrap().areaflags().unwrap().with_bits(layer, mission::areaflags::bits_for(names.len()));
            edit_file(app, &t("Area flags"), "areaflags.map", |m| { if let Some(x) = m.areaflags_mut() { *x = a2; } });
            flush(app);
        }
    }
    ui.separator();
    ui.horizontal_wrapped(|ui| {
        if ui.button(icons::with(icons::PAINT_BRUSH, &t("Paint on the map"))).clicked() {
            app.mission.tool = MapTool::Flags;
            app.mission.show_flags = true;
            app.mode = Mode::Mission;
            app.view_mode = crate::app::ViewMode::Top;
        }
        if ui.button(icons::with(icons::POLYGON, &t("Fill the selected shapes"))).on_hover_text(t("Sets the flag in every cell inside the polygons selected in the Shapes mode.")).clicked() { fill_shapes(app, true); }
        if ui.button(t("Clear the flag in the selected shapes")).clicked() { fill_shapes(app, false); }
        if ui.button(t("Clear the flag everywhere")).clicked() {
            let (layer, bit) = (app.mission.flag_layer, app.mission.flag_bit);
            edit_file(app, &t("Area flags"), "areaflags.map", |m| { if let Some(a) = m.areaflags_mut() { let (w, h) = (a.width, a.height); a.paint(layer, bit, false, 0, 0, w, h, |_, _| true); } });
            flush(app);
        }
    });
}

fn fill_shapes(app: &mut App, on: bool) {
    let Some(doc) = &app.doc else { return };
    let w = &doc.world;
    let polys: Vec<Vec<[f32; 2]>> = app.shape.selection.iter().filter_map(|(li, si)| w.shape_layers.get(*li).and_then(|l| l.shapes.get(*si))).filter(|s| s.points.len() >= 3).map(|s| s.points.clone()).collect();
    if polys.is_empty() {
        app.warn(t("Select polygons in the Shapes mode first."));
        return;
    }
    let (layer, bit) = (app.mission.flag_layer, app.mission.flag_bit);
    let mut n = 0;
    edit_file(app, &t("Area flags"), "areaflags.map", |m| {
        let Some(a) = m.areaflags_mut() else { return };
        let (cw, ch) = a.cell_size();
        for p in &polys {
            let (mut x0, mut z0, mut x1, mut z1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for q in p { x0 = x0.min(q[0]); z0 = z0.min(q[1]); x1 = x1.max(q[0]); z1 = z1.max(q[1]); }
            let cx0 = (x0 / cw).floor().max(0.0) as u32;
            let cy0 = (z0 / ch).floor().max(0.0) as u32;
            let cx1 = ((x1 / cw).ceil().max(0.0) as u32).min(a.width);
            let cy1 = ((z1 / ch).ceil().max(0.0) as u32).min(a.height);
            n += a.paint(layer, bit, on, cx0, cy0, cx1, cy1, |x, z| dz_core::shapes::point_in_polygon(p, x, z));
        }
    });
    flush(app);
    app.info(tf("{0} cells changed", &[&n.to_string()]));
}

// ---------------------------------------------------------------- init.c

fn init_tab(app: &mut App, ui: &mut egui::Ui) {
    use mission::initc;
    let Some(file) = mission(app).and_then(|m| m.key("init.c")) else {
        ui.weak(tf("The mission has no {0}", &["init.c"]));
        if ui.button(t("Make it")).clicked() {
            edit_file(app, "init.c", "init.c", |m| m.put("init.c", mission::FileData::Text(mission::TextDoc::new(mission::templates::INIT_C))));
            flush(app);
        }
        return;
    };
    let size = app.doc.as_ref().unwrap().world.meta.size_m();
    let original = mission(app).unwrap().text(&file).map(|t| t.text.clone()).unwrap_or_default();
    let mut text = original.clone();
    ui.columns(2, |cols| {
        let ui = &mut cols[0];
        section(ui, &t("Settings found in the script"), true, |ui| {
            match initc::reset_date(&text) {
                Some((mut mo, mut d)) => {
                    row(ui, &t("Date reset to"), |ui| {
                        let a = ui.add(egui::DragValue::new(&mut d).range(1..=31).prefix(t("day ")));
                        let b = ui.add(egui::DragValue::new(&mut mo).range(1..=12).prefix(t("month ")));
                        if a.changed() || b.changed() { text = initc::set_reset_date(&text, mo, d); }
                    });
                    ui.weak(t("The server keeps its date within a month from this day."));
                }
                None => { ui.weak(t("No date reset in the script.")); }
            }
            if let Some((mut lo, mut hi)) = initc::start_health(&text) {
                row(ui, &t("Health of the starting items"), |ui| {
                    let a = ui.add(egui::DragValue::new(&mut lo).range(0.0..=1.0).speed(0.01));
                    let b = ui.add(egui::DragValue::new(&mut hi).range(0.0..=1.0).speed(0.01));
                    if a.changed() || b.changed() { text = initc::set_start_health(&text, lo, hi.max(lo)); }
                });
            }
            let items = initc::start_items(&text);
            if !items.is_empty() {
                ui.label(t("Items new characters get:"));
                ui.label(RichText::new(items.join(", ")).small().color(theme::TEXT_DIM));
            }
        });
        section(ui, &t("Economy files written by the game"), true, |ui| {
            ui.weak(t("For a new map: the server writes these files into its storage folder on the next start. Remove the calls once they are written."));
            for e in initc::CeExport::ALL {
                let mut on = initc::has_export(&text, e);
                let name = match e { initc::CeExport::ProxyProto => "ExportProxyProto (mapgroupproto.xml)", initc::CeExport::ProxyData => "ExportProxyData (mapgrouppos.xml)", initc::CeExport::ClusterData => "ExportClusterData (mapgroupcluster.xml)", initc::CeExport::SpawnData => "ExportSpawnData (player spawns)" };
                if ui.checkbox(&mut on, name).changed() { text = initc::set_export(&text, e, on, [size * 0.5, size * 0.5], size); }
            }
        });
        let ui = &mut cols[1];
        egui::ScrollArea::vertical().id_salt("initc").auto_shrink([false, false]).show(ui, |ui| {
            ui.add(egui::TextEdit::multiline(&mut text).code_editor().desired_width(f32::INFINITY).desired_rows(30));
        });
    });
    if text != original {
        edit_file(app, "init.c", &file.clone(), |m| { if let Some(d) = m.text_mut(&file) { d.text = text; } });
    }
}

// ---------------------------------------------------------------- files

fn files_tab(app: &mut App, ui: &mut egui::Ui) {
    let m = mission(app).unwrap();
    let files: Vec<(String, &'static str, bool, u64)> = m.files.iter().map(|(k, f)| {
        let kind = match f.data { mission::FileData::Xml(_) => "XML", mission::FileData::Json(_) => "JSON", mission::FileData::Text(_) => "TXT", mission::FileData::AreaFlags(_) => "MAP", mission::FileData::Disk => "—" };
        (k.clone(), kind, f.dirty, f.origin.as_ref().and_then(|o| o.metadata().ok()).map(|m| m.len()).unwrap_or(0))
    }).collect();
    let mut sel = app.mission.file_sel.clone().filter(|s| m.files.contains_key(s));
    ui.columns(2, |cols| {
        let ui = &mut cols[0];
        egui::ScrollArea::vertical().id_salt("files_list").auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("files_grid").num_columns(3).striped(true).show(ui, |ui| {
                for (k, kind, dirty, size) in &files {
                    if ui.selectable_label(sel.as_ref() == Some(k), RichText::new(if *dirty { format!("{k} *") } else { k.clone() }).monospace()).on_hover_text(t(file_help(k).unwrap_or(""))).clicked() { sel = Some(k.clone()); }
                    ui.label(RichText::new(*kind).small());
                    ui.label(RichText::new(if *size > 0 { format!("{:.0} kB", *size as f64 / 1024.0) } else { String::new() }).small());
                    ui.end_row();
                }
            });
        });
        let ui = &mut cols[1];
        let Some(k) = sel.clone() else { ui.weak(t("Pick a file.")); return };
        ui.label(RichText::new(&k).strong());
        ui.weak(t(file_help(&k).unwrap_or("")));
        let kind = mission(app).and_then(|m| m.files.get(&k)).map(|f| match f.data { mission::FileData::Xml(_) => 0, mission::FileData::Json(_) => 1, mission::FileData::Text(_) => 2, mission::FileData::AreaFlags(_) => 3, mission::FileData::Disk => 4 }).unwrap_or(4);
        match kind {
            0 => element_list(app, ui, &k, &[], 180.0),
            1 => json_file_ui(app, ui, &k),
            2 => {
                let original = mission(app).unwrap().text(&k).unwrap().text.clone();
                let mut text = original.clone();
                egui::ScrollArea::vertical().id_salt("file_text").auto_shrink([false, false]).show(ui, |ui| { ui.add(egui::TextEdit::multiline(&mut text).code_editor().desired_width(f32::INFINITY)); });
                if text != original { edit_file(app, &k, &k.clone(), |m| { if let Some(d) = m.text_mut(&k) { d.text = text; } }); }
            }
            3 => { if ui.button(t("Open the area flags")).clicked() { app.mission.tab = Tab::Flags; } }
            _ => { ui.weak(t("This file is copied as it is.")); }
        }
        if ui.button(icons::with(icons::TRASH, &t("Remove the file from the mission"))).clicked() {
            let e = mission_mut(app).map(|m| mission::file_edit(m, &k.clone(), |m| { m.remove(&k); }));
            push(app, &t("Remove file"), e.into_iter().collect());
            sel = None;
        }
    });
    ui.horizontal(|ui| {
        if ui.button(icons::with(icons::FILE_PLUS, &t("Add a file..."))).on_hover_text(t("Copies a file into the mission, e.g. a types file of a mod.")).clicked() {
            if let Some(p) = rfd::FileDialog::new().set_title(t("Add a file...")).pick_file() {
                match std::fs::read(&p) {
                    Ok(b) => {
                        let name = p.file_name().unwrap().to_string_lossy().to_string();
                        let data = mission::read_file(&name, &b);
                        let data = if matches!(data, mission::FileData::Disk) { None } else { Some(data) };
                        if let Some(data) = data {
                            let e = mission_mut(app).map(|m| mission::file_edit(m, &name.clone(), |m| m.put(&name, data)));
                            push(app, &t("Add file"), e.into_iter().collect());
                            sel = Some(name);
                        } else {
                            app.warn(t("Only XML, JSON and .c files can be added."));
                        }
                    }
                    Err(e) => app.error(format!("{}: {e}", p.display())),
                }
            }
        }
    });
    app.mission.file_sel = sel;
}

// ---------------------------------------------------------------- check

fn check_tab(app: &mut App, ui: &mut egui::Ui) {
    let r = rev(app);
    let size = app.doc.as_ref().unwrap().world.meta.size_m();
    if app.mission.issues.as_ref().is_none_or(|(rv, _)| *rv != r) {
        let list = mission(app).map(|m| mission::validate::check(m, Some(size))).unwrap_or_default();
        app.mission.issues = Some((r, list));
    }
    let issues = app.mission.issues.as_ref().unwrap().1.clone();
    use mission::validate::Level;
    let count = |l: Level| issues.iter().filter(|i| i.level == l).count();
    ui.horizontal(|ui| {
        let lv = &mut app.mission.issue_levels;
        ui.checkbox(&mut lv[0], RichText::new(tf("{0} errors", &[&count(Level::Error).to_string()])).color(theme::ERROR));
        ui.checkbox(&mut lv[1], RichText::new(tf("{0} warnings", &[&count(Level::Warning).to_string()])).color(theme::WARN));
        ui.checkbox(&mut lv[2], tf("{0} notes", &[&count(Level::Info).to_string()]));
    });
    if issues.is_empty() {
        ui.label(RichText::new(icons::with(icons::CHECK_CIRCLE, &t("The files of the mission agree with each other."))).color(Color32::from_rgb(110, 210, 120)));
        return;
    }
    // Issues of one kind and file go under one heading.
    let mut groups: Vec<((Level, String, &'static str), Vec<String>)> = Vec::new();
    for i in issues.iter().filter(|i| app.mission.issue_levels[i.level as usize]) {
        let key = (i.level, i.file.clone(), i.text);
        let mut msg = t(i.text);
        for (n, a) in i.args.iter().enumerate() { msg = msg.replace(&format!("{{{n}}}"), a); }
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => v.push(msg),
            None => groups.push((key, vec![msg])),
        }
    }
    egui::ScrollArea::vertical().id_salt("issues").auto_shrink([false, false]).show(ui, |ui| {
        for ((level, file, _), msgs) in groups {
            let color = match level { Level::Error => theme::ERROR, Level::Warning => theme::WARN, Level::Info => theme::TEXT_DIM };
            if msgs.len() == 1 {
                ui.horizontal(|ui| {
                    if ui.link(RichText::new(&file).monospace()).clicked() { app.mission.file_sel = Some(file.clone()); app.mission.tab = Tab::Files; }
                    ui.label(RichText::new(&msgs[0]).color(color));
                });
            } else {
                egui::CollapsingHeader::new(RichText::new(format!("{file}: {} (×{})", msgs[0], msgs.len())).color(color)).id_salt((&file, &msgs[0])).show(ui, |ui| {
                    for m in msgs.iter().take(500) { ui.label(RichText::new(m).small()); }
                });
            }
        }
    });
}

// ---------------------------------------------------------------- scripted checks

/// `mission <command>` of the test scripts.
pub fn script(app: &mut App, arg: &str) {
    let mut it = arg.splitn(2, ' ');
    let (cmd, rest) = (it.next().unwrap_or(""), it.next().unwrap_or("").trim());
    match cmd {
        "new" => new_mission(app),
        "import" => import_from(app, rest.into(), false),
        "adapt" => import_from(app, rest.into(), true),
        "export" => export_to(app, std::path::Path::new(rest), false),
        "window" => app.mission.open = rest != "0",
        "tab" => { if let Some(t) = Tab::from_name(rest) { app.mission.tab = t; app.mission.open = true; } }
        "tool" => {
            app.mission.tool = match rest { "event" => MapTool::EventSpawn, "player" => MapTool::PlayerSpawn, "zone" => MapTool::Zone, "area" => MapTool::Area, "safe" => MapTool::SafePosition, "trigger" => MapTool::Trigger, "flags" => MapTool::Flags, _ => MapTool::Select };
            app.mode = Mode::Mission;
        }
        "event" => app.mission.event = rest.to_string(),
        "flags" => app.mission.show_flags = rest != "0",
        "show" => { if rest == "groups" { app.mission.shown.insert(PlaceKind::MapGroup); } }
        "hide" => app.mission.shown.clear(),
        "own" => app.mission.own_window = rest != "0",
        // As the context menu of the asset browser does.
        "groupprefab" => {
            let id = app.library.prefabs.iter().find(|p| p.name.eq_ignore_ascii_case(rest)).map(|p| p.id).or_else(|| app.library.prefabs.first().map(|p| p.id));
            if let Some(id) = id { group_from_prefab(app, id); } else { eprintln!("no prefabs ({} in the library)", app.library.prefabs.len()); }
        }
        "groupmodels" => {
            let models: Vec<String> = rest.split_whitespace().map(str::to_string).collect();
            group_from_models(app, &models);
        }
        "addmodels" => {
            let mut it = rest.split_whitespace();
            let group = it.next().unwrap_or("").to_string();
            let models: Vec<String> = it.map(str::to_string).collect();
            add_models_to_group(app, &group, &models);
        }
        "groupdump" => {
            if let Some(m) = mission(app) {
                for (_, (name, _, children)) in places::event_groups(m) {
                    eprintln!("GROUP {name}: {}", children.iter().map(|c| format!("{}@{},{}", c.kind, c.x, c.z)).collect::<Vec<_>>().join(" "));
                }
            }
            eprintln!("GHOSTS {} models={} unknown={:?}", app.mission.ghosts.list.len(), app.mission.ghosts.models.len(), app.mission.ghosts.unknown);
        }
        "selgroup" => {
            app.mission.shown.insert(PlaceKind::EventSpawn);
            let found = places_now(app).iter().find(|p| p.sub.as_deref().is_some_and(|g| g.eq_ignore_ascii_case(rest))).map(|p| (p.at.clone(), p.x, p.z));
            if let Some((at, x, z)) = found {
                app.mission.selected = Some(at);
                app.mission.group_sel = Some(rest.to_string());
                app.top_center = [x, z];
                app.top_mpp = 0.12;
            }
        }
        "eventsel" => {
            let ev = mission(app).and_then(|m| m.ce_files("events").into_iter().find_map(|f| {
                let d = m.xml(&f)?;
                d.root.children.iter().position(|n| matches!(n, Node::Element(e) if e.name == "event" && e.attr("name") == Some(rest))).map(|i| (f.clone(), i))
            }));
            app.mission.event_sel = ev;
            app.mission.event_filter = rest.to_string();
            app.mission.tab = Tab::Events;
            app.mission.open = true;
        }
        "groupfromsel" => {
            let mut it = rest.split_whitespace();
            let (name, event) = (it.next().unwrap_or("Group").to_string(), it.next().unwrap_or("StaticMilitaryConvoy").to_string());
            let props = selected_props(app, true);
            if let Some(m) = mission_mut(app) {
                if let Some((g, [x, y, z])) = places::group_from_objects(&name, &props, (2, 3)) {
                    let mut edits = vec![places::put_group(m, g)];
                    edits.extend(places::add_group_spawn(m, &event, &name, x, z, y));
                    push(app, "group", edits);
                    app.mission.group_sel = Some(name);
                }
            }
        }
        "bit" => app.mission.flag_bit = rest.parse().unwrap_or(0),
        "add" => {
            let n: Vec<f32> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
            if n.len() >= 2 { let tool = app.mission.tool; add_place(app, tool, n[0], n[1]); }
        }
        "paint" => {
            let n: Vec<f32> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
            if n.len() >= 2 { paint_flags(app, n[0], n[1], false); finish_flag_stroke(app); }
        }
        "groups" => make_groups(app),
        "filter" => app.mission.type_filter = rest.to_string(),
        "focus" => {
            let rows = type_rows(app);
            if let Some((f, i, _)) = rows.iter().find(|(_, _, r)| r.name.eq_ignore_ascii_case(rest)) {
                app.mission.type_focus = Some((f.clone(), *i));
                app.mission.type_sel = [(f.clone(), *i)].into_iter().collect();
            }
        }
        "dump" => {
            if let Some(m) = mission(app) {
                let p = places::collect(m, &PlaceKind::ALL);
                eprintln!("MISSION {} files={} dirty={} types={} places={}", m.name, m.files.len(), m.files.values().filter(|f| f.dirty).count(), mission::ce::all_types(m).len(), p.len());
                for k in PlaceKind::ALL { eprintln!("MISSION {:?}={}", k, p.iter().filter(|x| x.kind == k).count()); }
                let size = app.doc.as_ref().unwrap().world.meta.size_m();
                let issues = mission::validate::check(m, Some(size));
                eprintln!("MISSION issues={} errors={}", issues.len(), issues.iter().filter(|i| i.level == mission::validate::Level::Error).count());
            } else {
                eprintln!("MISSION none");
            }
        }
        _ => eprintln!("unknown mission command {cmd}"),
    }
}

// ---------------------------------------------------------------- event groups

/// The objects selected in the Select mode as objects of an event group.
fn selected_props(app: &mut App, de: bool) -> Vec<places::GroupObject> {
    let classes = class_index(app);
    let Some(doc) = &app.doc else { return Vec::new() };
    let w = &doc.world;
    let sea = w.meta.sea_level;
    app.selection.iter().filter_map(|id| w.objects.get(*id)).map(|o| {
        let model = &w.objects.models[o.model as usize].path;
        places::GroupObject {
            kind: classes.as_ref().and_then(|c| c.classes_for_model(model, de).into_iter().next()).unwrap_or_else(|| mission::gen::prop_class(model, de)),
            x: o.x,
            z: o.z,
            y: dz_core::objects::ObjectStore::origin_y(o, &w.height) - sea,
            ground: w.height.sample(o.x, o.z) - sea,
            heading: mission::gen::heading(o),
        }
    }).collect()
}

/// The groups the spawns of an event place, one to edit, and groups made from props.
fn event_groups_ui(app: &mut App, ui: &mut egui::Ui, event: &str) {
    ui.label(RichText::new(t("Event groups (cfgeventgroups.xml)")).strong());
    ui.weak(t("Each spawn of a static event places the objects of its group, offset from the spawn along the axes of the map; y is the height over the spawn, without it the object stands on the ground."));
    let Some(m) = mission(app) else { return };
    let spawns: Vec<Place> = places::collect(m, &[PlaceKind::EventSpawn]).into_iter().filter(|p| p.group == event).collect();
    let all = mission::ce::event_group_names(m);
    let mut used: Vec<(String, usize)> = Vec::new();
    for p in &spawns {
        if let Some(g) = &p.sub {
            match used.iter_mut().find(|(n, _)| n == g) {
                Some((_, c)) => *c += 1,
                None => used.push((g.clone(), 1)),
            }
        }
    }
    let without = spawns.iter().filter(|p| p.sub.is_none()).count();
    ui.horizontal_wrapped(|ui| {
        for (g, n) in &used {
            if ui.selectable_label(app.mission.group_sel.as_deref() == Some(g.as_str()), format!("{g} ({n})")).clicked() { app.mission.group_sel = Some(g.clone()); }
        }
    });
    if without > 0 && !used.is_empty() {
        ui.weak(tf("{0} spawns without a group", &[&without.to_string()]));
    }
    row(ui, &t("Group"), |ui| {
        egui::ComboBox::from_id_salt("ev_group_pick").selected_text(app.mission.group_sel.clone().unwrap_or_else(|| t("(none)"))).height(400.0).show_ui(ui, |ui| {
            for g in &all { ui.selectable_value(&mut app.mission.group_sel, Some(g.clone()), g); }
        });
    });
    if let Some(g) = app.mission.group_sel.clone() {
        if ui.button(icons::with(icons::MAP_PIN, &t("Place spawns of this group"))).clicked() {
            app.mission.event = event.to_string();
            app.mission.event_group = g.clone();
            app.mission.tool = MapTool::EventSpawn;
            app.mission.shown.insert(PlaceKind::EventSpawn);
            app.mode = Mode::Mission;
        }
        group_editor(app, ui, &g);
    }
    // Groups from the props placed on the map.
    ui.separator();
    let de = app.mission.group_de;
    let props = selected_props(app, de);
    ui.label(RichText::new(tf("From the selected objects ({0})", &[&props.len().to_string()])).strong());
    ui.weak(t("Place the props on the map, select them in the Select mode and make a group of them. The first selected object stands where the spawn will be."));
    row(ui, &t("Name"), |ui| { ui.add(egui::TextEdit::singleline(&mut app.mission.group_name).hint_text("Abandoned_MyTown")); });
    ui.horizontal(|ui| {
        ui.checkbox(&mut app.mission.group_de, t("Class names with _DE")).on_hover_text(t("The variants of the objects made for dynamic events end in _DE (StaticObj_Wreck_BMP2_DE). Check the names below: they are guessed from the models."));
        ui.label("loot");
        ui.add(egui::DragValue::new(&mut app.mission.group_loot.0).range(0..=20));
        ui.add(egui::DragValue::new(&mut app.mission.group_loot.1).range(0..=20));
    });
    ui.checkbox(&mut app.mission.group_with_spawn, tf("And a spawn of {0} at the first object", &[event]));
    let name = app.mission.group_name.trim().to_string();
    let exists = all.iter().any(|g| g.eq_ignore_ascii_case(&name));
    let ok = !props.is_empty() && !name.is_empty() && !name.contains(' ');
    let make = ui.add_enabled(ok, egui::Button::new(icons::with(icons::PLUS, &if exists { t("Replace the group") } else { t("New group") }))).clicked();
    // Adding to the open group needs a spawn of it to measure from.
    let target = app.mission.group_sel.clone();
    let origin = target.as_ref().and_then(|g| spawns.iter().find(|p| p.sub.as_deref() == Some(g.as_str()))).map(|p| [p.x, p.y.unwrap_or(0.0), p.z]);
    let mut add_to = false;
    if let Some(g) = &target {
        add_to = ui.add_enabled(!props.is_empty() && origin.is_some(), egui::Button::new(tf("Add to {0}", &[g]))).on_hover_text(t("Offsets are measured from the first spawn of the group of this event.")).on_disabled_hover_text(t("Select objects, and place a spawn of the group first.")).clicked();
    }
    if !props.is_empty() {
        egui::CollapsingHeader::new(t("Classes of the selected objects")).id_salt("props_classes").show(ui, |ui| {
            for p in props.iter().take(50) { ui.label(RichText::new(&p.kind).monospace().small()); }
        });
    }
    let loot = app.mission.group_loot;
    if make {
        let with_spawn = app.mission.group_with_spawn;
        let Some(m) = mission_mut(app) else { return };
        if let Some((group, [x, y, z])) = places::group_from_objects(&name, &props, loot) {
            let mut edits = vec![places::put_group(m, group)];
            if with_spawn {
                edits.extend(places::add_group_spawn(m, event, &name, x, z, y));
            }
            push(app, &t("New event group"), edits);
            app.mission.group_sel = Some(name.clone());
            app.info(tf("Group {0} made of {1} objects", &[&name, &props.len().to_string()]));
        }
    }
    if add_to {
        if let (Some(g), Some(o)) = (target, origin) {
            let e = mission_mut(app).and_then(|m| places::add_to_group(m, &g, &props, o, loot));
            push(app, &t("Add to event group"), e.into_iter().collect());
        }
    }
}

/// The objects of a group as a table to change.
fn group_editor(app: &mut App, ui: &mut egui::Ui, name: &str) {
    let Some(file) = mission(app).and_then(|m| m.key("cfgeventgroups.xml")) else { return };
    let found = mission(app).and_then(|m| m.xml(&file)).and_then(|d| d.root.children.iter().enumerate().find_map(|(i, n)| match n {
        Node::Element(e) if e.name == "group" && e.attr("name").is_some_and(|x| x.eq_ignore_ascii_case(name)) => Some((i, e.clone())),
        _ => None,
    }));
    let Some((gi, g)) = found else {
        ui.colored_label(theme::ERROR, tf("cfgeventgroups.xml has no group {0}", &[name]));
        return;
    };
    let mut copy = g.clone();
    let mut remove = None;
    let mut dup = None;
    egui::ScrollArea::horizontal().id_salt(("grp", name)).show(ui, |ui| {
        egui::Grid::new(("grp_grid", name)).num_columns(10).striped(true).spacing(vec2(5.0, 2.0)).show(ui, |ui| {
            for h in ["type", "x", "z", "y", "a", "deloot", "lootmin", "lootmax", "secondary", ""] { ui.label(RichText::new(h).small().strong()); }
            ui.end_row();
            for (ci, n) in copy.children.iter_mut().enumerate() {
                let Node::Element(c) = n else { continue };
                if c.name != "child" { continue; }
                let mut kind = c.attr("type").unwrap_or("").to_string();
                if ui.add(egui::TextEdit::singleline(&mut kind).desired_width(230.0)).changed() { c.set_attr("type", kind); }
                for (a, sp) in [("x", 0.05), ("z", 0.05)] { num_attr(ui, c, a, sp); }
                if c.attr("y").is_some() {
                    ui.horizontal(|ui| {
                        num_attr(ui, c, "y", 0.01);
                        if ui.small_button(icons::X).on_hover_text(t("On the ground")).clicked() { c.remove_attr("y"); }
                    });
                } else if ui.small_button(t("ground")).on_hover_text(t("Give it a height over the spawn")).clicked() {
                    c.set_attr("y", "0");
                }
                if c.attr("a").is_none() { ui.label("-"); } else { num_attr(ui, c, "a", 0.5); }
                let mut de = c.attr("deloot").is_some_and(mission::ce::is_true);
                if ui.checkbox(&mut de, "").on_hover_text(t("Loot of the dynamic event: the object gets loot of the prototype with de of its class.")).changed() { c.set_attr("deloot", if de { "1" } else { "0" }); }
                for a in ["lootmin", "lootmax"] {
                    if c.attr(a).is_none() { ui.label("-"); } else { num_attr(ui, c, a, 0.1); }
                }
                let mut sec = !c.attr("spawnsecondary").is_some_and(|v| v.trim().eq_ignore_ascii_case("false") || v.trim() == "0");
                if ui.checkbox(&mut sec, "").on_hover_text(t("The secondary event (infected) of the event may spawn at this object.")).changed() {
                    if sec { c.remove_attr("spawnsecondary"); } else { c.set_attr("spawnsecondary", "false"); }
                }
                ui.horizontal(|ui| {
                    if ui.small_button(icons::COPY).clicked() { dup = Some(ci); }
                    if ui.small_button(icons::TRASH).clicked() { remove = Some(ci); }
                });
                ui.end_row();
            }
        });
    });
    if let Some(i) = dup { let n = copy.children[i].clone(); copy.children.insert(i + 1, n); }
    if let Some(i) = remove { copy.children.remove(i); }
    if ui.small_button(icons::with(icons::PLUS, &t("Add object"))).clicked() {
        copy.push(Element::new("child").with_attr("type", "StaticObj_Wreck_Decal_Small2_DE").with_attr("spawnsecondary", "false").with_attr("x", "0").with_attr("z", "0").with_attr("a", "0"));
    }
    if copy != g {
        edit_el(app, &t("Edit event group"), &file, &[gi], |e| *e = copy);
    }
}

/// A new event group of models chosen in the asset browser, side by side six metres apart;
/// its spawns are then placed with the Event spawn tool.
pub fn group_from_models(app: &mut App, models: &[String]) {
    if mission(app).is_none() {
        app.warn(t("The project has no mission."));
        return;
    }
    let de = app.mission.group_de;
    let objects: Vec<places::GroupObject> = models.iter().enumerate().map(|(i, m)| places::GroupObject { kind: class_of_model(app, m, de), x: i as f32 * 6.0, z: 0.0, y: 0.0, ground: 0.0, heading: 0.0 }).collect();
    let stem = objects.first().map(|o| o.kind.trim_start_matches("Land_").trim_start_matches("StaticObj_").trim_end_matches("_DE").to_string()).unwrap_or_default();
    let have: BTreeSet<String> = mission(app).map(mission::ce::event_group_names).unwrap_or_default().into_iter().map(|g| g.to_ascii_lowercase()).collect();
    let mut name = format!("Group_{stem}");
    let mut n = 2;
    while have.contains(&name.to_ascii_lowercase()) {
        name = format!("Group_{stem}_{n}");
        n += 1;
    }
    let loot = app.mission.group_loot;
    let Some(m) = mission_mut(app) else { return };
    let Some((group, _)) = places::group_from_objects(&name, &objects, loot) else { return };
    let e = places::put_group(m, group);
    push(app, &t("New event group"), vec![e]);
    open_group(app, &name);
    app.info(tf("Group {0} made of {1} objects; place its spawns with the Event spawn tool.", &[&name, &objects.len().to_string()]));
}

/// Adds models chosen in the asset browser to a group, after its last object.
pub fn add_models_to_group(app: &mut App, group: &str, models: &[String]) {
    let de = app.mission.group_de;
    let kinds: Vec<String> = models.iter().map(|m| class_of_model(app, m, de)).collect();
    let next_x = mission(app).map(places::event_groups).and_then(|g| g.get(&group.to_ascii_lowercase()).map(|(_, _, c)| c.iter().map(|c| c.x).fold(-6.0f32, f32::max) + 6.0)).unwrap_or(0.0);
    let objects: Vec<places::GroupObject> = kinds.into_iter().enumerate().map(|(i, kind)| places::GroupObject { kind, x: next_x + i as f32 * 6.0, z: 0.0, y: 0.0, ground: 0.0, heading: 0.0 }).collect();
    let loot = app.mission.group_loot;
    let e = mission_mut(app).and_then(|m| places::add_to_group(m, group, &objects, [0.0, 0.0, 0.0], loot));
    push(app, &t("Add to event group"), e.into_iter().collect());
    open_group(app, group);
}

/// Shows a group in the Events tab, under an event that places it (or the event of the spawn
/// tool), and readies the spawn tool to place it.
fn open_group(app: &mut App, group: &str) {
    let users: Vec<String> = mission(app).map(|m| places::collect(m, &[PlaceKind::EventSpawn]).into_iter().filter(|p| p.sub.as_deref().is_some_and(|g| g.eq_ignore_ascii_case(group))).map(|p| p.group).collect()).unwrap_or_default();
    let names = mission(app).map(mission::ce::event_names).unwrap_or_default();
    let event = users.first().cloned().or_else(|| (!app.mission.event.is_empty()).then(|| app.mission.event.clone())).or_else(|| names.iter().find(|n| n.eq_ignore_ascii_case("StaticMilitaryConvoy")).cloned()).or_else(|| names.iter().find(|n| n.starts_with("Static")).cloned());
    if let Some(ev) = &event {
        app.mission.event = ev.clone();
        app.mission.event_sel = mission(app).and_then(|m| m.ce_files("events").into_iter().find_map(|f| {
            let d = m.xml(&f)?;
            d.root.children.iter().position(|n| matches!(n, Node::Element(e) if e.name == "event" && e.attr("name") == Some(ev.as_str()))).map(|i| (f.clone(), i))
        }));
        app.mission.event_filter = ev.clone();
    }
    app.mission.group_sel = Some(group.to_string());
    app.mission.event_group = group.to_string();
    app.mission.tool = MapTool::EventSpawn;
    app.mission.shown.insert(PlaceKind::EventSpawn);
    app.mission.tab = Tab::Events;
    app.mission.open = true;
}

/// The objects of a prefab of the library as objects of an event group, around the middle of
/// the prefab, which is where the spawn stands: offsets along the axes of the map, the yaw as
/// the direction, and a height over the spawn for objects above the ground.
fn prefab_objects(app: &mut App, id: u64) -> Option<(String, Vec<places::GroupObject>)> {
    let prefab = app.library.prefabs.iter().find(|p| p.id == id).cloned()?;
    let de = app.mission.group_de;
    let objects = prefab.items.iter().map(|i| places::GroupObject {
        kind: class_of_model(app, &i.model, de),
        x: i.x,
        z: i.z,
        // Over the ground at the middle when fixed, else over the ground under it; either way
        // the height over the spawn, which stands on the ground.
        y: i.elevation,
        ground: 0.0,
        heading: i.yaw,
    }).collect();
    Some((prefab.name, objects))
}

/// A new event group laid out as a prefab; its spawns are then placed with the Event spawn tool.
pub fn group_from_prefab(app: &mut App, id: u64) {
    if mission(app).is_none() {
        app.warn(t("The project has no mission."));
        return;
    }
    let Some((prefab, objects)) = prefab_objects(app, id) else { return };
    if objects.is_empty() {
        return;
    }
    let stem: String = prefab.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect::<String>().trim_matches('_').to_string();
    let stem = if stem.is_empty() { "Prefab".to_string() } else { stem };
    let have: BTreeSet<String> = mission(app).map(mission::ce::event_group_names).unwrap_or_default().into_iter().map(|g| g.to_ascii_lowercase()).collect();
    let mut name = stem.clone();
    let mut n = 2;
    while have.contains(&name.to_ascii_lowercase()) {
        name = format!("{stem}_{n}");
        n += 1;
    }
    let mut group = Element::new("group").with_attr("name", name.clone());
    for c in places::child_elements(&objects, [0.0, 0.0, 0.0], app.mission.group_loot) {
        group.push(c);
    }
    let Some(m) = mission_mut(app) else { return };
    let e = places::put_group(m, group);
    push(app, &t("New event group"), vec![e]);
    open_group(app, &name);
    app.info(tf("Group {0} made of {1} objects; place its spawns with the Event spawn tool.", &[&name, &objects.len().to_string()]));
}

/// Adds the objects of a prefab to a group, around the spawn of the group.
pub fn add_prefab_to_group(app: &mut App, group: &str, id: u64) {
    let Some((_, objects)) = prefab_objects(app, id) else { return };
    let loot = app.mission.group_loot;
    let e = mission_mut(app).and_then(|m| places::add_to_group(m, group, &objects, [0.0, 0.0, 0.0], loot));
    push(app, &t("Add to event group"), e.into_iter().collect());
    open_group(app, group);
}
