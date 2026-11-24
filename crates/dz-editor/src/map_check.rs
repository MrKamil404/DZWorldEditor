//! The map check: what would break the build or the game, and what is most likely a mistake,
//! in a window where a click goes to the place. Building checks the map first, and stops on
//! errors until the user says to build anyway.
use crate::app::{App, Finish};
use crate::i18n::{t, tf};
use crate::icons;
use dz_core::check::{Issue, IssueKind, Level};
use eframe::egui;

#[derive(Default)]
pub struct MapCheck {
    pub open: bool,
    pub issues: Option<Vec<Issue>>,
    /// A build (and whether to play) the check was made for, held back by its errors.
    pub held_build: Option<bool>,
    pub show_info: bool,
}

/// Checks the map in the background; `build` builds (and plays) afterwards when nothing is in
/// the way.
pub fn run(app: &mut App, build: Option<bool>) {
    let Some(d) = &app.doc else { return };
    let world = d.world.clone();
    let (db, root) = (app.db.clone(), app.settings.tools.work_drive.clone());
    app.spawn(t("Checking the map"), move |job| {
        job.stage("Checking the map");
        let issues = dz_core::check::check(&world, &db, Some(&root), job);
        Ok(Box::new(move |app: &mut App| {
            let errors = issues.iter().filter(|i| i.level() == Level::Error).count();
            let warnings = issues.iter().filter(|i| i.level() == Level::Warning).count();
            app.check.issues = Some(issues);
            match build {
                Some(play) if errors == 0 => {
                    if warnings > 0 {
                        app.warn(tf("The map check found {0} warnings; they are in Build > Check the map", &[&warnings.to_string()]));
                    }
                    app.build_checked(play);
                }
                Some(play) => {
                    app.check.held_build = Some(play);
                    app.check.open = true;
                    app.error(tf("The map check found {0} errors: the build waits", &[&errors.to_string()]));
                }
                None => {
                    app.check.held_build = None;
                    app.check.open = true;
                    app.info(tf("Map check: {0} errors, {1} warnings", &[&errors.to_string(), &warnings.to_string()]));
                }
            }
        }) as Finish)
    });
}

/// What an issue says, in words.
fn describe(i: &Issue) -> String {
    let n = i.count.to_string();
    match i.kind {
        IssueKind::BadMapId => tf("The map id '{0}' may only have letters, digits and underscores (File > Project settings)", &[&i.name]),
        IssueKind::BadTerrainSize => t("The terrain size does not divide into the land grid (File > Project settings)"),
        IssueKind::NoSatellite => t("The map has no satellite image: import one or make it from the mask in the Satellite tab"),
        IssueKind::MissingLayerTexture => tf("The texture of the surface '{0}' is not on the work drive", &[&i.name]),
        IssueKind::MissingModel => tf("{0} objects of a model that is not on the work drive: {1}", &[&n, &i.name]),
        IssueKind::TileSurfaces => tf("{0} texture tiles hold more surfaces than the project allows; the rarest are replaced in the build", &[&n]),
        IssueKind::Floating => tf("{0} objects hang over the ground", &[&n]),
        IssueKind::Buried => tf("{0} objects are wholly under the ground but not marked as underground", &[&n]),
        IssueKind::InSea => tf("{0} trees or buildings stand in the sea", &[&n]),
        IssueKind::Outside => tf("{0} objects lie outside the map", &[&n]),
        IssueKind::Duplicate => tf("{0} objects stand twice in the same place", &[&n]),
        IssueKind::Overlap => tf("{0} buildings stand in other buildings", &[&n]),
        IssueKind::EmptyRoad => tf("The road '{0}' has no parts", &[&i.name]),
        IssueKind::BadSpawn => t("The player start is outside the map or in the sea"),
        IssueKind::NoKeyPoints => t("The map has no key points, so the in-game map shows no names"),
        IssueKind::NoNavmesh => t("No navmesh yet: AI does not walk until it is generated (Build > Generate navmesh)"),
        IssueKind::Mission => tf("The mission has {0} errors (Mission tab)", &[&n]),
        IssueKind::DenseArea => tf("{0} objects in one 100 m square (may lower FPS)", &[&n]),
    }
}

/// Goes to the place of an issue and selects its objects.
fn go_to(app: &mut App, i: &Issue) {
    let Some([x, z]) = i.at else { return };
    crate::goto::fly_to(app, dz_core::goto::Spot { x, z, radius: 30.0 });
    let Some(d) = &app.doc else { return };
    if !i.objects.is_empty() {
        app.selection = i.objects.iter().copied().filter(|id| d.world.objects.get(*id).is_some()).collect();
        app.selection_rev += 1;
    }
}

/// Removes the objects of an issue (twice in one place, outside the map), as one undo step.
fn remove(app: &mut App, i: &Issue) {
    app.selection = i.objects.iter().copied().collect();
    app.selection_rev += 1;
    app.delete_selection();
}

pub fn window(app: &mut App, ctx: &egui::Context) {
    if !app.check.open {
        return;
    }
    let mut open = true;
    let mut action: Option<(Issue, bool)> = None;
    let mut again = false;
    let mut build_anyway = None;
    egui::Window::new(icons::with(icons::LIST_CHECKS, &t("Map check"))).id(egui::Id::new("map_check")).open(&mut open).default_width(520.0).default_height(420.0).show(ctx, |ui| {
        let Some(issues) = &app.check.issues else {
            ui.label(t("Checking the map"));
            return;
        };
        let count = |l: Level| issues.iter().filter(|i| i.level() == l).count();
        ui.horizontal(|ui| {
            ui.colored_label(crate::theme::ERROR, icons::with(icons::X_CIRCLE, &tf("{0} errors", &[&count(Level::Error).to_string()])));
            ui.colored_label(crate::theme::WARN, icons::with(icons::WARNING, &tf("{0} warnings", &[&count(Level::Warning).to_string()])));
            ui.checkbox(&mut app.check.show_info, tf("Notes ({0})", &[&count(Level::Info).to_string()]));
            if ui.button(icons::with(icons::ARROWS_CLOCKWISE, &t("Check again"))).clicked() { again = true; }
        });
        if let Some(play) = app.check.held_build {
            ui.horizontal(|ui| {
                ui.colored_label(crate::theme::ERROR, t("The build waits for the errors to be fixed."));
                if ui.button(t("Build anyway")).clicked() { build_anyway = Some(play); }
            });
        }
        if issues.is_empty() {
            ui.colored_label(crate::theme::OK, icons::with(icons::CHECK_CIRCLE, &t("Nothing found")));
        }
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let mut sorted: Vec<&Issue> = issues.iter().filter(|i| app.check.show_info || i.level() != Level::Info).collect();
            sorted.sort_by_key(|i| i.level());
            for (k, i) in sorted.into_iter().enumerate() {
                ui.push_id(k, |ui| ui.horizontal(|ui| {
                    let (icon, colour) = match i.level() {
                        Level::Error => (icons::X_CIRCLE, crate::theme::ERROR),
                        Level::Warning => (icons::WARNING, crate::theme::WARN),
                        Level::Info => (icons::INFO, crate::theme::TEXT_DIM),
                    };
                    ui.colored_label(colour, icon);
                    if i.at.is_some() && ui.small_button(icons::CROSSHAIR).on_hover_text(t("Go there and select the objects")).clicked() {
                        action = Some((i.clone(), false));
                    }
                    if matches!(i.kind, IssueKind::Duplicate | IssueKind::Outside) && ui.small_button(icons::TRASH).on_hover_text(t("Delete these objects")).clicked() {
                        action = Some((i.clone(), true));
                    }
                    ui.add(egui::Label::new(describe(i)).wrap());
                }));
            }
        });
    });
    app.check.open = open;
    if let Some((i, delete)) = action {
        if delete { remove(app, &i); } else { go_to(app, &i); }
    }
    if again {
        run(app, None);
    }
    if let Some(play) = build_anyway {
        app.check.held_build = None;
        app.build_checked(play);
    }
}
