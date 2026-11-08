//! Window layout modelled on the Unreal editor: menu and toolbar on top, modes on the left,
//! outliner and details on the right, content browser and log at the bottom.
use crate::app::*;
use crate::i18n::{t, tf};
use crate::theme::{self, row, section, tool_button};
use crate::viewport;
use dz_core::edit::SculptTool;
use dz_core::gen;
use dz_core::objects::{FLAG_ABSOLUTE, FLAG_UNDERGROUND};
use dz_core::world::{ModMode, ModShape, SurfaceLayer};
use eframe::egui::{self, vec2, Color32, RichText};

pub fn new_project_dialog() -> Dialog {
    Dialog::NewProject(NewProject { name: "NewMap".into(), preset: 4, base: 60.0, folder: String::new() })
}

pub fn ui(app: &mut App, ctx: &egui::Context) {
    guard_close(app, ctx);
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
            ui.menu_button(t("File"), |ui| {
                if ui.button(t("New project...")).clicked() { app.dialog = new_project_dialog(); ui.close_menu(); }
                if ui.button(t("Open project...")).clicked() { ui.close_menu(); app.pick_and_open(); }
                ui.menu_button(t("Recent projects"), |ui| {
                    for d in app.settings.recent.clone() {
                        if ui.button(d.display().to_string()).clicked() { ui.close_menu(); app.open_project(d); }
                    }
                });
                ui.separator();
                if ui.add_enabled(has, egui::Button::new(t("Save")).shortcut_text("Ctrl+S")).clicked() { ui.close_menu(); app.save(None); }
                if ui.add_enabled(has, egui::Button::new(t("Save as..."))).clicked() { ui.close_menu(); app.save_as(); }
                ui.separator();
                ui.menu_button(t("Import"), |ui| {
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
                    });
                });
                ui.separator();
                if ui.button(t("Project settings...")).clicked() { app.dialog = Dialog::ProjectSettings; ui.close_menu(); }
                if ui.button(t("Exit")).clicked() { ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
            });
            ui.menu_button(t("Edit"), |ui| {
                let (u, r) = app.doc.as_ref().map(|d| (d.history.undo_label().map(str::to_string), d.history.redo_label().map(str::to_string))).unwrap_or((None, None));
                if ui.add_enabled(u.is_some(), egui::Button::new(format!("{} {}", t("Undo"), u.unwrap_or_default())).shortcut_text("Ctrl+Z")).clicked() { app.undo(); ui.close_menu(); }
                if ui.add_enabled(r.is_some(), egui::Button::new(format!("{} {}", t("Redo"), r.unwrap_or_default())).shortcut_text("Ctrl+Y")).clicked() { app.redo(); ui.close_menu(); }
                ui.separator();
                if ui.add(egui::Button::new(t("Duplicate")).shortcut_text("Ctrl+D")).clicked() { app.duplicate_selection(); ui.close_menu(); }
                if ui.add(egui::Button::new(t("Delete")).shortcut_text("Del")).clicked() { app.delete_selection(); ui.close_menu(); }
                ui.separator();
                if ui.button(t("Settings...")).clicked() { app.dialog = Dialog::Settings; ui.close_menu(); }
            });
            ui.menu_button(t("View"), |ui| {
                ui.checkbox(&mut app.show_objects, t("Objects"));
                ui.checkbox(&mut app.show_grass, t("Grass"));
                ui.checkbox(&mut app.show_shadows, t("Shadows"));
                ui.checkbox(&mut app.show_water, t("Water"));
                ui.checkbox(&mut app.show_lines, t("Roads and lines"));
                ui.separator();
                ui.checkbox(&mut app.show_land_grid, t("Land grid"));
                ui.checkbox(&mut app.show_tile_grid, t("Texture tiles"));
                ui.checkbox(&mut app.show_contours, t("Contour lines"));
            });
            ui.menu_button(t("Build"), |ui| {
                ui.add_enabled_ui(has, |ui| {
                    if ui.button(t("Export files for the PBO")).clicked() { ui.close_menu(); app.export_map(false); }
                    if ui.button(t("Export with Terrain Builder sources")).clicked() { ui.close_menu(); app.export_map(true); }
                    if ui.button(t("Build PBO")).clicked() { ui.close_menu(); app.build(false); }
                    if ui.button(t("Build and play")).clicked() { ui.close_menu(); app.build(true); }
                    ui.separator();
                    if ui.button(t("Generate navmesh...")).clicked() { ui.close_menu(); app.generate_navmesh(); }
                });
            });
            ui.menu_button(t("Language"), |ui| {
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
            ui.menu_button(t("Help"), |ui| {
                if ui.button(t("About")).clicked() { app.dialog = Dialog::About; ui.close_menu(); }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                app.updater.bar(ui);
            });
        });
    });
}

fn toolbar(app: &mut App, ctx: &egui::Context) {
    egui::TopBottomPanel::top("toolbar").frame(egui::Frame::new().fill(theme::PANEL).inner_margin(egui::Margin::symmetric(8, 4))).show(ctx, |ui| {
        ui.horizontal(|ui| {
            if ui.button(t("Save")).on_hover_text("Ctrl+S").clicked() { app.save(None); }
            ui.separator();
            for (m, name, key) in [(Mode::Select, "Select", "1"), (Mode::Landscape, "Sculpt", "2"), (Mode::Paint, "Paint", "3"), (Mode::Place, "Place", "4"), (Mode::Lines, "Roads", "5"), (Mode::Generate, "Generate", "6")] {
                if tool_button(ui, app.mode == m, &t(name)).on_hover_text(key).clicked() { app.mode = m; }
            }
            ui.separator();
            ui.add_enabled_ui(app.mode == Mode::Select, |ui| {
                for (g, name, key) in [(Gizmo::Move, "Move", "W"), (Gizmo::Rotate, "Rotate", "E"), (Gizmo::Scale, "Scale", "R")] {
                    if tool_button(ui, app.gizmo == g, &t(name)).on_hover_text(key).clicked() { app.gizmo = g; }
                }
            });
            ui.separator();
            for (v, name) in [(ViewMode::Perspective, "3D"), (ViewMode::Top, "2D"), (ViewMode::Split, "3D | 2D")] {
                if tool_button(ui, app.view_mode == v, name).clicked() { app.view_mode = v; }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let has = app.doc.is_some() && app.task.is_none();
                if app.session.is_some() {
                    if ui.add(egui::Button::new(RichText::new(format!("■ {}", t("Stop"))).color(Color32::WHITE)).fill(theme::ERROR)).clicked() { app.stop_session(); }
                } else if ui.add_enabled(has, egui::Button::new(RichText::new(format!("▶ {}", t("Play"))).color(Color32::WHITE)).fill(theme::PLAY)).on_hover_text(t("Export, binarise and start the map in DayZ")).clicked() {
                    app.build(true);
                }
                if ui.add_enabled(has, egui::Button::new(t("Build"))).on_hover_text(t("Export and binarise the PBO")).clicked() { app.build(false); }
            });
        });
    });
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
        Mode::Generate => generate_mode(app, ui),
    });
}

fn select_mode(app: &mut App, ui: &mut egui::Ui) {
    section(ui, &t("Selection"), true, |ui| {
        ui.label(tf("{0} objects selected", &[&app.selection.len().to_string()]));
        ui.weak(t("Click to select, drag for a box, Shift adds. W, E, R change the gizmo. F frames the selection."));
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button(t("Duplicate")).clicked() { app.duplicate_selection(); }
            if ui.button(t("Delete")).clicked() { app.delete_selection(); }
        });
    });
    section(ui, &t("Terrain under objects"), true, |ui| {
        ui.weak(t("A modifier levels the ground under the object and follows it. The sculpted terrain stays untouched."));
        slider(ui, &t("Blend"), &mut app.place.pad_falloff, 0.0..=60.0, false);
        if ui.add_enabled(!app.selection.is_empty(), egui::Button::new(t("Add modifier to selection"))).clicked() {
            let f = app.place.pad_falloff;
            app.add_pads_for_selection(f);
        }
    });
}

fn sculpt_mode(app: &mut App, ui: &mut egui::Ui) {
    section(ui, &t("Sculpt tool"), true, |ui| {
        ui.horizontal_wrapped(|ui| {
            for (tool, name) in [(SculptTool::Raise, "Raise"), (SculptTool::Lower, "Lower"), (SculptTool::Smooth, "Smooth"), (SculptTool::Flatten, "Flatten"), (SculptTool::Noise, "Noise"), (SculptTool::Ramp, "Ramp")] {
                if tool_button(ui, app.sculpt.tool == tool, &t(name)).clicked() { app.sculpt.tool = tool; }
            }
        });
        ui.weak(t("Ctrl inverts raise and lower, Shift smooths. [ and ] or Shift+wheel change the size."));
    });
    section(ui, &t("Brush"), true, |ui| {
        slider(ui, &t("Radius"), &mut app.sculpt.radius, 2.0..=3000.0, true);
        slider(ui, &t("Falloff"), &mut app.sculpt.falloff, 0.0..=1.0, false);
        slider(ui, &t("Strength"), &mut app.sculpt.strength, 0.5..=200.0, true);
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
    section(ui, &t("Brush"), true, |ui| {
        slider(ui, &t("Radius"), &mut app.paint.radius, 0.5..=2000.0, true);
        slider(ui, &t("Edge noise"), &mut app.paint.scatter, 0.0..=0.8, false);
    });
    section(ui, &t("Surface layers"), true, |ui| {
        layer_list(app, ui);
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            let surfaces = app.surfaces.clone();
            ui.menu_button(format!("+ {}", t("Add layer")), |ui| {
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
    ui.weak(t("The game blends at most five surfaces per texture tile; if a tile uses more, the rarest are replaced on export."));
}

fn place_mode(app: &mut App, ui: &mut egui::Ui) {
    section(ui, &t("Place objects"), true, |ui| {
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
    section(ui, &t("Terrain under objects"), true, |ui| {
        ui.checkbox(&mut app.place.auto_pad, t("Level the ground when placing"));
        slider(ui, &t("Blend"), &mut app.place.pad_falloff, 0.0..=60.0, false);
    });
}

fn lines_mode(app: &mut App, ui: &mut egui::Ui) {
    section(ui, &t("Draw"), true, |ui| {
        ui.horizontal(|ui| {
            for (a, n) in [(LineAction::Road, "Road"), (LineAction::Fence, "Objects"), (LineAction::Plain, "Line")] {
                if tool_button(ui, app.line.action == a, &t(n)).clicked() { app.line.action = a; }
            }
        });
        ui.weak(t("Click to add points. Enter finishes, Backspace removes the last point, Esc cancels."));
        ui.label(tf("{0} points", &[&app.line.points.len().to_string()]));
        match app.line.action {
            LineAction::Road => {
                let families = app.road_families.clone();
                row(ui, &t("Road type"), |ui| {
                    egui::ComboBox::from_id_salt("family").selected_text(app.line.family.clone()).show_ui(ui, |ui| {
                        for f in families { ui.selectable_value(&mut app.line.family, f.clone(), f); }
                    });
                });
                slider(ui, &t("Minimum radius"), &mut app.line.radius, 10.0..=200.0, false);
                ui.checkbox(&mut app.line.grade, t("Grade the terrain under the road"));
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

pub fn run_scatter(app: &mut App, at: Option<(f32, f32)>) {
    let Some(doc) = &app.doc else { return };
    let mut lite = doc.world.lite();
    let rule = app.gen.scatter.clone();
    let area = match at { Some((x, z)) => gen::Area::circle(x, z, app.gen.scatter_radius.max(10.0)), None => gen::Area::whole(&lite) };
    app.spawn(t("Scattering objects"), move |job| {
        let objects = gen::scatter(&mut lite, &rule, area, job)?;
        let models: Vec<String> = lite.objects.models.iter().map(|m| m.path.clone()).collect();
        Ok(Box::new(move |a: &mut App| {
            let n = objects.len();
            a.merge_generated(&t("Scatter objects"), models, objects);
            a.info(tf("Placed {0} objects", &[&n.to_string()]));
        }) as Finish)
    });
}

pub fn run_settlement(app: &mut App, x: f32, z: f32) {
    let Some(doc) = &app.doc else { return };
    let mut lite = doc.world.lite();
    let rule = app.gen.settlement.clone();
    let db = app.db.clone();
    let falloff = app.place.pad_falloff;
    app.spawn(t("Generating settlement"), move |_| {
        let s = gen::settlement(&mut lite, &db, x, z, &rule)?;
        let models: Vec<String> = lite.objects.models.iter().map(|m| m.path.clone()).collect();
        Ok(Box::new(move |a: &mut App| {
            let ids = a.merge_generated(&t("Generate settlement"), models, s.objects);
            if let Some(d) = &mut a.doc { d.world.lines.extend(s.streets); }
            let pads: Vec<u32> = s.pads.iter().map(|(i, _, _)| ids[*i]).collect();
            a.add_pads(&pads, falloff);
            if !pads.is_empty() {
                if let Some(d) = &mut a.doc {
                    d.history.group_last(2, t("Generate settlement"));
                }
            }
        }) as Finish)
    });
}

fn height_job(app: &mut App, title: String, f: impl FnOnce(&mut dz_core::terrain::Heightmap, &dz_core::Job) -> Vec<dz_core::world::MapLine> + Send + 'static) {
    let Some(doc) = &app.doc else { return };
    let mut base = doc.world.base.clone();
    let label = title.clone();
    app.spawn(title, move |job| {
        let lines = f(&mut base, job);
        job.check()?;
        Ok(Box::new(move |a: &mut App| a.apply_heights(&label, base.data, lines)) as Finish)
    });
}

fn generate_mode(app: &mut App, ui: &mut egui::Ui) {
    section(ui, &t("Terrain shape"), true, |ui| {
        let g = &mut app.gen.terrain;
        row(ui, &t("Type"), |ui| {
            egui::ComboBox::from_id_salt("kind").selected_text(t(g.kind.name())).show_ui(ui, |ui| {
                for k in gen::TerrainKind::ALL { ui.selectable_value(&mut g.kind, k, t(k.name())); }
            });
        });
        row(ui, &t("Seed"), |ui| { ui.add(egui::DragValue::new(&mut g.seed)); });
        slider(ui, &t("Base height"), &mut g.base, 0.0..=800.0, false);
        slider(ui, &t("Height range"), &mut g.amplitude, 1.0..=1500.0, true);
        slider(ui, &t("Feature size"), &mut g.feature, 100.0..=15000.0, true);
        row(ui, &t("Detail"), |ui| { ui.add(egui::Slider::new(&mut g.octaves, 1..=12)); });
        slider(ui, &t("Roughness"), &mut g.roughness, 0.2..=0.8, false);
        slider(ui, &t("Warp"), &mut g.warp, 0.0..=2000.0, false);
        slider(ui, &t("Blend"), &mut g.blend, 0.0..=1.0, false);
        if ui.button(t("Generate terrain")).clicked() {
            let p = app.gen.terrain.clone();
            height_job(app, t("Generate terrain"), move |h, job| { gen::generate_terrain(h, &p, job); Vec::new() });
        }
    });
    section(ui, &t("Erosion"), false, |ui| {
        let e = &mut app.gen.erosion;
        row(ui, &t("Droplets"), |ui| { ui.add(egui::Slider::new(&mut e.droplets, 10_000..=4_000_000).logarithmic(true)); });
        slider(ui, &t("Erosion rate"), &mut e.erode, 0.05..=0.9, false);
        slider(ui, &t("Deposition"), &mut e.deposit, 0.05..=0.9, false);
        if ui.button(t("Hydraulic erosion")).clicked() {
            let p = app.gen.erosion.clone();
            height_job(app, t("Hydraulic erosion"), move |h, job| { gen::hydraulic_erosion(h, &p, job); Vec::new() });
        }
        ui.separator();
        row(ui, &t("Passes"), |ui| { ui.add(egui::Slider::new(&mut app.gen.thermal_iters, 1..=200)); });
        slider(ui, &t("Slope limit"), &mut app.gen.thermal_talus, 10.0..=70.0, false);
        if ui.button(t("Thermal erosion")).clicked() {
            let (n, a) = (app.gen.thermal_iters, app.gen.thermal_talus);
            height_job(app, t("Thermal erosion"), move |h, job| { gen::thermal_erosion(h, n, a, job); Vec::new() });
        }
    });
    section(ui, &t("Terraces and rivers"), false, |ui| {
        slider(ui, &t("Step height"), &mut app.gen.terrace_step, 1.0..=80.0, false);
        slider(ui, &t("Sharpness"), &mut app.gen.terrace_sharp, 0.0..=1.0, false);
        if ui.button(t("Make terraces")).clicked() {
            let (s, k) = (app.gen.terrace_step, app.gen.terrace_sharp);
            height_job(app, t("Make terraces"), move |h, _| { gen::terrace(h, s, k); Vec::new() });
        }
        ui.separator();
        row(ui, &t("Rivers"), |ui| { ui.add(egui::Slider::new(&mut app.gen.rivers, 1..=24)); });
        slider(ui, &t("Width"), &mut app.gen.river_width, 2.0..=80.0, false);
        slider(ui, &t("Depth"), &mut app.gen.river_depth, 0.5..=20.0, false);
        if ui.button(t("Carve rivers")).clicked() {
            let (n, w, d, s) = (app.gen.rivers, app.gen.river_width, app.gen.river_depth, app.gen.terrain.seed);
            height_job(app, t("Carve rivers"), move |h, _| gen::carve_rivers(h, n, w, d, s));
        }
    });
    section(ui, &t("Surface painting"), false, |ui| {
        let names: Vec<String> = app.doc.as_ref().map(|d| d.world.layers.iter().map(|l| l.name.clone()).collect()).unwrap_or_default();
        let mut remove = None;
        for (i, r) in app.gen.paint_rules.iter_mut().enumerate() {
            ui.push_id(i, |ui| {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("layer").width(110.0).selected_text(names.get(r.layer).cloned().unwrap_or_default()).show_ui(ui, |ui| {
                        for (k, n) in names.iter().enumerate() { ui.selectable_value(&mut r.layer, k, n); }
                    });
                    if ui.small_button("✖").clicked() { remove = Some(i); }
                });
                ui.horizontal(|ui| {
                    ui.label("H");
                    ui.add(egui::DragValue::new(&mut r.min_height).speed(1.0));
                    ui.add(egui::DragValue::new(&mut r.max_height).speed(1.0));
                    ui.label("°");
                    ui.add(egui::DragValue::new(&mut r.min_slope).speed(0.5).range(0.0..=90.0));
                    ui.add(egui::DragValue::new(&mut r.max_slope).speed(0.5).range(0.0..=90.0));
                });
                ui.horizontal(|ui| {
                    ui.label("~");
                    ui.add(egui::DragValue::new(&mut r.noise_scale).speed(5.0).range(0.0..=5000.0).suffix(" m"));
                    ui.add(egui::Slider::new(&mut r.coverage, 0.0..=1.0).show_value(true));
                });
                ui.separator();
            });
        }
        if let Some(i) = remove { app.gen.paint_rules.remove(i); }
        ui.horizontal(|ui| {
            if ui.button(format!("+ {}", t("Add rule"))).clicked() {
                app.gen.paint_rules.push(gen::PaintRule { layer: 0, min_height: -1000.0, max_height: 9000.0, min_slope: 0.0, max_slope: 90.0, noise_scale: 0.0, coverage: 1.0 });
            }
            if ui.button(t("Paint surfaces")).clicked() {
                if let Some(doc) = &app.doc {
                    let (w, rules, seed) = (doc.world.lite(), app.gen.paint_rules.clone(), app.gen.terrain.seed);
                    let label = t("Paint surfaces");
                    let l2 = label.clone();
                    app.spawn(label, move |job| {
                        let data = gen::auto_paint(&w, &rules, seed, job);
                        Ok(Box::new(move |a: &mut App| a.apply_mask(&l2, data)) as Finish)
                    });
                }
            }
        });
    });
    section(ui, &t("Scatter objects"), false, |ui| {
        ui.weak(t("Models: pick in the Content Browser and press Add."));
        let mut remove = None;
        for (i, m) in app.gen.scatter.models.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(m.rsplit('\\').next().unwrap_or(m));
                if ui.small_button("✖").clicked() { remove = Some(i); }
            });
        }
        if let Some(i) = remove { app.gen.scatter.models.remove(i); }
        if ui.button(format!("+ {}", t("Add selected model"))).clicked() {
            if let Some(m) = &app.place.model { if !app.gen.scatter.models.contains(m) { app.gen.scatter.models.push(m.clone()); } }
        }
        let s = &mut app.gen.scatter;
        slider(ui, &t("Per hectare"), &mut s.density, 0.1..=600.0, true);
        slider(ui, &t("Minimum distance"), &mut s.min_distance, 0.5..=100.0, true);
        slider(ui, &t("Max slope"), &mut s.max_slope, 1.0..=90.0, false);
        slider(ui, &t("Min height"), &mut s.min_height, -50.0..=800.0, false);
        slider(ui, &t("Min scale"), &mut s.scale_min, 0.1..=4.0, false);
        slider(ui, &t("Max scale"), &mut s.scale_max, 0.1..=4.0, false);
        ui.checkbox(&mut s.align_to_slope, t("Align to slope"));
        let names: Vec<String> = app.doc.as_ref().map(|d| d.world.layers.iter().map(|l| l.name.clone()).collect()).unwrap_or_default();
        ui.collapsing(t("Only on surfaces"), |ui| {
            for (i, n) in names.iter().enumerate() {
                let mut on = app.gen.scatter.layers.contains(&i);
                if ui.checkbox(&mut on, n).changed() {
                    if on { app.gen.scatter.layers.push(i); } else { app.gen.scatter.layers.retain(|l| *l != i); }
                }
            }
        });
        slider(ui, &t("Area radius"), &mut app.gen.scatter_radius, 10.0..=4000.0, true);
        ui.horizontal(|ui| {
            if ui.button(t("Whole map")).clicked() { run_scatter(app, None); }
            if tool_button(ui, app.gen.pick_scatter, &t("Click on the map")).clicked() { app.gen.pick_scatter = !app.gen.pick_scatter; }
        });
    });
    section(ui, &t("Forest (DayZForests)"), false, |ui| {
        let names: Vec<String> = app.doc.as_ref().map(|d| d.world.layers.iter().map(|l| l.name.clone()).collect()).unwrap_or_default();
        let presets = gen::forest_preset_names();
        let mut remove = None;
        for (i, z) in app.gen.forest_zones.iter_mut().enumerate() {
            ui.push_id(("zone", i), |ui| {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("l").width(90.0).selected_text(names.get(z.0).cloned().unwrap_or_default()).show_ui(ui, |ui| {
                        for (k, n) in names.iter().enumerate() { ui.selectable_value(&mut z.0, k, n); }
                    });
                    egui::ComboBox::from_id_salt("p").width(120.0).selected_text(z.1.clone()).show_ui(ui, |ui| {
                        for p in &presets { ui.selectable_value(&mut z.1, p.clone(), p); }
                    });
                    if ui.small_button("✖").clicked() { remove = Some(i); }
                });
            });
        }
        if let Some(i) = remove { app.gen.forest_zones.remove(i); }
        if ui.button(format!("+ {}", t("Add zone"))).clicked() { app.gen.forest_zones.push((0, presets.first().cloned().unwrap_or_default())); }
        slider(ui, &t("Max slope"), &mut app.gen.forest_slope, 5.0..=70.0, false);
        row(ui, &t("Clearings"), |ui| { ui.add(egui::Slider::new(&mut app.gen.forest_clearing, 0.0..=0.95)); });
        row(ui, &t("Seed"), |ui| { ui.add(egui::DragValue::new(&mut app.gen.forest_seed)); });
        if ui.button(t("Generate forest")).clicked() {
            if let Some(doc) = &app.doc {
                let mut lite = doc.world.lite();
                let req = gen::ForestRequest { zones: app.gen.forest_zones.clone(), seed: app.gen.forest_seed, max_slope: app.gen.forest_slope, clearing_strength: app.gen.forest_clearing };
                let assets = app.assets.clone();
                app.spawn(t("Generating forest"), move |job| {
                    let (objects, notes) = gen::generate_forest(&mut lite, &assets, &req, job)?;
                    let models: Vec<String> = lite.objects.models.iter().map(|m| m.path.clone()).collect();
                    Ok(Box::new(move |a: &mut App| {
                        a.merge_generated(&t("Generate forest"), models, objects);
                        for n in notes { a.info(n); }
                    }) as Finish)
                });
            }
        }
    });
    section(ui, &t("Settlement"), false, |ui| {
        ui.weak(t("Buildings: pick in the Content Browser and press Add."));
        let mut remove = None;
        for (i, m) in app.gen.settlement.buildings.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(m.rsplit('\\').next().unwrap_or(m));
                if ui.small_button("✖").clicked() { remove = Some(i); }
            });
        }
        if let Some(i) = remove { app.gen.settlement.buildings.remove(i); }
        if ui.button(format!("+ {}", t("Add selected model"))).clicked() {
            if let Some(m) = &app.place.model { if !app.gen.settlement.buildings.contains(m) { app.gen.settlement.buildings.push(m.clone()); } }
        }
        let s = &mut app.gen.settlement;
        slider(ui, &t("Radius"), &mut s.radius, 40.0..=1500.0, true);
        row(ui, &t("Streets"), |ui| { ui.add(egui::Slider::new(&mut s.streets, 1..=12)); });
        slider(ui, &t("Plot size"), &mut s.plot, 10.0..=80.0, false);
        slider(ui, &t("Distance from street"), &mut s.setback, 2.0..=40.0, false);
        slider(ui, &t("Max slope"), &mut s.max_slope, 2.0..=45.0, false);
        ui.checkbox(&mut s.flatten, t("Level the ground under buildings"));
        if tool_button(ui, app.gen.pick_settlement, &t("Click on the map")).clicked() { app.gen.pick_settlement = !app.gen.pick_settlement; }
    });
}

fn right_panel(app: &mut App, ui: &mut egui::Ui) {
    let h = ui.available_height();
    egui::TopBottomPanel::top("outliner").resizable(true).default_height(h * 0.42).height_range(120.0..=h - 160.0).frame(egui::Frame::new()).show_inside(ui, |ui| outliner(app, ui));
    egui::CentralPanel::default().frame(egui::Frame::new()).show_inside(ui, |ui| details(app, ui));
}

fn panel_title(ui: &mut egui::Ui, title: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::PANEL);
    ui.painter().text(egui::pos2(rect.left() + 8.0, rect.center().y), egui::Align2::LEFT_CENTER, title, egui::FontId::proportional(13.0), Color32::WHITE);
}

fn outliner(app: &mut App, ui: &mut egui::Ui) {
    panel_title(ui, &t("Outliner"));
    ui.add(egui::TextEdit::singleline(&mut app.outliner_filter).hint_text(t("Search...")).desired_width(f32::INFINITY));
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
    let lines: Vec<(usize, String, usize)> = doc.world.lines.iter().enumerate().map(|(i, l)| (i, l.name.clone(), l.points.len())).collect();
    let paths: Vec<String> = hist.iter().map(|(m, _)| doc.world.objects.models[*m as usize].path.clone()).collect();
    let mut select_model = None;
    let mut start_rename = None;
    let mut select_modifier = None;
    let mut select_line = None;
    egui::ScrollArea::vertical().id_salt("outl").auto_shrink([false, false]).show(ui, |ui| {
        egui::CollapsingHeader::new(tf("Terrain modifiers ({0})", &[&mods.len().to_string()])).default_open(true).show(ui, |ui| {
            for (id, name) in mods.iter().filter(|(_, n)| n.to_ascii_lowercase().contains(&filter)) {
                if ui.selectable_label(app.selected_modifier == Some(*id), name).clicked() { select_modifier = Some(*id); }
            }
        });
        egui::CollapsingHeader::new(tf("Roads and lines ({0})", &[&lines.len().to_string()])).default_open(false).show(ui, |ui| {
            for (i, name, n) in lines.iter().filter(|(_, n, _)| n.to_ascii_lowercase().contains(&filter)) {
                if ui.selectable_label(app.selected_line == Some(*i), format!("{name}  ({n})")).clicked() { select_line = Some(*i); }
            }
        });
        let live = layers.iter().filter(|l| !l.deleted).count();
        egui::CollapsingHeader::new(tf("Object layers ({0})", &[&live.to_string()])).default_open(true).show(ui, |ui| {
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
                    row.context_menu(|ui| {
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
        egui::CollapsingHeader::new(tf("Objects ({0})", &[&doc.world.objects.len().to_string()])).default_open(true).show(ui, |ui| {
            for (k, (m, count)) in hist.iter().enumerate() {
                let name = paths[k].rsplit('\\').next().unwrap_or(&paths[k]);
                if !filter.is_empty() && !name.contains(&filter) { continue; }
                if ui.selectable_label(false, format!("{name}  ×{count}")).on_hover_text(&paths[k]).clicked() { select_model = Some(*m); }
            }
        });
    });
    if let Some(m) = select_model {
        let shift = ui.input(|i| i.modifiers.shift);
        if !shift { app.selection.clear(); }
        let d = app.doc.as_ref().unwrap();
        app.selection.extend(d.world.objects.iter().filter(|(_, o)| o.model == m && d.world.object_interactive(o)).map(|(id, _)| id));
        app.selection_rev += 1;
    }
    if let Some(a) = layer_action {
        if matches!(a, LayerAction::Rename(..)) { app.layer_rename = None; }
        app.layer_action(a);
    }
    if start_rename.is_some() { app.layer_rename = start_rename; }
    if select_modifier.is_some() { app.selected_modifier = select_modifier; app.selected_line = None; }
    if select_line.is_some() { app.selected_line = select_line; app.selected_modifier = None; }
}

fn details(app: &mut App, ui: &mut egui::Ui) {
    panel_title(ui, &t("Details"));
    egui::ScrollArea::vertical().id_salt("det").auto_shrink([false, false]).show(ui, |ui| {
        if let Some(id) = app.selected_modifier { modifier_details(app, ui, id); return; }
        if let Some(i) = app.selected_line { line_details(app, ui, i); return; }
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
    section(ui, &t("Transform"), true, |ui| {
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
    section(ui, &t("Terrain modifier"), true, |ui| {
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
            ModShape::Rect { half_x, half_z } => {
                slider(ui, "X", half_x, 0.5..=300.0, true);
                slider(ui, "Z", half_z, 0.5..=300.0, true);
            }
        }
        slider(ui, &t("Blend"), &mut m.falloff, 0.0..=120.0, false);
        slider(ui, &t("Height offset"), &mut m.amount, -50.0..=50.0, false);
        let mut fixed = m.target.is_some();
        if ui.checkbox(&mut fixed, t("Fixed target height")).changed() { m.target = if fixed { Some(0.0) } else { None }; }
        if let Some(v) = &mut m.target { row(ui, &t("Height"), |ui| { ui.add(egui::DragValue::new(v).speed(0.1)); }); }
        if m.attached.is_none() {
            slider(ui, "X", &mut m.x, 0.0..=30000.0, false);
            slider(ui, "Z", &mut m.z, 0.0..=30000.0, false);
        }
        remove = ui.button(t("Remove modifier")).clicked();
    });
    if remove {
        app.selected_modifier = None;
        app.set_modifiers(&t("Remove modifier"), |list| list.retain(|x| x.id != id));
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

fn line_details(app: &mut App, ui: &mut egui::Ui, i: usize) {
    let Some(doc) = &mut app.doc else { return };
    let Some(line) = doc.world.lines.get_mut(i) else { app.selected_line = None; return };
    let mut remove = false;
    section(ui, &t("Road or line"), true, |ui| {
        row(ui, &t("Name"), |ui| { ui.text_edit_singleline(&mut line.name); });
        ui.label(tf("{0} points", &[&line.points.len().to_string()]));
        remove = ui.button(t("Delete")).clicked();
    });
    if remove {
        doc.world.lines.remove(i);
        doc.history.revision += 1;
        app.selected_line = None;
    }
}

fn bottom_panel(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        for (tab, name) in [(BottomTab::Content, "Content Browser"), (BottomTab::Output, "Output Log"), (BottomTab::Build, "Build")] {
            if tool_button(ui, app.bottom == tab, &t(name)).clicked() { app.bottom = tab; }
        }
    });
    match app.bottom {
        BottomTab::Content => content_browser(app, ui),
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
        row(ui, &t("Additional mods"), |ui| { ui.text_edit_singleline(&mut app.settings.extra_mods); });
        let ui = &mut c[1];
        ui.label(t("Log"));
        egui::ScrollArea::vertical().id_salt("b").stick_to_bottom(true).auto_shrink([false, false]).show_rows(ui, 14.0, app.log.len(), |ui, range| {
            for l in &app.log[range] { ui.label(RichText::new(&l.text).small().monospace()); }
        });
    });
}

fn content_browser(app: &mut App, ui: &mut egui::Ui) {
    let assets = app.assets.clone();
    egui::SidePanel::left("folders").resizable(true).default_width(260.0).frame(egui::Frame::new()).show_inside(ui, |ui| {
        ui.add(egui::TextEdit::singleline(&mut app.filter).hint_text(t("Search models...")).desired_width(f32::INFINITY));
        ui.horizontal(|ui| {
            ui.small(tf("{0} models", &[&assets.assets.len().to_string()]));
            if ui.small_button("↻").on_hover_text(t("Scan the work drive again")).clicked() { app.rescan_assets(); }
        });
        let folders: Vec<&String> = assets.by_folder.keys().collect();
        egui::ScrollArea::vertical().id_salt("f").auto_shrink([false, false]).show_rows(ui, 18.0, folders.len(), |ui, range| {
            for f in &folders[range] {
                let n = assets.by_folder[*f].len();
                if ui.selectable_label(app.browser_folder.as_deref() == Some(f.as_str()), format!("{f}  ({n})")).clicked() {
                    app.browser_folder = Some((*f).clone());
                }
            }
        });
    });
    let filter = app.filter.to_ascii_lowercase();
    let mut items: Vec<u32> = if filter.len() >= 2 {
        assets.assets.iter().enumerate().filter(|(_, a)| a.path.contains(&filter)).map(|(i, _)| i as u32).take(3000).collect()
    } else if let Some(f) = &app.browser_folder {
        assets.by_folder.get(f).cloned().unwrap_or_default()
    } else {
        Vec::new()
    };
    items.truncate(3000);
    if items.is_empty() {
        ui.centered_and_justified(|ui| { ui.weak(t("Choose a folder or type at least two letters")); });
        return;
    }
    let cell = vec2(104.0, 122.0);
    let columns = (ui.available_width() / (cell.x + 4.0)).floor().max(1.0) as usize;
    let rows = items.len().div_ceil(columns);
    egui::ScrollArea::vertical().id_salt("items").auto_shrink([false, false]).show_rows(ui, cell.y + 4.0, rows, |ui, range| {
        for r in range {
            ui.horizontal(|ui| {
                for c in 0..columns {
                    let Some(&idx) = items.get(r * columns + c) else { continue };
                    let a = &assets.assets[idx as usize];
                    let selected = app.place.model.as_deref() == Some(a.path.as_str());
                    let (rect, resp) = ui.allocate_exact_size(cell, egui::Sense::click_and_drag());
                    ui.painter().rect_filled(rect, 3.0, if selected { theme::ACCENT_SOFT } else if resp.hovered() { theme::HOVER } else { Color32::from_rgb(42, 42, 42) });
                    let img = egui::Rect::from_min_size(rect.min + vec2(4.0, 4.0), vec2(96.0, 96.0));
                    match app.thumbs.get(&a.path) {
                        Some(t) => { ui.painter().image(t.id(), img, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE); }
                        None => { ui.painter().rect_filled(img, 2.0, Color32::from_rgb(52, 52, 52)); }
                    }
                    let name: String = if a.name.chars().count() > 15 { a.name.chars().take(14).chain(['…']).collect() } else { a.name.clone() };
                    ui.painter().text(egui::pos2(rect.center().x, rect.bottom() - 11.0), egui::Align2::CENTER_CENTER, name, egui::FontId::proportional(11.0), theme::TEXT);
                    let resp = resp.on_hover_text(&a.path);
                    if resp.clicked() {
                        app.place.model = Some(a.path.clone());
                        if app.mode == Mode::Select { app.mode = Mode::Place; }
                    }
                    resp.dnd_set_drag_payload(a.path.clone());
                }
            });
        }
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
    path_choice(ui, &t("Roads (TV4P)"), &mut plan.tv4p, &scan.tv4p, file("Terrain Builder", &["tv4p"]));
    ui.weak(if plan.asc.is_some() { t("Terrain comes from the heightmap.") } else if plan.wrp.is_some() { t("Terrain comes from the WRP.") } else { t("Choose a heightmap or a WRP.") });
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
        Dialog::Settings => settings_window(app, ctx, &mut open),
        Dialog::ProjectSettings => project_window(app, ctx, &mut open),
        Dialog::About => {
            egui::Window::new(t("About")).collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
                ui.heading("DZ World Editor");
                ui.label(format!("{} {}", t("Version"), env!("CARGO_PKG_VERSION")));
                ui.label(t("Built on the DayZForests and DayZ Road Studio engines."));
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
        section(ui, &t("Paths"), true, |ui| {
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
        section(ui, &t("Display"), true, |ui| {
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
            slider(ui, &t("Terrain detail"), &mut app.settings.terrain_quality, 0.6..=3.0, false);
            slider(ui, &t("Object draw distance"), &mut app.settings.draw_distance, 300.0..=8000.0, true);
            slider(ui, &t("Full detail distance"), &mut app.settings.near_detail, 30.0..=500.0, false);
        });
        section(ui, &t("General"), true, |ui| {
            row(ui, &t("Autosave every"), |ui| { ui.add(egui::Slider::new(&mut app.settings.autosave_minutes, 0..=60).suffix(" min")); });
            ui.checkbox(&mut app.settings.check_updates, t("Check for updates at start"));
        });
    });
    if !*open { app.settings.save(); }
}

fn project_window(app: &mut App, ctx: &egui::Context, open: &mut bool) {
    let Some(doc) = &mut app.doc else { *open = false; return };
    let before = doc.world.meta.clone();
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
        ui.checkbox(&mut m.write_underground_flag, t("Write the newer WRP format with underground flag"));
        ui.separator();
        ui.label(tf("Terrain {0} × {0}, cell {1} m, map {2} m", &[&m.terrain_size.to_string(), &m.terrain_cell.to_string(), &m.size_m().to_string()]));
        ui.label(tf("Land grid {0}, cell {1} m, {2} tiles per side", &[&m.land_size().to_string(), &m.land_cell().to_string(), &m.tiles().to_string()]));
    });
    if doc.world.meta != before {
        doc.history.revision += 1;
    }
}
