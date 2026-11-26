//! Window layout modelled on the Unreal editor: menu and toolbar on top, modes on the left,
//! outliner and details on the right, content browser and log at the bottom.
use crate::road_draw::RoadDraw;
use crate::app::*;
use crate::i18n::{t, tf};
use crate::icons;
use crate::keys::{Action, Hold};
use crate::theme::{self, row, section, tool_button};
use crate::viewport;
use dz_core::edit::SculptTool;
use dz_core::objects::{FLAG_ABSOLUTE, FLAG_UNDERGROUND};
use dz_core::world::{ModMode, ModShape, SurfaceLayer};
use eframe::egui::{self, pos2, vec2, Color32, Rect, RichText};

pub fn new_project_dialog() -> Dialog {
    Dialog::NewProject(NewProject { name: "NewMap".into(), preset: 4, base: 60.0, folder: String::new(), real: false, place: Default::default() })
}

/// The New project window set to make the map of a real place.
pub fn real_place_dialog() -> Dialog {
    let mut d = new_project_dialog();
    if let Dialog::NewProject(p) = &mut d {
        p.real = true;
        p.name = "RealMap".into();
    }
    d
}

pub fn ui(app: &mut App, ctx: &egui::Context) {
    guard_close(app, ctx);
    app.reveal_asked(ctx);
    app.finish_grading_edit(ctx);
    app.finish_layer_colour_edit(ctx);
    crate::library_ui::handle_file_drop(app, ctx);
    if app.lib_dirty && !ctx.input(|i| i.pointer.any_down()) {
        app.lib_dirty = false;
        if let Err(e) = app.library.save(&crate::library_ui::library_path()) { app.error(format!("{e:#}")); }
    }
    if ctx.input(|i| i.key_pressed(egui::Key::F11)) && !ctx.wants_keyboard_input() {
        set_fullscreen(app, ctx, !app.fullscreen);
    }
    if app.fullscreen && app.doc.is_some() && !app.detached {
        // The view takes the whole screen; the panels slide in over it at the edges.
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::DARK)).show(ctx, |ui| viewport::central(app, ui));
        edge_panels(app, ctx);
    } else {
        menu_bar(app, ctx);
        toolbar(app, ctx);
        status_bar(app, ctx);
        crate::layout::docked(app, ctx);
    }
    if app.detached {
        detached_view(app, ctx);
    }
    dialogs(app, ctx);
    crate::mission_ui::window(app, ctx);
    crate::loot_proto_ui::window(app, ctx);
    crate::map_check::window(app, ctx);
    crate::goto::window(app, ctx);
    crate::team_ui::window(app, ctx);
    app.updater.window(ctx);
    crate::asset_browser::drag_ghost(app, ctx);
}

/// Turns the full screen view on or off, for the window as well.
pub fn set_fullscreen(app: &mut App, ctx: &egui::Context, on: bool) {
    app.fullscreen = on;
    app.edge_open = [false; 4];
    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
}

/// The full screen view's panels (see [`crate::layout::edge_panels`]), with the work going on
/// and a hint where the panels are.
fn edge_panels(app: &mut App, ctx: &egui::Context) {
    let screen = ctx.screen_rect();
    crate::layout::edge_panels(app, ctx);
    let frame = egui::Frame::new().fill(theme::PANEL).stroke(egui::Stroke::new(1.0_f32, egui::Color32::from_gray(60))).inner_margin(egui::Margin::same(6)).corner_radius(4.0);
    // Work going on in the background, which the hidden status bar would show.
    if let Some(task) = &app.task {
        let (title, stage, progress) = (task.title.clone(), task.job.current_stage(), task.job.progress());
        let mut cancel = false;
        let at = pos2(screen.center().x - 200.0, screen.bottom() - if app.edge_open[3] { 320.0 } else { 46.0 });
        egui::Area::new(egui::Id::new("fullscreen_task")).order(egui::Order::Foreground).fixed_pos(at).show(ctx, |ui| {
            frame.show(ui, |ui| {
                ui.set_width(400.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.add(egui::ProgressBar::new(progress).desired_width(300.0).text(format!("{title}: {stage}")));
                    cancel = ui.small_button(t("Cancel")).clicked();
                });
            });
        });
        if cancel { task.job.cancel(); }
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
    // A hint where the panels are, faint until used.
    if crate::layout::none_open(app) {
        let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("edge_hint")));
        p.text(pos2(screen.center().x, screen.top() + 6.0), egui::Align2::CENTER_TOP, t("Move to an edge for the panels · F11 leaves full screen"), egui::FontId::proportional(11.0), egui::Color32::from_white_alpha(70));
    }
}

/// What stands in the main window while the view is in a window of its own.
pub(crate) fn detached_placeholder(app: &mut App, ui: &mut egui::Ui) {
    ui.centered_and_justified(|ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.4);
            ui.label(t("The view is in a window of its own."));
            if ui.button(t("Bring it back")).clicked() { app.detached = false; }
        });
    });
}

/// The view in a window of its own, sized and placed at will.
fn detached_view(app: &mut App, ctx: &egui::Context) {
    let id = egui::ViewportId::from_hash_of("dzwe_view");
    let builder = egui::ViewportBuilder::default().with_title(format!("DZ World Editor – {}", t("View"))).with_inner_size([1280.0, 800.0]).with_min_inner_size([320.0, 240.0]).with_icon(crate::icons::window_icon());
    ctx.show_viewport_immediate(id, builder, |ctx, _class| {
        if ctx.input(|i| i.viewport().close_requested()) {
            app.detached = false;
            return;
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::DARK)).show(ctx, |ui| viewport::central(app, ui));
        app.handle_screenshots(ctx);
    });
}

/// What the viewports draw; shared by the View menu and the viewport toolbar.
pub fn visibility_toggles(app: &mut App, ui: &mut egui::Ui) {
    let keys = app.settings.keys.clone();
    ui.checkbox(&mut app.show_objects, t("Objects")).on_hover_text(keys.tip(Action::ToggleObjects));
    ui.checkbox(&mut app.show_grass, t("Grass")).on_hover_text(keys.tip(Action::ToggleGrass));
    ui.add_enabled(app.show_grass, egui::Checkbox::new(&mut app.grass_under_objects, t("Grass under objects")));
    ui.add_enabled(app.show_objects, egui::Checkbox::new(&mut app.show_proxies, t("Proxies in buildings"))).on_hover_text(t("Furniture and fittings that buildings place in their visual LOD, as the game shows them. Proxies of the geometry, fire, memory and roadway LODs are not drawn."));
    ui.checkbox(&mut app.show_shadows, t("Shadows"));
    ui.checkbox(&mut app.show_water, t("Water"));
    ui.checkbox(&mut app.show_lines, t("Roads and lines")).on_hover_text(keys.tip(Action::ToggleShapes));
    ui.checkbox(&mut app.show_shapes, t("Shapes")).on_hover_text(keys.tip(Action::ToggleShapes));
    ui.checkbox(&mut app.show_keypoints, t("Key points"));
    if ui.checkbox(&mut app.settings.minimap, t("Minimap")).changed() { app.settings.save(); }
    if ui.checkbox(&mut app.settings.show_fps, t("Frames per second")).changed() { app.settings.save(); }
    ui.checkbox(&mut app.show_contours, t("Contour lines"));
    if app.show_contours {
        row(ui, &t("Interval"), |ui| { ui.add(egui::DragValue::new(&mut app.contour_interval).speed(0.5).range(1.0..=200.0).suffix(" m")); });
    }
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

/// True while the editor draws its own title bar: set so, and not in full screen.
pub(crate) fn own_title_bar(app: &App) -> bool {
    app.settings.own_title_bar && !app.fullscreen
}

/// The menu bar; with the editor's own title bar it is the title bar too: the window is moved
/// by dragging it, a double click fills the screen, and its buttons are at the right.
fn menu_bar(app: &mut App, ctx: &egui::Context) {
    let own = own_title_bar(app);
    let panel = egui::TopBottomPanel::top("menu").frame(egui::Frame::new().fill(theme::STRIP).inner_margin(egui::Margin::symmetric(6, if own { 0 } else { 3 })));
    let panel = if own { panel.exact_height(34.0) } else { panel };
    panel.show(ctx, |ui| {
        if own {
            // Under everything else of the bar, so the menus and buttons still take their clicks.
            let bar = ui.max_rect();
            let r = ui.interact(bar, egui::Id::new("title_drag"), egui::Sense::click_and_drag());
            if r.double_clicked() {
                let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
            } else if r.drag_started_by(egui::PointerButton::Primary) {
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            ui.add_space(5.0);
        }
        // Everything of the bar in the middle of its height.
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| menu_contents(app, ui));
    });
    if own { resize_edges(ctx); }
}

/// Without the Windows frame, the edges of the window are grabbed here to make it bigger.
fn resize_edges(ctx: &egui::Context) {
    use egui::viewport::ResizeDirection as D;
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false)) { return; }
    let Some(p) = ctx.input(|i| i.pointer.hover_pos()) else { return };
    let r = ctx.screen_rect();
    const E: f32 = 5.0;
    let (l, rt, t, b) = (p.x - r.left() < E, r.right() - p.x < E, p.y - r.top() < E, r.bottom() - p.y < E);
    let dir = match (l, rt, t, b) {
        (true, _, true, _) => D::NorthWest,
        (_, true, true, _) => D::NorthEast,
        (true, _, _, true) => D::SouthWest,
        (_, true, _, true) => D::SouthEast,
        (true, ..) => D::West,
        (_, true, ..) => D::East,
        (_, _, true, _) => D::North,
        (.., true) => D::South,
        _ => return,
    };
    ctx.set_cursor_icon(match dir {
        D::NorthWest | D::SouthEast => egui::CursorIcon::ResizeNwSe,
        D::NorthEast | D::SouthWest => egui::CursorIcon::ResizeNeSw,
        D::West | D::East => egui::CursorIcon::ResizeHorizontal,
        _ => egui::CursorIcon::ResizeVertical,
    });
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
    }
}

/// The program's icon at the left of the title bar.
fn title_icon(ui: &mut egui::Ui) {
    let id = egui::Id::new("app_icon_texture");
    let tex = ui.ctx().data(|d| d.get_temp::<Option<egui::TextureHandle>>(id)).unwrap_or_else(|| {
        let t = crate::icons::app_icon_png(48).and_then(|png| image::load_from_memory(png).ok()).map(|img| {
            let img = img.to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            ui.ctx().load_texture("app_icon", egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()), egui::TextureOptions::LINEAR)
        });
        ui.ctx().data_mut(|d| d.insert_temp(id, t.clone()));
        t
    });
    if let Some(tex) = tex {
        ui.add(egui::Image::new((tex.id(), vec2(18.0, 18.0))));
        ui.add_space(6.0);
    }
}

/// The buttons of the window at the right of the title bar, drawn as Windows draws them:
/// minimise, maximise or restore, and close, red under the pointer.
fn window_buttons(ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    // As high as the row of the menus; the colour under the pointer fills the bar's height.
    let h = ui.available_height();
    let full = ui.clip_rect().y_range();
    let button = |ui: &mut egui::Ui, icon: &str, tip: String, close: bool| -> bool {
        let (rect, resp) = ui.allocate_exact_size(vec2(46.0, h), egui::Sense::click());
        let p = ui.painter();
        if resp.hovered() { p.rect_filled(egui::Rect::from_x_y_ranges(rect.x_range(), full), 0.0, if close { Color32::from_rgb(196, 43, 28) } else { theme::HOVER }); }
        p.text(rect.center(), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(14.0), if resp.hovered() && close { Color32::WHITE } else { theme::TEXT });
        resp.on_hover_text(tip).clicked()
    };
    ui.spacing_mut().item_spacing.x = 0.0;
    // Right to left: close first.
    if button(ui, icons::X, t("Close"), true) { ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
    if button(ui, if max { icons::COPY_SIMPLE } else { icons::SQUARE }, if max { t("Restore") } else { t("Maximise") }, false) { ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max)); }
    if button(ui, icons::MINUS, t("Minimise"), false) { ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)); }
    ui.add_space(6.0);
}

pub(crate) fn menu_contents(app: &mut App, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let ctx = &ctx;
    let _ = ctx;
        egui::menu::bar(ui, |ui| {
            if own_title_bar(app) { title_icon(ui); }
            let has = app.doc.is_some();
            let keys = app.settings.keys.clone();
            ui.menu_button(t("File"), |ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); 
                if ui.add(egui::Button::new(icons::with(icons::FILE_PLUS, &t("New project..."))).shortcut_text(keys.text(Action::NewProject))).clicked() { app.dialog = new_project_dialog(); ui.close_menu(); }
                if ui.button(icons::with(icons::GLOBE_HEMISPHERE_EAST, &t("New project from a real place..."))).clicked() { app.dialog = real_place_dialog(); ui.close_menu(); }
                if ui.add(egui::Button::new(icons::with(icons::FOLDER_OPEN, &t("Open project..."))).shortcut_text(keys.text(Action::Open))).clicked() { ui.close_menu(); app.pick_and_open(); }
                ui.menu_button(icons::with(icons::CLOCK, &t("Recent projects")), |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    let list = app.settings.recent.clone();
                    if list.is_empty() {
                        ui.weak(t("No recent projects"));
                    } else {
                        for d in list {
                            ui.horizontal(|ui| {
                                if ui.button(d.display().to_string()).clicked() {
                                    ui.close_menu();
                                    app.open_project(d.clone());
                                }
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button(icons::X).on_hover_text(t("Remove from recent projects")).clicked() {
                                        app.settings.forget_recent(&d);
                                    }
                                });
                            });
                        }
                        ui.separator();
                        if ui.button(icons::with(icons::TRASH, &t("Clear recent projects"))).clicked() {
                            app.settings.clear_recent();
                        }
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
                        if ui.button(t("DayZ Editor / Object Spawner (.dze, .json)...")).clicked() { ui.close_menu(); app.import_dayz_objects(); }
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
                if ui.add_enabled(app.doc.is_some(), egui::Button::new(icons::with(icons::FILE_CODE, &t("Map config...")))).on_hover_text(t("The world config (CfgWorlds) of the map, written to config.cpp on export.")).clicked() { app.dialog = Dialog::MapConfig; ui.close_menu(); }
                if ui.button(icons::with(icons::SIGN_OUT, &t("Exit"))).clicked() { ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
            });
            ui.menu_button(t("Edit"), |ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); 
                let (u, r) = app.doc.as_ref().map(|d| (d.history.undo_label().map(str::to_string), d.history.redo_label().map(str::to_string))).unwrap_or((None, None));
                if ui.add_enabled(u.is_some(), egui::Button::new(icons::with(icons::ARROW_COUNTER_CLOCKWISE, &format!("{} {}", t("Undo"), u.unwrap_or_default()))).shortcut_text(keys.text(Action::Undo))).clicked() { app.undo(); ui.close_menu(); }
                if ui.add_enabled(r.is_some(), egui::Button::new(icons::with(icons::ARROW_CLOCKWISE, &format!("{} {}", t("Redo"), r.unwrap_or_default()))).shortcut_text(keys.text(Action::Redo))).clicked() { app.redo(); ui.close_menu(); }
                ui.separator();
                if ui.add(egui::Button::new(icons::with(icons::COPY, &t("Duplicate"))).shortcut_text(keys.text(Action::Duplicate))).clicked() { app.duplicate_selection(); ui.close_menu(); }
                if ui.add_enabled(!app.selection.is_empty(), egui::Button::new(icons::with(icons::ARROW_DOWN, &t("Drop onto surface"))).shortcut_text(keys.text(Action::DropOnSurface))).clicked() { crate::viewport::drop_selection_on_surface(app); ui.close_menu(); }
                if ui.add(egui::Button::new(icons::with(icons::TRASH, &t("Delete"))).shortcut_text(keys.text(Action::Delete))).clicked() { app.delete_selection(); ui.close_menu(); }
                ui.separator();
                if ui.add(egui::Button::new(icons::with(icons::GEAR, &t("Settings..."))).shortcut_text(keys.text(Action::Settings))).clicked() { app.dialog = Dialog::Settings; ui.close_menu(); }
                if ui.add(egui::Button::new(icons::with(icons::KEYBOARD, &t("Keyboard shortcuts..."))).shortcut_text(keys.text(Action::Shortcuts))).clicked() { app.dialog = Dialog::Shortcuts; ui.close_menu(); }
            });
            ui.menu_button(t("View"), |ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                if ui.add_enabled(app.doc.is_some(), egui::Button::new(icons::with(icons::RULER, &t("Measure"))).selected(app.measure.active)).on_hover_text(t("Clicks on the map measure lengths, the climb along them and the area they close.")).clicked() {
                    app.measure = crate::app::Measure { active: !app.measure.active, points: Vec::new() };
                    ui.close_menu();
                }
                if ui.add_enabled(app.doc.is_some(), egui::Button::new(icons::with(icons::CROSSHAIR, &t("Go to..."))).shortcut_text(keys.text(Action::GoTo))).on_hover_text(t("Fly the camera to a key point, a position or a square of the game's map grid.")).clicked() {
                    crate::goto::open(app);
                    ui.close_menu();
                }
                saved_views_menu(app, ui);
                ui.separator();
                if ui.add_enabled(app.doc.is_some(), egui::Button::new(icons::with(icons::CORNERS_OUT, &t("Full screen"))).shortcut_text("F11")).on_hover_text(t("The view takes the whole screen; move the pointer to an edge for the panels.")).clicked() {
                    let on = !app.fullscreen;
                    set_fullscreen(app, ctx, on);
                    ui.close_menu();
                }
                if ui.add_enabled(app.doc.is_some(), egui::Button::new(icons::with(icons::ARROW_SQUARE_OUT, &if app.detached { t("Bring the view back") } else { t("View in a separate window") }))).clicked() {
                    app.detached = !app.detached;
                    ui.close_menu();
                }
                crate::layout::menu(app, ui);
                ui.separator();
                visibility_toggles(app, ui);
                ui.separator();
                if ui.add_enabled(!app.selection.is_empty(), egui::Button::new(icons::with(icons::EYE_SLASH, &t("Hide selected"))).shortcut_text(keys.text(Action::HideSelected))).clicked() { app.hide_selection(); ui.close_menu(); }
                if ui.add_enabled(!app.selection.is_empty() || !app.hidden_objects.is_empty(), egui::Button::new(icons::with(icons::EYE, &t("Isolate selection"))).shortcut_text(keys.text(Action::IsolateSelection))).clicked() { app.isolate_selection(); ui.close_menu(); }
                let hidden_n = app.hidden_objects.len();
                let show_label = if hidden_n > 0 { tf("Show hidden objects ({0})", &[&hidden_n.to_string()]) } else { t("Show hidden objects") };
                if ui.add_enabled(hidden_n > 0, egui::Button::new(icons::with(icons::EYE, &show_label)).shortcut_text(keys.text(Action::ShowHidden))).clicked() { app.show_hidden_objects(); ui.close_menu(); }
                ui.separator();
                ui.checkbox(&mut app.camera_under, t("Camera below the terrain")).on_hover_text(keys.tip(Action::CameraBelow));
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
                    if ui.button(icons::with(icons::LIST_CHECKS, &t("Check the map"))).clicked() { ui.close_menu(); crate::map_check::run(app, None); }
                    if ui.button(t("Generate navmesh...")).clicked() { ui.close_menu(); app.generate_navmesh(); }
                });
            });
            crate::team_ui::menu(app, ui);
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
                if own_title_bar(app) { window_buttons(ui); }
                app.updater.bar(ui);
                ui.add_space(6.0);
                coffee_button(ui);
            });
        });
    // The name of the project in the middle of the bar, with a dot while it has unsaved changes.
    if let Some(d) = &app.doc {
        let bar = ui.min_rect();
        let mut job = egui::text::LayoutJob::default();
        job.append(&d.world.meta.name, 0.0, egui::TextFormat { font_id: egui::FontId::proportional(12.5), color: theme::TEXT_DIM, ..Default::default() });
        if d.history.dirty() { job.append("\u{25cf}", 6.0, egui::TextFormat { font_id: egui::FontId::proportional(10.0), color: theme::FOCUS, valign: egui::Align::Center, ..Default::default() }); }
        let galley = ui.painter().layout_job(job);
        let at = pos2(bar.center().x - galley.size().x * 0.5, bar.center().y - galley.size().y * 0.5);
        // Only where the menus and the buttons at the right leave room for it.
        if ui.ctx().screen_rect().width() > 1100.0 { ui.painter().galley(at, galley, theme::TEXT_DIM); }
    }
}

/// Where the project lives, and where it can be supported.
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
pub const BUY_ME_A_COFFEE: &str = "https://buymeacoffee.com/mrkamil404";

/// A small button that opens the Buy Me a Coffee page.
fn coffee_button(ui: &mut egui::Ui) {
    // Yellow text on the dark bar: easy to find, but not louder than the tools.
    let text = egui::RichText::new(icons::with(icons::COFFEE, &t("Buy me a coffee"))).color(egui::Color32::from_rgb(240, 200, 70));
    let b = egui::Button::new(text).fill(egui::Color32::from_rgb(43, 36, 16)).stroke(egui::Stroke::NONE).small();
    if ui.add(b).on_hover_text(BUY_ME_A_COFFEE).clicked() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(BUY_ME_A_COFFEE));
    }
}

/// A flat icon button for the plates of the toolbar: no frame until the pointer is over it,
/// blue-tinted while on.
fn flat_tool(ui: &mut egui::Ui, active: bool, icon: &str, label: Option<&str>, tip: String) -> egui::Response {
    let text = match label { Some(l) => format!("{icon}  {l}"), None => icon.to_string() };
    let enabled = ui.is_enabled();
    let galley = ui.painter().layout_no_wrap(text, egui::FontId::proportional(if label.is_some() { 13.5 } else { 16.0 }), Color32::WHITE);
    let (rect, resp) = ui.allocate_exact_size(vec2((galley.size().x + 14.0).max(30.0), 30.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let fill = if active { theme::TOGGLE_ON } else if resp.hovered() && enabled { theme::HOVER } else { Color32::TRANSPARENT };
        p.rect_filled(rect, 4.0, fill);
        let color = if !enabled { Color32::from_gray(90) } else if active { theme::TOGGLE_ON_TEXT } else if resp.hovered() { theme::TEXT_BRIGHT } else { theme::TEXT };
        p.galley_with_override_text_color(rect.center() - galley.size() * 0.5, galley, color);
    }
    resp.on_hover_text(tip)
}

/// A mode of the mode switcher: the icon over its name, or the icon alone when room is short.
fn mode_button(ui: &mut egui::Ui, active: bool, icon: &str, name: Option<&str>, tip: String) -> egui::Response {
    let icon_font = egui::FontId::proportional(16.0);
    let name_font = egui::FontId::proportional(10.5);
    let width = (name.map(|n| ui.fonts(|f| f.layout_no_wrap(n.to_string(), name_font.clone(), Color32::WHITE).size().x)).unwrap_or(0.0).max(22.0) + 20.0).max(if name.is_some() { 52.0 } else { 32.0 });
    let (rect, resp) = ui.allocate_exact_size(vec2(width, theme::TOOL_H), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let fill = if active { theme::ACCENT } else if resp.hovered() { theme::HOVER } else { Color32::TRANSPARENT };
        p.rect_filled(rect, 4.0, fill);
        let color = if active { Color32::WHITE } else if resp.hovered() { theme::TEXT_BRIGHT } else { Color32::from_gray(180) };
        match name {
            Some(n) => {
                p.text(pos2(rect.center().x, rect.top() + 11.0), egui::Align2::CENTER_CENTER, icon, icon_font, color);
                p.text(pos2(rect.center().x, rect.bottom() - 8.0), egui::Align2::CENTER_CENTER, n, name_font, color);
            }
            None => { p.text(rect.center(), egui::Align2::CENTER_CENTER, icon, icon_font, color); }
        }
    }
    resp.on_hover_text(tip)
}

/// The modes with their icons and keys.
pub const MODES: [(Mode, &str, &str, Action); 9] = [
    (Mode::Select, icons::CURSOR, "Select", Action::ModeSelect),
    (Mode::Landscape, icons::MOUNTAINS, "Sculpt", Action::ModeSculpt),
    (Mode::Paint, icons::PAINT_BRUSH, "Paint", Action::ModePaint),
    (Mode::Place, icons::TREE_EVERGREEN, "Place", Action::ModePlace),
    (Mode::Lines, icons::ROAD_HORIZON, "Roads", Action::ModeRoads),
    (Mode::Shapes, icons::POLYGON, "Shapes", Action::ModeShapes),
    (Mode::KeyPoints, icons::MAP_PIN, "Key points", Action::ModeKeyPoints),
    (Mode::Satellite, icons::GLOBE_HEMISPHERE_WEST, "Satellite", Action::ModeSatellite),
    (Mode::Mission, icons::FLAG_BANNER, "Mission", Action::ModeMission),
];

fn toolbar(app: &mut App, ctx: &egui::Context) {
    egui::TopBottomPanel::top("toolbar").frame(egui::Frame::new().fill(theme::PANEL).inner_margin(egui::Margin::symmetric(8, 5)).stroke(egui::Stroke::new(1.0_f32, theme::GAP))).show(ctx, |ui| toolbar_contents(app, ui));
}

/// The toolbar, in groups as in Unreal: the file, the modes, the gizmo, snapping, the view,
/// and building at the right.
pub(crate) fn toolbar_contents(app: &mut App, ui: &mut egui::Ui) {
    // One row of a fixed height; every group is put in the middle of it (see `centred`).
    ui.allocate_ui_with_layout(vec2(ui.available_width(), TOOLBAR_ROW), egui::Layout::left_to_right(egui::Align::Min), |ui| {
        ui.set_min_height(TOOLBAR_ROW);
        ui.spacing_mut().item_spacing.x = 10.0;
        let keys = app.settings.keys.clone();
        let has = app.doc.is_some();
        let wide = ui.available_width();
        // The file: plain icons, no plate.
        centred(ui, "file", |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_enabled_ui(has, |ui| {
                if flat_tool(ui, false, icons::FLOPPY_DISK, None, keys.tip(Action::Save)).clicked() { app.save(None); }
                let (u, r) = app.doc.as_ref().map(|d| (d.history.undo_label().map(str::to_string), d.history.redo_label().map(str::to_string))).unwrap_or((None, None));
                let undo = format!("{}: {}", keys.tip(Action::Undo), u.clone().unwrap_or_default());
                if ui.add_enabled_ui(u.is_some(), |ui| flat_tool(ui, false, icons::ARROW_COUNTER_CLOCKWISE, None, undo)).inner.clicked() { app.undo(); }
                let redo = format!("{}: {}", keys.tip(Action::Redo), r.clone().unwrap_or_default());
                if ui.add_enabled_ui(r.is_some(), |ui| flat_tool(ui, false, icons::ARROW_CLOCKWISE, None, redo)).inner.clicked() { app.redo(); }
            });
        });
        centred(ui, "divider", tool_divider);
        // The modes on a plate, their names under them while there is room.
        let named = wide > 1280.0;
        centred(ui, "modes", |ui| theme::tool_group(ui, |ui| {
            for (m, icon, name, action) in MODES {
                if !app.mode_enabled(m) { continue; }
                if mode_button(ui, app.mode == m, icon, named.then(|| t(name)).as_deref(), format!("{}\n{}", t(name), keys.tip(action))).clicked() { app.mode = m; }
            }
        }));
        centred(ui, "divider", tool_divider);
        // The gizmo: plain icons, the chosen one tinted, then World or Local.
        centred(ui, "gizmo", |ui| ui.add_enabled_ui(app.mode == Mode::Select, |ui| {
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (g, icon, action) in [(Gizmo::Move, icons::ARROWS_OUT_CARDINAL, Action::GizmoMove), (Gizmo::Rotate, icons::ARROWS_CLOCKWISE, Action::GizmoRotate), (Gizmo::Scale, icons::ARROWS_OUT, Action::GizmoScale), (Gizmo::Copy, icons::ARROW_FAT_LINES_RIGHT, Action::GizmoCopy)] {
                    if flat_tool(ui, app.gizmo == g, icon, None, keys.tip(action)).clicked() { app.gizmo = g; }
                }
                let (icon, name) = if app.gizmo_local { (icons::CUBE_FOCUS, t("Local")) } else { (icons::GLOBE_SIMPLE, t("World")) };
                if flat_tool(ui, false, icon, (wide > 1100.0).then_some(name.as_str()), format!("{}\n{}", keys.tip(Action::GizmoSpace), t("World: the gizmo follows the axes of the map. Local: those of the selected object."))).clicked() { app.gizmo_local = !app.gizmo_local; }
            });
        }));
        // Snapping, each a pill with its switch and its value.
        centred(ui, "snapping", |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let mut changed = false;
            let grid = app.settings.grid_snap;
            let tip = format!("{}\n{}", t("Snap to the grid"), t("Moving along an arrow or over the ground goes by whole steps of the grid. Without it, Ctrl does the same while held."));
            if snap_pill(ui, grid, icons::GRID_FOUR, tip, |ui| {
                changed |= ui.add(egui::DragValue::new(&mut app.settings.grid_step).range(0.05..=100.0).speed(0.05).custom_formatter(|v, _| format!("{} m", short_number(v))).custom_parser(|s| s.trim().trim_end_matches('m').trim().replace(',', ".").parse().ok())).changed();
            }) { app.settings.grid_snap = !grid; changed = true; }
            let key = app.settings.keys.hold(Hold::AngleSnap).name().to_string();
            if snap_pill(ui, true, icons::ANGLE, tf("Angle step with {0}", &[&key]), |ui| {
                changed |= ui.add(egui::DragValue::new(&mut app.settings.angle_step).range(1.0..=90.0).speed(0.5).custom_formatter(|v, _| format!("{}°", short_number(v))).custom_parser(|s| s.trim().trim_end_matches('°').trim().replace(',', ".").parse().ok())).on_hover_text(tf("Angle step with {0}", &[&key])).changed();
            }) {}
            let ground = app.settings.align_to_ground;
            let tip = format!("{}\n{}", t("Stick to the ground"), t("Moved and turned objects tilt with the ground under them."));
            if snap_pill(ui, ground, icons::ALIGN_BOTTOM, tip.clone(), |ui| {
                if ui.add(egui::Label::new(RichText::new(t("Ground")).color(if ground { theme::TEXT } else { theme::TEXT_DIM })).sense(egui::Sense::click())).on_hover_text(tip).clicked() { app.settings.align_to_ground = !ground; changed = true; }
            }) { app.settings.align_to_ground = !ground; changed = true; }
            let surface = app.place.snap_surface;
            let tip = format!("{}\n{}", keys.tip(Action::TogglePlaceOnSurface), t("Clicks in the 3D view place the object on the roof, floor or deck under the cursor instead of on the terrain."));
            if snap_pill(ui, surface, icons::STACK, tip.clone(), |ui| {
                if ui.add(egui::Label::new(RichText::new(t("Surface")).color(if surface { theme::TEXT } else { theme::TEXT_DIM })).sense(egui::Sense::click())).on_hover_text(tip).clicked() { app.place.snap_surface = !surface; }
            }) { app.place.snap_surface = !surface; }
            if changed { app.settings.save(); }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let free = has && app.task.is_none();
            let size = vec2(0.0, 32.0);
            centred(ui, "play", |ui| if app.session.is_some() {
                if ui.add(egui::Button::new(RichText::new(format!("{}  {}", icons::STOP, t("Stop"))).size(13.5).color(Color32::WHITE)).fill(theme::ERROR).stroke(egui::Stroke::NONE).min_size(size).corner_radius(4)).clicked() { app.stop_session(); }
            } else if ui.add_enabled(free, egui::Button::new(RichText::new(format!("{}  {}", icons::PLAY, t("Play"))).size(13.5).strong().color(Color32::WHITE)).fill(theme::PLAY).stroke(egui::Stroke::NONE).min_size(size).corner_radius(4)).on_hover_text(format!("{}\n{}", keys.tip(Action::Play), t("Export, binarise and start the map in DayZ"))).clicked() {
                app.build(true);
            });
            // The key of Build, faint after its name.
            let mut text = egui::text::LayoutJob::default();
            text.append(&format!("{}  {}", icons::HAMMER, t("Build")), 0.0, egui::TextFormat { font_id: egui::FontId::proportional(13.5), color: theme::TEXT_BRIGHT, ..Default::default() });
            let key = keys.text(Action::Build);
            if !key.is_empty() { text.append(&key, 8.0, egui::TextFormat { font_id: egui::FontId::proportional(12.0), color: theme::TEXT_DIM, valign: egui::Align::Center, ..Default::default() }); }
            let build = egui::Button::new(text).fill(Color32::from_rgb(38, 38, 38)).stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(51, 51, 51))).min_size(size).corner_radius(4);
            centred(ui, "build", |ui| if ui.add_enabled(free, build).on_hover_text(format!("{}\n{}", keys.tip(Action::Build), t("Export and binarise the PBO"))).clicked() { app.build(false); });
            // The views: words on a plate, the chosen one grey.
            centred(ui, "views", |ui| view_switch(app, ui, &keys));
        });
    });
}

/// A number with no zeros at its end: 1, 1.5, 0.25.
fn short_number(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The height of the row of the toolbar.
const TOOLBAR_ROW: f32 = 36.0;

/// Puts a group of the toolbar in the middle of its row. A plate or a group starts where it is
/// put, so it is moved down by half of what its height (as last drawn) leaves of the row.
fn centred<R>(ui: &mut egui::Ui, name: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let key = egui::Id::new(("toolbar_height", name));
    let h = ui.data(|d| d.get_temp::<f32>(key)).unwrap_or(TOOLBAR_ROW);
    let pad = ((TOOLBAR_ROW - h) * 0.5).max(0.0).round();
    let right = ui.layout().main_dir() == egui::Direction::RightToLeft;
    let cross = if right { egui::Align::Max } else { egui::Align::Min };
    ui.allocate_ui_with_layout(vec2(ui.available_width(), TOOLBAR_ROW), egui::Layout::top_down(cross), |ui| {
        ui.add_space(pad);
        // As high as the group, with the text of its buttons in their middle.
        let row = if right { egui::Layout::right_to_left(egui::Align::Center) } else { egui::Layout::left_to_right(egui::Align::Center) };
        let r = ui.allocate_ui_with_layout(vec2(ui.available_width(), h.min(TOOLBAR_ROW - pad)), row, add);
        let used = r.response.rect.height();
        if (used - h).abs() > 0.5 {
            ui.data_mut(|d| d.insert_temp(key, used));
            ui.ctx().request_repaint();
        }
        r.inner
    }).inner
}

/// A thin upright line between groups of the toolbar.
fn tool_divider(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(1.0, 26.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, Color32::from_rgb(51, 51, 51));
}

/// A snapping setting as a pill: its switch (an icon, tinted while on) and its value, in one
/// outline. True when the switch was clicked.
fn snap_pill(ui: &mut egui::Ui, on: bool, icon: &str, tip: String, value: impl FnOnce(&mut egui::Ui)) -> bool {
    let mut clicked = false;
    egui::Frame::new().stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(44, 44, 44))).corner_radius(4).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.allocate_ui_with_layout(vec2(0.0, 26.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_height(26.0);
            let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 26.0), egui::Sense::click());
            let p = ui.painter();
            let fill = if on { theme::TOGGLE_ON } else if resp.hovered() { theme::HOVER } else { Color32::TRANSPARENT };
            p.rect_filled(rect, egui::CornerRadius { nw: 4, sw: 4, ne: 0, se: 0 }, fill);
            p.text(rect.center(), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(14.0), if on { theme::TOGGLE_ON_TEXT } else { theme::TEXT_DIM });
            clicked = resp.on_hover_text(tip).clicked();
            ui.add_space(6.0);
            // The value without a box of its own: the pill is its box.
            let w = &mut ui.visuals_mut().widgets;
            for s in [&mut w.inactive, &mut w.hovered, &mut w.active] { s.weak_bg_fill = Color32::TRANSPARENT; s.bg_fill = Color32::TRANSPARENT; s.bg_stroke = egui::Stroke::NONE; }
            ui.spacing_mut().interact_size.x = 0.0;
            value(ui);
            ui.add_space(8.0);
        });
    });
    clicked
}

/// The views as words on a plate: 3D, 2D and both, the chosen one on grey.
fn view_switch(app: &mut App, ui: &mut egui::Ui, keys: &crate::keys::KeyMap) {
    egui::Frame::new().fill(Color32::from_rgb(18, 18, 18)).stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(44, 44, 44))).corner_radius(6).inner_margin(egui::Margin::same(2)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        // As tight as its buttons; laid out right to left, so the last first.
        ui.allocate_ui_with_layout(vec2(0.0, 26.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            for (v, name, action) in [(ViewMode::Split, "3D | 2D", Action::ViewSplit), (ViewMode::Top, "2D", Action::View2d), (ViewMode::Perspective, "3D", Action::View3d)] {
                let on = app.view_mode == v;
                let b = egui::Button::new(RichText::new(name).color(if on { theme::TEXT_BRIGHT } else { theme::TEXT_DIM })).fill(if on { Color32::from_rgb(51, 51, 51) } else { Color32::TRANSPARENT }).stroke(egui::Stroke::NONE).min_size(vec2(0.0, 26.0)).corner_radius(4);
                if ui.add(b).on_hover_text(keys.tip(action)).clicked() { app.view_mode = v; }
            }
        });
    });
}

/// Keys shown next to what they do, as small key caps.
/// The angle step lines and turns snap to, and the key that does it.
fn angle_snap_ui(app: &mut App, ui: &mut egui::Ui) {
    let key = app.settings.keys.hold(Hold::AngleSnap).name().to_string();
    let before = app.settings.angle_step;
    row(ui, &tf("Angle step with {0}", &[&key]), |ui| {
        ui.add(crate::theme::ValueField::new(&mut app.settings.angle_step, 1.0..=90.0).suffix("°").step_by(1.0));
    });
    if app.settings.angle_step != before { app.settings.save(); }
}

/// Whether key help shows, set from the settings every frame.
pub static HINTS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

fn hints_on() -> bool {
    HINTS.load(std::sync::atomic::Ordering::Relaxed)
}

/// A line telling what clicks and keys do, hidden with the rest of the key help.
pub fn hint(ui: &mut egui::Ui, text: &str) {
    if hints_on() { ui.weak(text); }
}

pub fn key_hints(ui: &mut egui::Ui, rows: &[(String, String)]) {
    if !hints_on() { return; }
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
    egui::TopBottomPanel::bottom("status").exact_height(24.0).frame(egui::Frame::new().fill(theme::STRIP).inner_margin(egui::Margin::symmetric(8, 0))).show(ctx, |ui| {
        ui.horizontal_centered(|ui| {
            let mut cancel = false;
            if let Some(task) = &app.task {
                ui.spinner();
                ui.add(egui::ProgressBar::new(task.job.progress()).desired_width(220.0).text(format!("{}: {}", task.title, task.job.current_stage())));
                cancel = ui.small_button(t("Cancel")).clicked();
            } else {
                // One line: the first of the message, cut to the room left; all of it under the pointer.
                let first = app.status.lines().next().unwrap_or_default();
                let room = ui.available_width() * 0.5;
                ui.scope(|ui| {
                    ui.set_max_width(room);
                    ui.add(egui::Label::new(RichText::new(first).small()).truncate()).on_hover_text(&app.status);
                });
            }
            if cancel {
                if let Some(task) = &app.task { task.job.cancel(); }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // The figures of the main view: the 3D one, or the top view when it is alone; the
                // minimap is drawn last and would otherwise stand in for them.
                let stats = app.renderer.lock().unwrap().view_stats[usize::from(app.view_mode == crate::app::ViewMode::Top)];
                ui.label(RichText::new(format!("{:.1} ms  |  {} kT  |  {} DC  |  {} / {}", app.frame_ms, stats.triangles / 1000, stats.draw_calls, stats.mesh_instances, stats.impostors)).small().color(theme::TEXT_DIM))
                    .on_hover_text(tf("Drawing the view: {0} ms a frame, {1} thousand triangles, {2} draw calls, {3} models drawn in full and {4} as distant pictures.", &[&format!("{:.1}", app.frame_ms), &(stats.triangles / 1000).to_string(), &stats.draw_calls.to_string(), &stats.mesh_instances.to_string(), &stats.impostors.to_string()]));
                if let (Some(d), Some(at)) = (&app.doc, app.saved_at) {
                    let mins = at.elapsed().as_secs() / 60;
                    let when = if mins == 0 { t("Saved just now") } else { tf("Saved {0} min ago", &[&mins.to_string()]) };
                    let color = if d.history.dirty() { theme::TEXT_DIM } else { theme::OK };
                    ui.label(RichText::new(format!("{}  {when}", icons::CHECK_CIRCLE)).small().color(color));
                    ui.add_space(8.0);
                }
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
    row(ui, label, |ui| { ui.add(crate::theme::ValueField::new(v, r).logarithmic(log)); });
}

pub(crate) fn modes_panel(app: &mut App, ui: &mut egui::Ui) {
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
        Mode::Mission => crate::mission_ui::panel(app, ui),
    });
}

/// One line on what the chosen tool does; shown whether hints are on or not.
/// What a section or tool does: a quiet line cut to the width of the panel, all of it under
/// the pointer, so the panels stay short. With key help on, it shows in full.
pub(crate) fn about(ui: &mut egui::Ui, text: &str) {
    if hints_on() {
        ui.label(RichText::new(text).small().color(theme::TEXT_DIM));
    } else {
        ui.add(egui::Label::new(RichText::new(format!("{}  {text}", icons::INFO)).small().color(theme::TEXT_DIM)).truncate()).on_hover_text(text);
    }
}

/// The keys of a tool, folded away at the bottom of its panel.
pub(crate) fn shortcuts(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    if hints_on() {
        section(ui, &icons::with(icons::KEYBOARD, &t("Shortcuts")), false, add);
    }
}

fn select_mode(app: &mut App, ui: &mut egui::Ui) {
    section(ui, &icons::with(icons::CURSOR_CLICK, &t("Selection")), true, |ui| {
        let n = app.selection.len();
        let (badge_icon, badge_col, badge_stroke) = if n > 0 {
            (icons::CHECK_CIRCLE, theme::TEXT_BRIGHT, theme::ACCENT.gamma_multiply(0.75))
        } else {
            (icons::CURSOR_CLICK, theme::TEXT_DIM, theme::BORDER)
        };
        theme::card(ui, badge_stroke, |ui| {
            let mut job = egui::text::LayoutJob::default();
            job.append(badge_icon, 0.0, egui::TextFormat { font_id: egui::FontId::proportional(13.5), color: if n > 0 { theme::ACCENT } else { theme::TEXT_DIM }, ..Default::default() });
            let text = if n == 0 { t("Nothing selected") } else { selected_text(n) };
            job.append(&text, 6.0, egui::TextFormat { font_id: icons::strong(13.0), color: badge_col, ..Default::default() });
            ui.add(egui::Label::new(job).truncate());
        });
        ui.add_space(2.0);
        about(ui, &t("Click selects, a drag selects all inside. Shift+click adds, Shift+Alt+drag adds all inside, Ctrl+drag takes it out."));
        ui.add_space(2.0);
        theme::button_columns(ui, 3, |c| {
            let w = c[0].available_width();
            let wide = |b: egui::Button<'static>| b.min_size(vec2(w, 24.0));
            if c[0].add_enabled(n > 0, wide(egui::Button::new(icons::with(icons::COPY, &t("Duplicate"))))).on_hover_text(t("Duplicate")).clicked() { app.duplicate_selection(); }
            if c[1].add_enabled(n > 0, wide(egui::Button::new(icons::with(icons::TRASH, &t("Delete"))))).on_hover_text(t("Delete")).clicked() { app.delete_selection(); }
            if c[2].add_enabled(n > 0, wide(egui::Button::new(icons::with(icons::PACKAGE, &t("Create prefab"))))).on_hover_text(t("Saves the selected objects as a prefab of the library, to be placed again anywhere as one.")).clicked() { app.create_prefab(); }
        });
        let objects = n > 0 && app.whole.is_none();
        theme::button_columns(ui, 2, |c| {
            let picking = app.pick_direction;
            let w = c[0].available_width();
            if c[0].add_enabled(objects, egui::Button::new(icons::with(icons::EYEDROPPER, &t("Copy direction"))).selected(picking).min_size(vec2(w, 24.0))).on_hover_text(t("Click another object afterwards: the selection turns the way it faces.")).clicked() {
                app.pick_direction = !picking;
                app.status = if app.pick_direction { t("Click an object to take its direction, Esc cancels") } else { String::new() };
            }
            let w = c[1].available_width();
            if c[1].add_enabled(objects, egui::Button::new(icons::with(icons::ARROWS_IN, &t("Reset scale"))).min_size(vec2(w, 24.0))).on_hover_text(t("Sets the selected objects back to their own size.")).clicked() { app.reset_selection_scale(); }
        });
        ui.add_space(2.0);
        row(ui, &t("Drag selects"), |ui| {
            if theme::icon_segmented(ui, &mut app.settings.lasso, &[
                (false, icons::SELECTION, t("Box")),
                (true, icons::LASSO, format!("{}\n{}", t("Lasso"), t("Draw an outline round what to select."))),
            ]) {
                app.settings.save();
            }
        });
    });
    rows_section(app, ui);
    section(ui, &icons::with(icons::SLIDERS_HORIZONTAL, &t("Options")), true, |ui| {
        theme::check_row(ui, &mut app.select_whole, t("Select whole roads and generators"));
        if theme::check_row(ui, &mut app.settings.drag_by_body, t("Move objects by dragging them")).on_hover_text(t("An object held by its shape slides over the ground, as with the dot of the gizmo.")).changed() { app.settings.save(); }
        if theme::check_row(ui, &mut app.settings.align_to_ground, t("Stick to the ground")).on_hover_text(t("Moved and turned objects tilt with the ground under them.")).changed() { app.settings.save(); }
        if theme::check_row(ui, &mut app.settings.turn_about_press, t("Shift and drag turns about the press")).on_hover_text(t("The selection turns about the spot of the ground where the button went down, not about its own centre.")).changed() { app.settings.save(); }
        angle_snap_ui(app, ui);
        row(ui, &t("Gizmo and turn centre"), |ui| {
            use crate::settings::PivotMode;
            if theme::icon_segmented(ui, &mut app.settings.pivot_mode, &[
                (PivotMode::Middle, icons::ARROWS_IN_CARDINAL, format!("{}\n{}", t("Average"), t("The average of the positions of the selected objects."))),
                (PivotMode::Bounds, icons::BOUNDING_BOX, format!("{}\n{}", t("Box centre"), t("The middle of the box that holds the positions of the selected objects."))),
                (PivotMode::First, icons::NUMBER_ONE, format!("{}\n{}", t("First object"), t("The position of the first selected object."))),
            ]) {
                app.settings.save();
            }
        });
        ui.add_space(4.0);
        let w = ui.available_width();
        if ui.add_enabled(!app.selection.is_empty(), egui::Button::new(icons::with(icons::ARROW_LINE_DOWN, &t("Align to the ground"))).min_size(vec2(w, 24.0))).on_hover_text(t("Tilts the selected objects to the ground under them now.")).clicked() { app.align_selection_to_ground(); }
    });
    section(ui, &icons::with(icons::STACK, &t("Terrain under objects")), false, |ui| {
        about(ui, &t("A modifier levels the ground under the object and follows it. The sculpted terrain stays untouched."));
        let mut changed = pad_sliders(app, ui);
        if theme::check_row(ui, &mut app.place.pad_follow, t("The ground follows the height of the object")).on_hover_text(t("The ground is levelled to the height the object stands at and comes up or down with it. The object keeps a height of its own, so lifting the ground does not lift it again.")).changed() {
            let (ids, on): (Vec<u32>, bool) = (app.selection.iter().copied().collect(), app.place.pad_follow);
            app.set_pad_follow(&ids, on);
        }
        ui.add_space(4.0);
        let w = ui.available_width();
        if ui.add_enabled(!app.selection.is_empty(), egui::Button::new(icons::with(icons::PLUS, &t("Add modifier to selection"))).min_size(vec2(w, 24.0))).clicked() {
            let f = app.place.pad_falloff;
            app.add_pads_for_selection(f);
            changed = false;
        }
        if changed { app.update_selected_pads(); }
    });
    shortcuts(ui, |ui| {
        let k = &app.settings.keys;
        key_hints(ui, &[
            (format!("{} / {} / {}", k.text(Action::GizmoMove), k.text(Action::GizmoRotate), k.text(Action::GizmoScale)), t("Move, rotate, scale")),
            (k.text(Action::GizmoCopy), t("Copy in a row: drag an arrow, aside to turn, Ctrl by steps; the ring turns about the joint")),
            (format!("{} ({})", k.hold(Hold::Precise).name(), t("during a drag")), t("Move and turn slowly")),
            (k.text(Action::Focus), t(Action::Focus.name())),
            (k.text(Action::Duplicate), t(Action::Duplicate.name())),
            (k.text(Action::CopyAhead), t(Action::CopyAhead.name())),
            (k.text(Action::CopyBehind), t(Action::CopyBehind.name())),
            (k.text(Action::JoinNeighbour), t(Action::JoinNeighbour.name())),
            (k.text(Action::PasteHere), t(Action::PasteHere.name())),
            (k.text(Action::PasteLast), t(Action::PasteLast.name())),
            ("Shift".to_string(), t("Drag: turn")),
            ("Ctrl+Shift".to_string(), t("Drag: turn by steps")),
            ("Alt".to_string(), t("Drag: scale")),
            ("Shift+Alt".to_string(), t("Drag: add to the selection")),
            ("Ctrl".to_string(), t("Drag: take out of the selection")),
            (k.text(Action::Underground), t(Action::Underground.name())),
            (k.text(Action::CameraBelow), t(Action::CameraBelow.name())),
        ]);
    });
}

/// Walls and fences laid end to end: copies of the selection in a row, joining it to the
/// object next to it, taking a direction, and the gap between them.
fn rows_section(app: &mut App, ui: &mut egui::Ui) {
    section(ui, &icons::with(icons::ARROW_FAT_LINES_RIGHT, &t("Rows of walls and fences")), true, |ui| {
        about(ui, &t("Copies of the selection laid end to end along its long side, as walls and fences are."));
        ui.add_space(2.0);
        let objects = !app.selection.is_empty() && app.whole.is_none();
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (back, ahead) = (app.settings.keys.tip(Action::CopyBehind), app.settings.keys.tip(Action::CopyAhead));
                if ui.add_enabled(objects, egui::Button::new(icons::ARROW_LINE_RIGHT).min_size(vec2(28.0, 24.0))).on_hover_text(format!("{ahead}\n{}", t("A copy of the selection goes right ahead of it, along its long side, and is selected, so pressing again lays a row."))).clicked() { app.copy_selection_along(true); }
                if ui.add_enabled(objects, egui::Button::new(icons::ARROW_LINE_LEFT).min_size(vec2(28.0, 24.0))).on_hover_text(format!("{back}\n{}", t("A copy of the selection goes right behind it, along its long side, and is selected, so pressing again lays a row."))).clicked() { app.copy_selection_along(false); }
                let on = app.gizmo == Gizmo::Copy;
                let (fill, col) = if on { (theme::ACCENT, Color32::WHITE) } else { (theme::SECTION, theme::TEXT) };
                let w = ui.available_width();
                if ui.add(egui::Button::new(RichText::new(icons::with(icons::ARROW_FAT_LINES_RIGHT, &t("Copy in a row"))).color(col)).fill(fill).min_size(vec2(w, 24.0))).on_hover_text(format!("{}\n{}", app.settings.keys.tip(Action::GizmoCopy), t("The gizmo shows arrows at the ends of the selection: drag one out to lay copies, aside to turn the row. The green ring turns the selection about where it joins its neighbour."))).clicked() {
                    app.gizmo = if on { Gizmo::Move } else { Gizmo::Copy };
                }
            });
        });
        let w = ui.available_width();
        if ui.add_enabled(objects, egui::Button::new(icons::with(icons::LINK_SIMPLE, &t("Join to the neighbour"))).min_size(vec2(w, 24.0))).on_hover_text(format!("{}\n{}", app.settings.keys.tip(Action::JoinNeighbour), t("Moves the selection so its end meets the end of the wall or fence next to it, as after turning it."))).clicked() { app.join_selection_to_neighbour(); }
        ui.add_space(2.0);
        let items = crate::library_ui::preset_items(app);
        let mut preset = app.settings.row_preset;
        let (thumbs, selected) = (app.thumbs.handle(), app.browser.presets().first().copied());
        {
            use crate::asset_browser::Pick;
            crate::slots::preset_slot(ui, &t("Copies of"), "row_preset", &mut preset, Some((None, t("The selected objects"))), &items, &thumbs, &|p| if let Pick::Preset(id) = p { Some(Some(*id)) } else { None }, &|k| k.map(Pick::Preset), selected.map(Some), None);
        }
        if preset != app.settings.row_preset {
            app.settings.row_preset = preset;
            app.settings.save();
            app.intern_row_preset();
        }
        if preset.is_some() {
            about(ui, &t("Each copy takes a model of the preset, drawn by its weights: give a broken fence a small weight for one now and then. The copies are as long as the selection."));
        }
        let before = (app.settings.row_gap, app.settings.row_angle, app.settings.row_flip, app.settings.row_turn);
        slider(ui, &t("Gap"), &mut app.settings.row_gap, -3.0..=3.0, false);
        about(ui, &t("Metres between copies and between joined neighbours. Negative values overlap them."));
        row(ui, &t("Row direction"), |ui| {
            ui.add(crate::theme::ValueField::new(&mut app.settings.row_angle, -180.0..=180.0).suffix("°").step_by(1.0)).on_hover_text(t("The way the row goes, turned from the long side of the first object. For models whose copies belong another way, such as posts or gates."));
        });
        // The usual turns, under the value they set.
        theme::row_with(ui, |ui, w| {
            ui.add_space(w);
            egui::Frame::new()
                .fill(Color32::from_rgb(18, 18, 18))
                .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(44, 44, 44)))
                .corner_radius(4)
                .inner_margin(egui::Margin::same(2))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    ui.horizontal(|ui| {
                        for a in [0.0f32, 90.0, 180.0, -90.0] {
                            let on = (app.settings.row_angle - a).abs() < 0.5
                                || (a == 180.0 && (app.settings.row_angle + 180.0).abs() < 0.5);
                            let b = egui::Button::new(RichText::new(format!("{a:.0}°")).small().color(if on { Color32::WHITE } else { theme::TEXT }))
                                .fill(if on { theme::ACCENT } else { Color32::TRANSPARENT })
                                .stroke(egui::Stroke::NONE)
                                .min_size(vec2(0.0, 20.0));
                            if ui.add(b).clicked() { app.settings.row_angle = a; }
                        }
                    });
                });
        });
        theme::check_row(ui, &mut app.settings.row_flip, t("Turn copies by 180° at random")).on_hover_text(t("About half of the copies face the other way, so both sides of a wall or fence show along it."));
        slider(ui, &t("Random turn ±°"), &mut app.settings.row_turn, 0.0..=45.0, false);
        if (app.settings.row_gap, app.settings.row_angle, app.settings.row_flip, app.settings.row_turn) != before { app.settings.save(); }
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
    theme::check_row(ui, &mut g.embankment, t("Embankments and cuttings"));
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
        about(ui, &t("Heights along this route follow its grade, cut and fill limits. Change them in the route settings and fit again."));
    } else {
        slider(ui, &t("Height smoothing"), &mut g.smooth, 0.0..=200.0, false);
        slider(ui, &t("Max. climb %"), &mut g.max_grade, 0.0..=30.0, false);
    }
    slider(ui, &t("Raise the bed"), &mut g.raise, -6.0..=12.0, false);
}

fn sculpt_mode(app: &mut App, ui: &mut egui::Ui) {
    section(ui, &icons::with(icons::MOUNTAINS, &t("Sculpt tool")), true, |ui| {
        // The tools as icons; the tip says what each one does.
        use crate::tool_icons::Glyph as G;
        let tools = [
            (SculptTool::Raise, G::Raise, "Raise", "Raises the ground under the brush."),
            (SculptTool::Lower, G::Lower, "Lower", "Lowers the ground under the brush."),
            (SculptTool::Smooth, G::Smooth, "Smooth", "Evens out bumps and edges."),
            (SculptTool::Flatten, G::Flatten, "Flatten", "Levels the ground to the height where the stroke starts."),
            (SculptTool::Noise, G::Noise, "Noise", "Roughens the ground with random bumps."),
            (SculptTool::Ramp, G::Ramp, "Ramp", "An even slope from where the stroke starts to the cursor."),
            (SculptTool::SetHeight, G::SetHeight, "Set height", "Paints the ground to a height copied from the map: right click or Alt+click the terrain to copy it."),
        ];
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
            for (tool, icon, name, what) in tools {
                if crate::tool_icons::tile(ui, app.sculpt.tool == tool && !app.hole_tool, icon, &t(name), t(what)).clicked() { app.sculpt.tool = tool; app.hole_tool = false; }
            }
            if crate::tool_icons::tile(ui, app.hole_tool, G::Holes, &t("Holes"), t("Paints the cells the game does not draw, for entrances under the ground.")).clicked() { app.hole_tool = !app.hole_tool; }
        });
        let what = if app.hole_tool {
            "Paints the cells the game does not draw, for entrances under the ground."
        } else {
            tools.iter().find(|t| t.0 == app.sculpt.tool).map_or("", |t| t.3)
        };
        about(ui, &t(what));
        if !app.hole_tool && app.sculpt.tool == SculptTool::SetHeight {
            ui.add_space(2.0);
            let mut height = app.sculpt.height.unwrap_or(0.0);
            row(ui, &t("Copied height"), |ui| {
                if ui.add(crate::theme::ValueField::new(&mut height, -500.0..=5000.0).suffix(" m")).changed() { app.sculpt.height = Some(height); }
            });
            if app.sculpt.height.is_none() {
                theme::card(ui, theme::BORDER, |ui| {
                    ui.label(RichText::new(icons::with(icons::EYEDROPPER, &t("Nothing copied yet"))).small().color(theme::TEXT_DIM));
                });
            }
        }
    });
    if app.hole_tool {
        section(ui, &icons::with(icons::PAINT_BRUSH_BROAD, &t("Brush")), true, |ui| {
            if let Some(d) = &app.doc {
                let any = !d.world.holes.is_empty();
                theme::card(ui, if any { theme::ACCENT.gamma_multiply(0.7) } else { theme::BORDER }, |ui| {
                    let mut job = egui::text::LayoutJob::default();
                    job.append(icons::GRID_FOUR, 0.0, egui::TextFormat { font_id: egui::FontId::proportional(13.5), color: if any { theme::ACCENT } else { theme::TEXT_DIM }, ..Default::default() });
                    job.append(&tf("{0} cells", &[&d.world.holes.len().to_string()]), 6.0, egui::TextFormat { font_id: icons::strong(12.5), color: if any { theme::TEXT_BRIGHT } else { theme::TEXT_DIM }, ..Default::default() });
                    ui.label(job);
                });
                ui.add_space(2.0);
            }
            slider(ui, &t("Radius"), &mut app.hole_radius, 0.5..=100.0, true);
            about(ui, &t("Objects under the terrain need the underground flag, and holes cost performance."));
        });
        shortcuts(ui, |ui| brush_hints(app, ui, Some("Remove the holes"), false, false, false));
        return;
    }
    section(ui, &icons::with(icons::PAINT_BRUSH_BROAD, &t("Brush")), true, |ui| {
        brush_tip(app, ui, false);
        slider(ui, &t("Radius"), &mut app.sculpt.radius, 2.0..=3000.0, true);
        slider(ui, &t("Strength"), &mut app.sculpt.strength, 0.5..=200.0, true);
        slider(ui, &t("Falloff"), &mut app.sculpt.falloff, 0.0..=1.0, false);
        slider(ui, &t("Edge noise"), &mut app.sculpt.noise, 0.0..=1.0, false);
    });
    crate::stamps_ui::panel(app, ui);
    shortcuts(ui, |ui| brush_hints(app, ui, Some("Lower instead of raise"), true, true, true));
}

/// The surface layers as tiles: the texture of the surface, its mask colour in the corner and
/// its name, as the Content Browser shows assets with a surface.
fn layer_list(app: &mut App, ui: &mut egui::Ui) {
    let root = app.settings.tools.work_drive.clone();
    let mut copy_in = None;
    let mut material = None;
    let mut delete = None;
    let Some(doc) = &mut app.doc else { return };
    let id = doc.world.meta.map_id.clone();
    let layers = doc.world.layers.len();
    let avail = (ui.available_width() - 10.0).max(64.0);
    let per_row = ((avail + 4.0) / 72.0).floor().max(1.0);
    let side = (((avail + 4.0) / per_row - 4.0).floor() - 8.0).clamp(48.0, 74.0);
    let mut changed = false;
    let mut chosen = None;
    let mut recolour = None;
    egui::ScrollArea::vertical().id_salt("surface_tiles").max_height(330.0).auto_shrink([false, true]).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            for (i, l) in doc.world.layers.iter_mut().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(vec2(side + 8.0, side + 22.0), egui::Sense::click());
                let selected = app.paint.layer == i;
                let p = ui.painter();
                p.rect_filled(rect, 3.0, if selected { theme::ACCENT_SOFT } else if resp.hovered() { theme::HOVER } else { Color32::from_rgb(42, 42, 44) });
                if selected {
                    p.rect_stroke(rect, 3.0, egui::Stroke::new(1.0f32, theme::ACCENT), egui::StrokeKind::Inside);
                }
                let art = Rect::from_min_size(rect.min + vec2(4.0, 4.0), vec2(side, side));
                match app.layer_thumbs.get(i).and_then(|t| t.as_ref()) {
                    Some(tex) => { p.image(tex.id(), art, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE); }
                    None => {
                        let a = app.layer_averages.get(i).copied().unwrap_or([70, 70, 70]);
                        p.rect_filled(art, 2.0, Color32::from_rgb(a[0], a[1], a[2]));
                    }
                }
                // The mask colour of the surface, as the browser marks models with one.
                let mark = Rect::from_min_size(pos2(art.right() - 15.0, art.top() + 2.0), vec2(13.0, 13.0));
                p.rect_filled(mark, 2.0, Color32::from_rgb(l.color[0], l.color[1], l.color[2]));
                p.rect_stroke(mark, 2.0, egui::Stroke::new(1.0f32, Color32::BLACK), egui::StrokeKind::Outside);
                let badge = Rect::from_min_size(pos2(art.left() + 2.0, art.bottom() - 15.0), vec2(26.0, 13.0));
                p.rect_filled(badge, 3.0, Color32::from_black_alpha(170));
                p.text(badge.center(), egui::Align2::CENTER_CENTER, format!("L{i:02}"), egui::FontId::proportional(10.0), Color32::WHITE);
                let short: String = if l.name.chars().count() > 11 { l.name.chars().take(10).chain(['…']).collect() } else { l.name.clone() };
                p.text(pos2(rect.center().x, rect.bottom() - 9.0), egui::Align2::CENTER_CENTER, short, egui::FontId::proportional(11.0), theme::TEXT);
                let resp = resp.on_hover_text(format!("{}\n{}\n{}\n{}", l.name, l.texture, l.material, t("Right click: colour and material of the surface")));
                if resp.clicked() {
                    chosen = Some(i);
                }
                resp.context_menu(|ui| {
                    ui.label(RichText::new(&l.name).strong());
                    if ui.button(icons::with(icons::PAINT_BRUSH, &t("Material..."))).on_hover_text(t("Its textures, relief, repeats and lighting (.rvmat)")).clicked() {
                        material = Some(i);
                        ui.close_menu();
                    }
                    if ui.add_enabled(layers > 1, egui::Button::new(icons::with(icons::TRASH, &t("Delete layer...")))).clicked() {
                        delete = Some(i);
                        ui.close_menu();
                    }
                    let lower = l.texture.to_ascii_lowercase();
                    if !lower.starts_with("dz\\") && !lower.starts_with(&format!("{}\\", id.to_ascii_lowercase())) && ui.button(t("Copy the textures into the map")).on_hover_text(t("Into the map's folder of surfaces, which the build packs")).clicked() {
                        copy_in = Some(i);
                        ui.close_menu();
                    }
                    // The picker sits in a submenu of its own: a colour button would open it as a
                    // popup outside the menu, and the first click in it would close both.
                    let mut title = egui::text::LayoutJob::default();
                    title.append("\u{25a0}", 0.0, egui::TextFormat { font_id: egui::FontId::proportional(14.0), color: Color32::from_rgb(l.color[0], l.color[1], l.color[2]), ..Default::default() });
                    title.append(&t("Mask colour"), 6.0, egui::TextFormat { font_id: egui::FontId::proportional(13.0), color: theme::TEXT, ..Default::default() });
                    ui.menu_button(title, |ui| {
                        let mut c = Color32::from_rgb(l.color[0], l.color[1], l.color[2]);
                        if egui::color_picker::color_picker_color32(ui, &mut c, egui::color_picker::Alpha::Opaque) {
                            recolour = Some((i, [c.r(), c.g(), c.b()]));
                        }
                    });
                });
            }
        });
    });
    if let Some(i) = chosen { app.paint.layer = i; }
    if let Some((i, c)) = recolour { app.set_layer_colour(i, c); }
    let Some(doc) = &mut app.doc else { return };
    if let Some(i) = copy_in {
        let source = root.join(doc.world.layers[i].texture.replace('\\', "/"));
        match dz_core::surfaces::import_layer_texture(&root, &id, &source) {
            Ok((texture, material)) => {
                let l = &mut doc.world.layers[i];
                (l.texture, l.material) = (texture, material);
                changed = true;
                app.reload_layer_textures();
            }
            Err(e) => app.error(format!("{}: {e:#}", t("The texture could not be copied into the map"))),
        }
    }
    if let Some(i) = material {
        crate::material_ui::open(app, i);
    }
    if let Some(i) = delete {
        app.dialog = Dialog::DeleteLayer { layer: i, into: if i == 0 { 1 } else { 0 } };
    }
    let Some(doc) = &mut app.doc else { return };
    if changed {
        doc.history.revision += 1;
        app.push_palette();
    }
}

/// Menu of the surfaces of the game, with a search, that adds one as a layer.
fn add_layer_menu(app: &mut App, ui: &mut egui::Ui) {
    let surfaces = app.surfaces.clone();
    ui.spacing_mut().interact_size.y = 24.0;
    ui.menu_button(icons::with(icons::PLUS, &t("Add layer")), |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        let id = ui.make_persistent_id("surface_search");
        let mut search: String = ui.data_mut(|d| d.get_temp(id).unwrap_or_default());
        ui.add(egui::TextEdit::singleline(&mut search).hint_text(t("Search...")).desired_width(240.0));
        ui.data_mut(|d| d.insert_temp(id, search.clone()));
        let needle = search.to_ascii_lowercase();
        if surfaces.is_empty() {
            ui.weak(t("No surfaces of the game were found on the work drive"));
        }
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            for s in surfaces.iter().filter(|s| needle.is_empty() || s.contains(&needle)) {
                let name = s.rsplit('\\').next().unwrap_or(s).trim_end_matches("_ca.paa").to_string();
                let folder = s.split('\\').nth(1).unwrap_or("");
                if ui.button(format!("{name}   ({folder})")).on_hover_text(s).clicked() {
                    add_surface_layer(app, name, s.clone(), s.replace("_ca.paa", ".rvmat"));
                    ui.close_menu();
                }
            }
        });
        ui.separator();
        if ui.button(t("From a file...")).on_hover_text(t("A colour texture (_ca or _co) of the map's own surface, from anywhere: it is copied into the map's folder with its relief and material")).clicked() {
            ui.close_menu();
            let root = app.settings.tools.work_drive.clone();
            if let Some(f) = rfd::FileDialog::new().add_filter(t("Textures and images"), &["paa", "png", "tga", "jpg", "jpeg", "bmp", "tif"]).set_directory(&root).pick_file() {
                add_texture_layer(app, &f);
            }
        }
    });
}

/// Adds a layer of a colour texture chosen anywhere: those of the game stay where they are,
/// others are copied into the map's folder of surfaces with their material. Returns the
/// layer's texture.
pub(crate) fn add_texture_layer(app: &mut App, file: &std::path::Path) -> Option<String> {
    let root = app.settings.tools.work_drive.clone();
    let id = app.doc.as_ref()?.world.meta.map_id.clone();
    let vanilla = file.strip_prefix(&root).ok().filter(|r| r.components().next().is_some_and(|c| c.as_os_str().eq_ignore_ascii_case("dz")));
    let (texture, material) = match vanilla {
        Some(rel) => {
            let texture = rel.to_string_lossy().replace('/', "\\").to_ascii_lowercase();
            let stem = dz_core::surfaces::layer_stem(&texture);
            let dir = texture.rsplit_once('\\').map_or(String::new(), |d| format!("{}\\", d.0));
            (texture, format!("{dir}{stem}.rvmat"))
        }
        None => match dz_core::surfaces::import_layer_texture(&root, &id, file) {
            Ok(v) => v,
            Err(e) => {
                app.error(format!("{}: {e:#}", t("The texture could not be copied into the map")));
                return None;
            }
        },
    };
    let name = dz_core::surfaces::layer_stem(&texture);
    add_surface_layer(app, name, texture.clone(), material);
    Some(texture)
}

pub(crate) fn add_surface_layer(app: &mut App, name: String, texture: String, material: String) {
    let Some(d) = &mut app.doc else { return };
    let n = d.world.layers.len();
    if n >= 255 {
        app.warn(t("A project holds at most 255 surfaces"));
        return;
    }
    let color = forest_core::species::hsv_to_rgb(n as f64 * 67.0 % 360.0, 0.5, 0.75);
    d.world.layers.push(SurfaceLayer { name, texture, material, color });
    d.history.revision += 1;
    app.paint.layer = n;
    app.push_palette();
    app.reload_layer_textures();
}

fn paint_mode(app: &mut App, ui: &mut egui::Ui) {
    let names: Vec<String> = app.doc.as_ref().map(|d| d.world.layers.iter().map(|l| l.name.clone()).collect()).unwrap_or_default();
    section(ui, &icons::with(icons::PAINT_BRUSH_BROAD, &t("Brush")), true, |ui| {
        // What the brush paints: the colour of the layer on the mask, its name and number, and
        // the eyedropper that takes another from the map.
        let layer = app.paint.layer;
        let mut colour = app.doc.as_ref().and_then(|d| d.world.layers.get(layer)).map(|l| l.color);
        let before = colour;
        // Another layer with the same colour cannot be told apart on the mask of the map.
        let shared = colour.is_some_and(|c| app.doc.as_ref().is_some_and(|d| d.world.layers.iter().enumerate().any(|(i, l)| i != layer && l.color == c)));
        theme::card(ui, if shared { theme::WARN.gamma_multiply(0.8) } else { theme::BORDER }, |ui| {
            ui.horizontal(|ui| {
                // The colour of the layer on the mask: click it to change it.
                match colour.as_mut() {
                    Some(c) => ui.scope(|ui| {
                        ui.spacing_mut().interact_size = vec2(32.0, 32.0);
                        ui.color_edit_button_srgb(c).on_hover_text(t("Mask colour"));
                    }).response,
                    None => ui.allocate_exact_size(vec2(32.0, 32.0), egui::Sense::hover()).1,
                };
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    match names.get(app.paint.layer) {
                        Some(n) => {
                            ui.add(egui::Label::new(RichText::new(n).font(icons::strong(14.0)).color(theme::TEXT_BRIGHT)).truncate());
                            ui.label(RichText::new(tf("Surface layer L{0}", &[&format!("{:02}", app.paint.layer)])).small().color(theme::TEXT_DIM));
                            if shared { ui.label(RichText::new(t("Another layer has this colour")).small().color(theme::WARN)).on_hover_text(t("The mask of the map tells the surfaces apart by their colours. Give each layer a colour of its own.")); }
                        }
                        None => { ui.label(RichText::new(t("Choose a surface below")).color(theme::TEXT_DIM)); }
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if flat_tool(ui, app.paint.picking, icons::EYEDROPPER, None, format!("{}\n{}", t("Eyedropper"), t("Takes the surface under the cursor."))).clicked() { app.paint.picking = !app.paint.picking; }
                });
            });
        });
        if colour != before {
            if let Some(c) = colour { app.set_layer_colour(layer, c); }
        }
        ui.add_space(4.0);
        brush_tip(app, ui, true);
        slider(ui, &t("Radius"), &mut app.paint.radius, 0.5..=2000.0, true);
        slider(ui, &t("Edge noise"), &mut app.paint.scatter, 0.0..=0.8, false);
        row(ui, &t("Paint only on"), |ui| {
            let pics: Vec<_> = app.layer_thumbs.iter().map(|t| t.as_ref().map(|t| t.id())).collect();
            let w = ui.available_width();
            theme::surface_pick(ui, "only_on", &mut app.paint.only_on, Some((None, t("(any surface)"))), &names, &pics, |i, _| Some(i), w);
        });
        theme::check_row(ui, &mut app.paint.into_modifier, t("Paint into a modifier")).on_hover_text(t("Strokes go into a mask modifier, which can be hidden, erased from or deleted, and which merges cleanly in team work when each person paints in their own. Off: strokes go straight onto the mask, under the modifiers."));
        if app.paint.into_modifier {
            let target = app.doc.as_ref().and_then(|d| app.paint.target.and_then(|id| d.world.mask_mods.modifiers.iter().find(|m| m.id == id && m.kind == dz_core::maskmod::MaskKind::Strokes)).map(|m| m.name.clone()));
            if theme::strokes_target_row(ui, target.as_deref()) { app.paint.target = None; }
        }
    });
    section(ui, &icons::with(icons::STACK, &t("Surface layers")), true, |ui| {
        layer_list(app, ui);
        ui.add_space(4.0);
        theme::button_columns(ui, 2, |c| {
            c[0].with_layout(egui::Layout::top_down_justified(egui::Align::Center), |ui| {
                add_layer_menu(app, ui);
            });
            let w1 = c[1].available_width();
            let has_layer = app.doc.as_ref().is_some_and(|d| app.paint.layer < d.world.layers.len());
            if c[1].add_enabled(has_layer, egui::Button::new(icons::with(icons::PAINT_BRUSH, &t("Material..."))).min_size(vec2(w1, 24.0))).on_hover_text(t("Its textures, relief, repeats and lighting (.rvmat)")).clicked() {
                crate::material_ui::open(app, app.paint.layer);
            }
        });
        ui.add_space(2.0);
        about(ui, &t("The brush will not add a surface to a tile that already has as many as the project allows."));
    });
    crate::surfaces_ui::panel(app, ui);
    // The mask read from the satellite is painting of surfaces, so it is here with the brush.
    crate::satellite::classify_section(app, ui);
    crate::maskmods::panel(app, ui);
    shortcuts(ui, |ui| {
        let pick = app.settings.keys.hold(Hold::Pick).name().to_string();
        brush_hints(app, ui, app.paint.into_modifier.then_some("Erase from the chosen modifier"), false, false, true);
        key_hints(ui, &[(format!("{pick}+{}", t("click")), t("Take the surface under the cursor"))]);
    });
}

fn place_mode(app: &mut App, ui: &mut egui::Ui) {
    section(ui, &icons::with(icons::TREE_EVERGREEN, &t("Place objects")), true, |ui| {
        let presets: Vec<(u64, String)> = app.library.objects.iter().map(|p| (p.id, p.name.clone())).collect();
        let preset = app.place.preset.and_then(|id| presets.iter().find(|p| p.0 == id)).map(|p| p.1.clone());
        let (badge_icon, badge_text, active) = match (&preset, &app.place.model) {
            (Some(p), _) => (icons::PACKAGE, tf("Places the preset {0}", &[p]), true),
            (None, Some(m)) => (icons::CUBE, tf("Places {0}", &[m.rsplit('\\').next().unwrap_or(m)]), true),
            (None, None) => (icons::TREE_EVERGREEN, t("Choose a model in the Content Browser"), false),
        };
        theme::card(ui, if active { theme::ACCENT.gamma_multiply(0.75) } else { theme::BORDER }, |ui| {
            let mut job = egui::text::LayoutJob::default();
            job.append(badge_icon, 0.0, egui::TextFormat { font_id: egui::FontId::proportional(13.5), color: if active { theme::ACCENT } else { theme::TEXT_DIM }, ..Default::default() });
            job.append(&badge_text, 6.0, egui::TextFormat { font_id: icons::strong(13.0), color: if active { theme::TEXT_BRIGHT } else { theme::TEXT_DIM }, ..Default::default() });
            ui.add(egui::Label::new(job).truncate());
        });
        ui.add_space(2.0);
        about(ui, &t("Click on the terrain to place it; models can also be dragged onto the map."));
        ui.add_space(2.0);
        let items = crate::library_ui::preset_items(app);
        let (thumbs, selected) = (app.thumbs.handle(), app.browser.presets().first().copied());
        crate::library_ui::object_preset_slot(ui, &t("Object preset"), "place_preset", &mut app.place.preset, &items, &thumbs, selected);
        let pool = app.place.pool.len();
        if app.place.preset.is_none() && pool > 1 && app.place.model.as_ref().is_some_and(|m| app.place.pool.contains(m)) {
            ui.add_space(2.0);
            theme::card(ui, theme::BORDER, |ui| {
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new(t("Only one")).min_size(vec2(0.0, 22.0))).clicked() { app.place.pool.clear(); }
                        ui.add(egui::Label::new(RichText::new(tf("One of the {0} models selected in the Content Browser is placed at random.", &[&pool.to_string()])).small().color(theme::TEXT_DIM)).wrap());
                    });
                });
            });
            ui.add_space(2.0);
        }
        row(ui, &t("A click"), |ui| {
            use crate::tool_icons::Glyph as G;
            crate::tool_icons::segmented(ui, &mut app.place.brush, &[
                (false, G::PlaceOne, t("Places one")),
                (true, G::PlacePaint, format!("{}\n{}", t("Paints"), t("Hold the button to paint objects."))),
            ]);
        });
        if !app.place.brush {
            let snap_key = app.settings.keys.text(Action::TogglePlaceOnSurface);
            let mut tip = t("Clicks in the 3D view place the object on the roof, floor or deck under the cursor instead of on the terrain.");
            if !snap_key.is_empty() {
                tip.push_str(&format!(" ({snap_key})"));
            }
            theme::check_row(ui, &mut app.place.snap_surface, t("Place on top of objects"))
                .on_hover_text(tip);
        }
        if app.place.brush {
            slider(ui, &t("Radius"), &mut app.place.brush_radius, 1.0..=300.0, true);
            slider(ui, &t("Spacing"), &mut app.place.brush_spacing, 0.3..=60.0, true);
            row(ui, &t("Density"), |ui| {
                ui.add(crate::theme::ValueField::new(&mut app.place.brush_density, 0.0..=40.0).custom_formatter(|v, _| if v <= 0.0 { t("no limit") } else { format!("{v:.1}") }).custom_parser(|s| s.trim().parse::<f64>().ok()))
                    .on_hover_text(t("Objects put in 100 m² of the zone you paint while the button is held. Holding still adds nothing."));
            });
            // Objects this close fit in 100 m² at most; a density above that cannot be reached.
            let room = 115.0 / (app.place.brush_spacing * app.place.brush_spacing).max(0.09);
            if app.place.brush_density > room {
                ui.add_space(2.0);
                theme::card(ui, theme::WARN.gamma_multiply(0.75), |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(theme::WARN, icons::WARNING);
                        ui.add(egui::Label::new(RichText::new(tf("The spacing leaves room for about {0} per 100 m²", &[&format!("{room:.1}")])).small().color(theme::WARN)).wrap());
                    });
                });
                ui.add_space(2.0);
            }
            theme::check_row(ui, &mut app.place.brush_avoid, t("Skip spots taken by other objects")).on_hover_text(t("Leaves out the places where buildings and other larger objects stand, so nothing is put inside them."));
        }
    });
    section(ui, &icons::with(icons::ARROWS_CLOCKWISE, &t("Rotation and size")), true, |ui| {
        // A preset places its models with its own options, so those are the ones shown here.
        let preset = app.place.preset.and_then(|id| app.library.objects.iter().position(|p| p.id == id));
        let (mut random, mut align, mut lo, mut hi) = match preset {
            Some(i) => { let p = &app.library.objects[i]; (p.random_yaw, p.align, p.scale_min, p.scale_max) }
            None => (app.place.random_yaw, app.place.align, app.place.scale_min, app.place.scale_max),
        };
        if let Some(i) = preset {
            about(ui, &tf("Saved with the preset {0}.", &[&app.library.objects[i].name]));
        }
        theme::check_row(ui, &mut random, t("Random rotation"));
        theme::check_row(ui, &mut align, t("Align to slope"));
        row(ui, &t("Scale"), |ui| {
            let tip = t("Each placed object gets a random size in this range.");
            let from_lbl = t("from");
            let to_lbl = t("to");
            ui.columns(2, |c| {
                c[0].spacing_mut().slider_width = c[0].available_width();
                let fl = from_lbl.clone();
                let fl2 = from_lbl.clone();
                c[0].add(theme::ValueField::new(&mut lo, 0.1..=4.0).step_by(0.01)
                    .custom_formatter(move |v, _| format!("{fl} {v:.2}"))
                    .custom_parser(move |s| s.trim().strip_prefix(&fl2).unwrap_or(s).trim().parse::<f64>().ok()))
                    .on_hover_text(&tip);
                c[1].spacing_mut().slider_width = c[1].available_width();
                let tl = to_lbl.clone();
                let tl2 = to_lbl.clone();
                c[1].add(theme::ValueField::new(&mut hi, 0.1..=4.0).step_by(0.01)
                    .custom_formatter(move |v, _| format!("{tl} {v:.2}"))
                    .custom_parser(move |s| s.trim().strip_prefix(&tl2).unwrap_or(s).trim().parse::<f64>().ok()))
                    .on_hover_text(&tip);
            });
        });
        if hi < lo { hi = lo; }
        match preset {
            Some(i) => {
                let p = &mut app.library.objects[i];
                if (p.random_yaw, p.align, p.scale_min, p.scale_max) != (random, align, lo, hi) {
                    (p.random_yaw, p.align, p.scale_min, p.scale_max) = (random, align, lo, hi);
                    app.lib_dirty = true;
                }
            }
            None => (app.place.random_yaw, app.place.align, app.place.scale_min, app.place.scale_max) = (random, align, lo, hi),
        }
    });
    section(ui, &icons::with(icons::STACK, &t("Terrain under objects")), false, |ui| {
        theme::check_row(ui, &mut app.place.auto_pad, t("Level the ground when placing"));
        slider(ui, &t("Blend"), &mut app.place.pad_falloff, 0.0..=60.0, false);
    });
    if app.place.brush {
        shortcuts(ui, |ui| brush_hints(app, ui, Some("Remove objects"), false, true, false));
    }
}

/// Places of the camera kept in the project, to go back to.
fn saved_views_menu(app: &mut App, ui: &mut egui::Ui) {
    ui.add_enabled_ui(app.doc.is_some(), |ui| ui.menu_button(icons::with(icons::BOOKMARKS, &t("Saved views")), |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        if ui.button(icons::with(icons::BOOKMARK_SIMPLE, &t("Keep this view"))).clicked() {
            if let Some(d) = app.doc.as_mut() {
                let n = d.world.meta.views.len() + 1;
                let c = &app.camera;
                d.world.meta.views.push(dz_core::world::SavedView { name: tf("View {0}", &[&n.to_string()]), pos: c.pos, yaw: c.yaw, pitch: c.pitch });
                d.history.revision += 1;
            }
            ui.close_menu();
        }
        let views = app.doc.as_ref().map(|d| d.world.meta.views.clone()).unwrap_or_default();
        if !views.is_empty() { ui.separator(); }
        let mut remove = None;
        for (k, view) in views.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.button(&view.name).clicked() {
                    app.camera.pos = view.pos;
                    app.camera.yaw = view.yaw;
                    app.camera.pitch = view.pitch;
                    app.top_center = [view.pos[0], view.pos[2]];
                    ui.close_menu();
                }
                if ui.small_button(icons::TRASH).on_hover_text(t("Delete")).clicked() { remove = Some(k); }
            });
        }
        if let (Some(k), Some(d)) = (remove, app.doc.as_mut()) {
            d.world.meta.views.remove(k);
            d.history.revision += 1;
        }
    }));
}

fn shapes_mode(app: &mut App, ui: &mut egui::Ui) {
    section(ui, &icons::with(icons::PATH, &t("Draw")), true, |ui| {
        ui.horizontal_wrapped(|ui| {
            use crate::tool_icons::Glyph as G;
            for (tool, glyph, name, what) in [
                (ShapeTool::Edit, G::ShapeEdit, "Edit", "Click selects a shape. Drag a dot to move a point, a + to add one, the square to move the whole shape; right click or Delete removes a point."),
                (ShapeTool::Polygon, G::ShapePolygon, "Polygon", "A closed area, for forests, fields or places to keep free."),
                (ShapeTool::Polyline, G::ShapePolyline, "Polyline", "An open line, for roads, rivers, fences and power lines to follow."),
                (ShapeTool::Point, G::ShapePoint, "Point", "A single point; a click places it."),
            ] {
                ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
                if crate::tool_icons::tile(ui, app.shape.tool == tool, glyph, &t(name), t(what)).clicked() {
                    app.shape.tool = tool;
                    app.shape.points.clear();
                }
            }
        });
        let what = match app.shape.tool {
            ShapeTool::Edit => "Click selects a shape. Drag a dot to move a point, a + to add one, the square to move the whole shape; right click or Delete removes a point.",
            ShapeTool::Polygon => "A closed area, for forests, fields or places to keep free.",
            ShapeTool::Polyline => "An open line, for roads, rivers, fences and power lines to follow.",
            ShapeTool::Point => "A single point; a click places it.",
        };
        about(ui, &t(what));
        if !matches!(app.shape.tool, ShapeTool::Edit | ShapeTool::Point) {
            ui.add_space(4.0);
            let n = app.shape.points.len();
            let text = if n == 0 { t("Click on the map where the shape starts.") } else { tf("{0} points · Enter or a double click finishes · Backspace takes the last back · Esc cancels", &[&n.to_string()]) };
            theme::card(ui, if n > 0 { theme::ACCENT.gamma_multiply(0.75) } else { theme::BORDER }, |ui| {
                let mut job = egui::text::LayoutJob::default();
                job.append(icons::POLYGON, 0.0, egui::TextFormat { font_id: egui::FontId::proportional(13.5), color: if n > 0 { theme::ACCENT } else { theme::TEXT_DIM }, ..Default::default() });
                job.append(&text, 6.0, egui::TextFormat { font_id: egui::FontId::proportional(12.5), color: if n > 0 { theme::TEXT_BRIGHT } else { theme::TEXT }, ..Default::default() });
                ui.add(egui::Label::new(job).wrap());
            });
            ui.add_space(2.0);
            angle_snap_ui(app, ui);
        }
    });
    section(ui, &icons::with(icons::POLYGON, &t("Selected shape")), true, |ui| shape_details(app, ui));
    section(ui, &icons::with(icons::WAVES, &t("Water courses from the terrain")), false, |ui| {
        about(ui, &t("Lines along which water runs down the terrain, in a new shape layer: drop the River or Stream generator on them. The Water flow shading of the view shows the same."));
        ui.add_space(2.0);
        row(ui, &t("Wetness"), |ui| { ui.add(crate::theme::ValueField::new(&mut app.shape.course_wetness, 0.3..=0.95)).on_hover_text(t("How much water must gather for a course to start: higher gives fewer, larger ones.")); });
        row(ui, &t("Shortest m"), |ui| { ui.add(crate::theme::ValueField::new(&mut app.shape.course_length, 20.0..=5000.0).logarithmic(true)); });
        ui.add_space(4.0);
        let w = ui.available_width();
        if ui.add(egui::Button::new(icons::with(icons::WAVES, &t("Find water courses"))).min_size(vec2(w, 24.0))).clicked() {
            app.find_water_courses();
        }
    });
    section(ui, &icons::with(icons::FOLDER_OPEN, &t("Files")), false, |ui| {
        theme::button_columns(ui, 2, |c| {
            let w0 = c[0].available_width();
            if c[0].add(egui::Button::new(icons::with(icons::DOWNLOAD_SIMPLE, &t("Import..."))).min_size(vec2(w0, 24.0))).on_hover_text("SHP, TV4S").clicked() { app.import_shapes(); }
            let w1 = c[1].available_width();
            if c[1].add(egui::Button::new(icons::with(icons::UPLOAD_SIMPLE, &t("Export..."))).min_size(vec2(w1, 24.0))).on_hover_text("SHP").clicked() { app.export_shapes(); }
        });
    });
}

fn keypoints_mode(app: &mut App, ui: &mut egui::Ui) {
    section(ui, &icons::with(icons::MAP_PIN, &t("Key points tool")), true, |ui| {
        ui.horizontal(|ui| {
            use crate::tool_icons::{tile, Glyph as G};
            ui.spacing_mut().item_spacing.x = 3.0;
            if tile(ui, !app.shape.keypoint_adding, G::ShapeEdit, &t("Edit"), t("Click selects a key point. Drag moves it, Delete removes it.")).clicked() { app.shape.keypoint_adding = false; }
            if tile(ui, app.shape.keypoint_adding, G::Add, &t("Add"), t("A click on the map adds a key point.")).clicked() { app.shape.keypoint_adding = true; }
        });
        about(ui, &if app.shape.keypoint_adding { t("A click on the map adds a key point.") } else { t("Click selects a key point. Drag moves it, Delete removes it.") });
    });
    let total = app.doc.as_ref().map(|d| d.world.keypoints.len()).unwrap_or(0);
    let list_title = if total > 0 {
        format!("{} ({total})", icons::with(icons::LIST_BULLETS, &t("Key points")))
    } else {
        icons::with(icons::LIST_BULLETS, &t("Key points"))
    };
    section(ui, &list_title, true, |ui| keypoints_list_ui(app, ui));
    section(ui, &icons::with(icons::CROSSHAIR, &t("Selected key point")), true, |ui| keypoint_details_ui(app, ui));
}

/// Filterable list of map key points styled with native `theme::list_row` widgets.
fn keypoints_list_ui(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        if ui.button(icons::with(icons::DOWNLOAD_SIMPLE, &t("Import..."))).on_hover_text("TV4P, HPP").clicked() {
            app.import_keypoints();
        }
        ui.add(egui::TextEdit::singleline(&mut app.shape.keypoint_filter)
            .hint_text(format!("{} {}", icons::MAGNIFYING_GLASS, t("Search...")))
            .desired_width(f32::INFINITY));
    });
    let filter = app.shape.keypoint_filter.to_lowercase();
    let Some(doc) = &app.doc else { return };
    let items: Vec<(usize, String, String, bool, f32, f32)> = doc.world.keypoints.iter().enumerate()
        .filter(|(_, k)| k.name.to_lowercase().contains(&filter) || k.text.to_lowercase().contains(&filter) || k.kind.to_lowercase().contains(&filter))
        .map(|(i, k)| (i, k.name.clone(), k.kind.clone(), k.show, k.x, k.z))
        .collect();
    let has_selected = app.shape.keypoint.is_some_and(|i| i < doc.world.keypoints.len());
    let max_h = if has_selected { 220.0 } else { (ui.available_height() - 120.0).clamp(180.0, 420.0) };
    let mut jump: Option<(f32, f32)> = None;
    egui::ScrollArea::vertical().id_salt("keypoints").max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
        for (i, name, kind, show, x, z) in items {
            let tint = if show { theme::KIND_POINT } else { theme::TEXT_DIM };
            let r = theme::list_row(ui, app.shape.keypoint == Some(i), icons::MAP_PIN, tint, name.as_str(), &kind);
            if r.clicked() { app.shape.keypoint = Some(i); }
            if r.double_clicked() { jump = Some((x, z)); }
        }
    });
    if let Some((x, z)) = jump {
        let ground = app.doc.as_ref().map(|d| d.world.height.sample(x, z)).unwrap_or(0.0);
        app.camera.pos = [x, ground + 80.0, z - 120.0];
        app.camera.look_at([x, ground, z]);
        app.top_center = [x, z];
    }
}

/// Selected key point inspector styled with native `theme` rows, `ValueField`, and `vector_row`.
fn keypoint_details_ui(app: &mut App, ui: &mut egui::Ui) {
    use dz_core::keypoints::TYPES;
    let Some(i) = app.shape.keypoint.filter(|i| app.doc.as_ref().is_some_and(|d| *i < d.world.keypoints.len())) else {
        app.shape.keypoint = None;
        ui.weak(t("Nothing selected"));
        return;
    };
    let mut jump: Option<(f32, f32)> = None;
    let mut delete = false;
    {
        let doc = app.doc.as_mut().unwrap();
        let mut k = doc.world.keypoints[i].clone();
        theme::details_head(ui, icons::MAP_PIN, theme::KIND_POINT, &k.name, &k.kind);
        ui.add_space(4.0);
        row(ui, &t("Name"), |ui| {
            ui.add_sized([ui.available_width(), 22.0], egui::TextEdit::singleline(&mut k.name));
        });
        row(ui, &t("Type"), |ui| {
            egui::ComboBox::from_id_salt("keypoint type").width(ui.available_width()).selected_text(k.kind.clone()).show_ui(ui, |ui| {
                for n in TYPES { ui.selectable_value(&mut k.kind, n.to_string(), n); }
            });
        });
        row(ui, &t("Text"), |ui| {
            ui.add_sized([ui.available_width(), 22.0], egui::TextEdit::singleline(&mut k.text));
        });
        theme::check_row(ui, &mut k.show, t("Show on map"));
        theme::vector_row(ui, &t("Position"), &mut [
            (&mut k.x, 1.0, theme::AXIS[0], "X".into()),
            (&mut k.z, 1.0, theme::AXIS[2], "Y".into()),
        ]);
        row(ui, &t("Angle"), |ui| {
            ui.add(theme::ValueField::new(&mut k.angle, -360.0..=360.0).step_by(0.5).suffix("°"));
        });
        row(ui, &t("Width"), |ui| {
            ui.add(theme::ValueField::new(&mut k.width, 0.0..=5000.0).step_by(1.0).suffix(" m"));
        });
        row(ui, &t("Height"), |ui| {
            ui.add(theme::ValueField::new(&mut k.height, 0.0..=5000.0).step_by(1.0).suffix(" m"));
        });
        theme::subsection(ui, &t("Properties"), !k.properties.trim().is_empty(), |ui| {
            ui.add(egui::TextEdit::multiline(&mut k.properties).desired_rows(3).desired_width(f32::INFINITY));
        });
        ui.add_space(4.0);
        let (kx, kz) = (k.x, k.z);
        theme::button_columns(ui, 2, |cols| {
            if cols[0].add_sized([cols[0].available_width(), 24.0], egui::Button::new(icons::with(icons::CROSSHAIR, &t("Go to")))).clicked() {
                jump = Some((kx, kz));
            }
            if cols[1].add_sized([cols[1].available_width(), 24.0], egui::Button::new(icons::with(icons::TRASH, &t("Delete")))).clicked() {
                delete = true;
            }
        });
        let edited = k != doc.world.keypoints[i];
        if edited {
            let before = doc.world.keypoints.clone();
            app.shape.keypoints_before.get_or_insert(before);
            doc.world.keypoints[i] = k;
        } else if !ui.ctx().wants_keyboard_input() && !ui.ctx().input(|p| p.pointer.any_down()) {
            // A whole edit is one undo step.
            if !app.shape.keypoint_moving {
                if let Some(before) = app.shape.keypoints_before.take().filter(|b| *b != doc.world.keypoints) {
                    doc.history.push(t("Edit key point"), dz_core::edit::Change::KeyPoints(before));
                }
            }
        }
    }
    if let Some((x, z)) = jump {
        let ground = app.doc.as_ref().map(|d| d.world.height.sample(x, z)).unwrap_or(0.0);
        app.camera.pos = [x, ground + 80.0, z - 120.0];
        app.camera.look_at([x, ground, z]);
        app.top_center = [x, z];
    }
    if delete {
        app.delete_keypoint(i);
    }
}

/// Vertices, attributes and layer of the selected shapes.
fn shape_details(app: &mut App, ui: &mut egui::Ui) {
    use dz_core::shapes::ShapeKind;
    let count = app.shape.selection.len();
    if count == 0 {
        theme::card(ui, theme::BORDER, |ui| {
            let mut job = egui::text::LayoutJob::default();
            job.append(icons::POLYGON, 0.0, egui::TextFormat { font_id: egui::FontId::proportional(13.5), color: theme::TEXT_DIM, ..Default::default() });
            job.append(&t("Nothing selected"), 6.0, egui::TextFormat { font_id: icons::strong(13.0), color: theme::TEXT_DIM, ..Default::default() });
            ui.label(job);
        });
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
        theme::details_head(ui, icons::POLYGON, theme::KIND_SHAPE, &tf("{0} shapes selected", &[&count.to_string()]), names.get(target).map(String::as_str).unwrap_or(""));
    } else {
        let kind_name = match shape.kind { ShapeKind::Point => t("Point"), ShapeKind::Polyline => t("Polyline"), ShapeKind::Polygon => t("Polygon") };
        theme::details_head(ui, icons::POLYGON, theme::KIND_SHAPE, &kind_name, &tf("{0} points", &[&shape.points.len().to_string()]));
        if shape.kind != ShapeKind::Point {
            let mut kind = shape.kind;
            row(ui, &t("Type"), |ui| {
                theme::segmented(ui, &mut kind, &[
                    (ShapeKind::Polygon, t("Polygon"), String::new()),
                    (ShapeKind::Polyline, t("Polyline"), String::new()),
                ]);
            });
            if kind != shape.kind && shape.points.len() >= kind.min_points() {
                shape.kind = kind;
                changed = true;
            }
        }
    }
    row(ui, &t("Layer"), |ui| {
        let w = ui.available_width();
        egui::ComboBox::from_id_salt("shape_layer").width(w).selected_text(names.get(target).cloned().unwrap_or_default()).show_ui(ui, |ui| {
            for (i, n) in names.iter().enumerate() { ui.selectable_value(&mut target, i, n); }
        });
    });
    if count == 1 {
        theme::subsection(ui, &tf("{0} points", &[&shape.points.len().to_string()]), shape.points.len() <= 12, |ui| {
            egui::Frame::new()
                .fill(theme::DARK)
                .stroke(egui::Stroke::new(1.0_f32, theme::BORDER))
                .corner_radius(4)
                .inner_margin(egui::Margin::same(4))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().id_salt("shape_point_list").max_height(220.0).auto_shrink([false, true]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        let marked = app.shape.vertex;
                        for (i, p) in shape.points.iter_mut().enumerate() {
                            let is_marked = marked == Some(i);
                            let bg = if is_marked { theme::ACCENT_SOFT } else if i % 2 == 1 { theme::ROW_ALT } else { Color32::TRANSPARENT };
                            egui::Frame::new().fill(bg).corner_radius(2).inner_margin(egui::Margin::symmetric(3, 1)).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    if ui.add_sized([22.0, 20.0], egui::SelectableLabel::new(is_marked, RichText::new(format!("{}", i + 1)).monospace().small())).clicked() {
                                        app.shape.vertex = Some(i);
                                    }
                                    let field_w = ((ui.available_width() - 4.0) * 0.5).max(52.0);
                                    changed |= ui.add_sized([field_w, 20.0], egui::DragValue::new(&mut p[0]).speed(0.1).prefix("X ")).changed();
                                    changed |= ui.add_sized([field_w, 20.0], egui::DragValue::new(&mut p[1]).speed(0.1).prefix("Z ")).changed();
                                });
                            });
                        }
                    });
                });
        });
        for (k, v) in &mut shape.attributes {
            row(ui, k, |ui| { changed |= ui.add(egui::TextEdit::singleline(v).desired_width(f32::INFINITY)).changed(); });
        }
        if shape.kind != ShapeKind::Point {
            ui.add_space(4.0);
            let w = ui.available_width();
            if ui.add(egui::Button::new(icons::with(icons::PATH, &t("Use as a road or object line"))).min_size(vec2(w, 24.0))).on_hover_text(t("Opens the Roads tool with these points.")).clicked() {
                to_line = Some(shape.points.clone());
            }
        }
    }
    let mut select_inside = None;
    if app.selected_areas() > 0 {
        let w = ui.available_width();
        let r = ui.add(egui::Button::new(icons::with(icons::SELECTION_ALL, &t("Select objects inside"))).min_size(vec2(w, 24.0))).on_hover_text(t("Selects the objects inside the selected areas and opens the Select tool. Hold Shift to add them to what is already selected."));
        if r.clicked() { select_inside = Some(ui.input(|i| i.modifiers.shift)); }
    }
    let w = ui.available_width();
    if ui.add(egui::Button::new(RichText::new(icons::with(icons::TRASH, &t("Delete shape"))).color(Color32::from_rgb(235, 110, 100))).min_size(vec2(w, 24.0))).clicked() { remove = true; }
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
    } else if let Some(add) = select_inside {
        app.select_inside_shapes(add);
    }
}

/// The road bridges that can be chosen, read from the work drive once.
fn bridge_models() -> &'static [String] {
    // Read again when another work drive is chosen.
    static MODELS: std::sync::Mutex<Option<(std::path::PathBuf, &'static [String])>> = std::sync::Mutex::new(None);
    let drive = dayz_road_tool::drive::work_drive();
    let mut cached = MODELS.lock().unwrap();
    if cached.as_ref().is_none_or(|(d, _)| *d != drive) {
        let list: &'static [String] = Vec::leak(dayz_road_tool::rails::BridgeKit::road_models(&drive));
        *cached = Some((drive, list));
    }
    cached.as_ref().unwrap().1
}

fn routing_fields(ui: &mut egui::Ui, r: &mut dayz_road_tool::document::RoutingSettings, thumbs: &crate::models::ThumbPeek) -> bool {
    let mut changed = false;
    let mut field = |ui: &mut egui::Ui, label: String, v: &mut f64, range: std::ops::RangeInclusive<f64>| {
        row(ui, &label, |ui| { changed |= ui.add(crate::theme::ValueField::new(v, range)).changed(); });
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
    row(ui, &t("Bridge cost"), |ui| { changed |= ui.add(crate::theme::ValueField::new(&mut r.bridge_weight, 0.5..=10.0)).on_hover_text(t("A metre of bridge counts as this many metres of road. Lower it when cheap earthworks lead the road down into valleys instead of over them.")).changed(); });
    changed |= theme::check_row(ui, &mut r.bridge_over_forbidden, t("Bridges may cross forbidden areas and water")).changed();
    {
        use crate::asset_browser::Pick;
        let name = |m: &str| m.rsplit(['\\', '/']).next().unwrap_or(m).trim_end_matches(".p3d").to_string();
        let items: Vec<crate::theme::PickItem<String>> = bridge_models().iter().map(|m| crate::theme::PickItem { key: m.clone(), name: name(m), note: m.clone(), model: Some(m.clone()), picture: None }).collect();
        let known = |m: &str| bridge_models().iter().any(|b| b.eq_ignore_ascii_case(m));
        changed |= crate::slots::preset_slot(ui, &t("Bridge model"), "bridge_model", &mut r.bridge_model, Some((String::new(), t("By road width"))), &items, thumbs, &|p| if let Pick::Model(m) = p { known(m).then(|| m.clone()) } else { None }, &|m| (!m.is_empty()).then(|| Pick::Model(m.clone())), None, None);
    }
    row(ui, &t("Deck height"), |ui| {
        let shown = t(if r.deck_fixed { "Fixed" } else { "Between the banks" });
        egui::ComboBox::from_id_salt("deck_height").selected_text(shown).width(ui.available_width()).show_ui(ui, |ui| {
            changed |= ui.selectable_value(&mut r.deck_fixed, false, t("Between the banks")).changed();
            changed |= ui.selectable_value(&mut r.deck_fixed, true, t("Fixed")).changed();
        });
    });
    if r.deck_fixed {
        row(ui, &t("Deck at m"), |ui| { changed |= ui.add(crate::theme::ValueField::new(&mut r.deck_height, -50.0..=2000.0).suffix(" m")).on_hover_text(t("Height of the driving surface above sea level. The road on both banks rises or falls to it.")).changed(); });
    } else {
        row(ui, &t("Raise deck m"), |ui| { changed |= ui.add(crate::theme::ValueField::new(&mut r.deck_raise, -5.0..=20.0)).on_hover_text(t("Lifts the deck above the middle of its two banks; the road on both banks rises to it.")).changed(); });
    }
    changed
}

/// Everything about the route being edited.
/// The selected route in sections: what it is and the main actions first, then how it goes
/// on at its ends, its points, a crossroad, and the ground under it, the last ones folded.
fn route_details(app: &mut App, ui: &mut egui::Ui) {
    let Some(index) = app.line.selected_route.filter(|i| app.doc.as_ref().is_some_and(|d| *i < d.world.routes.len())) else {
        section(ui, &icons::with(icons::ROAD_HORIZON, &t("Selected route")), true, |ui| {
            theme::card(ui, theme::BORDER, |ui| {
                let mut job = egui::text::LayoutJob::default();
                job.append(icons::ROAD_HORIZON, 0.0, egui::TextFormat { font_id: egui::FontId::proportional(13.5), color: theme::TEXT_DIM, ..Default::default() });
                job.append(&t("Click a route on the map to edit it."), 6.0, egui::TextFormat { font_id: egui::FontId::proportional(12.5), color: theme::TEXT_DIM, ..Default::default() });
                ui.add(egui::Label::new(job).wrap());
            });
        });
        return;
    };
    let before = app.doc.as_ref().unwrap().world.routes[index].points.clone();
    let mut action = 0;
    let mut regrade = false;
    section(ui, &icons::with(icons::ROAD_HORIZON, &t("Selected route")), true, |ui| {
        let families = app.road_families.clone();
        let route = &mut app.doc.as_mut().unwrap().world.routes[index];
        row(ui, &t("Name"), |ui| { ui.add(egui::TextEdit::singleline(&mut route.name).desired_width(f32::INFINITY)); });
        about(ui, &tf("{0} models · {1} points", &[&route.objects.len().to_string(), &route.points.len().to_string()]));
        row(ui, &t("Road type"), |ui| {
            let w = ui.available_width();
            egui::ComboBox::from_id_salt("route_family").selected_text(route.family.clone()).width(w).truncate().show_ui(ui, |ui| {
                for f in families { ui.selectable_value(&mut route.family, f.clone(), f); }
            });
        });
        let mut method = route.method;
        ui.add_space(2.0);
        ui.label(RichText::new(t("Edit as")).small().color(theme::TEXT_DIM));
        ui.horizontal_wrapped(|ui| {
            for (m, g, n, tip) in draw_methods() {
                ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
                if crate::tool_icons::tile(ui, method == m, g, &t(n), t(tip)).clicked() { method = m; }
            }
        });
        if method != route.method {
            route.method = method;
            // Live editing shows the fit along the points, which the route then keeps.
            if method == RoadDraw::Live { route.terrain_route = false; }
        }
        if method == RoadDraw::Points { theme::check_row(ui, &mut route.terrain_route, t("Route across terrain")); }
        if method != RoadDraw::Parts {
            theme::check_row(ui, &mut app.line.live_keep, t("Keep the pieces already laid")).on_hover_text(t("On: changing a point fits the route again only from that point on; the pieces before it stay as they are. Off: the whole route is fitted again."));
        }
        let route = &mut app.doc.as_mut().unwrap().world.routes[index];
        let thumbs = app.thumbs.handle();
        theme::subsection(ui, &t("Route settings"), false, |ui| { routing_fields(ui, &mut route.settings, &thumbs); });
        ui.add_space(4.0);
        if method != RoadDraw::Parts {
            theme::button_columns(ui, 2, |c| {
                let w0 = c[0].available_width();
                if c[0].add(egui::Button::new(icons::with(icons::ARROWS_CLOCKWISE, &t("Fit again"))).min_size(vec2(w0, 24.0))).on_hover_text(t("Applies the type, settings and typed points.")).clicked() { action = 1; }
                let w1 = c[1].available_width();
                if c[1].add(egui::Button::new(icons::with(icons::ARROWS_LEFT_RIGHT, &t("Reverse"))).min_size(vec2(w1, 24.0))).on_hover_text(t("The road runs the other way: its start becomes its end.")).clicked() { action = 2; }
            });
        }
        let w = ui.available_width();
        if ui.add(egui::Button::new(RichText::new(icons::with(icons::TRASH, &t("Delete route"))).color(Color32::from_rgb(235, 110, 100))).min_size(vec2(w, 24.0))).clicked() { action = 4; }
    });
    if app.doc.as_ref().unwrap().world.routes[index].method == RoadDraw::Parts {
        section(ui, &icons::with(icons::PUZZLE_PIECE, &t("Pieces")), true, |ui| {
            let route = &app.doc.as_ref().unwrap().world.routes[index];
            let now = crate::road_draw::stamp(&route.parts);
            let taken = app.road_parts.target.filter(|t| t.0 == index && app.road_parts.stamp == now);
            if taken.is_none() { crate::road_draw::load_route(app, index, app.road_parts.target.is_some_and(|t| t.0 == index && t.1)); }
            let mut at_start = app.road_parts.target.is_some_and(|t| t.1);
            row(ui, &t("Change the pieces at the"), |ui| {
                use crate::tool_icons::Glyph as G;
                if crate::tool_icons::segmented(ui, &mut at_start, &[
                    (true, G::EndStart, t("start")),
                    (false, G::EndEnd, t("end")),
                ]) {
                    crate::road_draw::load_route(app, index, at_start);
                }
            });
            if app.road_parts.open.is_some() {
                parts_panel(app, ui);
            } else {
                theme::card(ui, theme::WARN.gamma_multiply(0.75), |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(theme::WARN, icons::WARNING);
                        ui.add(egui::Label::new(RichText::new(t("This route keeps no pieces to go on from. Fit it again with points first.")).small().color(theme::WARN)).wrap());
                    });
                });
            }
        });
    } else {
        if app.road_parts.target.is_some() {
            app.road_parts = crate::road_draw::PartsDraw { next: app.road_parts.next.take(), left: app.road_parts.left, ..Default::default() };
        }
        section(ui, &icons::with(icons::ARROWS_OUT_LINE_HORIZONTAL, &t("Extend")), true, |ui| {
            theme::button_columns(ui, 2, |c| {
                let start_on = app.line.extend == Some(true);
                let end_on = app.line.extend == Some(false);
                let w0 = c[0].available_width();
                if c[0].add(egui::Button::new(RichText::new(icons::with(icons::ARROW_LINE_LEFT, &t("Extend start"))).color(if start_on { Color32::WHITE } else { theme::TEXT })).fill(if start_on { theme::ACCENT } else { theme::SECTION }).min_size(vec2(w0, 24.0))).clicked() {
                    app.line.extend = if start_on { None } else { Some(true) };
                }
                let w1 = c[1].available_width();
                if c[1].add(egui::Button::new(RichText::new(icons::with(icons::ARROW_LINE_RIGHT, &t("Extend end"))).color(if end_on { Color32::WHITE } else { theme::TEXT })).fill(if end_on { theme::ACCENT } else { theme::SECTION }).min_size(vec2(w1, 24.0))).clicked() {
                    app.line.extend = if end_on { None } else { Some(false) };
                }
            });
            ui.add_space(2.0);
            about(ui, &t("Then click on the map beyond that end; Ctrl+click picks the nearer end itself."));
            theme::check_row(ui, &mut app.line.extend_only, t("Fit only the new stretch when extending")).on_hover_text(t("A route lengthened at its start or end keeps its models; Fit again lays models only from the old end to the new points. Moving other points still fits the whole route."));
        });
        let mut typed = false;
        let mut remove: Option<usize> = None;
        let count = app.doc.as_ref().unwrap().world.routes[index].points.len();
        section(ui, &icons::with(icons::DOTS_THREE, &t("Points")), false, |ui| {
            about(ui, &tf("{0} points; typed values wait for Fit again.", &[&count.to_string()]));
            ui.add_space(2.0);
            let marked = app.line.selected_point;
            let route = &mut app.doc.as_mut().unwrap().world.routes[index];
            let removable = route.points.len() > 2;
            egui::Frame::new()
                .fill(theme::DARK)
                .stroke(egui::Stroke::new(1.0_f32, theme::BORDER))
                .corner_radius(4)
                .inner_margin(egui::Margin::same(4))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().id_salt("route_point_list").max_height(220.0).auto_shrink([false, true]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for (i, p) in route.points.iter_mut().enumerate() {
                            let is_marked = marked == Some(i);
                            let bg = if is_marked { theme::ACCENT_SOFT } else if i % 2 == 1 { theme::ROW_ALT } else { Color32::TRANSPARENT };
                            egui::Frame::new().fill(bg).corner_radius(2).inner_margin(egui::Margin::symmetric(3, 1)).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    if ui.add_sized([22.0, 20.0], egui::SelectableLabel::new(is_marked, RichText::new(format!("{}", i + 1)).monospace().small())).clicked() {
                                        app.line.selected_point = Some(i);
                                    }
                                    let field_w = ((ui.available_width() - 28.0) * 0.5).max(46.0);
                                    typed |= ui.add_sized([field_w, 20.0], egui::DragValue::new(&mut p[0]).speed(0.5).prefix("X ")).changed();
                                    typed |= ui.add_sized([field_w, 20.0], egui::DragValue::new(&mut p[1]).speed(0.5).prefix("Z ")).changed();
                                    if ui.add_enabled(removable, egui::Button::new(icons::X).frame(false).min_size(vec2(20.0, 20.0))).on_hover_text(t("Delete the point (Delete or Backspace)")).clicked() {
                                        remove = Some(i);
                                    }
                                });
                            });
                        }
                    });
                });
        });
        if let Some(i) = remove {
            app.delete_route_point(index, i);
            return;
        }
        if typed && app.line.typed.as_ref().is_none_or(|t| t.0 != index) { app.line.typed = Some((index, before.clone())); }
        section(ui, &icons::with(icons::GIT_FORK, &t("Crossroad")), false, |ui| crossroad_fields(app, ui, index));
    }
    section(ui, &icons::with(icons::MOUNTAINS, &t("Terrain under the road")), false, |ui| {
        let road_width = {
            let d = app.doc.as_ref().unwrap();
            d.world.routes[index].objects.iter().take(24).filter_map(|id| d.world.objects.get(*id)).map(|o| app.db.get(d.world.objects.model_path(o))).filter(|i| i.known).map(|i| (i.max[0] - i.min[0]).min(i.max[2] - i.min[2])).fold(0.0f32, f32::max)
        };
        let route = &mut app.doc.as_mut().unwrap().world.routes[index];
        let grading = (route.grade, route.grade_width, route.grade_falloff, route.grading);
        theme::check_row(ui, &mut route.grade, t("Grade the terrain under the road"));
        ui.add_enabled_ui(route.grade, |ui| {
            let w = ui.available_width();
            if ui.add_enabled(road_width > 0.5, egui::Button::new(icons::with(icons::RULER, &t("Take the width from the road model"))).min_size(vec2(w, 24.0))).clicked() { route.grade_width = road_width; }
            ui.add_space(2.0);
            grading_ui(ui, &mut route.grading, &mut route.grade_width, &mut route.grade_falloff, !route.profile.is_empty());
        });
        regrade = grading != (route.grade, route.grade_width, route.grade_falloff, route.grading);
        if regrade {
            // The route as it was before this change, for one undo step of the whole drag.
            let mut previous = app.doc.as_ref().unwrap().world.routes.clone();
            let r = &mut previous[index];
            (r.grade, r.grade_width, r.grade_falloff, r.grading) = grading;
            app.begin_grading_edit(previous);
        }
    });
    // Typed values wait for "Fit again", which fits from the points before the typing.
    let typed_before = if action == 1 || action == 2 { app.line.typed.take().filter(|t| t.0 == index).map(|t| t.1) } else { None };
    match action {
        1 => app.fit_route_again(index, typed_before.unwrap_or(before)),
        2 => {
            if let Some(d) = &mut app.doc { d.world.routes[index].points.reverse(); }
            app.line.selected_point = None;
            app.fit_route_again(index, typed_before.unwrap_or(before));
        }
        4 => {
            if let Some(id) = app.doc.as_ref().and_then(|d| d.world.routes.get(index)).map(|r| r.id) { crate::deleting::request(app, crate::deleting::Target::Route(id)); }
        }
        _ if regrade => app.route_grading_changed(index),
        _ => {}
    }
}

/// Choosing a crossroad and putting it into the selected route with a click on it.
fn crossroad_fields(app: &mut App, ui: &mut egui::Ui, index: usize) {
    let Some(family) = app.doc.as_ref().and_then(|d| d.world.routes.get(index)).map(|r| r.family.clone()) else { return };
    if dayz_road_tool::rails::is_track_family(&family) {
        return;
    }
    let list = crate::crossroads::choices(&app.road_catalog, &family);
    if list.is_empty() {
        return;
    }
    if !list.iter().any(|c| c.path == app.line.crossroad_model) {
        app.line.crossroad_model = list[0].path.clone();
    }
    let chosen = list.iter().find(|c| c.path == app.line.crossroad_model).map(crate::crossroads::label).unwrap_or_default();
    row(ui, &t("Crossroad"), |ui| {
        let w = ui.available_width();
        egui::ComboBox::from_id_salt("route_crossroad").selected_text(chosen).width(w).truncate().show_ui(ui, |ui| {
            for c in &list {
                ui.selectable_value(&mut app.line.crossroad_model, c.path.clone(), crate::crossroads::label(c));
            }
        });
    });
    ui.add_space(4.0);
    let armed = app.line.crossroad_armed;
    let w = ui.available_width();
    let btn = egui::Button::new(RichText::new(icons::with(icons::GIT_FORK, &t("Put in a crossroad"))).color(if armed { Color32::WHITE } else { theme::TEXT }))
        .fill(if armed { theme::ACCENT } else { theme::SECTION })
        .min_size(vec2(w, 24.0));
    if ui.add(btn).on_hover_text(t("Click on the route where the crossroad goes; its side arm faces the side you click. The route is laid again as two roads leaving the crossroad.")).clicked() {
        app.line.crossroad_armed = !armed;
        app.line.extend = None;
    }
    if armed {
        ui.add_space(4.0);
        theme::card(ui, theme::WARN.gamma_multiply(0.75), |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(theme::WARN, icons::GIT_FORK);
                ui.add(egui::Label::new(RichText::new(t("Click on the route where the crossroad goes (Esc cancels).")).small().color(theme::WARN)).wrap());
            });
        });
    }
}

fn lines_mode(app: &mut App, ui: &mut egui::Ui) {
    lines_tool_panel(app, ui);
    if app.line.action == LineAction::Edit {
        route_details(app, ui);
    }
}

/// The pieces of the road type to lay one after another, and the road laid so far.
fn parts_panel(app: &mut App, ui: &mut egui::Ui) {
    let k = &app.settings.keys;
    let d = &app.road_parts;
    let editing = d.target.is_some();
    let stage = if editing { t("Lay pieces at the open end or take them away, then apply.") } else if d.open.is_some() { t("Choose a piece and click on the map (or press its key) to lay it.") } else if d.start.is_some() { t("Click where the road heads.") } else { t("Click where the road starts.") };
    theme::card(ui, if d.open.is_some() { theme::ACCENT.gamma_multiply(0.75) } else { theme::BORDER }, |ui| {
        let mut job = egui::text::LayoutJob::default();
        job.append(icons::PUZZLE_PIECE, 0.0, egui::TextFormat { font_id: egui::FontId::proportional(13.5), color: if d.open.is_some() { theme::ACCENT } else { theme::TEXT_DIM }, ..Default::default() });
        job.append(&stage, 6.0, egui::TextFormat { font_id: egui::FontId::proportional(12.5), color: if d.open.is_some() { theme::TEXT_BRIGHT } else { theme::TEXT }, ..Default::default() });
        ui.add(egui::Label::new(job).wrap());
    });
    key_hints(ui, &[
        (k.text(Action::RoadPieceAdd), t(Action::RoadPieceAdd.name())),
        (format!("{} / {}", k.text(Action::RoadPiecePrev), k.text(Action::RoadPieceNext)), t("Previous or next piece")),
        (k.text(Action::RoadPieceSide), t(Action::RoadPieceSide.name())),
        ("Backspace".into(), t("Take the last piece away")),
        ("Enter".into(), if editing { t("Apply") } else { t("Finish the road") }),
        ("Esc".into(), if editing { t("Discard the changes") } else { t("Cancel") }),
    ]);
    ui.add_space(2.0);
    row(ui, &t("Curves bend"), |ui| {
        use crate::tool_icons::Glyph as G;
        crate::tool_icons::segmented(ui, &mut app.road_parts.left, &[
            (true, G::BendLeft, t("Left")),
            (false, G::BendRight, t("Right")),
        ]);
    });
    let list = crate::road_draw::choices(&app.road_catalog, &crate::road_draw::family_of(app));
    if list.is_empty() {
        theme::card(ui, theme::WARN.gamma_multiply(0.75), |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(theme::WARN, icons::WARNING);
                ui.add(egui::Label::new(RichText::new(t("No pieces of this road type are loaded")).small().color(theme::WARN)).wrap());
            });
        });
    } else {
        egui::Frame::new()
            .fill(theme::DARK)
            .stroke(egui::Stroke::new(1.0_f32, theme::BORDER))
            .corner_radius(4)
            .inner_margin(egui::Margin::same(3))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("road_pieces").max_height(220.0).auto_shrink([false, true]).show(ui, |ui| {
                    for c in &list {
                        let name = c.path.rsplit(['\\', '/']).next().unwrap_or(&c.path).trim_end_matches(".p3d").to_string();
                        let turn = crate::road_draw::turn_of(c).abs();
                        let right = if c.category == 4 { format!("{turn:.1}°  {:.1} m", c.model.length) } else { format!("{:.1} m", c.model.length) };
                        let chosen = app.road_parts.next.as_ref() == Some(&c.path);
                        let r = theme::list_row(ui, chosen, icons::PUZZLE_PIECE, theme::KIND_ROAD, name.as_str(), &right);
                        if r.clicked() { app.road_parts.next = Some(c.path.clone()); }
                        if r.double_clicked() { crate::road_draw::add_piece(app); }
                    }
                });
            });
    }
    let laid = app.road_parts.parts.iter().filter(|p| !dayz_road_tool::roads::is_cap(p, &app.road_catalog)).count();
    ui.add_space(2.0);
    ui.label(RichText::new(tf("{0} pieces laid", &[&laid.to_string()])).small().color(theme::TEXT_DIM));
    theme::button_columns(ui, 2, |c| {
        let w0 = c[0].available_width();
        if c[0].add_enabled(app.road_parts.open.is_some(), egui::Button::new(icons::with(icons::PLUS, &t("Lay"))).min_size(vec2(w0, 24.0))).clicked() { crate::road_draw::add_piece(app); }
        let w1 = c[1].available_width();
        if c[1].add_enabled(laid > 0, egui::Button::new(icons::with(icons::ARROW_COUNTER_CLOCKWISE, &t("Take the last away"))).min_size(vec2(w1, 24.0))).clicked() { crate::road_draw::parts_back(app); }
    });
    let finish = if editing { t("Apply") } else { t("Finish the road") };
    let w = ui.available_width();
    let finish_btn = if laid > 0 {
        egui::Button::new(RichText::new(icons::with(icons::CHECK, &finish)).color(Color32::WHITE)).fill(theme::ACCENT).stroke(egui::Stroke::NONE).min_size(vec2(w, 24.0))
    } else {
        egui::Button::new(icons::with(icons::CHECK, &finish)).min_size(vec2(w, 24.0))
    };
    if ui.add_enabled(laid > 0, finish_btn).clicked() { crate::road_draw::parts_finish(app, app.line.grade); }
}

fn lines_tool_panel(app: &mut App, ui: &mut egui::Ui) {
    section(ui, &icons::with(icons::PATH, &t("Draw")), true, |ui| {
        // Roads are what this tool is for; the two plain line tools wait in a menu.
        ui.horizontal_wrapped(|ui| {
            for (a, glyph, n) in [(LineAction::Road, crate::tool_icons::Glyph::RoadNew, "New road"), (LineAction::Edit, crate::tool_icons::Glyph::RoadEdit, "Edit routes")] {
                ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
                if crate::tool_icons::tile(ui, app.line.action == a, glyph, &t(n), t(n)).clicked() { app.line.action = a; }
            }
            let other = matches!(app.line.action, LineAction::Fence | LineAction::Plain);
            let label = match app.line.action {
                LineAction::Fence => t("Objects along a line"),
                LineAction::Plain => t("Line to shapes"),
                _ => t("More"),
            };
            let button = egui::Button::new(egui::RichText::new(format!("{label} ⏷")).color(if other { Color32::WHITE } else { theme::TEXT })).fill(if other { theme::ACCENT } else { theme::SECTION }).min_size(vec2(56.0, 50.0)).corner_radius(4);
            egui::menu::menu_custom_button(ui, button, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                if ui.selectable_label(app.line.action == LineAction::Fence, t("Objects along a line")).on_hover_text(t("Places one model at even spacing along a clicked line, once. The Line generators do the same with more options and can be run again.")).clicked() { app.line.action = LineAction::Fence; ui.close_menu(); }
                if ui.selectable_label(app.line.action == LineAction::Plain, t("Line to shapes")).on_hover_text(t("A clicked line becomes a polyline on the active shape layer, as the Shapes tool draws it.")).clicked() { app.line.action = LineAction::Plain; ui.close_menu(); }
            });
        });
        ui.add_space(4.0);
        match app.line.action {
            LineAction::Road => road_draw_fields(app, ui),
            LineAction::Edit => {
                hint(ui, &t("Click a route to edit it. Drag its points; the + between two points adds one; Ctrl+click beyond an end extends it; Delete or Backspace removes the marked point."));
            }
            LineAction::Fence => {
                let thumbs = app.thumbs.handle();
                crate::slots::model_slot(ui, &t("Model"), &mut app.line.fence_model, app.place.model.as_deref(), &thumbs, None);
                slider(ui, &t("Spacing"), &mut app.line.fence_spacing, 0.5..=100.0, true);
                drawing_status(app, ui);
            }
            LineAction::Plain => drawing_status(app, ui),
        }
    });
    if app.line.action == LineAction::Road && app.line.draw != RoadDraw::Parts {
        section(ui, &icons::with(icons::CIRCLE_DASHED, &t("Roundabout")), false, |ui| crate::roundabouts::fields(app, ui));
    }
    if app.line.action == LineAction::Road {
        section(ui, &icons::with(icons::MOUNTAINS, &t("Terrain under the road")), false, |ui| {
            theme::check_row(ui, &mut app.line.grade, t("Grade the terrain under the road"));
            ui.add_enabled_ui(app.line.grade, |ui| {
                grading_ui(ui, &mut app.line.grading, &mut app.line.grade_width, &mut app.line.grade_falloff, false);
            });
        });
    }
}

/// How many points are set, the keys, and the angle step, while a line is drawn.
fn drawing_status(app: &mut App, ui: &mut egui::Ui) {
    let n = app.line.points.len();
    let text = if n == 0 { t(if app.line.action == LineAction::Road { "Click on the map where the road starts." } else { "Click on the map where the line starts." }) } else { tf("{0} points · Enter finishes · Backspace takes the last back · Esc cancels", &[&n.to_string()]) };
    theme::card(ui, if n > 0 { theme::ACCENT.gamma_multiply(0.75) } else { theme::BORDER }, |ui| {
        let mut job = egui::text::LayoutJob::default();
        job.append(icons::PATH, 0.0, egui::TextFormat { font_id: egui::FontId::proportional(13.5), color: if n > 0 { theme::ACCENT } else { theme::TEXT_DIM }, ..Default::default() });
        job.append(&text, 6.0, egui::TextFormat { font_id: egui::FontId::proportional(12.5), color: if n > 0 { theme::TEXT_BRIGHT } else { theme::TEXT }, ..Default::default() });
        ui.add(egui::Label::new(job).wrap());
    });
    ui.add_space(2.0);
    angle_snap_ui(app, ui);
}

/// The ways to lay a road: how each is drawn, its name and what it does.
fn draw_methods() -> [(RoadDraw, crate::tool_icons::Glyph, &'static str, &'static str); 3] {
    use crate::tool_icons::Glyph as G;
    [
        (RoadDraw::Points, G::DrawPoints, "By points", "Click the points of the road; Enter fits the models along them."),
        (RoadDraw::Live, G::DrawLive, "Live", "The models follow the mouse: green where they fit, red where they cannot. A click keeps the road up to there."),
        (RoadDraw::Parts, G::DrawParts, "Piece by piece", "Lay the road one chosen piece after another, as Terrain Builder's Road Tool does."),
    ]
}

/// A new road: its type, how it is laid, and how it is fitted.
fn road_draw_fields(app: &mut App, ui: &mut egui::Ui) {
    let families = app.road_families.clone();
    row(ui, &t("Road type"), |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(egui::Button::new(icons::FOLDER_OPEN).min_size(vec2(26.0, 22.0))).on_hover_text(t("Road types from TV4P...")).clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("Terrain Builder", &["tv4p"]).pick_file() { app.load_road_types(p); }
            }
            let w = ui.available_width();
            egui::ComboBox::from_id_salt("family").selected_text(app.line.family.clone()).width(w).truncate().show_ui(ui, |ui| {
                for f in families { ui.selectable_value(&mut app.line.family, f.clone(), f); }
            });
        });
    });
    // Wrapped, so a narrow panel is not held wide by the three buttons.
    ui.horizontal_wrapped(|ui| {
        for (d, g, n, tip) in draw_methods() {
            ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
            if crate::tool_icons::tile(ui, app.line.draw == d, g, &t(n), t(tip)).clicked() { app.line.draw = d; }
        }
    });
    let about_text = match app.line.draw {
        RoadDraw::Points => "Click the points of the road; Enter fits the models along them.",
        RoadDraw::Live => "The models follow the mouse: green where they fit, red where they cannot. A click keeps the road up to there.",
        RoadDraw::Parts => "Lay the road one chosen piece after another, as Terrain Builder's Road Tool does.",
    };
    about(ui, &t(about_text));
    ui.add_space(4.0);
    if app.line.draw == RoadDraw::Parts {
        parts_panel(app, ui);
        return;
    }
    drawing_status(app, ui);
    hint(ui, &t("A road begun or ended by a free arm of a crossroad starts from that arm."));
    let rail = dayz_road_tool::rails::is_track_family(&app.line.family);
    match app.line.draw {
        RoadDraw::Live => {
            theme::check_row(ui, &mut app.line.live_keep, t("Keep the pieces already laid")).on_hover_text(t("On: the pieces up to the points already clicked stay as they are, and only the stretch to the new point is fitted. Off: every new point fits the whole road again, so earlier pieces may shift."));
        }
        _ => {
            theme::check_row(ui, &mut app.line.route, t("Route across terrain")).on_hover_text(t("Finds the road through the clicked points within the grade limits, around water and forbidden areas, with bridges. Off: road parts follow the clicked line."));
            if app.line.route {
                let shapes: Vec<String> = app.doc.as_ref().map(|d| d.world.shape_layers.iter().map(|l| l.name.clone()).collect()).unwrap_or_default();
                if app.line.forbidden.is_some_and(|i| i >= shapes.len()) { app.line.forbidden = None; }
                row(ui, &t("Forbidden areas"), |ui| {
                    let shown = app.line.forbidden.map(|i| shapes[i].clone()).unwrap_or_else(|| t("(none)"));
                    let w = ui.available_width();
                    egui::ComboBox::from_id_salt("forbidden").width(w).selected_text(shown).show_ui(ui, |ui| {
                        ui.selectable_value(&mut app.line.forbidden, None, t("(none)"));
                        for (i, n) in shapes.iter().enumerate() { ui.selectable_value(&mut app.line.forbidden, Some(i), n); }
                    });
                });
            }
        }
    }
    let searched = app.line.draw == RoadDraw::Points && app.line.route;
    let r = if rail { &mut app.line.rail_routing } else { &mut app.line.routing };
    let thumbs = app.thumbs.handle();
    theme::subsection(ui, &t(if searched { "Route settings" } else { "Fit settings" }), false, |ui| { routing_fields(ui, r, &thumbs); });
}

/// The outliner as a tab of its own.
pub(crate) fn outliner_tab(app: &mut App, ui: &mut egui::Ui) {
    outliner(app, ui);
}

/// The details as a tab of its own.
pub(crate) fn details_tab(app: &mut App, ui: &mut egui::Ui) {
    details(app, ui);
}

/// The content browser as a tab of its own.
pub(crate) fn content_tab(app: &mut App, ui: &mut egui::Ui) {
    crate::asset_browser::ui(app, ui);
}

pub(crate) fn right_panel(app: &mut App, ui: &mut egui::Ui) {
    let h = ui.available_height();
    egui::TopBottomPanel::top("outliner").resizable(true).default_height(h * 0.42).height_range(120.0..=h - 160.0).frame(egui::Frame::new()).show_inside(ui, |ui| outliner(app, ui));
    egui::CentralPanel::default().frame(egui::Frame::new()).show_inside(ui, |ui| details(app, ui));
}

/// A section of the outliner; the fold button in its title opens or closes all of them.
/// An empty one is folded and dimmed; it keeps its own open state, so it opens again by itself
/// once something is in it.
pub(crate) fn outliner_section(fold: Option<bool>, salt: &str, title: String, empty: bool) -> egui::CollapsingHeader {
    let text = RichText::new(title).family(egui::FontFamily::Name(icons::STRONG.into()));
    let text = if empty { text.color(theme::TEXT_DIM) } else { text.color(theme::TEXT_BRIGHT) };
    egui::CollapsingHeader::new(text).id_salt(("outliner", salt, empty)).default_open(!empty).open(fold)
}

fn outliner(app: &mut App, ui: &mut egui::Ui) {
    let fold = app.outliner_fold.take();
    // The search and the button that folds or opens every section, over the list.
    ui.horizontal(|ui| {
        let (icon, tip) = if app.outliner_unfolded { (icons::ARROWS_IN_LINE_VERTICAL, t("Collapse all")) } else { (icons::ARROWS_OUT_LINE_VERTICAL, t("Expand all")) };
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // One height for the search, the team filter and the button, so their edges meet.
            const H: f32 = 24.0;
            ui.spacing_mut().interact_size.y = H;
            if ui.add_sized([H, H], egui::Button::new(RichText::new(icon).size(14.0))).on_hover_text(tip).clicked() {
                app.outliner_unfolded = !app.outliner_unfolded;
                app.outliner_fold = Some(app.outliner_unfolded);
            }
            crate::team_ui::filter_ui(app, ui);
            ui.add(egui::TextEdit::singleline(&mut app.outliner_filter).hint_text(format!("{} {}", icons::MAGNIFYING_GLASS, t("Search..."))).desired_width(f32::INFINITY).min_size(vec2(0.0, H)).vertical_align(egui::Align::Center));
        });
    });
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
    // In team work: whose things are shown, and who made each.
    let team = crate::team_ui::active(app);
    let by = |who: &str| if team && !who.is_empty() { format!("  · {who}") } else { String::new() };
    let mods: Vec<(u64, String, bool)> = doc.world.modifiers.iter().filter(|m| crate::team_ui::shown(app, doc.world.author(m.id))).map(|m| (m.id, format!("{}{}", m.name, by(doc.world.author(m.id))), m.enabled)).collect();
    let foreign: Vec<bool> = (0..doc.world.object_layers.len()).map(|i| doc.world.layer_foreign(i as u16)).collect();
    let unlocked: Vec<bool> = doc.world.object_layers.iter().map(|l| doc.world.team_unlocked.contains(&l.name)).collect();
    let layer_open: Vec<bool> = (0..doc.world.object_layers.len()).map(|i| doc.world.layer_interactive(i as u16)).collect();
    let paths: Vec<String> = hist.iter().map(|(m, _)| doc.world.objects.models[*m as usize].path.clone()).collect();
    let shape_rows: Vec<(usize, String, bool, usize)> = doc.world.shape_layers.iter().enumerate().map(|(i, l)| (i, l.name.clone(), l.visible, l.shapes.len())).collect();
    let active_shape = doc.world.active_shape_layer;
    let total_objects = doc.world.objects.len();
    // Models of the selected objects, lit in the list; a new selection scrolls to them.
    let selected_models: std::collections::HashSet<u32> = app.selection.iter().filter_map(|id| doc.world.objects.get(*id).map(|o| o.model)).collect();
    let scroll = app.outliner_rev != app.selection_rev;
    let mut reveal: Option<String> = None;
    let generators: Vec<(u64, String, &'static str, usize)> = doc.world.generators.iter().filter(|g| crate::team_ui::shown(app, doc.world.author(g.id))).map(|g| (g.id, format!("{}{}", g.name, by(doc.world.author(g.id))), g.kind.type_name(), g.objects.len())).collect();
    let open_shapes: Vec<(usize, Vec<(usize, dz_core::shapes::ShapeKind, usize)>)> = app.shape_open.iter().filter_map(|l| Some((*l, doc.world.shape_layers.get(*l)?.shapes.iter().enumerate().map(|(i, s)| (i, s.kind, s.points.len())).collect()))).collect();
    let mut shape_pick: Option<(usize, usize, bool)> = None;
    let mut shape_move: Option<(usize, usize, usize)> = None;
    let mut toggle_open: Option<usize> = None;
    let route_ids: Vec<u64> = doc.world.routes.iter().map(|r| r.id).collect();
    let routes: Vec<(usize, String, String)> = doc.world.routes.iter().enumerate().filter(|(_, r)| crate::team_ui::shown(app, doc.world.author(r.id))).map(|(i, r)| (i, r.name.clone(), format!("{}{}", r.family, by(doc.world.author(r.id))))).collect();
    let mut select_route = None;
    let mut delete_route = None;
    let mut shape_action = None;
    let mut start_shape_rename = None;
    let mut select_model = None;
    let mut start_rename = None;
    let mut select_modifier = None;
    let mut toggle_modifier: Option<(u64, bool)> = None;
    let mut move_modifier: Option<(u64, usize)> = None;
    let mut rename_modifier: Option<(u64, String)> = None;
    let mut rename_route: Option<(usize, String)> = None;
    let mut start_outliner_rename: Option<(crate::app::OutlinerRenameTarget, String)> = None;
    let mut cancel_outliner_rename = false;
    let mut move_shape_layer: Option<(usize, usize)> = None;
    let mut fly: Option<dz_core::goto::Spot> = None;
    let mut select_keypoint = None;
    let keypoints: Vec<(usize, String)> = doc.world.keypoints.iter().enumerate().map(|(i, k)| (i, k.name.clone())).collect();
    let total_mods = doc.world.modifiers.len();
    let total_shapes = doc.world.shape_layers.len();
    egui::ScrollArea::vertical().id_salt("outl").auto_shrink([false, false]).show(ui, |ui| {
        // Rows start close to their section's arrow, so names get the width of the panel.
        ui.spacing_mut().indent = 8.0;
        outliner_section(fold, "Terrain modifiers", icons::with(icons::MOUNTAINS, &tf("Terrain modifiers ({0})", &[&mods.len().to_string()])), mods.len() == 0).show(ui, |ui| {
            // Top of the list is the top of the stack (applied last, on top of modifiers below).
            for (id, name, enabled) in mods.iter().rev().filter(|(_, n, _)| n.to_ascii_lowercase().contains(&filter)) {
                let vec_idx = app.doc.as_ref().and_then(|d| d.world.modifiers.iter().position(|m| m.id == *id)).unwrap_or(0);
                let disp_idx = total_mods.saturating_sub(1).saturating_sub(vec_idx);
                let row_resp = ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if filter.is_empty() && total_mods > 1 {
                        theme::drag_grip(ui, theme::OutlinerDrag::TerrainMod(*id), &t("Drag to reorder"));
                    }
                    if theme::row_switch(ui, *enabled, icons::EYE, icons::EYE_SLASH, t("Visible")).clicked() {
                        toggle_modifier = Some((*id, !*enabled));
                    }
                    if let Some((_, text)) = app.outliner_rename.as_mut().filter(|(t, _)| *t == crate::app::OutlinerRenameTarget::TerrainMod(*id)) {
                        let edit = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
                        edit.request_focus();
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            cancel_outliner_rename = true;
                        } else if edit.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            rename_modifier = Some((*id, text.clone()));
                        }
                        return;
                    }
                    let label = if *enabled { RichText::new(name.as_str()) } else { RichText::new(name.as_str()).weak() };
                    let row = theme::list_row(ui, app.selected_modifier == Some(*id), icons::MOUNTAINS, if *enabled { theme::KIND_TERRAIN } else { theme::TEXT_DIM }, label, "");
                    if row.clicked() { select_modifier = Some(*id); }
                    row.context_menu(|ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        let spot = app.doc.as_ref().and_then(|d| d.world.modifiers.iter().find(|m| m.id == *id)).map(dz_core::goto::modifier_spot);
                        if let Some(s) = crate::goto::menu(ui, spot) { fly = Some(s); }
                        if ui.button(icons::with(icons::PENCIL_SIMPLE, &t("Rename"))).clicked() {
                            let raw = app.doc.as_ref().and_then(|d| d.world.modifiers.iter().find(|m| m.id == *id)).map(|m| m.name.clone()).unwrap_or_default();
                            start_outliner_rename = Some((crate::app::OutlinerRenameTarget::TerrainMod(*id), raw));
                            ui.close_menu();
                        }
                        if total_mods > 1 {
                            if ui.add_enabled(vec_idx + 1 < total_mods, egui::Button::new(icons::with(icons::ARROW_UP, &t("Move up")))).clicked() {
                                move_modifier = Some((*id, vec_idx + 1));
                                ui.close_menu();
                            }
                            if ui.add_enabled(vec_idx > 0, egui::Button::new(icons::with(icons::ARROW_DOWN, &t("Move down")))).clicked() {
                                move_modifier = Some((*id, vec_idx - 1));
                                ui.close_menu();
                            }
                        }
                        if team { crate::team_ui::author_menu(app, ui, *id); }
                    });
                }).response;
                if filter.is_empty() && total_mods > 1 {
                    if let Some((theme::OutlinerDrag::TerrainMod(drag_id), after)) = theme::dnd_reorder_slot(ui, row_resp.rect, |p| matches!(p, theme::OutlinerDrag::TerrainMod(_))) {
                        if drag_id != *id {
                            if let Some(from_vec) = app.doc.as_ref().and_then(|d| d.world.modifiers.iter().position(|m| m.id == drag_id)) {
                                let from_disp = total_mods.saturating_sub(1).saturating_sub(from_vec);
                                let to_vec = theme::reorder_target(from_disp, disp_idx, after, total_mods, true);
                                move_modifier = Some((drag_id, to_vec));
                            }
                        }
                    }
                }
            }
        });
        crate::maskmods::outliner(app, ui, &filter, fold);
        crate::satellite::outliner(app, ui, &filter, fold);
        outliner_section(fold, "Generators", icons::with(icons::MAGIC_WAND, &tf("Generators ({0})", &[&generators.len().to_string()])), generators.len() == 0).show(ui, |ui| {
            crate::library_ui::outliner_rows(app, ui, &generators, &filter);
        });
        outliner_section(fold, "Routes", icons::with(icons::ROAD_HORIZON, &tf("Routes ({0})", &[&routes.len().to_string()])), routes.len() == 0).show(ui, |ui| {
            for (i, name, family) in routes.iter().filter(|(_, n, _)| n.to_ascii_lowercase().contains(&filter)) {
                if let Some((_, text)) = app.outliner_rename.as_mut().filter(|(t, _)| *t == crate::app::OutlinerRenameTarget::Route(*i)) {
                    let edit = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
                    edit.request_focus();
                    if ui.input(|k| k.key_pressed(egui::Key::Escape)) {
                        cancel_outliner_rename = true;
                    } else if edit.lost_focus() || ui.input(|k| k.key_pressed(egui::Key::Enter)) {
                        rename_route = Some((*i, text.clone()));
                    }
                    continue;
                }
                let row = theme::list_row(ui, app.line.selected_route == Some(*i), icons::ROAD_HORIZON, theme::KIND_ROAD, name.as_str(), family);
                if row.clicked() { select_route = Some(*i); }
                row.context_menu(|ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    let spot = app.doc.as_ref().and_then(|d| d.world.routes.get(*i)).and_then(dz_core::goto::route_spot);
                    if let Some(s) = crate::goto::menu(ui, spot) { fly = Some(s); }
                    if ui.button(icons::with(icons::PENCIL_SIMPLE, &t("Rename"))).clicked() {
                        start_outliner_rename = Some((crate::app::OutlinerRenameTarget::Route(*i), name.clone()));
                        ui.close_menu();
                    }
                    if ui.button(t("Delete route")).clicked() { delete_route = Some(*i); ui.close_menu(); }
                    crate::team_ui::author_menu(app, ui, route_ids[*i]);
                });
            }
        });
        outliner_section(fold, "Key points", icons::with(icons::MAP_PIN, &tf("Key points ({0})", &[&keypoints.len().to_string()])), keypoints.is_empty()).show(ui, |ui| {
            for (i, name) in keypoints.iter().filter(|(_, n)| n.to_ascii_lowercase().contains(&filter)) {
                let row = theme::list_row(ui, app.shape.keypoint == Some(*i), icons::MAP_PIN, theme::KIND_POINT, name.as_str(), "");
                if row.clicked() { select_keypoint = Some(*i); }
                row.context_menu(|ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    let spot = app.doc.as_ref().and_then(|d| d.world.keypoints.get(*i)).map(dz_core::goto::keypoint_spot);
                    if let Some(s) = crate::goto::menu(ui, spot) { fly = Some(s); }
                });
            }
        });
        outliner_section(fold, "Shape layers", icons::with(icons::POLYGON, &tf("Shape layers ({0})", &[&shape_rows.len().to_string()])), false).show(ui, |ui| {
            if ui.small_button(format!("+ {}", t("New layer"))).clicked() { shape_action = Some(ShapeAction::New); }
            for (i, name, visible, count) in shape_rows.iter().filter(|(_, n, _, _)| n.to_ascii_lowercase().contains(&filter)) {
                let row_resp = ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if filter.is_empty() && total_shapes > 1 {
                        theme::drag_grip(ui, theme::OutlinerDrag::ShapeLayer(*i), &t("Drag to reorder"));
                    }
                    let opened = open_shapes.iter().any(|(l, _)| l == i);
                    if theme::row_switch(ui, true, if opened { icons::CARET_DOWN } else { icons::CARET_RIGHT }, "", String::new()).clicked() { toggle_open = Some(*i); }
                    if theme::row_switch(ui, *visible, icons::EYE, icons::EYE_SLASH, t("Visible")).clicked() { shape_action = Some(ShapeAction::Visible(*i)); }
                    if let Some((_, text)) = app.shape_rename.as_mut().filter(|(r, _)| r == i) {
                        let edit = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
                        edit.request_focus();
                        edit.scroll_to_me(Some(egui::Align::Center));
                        if edit.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) { shape_action = Some(ShapeAction::Rename(*i, text.clone())); }
                        return;
                    }
                    let label = RichText::new(name);
                    let row = theme::list_row(ui, active_shape == *i, icons::POLYGON, theme::KIND_SHAPE, if *visible { label } else { label.weak() }, &count.to_string());
                    if row.clicked() { shape_action = Some(ShapeAction::Active(*i)); }
                    if let Some(p) = row.dnd_release_payload::<(usize, usize)>() { shape_move = Some((p.0, p.1, *i)); }
                    row.context_menu(|ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        let spot = app.doc.as_ref().and_then(|d| d.world.shape_layers.get(*i)).and_then(dz_core::goto::layer_spot);
                        if let Some(s) = crate::goto::menu(ui, spot) { fly = Some(s); }
                        if total_shapes > 1 {
                            if ui.add_enabled(*i > 0, egui::Button::new(icons::with(icons::ARROW_UP, &t("Move up")))).clicked() { move_shape_layer = Some((*i, *i - 1)); ui.close_menu(); }
                            if ui.add_enabled(*i + 1 < total_shapes, egui::Button::new(icons::with(icons::ARROW_DOWN, &t("Move down")))).clicked() { move_shape_layer = Some((*i, *i + 1)); ui.close_menu(); }
                        }
                        if ui.button(t("Rename")).clicked() { start_shape_rename = Some((*i, name.clone())); ui.close_menu(); }
                        if ui.button(t("Delete layer")).clicked() { shape_action = Some(ShapeAction::Delete(*i)); ui.close_menu(); }
                    });
                }).response;
                if filter.is_empty() && total_shapes > 1 {
                    if let Some((theme::OutlinerDrag::ShapeLayer(from), after)) = theme::dnd_reorder_slot(ui, row_resp.rect, |p| matches!(p, theme::OutlinerDrag::ShapeLayer(_))) {
                        if from != *i {
                            let to = theme::reorder_target(from, *i, after, total_shapes, false);
                            move_shape_layer = Some((from, to));
                        }
                    }
                }
                if let Some((_, list)) = open_shapes.iter().find(|(l, _)| l == i) {
                    ui.indent(("shapes", *i), |ui| {
                        egui::ScrollArea::vertical().id_salt(("shape_list", *i)).max_height(180.0).auto_shrink([false, true]).show(ui, |ui| {
                            for (s, kind, n) in list {
                                let kind = match kind { dz_core::shapes::ShapeKind::Point => t("Point"), dz_core::shapes::ShapeKind::Polyline => t("Polyline"), dz_core::shapes::ShapeKind::Polygon => t("Polygon") };
                                let selected = app.shape.selection.contains(&(*i, *s));
                                let r = theme::list_row(ui, selected, "", theme::KIND_SHAPE, format!("{}  {kind}", s + 1), &n.to_string()).interact(egui::Sense::click_and_drag());
                                if r.clicked() { shape_pick = Some((*i, *s, ui.input(|i| i.modifiers.shift || i.modifiers.command))); }
                                r.dnd_set_drag_payload((*i, *s));
                                r.context_menu(|ui| {
                                    let spot = app.doc.as_ref().and_then(|d| d.world.shape_layers.get(*i)?.shapes.get(*s)).and_then(dz_core::goto::shape_spot);
                                    if let Some(sp) = crate::goto::menu(ui, spot) { fly = Some(sp); }
                                });
                            }
                        });
                    });
                }
            }
        });
        let live = layers.iter().filter(|l| !l.deleted).count();
        outliner_section(fold, "Object layers", icons::with(icons::STACK, &tf("Object layers ({0})", &[&live.to_string()])), false).show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.small_button(format!("+ {}", t("New layer"))).clicked() { layer_action = Some(LayerAction::New); }
                if ui.small_button(t("Edit all")).clicked() { layer_action = Some(LayerAction::EditAll); }
                if ui.small_button(t("Show all")).clicked() { layer_action = Some(LayerAction::ShowAll); }
            });
            for (i, l) in layers.iter().enumerate().filter(|(_, l)| !l.deleted) {
                if !filter.is_empty() && !l.name.to_ascii_lowercase().contains(&filter) { continue; }
                if team && !crate::team_ui::shown(app, &l.owner) { continue; }
                let id = i as u16;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if theme::row_switch(ui, l.visible, icons::EYE, icons::EYE_SLASH, t("Visible")).clicked() { layer_action = Some(LayerAction::Visible(id)); }
                    if theme::row_switch(ui, l.editable, icons::PENCIL_SIMPLE, icons::PENCIL_SIMPLE_SLASH, t("Editable")).clicked() { layer_action = Some(LayerAction::Editable(id)); }
                    // A layer of someone else in team work: locked until unlocked here.
                    if foreign[i] || unlocked[i] {
                        let (icon, tip) = if foreign[i] {
                            (icons::LOCK_SIMPLE, tf("The layer of {0}. Click to work on it anyway.", &[&l.owner]))
                        } else {
                            (icons::LOCK_SIMPLE_OPEN, tf("The layer of {0}, unlocked. Click to leave it again.", &[&l.owner]))
                        };
                        if ui.add(egui::Button::new(icon).small().selected(foreign[i])).on_hover_text(tip).clicked() { layer_action = Some(LayerAction::Unlock(id)); }
                    }
                    if let Some((_, name)) = app.layer_rename.as_mut().filter(|(r, _)| *r == id) {
                        let edit = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
                        edit.request_focus();
                        edit.scroll_to_me(Some(egui::Align::Center));
                        if edit.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            layer_action = Some(LayerAction::Rename(id, name.clone()));
                        }
                        return;
                    }
                    let mut text = RichText::new(format!("{}{}", l.name, by(&l.owner)));
                    if !layer_open[i] { text = text.weak(); }
                    if !l.export { text = text.italics(); }
                    let row = theme::list_row(ui, active == id, icons::STACK, theme::KIND_LAYER, text, &counts.get(i).copied().unwrap_or(0).to_string()).on_hover_text(t("Click: active layer (new objects go here). Ctrl+click: turn editing on or off."));
                    if row.clicked() {
                        layer_action = Some(if ui.input(|i| i.modifiers.command) { LayerAction::Editable(id) } else { LayerAction::Active(id) });
                    }
                    row.context_menu(|ui| { ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend); 
                        if ui.button(t("Edit only this layer")).clicked() { layer_action = Some(LayerAction::Solo(id)); ui.close_menu(); }
                        if ui.button(t("Select objects")).clicked() { layer_action = Some(LayerAction::Select(id)); ui.close_menu(); }
                        if ui.add_enabled(has_selection, egui::Button::new(t("Move selection here"))).clicked() { layer_action = Some(LayerAction::MoveSelection(id)); ui.close_menu(); }
                        let mut export = l.export;
                        if ui.checkbox(&mut export, t("Include in export")).clicked() { layer_action = Some(LayerAction::Export(id)); ui.close_menu(); }
                        let mut satellite = l.satellite;
                        if ui.checkbox(&mut satellite, t("Bake into the satellite")).on_hover_text(t("Turn it off for a layer of wrecks, rubbish or fences, so baking objects into the satellite image leaves them out.")).clicked() { layer_action = Some(LayerAction::Satellite(id)); ui.close_menu(); }
                        if team {
                            ui.separator();
                            ui.menu_button(icons::with(icons::USER, &t("Owner")), |ui| {
                                if let Some(p) = crate::team_ui::person_menu(app, ui, &l.owner) { layer_action = Some(LayerAction::Owner(id, p)); }
                            });
                        }
                        ui.separator();
                        if ui.button(t("Rename")).clicked() { start_rename = Some((id, l.name.clone())); ui.close_menu(); }
                        if ui.add_enabled(id != 0, egui::Button::new(t("Delete layer, keep objects"))).clicked() { layer_action = Some(LayerAction::Delete(id)); ui.close_menu(); }
                        if ui.add_enabled(id != 0, egui::Button::new(t("Delete layer and objects"))).clicked() { layer_action = Some(LayerAction::DeleteWithObjects(id)); ui.close_menu(); }
                    });
                });
            }
        });
        outliner_section(fold, "Objects", icons::with(icons::CUBE, &tf("Objects ({0})", &[&total_objects.to_string()])), total_objects == 0).show(ui, |ui| {
            let mut scrolled = false;
            for (k, (m, count)) in hist.iter().enumerate() {
                let name = paths[k].rsplit('\\').next().unwrap_or(&paths[k]);
                if !filter.is_empty() && !name.contains(&filter) { continue; }
                let lit = selected_models.contains(m);
                let row = theme::list_row(ui, lit, icons::CUBE, theme::KIND_MODEL, name, &format!("×{count}")).on_hover_text(&paths[k]);
                if lit && scroll && !scrolled {
                    row.scroll_to_me(Some(egui::Align::Center));
                    scrolled = true;
                }
                if row.clicked() { select_model = Some(*m); }
                let path = paths[k].clone();
                row.context_menu(|ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    // The nearest object of the model to the camera.
                    let spot = app.doc.as_ref().and_then(|d| {
                        let c = app.camera.pos;
                        d.world.objects.iter().filter(|(_, o)| o.model == *m).min_by(|(_, a), (_, b)| {
                            let dist = |o: &dz_core::objects::Obj| (o.x - c[0]).powi(2) + (o.z - c[2]).powi(2);
                            dist(a).total_cmp(&dist(b))
                        }).map(|(_, o)| dz_core::goto::Spot { x: o.x, z: o.z, radius: 20.0 })
                    });
                    if let Some(s) = crate::goto::menu(ui, spot) { fly = Some(s); }
                    let skipped = app.doc.as_ref().is_some_and(|d| d.world.meta.bake_skip.iter().any(|s| s.eq_ignore_ascii_case(&path)));
                    if ui.button(if skipped { t("Bake it into the satellite again") } else { t("Leave out of the satellite bake") }).on_hover_text(t("Baking objects into the satellite image skips this model.")).clicked() { crate::satellite::set_bake_skip(app, std::slice::from_ref(&path), !skipped); ui.close_menu(); }
                    if ui.button(icons::with(icons::FOLDER_OPEN, &t("Show in asset browser"))).clicked() { reveal = Some(path.clone()); ui.close_menu(); }
                    if ui.button(t("Select objects")).clicked() { select_model = Some(*m); ui.close_menu(); }
                });
            }
        });
    });
    app.outliner_rev = app.selection_rev;
    if let Some(p) = reveal { app.show_in_browser(&p); }
    if let Some(s) = fly { crate::goto::fly_to(app, s); }
    if let Some(i) = select_keypoint {
        app.shape.keypoint = Some(i);
        app.mode = Mode::KeyPoints;
    }
    if select_model.is_some() || shape_pick.is_some() || select_route.is_some() || select_modifier.is_some() {
        app.leave_browser();
    }
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
    if let Some((id, on)) = toggle_modifier {
        if let Some(d) = &mut app.doc {
            let before = d.world.modifiers.clone();
            if let Some(at) = d.world.modifiers.iter().position(|m| m.id == id) {
                d.world.modifiers[at].enabled = on;
                let area = d.world.modifier_rect(&d.world.modifiers[at].clone());
                if let Some(r) = area {
                    d.world.recompose(Some(r));
                    d.history.push(t("Terrain modifier"), dz_core::edit::Change::Modifiers(before));
                    app.height_changed(r);
                } else {
                    d.history.push(t("Terrain modifier"), dz_core::edit::Change::Modifiers(before));
                }
            }
        }
    }
    if let Some((id, to_idx)) = move_modifier {
        if let Some(d) = &mut app.doc {
            let before = d.world.modifiers.clone();
            let area = d.world.move_modifier(id, to_idx);
            if d.world.modifiers != before {
                d.history.push(t("Reorder terrain modifiers"), dz_core::edit::Change::Modifiers(before));
                if let Some(r) = area {
                    app.height_changed(r);
                }
            }
        }
    }
    if let Some((from, to)) = move_shape_layer {
        if let Some(d) = &mut app.doc {
            let before = d.world.shape_layers.clone();
            if d.world.move_shape_layer(from, to) {
                d.history.push(t("Reorder shape layers"), dz_core::edit::Change::Shapes(before));
                app.shape.selection = app.shape.selection.iter().map(|&(l, s)| (dz_core::world::remap_moved_index(l, from, to), s)).collect();
                app.shape_open = app.shape_open.iter().map(|&l| dz_core::world::remap_moved_index(l, from, to)).collect();
            }
        }
    }
    if cancel_outliner_rename {
        app.outliner_rename = None;
    }
    if let Some((id, new_name)) = rename_modifier {
        app.outliner_rename = None;
        let trimmed = new_name.trim();
        if !trimmed.is_empty() {
            if let Some(d) = &mut app.doc {
                let before = d.world.modifiers.clone();
                if let Some(m) = d.world.modifiers.iter_mut().find(|m| m.id == id) {
                    if m.name != trimmed {
                        m.name = trimmed.to_string();
                        d.history.push(t("Rename"), dz_core::edit::Change::Modifiers(before));
                    }
                }
            }
        }
    }
    if let Some((idx, new_name)) = rename_route {
        app.outliner_rename = None;
        let trimmed = new_name.trim();
        if !trimmed.is_empty() {
            if let Some(d) = &mut app.doc {
                let before = d.world.routes.clone();
                if let Some(r) = d.world.routes.get_mut(idx) {
                    if r.name != trimmed {
                        r.name = trimmed.to_string();
                        d.history.push(t("Rename"), dz_core::edit::Change::Routes(before));
                    }
                }
            }
        }
    }
    if start_outliner_rename.is_some() {
        app.outliner_rename = start_outliner_rename;
    }
}

fn details(app: &mut App, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().id_salt("det").auto_shrink([false, false]).show(ui, |ui| {
        if let Some(id) = app.selected_modifier { modifier_details(app, ui, id); return; }
        if crate::library_ui::details(app, ui) { return; }
        if app.mode != Mode::Shapes && app.selection.is_empty() && !app.shape.selection.is_empty() { shape_details(app, ui); return; }
        if app.selection.len() == 1 {
            object_details(app, ui);
            placement_section(app, ui, false);
            replace_section(app, ui);
        } else if app.selection.len() > 1 {
            theme::details_head(ui, icons::SELECTION, theme::KIND_MODEL, &selected_text(app.selection.len()), "");
            placement_section(app, ui, true);
            replace_section(app, ui);
        } else {
            map_summary(app, ui);
        }
    });
}

/// The flags of several selected objects, each shown as set, cleared or mixed; changing one
/// sets it on all of them.
fn selection_flags(app: &mut App, ui: &mut egui::Ui) {
    let Some(doc) = &app.doc else { return };
    let objects: Vec<u32> = app.selection.iter().filter_map(|id| doc.world.objects.get(*id)).map(|o| o.flags).collect();
    if objects.is_empty() {
        return;
    }
    let mut set = None;
    let flags = [
        (FLAG_UNDERGROUND, t("Underground"), String::new()),
        (FLAG_ABSOLUTE, t("Fixed height, ignores the terrain"), t("The elevation is then measured from sea level")),
    ];
    for (flag, label, hint) in flags {
        let on = objects.iter().filter(|f| *f & flag != 0).count();
        let (all, mixed) = (on == objects.len(), on > 0 && on < objects.len());
        let mut shown = all;
        let r = ui.add(egui::Checkbox::new(&mut shown, &label).indeterminate(mixed));
        let r = if hint.is_empty() { r } else { r.on_hover_text(hint) };
        if r.clicked() {
            // A mixed selection turns the flag on for all.
            set = Some((flag, !all, label));
        }
    }
    if let Some((flag, on, label)) = set {
        app.set_selection_flag(flag, on, &label);
    }
}

/// What the Details show with nothing selected: the map in a few lines, and where to click.
fn map_summary(app: &App, ui: &mut egui::Ui) {
    about(ui, &t("Click an object, a route, a generator or a modifier to see and change it here."));
    let Some(doc) = &app.doc else { return };
    let w = &doc.world;
    ui.add_space(6.0);
    ui.label(RichText::new(&w.meta.name).strong().color(Color32::WHITE));
    let km = w.meta.size_m() / 1000.0;
    egui::Grid::new("map_summary").num_columns(2).spacing(vec2(12.0, 3.0)).show(ui, |ui| {
        let mut line = |label: String, value: String| { ui.label(RichText::new(label).color(theme::TEXT_DIM)); ui.label(value); ui.end_row(); };
        line(t("Size"), format!("{km:.2} × {km:.2} km"));
        line(t("Objects"), w.objects.len().to_string());
        line(t("Routes"), w.routes.len().to_string());
        line(t("Generators"), w.generators.len().to_string());
        line(t("Surfaces"), w.layers.len().to_string());
        line(t("Terrain modifiers"), w.modifiers.len().to_string());
    });
}

/// Putting another model in place of the selected objects: dropped here from the Content
/// Browser, or the one chosen there. Where they stand, how they turn and their size stay.
fn replace_section(app: &mut App, ui: &mut egui::Ui) {
    // A whole road or generator is changed through its own settings, not model by model.
    if app.whole.is_some() {
        return;
    }
    let n = app.selection.len();
    section(ui, &icons::with(icons::SWAP, &t("Replace model")), false, |ui| {
        about(ui, &t("Puts another model in place of the selected objects; where they stand, how they are turned and their size stay."));
        let (_, dropped) = theme::model_drop_box(ui, vec2(ui.available_width(), 44.0), &tf("Drop a model here to replace {0} objects", &[&n.to_string()]), &t("Drop to replace"));
        let dropped: Vec<String> = dropped.iter().map(|m| (**m).clone()).collect();
        // The model or models chosen in the Content Browser.
        let mut chosen: Vec<String> = app.browser.selected.iter().filter_map(|p| if let crate::asset_browser::Pick::Model(m) = p { Some(m.clone()) } else { None }).collect();
        if chosen.is_empty() { chosen.extend(app.place.model.clone()); }
        let label = match chosen.len() {
            0 => t("Choose a model in the Content Browser"),
            1 => tf("Use {0}", &[chosen[0].rsplit('\\').next().unwrap_or(&chosen[0]).trim_end_matches(".p3d")]),
            k => tf("Use the {0} models chosen, at random", &[&k.to_string()]),
        };
        let use_chosen = ui.add_enabled(!chosen.is_empty(), egui::Button::new(icons::with(icons::SWAP, &label))).on_hover_text(t("The models selected in the Content Browser; with several, each object takes one of them.")).clicked();
        ui.checkbox(&mut app.replace_keep_ground, t("Rest on the ground as the old model did")).on_hover_text(t("A model whose bottom lies higher or lower than the old one's is raised or lowered so that it stands on the ground the same way. Objects at a fixed height keep it."));
        if !dropped.is_empty() {
            app.replace_selection(&dropped);
        } else if use_chosen {
            app.replace_selection(&chosen);
        }
    });
}

/// Tilting the selected objects with the slope under them, now or as they are moved.
/// "1 object selected" or "5 objects selected".
pub(crate) fn selected_text(n: usize) -> String {
    if n == 1 { t("1 object selected") } else { tf("{0} objects selected", &[&n.to_string()]) }
}

/// Where the selected objects belong and how they sit on the ground: their layer, the flags
/// of several (one object shows its own in Transform) and tilting them to the ground.
fn placement_section(app: &mut App, ui: &mut egui::Ui, flags: bool) {
    section(ui, &icons::with(icons::STACK, &t("Placement")), true, |ui| {
        selection_layer(app, ui);
        if flags { selection_flags(app, ui); }
        ground_fit(app, ui);
    });
}

fn ground_fit(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        if ui.button(t("Align to the ground")).on_hover_text(t("Tilts the selected objects to the ground under them now.")).clicked() { app.align_selection_to_ground(); }
        if ui.checkbox(&mut app.settings.align_to_ground, t("Stick to the ground")).on_hover_text(t("Moved and turned objects tilt with the ground under them.")).changed() { app.settings.save(); }
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
    let mut free_variant = false;
    let file = path.rsplit('\\').next().unwrap_or(&path).to_string();
    let folder = path[..path.len() - file.len()].trim_end_matches('\\').to_string();
    theme::details_head(ui, icons::CUBE, theme::KIND_MODEL, &file, &folder);
    section(ui, &icons::with(icons::ARROWS_OUT_CARDINAL, &t("Transform")), true, |ui| {
        let [x, y, z] = theme::AXIS;
        done |= theme::vector_row(ui, &t("Location"), &mut [(&mut o.x, 0.1, x, "X".into()), (&mut o.elevation, 0.02, y, t("Elevation")), (&mut o.z, 0.1, z, "Z".into())]);
        done |= theme::vector_row(ui, &t("Rotation"), &mut [(&mut o.yaw, 0.5, y, t("Yaw")), (&mut o.pitch, 0.25, x, t("Pitch")), (&mut o.roll, 0.25, z, t("Roll"))]);
        done |= theme::vector_row(ui, &t("Scale"), &mut [(&mut o.scale, 0.01, theme::TEXT_DIM, String::new())]);
        ui.add_space(2.0);
        let mut under = o.flags & FLAG_UNDERGROUND != 0;
        if ui.checkbox(&mut under, t("Underground")).changed() { o.flags ^= FLAG_UNDERGROUND; done = true; }
        let mut abs = o.flags & FLAG_ABSOLUTE != 0;
        if ui.checkbox(&mut abs, t("Fixed height, ignores the terrain")).on_hover_text(t("The elevation is then measured from sea level")).changed() {
            let ground = doc.world.height.sample(o.x, o.z);
            o.elevation += if abs { ground } else { -ground };
            o.flags ^= FLAG_ABSOLUTE;
            done = true;
        }
        // The game sets the height of these from the ground under their land contact points.
        let info = app.db.get(&path);
        if dz_core::model_info::stands_on_contacts(&info) && o.flags & FLAG_ABSOLUTE == 0 {
            ui.add_space(2.0);
            ui.colored_label(theme::WARN, icons::with(icons::INFO, &t("Stands on the ground")))
                .on_hover_text(t("This model has placement = slopelandcontact: the game puts its land contact points on the ground and does not use the elevation, so it cannot be raised or sunk. Its copy without land contact (_nolc) keeps the elevation it is given."));
            if let Some(free) = app.db.free_variant(&path) {
                let name = free.rsplit('\\').next().unwrap_or(&free).trim_end_matches(".p3d").to_string();
                if ui.button(tf("Use {0}, which can be raised", &[&name])).on_hover_text(t("Puts the copy without land contact in place of the selected objects of this kind. They stay where they are shown.")).clicked() {
                    free_variant = true;
                }
            }
        }
    });
    if free_variant {
        app.use_free_variants();
        return;
    }
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
            ModShape::Stamp { size, height, .. } => crate::stamps_ui::details(ui, size, height, &mut m.yaw),
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
        // A stamp changes the ground roads are laid on: those over it are graded again.
        if old.is_form() || new.is_form() {
            let square = |m: &dz_core::world::TerrainModifier| { let r = m.reach(); [m.x - r, m.z - r, m.x + r, m.z + r] };
            let (a, b) = (square(&old), square(&new));
            app.regrade_routes_in([a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]);
        }
    }
}

pub(crate) fn bottom_panel(app: &mut App, ui: &mut egui::Ui) {
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
            ui.add(crate::theme::ValueField::new(&mut app.settings.library_tile, 48.0..=240.0).show_value(false).text(icons::MAGNIFYING_GLASS_PLUS));
        }
    });
    match app.bottom {
        BottomTab::Content => crate::asset_browser::ui(app, ui),
        BottomTab::Output => output_log(app, ui),
        BottomTab::Build => build_tab(app, ui),
    }
}

pub(crate) fn output_log(app: &mut App, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show_rows(ui, 15.0, app.log.len(), |ui, range| {
        for l in &app.log[range] {
            let c = match l.level { 2 => theme::ERROR, 1 => theme::WARN, _ => theme::TEXT };
            ui.label(RichText::new(&l.text).monospace().color(c));
        }
    });
}

/// The key the built PBOs are signed with.
fn sign_key_row(app: &mut App, ui: &mut egui::Ui) {
    ui.label(RichText::new(t("Signing")).strong());
    let key = app.settings.tools.sign_key.clone();
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        if key.as_os_str().is_empty() {
            ui.weak(t("Not signed"));
        } else {
            let ok = key.is_file();
            ui.label(RichText::new(key.display().to_string()).color(if ok { theme::TEXT } else { theme::WARN })).on_hover_text(if ok { String::new() } else { t("The file is not there") });
        }
        if ui.button(t("Choose key...")).on_hover_text(t("A private key (.biprivatekey) made with DSUtils of DayZ Tools")).clicked() {
            let start = key.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| app.settings.tools.dayz_tools.join("Bin").join("DsUtils"));
            if let Some(f) = rfd::FileDialog::new().add_filter("BI private key", &["biprivatekey"]).set_directory(start).pick_file() {
                app.settings.tools.sign_key = f;
                changed = true;
            }
        }
        if !key.as_os_str().is_empty() && ui.small_button("✖").on_hover_text(t("Do not sign")).clicked() {
            app.settings.tools.sign_key = Default::default();
            changed = true;
        }
    });
    if !key.as_os_str().is_empty() {
        let public = dz_core::build::public_key(&key);
        if public.is_file() {
            ui.weak(tf("Every PBO gets its .bisign; {0} goes into Keys of the mod for servers.", &[&public.file_name().unwrap_or_default().to_string_lossy()]));
        } else {
            ui.colored_label(theme::WARN, tf("No {0} next to the key: servers need the public key to let the mod in.", &[&public.file_name().unwrap_or_default().to_string_lossy()]));
        }
    }
    if changed {
        app.settings.save();
    }
}

/// Folders of the work drive built into the map's mod besides the map: buildings and other
/// content, each its own PBO.
fn build_folders(app: &mut App, ui: &mut egui::Ui) {
    use dz_core::build::BuildFolder;
    let root = app.settings.tools.work_drive.clone();
    let Some(doc) = &mut app.doc else { return };
    let id = doc.world.meta.map_id.clone();
    ui.label(RichText::new(t("Also built into the mod")).strong());
    let before = doc.world.meta.build_folders.clone();
    let list = &mut doc.world.meta.build_folders;
    // Folders in the map's folder not listed yet, offered switched off.
    for f in dz_core::build::mod_folders(&root, &id) {
        if !list.iter().any(|b| b.folder.eq_ignore_ascii_case(&f)) {
            list.push(BuildFolder { enabled: false, ..BuildFolder::new(&f) });
        }
    }
    let mut remove = None;
    let mut warn = None;
    for (i, b) in list.iter_mut().enumerate() {
        ui.push_id(("build_folder", i), |ui| ui.horizontal(|ui| {
            ui.checkbox(&mut b.enabled, "").on_hover_text(t("Build this folder"));
            let missing = !root.join(&b.folder).is_dir();
            let text = format!("{}.pbo  ←  {}", b.pbo_name(&id), b.folder);
            ui.add(egui::Label::new(if missing { RichText::new(text).color(theme::WARN) } else { RichText::new(text) }).truncate()).on_hover_text(if missing { t("The folder is not on the work drive") } else { b.folder.clone() });
            ui.checkbox(&mut b.binarize, t("Binarise")).on_hover_text(t("Off packs the files as they are, for models that are binarised already"));
            if ui.small_button("✖").on_hover_text(t("Remove from the list")).clicked() { remove = Some(i); }
        }));
    }
    if let Some(i) = remove { list.remove(i); }
    if ui.button(format!("+ {}", t("Add folder..."))).clicked() {
        for dir in rfd::FileDialog::new().set_title(t("Add folder...")).set_directory(&root).pick_folders().unwrap_or_default() {
            match dir.strip_prefix(&root) {
                Ok(rel) => {
                    let f = BuildFolder::new(&rel.to_string_lossy());
                    if !f.folder.is_empty() && !list.iter().any(|b| b.folder.eq_ignore_ascii_case(&f.folder)) { list.push(f); }
                }
                Err(_) => warn = Some(t("The folder has to be on the work drive")),
            }
        }
    }
    about(ui, &t("Each folder becomes a PBO of its own, with its path on the work drive as the prefix, so models keep the paths the map uses."));
    if doc.world.meta.build_folders != before {
        // Folders only offered, not chosen, do not count as a change.
        let chosen = |l: &[BuildFolder]| l.iter().filter(|b| b.enabled || !b.binarize).cloned().collect::<Vec<_>>();
        if chosen(&doc.world.meta.build_folders) != chosen(&before) || doc.world.meta.build_folders.len() < before.len() {
            doc.history.revision += 1;
        }
    }
    if let Some(w) = warn { app.warn(w); }
}

pub(crate) fn build_tab(app: &mut App, ui: &mut egui::Ui) {
    ui.columns(2, |c| {
        let ui = &mut c[0];
        egui::ScrollArea::vertical().id_salt("build_steps").auto_shrink([false, false]).show(ui, |ui| {
            let problems = app.settings.tools.check();
            if problems.is_empty() {
                ui.colored_label(theme::OK, icons::with(icons::CHECK_CIRCLE, &t("DayZ Tools, game and work drive found")));
            }
            for p in problems { ui.colored_label(theme::WARN, p); }
            ui.add_space(4.0);
            let free = app.doc.is_some() && app.task.is_none();
            // The steps in order, each with what it does; a later step does the earlier ones too.
            egui::Grid::new("build_steps_grid").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                let steps: [(&str, &str, u8); 3] = [
                    ("1. Export files", "Writes the terrain, surfaces, objects and config the PBO is made of.", 0),
                    ("2. Build PBO", "Exports, then packs the map with DayZ Tools.", 1),
                    ("3. Build and play", "Builds, then starts a server with the map, and the game if chosen below.", 2),
                ];
                for (name, what, k) in steps {
                    let text = if k == 2 { RichText::new(icons::with(icons::PLAY, &t(name))).color(Color32::WHITE) } else { RichText::new(t(name)) };
                    let button = if k == 2 { egui::Button::new(text).fill(theme::ACCENT) } else { egui::Button::new(text) };
                    if ui.add_enabled(free, button.min_size(vec2(150.0, 0.0))).clicked() {
                        match k { 0 => app.export_map(false), 1 => app.build(false), _ => app.build(true) }
                    }
                    ui.label(RichText::new(t(what)).small().color(theme::TEXT_DIM));
                    ui.end_row();
                }
            });
            ui.checkbox(&mut app.settings.check_before_build, t("Check the map before building")).on_hover_text(t("Errors, such as models or surface textures missing from the work drive, stop the build until they are fixed or you build anyway."));
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(free, egui::Button::new(icons::with(icons::LIST_CHECKS, &t("Check the map")))).clicked() { crate::map_check::run(app, None); }
                if ui.add_enabled(free, egui::Button::new(t("Generate navmesh..."))).clicked() { app.generate_navmesh(); }
                if let Some(d) = &app.doc {
                    let folder = app.settings.tools.work_drive.join(&d.world.meta.map_id);
                    if ui.button(icons::with(icons::FOLDER_OPEN, &t("Open map folder"))).clicked() { app.open_folder(&folder); }
                }
            });
            ui.separator();
            build_folders(app, ui);
            ui.separator();
            sign_key_row(app, ui);
            ui.separator();
            ui.label(RichText::new(t("Playing")).strong());
            ui.checkbox(&mut app.settings.with_client, t("Start the game client too"));
            let mut mods: Vec<String> = app.settings.extra_mods.split(';').map(str::trim).filter(|m| !m.is_empty()).map(String::from).collect();
            ui.label(tf("Additional mods ({0})", &[&mods.len().to_string()]));
            let mut remove = None;
            for (i, m) in mods.iter().enumerate() {
                ui.push_id(("mod", i), |ui| ui.horizontal(|ui| {
                    if ui.small_button("✖").clicked() { remove = Some(i); }
                    ui.add(egui::Label::new(m).truncate());
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
        });
        let ui = &mut c[1];
        ui.horizontal(|ui| {
            ui.label(RichText::new(t("Latest output")).strong());
            if ui.small_button(t("Full log")).on_hover_text(t("The Output Log tab, with errors and warnings marked.")).clicked() { app.bottom = BottomTab::Output; }
        });
        egui::ScrollArea::vertical().id_salt("b").stick_to_bottom(true).auto_shrink([false, false]).show_rows(ui, 14.0, app.log.len(), |ui, range| {
            for l in &app.log[range] {
                let c = match l.level { 2 => theme::ERROR, 1 => theme::WARN, _ => theme::TEXT };
                ui.label(RichText::new(&l.text).small().monospace().color(c));
            }
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
    path_choice(ui, &t("Surfaces of the map (CfgSurfaces)"), &mut plan.surfaces_cfg, &scan.surfaces_cfg, file("config.cpp", &["cpp", "hpp"]));
    if plan.surfaces_cfg.is_none() {
        ui.weak(t("No config of the map's own surfaces was found. A map with layers of its own textures needs one: choose it, or the layers get the surface of the game whose files they match, if any."));
    }
    path_choice(ui, &t("Roads (TV4P)"), &mut plan.tv4p, &scan.tv4p, file("Terrain Builder", &["tv4p"]));
    ui.weak(if plan.asc.is_some() { t("Terrain comes from the heightmap.") } else if plan.wrp.is_some() { t("Terrain comes from the WRP.") } else { t("Choose a heightmap or a WRP.") });
    row(ui, &t("Surfaces per tile"), |ui| { ui.add(crate::theme::ValueField::new(&mut plan.max_tile_surfaces, 1..=6)); });
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
    ui.weak(t("Objects of the layers stand on the chosen heightmap, and models that follow the ground (path panels, barriers) come in level when tilted by less than 10°. A built world (WRP) has them as the game shows them."));
    if plan.objects_from_wrp && plan.wrp.is_some() && plan.layers.iter().any(|l| l.import) {
        ui.colored_label(Color32::from_rgb(235, 170, 70), t("Objects from the WRP and from layers are both chosen; the same objects would come in twice."));
    }
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
    crate::material_ui::window(app, ctx);
    let mut dialog = std::mem::replace(&mut app.dialog, Dialog::None);
    let mut open = true;
    let mut close = false;
    match &mut dialog {
        Dialog::None => {}
        Dialog::NewProject(p) => {
            egui::Window::new(t("New project")).collapsible(false).resizable(false).open(&mut open).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                theme::segmented(ui, &mut p.real, &[
                    (false, crate::icons::with(crate::icons::SQUARE, &t("Flat ground")), t("An empty map at one height.")),
                    (true, crate::icons::with(crate::icons::GLOBE_HEMISPHERE_EAST, &t("Real place")), t("The satellite picture, heights and OpenStreetMap shapes of a real place.")),
                ]);
                ui.add_space(4.0);
                if p.real {
                    row(ui, &t("Name"), |ui| { ui.text_edit_singleline(&mut p.name); });
                    row(ui, &t("Folder"), |ui| {
                        ui.add(egui::TextEdit::singleline(&mut p.folder).desired_width(200.0));
                        if ui.button("…").clicked() {
                            if let Some(d) = rfd::FileDialog::new().pick_folder() { p.folder = d.display().to_string(); }
                        }
                    });
                    ui.add_space(4.0);
                    let families = app.road_families.clone();
                    crate::real_terrain_ui::ui(ui, p, &families);
                    ui.add_space(6.0);
                    let ready = crate::real_terrain_ui::ready(p);
                    let r = ui.add_enabled(ready.is_ok() && app.task.is_none(), egui::Button::new(crate::icons::with(crate::icons::DOWNLOAD_SIMPLE, &t("Download and create"))));
                    let r = match &ready { Err(e) => r.on_disabled_hover_text(e), Ok(()) => r };
                    if r.clicked() {
                        crate::real_terrain_ui::create(app, p);
                        close = true;
                    }
                    return;
                }
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
        Dialog::MapConfig => map_config_window(app, ctx, &mut open),
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
        Dialog::ConvertToBrush { kind, id, name } => {
            let (kind, id, name) = (*kind, *id, name.clone());
            let mut confirmed = false;
            egui::Window::new(icons::with(icons::PAINT_BRUSH, &t("Convert to brush")))
                .collapsible(false)
                .resizable(false)
                .min_width(360.0)
                .max_width(420.0)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label(RichText::new(tf("Convert {0} to a brush modifier?", &[&name])).strong());
                    ui.add_space(4.0);
                    ui.label(t("You will be able to paint and erase it like a hand-painted brush layer, and merge it with neighbouring brush layers."));
                    ui.add_space(6.0);
                    egui::Frame::new()
                        .fill(Color32::from_rgba_unmultiplied(230, 170, 50, 22))
                        .stroke(egui::Stroke::new(1.0_f32, theme::WARN))
                        .corner_radius(4.0)
                        .inner_margin(egui::Margin::same(8))
                        .show(ui, |ui| {
                            ui.colored_label(theme::WARN, icons::with(icons::WARNING, &t("After converting, you will no longer be able to adjust it through its original settings (such as generator, stamp, or colour correction parameters).")));
                        });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let btn = egui::Button::new(RichText::new(icons::with(icons::PAINT_BRUSH, &t("Convert to brush"))).color(Color32::WHITE))
                            .fill(theme::ACCENT);
                        if ui.add(btn).clicked() {
                            confirmed = true;
                            close = true;
                        }
                        if ui.button(t("Cancel")).clicked() {
                            close = true;
                        }
                    });
                });
            if confirmed {
                match kind {
                    crate::app::ConvertKind::Mask => {
                        if let Some(d) = app.doc.as_mut() {
                            let before_mods = d.world.mask_mods.modifiers.clone();
                            let detached_gen = d.world.generators.iter().any(|g| g.mask_modifier == Some(id));
                            let before_gens = detached_gen.then(|| d.world.generators.clone());
                            if d.world.mask_convert_to_brush(id) {
                                if let Some(gens) = before_gens {
                                    d.history.push(t("Convert to brush"), dz_core::edit::Change::Group(vec![
                                        dz_core::edit::Change::MaskMods(before_mods),
                                        dz_core::edit::Change::Generators(gens),
                                    ]));
                                } else {
                                    d.history.push(t("Convert to brush"), dz_core::edit::Change::MaskMods(before_mods));
                                }
                                app.paint.target = Some(id);
                                app.paint.into_modifier = true;
                                app.info(tf("{0} converted to a brush modifier", &[&name]));
                            }
                        }
                    }
                    crate::app::ConvertKind::Satellite => {
                        if let Some(d) = app.doc.as_mut() {
                            let before = d.world.sat.modifiers.clone();
                            let (ok, touched) = d.world.sat_convert_to_brush(id);
                            if ok {
                                d.history.push(t("Convert to brush"), dz_core::edit::Change::Satellite(before));
                                d.dirty.satellite = true;
                                app.sat.active = Some(id);
                                if let Some(r) = touched {
                                    app.satellite_changed(r);
                                }
                                app.info(tf("{0} converted to a brush modifier", &[&name]));
                            }
                        }
                    }
                }
            }
        }
        Dialog::DeleteLayer { layer, into } => {
            let names: Vec<String> = app.doc.as_ref().map(|d| d.world.layers.iter().map(|l| l.name.clone()).collect()).unwrap_or_default();
            let (layer, mut into_now) = (*layer, *into);
            let mut confirmed = false;
            if layer >= names.len() || names.len() < 2 {
                close = true;
            } else {
                egui::Window::new(t("Delete layer")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).open(&mut open).show(ctx, |ui| {
                    ui.label(RichText::new(tf("Delete the layer {0}?", &[&names[layer]])).strong());
                    row(ui, &t("Its ground becomes"), |ui| {
                        let pics: Vec<_> = app.layer_thumbs.iter().map(|t| t.as_ref().map(|t| t.id())).collect();
                        let items: Vec<_> = theme::surface_items(&names, &pics, |k, _| k).into_iter().filter(|i| i.key != layer).collect();
                        theme::search_pick(ui, "delete_layer_into", &mut into_now, None, &items, &|_| None, 200.0);
                    });
                    ui.colored_label(theme::WARN, t("The removal itself cannot be undone; earlier steps still can. The files of its texture stay on the disk."));
                    ui.horizontal(|ui| {
                        if ui.button(icons::with(icons::TRASH, &t("Delete"))).clicked() { confirmed = true; close = true; }
                        if ui.button(t("Cancel")).clicked() { close = true; }
                    });
                });
            }
            *into = into_now;
            if confirmed {
                delete_layer(app, layer, into_now);
            }
        }
    }
    if open && !close && matches!(app.dialog, Dialog::None) {
        app.dialog = dialog;
    }
}

/// Removes surface layer `layer`, its ground becoming layer `into`.
fn delete_layer(app: &mut App, layer: usize, into: usize) {
    let Some(doc) = &mut app.doc else { return };
    if !doc.world.remove_surface_layer(layer, into) {
        return;
    }
    doc.history.remap_mask(&dz_core::maskmod::layer_remap(layer, into));
    let shift = |i: usize| if i == layer { if into > layer { into - 1 } else { into } } else if i > layer { i - 1 } else { i };
    app.paint.layer = shift(app.paint.layer);
    app.paint.only_on = app.paint.only_on.filter(|i| *i != layer).map(shift);
    if app.material_edit.as_ref().is_some_and(|m| m.layer >= layer) {
        app.material_edit = None;
    }
    if layer < app.layer_thumbs.len() { app.layer_thumbs.remove(layer); }
    if layer < app.layer_averages.len() { app.layer_averages.remove(layer); }
    app.whole_mask_changed();
    app.push_palette();
    app.reload_layer_textures();
}

/// The pages of the settings window.
#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum SettingsPage {
    Paths,
    Display,
    Graphics,
    General,
    Backups,
    Shortcuts,
}

impl SettingsPage {
    const ALL: [SettingsPage; 6] = [
        SettingsPage::Paths,
        SettingsPage::Display,
        SettingsPage::Graphics,
        SettingsPage::General,
        SettingsPage::Backups,
        SettingsPage::Shortcuts,
    ];
    fn title(self) -> (&'static str, &'static str) {
        match self {
            SettingsPage::Paths => (icons::FOLDER_OPEN, "Paths"),
            SettingsPage::Display => (icons::EYE, "Display"),
            SettingsPage::Graphics => (icons::SUN, "Graphics"),
            SettingsPage::General => (icons::GEAR, "General"),
            SettingsPage::Backups => (icons::CLOCK, "Backups"),
            SettingsPage::Shortcuts => (icons::KEYBOARD, "Shortcuts"),
        }
    }
}

/// Opens the settings at a page by its name, for scripted checks.
pub(crate) fn open_settings(app: &mut App, ctx: &egui::Context, page: &str) {
    let p = match page {
        "display" => SettingsPage::Display,
        "graphics" => SettingsPage::Graphics,
        "general" => SettingsPage::General,
        "backups" => SettingsPage::Backups,
        "shortcuts" => SettingsPage::Shortcuts,
        _ => SettingsPage::Paths,
    };
    ctx.data_mut(|d| d.insert_persisted(egui::Id::new("settings_page"), p));
    app.dialog = Dialog::Settings;
}

/// Draws a full-width frame without expanding the parent window (prevents egui resize feedback).
fn fit_frame(ui: &mut egui::Ui, frame: egui::Frame, add: impl FnOnce(&mut egui::Ui)) {
    frame.show(ui, |ui| {
        ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), add);
    });
}

/// Settings as a window of pages: the pages on the left, the chosen one on the right.
fn settings_window(app: &mut App, ctx: &egui::Context, open: &mut bool) {
    let id = egui::Id::new("settings_page");
    let mut page: SettingsPage = ctx.data_mut(|d| d.get_persisted(id)).unwrap_or(SettingsPage::Paths);
    let title = icons::with(icons::GEAR, &t("Settings"));
    theme::PopupWindow::new(egui::Id::new("settings_popup"), &title)
        .default_size([760.0, 560.0])
        .min_size([580.0, 400.0])
        .show(ctx, open, |ui| {
        let path_warn = !app.settings.tools.check().is_empty();
        let key_warn = !app.settings.keys.clashes().is_empty();
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(vec2(160.0, 0.0), egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                for p in SettingsPage::ALL {
                    let (icon, name) = p.title();
                    let warn = (p == SettingsPage::Paths && path_warn) || (p == SettingsPage::Shortcuts && key_warn);
                    let text = RichText::new(icons::with(icon, &t(name))).color(if warn { theme::WARN } else if page == p { Color32::WHITE } else { theme::TEXT });
                    if ui.add(egui::SelectableLabel::new(page == p, text)).clicked() {
                        if page == SettingsPage::Shortcuts && p != SettingsPage::Shortcuts {
                            app.key_capture = None;
                        }
                        page = p;
                    }
                    ui.add_space(2.0);
                }
            });
            ui.separator();
            ui.vertical(|ui| {
                if page == SettingsPage::Shortcuts {
                    crate::keys::panel(ctx, ui, &mut app.settings.keys, &mut app.key_capture);
                } else {
                    egui::ScrollArea::vertical().id_salt(("settings_page_body", page as u8)).auto_shrink([false, false]).show(ui, |ui| {
                        match page {
                            SettingsPage::Paths => settings_paths(app, ui),
                            SettingsPage::Display => settings_display(app, ui, ctx),
                            SettingsPage::Graphics => settings_graphics(app, ui),
                            SettingsPage::General => settings_general(app, ui),
                            SettingsPage::Backups => settings_backups(app, ui),
                            SettingsPage::Shortcuts => {}
                        }
                    });
                }
            });
        });
    });
    ctx.data_mut(|d| d.insert_persisted(id, page));
    if !*open {
        app.key_capture = None;
        app.settings.save();
    }
}

fn settings_paths(app: &mut App, ui: &mut egui::Ui) {
    use egui::Stroke;
    let mut changed = false;
    section(ui, &icons::with(icons::FOLDER_OPEN, &t("Paths")), true, |ui| {
        about(ui, &t("Where the work drive, DayZ Tools and the game are; the editor reads models and builds the map with them."));
        ui.add_space(4.0);
        let mut path_row = |ui: &mut egui::Ui, label: String, p: &mut std::path::PathBuf| {
            row(ui, &label, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new(icons::FOLDER_OPEN).min_size(egui::vec2(26.0, 22.0))).on_hover_text(t("Choose...")).clicked() {
                        if let Some(d) = rfd::FileDialog::new().pick_folder() {
                            *p = d;
                            changed = true;
                        }
                    }
                    let mut s = p.display().to_string();
                    if ui.add(egui::TextEdit::singleline(&mut s).desired_width(f32::INFINITY)).changed() {
                        *p = s.into();
                        changed = true;
                    }
                });
            });
        };
        path_row(ui, t("Work drive"), &mut app.settings.tools.work_drive);
        path_row(ui, t("DayZ Tools"), &mut app.settings.tools.dayz_tools);
        path_row(ui, t("DayZ"), &mut app.settings.tools.dayz);
        ui.add_space(6.0);
        let problems = app.settings.tools.check();
        let detect_btn = if !problems.is_empty() {
            egui::Button::new(RichText::new(icons::with(icons::MAGNIFYING_GLASS, &t("Detect automatically"))).color(Color32::WHITE))
                .fill(theme::ACCENT)
                .stroke(Stroke::NONE)
                .min_size(egui::vec2(0.0, 24.0))
        } else {
            egui::Button::new(icons::with(icons::MAGNIFYING_GLASS, &t("Detect automatically")))
                .min_size(egui::vec2(0.0, 24.0))
        };
        if ui.add(detect_btn).clicked() {
            app.settings.tools = dz_core::build::detect();
            changed = true;
        }
        if changed {
            app.work_drive_changed();
        }
        let problems = app.settings.tools.check();
        ui.add_space(6.0);
        let col = if problems.is_empty() { theme::OK } else { theme::WARN };
        fit_frame(
            ui,
            egui::Frame::new()
                .fill(theme::DARK)
                .stroke(Stroke::new(1.0_f32, col.gamma_multiply(0.7)))
                .corner_radius(4)
                .inner_margin(egui::Margin::symmetric(8, 6)),
            |ui| {
                if problems.is_empty() {
                    ui.horizontal(|ui| {
                        ui.colored_label(theme::OK, icons::CHECK_CIRCLE);
                        ui.colored_label(theme::OK, t("All paths found."));
                    });
                } else {
                    for p in problems {
                        ui.horizontal_wrapped(|ui| {
                            ui.colored_label(theme::WARN, icons::WARNING);
                            ui.colored_label(theme::WARN, p);
                        });
                    }
                }
            },
        );
    });
}

fn settings_display(app: &mut App, ui: &mut egui::Ui, ctx: &egui::Context) {
    section(ui, &icons::with(icons::EYE, &t("Interface")), true, |ui| {
        row(ui, &t("Language"), |ui| {
            let cur = crate::i18n::current();
            let names = crate::i18n::languages();
            let label = names.iter().find(|(c, _)| *c == cur).map(|(_, n)| n.clone()).unwrap_or_default();
            let mut pick = None;
            egui::ComboBox::from_id_salt("lang").width(ui.available_width().min(220.0)).selected_text(label).show_ui(ui, |ui| {
                for (c, n) in names { if ui.selectable_label(c == cur, n).clicked() { pick = Some(c); } }
            });
            if let Some(c) = pick { app.sync_language(&c); }
        });
        let before = app.settings.ui_scale;
        row(ui, &t("Interface scale"), |ui| { ui.add(crate::theme::ValueField::new(&mut app.settings.ui_scale, 0.75..=2.0)); });
        if before != app.settings.ui_scale { theme::apply(ctx, app.settings.ui_scale); }
        let hints = format!("{} ({})", t("Show key help"), app.settings.keys.text(crate::keys::Action::ToggleHints));
        if theme::check_row(ui, &mut app.settings.own_title_bar, t("Own title bar"))
            .on_hover_text(t("The menus sit in the title bar, which the editor draws itself. Off, Windows draws its usual title bar."))
            .changed()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(!app.settings.own_title_bar));
            app.settings.save();
        }
        if theme::check_row(ui, &mut app.settings.show_hints, hints)
            .on_hover_text(t("The key tables and the lines telling what clicks and keys do, in all panels."))
            .changed()
        {
            app.settings.save();
        }
    });
    section(ui, &icons::with(icons::LAYOUT, &t("Layout")), true, |ui| {
        about(ui, &t("Puts every panel back where it was at first, at its first size."));
        ui.add_space(4.0);
        if ui.add(egui::Button::new(icons::with(icons::LAYOUT, &t("Default layout"))).min_size(egui::vec2(0.0, 24.0))).clicked() {
            crate::layout::reset(app);
        }
    });
}

fn settings_graphics(app: &mut App, ui: &mut egui::Ui) {
    section(ui, &icons::with(icons::SPARKLE, &t("Quality")), true, |ui| {
        let mut current = app.settings.graphics_preset();
        let choices: Vec<(Option<crate::settings::GraphicsPreset>, String, String)> = crate::settings::GraphicsPreset::ALL
            .iter()
            .map(|&p| (Some(p), t(p.name()), String::new()))
            .collect();
        row(ui, &t("Quality"), |ui| {
            ui.horizontal(|ui| {
                if theme::segmented(ui, &mut current, &choices) {
                    if let Some(p) = current {
                        app.settings.apply_graphics(p);
                    }
                }
                if current.is_none() {
                    ui.label(RichText::new(t("Custom")).small().color(theme::TEXT_DIM));
                }
            });
        });
        about(ui, &t("A quality sets everything below; changing any of it makes it Custom."));
    });
    let seg_u32 = |ui: &mut egui::Ui, label: &str, value: &mut u32, options: &[(u32, String)]| {
        let items: Vec<(u32, String, String)> = options.iter().map(|(v, n)| (*v, n.clone(), String::new())).collect();
        row(ui, label, |ui| { theme::segmented(ui, value, &items); });
    };
    section(ui, &icons::with(icons::RULER, &t("Distances")), true, |ui| {
        slider(ui, &t("Terrain detail"), &mut app.settings.terrain_quality, 0.6..=3.0, false);
        slider(ui, &t("Object draw distance"), &mut app.settings.draw_distance, 300.0..=8000.0, true);
        slider(ui, &t("Full detail distance"), &mut app.settings.near_detail, 20.0..=5000.0, true);
        slider(ui, &t("Surface texture range m"), &mut app.settings.surface_range, 60.0..=5000.0, true);
        theme::check_row(ui, &mut app.settings.occlusion, t("Leave out objects hidden behind the terrain or buildings"))
            .on_hover_text(t("Objects behind a hill, or behind the solid walls of a building or rock, are not drawn. Glass, fences and leaves never hide anything."));
    });
    section(ui, &icons::with(icons::IMAGE, &t("Textures")), true, |ui| {
        seg_u32(ui, &t("Texture detail"), &mut app.settings.texture_size, &[(512, "512".into()), (1024, "1024".into()), (2048, "2048".into()), (4096, t("Full"))]);
        seg_u32(ui, &t("Memory for models"), &mut app.settings.model_memory, &[(0, t("Auto")), (1024, "1 GB".into()), (2048, "2 GB".into()), (4096, "4 GB".into()), (8192, "8 GB".into())]);
        about(ui, &t("Models and textures not seen for a while are let go past this, and load again when the camera comes back. Less keeps the editor's memory down; more reloads less often."));
        seg_u32(ui, &t("Texture filtering"), &mut app.settings.anisotropy, &[(1, t("Off")), (2, "2x".into()), (4, "4x".into()), (8, "8x".into()), (16, "16x".into())]);
        slider(ui, &t("Surface parallax cm"), &mut app.settings.surface_parallax, 0.0..=15.0, false);
        slider(ui, &t("Surface blend by relief"), &mut app.settings.surface_blend, 0.0..=1.0, false);
        theme::check_row(ui, &mut app.settings.game_surfaces, t("Surfaces as the game shows them"))
            .on_hover_text(t("The ground shows the surfaces the built map will have: at most the surfaces a texture tile may hold, and two in each block of 4 metres, as the game's compressed masks keep them. Specks of a third surface at an edge go, as in the game. The Mask shading shows the same."));
        theme::check_row(ui, &mut app.settings.surface_pixels, t("Hard mask pixels"))
            .on_hover_text(t("The ground shows each pixel of the mask as one surface, with hard edges and no blending: to see exactly what is painted where."));
        slider(ui, &t("Ground colour on grass"), &mut app.settings.grass_ground_color, 0.0..=1.0, false);
    });
    section(ui, &icons::with(icons::SUN, &t("Shadows and effects")), true, |ui| {
        seg_u32(ui, &t("Shadow detail"), &mut app.settings.shadow_map, &[(2048, "2048".into()), (4096, "4096".into()), (8192, "8192".into())]);
        slider(ui, &t("Shadow range"), &mut app.settings.lighting.shadow_range, 60.0..=2000.0, true);
        seg_u32(ui, &t("Antialiasing"), &mut app.settings.edge_smoothing, &[(0, t("Off")), (1, "FXAA".into()), (2, "FXAA + 1.5x".into()), (3, "FXAA + 2x".into())]);
        seg_u32(ui, &t("Ambient occlusion"), &mut app.settings.ambient_occlusion, &[(0, t("Off")), (8, t("Low")), (12, t("Medium")), (20, t("High"))]);
    });
}

fn settings_general(app: &mut App, ui: &mut egui::Ui) {
    section(ui, &icons::with(icons::FLOPPY_DISK, &t("Saving")), true, |ui| {
        row(ui, &t("Autosave every"), |ui| { ui.add(crate::theme::ValueField::new(&mut app.settings.autosave_minutes, 0..=60).suffix(" min")); });
        about(ui, &t("0 turns autosave off."));
    });
    section(ui, &icons::with(icons::GEAR, &t("Application")), true, |ui| {
        theme::check_row(ui, &mut app.settings.check_updates, t("Check for updates at start"));
        theme::check_row(ui, &mut app.settings.experimental, t("Show experimental features"))
            .on_hover_text(t("Features still being worked on, which may change or not work fully yet. For now: the mission editor."));
        about(ui, &t("For now: the mission editor and team work."));
    });
}

fn settings_backups(app: &mut App, ui: &mut egui::Ui) {
    use egui::Stroke;
    section(ui, &icons::with(icons::CLOCK_COUNTER_CLOCKWISE, &t("Automatic backups")), true, |ui| {
        theme::check_row(ui, &mut app.settings.backup_enabled, t("Make backups automatically"));
        ui.add_enabled_ui(app.settings.backup_enabled, |ui| {
            row(ui, &t("Back up every"), |ui| { ui.add(crate::theme::ValueField::new(&mut app.settings.backup_minutes, 5..=240).suffix(" min")); });
            row(ui, &t("Backups to keep"), |ui| { ui.add(crate::theme::ValueField::new(&mut app.settings.backup_keep, 1..=100)); });
        });
        about(ui, &t("A backup is only made when the project has changed. Files that did not change are shared between backups, so they take no extra space."));
    });
    section(ui, &icons::with(icons::FOLDER, &t("Folder")), true, |ui| {
        let shown = if app.settings.backup_folder.is_empty() { t("backups in the project folder") } else { app.settings.backup_folder.clone() };
        fit_frame(
            ui,
            egui::Frame::new()
                .fill(theme::DARK)
                .stroke(Stroke::new(1.0_f32, theme::BORDER))
                .corner_radius(4)
                .inner_margin(egui::Margin::symmetric(8, 6)),
            |ui| {
                ui.add(egui::Label::new(RichText::new(shown).monospace().color(theme::TEXT_BRIGHT)).truncate());
            },
        );
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if ui.add(egui::Button::new(icons::with(icons::FOLDER_OPEN, &t("Choose..."))).min_size(egui::vec2(0.0, 24.0))).clicked() {
                if let Some(d) = rfd::FileDialog::new().pick_folder() { app.settings.backup_folder = d.display().to_string(); }
            }
            if ui.add_enabled(!app.settings.backup_folder.is_empty(), egui::Button::new(icons::with(icons::ARROW_COUNTER_CLOCKWISE, &t("Use the project folder"))).min_size(egui::vec2(0.0, 24.0))).clicked() {
                app.settings.backup_folder.clear();
            }
        });
    });
    section(ui, &icons::with(icons::ARCHIVE, &t("Now")), true, |ui| {
        ui.horizontal_wrapped(|ui| {
            let saved = app.doc.as_ref().is_some_and(|d| d.dir.is_some());
            if ui.add_enabled(saved && app.backup_job.is_none(), egui::Button::new(icons::with(icons::FLOPPY_DISK, &t("Back up now"))).min_size(egui::vec2(0.0, 24.0))).clicked() {
                app.start_backup(true);
            }
            if ui.add_enabled(saved, egui::Button::new(icons::with(icons::FOLDER_OPEN, &t("Open backups folder"))).min_size(egui::vec2(0.0, 24.0))).clicked() {
                if let Some(root) = app.backup_root() {
                    let _ = std::fs::create_dir_all(&root);
                    app.open_folder(&root);
                }
            }
        });
        ui.add_space(4.0);
        about(ui, &t("To restore a backup, open the project.dzwe inside it and use Save as."));
    });
}

/// The settings of the world config of the map, beside the config.cpp they make.
/// What the base world has for a setting the map config can override, shown as a hint.
fn base_value(base: &dz_core::world::BaseWorld, key: &str) -> &'static str {
    use dz_core::world::BaseWorld as B;
    let chernarus = matches!(base, B::ChernarusPlus);
    match key {
        "cutscenes" => if chernarus { "ChernarusPlusIntro" } else { "EnochIntro" },
        "startTime" => "9:00",
        "startDate" => if chernarus { "5/10/2015" } else { "25/8/2015" },
        "ceFiles" => if chernarus { r"DZ\worlds\chernarusplus\ce" } else { r"DZ\worlds\enoch\ce" },
        "mapDisplayNameKey" => "#STR_CfgVehicles_ChernarusMap0",
        "mapDescriptionKey" => "#STR_CfgVehicles_ChernarusMap1",
        "mapTextureClosed" => if chernarus { r"dz\gear\navigation\data\map_chernarus_co.paa" } else { r"dz\gear\navigation\data\map_enoch_co.paa" },
        "mapTextureOpened" => if chernarus { r"dz\structures\signs\tourist\data\karta_co.paa" } else { r"dz\structures_bliss\signs\tourist\data\karta_enoch_co.paa" },
        "mapTextureLegend" => if chernarus { r"dz\structures\signs\tourist\data\karta_side_co.paa" } else { r"dz\structures_bliss\signs\tourist\data\karta_enoch_side_co.paa" },
        "userMapPath" => if chernarus { r"dz\gear\navigation\data\usermap" } else { r"dz\worlds\enoch\data\usermap" },
        "satellite" => if chernarus { r"DZ\worlds\chernarusplus\data\outside_sat_co.paa" } else { r"DZ\worlds\enoch\data\enoch_satout_co.paa" },
        "texture" => if chernarus { r"DZ\surfaces\data\terrain\cp_grass_ca.paa" } else { r"DZ\surfaces_bliss\data\terrain\en_grass1_ca.paa" },
        "nopx" => if chernarus { r"DZ\surfaces\data\terrain\cp_grass_nopx.paa" } else { r"DZ\surfaces_bliss\data\terrain\en_grass1_nopx.paa" },
        "midDetailTexture" => if chernarus { r"DZ\worlds\chernarusplus\data\middle_sat_mco.paa" } else { r"DZ\worlds\enoch\data\enoch_middle_mco.paa" },
        "terrainNormalTexture" => if chernarus { r"DZ\worlds\chernarusplus\data\global_nohq.paa" } else { r"DZ\worlds\enoch\data\enoch_global_nohq.paa" },
        "oceanMaterial" => if chernarus { "" } else { r"dz\worlds\enoch\data\enoch_riversea.emat" },
        "noDetailDist" => if chernarus { "40" } else { "65" },
        "fullDetailDist" => if chernarus { "15" } else { "10" },
        "clutterDist" => "125",
        "clutterGrid" => "1",
        "minTrees" => "10",
        "minRocks" => "5",
        "envTexture" => r"DZ\Data\data\env_co.paa",
        "atmosphereTexture" => r"DZ\worlds\chernarusplus\data\Sky_Stage01_Clear_sky.paa",
        "maxTide" => "1.5",
        "maxWave" => "0.25",
        "waveAmplitude" => "0.081",
        "waveFrequency" => "9",
        "waveOctaves" => "6",
        "wavePersistence" => "0.745",
        "starsIntensity" => "0.181",
        "hazeDistCoef" => "0",
        "hazeFogCoef" => "1",
        "volFogOffset" => "0",
        "satNormStart" => "100",
        "satNormEnd" => "500",
        "terrainNormalPower" => "3.21",
        "soundMapAttenCoef" => "0.01",
        "tileWidth" => "50",
        "maxSlope" => "60",
        "maxStep" => "0.45",
        _ => "",
    }
}

/// A text setting of the config: empty keeps the value of the base world, shown greyed.
fn config_text(ui: &mut egui::Ui, label: &str, value: &mut String, inherited: &str) {
    row(ui, label, |ui| {
        let hint = if inherited.is_empty() { t("(from the base world)") } else { inherited.to_string() };
        ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(f32::INFINITY));
    });
}

/// A number setting of the config: off keeps the value of the base world.
fn config_number(ui: &mut egui::Ui, label: &str, value: &mut Option<f32>, inherited: &str, range: std::ops::RangeInclusive<f32>, suffix: &str) {
    row(ui, label, |ui| {
        let mut on = value.is_some();
        if ui.checkbox(&mut on, "").changed() {
            *value = on.then(|| inherited.parse().unwrap_or(*range.start()));
        }
        match value {
            Some(v) => { ui.add(egui::DragValue::new(v).speed(1.0).range(range).suffix(suffix)); }
            None => { ui.weak(tf("{0} (base world)", &[inherited])); }
        }
    });
}

fn map_config_window(app: &mut App, ctx: &egui::Context, open: &mut bool) {
    use dz_core::world::BaseWorld;
    let camera = [app.camera.pos[0], app.camera.pos[2]];
    let (cam_pos, cam_fwd, cam_fov) = (app.camera.pos, app.camera.forward(), app.camera.fov);
    let mut go_to: Option<dz_core::world::CharacterScene> = None;
    let Some(doc) = &mut app.doc else { *open = false; return };
    let before = doc.world.meta.clone();
    let size = doc.world.meta.size_m();
    let mut copied = false;
    let title = t("Map config");
    crate::theme::PopupWindow::new(egui::Id::new("map_config_window"), &title)
        .default_size([980.0, 620.0])
        .min_size([640.0, 420.0])
        .show(ctx, open, |ui| {
        ui.columns(2, |cols| {
            let ui = &mut cols[0];
            egui::ScrollArea::vertical().id_salt("cfg_form").auto_shrink([false, false]).show(ui, |ui| {
                let height = &doc.world.height;
                let doc_height = |x: f32, z: f32| height.sample(x, z);
                let m = &mut doc.world.meta;
                section(ui, &t("World"), true, |ui| {
                    row(ui, &t("Description"), |ui| { ui.add(egui::TextEdit::singleline(&mut m.config.description).hint_text(m.name.clone()).desired_width(f32::INFINITY)); });
                    ui.weak(t("The name the game shows; empty takes the name of the project."));
                    row(ui, &t("Map ID"), |ui| { ui.text_edit_singleline(&mut m.map_id); });
                    row(ui, &t("Author"), |ui| { ui.text_edit_singleline(&mut m.author); });
                    let kind = match &m.config.base { BaseWorld::Enoch => 0, BaseWorld::ChernarusPlus => 1, BaseWorld::Custom { .. } => 2 };
                    let mut pick = kind;
                    row(ui, &t("Built on"), |ui| {
                        egui::ComboBox::from_id_salt("cfg_base").selected_text(match kind { 0 => "Enoch (Livonia)".to_string(), 1 => "ChernarusPlus".to_string(), _ => t("Another world") }).show_ui(ui, |ui| {
                            ui.selectable_value(&mut pick, 0, "Enoch (Livonia)");
                            ui.selectable_value(&mut pick, 1, "ChernarusPlus");
                            ui.selectable_value(&mut pick, 2, t("Another world"));
                        });
                    });
                    if pick != kind {
                        m.config.base = match pick { 0 => BaseWorld::Enoch, 1 => BaseWorld::ChernarusPlus, _ => BaseWorld::Custom { class: "Enoch".into(), addon: "DZ_Worlds_Enoch".into() } };
                    }
                    if let BaseWorld::Custom { class, addon } = &mut m.config.base {
                        row(ui, &t("World class"), |ui| { ui.text_edit_singleline(class); });
                        row(ui, &t("Its addon"), |ui| { ui.text_edit_singleline(addon); });
                    }
                    ui.weak(t("The map takes over the settings of this world: its weather, light, sounds and the rest the config does not set."));
                    ui.checkbox(&mut m.config.has_ocean, t("Sea around the map (hasOcean)"));
                });
                section(ui, &t("Position"), true, |ui| {
                    let mut own = m.config.center.is_some();
                    if ui.checkbox(&mut own, t("Own map centre")).on_hover_text(t("Where the game's map opens; without it the middle of the terrain.")).changed() {
                        m.config.center = own.then_some([size * 0.5, size * 0.5]);
                    }
                    if let Some(c) = &mut m.config.center {
                        row(ui, "X", |ui| { ui.add(egui::DragValue::new(&mut c[0]).speed(5.0).range(0.0..=size).suffix(" m")); });
                        row(ui, "Z", |ui| { ui.add(egui::DragValue::new(&mut c[1]).speed(5.0).range(0.0..=size).suffix(" m")); });
                        if ui.button(t("Where the camera is")).clicked() { *c = [camera[0].clamp(0.0, size), camera[1].clamp(0.0, size)]; }
                    }
                    row(ui, &t("Latitude"), |ui| { ui.add(egui::DragValue::new(&mut m.latitude).speed(0.1).range(-90.0..=90.0).suffix("°")); });
                    row(ui, &t("Longitude"), |ui| { ui.add(egui::DragValue::new(&mut m.longitude).speed(0.1).range(-180.0..=180.0).suffix("°")); });
                    ui.weak(t("Latitude and longitude set where the sun and the stars stand."));
                });
                section(ui, &t("Map grid"), false, |ui| {
                    for (k, name) in ["Close up", "Middle", "Far out"].iter().enumerate() {
                        row(ui, &t(name), |ui| { ui.add(egui::DragValue::new(&mut m.config.grid_steps[k]).speed(10.0).range(10..=100000).suffix(" m")); });
                    }
                    ui.weak(t("Steps of the grid on the in-game map at its three zoom levels."));
                });
                section(ui, &t("Main menu scenes"), false, |ui| {
                    ui.weak(t("Places the main menu shows behind the character. Set one from where the 3D camera stands and looks; without any the base world's are used."));
                    let ground = |x: f32, z: f32| doc_height(x, z);
                    let from_camera = |s: &mut dz_core::world::CharacterScene| {
                        s.position = [cam_pos[0], cam_pos[2], (cam_pos[1] - ground(cam_pos[0], cam_pos[2])).max(0.2)];
                        let reach = 60.0;
                        s.target = [cam_pos[0] + cam_fwd[0] * reach, cam_pos[2] + cam_fwd[2] * reach, cam_pos[1] + cam_fwd[1] * reach];
                        s.fov = cam_fov.clamp(0.1, 2.5);
                    };
                    let mut remove = None;
                    let count = m.config.scenes.len();
                    for (k, s) in m.config.scenes.iter_mut().enumerate() {
                        egui::CollapsingHeader::new(format!("{}  {}", k + 1, s.name)).id_salt(("scene", k)).default_open(k + 1 == count).show(ui, |ui| {
                            row(ui, &t("Name"), |ui| { ui.text_edit_singleline(&mut s.name); });
                            row(ui, &t("Camera"), |ui| {
                                ui.add(egui::DragValue::new(&mut s.position[0]).speed(1.0).prefix("X "));
                                ui.add(egui::DragValue::new(&mut s.position[1]).speed(1.0).prefix("Z "));
                                ui.add(egui::DragValue::new(&mut s.position[2]).speed(0.1).prefix("↑ ").suffix(" m"));
                            });
                            row(ui, &t("Looks at"), |ui| {
                                ui.add(egui::DragValue::new(&mut s.target[0]).speed(1.0).prefix("X "));
                                ui.add(egui::DragValue::new(&mut s.target[1]).speed(1.0).prefix("Z "));
                                ui.add(egui::DragValue::new(&mut s.target[2]).speed(0.5).prefix("Y "));
                            });
                            let mut deg = s.fov.to_degrees();
                            row(ui, &t("Field of view"), |ui| { if ui.add(crate::theme::ValueField::new(&mut deg, 10.0..=120.0).suffix("°")).changed() { s.fov = deg.to_radians(); } });
                            row(ui, &t("Date and time"), |ui| {
                                ui.add(egui::DragValue::new(&mut s.date[2]).range(1..=31).prefix(""));
                                ui.label(".");
                                ui.add(egui::DragValue::new(&mut s.date[1]).range(1..=12));
                                ui.label(".");
                                ui.add(egui::DragValue::new(&mut s.date[0]).range(1900..=2100));
                                ui.add(egui::DragValue::new(&mut s.date[3]).range(0..=23));
                                ui.label(":");
                                ui.add(egui::DragValue::new(&mut s.date[4]).range(0..=59));
                            });
                            row(ui, &t("Overcast"), |ui| { ui.add(crate::theme::ValueField::new(&mut s.overcast, 0.0..=1.0)); });
                            row(ui, &t("Rain"), |ui| { ui.add(crate::theme::ValueField::new(&mut s.rain, 0.0..=1.0)); });
                            row(ui, &t("Fog"), |ui| { ui.add(crate::theme::ValueField::new(&mut s.fog, 0.0..=1.0)); });
                            ui.horizontal(|ui| {
                                if ui.button(icons::with(icons::CAMERA, &t("From the camera"))).on_hover_text(t("Where the 3D camera stands and what it looks at.")).clicked() { from_camera(s); }
                                if ui.button(icons::with(icons::EYE, &t("Look from here"))).on_hover_text(t("Takes the 3D camera to this scene.")).clicked() { go_to = Some(s.clone()); }
                                if ui.button(icons::with(icons::TRASH, &t("Delete"))).clicked() { remove = Some(k); }
                            });
                        });
                    }
                    if let Some(k) = remove { m.config.scenes.remove(k); }
                    if ui.button(icons::with(icons::PLUS, &t("Add a scene from the camera"))).clicked() {
                        let mut s = dz_core::world::CharacterScene { name: format!("loc{}", m.config.scenes.len() + 1), ..Default::default() };
                        from_camera(&mut s);
                        m.config.scenes.push(s);
                    }
                });
                section(ui, &t("Start and intro"), false, |ui| {
                    let base = m.config.base.clone();
                    let mut list = m.config.cutscenes.join(", ");
                    row(ui, &t("Intro (cutscenes)"), |ui| {
                        if ui.add(egui::TextEdit::singleline(&mut list).hint_text(base_value(&base, "cutscenes")).desired_width(f32::INFINITY)).changed() {
                            m.config.cutscenes = list.split(',').map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect();
                        }
                    });
                    ui.weak(t("Names of the intro missions, separated by commas; the base world's intro shows its own map."));
                    config_text(ui, &t("Start time"), &mut m.config.start_time, base_value(&base, "startTime"));
                    config_text(ui, &t("Start date"), &mut m.config.start_date, base_value(&base, "startDate"));
                    config_text(ui, &t("Central Economy folder (ceFiles)"), &mut m.config.ce_files, base_value(&base, "ceFiles"));
                    if ui.small_button(t("This map's ce folder")).clicked() { m.config.ce_files = format!("{}\\ce", m.map_id); }
                });
                section(ui, &t("Map item"), false, |ui| {
                    let base = m.config.base.clone();
                    ui.weak(t("The paper map players carry and the tourist boards; empty fields keep those of the base world."));
                    config_text(ui, &t("Name key"), &mut m.config.map_display_name_key, base_value(&base, "mapDisplayNameKey"));
                    config_text(ui, &t("Description key"), &mut m.config.map_description_key, base_value(&base, "mapDescriptionKey"));
                    config_text(ui, &t("Folded map texture"), &mut m.config.map_texture_closed, base_value(&base, "mapTextureClosed"));
                    config_text(ui, &t("Open map texture"), &mut m.config.map_texture_opened, base_value(&base, "mapTextureOpened"));
                    config_text(ui, &t("Legend texture"), &mut m.config.map_texture_legend, base_value(&base, "mapTextureLegend"));
                    config_text(ui, &t("User map folder"), &mut m.config.user_map_path, base_value(&base, "userMapPath"));
                });
                section(ui, &t("Terrain look"), false, |ui| {
                    let base = m.config.base.clone();
                    ui.label(t("Beyond the edge of the map"));
                    config_text(ui, &t("Satellite"), &mut m.config.outside_satellite, base_value(&base, "satellite"));
                    row(ui, &t("Terrain synthesis"), |ui| {
                        egui::ComboBox::from_id_salt("cfg_synth").selected_text(match m.config.outside_synth { None => t("(from the base world)"), Some(true) => t("On"), Some(false) => t("Off") }).show_ui(ui, |ui| {
                            ui.selectable_value(&mut m.config.outside_synth, None, t("(from the base world)"));
                            ui.selectable_value(&mut m.config.outside_synth, Some(true), t("On"));
                            ui.selectable_value(&mut m.config.outside_synth, Some(false), t("Off"));
                        });
                    });
                    config_text(ui, &t("Surface texture"), &mut m.config.outside_texture, base_value(&base, "texture"));
                    config_text(ui, &t("Surface relief (nopx)"), &mut m.config.outside_nopx, base_value(&base, "nopx"));
                    ui.separator();
                    config_text(ui, &t("Middle distance texture"), &mut m.config.mid_detail_texture, base_value(&base, "midDetailTexture"));
                    config_text(ui, &t("Terrain normal map"), &mut m.config.terrain_normal_texture, base_value(&base, "terrainNormalTexture"));
                    config_number(ui, &t("Normal blend start"), &mut m.config.satellite_normal_blend_start, base_value(&base, "satNormStart"), 0.0..=5000.0, " m");
                    config_number(ui, &t("Normal blend end"), &mut m.config.satellite_normal_blend_end, base_value(&base, "satNormEnd"), 0.0..=10000.0, " m");
                    config_number(ui, &t("Terrain normal power"), &mut m.config.terrain_normal_power, base_value(&base, "terrainNormalPower"), 0.0..=10.0, "");
                    for (label, id_salt, flag) in [
                        (t("Height blending"), "cfg_hblend", &mut m.config.height_blending_mode),
                        (t("Bicubic filtering"), "cfg_bicubic", &mut m.config.bicubic_mode),
                    ] {
                        row(ui, &label, |ui| {
                            egui::ComboBox::from_id_salt(id_salt).selected_text(match flag { None => t("(from the base world)"), Some(true) => t("On"), Some(false) => t("Off") }).show_ui(ui, |ui| {
                                ui.selectable_value(flag, None, t("(from the base world)"));
                                ui.selectable_value(flag, Some(true), t("On"));
                                ui.selectable_value(flag, Some(false), t("Off"));
                            });
                        });
                    }
                    ui.weak(t("The textures of the base world show its own terrain; a map of its own needs its own."));
                });
                section(ui, &t("Sea and waves"), false, |ui| {
                    let base = m.config.base.clone();
                    config_text(ui, &t("Sea material"), &mut m.config.ocean_material, base_value(&base, "oceanMaterial"));
                    config_text(ui, &t("Calm sea material"), &mut m.config.ocean_nice_material, base_value(&base, "oceanMaterial"));
                    config_text(ui, &t("Storm sea material"), &mut m.config.ocean_storm_material, base_value(&base, "oceanMaterial"));
                    ui.weak(t("Leave empty to reuse Sea material for calm and storm weather too."));
                    config_number(ui, &t("Max tide"), &mut m.config.max_tide, base_value(&base, "maxTide"), 0.0..=10.0, " m");
                    config_number(ui, &t("Max wave"), &mut m.config.max_wave, base_value(&base, "maxWave"), 0.0..=5.0, " m");
                    config_number(ui, &t("Wave amplitude"), &mut m.config.wave_amplitude, base_value(&base, "waveAmplitude"), 0.0..=2.0, "");
                    config_number(ui, &t("Wave frequency"), &mut m.config.wave_frequency, base_value(&base, "waveFrequency"), 0.0..=50.0, "");
                    let mut oct = m.config.wave_octaves.map(|v| v as f32);
                    config_number(ui, &t("Wave octaves"), &mut oct, base_value(&base, "waveOctaves"), 1.0..=12.0, "");
                    m.config.wave_octaves = oct.map(|v| v.round() as u32);
                    config_number(ui, &t("Wave persistence"), &mut m.config.wave_persistence, base_value(&base, "wavePersistence"), 0.0..=2.0, "");
                });
                section(ui, &t("Atmosphere and sky"), false, |ui| {
                    let base = m.config.base.clone();
                    config_text(ui, &t("Environment map (envTexture)"), &mut m.config.env_texture, base_value(&base, "envTexture"));
                    config_text(ui, &t("Atmosphere texture"), &mut m.config.atmosphere_texture, base_value(&base, "atmosphereTexture"));
                    config_number(ui, &t("Stars intensity"), &mut m.config.stars_intensity, base_value(&base, "starsIntensity"), 0.0..=2.0, "");
                    config_number(ui, &t("Haze distance coef"), &mut m.config.haze_dist_coef, base_value(&base, "hazeDistCoef"), 0.0..=5.0, "");
                    config_number(ui, &t("Haze fog coef"), &mut m.config.haze_fog_coef, base_value(&base, "hazeFogCoef"), 0.0..=5.0, "");
                    config_number(ui, &t("Volumetric fog offset"), &mut m.config.vol_fog_offset, base_value(&base, "volFogOffset"), -500.0..=500.0, " m");
                });
                section(ui, &t("Ambient sound map"), false, |ui| {
                    let base = m.config.base.clone();
                    config_number(ui, &t("Sound map attenuation"), &mut m.config.sound_map_atten_coef, base_value(&base, "soundMapAttenCoef"), 0.0..=1.0, "");
                    let mut own_sm = m.config.sound_map.is_some();
                    if ui.checkbox(&mut own_sm, t("Override SoundMapValues")).changed() {
                        m.config.sound_map = own_sm.then(Default::default);
                    }
                    if let Some(sm) = &mut m.config.sound_map {
                        for (label, val) in [
                            (t("Hard trees"), &mut sm.treehard),
                            (t("Soft trees"), &mut sm.treesoft),
                            (t("Hard bushes"), &mut sm.bushhard),
                            (t("Soft bushes"), &mut sm.bushsoft),
                            (t("Forest weight"), &mut sm.forest),
                            (t("Houses weight"), &mut sm.house),
                            (t("Church weight"), &mut sm.church),
                        ] {
                            row(ui, &label, |ui| { ui.add(egui::DragValue::new(val).speed(0.01).range(0.0..=5.0)); });
                        }
                    }
                });
                section(ui, &t("Navmesh settings"), false, |ui| {
                    let base = m.config.base.clone();
                    row(ui, &t("Filter isolated islands"), |ui| {
                        egui::ComboBox::from_id_salt("cfg_nm_islands").selected_text(match m.config.navmesh_filter_islands { None => t("(from the base world)"), Some(true) => t("On"), Some(false) => t("Off") }).show_ui(ui, |ui| {
                            ui.selectable_value(&mut m.config.navmesh_filter_islands, None, t("(from the base world)"));
                            ui.selectable_value(&mut m.config.navmesh_filter_islands, Some(true), t("On"));
                            ui.selectable_value(&mut m.config.navmesh_filter_islands, Some(false), t("Off"));
                        });
                    });
                    config_number(ui, &t("Tile width"), &mut m.config.navmesh_tile_width, base_value(&base, "tileWidth"), 10.0..=200.0, " m");
                    config_number(ui, &t("Agent max slope"), &mut m.config.navmesh_max_slope, base_value(&base, "maxSlope"), 10.0..=85.0, "°");
                    config_number(ui, &t("Agent max step"), &mut m.config.navmesh_max_step, base_value(&base, "maxStep"), 0.1..=2.0, " m");
                });
                section(ui, &t("Detail distances"), false, |ui| {
                    let base = m.config.base.clone();
                    config_number(ui, &t("Grass distance"), &mut m.config.clutter_dist, base_value(&base, "clutterDist"), 10.0..=500.0, " m");
                    config_number(ui, &t("Grass grid"), &mut m.config.clutter_grid, base_value(&base, "clutterGrid"), 0.25..=10.0, " m");
                    config_number(ui, &t("Surface detail distance"), &mut m.config.no_detail_dist, base_value(&base, "noDetailDist"), 5.0..=500.0, " m");
                    config_number(ui, &t("Full detail distance"), &mut m.config.full_detail_dist, base_value(&base, "fullDetailDist"), 1.0..=200.0, " m");
                    let mut trees = m.config.min_trees_in_forest_square.map(|v| v as f32);
                    config_number(ui, &t("Trees for a forest square"), &mut trees, base_value(&base, "minTrees"), 1.0..=100.0, "");
                    m.config.min_trees_in_forest_square = trees.map(|v| v.round() as u32);
                    let mut rocks = m.config.min_rocks_in_rock_square.map(|v| v as f32);
                    config_number(ui, &t("Rocks for a rock square"), &mut rocks, base_value(&base, "minRocks"), 1.0..=100.0, "");
                    m.config.min_rocks_in_rock_square = rocks.map(|v| v.round() as u32);
                });
                section(ui, &t("Addons and extra lines"), false, |ui| {
                    ui.label(t("Required addons, one per line"));
                    let mut text = m.config.required_addons.join("\n");
                    if ui.add(egui::TextEdit::multiline(&mut text).code_editor().desired_rows(3).desired_width(f32::INFINITY).hint_text("MyMod_Data")).changed() {
                        m.config.required_addons = text.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
                    }
                    ui.weak(t("DZ_Data and the addon of the base world are always there."));
                    ui.label(t("Lines in the world class"));
                    ui.add(egui::TextEdit::multiline(&mut m.config.extra_world).code_editor().desired_rows(5).desired_width(f32::INFINITY).hint_text("minTreesInForestSquare=10;"));
                    ui.label(t("Lines at the end of the file"));
                    ui.add(egui::TextEdit::multiline(&mut m.config.extra_file).code_editor().desired_rows(4).desired_width(f32::INFINITY));
                    ui.weak(t("Written as they are; locations come from the key points."));
                });
                ui.add_space(6.0);
                ui.separator();
                if ui.button(icons::with(icons::ARROW_COUNTER_CLOCKWISE, &t("Back to the defaults"))).on_hover_text(t("Clears every setting of this window, so the values of the base world apply again.")).clicked() { m.config = dz_core::world::MapConfig { surfaces: std::mem::take(&mut m.config.surfaces), ..Default::default() }; }
            });
            let ui = &mut cols[1];
            ui.horizontal(|ui| {
                ui.strong("world\\config.cpp");
                if ui.button(icons::with(icons::COPY, &t("Copy"))).clicked() { copied = true; }
            });
            let mut text = dz_core::export::world_config_text(&doc.world, false);
            if copied { ui.ctx().copy_text(text.clone()); }
            let braces = text.matches('{').count() as i64 - text.matches('}').count() as i64;
            if braces != 0 {
                ui.colored_label(Color32::from_rgb(235, 110, 90), tf("The braces do not pair up ({0}); check the extra lines.", &[&braces.to_string()]));
            }
            egui::ScrollArea::both().id_salt("cfg_preview").auto_shrink([false, false]).show(ui, |ui| {
                ui.add(egui::TextEdit::multiline(&mut text).code_editor().interactive(false).desired_width(f32::INFINITY));
            });
        });
    });
    if doc.world.meta != before {
        doc.history.revision += 1;
    }
    if copied {
        app.status = t("The config is on the clipboard");
    }
    if let Some(s) = go_to {
        let ground = app.doc.as_ref().map_or(0.0, |d| d.world.height.sample(s.position[0], s.position[1]));
        let pos = [s.position[0], ground + s.position[2], s.position[1]];
        let d = [s.target[0] - pos[0], s.target[2] - pos[1], s.target[1] - pos[2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-3);
        app.camera.pos = pos;
        app.camera.yaw = d[0].atan2(d[2]);
        app.camera.pitch = (d[1] / len).asin();
        app.camera.fov = s.fov;
        app.view_mode = crate::app::ViewMode::Perspective;
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum ProjectPage {
    #[default]
    General,
    TileSurfaces,
}

impl ProjectPage {
    const ALL: [Self; 2] = [Self::General, Self::TileSurfaces];
    fn title(self) -> (&'static str, &'static str) {
        match self {
            Self::General => (icons::GEAR, "General"),
            Self::TileSurfaces => (icons::SQUARES_FOUR, "Tile surfaces"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LayerRule {
    Auto,
    Protect,
    RemoveFirst,
}

fn draw_surface_swatch(ui: &mut egui::Ui, size: f32, thumb: Option<egui::TextureId>, color: [u8; 3]) {
    use egui::Stroke;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    match thumb {
        Some(tex) => {
            ui.painter().image(tex, rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
            ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0_f32, theme::BORDER), egui::StrokeKind::Inside);
        }
        None => {
            ui.painter().rect_filled(rect, 2.0, Color32::from_rgb(color[0], color[1], color[2]));
            ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0_f32, Color32::from_black_alpha(120)), egui::StrokeKind::Inside);
        }
    }
}

fn project_window(app: &mut App, ctx: &egui::Context, open: &mut bool) {
    use dz_core::mask_limit::{FixScope, ReplaceMode};
    use egui::Stroke;
    let layer_pics: Vec<Option<egui::TextureId>> = app.layer_thumbs.iter().map(|t| t.as_ref().map(|t| t.id())).collect();
    let Some(doc) = &mut app.doc else { *open = false; return };
    let before = doc.world.meta.clone();
    let layers: Vec<(String, [u8; 3])> = doc.world.layers.iter().map(|l| (l.name.clone(), l.color)).collect();
    let layer_names: Vec<String> = layers.iter().map(|(n, _)| n.clone()).collect();
    let mask_w = doc.world.mask.width.max(1) as f32;
    let mask_h = doc.world.mask.height.max(1) as f32;
    let size_m = doc.world.meta.size_m();
    let mut tile_limit = app.tile_limit.clone();
    let opts_before = tile_limit.options.clone();
    let (mut check, mut clear_marks, mut fix) = (false, false, false);
    let mut fly_target: Option<(u32, u32, [f32; 2])> = None;
    let mut open_config = false;
    let m = &mut doc.world.meta;
    let page_id = egui::Id::new("project_settings_page");
    let mut page: ProjectPage = ctx.data_mut(|d| d.get_temp(page_id)).unwrap_or_default();
    let title = icons::with(icons::SLIDERS_HORIZONTAL, &t("Project settings"));
    crate::theme::PopupWindow::new(egui::Id::new("project_settings_window"), &title)
        .default_size([740.0, 560.0])
        .min_size([560.0, 400.0])
        .show(ctx, open, |ui| {
        ui.horizontal_top(|ui| {
            // Left navigation column, matching the editor Settings window.
            ui.allocate_ui_with_layout(egui::vec2(160.0, 0.0), egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                for p in ProjectPage::ALL {
                    let (icon, name) = p.title();
                    let label = if p == ProjectPage::TileSurfaces && !tile_limit.issues.is_empty() {
                        format!("{} ({})", icons::with(icon, &t(name)), tile_limit.issues.len())
                    } else {
                        icons::with(icon, &t(name))
                    };
                    let color = if p == ProjectPage::TileSurfaces && tile_limit.needs_check {
                        theme::WARN
                    } else if p == ProjectPage::TileSurfaces && !tile_limit.issues.is_empty() {
                        theme::ERROR
                    } else if page == p {
                        Color32::WHITE
                    } else {
                        theme::TEXT
                    };
                    if ui.add(egui::SelectableLabel::new(page == p, RichText::new(label).color(color))).clicked() {
                        page = p;
                    }
                    ui.add_space(2.0);
                }
            });
            ui.separator();
            ui.vertical(|ui| {
                egui::ScrollArea::vertical().id_salt(("project_page_body", page as u8)).auto_shrink([false, false]).show(ui, |ui| {
                    match page {
                        ProjectPage::General => {
                            section(ui, &icons::with(icons::FOLDER, &t("Project")), true, |ui| {
                                row(ui, &t("Name"), |ui| { ui.add(egui::TextEdit::singleline(&mut m.name).desired_width(f32::INFINITY)); });
                                row(ui, &t("Map ID"), |ui| { ui.add(egui::TextEdit::singleline(&mut m.map_id).desired_width(f32::INFINITY)); });
                                row(ui, &t("Author"), |ui| { ui.add(egui::TextEdit::singleline(&mut m.author).desired_width(f32::INFINITY)); });
                            });
                            section(ui, &icons::with(icons::GLOBE, &t("World")), true, |ui| {
                                row(ui, &t("Sea level"), |ui| { ui.add_sized([ui.available_width(), 22.0], egui::DragValue::new(&mut m.sea_level).speed(0.1).suffix(" m")); });
                                row(ui, &t("Easting offset"), |ui| { ui.add_sized([ui.available_width(), 22.0], egui::DragValue::new(&mut m.easting).speed(100.0)); });
                                row(ui, &t("Latitude"), |ui| { ui.add_sized([ui.available_width(), 22.0], egui::DragValue::new(&mut m.latitude).speed(0.1).range(-90.0..=90.0).suffix("°")); });
                                row(ui, &t("Longitude"), |ui| { ui.add_sized([ui.available_width(), 22.0], egui::DragValue::new(&mut m.longitude).speed(0.1).range(-180.0..=180.0).suffix("°")); });
                            });
                            section(ui, &icons::with(icons::EXPORT, &t("Export")), true, |ui| {
                                theme::check_row(ui, &mut m.write_underground_flag, t("Write the newer WRP format with underground flag"));
                                ui.add_space(4.0);
                                if ui.button(icons::with(icons::FILE_CODE, &t("Map config..."))).on_hover_text(t("The world config (CfgWorlds) of the map, written to config.cpp on export.")).clicked() {
                                    open_config = true;
                                }
                            });
                            section(ui, &icons::with(icons::INFO, &t("Terrain summary")), true, |ui| {
                                fit_frame(
                                    ui,
                                    egui::Frame::new()
                                        .fill(theme::DARK)
                                        .stroke(Stroke::new(1.0_f32, theme::BORDER))
                                        .corner_radius(4)
                                        .inner_margin(egui::Margin::symmetric(10, 8)),
                                    |ui| {
                                        ui.label(RichText::new(tf("Terrain {0} × {0}, cell {1} m, map {2} m", &[&m.terrain_size.to_string(), &m.terrain_cell.to_string(), &m.size_m().to_string()])).color(theme::TEXT_BRIGHT));
                                        ui.label(RichText::new(tf("Land grid {0}, cell {1} m, {2} tiles per side", &[&m.land_size().to_string(), &m.land_cell().to_string(), &m.tiles().to_string()])).color(theme::TEXT_DIM));
                                    },
                                );
                            });
                        }
                        ProjectPage::TileSurfaces => {
                            section(ui, &icons::with(icons::GRID_FOUR, &t("Tile grid")), true, |ui| {
                                row(ui, &t("Tile size"), |ui| {
                                    theme::segmented(ui, &mut m.tile_px, &[
                                        (256u32, "256".to_string(), String::new()),
                                        (512, "512".to_string(), String::new()),
                                        (1024, "1024".to_string(), String::new()),
                                        (2048, "2048".to_string(), String::new()),
                                    ]);
                                });
                                row(ui, &t("Surfaces per tile"), |ui| {
                                    theme::segmented(ui, &mut m.max_tile_surfaces, &[
                                        (1u8, "1".to_string(), String::new()),
                                        (2, "2".to_string(), String::new()),
                                        (3, "3".to_string(), String::new()),
                                        (4, "4".to_string(), String::new()),
                                        (5, "5".to_string(), String::new()),
                                        (6, "6".to_string(), String::new()),
                                    ]);
                                });
                                ui.add_space(2.0);
                                about(ui, &t("Changing this does not modify the mask; run Check tiles afterwards."));
                                about(ui, &t("When a tile really uses 6, the whole map is exported like Terrain Builder's 6-material mode (TerrainX), without satellite normal maps."));
                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    let check_btn = if tile_limit.needs_check {
                                        egui::Button::new(RichText::new(icons::with(icons::MAGNIFYING_GLASS, &t("Check tiles"))).color(Color32::WHITE))
                                            .fill(theme::ACCENT)
                                            .stroke(Stroke::NONE)
                                            .min_size(egui::vec2(0.0, 24.0))
                                    } else {
                                        egui::Button::new(icons::with(icons::MAGNIFYING_GLASS, &t("Check tiles")))
                                            .min_size(egui::vec2(0.0, 24.0))
                                    };
                                    if ui.add(check_btn).clicked() {
                                        check = true;
                                    }
                                    if !tile_limit.issues.is_empty() {
                                        let on = tile_limit.show_pixels;
                                        let eye_btn = egui::Button::new(
                                            RichText::new(icons::with(icons::EYE, &t("Highlight excess pixels")))
                                                .color(if on { theme::TOGGLE_ON_TEXT } else { theme::TEXT }),
                                        )
                                        .fill(if on { theme::TOGGLE_ON } else { theme::SECTION })
                                        .min_size(egui::vec2(0.0, 24.0));
                                        if ui.add(eye_btn).clicked() {
                                            tile_limit.show_pixels = !tile_limit.show_pixels;
                                        }
                                        if ui.add(egui::Button::new(icons::with(icons::X, &t("Clear marks"))).min_size(egui::vec2(0.0, 24.0))).clicked() {
                                            clear_marks = true;
                                        }
                                    }
                                });
                                // Status banner card
                                let banner: Option<(Color32, &str, String)> = if tile_limit.needs_check {
                                    Some((theme::WARN, icons::WARNING, t("Surface limit changed. Run Check tiles to find tiles over the limit.")))
                                } else if !tile_limit.issues.is_empty() {
                                    let total_px: u64 = tile_limit.issues.iter().map(|i| i.excess_pixels as u64).sum();
                                    Some((theme::ERROR, icons::WARNING_CIRCLE, tf("{0} tiles over the limit ({1} excess pixels)", &[&tile_limit.issues.len().to_string(), &total_px.to_string()])))
                                } else if let Some(rep) = tile_limit.last_report {
                                    if rep.remaining_over == 0 {
                                        Some((theme::OK, icons::CHECK_CIRCLE, tf("Fixed {0} tiles ({1} pixels changed).", &[&rep.tiles_fixed.to_string(), &rep.pixels_changed.to_string()])))
                                    } else {
                                        Some((theme::WARN, icons::WARNING, tf("Fixed {0} tiles ({1} pixels changed), {2} tiles still over the limit.", &[&rep.tiles_fixed.to_string(), &rep.pixels_changed.to_string(), &rep.remaining_over.to_string()])))
                                    }
                                } else {
                                    None
                                };
                                if let Some((col, icon, msg)) = banner {
                                    ui.add_space(4.0);
                                    fit_frame(
                                        ui,
                                        egui::Frame::new()
                                            .fill(theme::DARK)
                                            .stroke(Stroke::new(1.0_f32, col.gamma_multiply(0.7)))
                                            .corner_radius(4)
                                            .inner_margin(egui::Margin::symmetric(8, 6)),
                                        |ui| {
                                            ui.horizontal_wrapped(|ui| {
                                                ui.colored_label(col, icon);
                                                ui.colored_label(col, msg);
                                            });
                                        },
                                    );
                                }
                            });

                            if !tile_limit.issues.is_empty() {
                                let limit = m.max_tile_surfaces.max(1) as usize;
                                section(ui, &icons::with(icons::LIST_CHECKS, &t("Tiles over the limit")), true, |ui| {
                                    let avail_w = ui.available_width();
                                    let left_w = (avail_w * 0.42).clamp(170.0, 230.0);
                                    let right_w = (avail_w - left_w - 8.0).max(200.0);
                                    ui.horizontal_top(|ui| {
                                        // Left pane: scrollable tile list with alternating rows.
                                        egui::Frame::new()
                                            .fill(theme::DARK)
                                            .stroke(Stroke::new(1.0_f32, theme::BORDER))
                                            .corner_radius(4)
                                            .inner_margin(egui::Margin::same(4))
                                            .show(ui, |ui| {
                                                ui.set_width(left_w);
                                                egui::ScrollArea::vertical().id_salt("tile_issues_list").max_height(190.0).auto_shrink([false, false]).show(ui, |ui| {
                                                    ui.spacing_mut().item_spacing.y = 1.0;
                                                    for (idx, issue) in tile_limit.issues.iter().enumerate() {
                                                        let is_sel = tile_limit.selected == Some((issue.col, issue.row));
                                                        let bg = if is_sel { theme::ACCENT_SOFT } else if idx % 2 == 1 { theme::ROW_ALT } else { Color32::TRANSPARENT };
                                                        fit_frame(ui, egui::Frame::new().fill(bg).corner_radius(2).inner_margin(egui::Margin::symmetric(4, 2)), |ui| {
                                                            ui.horizontal(|ui| {
                                                                if ui.add(egui::Button::new(RichText::new(icons::CROSSHAIR).color(if is_sel { Color32::WHITE } else { theme::TEXT_DIM })).frame(false))
                                                                    .on_hover_text(t("Go to the excess pixels on this tile"))
                                                                    .clicked()
                                                                {
                                                                    fly_target = Some((issue.col, issue.row, issue.focus_px));
                                                                }
                                                                let name = format!("{:03}_{:03}", issue.col, issue.row);
                                                                let r = ui.add(egui::Label::new(RichText::new(name).monospace().color(if is_sel { Color32::WHITE } else { theme::TEXT_BRIGHT })).sense(egui::Sense::click()));
                                                                if r.clicked() {
                                                                    tile_limit.selected = Some((issue.col, issue.row));
                                                                }
                                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                                    let right_txt = format!("({}/{})  {} px", issue.surfaces.len(), limit, issue.excess_pixels);
                                                                    let r2 = ui.add(egui::Label::new(RichText::new(right_txt).small().monospace().color(if is_sel { Color32::from_rgb(190, 215, 245) } else { theme::TEXT_DIM })).sense(egui::Sense::click()));
                                                                    if r2.clicked() {
                                                                        tile_limit.selected = Some((issue.col, issue.row));
                                                                    }
                                                                });
                                                            });
                                                        });
                                                    }
                                                });
                                            });
                                        // Right pane: surfaces on the selected tile.
                                        egui::Frame::new()
                                            .fill(theme::DARK)
                                            .stroke(Stroke::new(1.0_f32, theme::BORDER))
                                            .corner_radius(4)
                                            .inner_margin(egui::Margin::same(6))
                                            .show(ui, |ui| {
                                                ui.set_width(right_w);
                                                let chosen_issue = tile_limit.selected
                                                    .and_then(|(sc, sr)| tile_limit.issues.iter().find(|i| i.col == sc && i.row == sr))
                                                    .or_else(|| tile_limit.issues.first())
                                                    .cloned();
                                                if let Some(issue) = chosen_issue {
                                                    let tile_name = format!("{:03}_{:03}", issue.col, issue.row);
                                                    ui.label(RichText::new(tf("Tile {0} surfaces", &[&tile_name])).font(icons::strong(12.5)).color(theme::TEXT_BRIGHT));
                                                    ui.add_space(4.0);
                                                    egui::ScrollArea::vertical().id_salt("tile_selected_surfaces").max_height(166.0).auto_shrink([false, false]).show(ui, |ui| {
                                                        ui.spacing_mut().item_spacing.y = 1.0;
                                                        for (idx, &(layer, count)) in issue.surfaces.iter().enumerate() {
                                                            let (lname, lcol) = layers.get(layer as usize).map(|(n, c)| (n.as_str(), *c)).unwrap_or(("?", [128, 128, 128]));
                                                            let thumb = layer_pics.get(layer as usize).copied().flatten();
                                                            let is_excess = issue.excess_layers.contains(&layer);
                                                            let is_prot = tile_limit.options.protected.contains(&layer);
                                                            let is_rem = tile_limit.options.remove_first.contains(&layer);
                                                            let bg = if idx % 2 == 1 { theme::ROW_ALT } else { Color32::TRANSPARENT };
                                                            fit_frame(ui, egui::Frame::new().fill(bg).corner_radius(2).inner_margin(egui::Margin::symmetric(4, 3)), |ui| {
                                                                ui.horizontal(|ui| {
                                                                    draw_surface_swatch(ui, 16.0, thumb, lcol);
                                                                    ui.add(egui::Label::new(RichText::new(lname).color(if is_excess { theme::TEXT_BRIGHT } else { theme::TEXT })).truncate());
                                                                    ui.label(RichText::new(format!("{count} px")).small().monospace().color(theme::TEXT_DIM));
                                                                    if is_excess {
                                                                        ui.colored_label(theme::ERROR, RichText::new(t("to remove")).small());
                                                                    } else if is_prot {
                                                                        ui.colored_label(theme::OK, RichText::new(t("protected")).small());
                                                                    }
                                                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                                        let rem_btn = egui::Button::new(RichText::new(icons::TRASH).color(if is_rem { Color32::WHITE } else { theme::TEXT_DIM }))
                                                                            .fill(if is_rem { theme::ERROR } else { theme::SECTION })
                                                                            .min_size(egui::vec2(22.0, 20.0));
                                                                        if ui.add(rem_btn).on_hover_text(t("Remove first")).clicked() {
                                                                            tile_limit.options.protected.retain(|&x| x != layer);
                                                                            if is_rem { tile_limit.options.remove_first.retain(|&x| x != layer); } else { tile_limit.options.remove_first.push(layer); }
                                                                        }
                                                                        let prot_btn = egui::Button::new(RichText::new(icons::SHIELD_CHECK).color(if is_prot { Color32::WHITE } else { theme::TEXT_DIM }))
                                                                            .fill(if is_prot { theme::OK } else { theme::SECTION })
                                                                            .min_size(egui::vec2(22.0, 20.0));
                                                                        if ui.add(prot_btn).on_hover_text(t("Protect")).clicked() {
                                                                            tile_limit.options.remove_first.retain(|&x| x != layer);
                                                                            if is_prot { tile_limit.options.protected.retain(|&x| x != layer); } else { tile_limit.options.protected.push(layer); }
                                                                        }
                                                                    });
                                                                });
                                                            });
                                                        }
                                                    });
                                                }
                                            });
                                    });
                                });
                            }

                            section(ui, &icons::with(icons::WRENCH, &t("Fix options")), true, |ui| {
                                let mut replace_kind = match tile_limit.options.replace {
                                    ReplaceMode::Nearest => 0u8,
                                    ReplaceMode::MostUsed => 1,
                                    ReplaceMode::Layer(_) => 2,
                                };
                                row(ui, &t("Replace with"), |ui| {
                                    if theme::segmented(ui, &mut replace_kind, &[
                                        (0, t("Nearest"), t("Nearest kept surface")),
                                        (1, t("Most used"), t("Most used surface on tile")),
                                        (2, t("Chosen surface"), String::new()),
                                    ]) {
                                        tile_limit.options.replace = match replace_kind {
                                            0 => ReplaceMode::Nearest,
                                            1 => ReplaceMode::MostUsed,
                                            _ => ReplaceMode::Layer(match tile_limit.options.replace { ReplaceMode::Layer(l) => l, _ => 0 }),
                                        };
                                    }
                                });
                                if let ReplaceMode::Layer(mut chosen) = tile_limit.options.replace {
                                    row(ui, &t("Target surface"), |ui| {
                                        let mut idx = chosen as usize;
                                        let w = ui.available_width();
                                        if theme::surface_pick(ui, "fix_target_surface", &mut idx, None, &layer_names, &layer_pics, |i, _| i, w) {
                                            chosen = idx as u8;
                                        }
                                    });
                                    tile_limit.options.replace = ReplaceMode::Layer(chosen);
                                }
                                let mut is_single = matches!(tile_limit.options.scope, FixScope::SingleTile(_, _));
                                row(ui, &t("Fix scope"), |ui| {
                                    if theme::segmented(ui, &mut is_single, &[
                                        (false, t("All tiles over the limit"), String::new()),
                                        (true, t("Selected tile only"), String::new()),
                                    ]) {
                                        tile_limit.options.scope = if is_single {
                                            let (c, r) = tile_limit.selected.unwrap_or((0, 0));
                                            FixScope::SingleTile(c, r)
                                        } else {
                                            FixScope::AllOverLimit
                                        };
                                    }
                                });
                                if matches!(tile_limit.options.scope, FixScope::SingleTile(_, _)) {
                                    if let Some((c, r)) = tile_limit.selected {
                                        tile_limit.options.scope = FixScope::SingleTile(c, r);
                                    }
                                }
                                ui.add_space(4.0);
                                theme::subsection(ui, &t("Surface priority"), false, |ui| {
                                    about(ui, &t("Protect surfaces you want to keep, or mark surfaces to remove first."));
                                    ui.add_space(2.0);
                                    fit_frame(
                                        ui,
                                        egui::Frame::new()
                                            .fill(theme::DARK)
                                            .stroke(Stroke::new(1.0_f32, theme::BORDER))
                                            .corner_radius(4)
                                            .inner_margin(egui::Margin::same(4)),
                                        |ui| {
                                            egui::ScrollArea::vertical().id_salt("fix_surface_priority").max_height(160.0).auto_shrink([false, false]).show(ui, |ui| {
                                                ui.spacing_mut().item_spacing.y = 1.0;
                                                for (idx, (lname, lcol)) in layers.iter().enumerate() {
                                                    let id = idx as u8;
                                                    let thumb = layer_pics.get(idx).copied().flatten();
                                                    let mut rule = if tile_limit.options.protected.contains(&id) {
                                                        LayerRule::Protect
                                                    } else if tile_limit.options.remove_first.contains(&id) {
                                                        LayerRule::RemoveFirst
                                                    } else {
                                                        LayerRule::Auto
                                                    };
                                                    let bg = if idx % 2 == 1 { theme::ROW_ALT } else { Color32::TRANSPARENT };
                                                    fit_frame(ui, egui::Frame::new().fill(bg).corner_radius(2).inner_margin(egui::Margin::symmetric(6, 2)), |ui| {
                                                        ui.horizontal(|ui| {
                                                            draw_surface_swatch(ui, 16.0, thumb, *lcol);
                                                            ui.add(egui::Label::new(lname).truncate());
                                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                                if theme::segmented(ui, &mut rule, &[
                                                                    (LayerRule::Auto, t("Auto"), String::new()),
                                                                    (LayerRule::Protect, t("Protect"), String::new()),
                                                                    (LayerRule::RemoveFirst, t("Remove first"), String::new()),
                                                                ]) {
                                                                    tile_limit.options.protected.retain(|&x| x != id);
                                                                    tile_limit.options.remove_first.retain(|&x| x != id);
                                                                    match rule {
                                                                        LayerRule::Protect => tile_limit.options.protected.push(id),
                                                                        LayerRule::RemoveFirst => tile_limit.options.remove_first.push(id),
                                                                        LayerRule::Auto => {}
                                                                    }
                                                                }
                                                            });
                                                        });
                                                    });
                                                }
                                            });
                                        });
                                });
                                ui.add_space(6.0);
                                let single_ready = !matches!(tile_limit.options.scope, FixScope::SingleTile(_, _)) || tile_limit.selected.is_some();
                                let preview_px: u64 = match tile_limit.options.scope {
                                    FixScope::AllOverLimit => tile_limit.issues.iter().map(|i| i.excess_pixels as u64).sum(),
                                    FixScope::SingleTile(c, r) => tile_limit.issues.iter().find(|i| i.col == c && i.row == r).map_or(0, |i| i.excess_pixels as u64),
                                };
                                let fix_label = if tile_limit.issues.is_empty() {
                                    t("Fix tiles")
                                } else if matches!(tile_limit.options.scope, FixScope::SingleTile(_, _)) {
                                    tf("Fix selected tile ({0} px)", &[&preview_px.to_string()])
                                } else {
                                    tf("Fix tiles ({0} px)", &[&preview_px.to_string()])
                                };
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let btn = egui::Button::new(
                                        RichText::new(icons::with(icons::WRENCH, &fix_label))
                                            .color(if single_ready { Color32::WHITE } else { theme::TEXT_DIM }),
                                    )
                                    .fill(if single_ready { theme::ACCENT } else { theme::SECTION })
                                    .stroke(Stroke::NONE)
                                    .min_size(egui::vec2(140.0, 26.0));
                                    let resp = ui.add_enabled(single_ready, btn);
                                    if !single_ready {
                                        resp.on_disabled_hover_text(t("Choose a tile from the list above first."));
                                    } else if resp.clicked() {
                                        fix = true;
                                    }
                                });
                            });
                        }
                    }
                });
            });
        });
    });
    ctx.data_mut(|d| d.insert_temp(page_id, page));

    let limit_changed = doc.world.meta.max_tile_surfaces != before.max_tile_surfaces || doc.world.meta.tile_px != before.tile_px;
    if doc.world.meta != before {
        doc.history.revision += 1;
    }
    if limit_changed {
        // Changing the surface limit must NOT alter mask pixels; prompt the user to run Check tiles.
        tile_limit.issues.clear();
        tile_limit.selected = None;
        tile_limit.last_report = None;
        tile_limit.needs_check = true;
    } else if clear_marks {
        tile_limit.issues.clear();
        tile_limit.selected = None;
        tile_limit.last_report = None;
    } else if !tile_limit.issues.is_empty() && tile_limit.options != opts_before {
        // Live-update excess pixel highlights whenever the user changes fix options.
        tile_limit.issues = dz_core::mask_limit::scan_detailed(&doc.world, &tile_limit.options);
    }
    app.tile_limit = tile_limit;
    if limit_changed {
        app.info(t("Surface limit changed. Run Check tiles to find tiles over the limit."));
    }
    if open_config {
        app.dialog = Dialog::MapConfig;
        return;
    }
    if let Some((col, row, focus_px)) = fly_target {
        app.tile_limit.selected = Some((col, row));
        let wx = (focus_px[0] / mask_w * size_m).clamp(0.0, size_m);
        let wz = ((1.0 - focus_px[1] / mask_h) * size_m).clamp(0.0, size_m);
        crate::goto::fly_to(app, dz_core::goto::Spot { x: wx, z: wz, radius: 60.0 });
    }
    if check {
        let Some(doc) = &app.doc else { return };
        app.tile_limit.needs_check = false;
        app.tile_limit.last_report = None;
        app.tile_limit.issues = dz_core::mask_limit::scan_detailed(&doc.world, &app.tile_limit.options);
        if let Some(sel) = app.tile_limit.selected {
            if !app.tile_limit.issues.iter().any(|i| (i.col, i.row) == sel) {
                app.tile_limit.selected = app.tile_limit.issues.first().map(|i| (i.col, i.row));
            }
        } else {
            app.tile_limit.selected = app.tile_limit.issues.first().map(|i| (i.col, i.row));
        }
        let n = app.tile_limit.issues.len();
        if n == 0 {
            app.info(t("All tiles are within the surface limit."));
        } else {
            let total_px: u64 = app.tile_limit.issues.iter().map(|i| i.excess_pixels as u64).sum();
            app.info(tf("{0} tiles over the limit ({1} excess pixels)", &[&n.to_string(), &total_px.to_string()]));
        }
    }
    if fix {
        let Some(doc) = &app.doc else { return };
        let opts = app.tile_limit.options.clone();
        let mut w = doc.world.lite();
        let report = dz_core::mask_limit::enforce_with(&mut w, &opts);
        let data = w.mask.data;
        if report.pixels_changed > 0 {
            app.apply_mask(&t("Fix tiles"), data);
        }
        app.tile_limit.needs_check = false;
        app.tile_limit.last_report = Some(report);
        if let Some(doc) = &app.doc {
            app.tile_limit.issues = dz_core::mask_limit::scan_detailed(&doc.world, &app.tile_limit.options);
            if let Some(sel) = app.tile_limit.selected {
                if !app.tile_limit.issues.iter().any(|i| (i.col, i.row) == sel) {
                    app.tile_limit.selected = app.tile_limit.issues.first().map(|i| (i.col, i.row));
                }
            }
        }
        if report.remaining_over == 0 {
            app.info(tf("Fixed {0} tiles ({1} pixels changed).", &[&report.tiles_fixed.to_string(), &report.pixels_changed.to_string()]));
        } else {
            app.info(tf("Fixed {0} tiles ({1} pixels changed), {2} tiles still over the limit.", &[&report.tiles_fixed.to_string(), &report.pixels_changed.to_string(), &report.remaining_over.to_string()]));
        }
    }
}
