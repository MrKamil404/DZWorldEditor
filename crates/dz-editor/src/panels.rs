//! Window layout modelled on the Unreal editor: menu and toolbar on top, modes on the left,
//! outliner and details on the right, content browser and log at the bottom.
use crate::app::*;
use crate::i18n::{t, tf};
use crate::icons;
use crate::keys::{Action, Hold};
use crate::theme::{self, row, section, tool_button};
use crate::viewport;
use dz_core::edit::SculptTool;
use dz_core::objects::{FLAG_ABSOLUTE, FLAG_UNDERGROUND};
use dz_core::world::{ModMode, ModShape, SurfaceLayer};
use eframe::egui::{self, vec2, Color32, RichText};

pub fn new_project_dialog() -> Dialog {
    Dialog::NewProject(NewProject { name: "NewMap".into(), preset: 4, base: 60.0, folder: String::new() })
}

pub fn ui(app: &mut App, ctx: &egui::Context) {
    guard_close(app, ctx);
    crate::library_ui::handle_file_drop(app, ctx);
    if app.lib_dirty && !ctx.input(|i| i.pointer.any_down()) {
        app.lib_dirty = false;
        if let Err(e) = app.library.save(&crate::library_ui::library_path()) { app.error(format!("{e:#}")); }
    }
    menu_bar(app, ctx);
    toolbar(app, ctx);
    status_bar(app, ctx);
    if app.doc.is_some() {
        egui::SidePanel::left("modes").default_width(300.0).width_range(240.0..=460.0).show(ctx, |ui| modes_panel(app, ui));
        egui::SidePanel::right("right").default_width(320.0).width_range(240.0..=520.0).show(ctx, |ui| right_panel(app, ui));
    }
    egui::TopBottomPanel::bottom("bottom").resizable(true).default_height(230.0).height_range(90.0..=600.0).show(ctx, |ui| bottom_panel(app, ui));
    egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::DARK)).show(ctx, |ui| viewport::central(app, ui));
    dialogs(app, ctx);
    app.updater.window(ctx);
    crate::asset_browser::drag_ghost(app, ctx);
}

/// What the viewports draw; shared by the View menu and the viewport toolbar.
pub fn visibility_toggles(app: &mut App, ui: &mut egui::Ui) {
    let keys = app.settings.keys.clone();
    ui.checkbox(&mut app.show_objects, t("Objects")).on_hover_text(keys.tip(Action::ToggleObjects));
    ui.checkbox(&mut app.show_grass, t("Grass")).on_hover_text(keys.tip(Action::ToggleGrass));
    ui.add_enabled(app.show_grass, egui::Checkbox::new(&mut app.grass_under_objects, t("Grass under objects")));
    ui.checkbox(&mut app.show_shadows, t("Shadows"));
    ui.checkbox(&mut app.show_water, t("Water"));
    ui.checkbox(&mut app.show_lines, t("Roads and lines")).on_hover_text(keys.tip(Action::ToggleShapes));
    ui.checkbox(&mut app.show_shapes, t("Shapes")).on_hover_text(keys.tip(Action::ToggleShapes));
    ui.checkbox(&mut app.show_keypoints, t("Key points"));
    ui.checkbox(&mut app.show_contours, t("Contour lines"));
    if app.show_contours {
        row(ui, &t("Interval"), |ui| { ui.add(egui::DragValue::new(&mut app.contour_interval).speed(0.5).range(1.0..=200.0).suffix(" m")); });
    }
    ui.checkbox(&mut app.camera_under, t("Camera below the terrain")).on_hover_text(keys.tip(Action::CameraBelow));
}

fn guard_close(app: &mut App, ctx: &egui::Context) {
    if ctx.input(|i| i.viewport().close_requested()) && !app.close_confirmed && app.autoscript.is_none() && app.doc.as_ref().is_some_and(|d| d.history.dirty()) {
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        app.dialog = Dialog::Confirm { text: t("The project has unsaved changes. Close without saving?"), action: |a| {
            a.close_confirmed = true;
            a.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }};
    }
}

fn menu_bar(app: &mut App, ctx: &egui::Context) {
    egui::TopBottomPanel::top("menu").show(ctx, |ui| {
        egui::menu::bar(ui, |ui| {
            let has = app.doc.is_some();
            let keys = app.settings.keys.clone();
            ui.menu_button(t("File"), |ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); 
                if ui.add(egui::Button::new(icons::with(icons::FILE_PLUS, &t("New project..."))).shortcut_text(keys.text(Action::NewProject))).clicked() { app.dialog = new_project_dialog(); ui.close_menu(); }
                if ui.add(egui::Button::new(icons::with(icons::FOLDER_OPEN, &t("Open project..."))).shortcut_text(keys.text(Action::Open))).clicked() { ui.close_menu(); app.pick_and_open(); }
                ui.menu_button(icons::with(icons::CLOCK, &t("Recent projects")), |ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); 
                    for d in app.settings.recent.clone() {
                        if ui.button(d.display().to_string()).clicked() { ui.close_menu(); app.open_project(d); }
                    }
                });
                ui.separator();
                if ui.add_enabled(has, egui::Button::new(icons::with(icons::FLOPPY_DISK, &t("Save"))).shortcut_text(keys.text(Action::Save))).clicked() { ui.close_menu(); app.save(None); }
                if ui.add_enabled(has, egui::Button::new(icons::with(icons::FLOPPY_DISK_BACK, &t("Save as..."))).shortcut_text(keys.text(Action::SaveAs))).clicked() { ui.close_menu(); app.save_as(); }
                ui.separator();
                ui.menu_button(icons::with(icons::DOWNLOAD_SIMPLE, &t("Import")), |ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); 
                    if ui.button(t("Terrain Builder or built map folder...")).clicked() { ui.close_menu(); app.import_folder(); }
                    if ui.button(t("WRP world...")).clicked() { ui.close_menu(); app.import_wrp(); }
                    ui.separator();
                    ui.add_enabled_ui(has, |ui| {
                        if ui.button(t("Heightmap (ASC)...")).clicked() { ui.close_menu(); app.import_heightmap(); }
                        if ui.button(t("Satellite image...")).clicked() { ui.close_menu(); app.import_satellite(); }
                        if ui.button(t("Mask image...")).clicked() { ui.close_menu(); app.import_mask(); }
                        if ui.button(t("layers.cfg...")).clicked() { ui.close_menu(); app.import_layers_cfg(); }
                        if ui.button(t("Terrain Builder layers (TV4L)...")).clicked() { ui.close_menu(); app.import_tv4l(); }
                        if ui.button(t("Terrain Builder objects (TXT)...")).clicked() { ui.close_menu(); app.import_objects_txt(); }
                        if ui.button(t("DayZForests project...")).clicked() { ui.close_menu(); app.import_forest(); }
                        if ui.button(t("Road Studio project...")).clicked() { ui.close_menu(); app.import_road_project(); }
                        if ui.button(t("Roads from TV4P...")).clicked() { ui.close_menu(); app.import_tv4p(); }
                        if ui.button(t("Shapes (SHP, TV4S)...")).clicked() { ui.close_menu(); app.import_shapes(); }
                    });
                });
                ui.add_enabled_ui(has, |ui| {
                    ui.menu_button(icons::with(icons::EXPORT, &t("Export")), |ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); crate::exporting::menu(app, ui); });
                });
                ui.separator();
                if ui.button(icons::with(icons::SLIDERS_HORIZONTAL, &t("Project settings..."))).clicked() { app.dialog = Dialog::ProjectSettings; ui.close_menu(); }
                if ui.button(icons::with(icons::SIGN_OUT, &t("Exit"))).clicked() { ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
            });
            ui.menu_button(t("Edit"), |ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); 
                let (u, r) = app.doc.as_ref().map(|d| (d.history.undo_label().map(str::to_string), d.history.redo_label().map(str::to_string))).unwrap_or((None, None));
                if ui.add_enabled(u.is_some(), egui::Button::new(icons::with(icons::ARROW_COUNTER_CLOCKWISE, &format!("{} {}", t("Undo"), u.unwrap_or_default()))).shortcut_text(keys.text(Action::Undo))).clicked() { app.undo(); ui.close_menu(); }
                if ui.add_enabled(r.is_some(), egui::Button::new(icons::with(icons::ARROW_CLOCKWISE, &format!("{} {}", t("Redo"), r.unwrap_or_default()))).shortcut_text(keys.text(Action::Redo))).clicked() { app.redo(); ui.close_menu(); }
                ui.separator();
                if ui.add(egui::Button::new(icons::with(icons::COPY, &t("Duplicate"))).shortcut_text(keys.text(Action::Duplicate))).clicked() { app.duplicate_selection(); ui.close_menu(); }
                if ui.add(egui::Button::new(icons::with(icons::TRASH, &t("Delete"))).shortcut_text(keys.text(Action::Delete))).clicked() { app.delete_selection(); ui.close_menu(); }
                ui.separator();
                if ui.add(egui::Button::new(icons::with(icons::GEAR, &t("Settings..."))).shortcut_text(keys.text(Action::Settings))).clicked() { app.dialog = Dialog::Settings; ui.close_menu(); }
                if ui.add(egui::Button::new(icons::with(icons::KEYBOARD, &t("Keyboard shortcuts..."))).shortcut_text(keys.text(Action::Shortcuts))).clicked() { app.dialog = Dialog::Shortcuts; ui.close_menu(); }
            });
            ui.menu_button(t("View"), |ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); 
                visibility_toggles(app, ui);
                ui.separator();
                ui.checkbox(&mut app.show_land_grid, t("Land grid"));
                ui.checkbox(&mut app.show_tile_grid, t("Texture tiles"));
            });
            ui.menu_button(t("Build"), |ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); 
                ui.add_enabled_ui(has, |ui| {
                    if ui.button(t("Export files for the PBO")).clicked() { ui.close_menu(); app.export_map(false); }
                    if ui.button(t("Export with Terrain Builder sources")).clicked() { ui.close_menu(); app.export_map(true); }
                    if ui.add(egui::Button::new(icons::with(icons::HAMMER, &t("Build PBO"))).shortcut_text(keys.text(Action::Build))).clicked() { ui.close_menu(); app.build(false); }
                    if ui.add(egui::Button::new(icons::with(icons::PLAY, &t("Build and play"))).shortcut_text(keys.text(Action::Play))).clicked() { ui.close_menu(); app.build(true); }
                    ui.separator();
                    if ui.button(t("Generate navmesh...")).clicked() { ui.close_menu(); app.generate_navmesh(); }
                });
            });
            ui.menu_button(t("Language"), |ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); 
                let cur = crate::i18n::current();
                for (code, name) in crate::i18n::languages() {
                    if ui.selectable_label(cur == code, name).clicked() { app.sync_language(&code); ui.close_menu(); }
                }
                ui.separator();
                if ui.button(t("Open language folder")).clicked() {
                    let dir = crate::settings::data_dir().join("lang");
                    let _ = std::fs::create_dir_all(&dir);
                    app.open_folder(&dir);
                    ui.close_menu();
                }
            });
            ui.menu_button(t("Help"), |ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); 
                if ui.add(egui::Button::new(icons::with(icons::KEYBOARD, &t("Keyboard shortcuts..."))).shortcut_text(keys.text(Action::Shortcuts))).clicked() { app.dialog = Dialog::Shortcuts; ui.close_menu(); }
                if ui.button(icons::with(icons::INFO, &t("About"))).clicked() { app.dialog = Dialog::About; ui.close_menu(); }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                app.updater.bar(ui);
                coffee_button(ui);
            });
        });
    });
}

/// Where the project lives, and where it can be supported.
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
pub const BUY_ME_A_COFFEE: &str = "https://buymeacoffee.com/mrkamil404";

/// A small button that opens the Buy Me a Coffee page.
fn coffee_button(ui: &mut egui::Ui) {
    let text = egui::RichText::new(icons::with(icons::COFFEE, &t("Buy me a coffee"))).color(egui::Color32::from_rgb(40, 30, 10));
    let b = egui::Button::new(text).fill(egui::Color32::from_rgb(255, 221, 0)).small();
    if ui.add(b).on_hover_text(BUY_ME_A_COFFEE).clicked() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(BUY_ME_A_COFFEE));
    }
}

/// An icon button of the toolbar, lit when `active`, with the action's name and key as tip.
fn icon_tool(ui: &mut egui::Ui, active: bool, icon: &str, label: Option<&str>, tip: String) -> egui::Response {
    let text = match label { Some(l) => format!("{icon} {l}"), None => icon.to_string() };
    let r = ui.add(egui::Button::new(RichText::new(text).size(14.0).color(if active { Color32::WHITE } else { theme::TEXT })).fill(if active { theme::ACCENT } else { Color32::from_rgb(56, 56, 56) }).min_size(vec2(26.0, 24.0)));
    r.on_hover_text(tip)
}

/// The modes with their icons and keys.
pub const MODES: [(Mode, &str, &str, Action); 8] = [
    (Mode::Select, icons::CURSOR, "Select", Action::ModeSelect),
    (Mode::Landscape, icons::MOUNTAINS, "Sculpt", Action::ModeSculpt),
    (Mode::Paint, icons::PAINT_BRUSH, "Paint", Action::ModePaint),
    (Mode::Place, icons::TREE_EVERGREEN, "Place", Action::ModePlace),
    (Mode::Lines, icons::ROAD_HORIZON, "Roads", Action::ModeRoads),
    (Mode::Shapes, icons::POLYGON, "Shapes", Action::ModeShapes),
    (Mode::KeyPoints, icons::MAP_PIN, "Key points", Action::ModeKeyPoints),
    (Mode::Satellite, icons::GLOBE_HEMISPHERE_WEST, "Satellite", Action::ModeSatellite),
];

fn toolbar(app: &mut App, ctx: &egui::Context) {
    egui::TopBottomPanel::top("toolbar").frame(egui::Frame::new().fill(theme::PANEL).inner_margin(egui::Margin::symmetric(8, 4))).show(ctx, |ui| {
        ui.horizontal(|ui| {
            let keys = app.settings.keys.clone();
            let has = app.doc.is_some();
            ui.add_enabled_ui(has, |ui| {
                if icon_tool(ui, false, icons::FLOPPY_DISK, None, keys.tip(Action::Save)).clicked() { app.save(None); }
                let (u, r) = app.doc.as_ref().map(|d| (d.history.undo_label().map(str::to_string), d.history.redo_label().map(str::to_string))).unwrap_or((None, None));
                let undo = format!("{}: {}", keys.tip(Action::Undo), u.clone().unwrap_or_default());
                if ui.add_enabled_ui(u.is_some(), |ui| icon_tool(ui, false, icons::ARROW_COUNTER_CLOCKWISE, None, undo)).inner.clicked() { app.undo(); }
                let redo = format!("{}: {}", keys.tip(Action::Redo), r.clone().unwrap_or_default());
                if ui.add_enabled_ui(r.is_some(), |ui| icon_tool(ui, false, icons::ARROW_CLOCKWISE, None, redo)).inner.clicked() { app.redo(); }
            });
            ui.separator();
            // The modes show their names while there is room for them.
            let named = ui.available_width() > 1100.0;
            for (m, icon, name, action) in MODES {
                if icon_tool(ui, app.mode == m, icon, named.then(|| t(name)).as_deref(), keys.tip(action)).clicked() { app.mode = m; }
            }
            ui.separator();
            ui.add_enabled_ui(app.mode == Mode::Select, |ui| {
                for (g, icon, action) in [(Gizmo::Move, icons::ARROWS_OUT_CARDINAL, Action::GizmoMove), (Gizmo::Rotate, icons::ARROWS_CLOCKWISE, Action::GizmoRotate), (Gizmo::Scale, icons::ARROWS_OUT, Action::GizmoScale)] {
                    if icon_tool(ui, app.gizmo == g, icon, None, keys.tip(action)).clicked() { app.gizmo = g; }
                }
                // World or local axes for moving and turning.
                let (icon, name) = if app.gizmo_local { (icons::CUBE_FOCUS, t("Local")) } else { (icons::GLOBE_SIMPLE, t("World")) };
                if icon_tool(ui, app.gizmo_local, icon, Some(&name), format!("{}\n{}", keys.tip(Action::GizmoSpace), t("World: the gizmo follows the axes of the map. Local: those of the selected object."))).clicked() { app.gizmo_local = !app.gizmo_local; }
            });
            ui.separator();
            for (v, icon, name, action) in [(ViewMode::Perspective, icons::CUBE, "3D", Action::View3d), (ViewMode::Top, icons::SQUARE, "2D", Action::View2d), (ViewMode::Split, icons::COLUMNS, "3D | 2D", Action::ViewSplit)] {
                if icon_tool(ui, app.view_mode == v, icon, Some(name), keys.tip(action)).clicked() { app.view_mode = v; }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let free = has && app.task.is_none();
                if app.session.is_some() {
                    if ui.add(egui::Button::new(RichText::new(format!("{} {}", icons::STOP, t("Stop"))).color(Color32::WHITE)).fill(theme::ERROR)).clicked() { app.stop_session(); }
                } else if ui.add_enabled(free, egui::Button::new(RichText::new(format!("{} {}", icons::PLAY, t("Play"))).color(Color32::WHITE)).fill(theme::PLAY)).on_hover_text(format!("{}\n{}", keys.tip(Action::Play), t("Export, binarise and start the map in DayZ"))).clicked() {
                    app.build(true);
                }
                if ui.add_enabled(free, egui::Button::new(format!("{} {}", icons::HAMMER, t("Build")))).on_hover_text(format!("{}\n{}", keys.tip(Action::Build), t("Export and binarise the PBO"))).clicked() { app.build(false); }
                if ui.add(egui::Button::new(icons::KEYBOARD).frame(false)).on_hover_text(keys.tip(Action::Shortcuts)).clicked() { app.dialog = Dialog::Shortcuts; }
            });
        });
    });
}

/// Keys shown next to what they do, as small key caps.
/// The angle step lines and turns snap to, and the key that does it.
fn angle_snap_ui(app: &mut App, ui: &mut egui::Ui) {
    let key = app.settings.keys.hold(Hold::AngleSnap).name().to_string();
    let before = app.settings.angle_step;
    row(ui, &tf("Angle step with {0}", &[&key]), |ui| {
        ui.add(egui::DragValue::new(&mut app.settings.angle_step).range(1.0..=90.0).speed(0.5).suffix("°"));
    });
    if app.settings.angle_step != before { app.settings.save(); }
}

pub fn key_hints(ui: &mut egui::Ui, rows: &[(String, String)]) {
    egui::Grid::new(ui.next_auto_id()).num_columns(2).spacing(vec2(8.0, 3.0)).show(ui, |ui| {
        for (keys, what) in rows {
            if keys.is_empty() { continue; }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                for (n, k) in keys.split(" / ").enumerate() {
                    if n > 0 { ui.weak("/"); }
                    egui::Frame::new().fill(Color32::from_rgb(30, 30, 30)).stroke(egui::Stroke::new(1.0f32, Color32::from_rgb(78, 78, 78))).corner_radius(3).inner_margin(egui::Margin::symmetric(4, 1)).show(ui, |ui| {
                        ui.label(RichText::new(k).monospace().size(11.0).color(Color32::from_gray(220)));
                    });
                }
            });
            ui.label(RichText::new(what).small().color(theme::TEXT_DIM));
            ui.end_row();
        }
    });
}

/// The keys of the brush of the current tool, from the key map.
pub fn brush_hints(app: &App, ui: &mut egui::Ui, invert: Option<&str>, smooth: bool, strength: bool, noise: bool) {
    let k = &app.settings.keys;
    let wheel = |h: Hold| format!("{}+{}", k.hold(h).name(), t("wheel"));
    let both = |a: Action, b: Action| [k.text(a), k.text(b)].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
    let mut rows = vec![(format!("{} / {}", both(Action::BrushSmaller, Action::BrushBigger), wheel(Hold::WheelSize)), t("Brush size"))];
    if strength { rows.push((format!("{} / {}", both(Action::BrushWeaker, Action::BrushStronger), wheel(Hold::WheelStrength)), t("Strength"))); }
    if noise { rows.push((wheel(Hold::WheelNoise), t("Edge noise"))); }
    if let Some(what) = invert { rows.push((k.hold(Hold::Invert).name().to_string(), t(what))); }
    if smooth { rows.push((k.hold(Hold::Smooth).name().to_string(), t("Smooth"))); }
    key_hints(ui, &rows);
}

fn status_bar(app: &mut App, ctx: &egui::Context) {
    egui::TopBottomPanel::bottom("status").exact_height(24.0).show(ctx, |ui| {
        ui.horizontal_centered(|ui| {
            let mut cancel = false;
            if let Some(task) = &app.task {
                ui.spinner();
                ui.add(egui::ProgressBar::new(task.job.progress()).desired_width(220.0).text(format!("{}: {}", task.title, task.job.current_stage())));
                cancel = ui.small_button(t("Cancel")).clicked();
            } else {
                ui.label(RichText::new(&app.status).small());
            }
            if cancel {
                if let Some(task) = &app.task { task.job.cancel(); }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let stats = app.renderer.lock().unwrap().stats;
                ui.label(RichText::new(format!("{:.1} ms  |  {} kT  |  {} DC  |  {} / {}", app.frame_ms, stats.triangles / 1000, stats.draw_calls, stats.mesh_instances, stats.impostors)).small().color(theme::TEXT_DIM));
                if let Some(d) = &app.doc {
                    let mark = if d.history.dirty() { "*" } else { "" };
                    ui.label(RichText::new(format!("{}{}  |  {} km  |  {} {}", d.world.meta.name, mark, d.world.meta.size_m() / 1000.0, d.world.objects.len(), t("objects"))).small());
                }
                if app.loader.pending() > 0 {
                    ui.label(RichText::new(tf("Loading models: {0}", &[&app.loader.pending().to_string()])).small().color(theme::WARN));
                }
            });
        });
    });
}

fn slider(ui: &mut egui::Ui, label: &str, v: &mut f32, r: std::ops::RangeInclusive<f32>, log: bool) {
    row(ui, label, |ui| { ui.add(egui::Slider::new(v, r).logarithmic(log)); });
}

fn modes_panel(app: &mut App, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match app.mode {
        Mode::Select => select_mode(app, ui),
        Mode::Landscape => sculpt_mode(app, ui),
        Mode::Paint => paint_mode(app, ui),
        Mode::Place => place_mode(app, ui),
        Mode::Lines => lines_mode(app, ui),
        Mode::Generate => {}
        Mode::Shapes => shapes_mode(app, ui),
        Mode::KeyPoints => keypoints_mode(app, ui),
        Mode::Satellite => crate::satellite::panel(app, ui),
    });
}

fn select_mode(app: &mut App, ui: &mut egui::Ui) {
    section(ui, &icons::with(icons::CURSOR_CLICK, &t("Selection")), true, |ui| {
        ui.label(tf("{0} objects selected", &[&app.selection.len().to_string()]));
        ui.weak(t("Click to select, drag for a box, Shift adds. Shift or Ctrl held while dragging a selected object rotates or scales it."));
        ui.horizontal(|ui| {
            ui.label(t("Drag selects"));
            let mut changed = false;
            if crate::theme::tool_button(ui, !app.settings.lasso, &icons::with(icons::SELECTION, &t("Box"))).clicked() { app.settings.lasso = false; changed = true; }
            if crate::theme::tool_button(ui, app.settings.lasso, &icons::with(icons::LASSO, &t("Lasso"))).on_hover_text(t("Draw an outline round what to select.")).clicked() { app.settings.lasso = true; changed = true; }
            if changed { app.settings.save(); }
        });
        ui.checkbox(&mut app.select_whole, t("Select whole roads and generators"));
        if ui.checkbox(&mut app.settings.drag_by_body, t("Move objects by dragging them")).on_hover_text(t("An object held by its shape slides over the ground, as with the dot of the gizmo.")).changed() { app.settings.save(); }
        angle_snap_ui(app, ui);
        let k = &app.settings.keys;
        key_hints(ui, &[
            (format!("{} / {} / {}", k.text(Action::GizmoMove), k.text(Action::GizmoRotate), k.text(Action::GizmoScale)), t("Move, rotate, scale")),
            (k.hold(Hold::Precise).name().to_string(), t("Move and turn slowly")),
            (k.text(Action::Focus), t(Action::Focus.name())),
            (k.text(Action::Duplicate), t(Action::Duplicate.name())),
            (k.text(Action::Underground), t(Action::Underground.name())),
            (k.text(Action::CameraBelow), t(Action::CameraBelow.name())),
        ]);
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button(t("Duplicate")).clicked() { app.duplicate_selection(); }
            if ui.button(t("Delete")).clicked() { app.delete_selection(); }
            if ui.add_enabled(!app.selection.is_empty(), egui::Button::new(icons::with(icons::PACKAGE, &t("Create prefab")))).on_hover_text(t("Saves the selected objects as a prefab of the library, to be placed again anywhere as one.")).clicked() { app.create_prefab(); }
        });
    });
    section(ui, &icons::with(icons::STACK, &t("Terrain under objects")), true, |ui| {
        ui.weak(t("A modifier levels the ground under the object and follows it. The sculpted terrain stays untouched."));
        let mut changed = pad_sliders(app, ui);
        if ui.checkbox(&mut app.place.pad_follow, t("The ground follows the height of the object")).on_hover_text(t("The ground is levelled to the height the object stands at and comes up or down with it. The object keeps a height of its own, so lifting the ground does not lift it again.")).changed() {
            let (ids, on): (Vec<u32>, bool) = (app.selection.iter().copied().collect(), app.place.pad_follow);
            app.set_pad_follow(&ids, on);
        }
        if ui.add_enabled(!app.selection.is_empty(), egui::Button::new(t("Add modifier to selection"))).clicked() {
            let f = app.place.pad_falloff;
            app.add_pads_for_selection(f);
            changed = false;
        }
        if changed { app.update_selected_pads(); }
    });
}

/// Width and blend of the ground levelled under objects.
fn pad_sliders(app: &mut App, ui: &mut egui::Ui) -> bool {
    let before = (app.place.pad_margin, app.place.pad_falloff);
    slider(ui, &t("Margin"), &mut app.place.pad_margin, 0.0..=40.0, false);
    slider(ui, &t("Blend"), &mut app.place.pad_falloff, 0.0..=60.0, false);
    before != (app.place.pad_margin, app.place.pad_falloff)
}

/// Built-in outlines, the user's pictures and the spray, for the sculpt or the paint brush.
fn brush_tip(app: &mut App, ui: &mut egui::Ui, paint: bool) {
    let (shape, picture, spray, density) = if paint {
        (&mut app.paint.shape, &mut app.paint.picture, &mut app.paint.spray, &mut app.paint.spray_density)
    } else {
        (&mut app.sculpt.shape, &mut app.sculpt.picture, &mut app.sculpt.spray, &mut app.sculpt.spray_density)
    };
    let action = crate::brushes::tip_row(ui, &mut app.brushes, shape, picture);
    crate::brushes::spray_row(ui, spray, density);
    if let crate::brushes::TipAction::Problems(p) = action {
        for e in p { app.error(e); }
    }
}

/// The sides, level and height profile of the ground under a road.
pub(crate) fn grading_ui(ui: &mut egui::Ui, g: &mut dz_core::world::RoadGrading, width: &mut f32, blend: &mut f32, designed: bool) {
    slider(ui, &t("Width"), width, 1.0..=80.0, false);
    slider(ui, &t("Shoulder"), &mut g.shoulder, 0.0..=12.0, false);
    ui.checkbox(&mut g.embankment, t("Embankments and cuttings"));
    if g.embankment {
        slider(ui, &t("Embankment slope"), &mut g.fill_slope, 10.0..=70.0, false);
        slider(ui, &t("Cutting slope"), &mut g.cut_slope, 10.0..=80.0, false);
        slider(ui, &t("Bank reach"), &mut g.max_bank, 2.0..=200.0, false);
        slider(ui, &t("Ditch depth"), &mut g.ditch_depth, 0.0..=4.0, false);
        if g.ditch_depth > 0.0 { slider(ui, &t("Ditch width"), &mut g.ditch_width, 0.5..=8.0, false); }
    } else {
        slider(ui, &t("Blend"), blend, 0.0..=120.0, false);
    }
    if designed {
        ui.weak(t("Heights along this route follow its grade, cut and fill limits. Change them in the route settings and fit again."));
    } else {
        slider(ui, &t("Height smoothing"), &mut g.smooth, 0.0..=200.0, false);
        slider(ui, &t("Max. climb %"), &mut g.max_grade, 0.0..=30.0, false);
    }
    slider(ui, &t("Raise the bed"), &mut g.raise, -6.0..=12.0, false);
}

fn sculpt_mode(app: &mut App, ui: &mut egui::Ui) {
    section(ui, &icons::with(icons::MOUNTAINS, &t("Sculpt tool")), true, |ui| {
        ui.horizontal_wrapped(|ui| {
            for (tool, name) in [(SculptTool::Raise, "Raise"), (SculptTool::Lower, "Lower"), (SculptTool::Smooth, "Smooth"), (SculptTool::Flatten, "Flatten"), (SculptTool::Noise, "Noise"), (SculptTool::Ramp, "Ramp")] {
                if tool_button(ui, app.sculpt.tool == tool && !app.hole_tool, &t(name)).clicked() { app.sculpt.tool = tool; app.hole_tool = false; }
            }
            if tool_button(ui, app.hole_tool, &t("Terrain holes")).clicked() { app.hole_tool = !app.hole_tool; }
        });
    });
    if app.hole_tool {
        section(ui, &icons::with(icons::CIRCLE_DASHED, &t("Terrain holes")), true, |ui| {
            slider(ui, &t("Radius"), &mut app.hole_radius, 0.5..=100.0, true);
            ui.weak(t("Paint the cells the game should not draw. Objects under the terrain need the underground flag, and they cost performance."));
            brush_hints(app, ui, Some("Remove the holes"), false, false, false);
            if let Some(d) = &app.doc { ui.label(tf("{0} cells", &[&d.world.holes.len().to_string()])); }
        });
        return;
    }
    section(ui, &icons::with(icons::PAINT_BRUSH_BROAD, &t("Brush")), true, |ui| {
        brush_tip(app, ui, false);
        slider(ui, &t("Radius"), &mut app.sculpt.radius, 2.0..=3000.0, true);
        slider(ui, &t("Falloff"), &mut app.sculpt.falloff, 0.0..=1.0, false);
        slider(ui, &t("Strength"), &mut app.sculpt.strength, 0.5..=200.0, true);
        slider(ui, &t("Edge noise"), &mut app.sculpt.noise, 0.0..=1.0, false);
        brush_hints(app, ui, Some("Lower instead of raise"), true, true, true);
    });
}

fn layer_list(app: &mut App, ui: &mut egui::Ui) {
    let Some(doc) = &mut app.doc else { return };
    let mut changed = false;
    for (i, l) in doc.world.layers.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            if ui.color_edit_button_srgb(&mut l.color).changed() { changed = true; }
            if ui.selectable_label(app.paint.layer == i, format!("L{i:02}  {}", l.name)).on_hover_text(&l.texture).clicked() { app.paint.layer = i; }
        });
    }
    if changed {
        doc.history.revision += 1;
        app.push_palette();
    }
}

fn paint_mode(app: &mut App, ui: &mut egui::Ui) {
    section(ui, &icons::with(icons::PAINT_BRUSH_BROAD, &t("Brush")), true, |ui| {
        brush_tip(app, ui, true);
        slider(ui, &t("Radius"), &mut app.paint.radius, 0.5..=2000.0, true);
        slider(ui, &t("Edge noise"), &mut app.paint.scatter, 0.0..=0.8, false);
        if tool_button(ui, app.paint.picking, &t("Eyedropper")).on_hover_text(t("Takes the surface under the cursor.")).clicked() { app.paint.picking = !app.paint.picking; }
        let names: Vec<String> = app.doc.as_ref().map(|d| d.world.layers.iter().map(|l| l.name.clone()).collect()).unwrap_or_default();
        row(ui, &t("Paint only on"), |ui| {
            let shown = app.paint.only_on.and_then(|i| names.get(i).cloned()).unwrap_or_else(|| t("(none)"));
            egui::ComboBox::from_id_salt("only_on").selected_text(shown).show_ui(ui, |ui| {
                ui.selectable_value(&mut app.paint.only_on, None, t("(none)"));
                for (i, n) in names.iter().enumerate() { ui.selectable_value(&mut app.paint.only_on, Some(i), n); }
            });
        });
        let pick = app.settings.keys.hold(Hold::Pick).name().to_string();
        brush_hints(app, ui, None, false, false, true);
        key_hints(ui, &[(format!("{pick}+{}", t("click")), t("Take the surface under the cursor"))]);
    });
    crate::maskmods::panel(app, ui);
    section(ui, &icons::with(icons::STACK, &t("Surface layers")), true, |ui| {
        layer_list(app, ui);
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            let surfaces = app.surfaces.clone();
            ui.menu_button(format!("+ {}", t("Add layer")), |ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); 
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for s in surfaces {
                        let name = s.rsplit('\\').next().unwrap_or(&s).trim_end_matches("_ca.paa").to_string();
                        if ui.button(&name).clicked() {
                            if let Some(d) = &mut app.doc {
                                let n = d.world.layers.len();
                                if n < 255 {
                                    let color = forest_core::species::hsv_to_rgb(n as f64 * 67.0 % 360.0, 0.5, 0.75);
                                    d.world.layers.push(SurfaceLayer { name, texture: s.clone(), material: s.replace("_ca.paa", ".rvmat"), color });
                                    d.history.revision += 1;
                                }
                            }
                            app.push_palette();
                            app.reload_layer_textures();
                            ui.close_menu();
                        }
                    }
                });
            });
        });
    });
    ui.weak(t("The brush will not add a surface to a tile that already has as many as the project allows."));
}

fn place_mode(app: &mut App, ui: &mut egui::Ui) {
    section(ui, &icons::with(icons::TREE_EVERGREEN, &t("Place objects")), true, |ui| {
        let presets: Vec<(u64, String)> = app.library.objects.iter().map(|p| (p.id, p.name.clone())).collect();
        row(ui, &t("Object preset"), |ui| {
            let shown = app.place.preset.and_then(|id| presets.iter().find(|p| p.0 == id)).map(|p| p.1.clone()).unwrap_or_else(|| t("(none)"));
            egui::ComboBox::from_id_salt("place_preset").selected_text(shown).show_ui(ui, |ui| {
                ui.selectable_value(&mut app.place.preset, None, t("(none)"));
                for (id, name) in &presets { ui.selectable_value(&mut app.place.preset, Some(*id), name); }
            });
        });
        let pool = app.place.pool.len();
        if app.place.preset.is_none() && pool > 1 && app.place.model.as_ref().is_some_and(|m| app.place.pool.contains(m)) {
            ui.horizontal_wrapped(|ui| {
                ui.weak(tf("One of the {0} models selected in the Content Browser is placed at random.", &[&pool.to_string()]));
                if ui.small_button(t("Only one")).clicked() { app.place.pool.clear(); }
            });
        }
        ui.checkbox(&mut app.place.brush, t("Paint with a brush"));
        if app.place.brush {
            slider(ui, &t("Radius"), &mut app.place.brush_radius, 1.0..=300.0, true);
            slider(ui, &t("Spacing"), &mut app.place.brush_spacing, 0.3..=60.0, true);
            ui.weak(t("Hold the button to paint objects."));
            brush_hints(app, ui, Some("Remove objects"), false, true, false);
        }
        match &app.place.model {
            Some(m) => { ui.label(RichText::new(m.rsplit('\\').next().unwrap_or(m)).color(Color32::WHITE)); }
            None => { ui.weak(t("Choose a model in the Content Browser below, then click on the terrain. Models can also be dragged onto the map.")); }
        }
        ui.checkbox(&mut app.place.random_yaw, t("Random rotation"));
        ui.checkbox(&mut app.place.align, t("Align to slope"));
        slider(ui, &t("Min scale"), &mut app.place.scale_min, 0.1..=4.0, false);
        slider(ui, &t("Max scale"), &mut app.place.scale_max, 0.1..=4.0, false);
        if app.place.scale_max < app.place.scale_min { app.place.scale_max = app.place.scale_min; }
    });
    section(ui, &icons::with(icons::STACK, &t("Terrain under objects")), true, |ui| {
        ui.checkbox(&mut app.place.auto_pad, t("Level the ground when placing"));
        slider(ui, &t("Blend"), &mut app.place.pad_falloff, 0.0..=60.0, false);
    });
}

fn shapes_mode(app: &mut App, ui: &mut egui::Ui) {
    section(ui, &icons::with(icons::PATH, &t("Draw")), true, |ui| {
        ui.horizontal(|ui| {
            for (tool, name) in [(ShapeTool::Edit, "Edit"), (ShapeTool::Polygon, "Polygon"), (ShapeTool::Polyline, "Polyline"), (ShapeTool::Point, "Point")] {
                if tool_button(ui, app.shape.tool == tool, &t(name)).clicked() {
                    app.shape.tool = tool;
                    app.shape.points.clear();
                }
            }
        });
        if app.shape.tool == ShapeTool::Edit {
            ui.weak(t("Click selects a shape. Drag a dot to move a point, a + to add one, the square to move the whole shape. Right click or Delete removes a point, Esc cancels a drag."));
        } else {
            ui.weak(t("Click to add points. Enter or a double click finishes, Backspace removes the last point, Esc cancels."));
            ui.label(tf("{0} points", &[&app.shape.points.len().to_string()]));
            angle_snap_ui(app, ui);
        }
    });
    section(ui, &icons::with(icons::FOLDER_OPEN, &t("Files")), true, |ui| {
        ui.horizontal(|ui| {
            if ui.button(t("Import...")).on_hover_text("SHP, TV4S").clicked() { app.import_shapes(); }
            if ui.button(t("Export...")).on_hover_text("SHP").clicked() { app.export_shapes(); }
        });
    });
    section(ui, &icons::with(icons::POLYGON, &t("Selected shape")), true, |ui| shape_details(app, ui));
}

fn keypoints_mode(app: &mut App, ui: &mut egui::Ui) {
    section(ui, &icons::with(icons::PATH, &t("Draw")), true, |ui| {
        ui.horizontal(|ui| {
            if tool_button(ui, !app.shape.keypoint_adding, &t("Edit")).clicked() { app.shape.keypoint_adding = false; }
            if tool_button(ui, app.shape.keypoint_adding, &t("Add")).clicked() { app.shape.keypoint_adding = true; }
        });
        ui.weak(if app.shape.keypoint_adding { t("Click on the map adds a key point.") } else { t("Click selects a key point. Drag moves it, Delete removes it.") });
    });
    section(ui, &icons::with(icons::MAP_PIN, &t("Key points")), true, |ui| keypoints_ui(app, ui));
}

/// The key points of the map, as Terrain Builder's selection properties show them.
fn keypoints_ui(app: &mut App, ui: &mut egui::Ui) {
    use dz_core::keypoints::TYPES;
    if ui.button(t("Import...")).on_hover_text("TV4P, HPP").clicked() { app.import_keypoints(); }
    ui.add(egui::TextEdit::singleline(&mut app.shape.keypoint_filter).hint_text(format!("{} {}", icons::MAGNIFYING_GLASS, t("Search..."))));
    let filter = app.shape.keypoint_filter.to_lowercase();
    let Some(doc) = &app.doc else { return };
    let names: Vec<(usize, String)> = doc.world.keypoints.iter().enumerate().filter(|(_, k)| k.name.to_lowercase().contains(&filter)).map(|(i, k)| (i, k.name.clone())).collect();
    egui::ScrollArea::vertical().id_salt("keypoints").max_height(140.0).show(ui, |ui| {
        for (i, name) in names {
            if ui.selectable_label(app.shape.keypoint == Some(i), name).clicked() { app.shape.keypoint = Some(i); }
        }
    });
    let Some(i) = app.shape.keypoint.filter(|i| app.doc.as_ref().is_some_and(|d| *i < d.world.keypoints.len())) else {
        app.shape.keypoint = None;
        return;
    };
    let doc = app.doc.as_mut().unwrap();
    let mut k = doc.world.keypoints[i].clone();
    ui.separator();
    row(ui, &t("Name"), |ui| { ui.text_edit_singleline(&mut k.name); });
    row(ui, &t("Type"), |ui| {
        egui::ComboBox::from_id_salt("keypoint type").selected_text(k.kind.clone()).show_ui(ui, |ui| {
            for n in TYPES { ui.selectable_value(&mut k.kind, n.to_string(), n); }
        });
    });
    ui.label(t("Properties"));
    ui.add(egui::TextEdit::multiline(&mut k.properties).desired_rows(2));
    let number = |ui: &mut egui::Ui, label: &str, v: &mut f32, speed: f64| row(ui, label, |ui| { ui.add(egui::DragValue::new(v).speed(speed).max_decimals(3)); });
    number(ui, "X", &mut k.x, 1.0);
    number(ui, "Y", &mut k.z, 1.0);
    number(ui, &t("Angle"), &mut k.angle, 0.5);
    number(ui, &t("Width"), &mut k.width, 1.0);
    number(ui, &t("Height"), &mut k.height, 1.0);
    ui.checkbox(&mut k.show, t("Show on map"));
    row(ui, &t("Text"), |ui| { ui.text_edit_singleline(&mut k.text); });
    let edited = k != doc.world.keypoints[i];
    if edited {
        let before = doc.world.keypoints.clone();
        app.shape.keypoints_before.get_or_insert(before);
        doc.world.keypoints[i] = k;
    } else if !ui.ctx().wants_keyboard_input() && !ui.ctx().input(|p| p.pointer.any_down()) {
        // A whole edit is one undo step.
        if !app.shape.keypoint_moving {
            if let Some(before) = app.shape.keypoints_before.take().filter(|b| *b != doc.world.keypoints) { doc.history.push(t("Edit key point"), dz_core::edit::Change::KeyPoints(before)); }
        }
    }
    if ui.button(t("Delete")).clicked() { app.delete_keypoint(i); }
}

/// Vertices, attributes and layer of the selected shapes.
fn shape_details(app: &mut App, ui: &mut egui::Ui) {
    use dz_core::shapes::ShapeKind;
    let count = app.shape.selection.len();
    if count == 0 {
        ui.weak(t("Nothing selected"));
        return;
    }
    let Some(doc) = &app.doc else { return };
    let previous = doc.world.shape_layers.clone();
    let mut layers = previous.clone();
    let names: Vec<String> = layers.iter().map(|l| l.name.clone()).collect();
    let first = *app.shape.selection.iter().next().unwrap();
    let Some(mut shape) = layers.get(first.0).and_then(|l| l.shapes.get(first.1)).cloned() else {
        app.shape.selection.clear();
        return;
    };
    let mut target = first.0;
    let mut changed = false;
    let mut remove = false;
    let mut to_line = None;
    if count > 1 {
        ui.label(tf("{0} shapes selected", &[&count.to_string()]));
    } else {
        ui.label(match shape.kind { ShapeKind::Point => t("Point"), ShapeKind::Polyline => t("Polyline"), ShapeKind::Polygon => t("Polygon") });
        if shape.kind != ShapeKind::Point {
            let mut kind = shape.kind;
            ui.horizontal(|ui| {
                ui.selectable_value(&mut kind, ShapeKind::Polygon, t("Polygon"));
                ui.selectable_value(&mut kind, ShapeKind::Polyline, t("Polyline"));
            });
            if kind != shape.kind && shape.points.len() >= kind.min_points() {
                shape.kind = kind;
                changed = true;
            }
        }
    }
    row(ui, &t("Layer"), |ui| {
        egui::ComboBox::from_id_salt("shape_layer").selected_text(names.get(target).cloned().unwrap_or_default()).show_ui(ui, |ui| {
            for (i, n) in names.iter().enumerate() { ui.selectable_value(&mut target, i, n); }
        });
    });
    if count == 1 {
        egui::CollapsingHeader::new(tf("{0} points", &[&shape.points.len().to_string()])).id_salt("shape_points").default_open(shape.points.len() <= 12).show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("shape_point_list").max_height(220.0).auto_shrink([false, true]).show(ui, |ui| {
                let marked = app.shape.vertex;
                for (i, p) in shape.points.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.selectable_label(marked == Some(i), format!("{}", i + 1)).clicked() { app.shape.vertex = Some(i); }
                        changed |= ui.add(egui::DragValue::new(&mut p[0]).speed(0.1).prefix("X ")).changed();
                        changed |= ui.add(egui::DragValue::new(&mut p[1]).speed(0.1).prefix("Z ")).changed();
                    });
                }
            });
        });
        for (k, v) in &mut shape.attributes {
            row(ui, k, |ui| { changed |= ui.text_edit_singleline(v).changed(); });
        }
        if shape.kind != ShapeKind::Point && ui.button(t("Use as a road or object line")).on_hover_text(t("Opens the Roads tool with these points.")).clicked() {
            to_line = Some(shape.points.clone());
        }
    }
    if ui.button(t("Delete shape")).clicked() { remove = true; }
    if changed {
        layers[first.0].shapes[first.1] = shape;
        let doc = app.doc.as_mut().unwrap();
        doc.world.shape_layers = layers;
        // One undo step for a whole drag or a typed value.
        app.shape.drag_before.get_or_insert(previous);
    }
    let settled = !ui.ctx().input(|i| i.pointer.any_down()) && !ui.ctx().wants_keyboard_input() && app.shape.boxing.is_none();
    if settled {
        if let Some(before) = app.shape.drag_before.take() {
            app.doc.as_mut().unwrap().history.push(t("Edit shape"), dz_core::edit::Change::Shapes(before));
        }
    }
    if target != first.0 {
        let doc = app.doc.as_mut().unwrap();
        let before = doc.world.shape_layers.clone();
        let mut moved = Vec::new();
        for (l, s) in app.shape.selection.iter().rev() {
            if let Some(list) = doc.world.shape_layers.get_mut(*l).map(|x| &mut x.shapes) {
                if *s < list.len() { moved.push(list.remove(*s)); }
            }
        }
        let dest = &mut doc.world.shape_layers[target];
        let start = dest.shapes.len();
        let n = moved.len();
        dest.shapes.extend(moved);
        // Removing from the same layer earlier shifts what is left, so indices come from the end.
        let end = dest.shapes.len();
        app.shape.selection = (end - n.min(end)..end.max(start)).map(|i| (target, i)).collect();
        app.shape.vertex = None;
        doc.history.push(t("Edit shape"), dz_core::edit::Change::Shapes(before));
    }
    if let Some(points) = to_line {
        app.line.points = points;
        app.mode = Mode::Lines;
    }
    if remove {
        app.shape.vertex = None;
        app.delete_shape_part();
    }
}

fn routing_fields(ui: &mut egui::Ui, r: &mut dayz_road_tool::document::RoutingSettings) -> bool {
    let mut changed = false;
    let mut field = |ui: &mut egui::Ui, label: String, v: &mut f64, range: std::ops::RangeInclusive<f64>| {
        row(ui, &label, |ui| { changed |= ui.add(egui::Slider::new(v, range)).changed(); });
    };
    field(ui, t("Minimum radius"), &mut r.min_radius, 5.0..=300.0);
    field(ui, t("Max. grade %"), &mut r.max_grade, 1.0..=30.0);
    field(ui, t("Slope weight"), &mut r.slope_weight, 0.0..=20.0);
    field(ui, t("Search step m"), &mut r.cell, 5.0..=60.0);
    field(ui, t("Tolerance m"), &mut r.tolerance, 0.5..=20.0);
    field(ui, t("Max. cut m"), &mut r.max_cut, 0.0..=20.0);
    field(ui, t("Max. fill m"), &mut r.max_fill, 0.0..=20.0);
    field(ui, t("Earthwork penalty"), &mut r.earth_weight, 0.0..=5.0);
    field(ui, t("Max. bridge height m"), &mut r.bridge_height, 0.0..=60.0);
    field(ui, t("Max. bridge length m"), &mut r.bridge_max, 0.0..=600.0);
    row(ui, &t("Bridge cost"), |ui| { changed |= ui.add(egui::Slider::new(&mut r.bridge_weight, 0.5..=10.0)).on_hover_text(t("A metre of bridge counts as this many metres of road. Lower it when cheap earthworks lead the road down into valleys instead of over them.")).changed(); });
    changed |= ui.checkbox(&mut r.bridge_over_forbidden, t("Bridges may cross forbidden areas and water")).changed();
    changed
}

/// Everything about the route being edited.
fn route_details(app: &mut App, ui: &mut egui::Ui) {
    let Some(doc) = &mut app.doc else { return };
    let Some(index) = app.line.selected_route.filter(|i| *i < doc.world.routes.len()) else {
        ui.weak(t("Nothing selected"));
        return;
    };
    let before = doc.world.routes[index].points.clone();
    let families = app.road_families.clone();
    let road_width = doc.world.routes[index].objects.iter().take(24).filter_map(|id| doc.world.objects.get(*id)).map(|o| app.db.get(doc.world.objects.model_path(o))).filter(|i| i.known).map(|i| (i.max[0] - i.min[0]).min(i.max[2] - i.min[2])).fold(0.0f32, f32::max);
    let route = &mut doc.world.routes[index];
    row(ui, &t("Name"), |ui| { ui.text_edit_singleline(&mut route.name); });
    row(ui, &t("Road type"), |ui| {
        egui::ComboBox::from_id_salt("route_family").selected_text(route.family.clone()).show_ui(ui, |ui| {
            for f in families { ui.selectable_value(&mut route.family, f.clone(), f); }
        });
    });
    ui.checkbox(&mut route.terrain_route, t("Route across terrain"));
    ui.collapsing(t("Route settings"), |ui| { routing_fields(ui, &mut route.settings); });
    let grading = (route.grade, route.grade_width, route.grade_falloff, route.grading);
    ui.checkbox(&mut route.grade, t("Grade the terrain under the road"));
    if route.grade {
        grading_ui(ui, &mut route.grading, &mut route.grade_width, &mut route.grade_falloff, !route.profile.is_empty());
        if ui.add_enabled(road_width > 0.5, egui::Button::new(t("Take the width from the road model"))).clicked() { route.grade_width = road_width; }
    }
    let regrade = grading != (route.grade, route.grade_width, route.grade_falloff, route.grading);
    ui.label(tf("{0} models", &[&route.objects.len().to_string()]));
    let mut typed = false;
    egui::CollapsingHeader::new(tf("{0} points", &[&route.points.len().to_string()])).id_salt("route_points").default_open(route.points.len() <= 10).show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("route_point_list").max_height(200.0).auto_shrink([false, true]).show(ui, |ui| {
            let marked = app.line.selected_point;
            for (i, p) in route.points.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    if ui.selectable_label(marked == Some(i), format!("{}", i + 1)).clicked() { app.line.selected_point = Some(i); }
                    typed |= ui.add(egui::DragValue::new(&mut p[0]).speed(0.5).prefix("X ")).changed();
                    typed |= ui.add(egui::DragValue::new(&mut p[1]).speed(0.5).prefix("Z ")).changed();
                });
            }
        });
    });
    if typed && app.line.point_drag.is_none() { app.line.point_drag = Some(before.clone()); }
    let mut action = 0;
    ui.horizontal(|ui| {
        if ui.button(t("Fit again")).on_hover_text(t("Applies the type, settings and typed points.")).clicked() { action = 1; }
        if ui.button(t("Reverse")).clicked() { action = 2; }
    });
    ui.horizontal(|ui| {
        if tool_button(ui, app.line.extend == Some(true), &t("Extend start")).clicked() { app.line.extend = if app.line.extend == Some(true) { None } else { Some(true) }; }
        if tool_button(ui, app.line.extend == Some(false), &t("Extend end")).clicked() { app.line.extend = if app.line.extend == Some(false) { None } else { Some(false) }; }
    });
    ui.checkbox(&mut app.line.extend_only, t("Fit only the new stretch when extending")).on_hover_text(t("A route lengthened at its start or end keeps its models; Fit again lays models only from the old end to the new points. Moving other points still fits the whole route."));
    if ui.button(t("Delete route")).clicked() { action = 4; }
    // Typed values wait for "Fit again"; dragging in the list must not start the viewport drag.
    let typed_before = if app.mode == Mode::Lines && !ui.ctx().input(|i| i.pointer.primary_down()) { app.line.point_drag.take() } else { None };
    match action {
        1 => app.refit_route(index, typed_before.unwrap_or(before)),
        2 => {
            if let Some(d) = &mut app.doc { d.world.routes[index].points.reverse(); }
            app.line.selected_point = None;
            app.refit_route(index, typed_before.unwrap_or(before));
        }
        4 => {
            if let Some(id) = app.doc.as_ref().and_then(|d| d.world.routes.get(index)).map(|r| r.id) { crate::deleting::request(app, crate::deleting::Target::Route(id)); }
        }
        _ if regrade => app.route_grading_changed(index),
        _ => {}
    }
}

fn lines_mode(app: &mut App, ui: &mut egui::Ui) {
    lines_tool_panel(app, ui);
    if app.line.action == LineAction::Edit {
        section(ui, &icons::with(icons::ROAD_HORIZON, &t("Selected route")), true, |ui| route_details(app, ui));
    }
}

fn lines_tool_panel(app: &mut App, ui: &mut egui::Ui) {
    section(ui, &icons::with(icons::PATH, &t("Draw")), true, |ui| {
        ui.horizontal(|ui| {
            for (a, n) in [(LineAction::Road, "Road"), (LineAction::Fence, "Objects"), (LineAction::Plain, "Line"), (LineAction::Edit, "Edit routes")] {
                if tool_button(ui, app.line.action == a, &t(n)).clicked() { app.line.action = a; }
            }
        });
        if app.line.action == LineAction::Edit {
            ui.weak(t("Click a route, then drag its points. The + between two points adds one, Ctrl+click extends the route from its nearer end, Delete removes the marked point. New points wait for Fit again."));
        } else {
            ui.weak(t("Click to add points. Enter finishes, Backspace removes the last point, Esc cancels."));
            ui.label(tf("{0} points", &[&app.line.points.len().to_string()]));
            angle_snap_ui(app, ui);
        }
        match app.line.action {
            LineAction::Edit => {}
            LineAction::Road => {
                let families = app.road_families.clone();
                row(ui, &t("Road type"), |ui| {
                    egui::ComboBox::from_id_salt("family").selected_text(app.line.family.clone()).show_ui(ui, |ui| {
                        for f in families { ui.selectable_value(&mut app.line.family, f.clone(), f); }
                    });
                });
                ui.checkbox(&mut app.line.route, t("Route across terrain")).on_hover_text(t("Finds the road through the clicked points within the grade limits, around water and forbidden areas, with bridges. Off: road parts follow the clicked line."));
                if app.line.route {
                    let shapes: Vec<String> = app.doc.as_ref().map(|d| d.world.shape_layers.iter().map(|l| l.name.clone()).collect()).unwrap_or_default();
                    if app.line.forbidden.is_some_and(|i| i >= shapes.len()) { app.line.forbidden = None; }
                    row(ui, &t("Forbidden areas"), |ui| {
                        let shown = app.line.forbidden.map(|i| shapes[i].clone()).unwrap_or_else(|| t("(none)"));
                        egui::ComboBox::from_id_salt("forbidden").selected_text(shown).show_ui(ui, |ui| {
                            ui.selectable_value(&mut app.line.forbidden, None, t("(none)"));
                            for (i, n) in shapes.iter().enumerate() { ui.selectable_value(&mut app.line.forbidden, Some(i), n); }
                        });
                    });
                    let r = if dayz_road_tool::rails::is_track_family(&app.line.family) { &mut app.line.rail_routing } else { &mut app.line.routing };
                    ui.collapsing(t("Route settings"), |ui| routing_fields(ui, r));
                }
                if ui.button(t("Road types from TV4P...")).clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("Terrain Builder", &["tv4p"]).pick_file() { app.load_road_types(p); }
                }
                ui.checkbox(&mut app.line.grade, t("Grade the terrain under the road"));
                if app.line.grade {
                    grading_ui(ui, &mut app.line.grading, &mut app.line.grade_width, &mut app.line.grade_falloff, false);
                }
            }
            LineAction::Fence => {
                row(ui, &t("Model"), |ui| {
                    let label = app.line.fence_model.rsplit('\\').next().unwrap_or("").to_string();
                    ui.label(if label.is_empty() { t("none") } else { label });
                    if ui.small_button(t("Use selected")).clicked() {
                        if let Some(m) = &app.place.model { app.line.fence_model = m.clone(); }
                    }
                });
                slider(ui, &t("Spacing"), &mut app.line.fence_spacing, 0.5..=100.0, true);
            }
            LineAction::Plain => {}
        }
    });
}

fn right_panel(app: &mut App, ui: &mut egui::Ui) {
    let h = ui.available_height();
    egui::TopBottomPanel::top("outliner").resizable(true).default_height(h * 0.42).height_range(120.0..=h - 160.0).frame(egui::Frame::new()).show_inside(ui, |ui| outliner(app, ui));
    egui::CentralPanel::default().frame(egui::Frame::new()).show_inside(ui, |ui| details(app, ui));
}

fn panel_title(ui: &mut egui::Ui, title: &str) -> egui::Rect {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::PANEL);
    ui.painter().text(egui::pos2(rect.left() + 8.0, rect.center().y), egui::Align2::LEFT_CENTER, title, egui::FontId::proportional(13.0), Color32::WHITE);
    rect
}

/// A section of the outliner; the fold button in its title opens or closes all of them.
pub(crate) fn outliner_section(fold: Option<bool>, salt: &str, title: String) -> egui::CollapsingHeader {
    egui::CollapsingHeader::new(title).id_salt(("outliner", salt)).default_open(true).open(fold)
}

fn outliner(app: &mut App, ui: &mut egui::Ui) {
    let label = icons::with(icons::TREE_STRUCTURE, &t("Outliner"));
    let title = panel_title(ui, &label);
    let fold = app.outliner_fold.take();
    // The search sits in the title bar, between the title and the fold button.
    let label_width = ui.painter().layout_no_wrap(label, egui::FontId::proportional(13.0), Color32::WHITE).size().x;
    let search = egui::Rect::from_min_max(egui::pos2(title.left() + label_width + 18.0, title.top() + 2.0), egui::pos2(title.right() - 30.0, title.bottom() - 2.0));
    if search.width() > 40.0 {
        ui.put(search, egui::TextEdit::singleline(&mut app.outliner_filter).hint_text(t("Search...")).desired_width(search.width()));
    } else {
        ui.add(egui::TextEdit::singleline(&mut app.outliner_filter).hint_text(t("Search...")).desired_width(f32::INFINITY));
    }
    let (icon, tip) = if app.outliner_unfolded { (icons::ARROWS_IN_LINE_VERTICAL, t("Collapse all")) } else { (icons::ARROWS_OUT_LINE_VERTICAL, t("Expand all")) };
    let button = egui::Rect::from_min_size(egui::pos2(title.right() - 26.0, title.top() + 2.0), vec2(22.0, 20.0));
    if ui.put(button, egui::Button::new(RichText::new(icon).size(15.0)).frame(false)).on_hover_text(tip).clicked() {
        app.outliner_unfolded = !app.outliner_unfolded;
        app.outliner_fold = Some(app.outliner_unfolded);
    }
    let Some(doc) = &app.doc else { return };
    let filter = app.outliner_filter.to_ascii_lowercase();
    let revision = doc.world.objects.revision;
    let layer_total = doc.world.object_layers.len();
    if app.outliner_cache.as_ref().is_none_or(|c| c.revision != revision || c.layers != layer_total) {
        app.outliner_cache = Some(OutlinerCache { revision, layers: layer_total, histogram: doc.world.objects.histogram(), layer_counts: doc.world.objects.layer_counts(layer_total) });
    }
    let cache = app.outliner_cache.as_ref().unwrap();
    let hist = cache.histogram.clone();
    let counts = cache.layer_counts.clone();
    let layers = doc.world.object_layers.clone();
    let active = doc.world.target_layer();
    let has_selection = !app.selection.is_empty();
    let mut layer_action = None;
    let mods: Vec<(u64, String)> = doc.world.modifiers.iter().map(|m| (m.id, m.name.clone())).collect();
    let paths: Vec<String> = hist.iter().map(|(m, _)| doc.world.objects.models[*m as usize].path.clone()).collect();
    let shape_rows: Vec<(usize, String, bool, usize)> = doc.world.shape_layers.iter().enumerate().map(|(i, l)| (i, l.name.clone(), l.visible, l.shapes.len())).collect();
    let active_shape = doc.world.active_shape_layer;
    let total_objects = doc.world.objects.len();
    // Models of the selected objects, lit in the list; a new selection scrolls to them.
    let selected_models: std::collections::HashSet<u32> = app.selection.iter().filter_map(|id| doc.world.objects.get(*id).map(|o| o.model)).collect();
    let scroll = app.outliner_rev != app.selection_rev;
    let mut reveal: Option<String> = None;
    let generators: Vec<(u64, String, &'static str, usize)> = doc.world.generators.iter().map(|g| (g.id, g.name.clone(), g.kind.type_name(), g.objects.len())).collect();
    let open_shapes: Vec<(usize, Vec<(usize, dz_core::shapes::ShapeKind, usize)>)> = app.shape_open.iter().filter_map(|l| Some((*l, doc.world.shape_layers.get(*l)?.shapes.iter().enumerate().map(|(i, s)| (i, s.kind, s.points.len())).collect()))).collect();
    let mut shape_pick: Option<(usize, usize, bool)> = None;
    let mut shape_move: Option<(usize, usize, usize)> = None;
    let mut toggle_open: Option<usize> = None;
    let routes: Vec<(usize, String, String)> = doc.world.routes.iter().enumerate().map(|(i, r)| (i, r.name.clone(), r.family.clone())).collect();
    let mut select_route = None;
    let mut delete_route = None;
    let mut shape_action = None;
    let mut start_shape_rename = None;
    let mut select_model = None;
    let mut start_rename = None;
    let mut select_modifier = None;
    egui::ScrollArea::vertical().id_salt("outl").auto_shrink([false, false]).show(ui, |ui| {
        outliner_section(fold, "Terrain modifiers", icons::with(icons::MOUNTAINS, &tf("Terrain modifiers ({0})", &[&mods.len().to_string()]))).show(ui, |ui| {
            for (id, name) in mods.iter().filter(|(_, n)| n.to_ascii_lowercase().contains(&filter)) {
                if ui.selectable_label(app.selected_modifier == Some(*id), name).clicked() { select_modifier = Some(*id); }
            }
        });
        crate::maskmods::outliner(app, ui, &filter, fold);
        crate::satellite::outliner(app, ui, &filter, fold);
        outliner_section(fold, "Generators", icons::with(icons::MAGIC_WAND, &tf("Generators ({0})", &[&generators.len().to_string()]))).show(ui, |ui| {
            crate::library_ui::outliner_rows(app, ui, &generators, &filter);
        });
        outliner_section(fold, "Routes", icons::with(icons::ROAD_HORIZON, &tf("Routes ({0})", &[&routes.len().to_string()]))).show(ui, |ui| {
            for (i, name, family) in routes.iter().filter(|(_, n, _)| n.to_ascii_lowercase().contains(&filter)) {
                let row = ui.selectable_label(app.line.selected_route == Some(*i), format!("{name}  ({family})"));
                if row.clicked() { select_route = Some(*i); }
                row.context_menu(|ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); if ui.button(t("Delete route")).clicked() { delete_route = Some(*i); ui.close_menu(); } });
            }
        });
        outliner_section(fold, "Shape layers", icons::with(icons::POLYGON, &tf("Shape layers ({0})", &[&shape_rows.len().to_string()]))).show(ui, |ui| {
            if ui.small_button(format!("+ {}", t("New layer"))).clicked() { shape_action = Some(ShapeAction::New); }
            for (i, name, visible, count) in shape_rows.iter().filter(|(_, n, _, _)| n.to_ascii_lowercase().contains(&filter)) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let opened = open_shapes.iter().any(|(l, _)| l == i);
                    if ui.small_button(if opened { "-" } else { "+" }).clicked() { toggle_open = Some(*i); }
                    if ui.add(egui::Button::new("👁").small().selected(*visible)).on_hover_text(t("Visible")).clicked() { shape_action = Some(ShapeAction::Visible(*i)); }
                    if let Some((_, text)) = app.shape_rename.as_mut().filter(|(r, _)| r == i) {
                        let edit = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
                        edit.request_focus();
                        edit.scroll_to_me(Some(egui::Align::Center));
                        if edit.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) { shape_action = Some(ShapeAction::Rename(*i, text.clone())); }
                        return;
                    }
                    let label = RichText::new(format!("{name}  ({count})"));
                    let row = ui.selectable_label(active_shape == *i, if *visible { label } else { label.weak() });
                    if row.clicked() { shape_action = Some(ShapeAction::Active(*i)); }
                    if let Some(p) = row.dnd_release_payload::<(usize, usize)>() { shape_move = Some((p.0, p.1, *i)); }
                    row.context_menu(|ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); 
                        if ui.button(t("Rename")).clicked() { start_shape_rename = Some((*i, name.clone())); ui.close_menu(); }
                        if ui.button(t("Delete layer")).clicked() { shape_action = Some(ShapeAction::Delete(*i)); ui.close_menu(); }
                    });
                });
                if let Some((_, list)) = open_shapes.iter().find(|(l, _)| l == i) {
                    ui.indent(("shapes", *i), |ui| {
                        egui::ScrollArea::vertical().id_salt(("shape_list", *i)).max_height(180.0).auto_shrink([false, true]).show(ui, |ui| {
                            for (s, kind, n) in list {
                                let kind = match kind { dz_core::shapes::ShapeKind::Point => t("Point"), dz_core::shapes::ShapeKind::Polyline => t("Polyline"), dz_core::shapes::ShapeKind::Polygon => t("Polygon") };
                                let selected = app.shape.selection.contains(&(*i, *s));
                                let r = ui.selectable_label(selected, format!("{}  {kind}  ({n})", s + 1)).interact(egui::Sense::click_and_drag());
                                if r.clicked() { shape_pick = Some((*i, *s, ui.input(|i| i.modifiers.shift || i.modifiers.command))); }
                                r.dnd_set_drag_payload((*i, *s));
                            }
                        });
                    });
                }
            }
        });
        let live = layers.iter().filter(|l| !l.deleted).count();
        outliner_section(fold, "Object layers", icons::with(icons::STACK, &tf("Object layers ({0})", &[&live.to_string()]))).show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.small_button(format!("+ {}", t("New layer"))).clicked() { layer_action = Some(LayerAction::New); }
                if ui.small_button(t("Edit all")).clicked() { layer_action = Some(LayerAction::EditAll); }
                if ui.small_button(t("Show all")).clicked() { layer_action = Some(LayerAction::ShowAll); }
            });
            for (i, l) in layers.iter().enumerate().filter(|(_, l)| !l.deleted) {
                if !filter.is_empty() && !l.name.to_ascii_lowercase().contains(&filter) { continue; }
                let id = i as u16;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if ui.add(egui::Button::new("👁").small().selected(l.visible)).on_hover_text(t("Visible")).clicked() { layer_action = Some(LayerAction::Visible(id)); }
                    if ui.add(egui::Button::new("✏").small().selected(l.editable)).on_hover_text(t("Editable")).clicked() { layer_action = Some(LayerAction::Editable(id)); }
                    if let Some((_, name)) = app.layer_rename.as_mut().filter(|(r, _)| *r == id) {
                        let edit = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
                        edit.request_focus();
                        edit.scroll_to_me(Some(egui::Align::Center));
                        if edit.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            layer_action = Some(LayerAction::Rename(id, name.clone()));
                        }
                        return;
                    }
                    let mut text = RichText::new(format!("{}  ({})", l.name, counts.get(i).copied().unwrap_or(0)));
                    if !l.interactive() { text = text.weak(); }
                    if !l.export { text = text.italics(); }
                    let row = ui.selectable_label(active == id, text).on_hover_text(t("Click: active layer (new objects go here). Ctrl+click: turn editing on or off."));
                    if row.clicked() {
                        layer_action = Some(if ui.input(|i| i.modifiers.command) { LayerAction::Editable(id) } else { LayerAction::Active(id) });
                    }
                    row.context_menu(|ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); 
                        if ui.button(t("Edit only this layer")).clicked() { layer_action = Some(LayerAction::Solo(id)); ui.close_menu(); }
                        if ui.button(t("Select objects")).clicked() { layer_action = Some(LayerAction::Select(id)); ui.close_menu(); }
                        if ui.add_enabled(has_selection, egui::Button::new(t("Move selection here"))).clicked() { layer_action = Some(LayerAction::MoveSelection(id)); ui.close_menu(); }
                        let mut export = l.export;
                        if ui.checkbox(&mut export, t("Include in export")).clicked() { layer_action = Some(LayerAction::Export(id)); ui.close_menu(); }
                        ui.separator();
                        if ui.button(t("Rename")).clicked() { start_rename = Some((id, l.name.clone())); ui.close_menu(); }
                        if ui.add_enabled(id != 0, egui::Button::new(t("Delete layer, keep objects"))).clicked() { layer_action = Some(LayerAction::Delete(id)); ui.close_menu(); }
                        if ui.add_enabled(id != 0, egui::Button::new(t("Delete layer and objects"))).clicked() { layer_action = Some(LayerAction::DeleteWithObjects(id)); ui.close_menu(); }
                    });
                });
            }
        });
        outliner_section(fold, "Objects", icons::with(icons::CUBE, &tf("Objects ({0})", &[&total_objects.to_string()]))).show(ui, |ui| {
            let mut scrolled = false;
            for (k, (m, count)) in hist.iter().enumerate() {
                let name = paths[k].rsplit('\\').next().unwrap_or(&paths[k]);
                if !filter.is_empty() && !name.contains(&filter) { continue; }
                let lit = selected_models.contains(m);
                let row = ui.selectable_label(lit, format!("{name}  ×{count}")).on_hover_text(&paths[k]);
                if lit && scroll && !scrolled {
                    row.scroll_to_me(Some(egui::Align::Center));
                    scrolled = true;
                }
                if row.clicked() { select_model = Some(*m); }
                let path = paths[k].clone();
                row.context_menu(|ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    if ui.button(icons::with(icons::FOLDER_OPEN, &t("Show in asset browser"))).clicked() { reveal = Some(path.clone()); ui.close_menu(); }
                    if ui.button(t("Select objects")).clicked() { select_model = Some(*m); ui.close_menu(); }
                });
            }
        });
    });
    app.outliner_rev = app.selection_rev;
    if let Some(p) = reveal { app.show_in_browser(&p); }
    if let Some(m) = select_model {
        let shift = ui.input(|i| i.modifiers.shift);
        if !shift { app.selection.clear(); }
        app.selected_generator = None;
        app.selected_modifier = None;
        let d = app.doc.as_ref().unwrap();
        app.selection.extend(d.world.objects.iter().filter(|(_, o)| o.model == m && d.world.object_interactive(o)).map(|(id, _)| id));
        app.selection_rev += 1;
    }
    if let Some(a) = layer_action {
        if matches!(a, LayerAction::Rename(..)) { app.layer_rename = None; }
        app.layer_action(a);
    }
    if start_rename.is_some() { app.layer_rename = start_rename; }
    if let Some(i) = toggle_open { if !app.shape_open.remove(&i) { app.shape_open.insert(i); } }
    if let Some((l, s, add)) = shape_pick {
        if !add { app.shape.selection.clear(); }
        app.shape.vertex = None;
        if !app.shape.selection.remove(&(l, s)) { app.shape.selection.insert((l, s)); }
        app.selected_generator = None;
    }
    if let Some((from, shape, to)) = shape_move {
        if from != to {
            if let Some(d) = &mut app.doc {
                let before = d.world.shape_layers.clone();
                if shape < d.world.shape_layers[from].shapes.len() {
                    let moved = d.world.shape_layers[from].shapes.remove(shape);
                    d.world.shape_layers[to].shapes.push(moved);
                    d.history.push(t("Edit shape"), dz_core::edit::Change::Shapes(before));
                    app.shape.selection.clear();
                }
            }
        }
    }
    if let Some(id) = delete_route.and_then(|i| app.doc.as_ref().and_then(|d| d.world.routes.get(i)).map(|r| r.id)) { crate::deleting::request(app, crate::deleting::Target::Route(id)); }
    if select_route.is_some() {
        app.line.selected_route = select_route;
        app.line.selected_point = None;
        app.line.action = LineAction::Edit;
        app.mode = Mode::Lines;
    }
    if let Some(a) = shape_action {
        if matches!(a, ShapeAction::Rename(..)) { app.shape_rename = None; }
        app.shape_action(a);
    }
    if start_shape_rename.is_some() { app.shape_rename = start_shape_rename; }
    if select_modifier.is_some() { app.selected_modifier = select_modifier; }
}

fn details(app: &mut App, ui: &mut egui::Ui) {
    panel_title(ui, &t("Details"));
    egui::ScrollArea::vertical().id_salt("det").auto_shrink([false, false]).show(ui, |ui| {
        if let Some(id) = app.selected_modifier { modifier_details(app, ui, id); return; }
        if crate::library_ui::details(app, ui) { return; }
        if app.mode != Mode::Shapes && app.selection.is_empty() && !app.shape.selection.is_empty() { shape_details(app, ui); return; }
        selection_layer(app, ui);
        if app.selection.len() == 1 {
            object_details(app, ui);
        } else if app.selection.len() > 1 {
            ui.label(tf("{0} objects selected", &[&app.selection.len().to_string()]));
        } else {
            ui.weak(t("Nothing selected"));
        }
    });
}

/// Layer of the selected objects, with a way to move them to another one.
fn selection_layer(app: &mut App, ui: &mut egui::Ui) {
    let Some(doc) = &app.doc else { return };
    let Some(first) = app.selection.iter().next().and_then(|id| doc.world.objects.get(*id)) else { return };
    let mut chosen = first.layer();
    let current = chosen;
    let layers: Vec<(u16, String)> = doc.world.object_layers.iter().enumerate().filter(|(_, l)| !l.deleted).map(|(i, l)| (i as u16, l.name.clone())).collect();
    row(ui, &t("Layer"), |ui| {
        let shown = layers.iter().find(|(i, _)| *i == current).map(|(_, n)| n.clone()).unwrap_or_default();
        egui::ComboBox::from_id_salt("sel_layer").selected_text(shown).show_ui(ui, |ui| {
            for (i, n) in &layers { ui.selectable_value(&mut chosen, *i, n); }
        });
    });
    if chosen != current { app.layer_action(LayerAction::MoveSelection(chosen)); }
}

fn object_details(app: &mut App, ui: &mut egui::Ui) {
    let id = *app.selection.iter().next().unwrap();
    let Some(doc) = &mut app.doc else { return };
    let Some(before) = doc.world.objects.get(id).copied() else { return };
    let mut o = before;
    let path = doc.world.objects.model_path(&o).to_string();
    let mut done = false;
    section(ui, &icons::with(icons::ARROWS_OUT_CARDINAL, &t("Transform")), true, |ui| {
        ui.label(RichText::new(path.rsplit('\\').next().unwrap_or(&path)).color(Color32::WHITE)).on_hover_text(&path);
        let mut f = |ui: &mut egui::Ui, label: String, v: &mut f32, speed: f64| {
            row(ui, &label, |ui| {
                let r = ui.add(egui::DragValue::new(v).speed(speed).max_decimals(3));
                done |= r.drag_stopped() || r.lost_focus();
            });
        };
        f(ui, "X".into(), &mut o.x, 0.1);
        f(ui, "Z".into(), &mut o.z, 0.1);
        f(ui, t("Elevation"), &mut o.elevation, 0.02);
        f(ui, t("Yaw"), &mut o.yaw, 0.5);
        f(ui, t("Pitch"), &mut o.pitch, 0.25);
        f(ui, t("Roll"), &mut o.roll, 0.25);
        f(ui, t("Scale"), &mut o.scale, 0.01);
        let mut under = o.flags & FLAG_UNDERGROUND != 0;
        if ui.checkbox(&mut under, t("Underground")).changed() { o.flags ^= FLAG_UNDERGROUND; done = true; }
        let mut abs = o.flags & FLAG_ABSOLUTE != 0;
        if ui.checkbox(&mut abs, t("Fixed height, ignores the terrain")).on_hover_text(t("The elevation is then measured from sea level")).changed() {
            let ground = doc.world.height.sample(o.x, o.z);
            o.elevation += if abs { ground } else { -ground };
            o.flags ^= FLAG_ABSOLUTE;
            done = true;
        }
    });
    if o != before {
        doc.world.objects.update(id, o);
        doc.dirty.objects = true;
        if let Some(r) = doc.world.sync_attached(id) { app.height_changed(r); }
        app.selection_rev += 1;
    }
    if done {
        let d = app.doc.as_mut().unwrap();
        if let Some(now) = d.world.objects.get(id).copied() {
            d.history.push(t("Transform objects"), dz_core::edit::Change::Objects(vec![(id, Some(before), Some(now))]));
        }
    }
}

fn modifier_details(app: &mut App, ui: &mut egui::Ui, id: u64) {
    let Some(doc) = &mut app.doc else { return };
    let Some(index) = doc.world.modifiers.iter().position(|m| m.id == id) else { app.selected_modifier = None; return };
    let mut m = doc.world.modifiers[index].clone();
    let mut remove = false;
    let mut follow: Option<(u32, bool)> = None;
    section(ui, &icons::with(icons::MOUNTAINS, &t("Terrain modifier")), true, |ui| {
        row(ui, &t("Name"), |ui| { ui.text_edit_singleline(&mut m.name); });
        ui.checkbox(&mut m.enabled, t("Enabled"));
        row(ui, &t("Effect"), |ui| {
            egui::ComboBox::from_id_salt("mm").selected_text(match m.mode { ModMode::Flatten => t("Flatten"), ModMode::Offset => t("Offset"), ModMode::Fill => t("Fill"), ModMode::Cut => t("Cut") }).show_ui(ui, |ui| {
                ui.selectable_value(&mut m.mode, ModMode::Flatten, t("Flatten"));
                ui.selectable_value(&mut m.mode, ModMode::Offset, t("Offset"));
                ui.selectable_value(&mut m.mode, ModMode::Fill, t("Fill"));
                ui.selectable_value(&mut m.mode, ModMode::Cut, t("Cut"));
            });
        });
        match &mut m.shape {
            ModShape::Circle { radius } => slider(ui, &t("Radius"), radius, 1.0..=500.0, true),
            ModShape::Pond { depth, blend, bank, .. } => {
                slider(ui, &t("Depth"), depth, 0.3..=30.0, false);
                slider(ui, &t("Blend"), blend, 0.0..=80.0, false);
                slider(ui, &t("Bank width m"), bank, 0.0..=100.0, false);
            }
            ModShape::Line { half_width, .. } => {
                let mut width = *half_width * 2.0;
                slider(ui, &t("Width"), &mut width, 1.0..=80.0, false);
                *half_width = width * 0.5;
            }
            ModShape::Rect { half_x, half_z } => {
                slider(ui, "X", half_x, 0.5..=300.0, true);
                slider(ui, "Z", half_z, 0.5..=300.0, true);
            }
        }
        slider(ui, &t("Blend"), &mut m.falloff, 0.0..=120.0, false);
        slider(ui, &t("Height offset"), &mut m.amount, -50.0..=50.0, false);
        if let Some(object) = m.attached {
            let mut on = m.follow;
            if ui.checkbox(&mut on, t("The ground follows the height of the object")).changed() { follow = Some((object, on)); }
        }
        if m.follow {
            if let Some(v) = m.target { ui.weak(tf("Levelled to {0} m, the height of the object", &[&format!("{v:.2}")])); }
        } else {
            let mut fixed = m.target.is_some();
            if ui.checkbox(&mut fixed, t("Fixed target height")).changed() { m.target = if fixed { Some(0.0) } else { None }; }
            if let Some(v) = &mut m.target { row(ui, &t("Height"), |ui| { ui.add(egui::DragValue::new(v).speed(0.1)); }); }
        }
        if m.attached.is_none() {
            slider(ui, "X", &mut m.x, 0.0..=30000.0, false);
            slider(ui, "Z", &mut m.z, 0.0..=30000.0, false);
        }
        remove = ui.button(t("Remove modifier")).clicked();
    });
    if remove {
        crate::deleting::request(app, crate::deleting::Target::Modifier(id));
        return;
    }
    if let Some((object, on)) = follow {
        app.set_pad_follow(&[object], on);
        return;
    }
    if m != doc.world.modifiers[index] {
        let (old, new) = (doc.world.modifiers[index].clone(), m.clone());
        doc.world.modifiers[index] = m;
        let area = [doc.world.modifier_rect(&old), doc.world.modifier_rect(&new)].into_iter().flatten().reduce(|a, b| a.union(b));
        doc.history.revision += 1;
        doc.dirty.objects = true;
        if let Some(r) = area {
            doc.world.recompose(Some(r));
            app.height_changed(r);
        }
    }
}

fn bottom_panel(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        // What the log took in while it was not open is read once it is.
        if app.bottom == BottomTab::Output { app.log_seen = app.log.len(); }
        let unseen = &app.log[app.log_seen.min(app.log.len())..];
        let (errors, warnings) = (unseen.iter().filter(|l| l.level >= 2).count(), unseen.iter().filter(|l| l.level == 1).count());
        let latest = unseen.iter().rev().find(|l| l.level >= 1).map(|l| l.text.clone());
        for (tab, icon, name) in [(BottomTab::Content, icons::SQUARES_FOUR, "Content Browser"), (BottomTab::Output, icons::TERMINAL_WINDOW, "Output Log"), (BottomTab::Build, icons::HAMMER, "Build")] {
            let r = tool_button(ui, app.bottom == tab, &icons::with(icon, &t(name)));
            if tab == BottomTab::Output && errors + warnings > 0 {
                // A mark on the button: red for errors, amber for warnings, with how many.
                let (n, color, glyph) = if errors > 0 { (errors, theme::ERROR, icons::X_CIRCLE) } else { (warnings, theme::WARN, icons::WARNING) };
                let text = format!("{glyph} {n}");
                let galley = ui.painter().layout_no_wrap(text, egui::FontId::proportional(11.0), Color32::BLACK);
                let pill = egui::Rect::from_min_size(egui::pos2(r.rect.right() - galley.size().x * 0.5 - 6.0, r.rect.top() - 7.0), vec2(galley.size().x + 8.0, 15.0));
                let p = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, ui.id().with("log_badge")));
                p.rect_filled(pill, 7.5, color);
                p.galley(pill.center() - galley.size() * 0.5, galley, Color32::BLACK);
                let mut tip = tf("{0} errors, {1} warnings since the log was last open", &[&errors.to_string(), &warnings.to_string()]);
                if let Some(l) = &latest { tip = format!("{tip}\n{l}"); }
                let r = r.on_hover_text(tip);
                if r.clicked() { app.bottom = tab; }
            } else if r.clicked() {
                app.bottom = tab;
            }
        }
        if app.bottom == BottomTab::Content {
            ui.separator();
            ui.add(egui::Slider::new(&mut app.settings.library_tile, 48.0..=240.0).show_value(false).text(icons::MAGNIFYING_GLASS_PLUS));
        }
    });
    match app.bottom {
        BottomTab::Content => crate::asset_browser::ui(app, ui),
        BottomTab::Output => output_log(app, ui),
        BottomTab::Build => build_tab(app, ui),
    }
}

fn output_log(app: &mut App, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show_rows(ui, 15.0, app.log.len(), |ui, range| {
        for l in &app.log[range] {
            let c = match l.level { 2 => theme::ERROR, 1 => theme::WARN, _ => theme::TEXT };
            ui.label(RichText::new(&l.text).monospace().color(c));
        }
    });
}

fn build_tab(app: &mut App, ui: &mut egui::Ui) {
    ui.columns(2, |c| {
        let ui = &mut c[0];
        ui.heading(t("Package the map"));
        let problems = app.settings.tools.check();
        if problems.is_empty() {
            ui.colored_label(theme::OK, t("DayZ Tools, game and work drive found"));
        }
        for p in problems { ui.colored_label(theme::WARN, p); }
        ui.horizontal_wrapped(|ui| {
            let free = app.doc.is_some() && app.task.is_none();
            if ui.add_enabled(free, egui::Button::new(t("1. Export files"))).clicked() { app.export_map(false); }
            if ui.add_enabled(free, egui::Button::new(t("2. Build PBO"))).clicked() { app.build(false); }
            if ui.add_enabled(free, egui::Button::new(t("3. Build and play"))).clicked() { app.build(true); }
            if ui.add_enabled(free, egui::Button::new(t("Generate navmesh..."))).clicked() { app.generate_navmesh(); }
            if let Some(d) = &app.doc {
                let folder = app.settings.tools.work_drive.join(&d.world.meta.map_id);
                if ui.button(t("Open map folder")).clicked() { app.open_folder(&folder); }
            }
        });
        ui.checkbox(&mut app.settings.with_client, t("Start the game client too"));
        ui.label(t("Additional mods"));
        let mut mods: Vec<String> = app.settings.extra_mods.split(';').map(str::trim).filter(|m| !m.is_empty()).map(String::from).collect();
        let mut remove = None;
        for (i, m) in mods.iter().enumerate() {
            ui.push_id(("mod", i), |ui| ui.horizontal(|ui| {
                if ui.small_button("✖").clicked() { remove = Some(i); }
                ui.label(m);
            }));
        }
        let mut changed = false;
        if let Some(i) = remove { mods.remove(i); changed = true; }
        if ui.button(format!("+ {}", t("Add mod folder"))).clicked() {
            for dir in rfd::FileDialog::new().set_title(t("Add mod folder")).pick_folders().unwrap_or_default() {
                let path = dir.display().to_string();
                if !mods.contains(&path) { mods.push(path); changed = true; }
            }
        }
        if changed { app.settings.extra_mods = mods.join(";"); }
        let ui = &mut c[1];
        ui.label(t("Log"));
        egui::ScrollArea::vertical().id_salt("b").stick_to_bottom(true).auto_shrink([false, false]).show_rows(ui, 14.0, app.log.len(), |ui, range| {
            for l in &app.log[range] { ui.label(RichText::new(&l.text).small().monospace()); }
        });
    });
}

fn path_choice(ui: &mut egui::Ui, label: &str, value: &mut Option<std::path::PathBuf>, choices: &[std::path::PathBuf], pick: impl FnOnce() -> Option<std::path::PathBuf>) {
    row(ui, label, |ui| {
        let shown = value.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| t("(none)"));
        egui::ComboBox::from_id_salt(label).width(ui.available_width() - 36.0).truncate().selected_text(shown).show_ui(ui, |ui| {
            ui.selectable_value(value, None, t("(none)"));
            for c in choices { ui.selectable_value(value, Some(c.clone()), c.display().to_string()); }
        });
        if ui.button("…").clicked() {
            if let Some(p) = pick() { *value = Some(p); }
        }
    });
}

fn import_window(app: &mut App, ui: &mut egui::Ui, d: &mut ImportDialog, close: &mut bool) {
    let ImportDialog { plan, scan } = d;
    let file = |name: &'static str, ext: &'static [&'static str]| move || rfd::FileDialog::new().add_filter(name, ext).pick_file();
    let folder = || rfd::FileDialog::new().pick_folder();
    row(ui, &t("Name"), |ui| { ui.text_edit_singleline(&mut plan.name); });
    ui.add_space(4.0);
    path_choice(ui, &t("Heightmap"), &mut plan.asc, &scan.asc, file("ASC", &["asc"]));
    path_choice(ui, &t("Built world (WRP)"), &mut plan.wrp, &scan.wrp, file("WRP", &["wrp"]));
    let tiles: Vec<_> = scan.tile_dir.iter().cloned().collect();
    path_choice(ui, &t("Generated tiles"), &mut plan.tile_dir, &tiles, folder);
    path_choice(ui, &t("Satellite"), &mut plan.satellite, &scan.satellite, file("Image", &["bmp", "png", "tif"]));
    path_choice(ui, &t("Mask"), &mut plan.mask, &scan.mask, file("Image", &["bmp", "png", "tif"]));
    path_choice(ui, &t("Surfaces (layers.cfg)"), &mut plan.layers_cfg, &scan.layers_cfg, file("layers.cfg", &["cfg"]));
    path_choice(ui, &t("Terrain holes (holes.cfg)"), &mut plan.holes_cfg, &scan.holes_cfg, file("holes.cfg", &["cfg"]));
    path_choice(ui, &t("Roads (TV4P)"), &mut plan.tv4p, &scan.tv4p, file("Terrain Builder", &["tv4p"]));
    ui.weak(if plan.asc.is_some() { t("Terrain comes from the heightmap.") } else if plan.wrp.is_some() { t("Terrain comes from the WRP.") } else { t("Choose a heightmap or a WRP.") });
    row(ui, &t("Surfaces per tile"), |ui| { ui.add(egui::Slider::new(&mut plan.max_tile_surfaces, 1..=6)); });
    ui.separator();
    ui.add_enabled(plan.wrp.is_some(), egui::Checkbox::new(&mut plan.objects_from_wrp, t("Objects from the WRP")));
    ui.horizontal(|ui| {
        ui.strong(t("Terrain Builder layers"));
        if ui.small_button(t("All")).clicked() { plan.layers.iter_mut().for_each(|l| l.import = true); }
        if ui.small_button(t("Visible only")).clicked() { plan.layers.iter_mut().for_each(|l| l.import = l.info.visible && l.info.objects > 0); }
        if ui.small_button(t("None")).clicked() { plan.layers.iter_mut().for_each(|l| l.import = false); }
        if ui.small_button(t("Add layers...")).clicked() {
            for p in rfd::FileDialog::new().add_filter("Terrain Builder", &["tv4l"]).pick_files().unwrap_or_default() {
                if plan.layers.iter().any(|l| l.path == p) { continue; }
                if let Ok(info) = dz_core::tv4l::read_info(&p) { plan.layers.push(dz_core::import::LayerFile { path: p, info, import: true }); }
            }
        }
    });
    egui::ScrollArea::vertical().id_salt("import_layers").max_height(190.0).auto_shrink([false, true]).show(ui, |ui| {
        for l in &mut plan.layers {
            ui.horizontal(|ui| {
                ui.checkbox(&mut l.import, &l.info.name).on_hover_text(l.path.display().to_string());
                ui.weak(l.info.objects.to_string());
                if !l.info.visible { ui.weak(t("hidden in Terrain Builder")); }
            });
        }
    });
    if !plan.shapes.is_empty() {
        ui.strong(t("Shapes"));
        egui::ScrollArea::vertical().id_salt("import_shapes").max_height(110.0).auto_shrink([false, true]).show(ui, |ui| {
            for f in &mut plan.shapes {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut f.import, &f.name).on_hover_text(f.path.display().to_string());
                    ui.weak(format!("{}  {}", f.shapes, f.path.extension().map(|e| e.to_string_lossy().to_ascii_uppercase()).unwrap_or_default()));
                });
            }
        });
    }
    let chosen: u64 = plan.layers.iter().filter(|l| l.import).map(|l| l.info.objects as u64).sum();
    row(ui, &t("Template libraries"), |ui| {
        ui.label(tf("{0} files", &[&plan.libraries.len().to_string()]));
        if ui.button("…").clicked() {
            if let Some(d) = rfd::FileDialog::new().pick_folder() { plan.libraries = dz_core::tv4l::libraries_in(&d); }
        }
    });
    if chosen > 0 && plan.libraries.is_empty() {
        ui.colored_label(Color32::from_rgb(235, 170, 70), t("Without template libraries the layers cannot be matched to models."));
    }
    if !plan.lists.is_empty() {
        ui.strong(t("Object lists (TXT)"));
        egui::ScrollArea::vertical().id_salt("import_lists").max_height(100.0).auto_shrink([false, true]).show(ui, |ui| {
            for (p, on) in &mut plan.lists {
                ui.checkbox(on, p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()).on_hover_text(p.display().to_string());
            }
        });
    }
    ui.separator();
    ui.horizontal(|ui| {
        if ui.add_enabled(plan.can_import(), egui::Button::new(t("Import"))).clicked() {
            app.run_import(plan.clone());
            *close = true;
        }
        if ui.button(t("Cancel")).clicked() { *close = true; }
        if chosen > 0 { ui.weak(tf("{0} objects will be imported", &[&chosen.to_string()])); }
    });
}

fn dialogs(app: &mut App, ctx: &egui::Context) {
    let mut dialog = std::mem::replace(&mut app.dialog, Dialog::None);
    let mut open = true;
    let mut close = false;
    match &mut dialog {
        Dialog::None => {}
        Dialog::NewProject(p) => {
            egui::Window::new(t("New project")).collapsible(false).resizable(false).open(&mut open).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                row(ui, &t("Name"), |ui| { ui.text_edit_singleline(&mut p.name); });
                row(ui, &t("Size"), |ui| {
                    let text = |i: usize| { let (n, c) = SIZE_PRESETS[i]; format!("{n} × {c} m  =  {} m", n as f32 * c) };
                    egui::ComboBox::from_id_salt("size").width(220.0).selected_text(text(p.preset)).show_ui(ui, |ui| {
                        for i in 0..SIZE_PRESETS.len() { ui.selectable_value(&mut p.preset, i, text(i)); }
                    });
                });
                row(ui, &t("Base height"), |ui| { ui.add(egui::DragValue::new(&mut p.base).speed(1.0).suffix(" m")); });
                row(ui, &t("Folder"), |ui| {
                    ui.add(egui::TextEdit::singleline(&mut p.folder).desired_width(200.0));
                    if ui.button("…").clicked() {
                        if let Some(d) = rfd::FileDialog::new().pick_folder() { p.folder = d.display().to_string(); }
                    }
                });
                ui.add_space(6.0);
                if ui.button(t("Create")).clicked() {
                    app.create_project(p);
                    close = true;
                }
            });
        }
        Dialog::Import(d) => {
            egui::Window::new(t("Import map")).collapsible(false).resizable(true).default_width(760.0).open(&mut open).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| import_window(app, ui, d, &mut close));
        }
        Dialog::RoadCatalog => crate::library_ui::road_catalog_window(app, ctx, &mut open),
        Dialog::Settings => settings_window(app, ctx, &mut open),
        Dialog::Delete(plan) => {
            if crate::deleting::window(app, ctx, plan, &mut open) { close = true; }
        }
        Dialog::Shortcuts => {
            let mut keys = app.settings.keys.clone();
            crate::keys::window(ctx, &mut keys, &mut app.key_capture, &mut open);
            if keys != app.settings.keys {
                app.settings.keys = keys;
                app.settings.save();
            }
        }
        Dialog::ProjectSettings => project_window(app, ctx, &mut open),
        Dialog::Export(only) => {
            if crate::exporting::window(app, ctx, *only, &mut open) { close = true; }
        }
        Dialog::About => {
            egui::Window::new(t("About")).collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
                ui.heading("DZ World Editor");
                ui.label(format!("{} {}", t("Version"), crate::updater::version()));
                ui.label(t("Built on the DayZForests and DayZ Road Studio engines."));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(icons::GITHUB_LOGO);
                    ui.hyperlink_to(t("Source code on GitHub"), REPOSITORY).on_hover_text(REPOSITORY);
                });
                ui.horizontal(|ui| {
                    ui.label(icons::COFFEE);
                    ui.hyperlink_to(t("Buy me a coffee"), BUY_ME_A_COFFEE).on_hover_text(BUY_ME_A_COFFEE);
                });
            });
        }
        Dialog::Confirm { text, action } => {
            let action = *action;
            let text = text.clone();
            egui::Window::new(t("Confirm")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.label(text);
                ui.horizontal(|ui| {
                    if ui.button(t("Yes")).clicked() { action(app); close = true; }
                    if ui.button(t("No")).clicked() { close = true; }
                });
            });
        }
    }
    if open && !close && matches!(app.dialog, Dialog::None) {
        app.dialog = dialog;
    }
}

fn settings_window(app: &mut App, ctx: &egui::Context, open: &mut bool) {
    egui::Window::new(t("Settings")).collapsible(false).default_width(520.0).open(open).show(ctx, |ui| {
        section(ui, &icons::with(icons::FOLDER_OPEN, &t("Paths")), true, |ui| {
            let mut changed = false;
            let mut path_row = |ui: &mut egui::Ui, label: String, p: &mut std::path::PathBuf| {
                row(ui, &label, |ui| {
                    let mut s = p.display().to_string();
                    if ui.add(egui::TextEdit::singleline(&mut s).desired_width(300.0)).changed() { *p = s.into(); changed = true; }
                    if ui.button("…").clicked() {
                        if let Some(d) = rfd::FileDialog::new().pick_folder() { *p = d; changed = true; }
                    }
                });
            };
            path_row(ui, t("Work drive"), &mut app.settings.tools.work_drive);
            path_row(ui, t("DayZ Tools"), &mut app.settings.tools.dayz_tools);
            path_row(ui, t("DayZ"), &mut app.settings.tools.dayz);
            if ui.button(t("Detect automatically")).clicked() { app.settings.tools = dz_core::build::detect(); changed = true; }
            if changed { app.db = dz_core::model_info::ModelDb::new(app.settings.tools.work_drive.clone()); }
            for p in app.settings.tools.check() { ui.colored_label(theme::WARN, p); }
        });
        section(ui, &icons::with(icons::EYE, &t("Display")), true, |ui| {
            row(ui, &t("Language"), |ui| {
                let cur = crate::i18n::current();
                let names = crate::i18n::languages();
                let label = names.iter().find(|(c, _)| *c == cur).map(|(_, n)| n.clone()).unwrap_or_default();
                let mut pick = None;
                egui::ComboBox::from_id_salt("lang").selected_text(label).show_ui(ui, |ui| {
                    for (c, n) in names { if ui.selectable_label(c == cur, n).clicked() { pick = Some(c); } }
                });
                if let Some(c) = pick { app.sync_language(&c); }
            });
            let before = app.settings.ui_scale;
            row(ui, &t("Interface scale"), |ui| { ui.add(egui::Slider::new(&mut app.settings.ui_scale, 0.75..=2.0)); });
            if before != app.settings.ui_scale { theme::apply(ctx, app.settings.ui_scale); }
        });
        section(ui, &icons::with(icons::SUN, &t("Graphics")), true, |ui| {
            row(ui, &t("Quality"), |ui| {
                let current = app.settings.graphics_preset();
                let mut pick = None;
                egui::ComboBox::from_id_salt("graphics_preset").selected_text(current.map_or(t("Custom"), |p| t(p.name()))).show_ui(ui, |ui| {
                    for p in crate::settings::GraphicsPreset::ALL { if ui.selectable_label(current == Some(p), t(p.name())).clicked() { pick = Some(p); } }
                });
                if let Some(p) = pick { app.settings.apply_graphics(p); }
            });
            slider(ui, &t("Terrain detail"), &mut app.settings.terrain_quality, 0.6..=3.0, false);
            slider(ui, &t("Object draw distance"), &mut app.settings.draw_distance, 300.0..=8000.0, true);
            slider(ui, &t("Full detail distance"), &mut app.settings.near_detail, 20.0..=5000.0, true);
            slider(ui, &t("Surface texture range m"), &mut app.settings.surface_range, 60.0..=5000.0, true);
            slider(ui, &t("Surface parallax cm"), &mut app.settings.surface_parallax, 0.0..=15.0, false);
            slider(ui, &t("Surface blend by relief"), &mut app.settings.surface_blend, 0.0..=1.0, false);
            let choice = |ui: &mut egui::Ui, label: &str, id: &str, value: &mut u32, options: &[(u32, String)]| {
                row(ui, label, |ui| {
                    let shown = options.iter().find(|o| o.0 == *value).map_or_else(|| value.to_string(), |o| o.1.clone());
                    egui::ComboBox::from_id_salt(id).selected_text(shown).show_ui(ui, |ui| {
                        for (v, name) in options { ui.selectable_value(value, *v, name.as_str()); }
                    });
                });
            };
            choice(ui, &t("Texture detail"), "texture_size", &mut app.settings.texture_size, &[(512, "512".into()), (1024, "1024".into()), (2048, "2048".into()), (4096, t("Full"))]);
            choice(ui, &t("Texture filtering"), "anisotropy", &mut app.settings.anisotropy, &[(1, t("Off")), (2, "2x".into()), (4, "4x".into()), (8, "8x".into()), (16, "16x".into())]);
            choice(ui, &t("Shadow detail"), "shadow_map", &mut app.settings.shadow_map, &[(2048, "2048".into()), (4096, "4096".into()), (8192, "8192".into())]);
            slider(ui, &t("Shadow range"), &mut app.settings.lighting.shadow_range, 60.0..=2000.0, true);
            choice(ui, &t("Antialiasing"), "edge_smoothing", &mut app.settings.edge_smoothing, &[(0, t("Off")), (1, "FXAA".into()), (2, "FXAA + 1.5x".into()), (3, "FXAA + 2x".into())]);
            choice(ui, &t("Ambient occlusion"), "ambient_occlusion", &mut app.settings.ambient_occlusion, &[(0, t("Off")), (8, t("Low")), (12, t("Medium")), (20, t("High"))]);
            slider(ui, &t("Ground colour on grass"), &mut app.settings.grass_ground_color, 0.0..=1.0, false);
        });
        section(ui, &icons::with(icons::GEAR, &t("General")), true, |ui| {
            row(ui, &t("Autosave every"), |ui| { ui.add(egui::Slider::new(&mut app.settings.autosave_minutes, 0..=60).suffix(" min")); });
            ui.checkbox(&mut app.settings.check_updates, t("Check for updates at start"));
        });
        section(ui, &icons::with(icons::CLOCK, &t("Backups")), true, |ui| {
            ui.checkbox(&mut app.settings.backup_enabled, t("Make backups automatically"));
            ui.add_enabled_ui(app.settings.backup_enabled, |ui| {
                row(ui, &t("Back up every"), |ui| { ui.add(egui::Slider::new(&mut app.settings.backup_minutes, 5..=240).suffix(" min")); });
                row(ui, &t("Backups to keep"), |ui| { ui.add(egui::Slider::new(&mut app.settings.backup_keep, 1..=100)); });
            });
            ui.weak(t("A backup is only made when the project has changed. Files that did not change are shared between backups, so they take no extra space."));
            row(ui, &t("Folder"), |ui| {
                let shown = if app.settings.backup_folder.is_empty() { t("backups in the project folder") } else { app.settings.backup_folder.clone() };
                ui.add(egui::Label::new(shown).truncate());
            });
            ui.horizontal(|ui| {
                if ui.button(t("Choose...")).clicked() {
                    if let Some(d) = rfd::FileDialog::new().pick_folder() { app.settings.backup_folder = d.display().to_string(); }
                }
                if ui.add_enabled(!app.settings.backup_folder.is_empty(), egui::Button::new(t("Use the project folder"))).clicked() { app.settings.backup_folder.clear(); }
            });
            ui.horizontal(|ui| {
                let saved = app.doc.as_ref().is_some_and(|d| d.dir.is_some());
                if ui.add_enabled(saved && app.backup_job.is_none(), egui::Button::new(t("Back up now"))).clicked() { app.start_backup(true); }
                if ui.add_enabled(saved, egui::Button::new(t("Open backups folder"))).clicked() {
                    if let Some(root) = app.backup_root() {
                        let _ = std::fs::create_dir_all(&root);
                        app.open_folder(&root);
                    }
                }
            });
            ui.weak(t("To restore a backup, open the project.dzwe inside it and use Save as."));
        });
    });
    if !*open { app.settings.save(); }
}

fn project_window(app: &mut App, ctx: &egui::Context, open: &mut bool) {
    let Some(doc) = &mut app.doc else { *open = false; return };
    let before = doc.world.meta.clone();
    let issues = app.tile_issues.clone();
    let (mut check, mut fix) = (false, false);
    let m = &mut doc.world.meta;
    egui::Window::new(t("Project settings")).collapsible(false).open(open).show(ctx, |ui| {
        row(ui, &t("Name"), |ui| { ui.text_edit_singleline(&mut m.name); });
        row(ui, &t("Map ID"), |ui| { ui.text_edit_singleline(&mut m.map_id); });
        row(ui, &t("Author"), |ui| { ui.text_edit_singleline(&mut m.author); });
        row(ui, &t("Sea level"), |ui| { ui.add(egui::DragValue::new(&mut m.sea_level).speed(0.1).suffix(" m")); });
        row(ui, &t("Easting offset"), |ui| { ui.add(egui::DragValue::new(&mut m.easting).speed(100.0)); });
        row(ui, &t("Latitude"), |ui| { ui.add(egui::DragValue::new(&mut m.latitude).speed(0.1)); });
        row(ui, &t("Longitude"), |ui| { ui.add(egui::DragValue::new(&mut m.longitude).speed(0.1)); });
        row(ui, &t("Tile size"), |ui| {
            egui::ComboBox::from_id_salt("tilepx").selected_text(format!("{}", m.tile_px)).show_ui(ui, |ui| {
                for v in [256u32, 512, 1024, 2048] { ui.selectable_value(&mut m.tile_px, v, v.to_string()); }
            });
        });
        row(ui, &t("Surfaces per tile"), |ui| { ui.add(egui::Slider::new(&mut m.max_tile_surfaces, 1..=6)); });
        ui.weak(t("When a tile really uses 6, the whole map is exported like Terrain Builder's 6-material mode (TerrainX), without satellite normal maps."));
        ui.horizontal(|ui| {
            check = ui.button(t("Check tiles")).clicked();
            fix = ui.button(t("Fix tiles")).on_hover_text(t("Replaces the rarest surfaces of a tile with its main surface.")).clicked();
        });
        if !issues.is_empty() { ui.colored_label(Color32::from_rgb(235, 110, 90), tf("{0} tiles over the limit, marked in the 2D view", &[&issues.len().to_string()])); }
        ui.checkbox(&mut m.write_underground_flag, t("Write the newer WRP format with underground flag"));
        ui.separator();
        ui.label(tf("Terrain {0} × {0}, cell {1} m, map {2} m", &[&m.terrain_size.to_string(), &m.terrain_cell.to_string(), &m.size_m().to_string()]));
        ui.label(tf("Land grid {0}, cell {1} m, {2} tiles per side", &[&m.land_size().to_string(), &m.land_cell().to_string(), &m.tiles().to_string()]));
    });
    if doc.world.meta != before {
        doc.history.revision += 1;
    }
    if check {
        app.tile_issues = dz_core::mask_limit::scan(&doc.world);
        let n = app.tile_issues.len();
        app.info(tf("{0} tiles over the limit, marked in the 2D view", &[&n.to_string()]));
    }
    if fix {
        let Some(doc) = &app.doc else { return };
        let mut w = doc.world.lite();
        dz_core::mask_limit::enforce(&mut w);
        let data = w.mask.data;
        app.tile_issues.clear();
        app.apply_mask(&t("Fix tiles"), data);
    }
}
