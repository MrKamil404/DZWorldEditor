//! Team work (experimental): everyone works on their own copy of the map and the changes are
//! merged at the end of the day through a shared folder; see `dz_core::team`.
use crate::app::{App, Finish};
use crate::i18n::{t, tf};
use crate::icons;
use dz_core::project;
use dz_core::team::{self, Choice, ConflictKey, ConflictKind, Merged, PatchHeader, Prepared, Summary, TeamState};
use dz_core::world::World;
use eframe::egui;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How often the team folder is looked at for new changes.
const SCAN_EVERY: Duration = Duration::from_secs(20);

#[derive(Default)]
pub struct TeamUi {
    pub open: bool,
    /// The project the state below belongs to.
    dir: Option<PathBuf>,
    state: TeamState,
    incoming: Vec<(PathBuf, PatchHeader)>,
    last_scan: Option<Instant>,
    merge: Option<Box<MergeView>>,
    /// A team task runs: the project on disk is being read or written.
    busy: bool,
    /// The outliner shows only what this person made or owns; `None` shows everything.
    pub filter: Option<String>,
    /// People named in this session who have nothing of their own yet.
    extra_people: Vec<String>,
    /// A name being typed in a menu of people.
    new_person: String,
    /// The merge itself runs: the project is about to be replaced, so editing waits.
    merging: bool,
}

struct MergeView {
    prepared: Prepared,
    merged: Merged,
    choices: HashMap<ConflictKey, Choice>,
}

impl TeamUi {
    /// Changes of others waiting to be merged.
    pub fn waiting(&self) -> usize {
        self.incoming.len()
    }
    /// A team task reads or writes the project now; autosave waits for it.
    pub fn busy(&self) -> bool {
        self.busy
    }
    /// Changes of others are being merged into the project; editing waits for it.
    pub fn merging(&self) -> bool {
        self.merging
    }
}

/// Starts a team task in the background.
fn spawn_team(app: &mut App, title: String, merging: bool, f: impl FnOnce() -> anyhow::Result<Finish> + Send + 'static) {
    if app.task.is_some() {
        app.warn(t("Another task is still running"));
        return;
    }
    app.spawn(title, move |_| f());
    app.team.busy = app.task.is_some();
    app.team.merging = merging && app.team.busy;
}

/// The oldest changes of others waiting to be merged.
pub fn first_waiting(app: &App) -> Option<(PathBuf, PatchHeader)> {
    app.team.incoming.first().cloned()
}

fn project_dir(app: &App) -> Option<PathBuf> {
    app.doc.as_ref()?.dir.clone()
}

/// Reads the team state and the waiting changes again.
pub fn refresh(app: &mut App) {
    let dir = project_dir(app);
    app.team.last_scan = Some(Instant::now());
    app.team.dir = dir.clone();
    match (&dir, &app.doc) {
        (Some(d), Some(doc)) => {
            app.team.state = TeamState::load(d);
            app.team.incoming = team::incoming(d, &doc.world.meta.map_id);
        }
        _ => {
            app.team.state = TeamState::default();
            app.team.incoming.clear();
        }
    }
}

/// Keeps the state current: after another project is opened and now and then for new changes.
fn watch(app: &mut App) {
    // A task that ended in an error leaves no one else to say so.
    if app.task.is_none() {
        app.team.busy = false;
        app.team.merging = false;
    }
    let dir = project_dir(app);
    if dir != app.team.dir || app.team.last_scan.is_none_or(|t| t.elapsed() > SCAN_EVERY) {
        refresh(app);
        app.ctx.request_repaint_after(SCAN_EVERY);
    }
    // Who works on this copy: new layers, modifiers, roads and generators are theirs.
    let me = if app.team.state.started() { author(app) } else { String::new() };
    set_me(app, me);
}

/// Sets the person working on the open project, and notes what they made.
fn set_me(app: &mut App, me: String) {
    let Some(doc) = &mut app.doc else { return };
    if doc.world.team_me != me {
        doc.world.team_me = me;
        doc.world.objects.revision += 1;
    }
    doc.world.note_authors();
}

/// Team work is on for the open project.
pub fn active(app: &App) -> bool {
    app.doc.as_ref().is_some_and(|d| !d.world.team_me.is_empty())
}

/// A name in the outliner, with who made it in team work.
pub fn with_author(team: bool, name: &str, author: &str) -> String {
    if team && !author.is_empty() { format!("{name}  · {author}") } else { name.to_string() }
}

/// Whether the outliner shows something made or owned by `who`.
pub fn shown(app: &App, who: &str) -> bool {
    app.team.filter.as_deref().is_none_or(|f| f == who)
}

/// The names the outliner can show the work of: this person first, then the others.
pub fn people(app: &App) -> Vec<String> {
    let Some(doc) = &app.doc else { return Vec::new() };
    let mut out = vec![doc.world.team_me.clone()];
    let others = doc.world.object_layers.iter().filter(|l| !l.deleted).map(|l| l.owner.clone())
        .chain(doc.world.authors.iter().flat_map(|a| a.values().cloned()))
        .chain(app.team.incoming.iter().map(|(_, h)| h.author.clone()))
        .chain(app.team.extra_people.iter().cloned());
    for n in others {
        if !n.is_empty() && !out.contains(&n) {
            out.push(n);
        }
    }
    out
}

/// A menu of the people of the team, with no one and a field for someone new. Gives the one
/// clicked: a name, or empty for no one.
pub fn person_menu(app: &mut App, ui: &mut egui::Ui, current: &str) -> Option<String> {
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
    let people = people(app);
    let me = people.first().cloned().unwrap_or_default();
    let mut pick = None;
    for p in &people {
        let label = if *p == me { tf("Me ({0})", &[p]) } else { p.clone() };
        if ui.add(egui::Button::new(icons::with(icons::USER, &label)).selected(current == p)).clicked() {
            pick = Some(p.clone());
        }
    }
    if ui.add(egui::Button::new(t("No one")).selected(current.is_empty())).on_hover_text(t("Anyone may work on it without unlocking it.")).clicked() {
        pick = Some(String::new());
    }
    ui.separator();
    ui.horizontal(|ui| {
        let edit = ui.add(egui::TextEdit::singleline(&mut app.team.new_person).hint_text(t("Someone else...")).desired_width(130.0));
        let name = app.team.new_person.trim().to_string();
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.add_enabled(!name.is_empty(), egui::Button::new(t("Choose"))).clicked() || enter) && !name.is_empty() {
            if !app.team.extra_people.contains(&name) {
                app.team.extra_people.push(name.clone());
            }
            app.team.new_person.clear();
            pick = Some(name);
        }
    });
    if pick.is_some() {
        ui.close_menu();
    }
    pick
}

/// Makes `name` the author of the thing under `key`, so it can be undone.
pub fn set_author(app: &mut App, key: u64, name: String) {
    let Some(doc) = &mut app.doc else { return };
    if doc.world.author(key) == name {
        return;
    }
    let before = doc.world.set_author(key, &name);
    doc.history.push(t("Author"), dz_core::edit::Change::Authors(before));
}

/// The Author submenu of a modifier, road or generator in the outliner, in team work.
pub fn author_menu(app: &mut App, ui: &mut egui::Ui, key: u64) {
    if !active(app) {
        return;
    }
    let current = app.doc.as_ref().map(|d| d.world.author(key).to_string()).unwrap_or_default();
    let mut pick = None;
    ui.menu_button(icons::with(icons::USER, &t("Author")), |ui| pick = person_menu(app, ui, &current));
    if let Some(p) = pick {
        set_author(app, key, p);
    }
}

/// Moves the selected objects to a layer of `person`, made when they have none; empty gives
/// them to a layer of no one.
pub fn give_selection(app: &mut App, person: String) {
    let Some(doc) = &mut app.doc else { return };
    if app.selection.is_empty() {
        app.warn(t("Select objects first"));
        return;
    }
    let count = app.selection.len();
    let layers = &doc.world.object_layers;
    let found = layers.iter().enumerate().position(|(i, l)| !l.deleted && l.owner == person && (i > 0 || person.is_empty()));
    let (layer, made) = match found {
        Some(i) => (i as u16, false),
        None => {
            let before = doc.world.object_layers.clone();
            let name = if person.is_empty() { t("Shared") } else { person.clone() };
            let n = doc.world.add_object_layer(&name);
            doc.world.object_layers[n as usize].owner = person.clone();
            doc.history.push(t("New layer"), dz_core::edit::Change::ObjectLayers(before));
            (n, true)
        }
    };
    app.layer_action(crate::app::LayerAction::MoveSelection(layer));
    if let Some(doc) = &mut app.doc {
        if made {
            doc.history.group_last(2, t("Give objects"));
        }
        let to = doc.world.object_layers[layer as usize].name.clone();
        app.info(tf("{0} objects moved to the layer {1}", &[&count.to_string(), &to]));
    }
}

/// The choice of whose work the outliner shows.
pub fn filter_ui(app: &mut App, ui: &mut egui::Ui) {
    if !active(app) {
        app.team.filter = None;
        return;
    }
    let people = people(app);
    let me = people[0].clone();
    let name = |n: &Option<String>| match n {
        None => t("Everyone"),
        Some(n) if *n == me => tf("Mine ({0})", &[n]),
        Some(n) => n.clone(),
    };
    ui.horizontal(|ui| {
        ui.label(icons::USERS).on_hover_text(t("Whose work the outliner shows: layers by their owner, modifiers, roads and generators by who made them."));
        egui::ComboBox::from_id_salt("team_filter").selected_text(name(&app.team.filter)).show_ui(ui, |ui| {
            ui.selectable_value(&mut app.team.filter, None, name(&None));
            for p in &people {
                ui.selectable_value(&mut app.team.filter, Some(p.clone()), name(&Some(p.clone())));
            }
        });
    });
}

/// A copy of the world without the pixels of the satellite image, which team work leaves out.
fn lite_copy(app: &mut App) -> Option<World> {
    let doc = app.doc.as_mut()?;
    let image = doc.world.satellite.take();
    let mut copy = doc.world.clone();
    if let Some(s) = &image {
        copy.satellite = Some(dz_core::terrain::RgbImage { width: s.width, height: s.height, data: Vec::new() });
    }
    doc.world.satellite = image;
    Some(copy)
}

/// Saves the project; team work starts from what is on disk.
fn save_first(app: &mut App) -> Option<PathBuf> {
    let Some(dir) = project_dir(app) else {
        app.warn(t("Save the project first"));
        return None;
    };
    crate::mission_ui::flush(app);
    // The objects are always written: a project from before team work has no lasting
    // identities on disk yet, and they must match the ones the shared version has.
    if let Some(doc) = &mut app.doc {
        doc.dirty.objects = true;
    }
    app.save(None).then_some(dir)
}

pub fn start(app: &mut App, folder: String) {
    let Some(dir) = save_first(app) else { return };
    let Some(copy) = lite_copy(app) else { return };
    spawn_team(app, t("Setting up team work"), false, move || {
        team::start(&dir, &copy, &folder)?;
        Ok(Box::new(|app: &mut App| {
            refresh(app);
            app.info(t("Team work is set up. Send a full copy to the others."));
        }) as Finish)
    });
}

fn author(app: &App) -> String {
    let n = app.settings.team_name.trim();
    if n.is_empty() { std::env::var("USERNAME").unwrap_or_else(|_| "?".into()) } else { n.to_string() }
}

pub fn send(app: &mut App) {
    let Some(dir) = save_first(app) else { return };
    let Some(copy) = lite_copy(app) else { return };
    let (who, time) = (author(app), crate::settings::timestamp());
    spawn_team(app, t("Sending my changes"), false, move || {
        let (file, summary) = team::send(&dir, &copy, &who, &time)?;
        Ok(Box::new(move |app: &mut App| {
            refresh(app);
            let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if summary.is_empty() {
                app.info(tf("Sent {0}: no changes since the last shared version", &[&name]));
            } else {
                app.info(tf("Sent {0}: {1}", &[&name, &summary_text(&summary)]));
            }
        }) as Finish)
    });
}

fn send_full(app: &mut App) {
    let Some(dir) = save_first(app) else { return };
    let map = app.doc.as_ref().map(|d| d.world.meta.map_id.clone()).unwrap_or_default();
    let time = crate::settings::timestamp();
    spawn_team(app, t("Copying the project to the team folder"), false, move || {
        let to = team::send_full(&dir, &map, &time)?;
        Ok(Box::new(move |app: &mut App| {
            app.info(tf("Full copy written to {0}", &[&to.display().to_string()]));
        }) as Finish)
    });
}

fn take_full(app: &mut App) {
    let start = Path::new(&app.team.state.folder).join("full");
    let mut pick = rfd::FileDialog::new().set_title(t("Choose the full copy in the team folder"));
    if start.is_dir() {
        pick = pick.set_directory(&start);
    }
    let Some(from) = pick.pick_folder() else { return };
    if !project::is_project(&from) {
        app.warn(t("This folder holds no project"));
        return;
    }
    let Some(parent) = rfd::FileDialog::new().set_title(t("Choose where to keep your copy")).pick_folder() else { return };
    let to = parent.join(from.file_name().unwrap_or_default());
    app.spawn(t("Copying the project"), move |_| {
        team::take_full(&from, &to)?;
        let w = project::load(&to)?;
        Ok(Box::new(move |app: &mut App| {
            app.open_world(w, Some(to));
            refresh(app);
            app.team.open = true;
            app.info(t("Your copy is ready. The team folder is the one it came from."));
        }) as Finish)
    });
}

pub fn begin_merge(app: &mut App, file: PathBuf) {
    let Some(dir) = save_first(app) else { return };
    spawn_team(app, t("Reading the changes"), false, move || {
        let (prepared, merged) = team::prepare(&dir, &file)?;
        Ok(Box::new(move |app: &mut App| {
            app.team.merge = Some(Box::new(MergeView { prepared, merged, choices: HashMap::new() }));
        }) as Finish)
    });
}

pub fn finish_merge(app: &mut App) {
    let Some(view) = app.team.merge.take() else { return };
    let Some(dir) = save_first(app) else {
        app.team.merge = Some(view);
        return;
    };
    let MergeView { prepared, choices, .. } = *view;
    let (who, time) = (prepared.header.author.clone(), crate::settings::timestamp());
    spawn_team(app, t("Merging the changes"), true, move || {
        let merged = team::finish(&dir, prepared, &choices, &time)?;
        let w = project::load(&dir)?;
        Ok(Box::new(move |app: &mut App| {
            // The view stays where it was.
            let (camera, center, mpp) = (app.camera, app.top_center, app.top_mpp);
            app.open_world(w, Some(dir));
            (app.camera, app.top_center, app.top_mpp) = (camera, center, mpp);
            refresh(app);
            let mut text = tf("Merged the changes of {0}: {1}", &[&who, &summary_text(&merged.taken)]);
            if !merged.conflicts.is_empty() {
                text += &format!(" · {}", tf("{0} conflicts decided", &[&merged.conflicts.len().to_string()]));
            }
            app.info(text);
            if !merged.skipped.is_empty() {
                let names: Vec<String> = merged.skipped.iter().map(|n| part_name(n)).collect();
                app.warn(tf("Not merged: {0}", &[&names.join(", ")]));
            }
        }) as Finish)
    });
}

fn part_name(name: &str) -> String {
    t(match name {
        "surfaces" => "Surface layers",
        "shapes" => "Shapes",
        "keypoints" => "Key points",
        "holes" => "Terrain holes",
        "road_parts" => "Road catalog",
        "mask_modifiers" => "Mask modifiers",
        "modifiers" => "Terrain modifiers",
        "routes" => "Roads",
        "generators" => "Generators",
        "satellite" => "Satellite image",
        "sat_modifiers" => "Satellite modifiers",
        "satellite_size" => "Satellite image of another size (send a full copy)",
        "mission" => "Mission",
        "layer_owners" => "Owners of layers",
        "authors" => "Authors",
        "map_settings" => "Map settings",
        "map_id" => "Map class name or terrain size (send a full copy)",
        other => other,
    })
}

pub fn summary_text(s: &Summary) -> String {
    let mut parts = Vec::new();
    if s.height_tiles > 0 {
        parts.push(tf("terrain in {0} places", &[&s.height_tiles.to_string()]));
    }
    if s.mask_tiles > 0 {
        parts.push(tf("surfaces in {0} places", &[&s.mask_tiles.to_string()]));
    }
    if s.satellite_tiles > 0 {
        parts.push(tf("satellite image in {0} places", &[&s.satellite_tiles.to_string()]));
    }
    if s.objects_added + s.objects_changed + s.objects_removed > 0 {
        parts.push(tf("objects: +{0} ~{1} -{2}", &[&s.objects_added.to_string(), &s.objects_changed.to_string(), &s.objects_removed.to_string()]));
    }
    for o in &s.other {
        parts.push(part_name(o));
    }
    if parts.is_empty() { t("nothing") } else { parts.join(", ") }
}

fn show_at(app: &mut App, p: [f32; 2]) {
    let Some(doc) = &app.doc else { return };
    let y = doc.world.height.sample(p[0], p[1]);
    let f = app.camera.forward();
    app.camera.pos = [p[0] - f[0] * 120.0, (y - f[1] * 120.0).max(y + 20.0), p[1] - f[2] * 120.0];
    app.top_center = p;
    app.top_mpp = app.top_mpp.min(1.0);
}

/// The Team menu of the menu bar.
pub fn menu(app: &mut App, ui: &mut egui::Ui) {
    let n = app.team.waiting();
    let title = if n > 0 { format!("{} ({n})", t("Team")) } else { t("Team") };
    ui.menu_button(title, |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        let ready = app.doc.is_some() && app.team.state.started();
        if ui.button(icons::with(icons::USERS, &t("Team work..."))).clicked() {
            app.team.open = true;
            refresh(app);
            ui.close_menu();
        }
        ui.separator();
        if ui.add_enabled(ready, egui::Button::new(icons::with(icons::UPLOAD_SIMPLE, &t("Send my changes")))).clicked() {
            ui.close_menu();
            send(app);
        }
        ui.add_enabled_ui(active(app) && !app.selection.is_empty(), |ui| {
            ui.menu_button(icons::with(icons::HAND_ARROW_UP, &t("Give the selected objects")), |ui| {
                if let Some(p) = person_menu(app, ui, "") {
                    give_selection(app, p);
                }
            }).response.on_hover_text(t("Moves them to a layer of the one you choose, made when they have none. Select an area of the map to give it."));
        });
        let label = if n > 0 { tf("Merge changes of others ({0})", &[&n.to_string()]) } else { t("Merge changes of others") };
        if ui.add_enabled(ready && n > 0, egui::Button::new(icons::with(icons::GIT_MERGE, &label))).clicked() {
            ui.close_menu();
            let first = app.team.incoming[0].0.clone();
            begin_merge(app, first);
        }
    });
}

pub fn window(app: &mut App, ctx: &egui::Context) {
    if !app.settings.experimental {
        app.team.open = false;
        app.team.merge = None;
        set_me(app, String::new());
        return;
    }
    watch(app);
    // While the merge writes the project, what would be done in the editor would be lost.
    if app.team.merging {
        egui::Modal::new(egui::Id::new("team_merging")).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(t("Merging the changes"));
            });
            ui.weak(t("Editing waits until the merge is done."));
        });
    }
    if app.team.open {
        let mut open = true;
        egui::Window::new(icons::with(icons::USERS, &t("Team work"))).open(&mut open).default_width(460.0).show(ctx, |ui| team_window(app, ui));
        app.team.open = open;
    }
    if app.team.merge.is_some() {
        merge_window(app, ctx);
    }
}

fn team_window(app: &mut App, ui: &mut egui::Ui) {
    ui.label(t("Everyone works on their own copy of the map. At the end of the day one person sends their changes, the other merges them and sends the result back."));
    ui.add_space(6.0);
    egui::Grid::new("team_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label(t("Your name"));
        if ui.text_edit_singleline(&mut app.settings.team_name).lost_focus() {
            app.settings.save();
        }
        ui.end_row();
        ui.label(t("Team folder"));
        ui.horizontal(|ui| {
            let folder = if app.team.state.folder.is_empty() { t("(none)") } else { app.team.state.folder.clone() };
            ui.label(folder).on_hover_text(t("A folder that Dropbox, Google Drive, OneDrive or Syncthing keeps the same on every computer. Keep the project itself outside it."));
            if ui.button(t("Choose...")).clicked() {
                if let Some(f) = rfd::FileDialog::new().set_title(t("Choose the team folder")).pick_folder() {
                    app.team.state.folder = f.display().to_string();
                    if let Some(d) = project_dir(app).filter(|_| app.team.state.started()) {
                        if let Err(e) = app.team.state.save(&d) {
                            app.error(format!("{e:#}"));
                        }
                    }
                    refresh_keep_folder(app);
                }
            }
        });
        ui.end_row();
    });
    ui.separator();
    let Some(dir) = project_dir(app) else {
        ui.label(t("Save the project first"));
        ui.separator();
        take_full_button(app, ui);
        return;
    };
    let busy = app.task.is_some();
    if !app.team.state.started() {
        ui.label(t("This project is not shared yet. Starting remembers its current state as the version everyone starts from."));
        let folder = app.team.state.folder.clone();
        if ui.add_enabled(!folder.is_empty() && !busy, egui::Button::new(icons::with(icons::USERS, &t("Start team work")))).clicked() {
            start(app, folder);
        }
        ui.separator();
        take_full_button(app, ui);
        return;
    }
    ui.horizontal(|ui| {
        if ui.add_enabled(!busy, egui::Button::new(icons::with(icons::UPLOAD_SIMPLE, &t("Send my changes")))).on_hover_text(t("Writes what you changed since the last shared version to the team folder.")).clicked() {
            send(app);
        }
        if ui.add_enabled(!busy, egui::Button::new(icons::with(icons::FOLDER_SIMPLE_PLUS, &t("Send a full copy")))).on_hover_text(t("Copies the whole project to the team folder, for someone who joins.")).clicked() {
            send_full(app);
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.strong(t("Changes of others"));
        if ui.small_button(icons::ARROWS_CLOCKWISE).on_hover_text(t("Look again")).clicked() {
            refresh(app);
        }
    });
    if app.team.incoming.is_empty() {
        ui.weak(t("Nothing new in the team folder."));
    }
    let mut merge = None;
    for (file, h) in &app.team.incoming {
        ui.horizontal(|ui| {
            if ui.add_enabled(!busy && app.team.merge.is_none(), egui::Button::new(icons::with(icons::GIT_MERGE, &t("Merge")))).clicked() {
                merge = Some(file.clone());
            }
            ui.vertical(|ui| {
                ui.label(format!("{} · {}", h.author, h.time.replace('_', " ")));
                ui.weak(summary_text(&h.summary));
            });
        });
    }
    if let Some(f) = merge {
        begin_merge(app, f);
    }
    ui.separator();
    ui.horizontal(|ui| {
        ui.weak(t("Before every merge the project is copied to .team/before-merge."));
        if ui.small_button(t("Open")).clicked() {
            let p = dir.join(team::TEAM_DIR).join("before-merge");
            let _ = std::fs::create_dir_all(&p);
            app.open_folder(&p);
        }
    });
    take_full_button(app, ui);
}

/// After choosing a folder before team work starts, the choice is kept until it does.
fn refresh_keep_folder(app: &mut App) {
    let folder = app.team.state.folder.clone();
    refresh(app);
    if !app.team.state.started() {
        app.team.state.folder = folder;
    }
}

fn take_full_button(app: &mut App, ui: &mut egui::Ui) {
    if ui.add_enabled(app.task.is_none(), egui::Button::new(icons::with(icons::DOWNLOAD_SIMPLE, &t("Take a full copy...")))).on_hover_text(t("Copies a project someone sent to the team folder to your computer and opens it.")).clicked() {
        take_full(app);
    }
}

fn conflict_text(c: &team::Conflict, who: &str) -> String {
    let kind = match c.kind {
        ConflictKind::Terrain => t("Both of you changed the terrain here"),
        ConflictKind::Surface => t("Both of you painted different surfaces here"),
        ConflictKind::Satellite => t("Both of you changed the satellite image here"),
        ConflictKind::Lines => t("Both of you changed the same lines"),
        ConflictKind::Changed if matches!(c.key, ConflictKey::ModTile(..)) => t("Both of you changed this part of it"),
        ConflictKind::Changed => t("Both of you changed it differently"),
        ConflictKind::DeletedHere => tf("You deleted it, {0} changed it", &[who]),
        ConflictKind::DeletedThere => tf("{0} deleted it, you changed it", &[who]),
    };
    let what = match c.key {
        ConflictKey::Element(list, _) | ConflictKey::ModTile(list, ..) => format!("{}: {}", part_name(list), c.what),
        ConflictKey::Part(p) => part_name(p),
        ConflictKey::Mission(_) => format!("{}: {}", t("Mission"), c.what),
        ConflictKey::Setting(_) => format!("{}: {}", t("Map settings"), c.what),
        _ => c.what.rsplit('\\').next().unwrap_or_default().to_string(),
    };
    if what.trim().is_empty() || what.ends_with(": ") { kind } else { format!("{what} — {kind}") }
}

fn merge_window(app: &mut App, ctx: &egui::Context) {
    let mut open = true;
    let mut cancel = false;
    let mut go = false;
    let mut show = None;
    let busy = app.task.is_some();
    {
        let Some(view) = app.team.merge.as_mut() else { return };
        let h = view.prepared.header.clone();
        egui::Window::new(icons::with(icons::GIT_MERGE, &tf("Merge the changes of {0}", &[&h.author]))).open(&mut open).default_width(520.0).show(ctx, |ui| {
            ui.label(format!("{} · {}", h.author, h.time.replace('_', " ")));
            ui.label(tf("Brings: {0}", &[&summary_text(&h.summary)]));
            if !h.summary.not_carried.is_empty() {
                let names: Vec<String> = h.summary.not_carried.iter().map(|n| part_name(n)).collect();
                ui.colored_label(ui.visuals().warn_fg_color, tf("Changed but not carried over: {0}", &[&names.join(", ")]));
            }
            ui.separator();
            let conflicts = &view.merged.conflicts;
            if conflicts.is_empty() {
                ui.label(t("No conflicts: everything can be merged as it is."));
            } else {
                ui.label(tf("{0} places where both of you changed the same thing. Choose what stays:", &[&conflicts.len().to_string()]));
                ui.horizontal(|ui| {
                    if ui.small_button(t("All mine")).clicked() {
                        for c in conflicts { view.choices.insert(c.key, Choice::Mine); }
                    }
                    if ui.small_button(t("All theirs")).clicked() {
                        for c in conflicts { view.choices.insert(c.key, Choice::Theirs); }
                    }
                    if ui.small_button(t("As suggested")).clicked() {
                        view.choices.clear();
                    }
                });
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for c in conflicts {
                        ui.horizontal(|ui| {
                            let enabled = c.at.is_some();
                            if ui.add_enabled(enabled, egui::Button::new(icons::CROSSHAIR).small()).on_hover_text(t("Show on the map")).clicked() {
                                show = c.at;
                            }
                            ui.label(conflict_text(c, &h.author));
                        });
                        ui.horizontal(|ui| {
                            ui.add_space(28.0);
                            let cur = view.choices.get(&c.key).copied().unwrap_or(c.default);
                            let mut pick = cur;
                            ui.radio_value(&mut pick, Choice::Mine, t("Keep mine"));
                            ui.radio_value(&mut pick, Choice::Theirs, t("Take theirs"));
                            if c.kind == ConflictKind::Terrain {
                                ui.radio_value(&mut pick, Choice::Both, t("Add both")).on_hover_text(t("Both height changes are added together."));
                            }
                            if pick != cur {
                                view.choices.insert(c.key, pick);
                            }
                        });
                    }
                });
            }
            ui.separator();
            ui.weak(t("Your work is saved first and the project is copied to .team/before-merge. Undo starts anew after merging."));
            ui.horizontal(|ui| {
                if ui.add_enabled(!busy, egui::Button::new(icons::with(icons::GIT_MERGE, &t("Merge")))).clicked() {
                    go = true;
                }
                if ui.button(t("Cancel")).clicked() {
                    cancel = true;
                }
            });
        });
    }
    if let Some(p) = show {
        show_at(app, p);
    }
    if go {
        finish_merge(app);
    } else if !open || cancel {
        app.team.merge = None;
    }
}
