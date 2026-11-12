//! Deleting generators, routes and modifiers together with what they made. When something
//! else hangs on what is deleted, a window asks what of it to keep; all of it is one step to
//! undo.
use crate::app::{App, Dialog};
use crate::i18n::{t, tf};
use dz_core::edit::Change;
use dz_core::library::GenTarget;
use eframe::egui;

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Generator(u64),
    /// A road or track, by its id.
    Route(u64),
    /// A terrain modifier.
    Modifier(u64),
    MaskModifier(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The objects a generator placed or the models of a route.
    Objects,
    /// The objects a generator took off the map come back.
    Restore,
    /// The terrain change of a generator or route.
    Terrain,
    /// The surface a generator painted.
    Surface,
    /// What a modifier belongs to: its generator, route or object.
    Owner,
    /// The generators that follow a route.
    Followers,
}

#[derive(Clone, Debug)]
pub struct Part {
    pub kind: Kind,
    pub label: String,
    /// Deleted too, when set.
    pub remove: bool,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub target: Target,
    pub title: String,
    pub parts: Vec<Part>,
}

impl Plan {
    fn removes(&self, k: Kind) -> bool {
        self.parts.iter().any(|p| p.kind == k && p.remove)
    }
}

fn plan(app: &App, target: Target) -> Option<Plan> {
    let w = &app.doc.as_ref()?.world;
    let alive = |ids: &[u32]| ids.iter().filter(|id| w.objects.get(**id).is_some()).count();
    let mut parts = Vec::new();
    let mut part = |kind, label: String, remove| parts.push(Part { kind, label, remove });
    let title = match &target {
        Target::Generator(id) => {
            let g = w.generators.iter().find(|g| g.id == *id)?;
            let n = alive(&g.objects);
            if n > 0 { part(Kind::Objects, tf("The objects it placed ({0})", &[&n.to_string()]), true); }
            if !g.removed.is_empty() { part(Kind::Restore, tf("Put back the objects it removed ({0})", &[&g.removed.len().to_string()]), true); }
            if g.modifier.is_some_and(|m| w.modifiers.iter().any(|x| x.id == m)) { part(Kind::Terrain, t("Its change of the terrain"), true); }
            if g.mask_modifier.is_some_and(|m| w.mask_mods.position(m).is_some()) { part(Kind::Surface, t("The surface it painted"), true); }
            tf("Delete generator {0}", &[&g.name])
        }
        Target::Route(id) => {
            let r = w.routes.iter().find(|r| r.id == *id)?;
            let n = alive(&r.objects);
            if n > 0 { part(Kind::Objects, tf("Its models ({0})", &[&n.to_string()]), true); }
            if r.modifier.is_some_and(|m| w.modifiers.iter().any(|x| x.id == m)) { part(Kind::Terrain, t("The terrain levelled under it"), true); }
            let followers = w.generators.iter().filter(|g| g.target == GenTarget::Route(*id)).count();
            if followers > 0 { part(Kind::Followers, tf("The generators along it and what they made ({0})", &[&followers.to_string()]), true); }
            tf("Delete route {0}", &[&r.name])
        }
        Target::Modifier(id) => {
            let m = w.modifiers.iter().find(|m| m.id == *id)?;
            if let Some(r) = w.routes.iter().find(|r| r.modifier == Some(*id)) {
                part(Kind::Owner, tf("Delete the route {0} too", &[&r.name]), false);
            } else if let Some(g) = w.generators.iter().find(|g| g.modifier == Some(*id)) {
                part(Kind::Owner, tf("Delete the generator {0} too", &[&g.name]), false);
            } else if let Some(o) = m.attached.and_then(|a| w.objects.get(a)) {
                let name = w.objects.model_path(o).rsplit('\\').next().unwrap_or("").to_string();
                part(Kind::Owner, tf("Delete the object {0} too", &[&name]), false);
            }
            tf("Delete terrain modifier {0}", &[&m.name])
        }
        Target::MaskModifier(id) => {
            let at = w.mask_mods.position(*id)?;
            if let Some(g) = w.generators.iter().find(|g| g.mask_modifier == Some(*id)) {
                part(Kind::Owner, tf("Delete the generator {0} too", &[&g.name]), false);
            }
            tf("Delete mask modifier {0}", &[&w.mask_mods.modifiers[at].name])
        }
    };
    Some(Plan { target, title, parts })
}

/// Deletes `target`; when anything else hangs on it, a window first asks what to keep.
pub fn request(app: &mut App, target: Target) {
    let Some(plan) = plan(app, target) else { return };
    // Only what a modifier belongs to is a real question; the effects of a generator or a
    // route are asked about too, as they may be worth keeping.
    if plan.parts.is_empty() {
        execute(app, &plan);
    } else {
        app.dialog = Dialog::Delete(plan);
    }
}

/// The window that asks what to keep. Returns true when it is done.
pub fn window(app: &mut App, ctx: &egui::Context, plan: &mut Plan, open: &mut bool) -> bool {
    let mut done = false;
    egui::Window::new(format!("{} {}", crate::icons::TRASH, plan.title)).collapsible(false).resizable(false).open(open).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
        ui.label(t("Delete with it:"));
        for p in &mut plan.parts {
            ui.checkbox(&mut p.remove, &p.label);
        }
        ui.weak(t("What is not ticked stays on the map. All of it can be undone in one step."));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(egui::RichText::new(format!("{} {}", crate::icons::TRASH, t("Delete"))).color(egui::Color32::WHITE)).fill(crate::theme::ERROR)).clicked() {
                execute(app, plan);
                done = true;
            }
            if ui.button(t("Cancel")).clicked() {
                done = true;
            }
        });
    });
    done
}

/// Deletes what the plan says, as one step of the history.
pub fn execute(app: &mut App, plan: &Plan) {
    let Some(doc) = app.doc.as_mut() else { return };
    let w = &mut doc.world;
    let label = plan.title.clone();
    let (gens_before, routes_before, mods_before, masks_before) = (w.generators.clone(), w.routes.clone(), w.modifiers.clone(), w.mask_mods.modifiers.clone());
    let mut objects: Vec<u32> = Vec::new();
    let mut restore = Vec::new();
    let mut drop_mods: Vec<u64> = Vec::new();
    let mut drop_masks: Vec<u32> = Vec::new();
    let mut drop_gens: Vec<u64> = Vec::new();
    let mut drop_route: Option<u64> = None;
    // A generator with everything it made, as far as the plan lets it go.
    let take_generator = |w: &dz_core::world::World, id: u64, all: bool, objects: &mut Vec<u32>, restore: &mut Vec<dz_core::library::RemovedObject>, drop_mods: &mut Vec<u64>, drop_masks: &mut Vec<u32>, drop_gens: &mut Vec<u64>| {
        let Some(g) = w.generators.iter().find(|g| g.id == id) else { return };
        drop_gens.push(id);
        if all || plan.removes(Kind::Objects) { objects.extend(&g.objects); }
        if all || plan.removes(Kind::Restore) { restore.extend(g.removed.iter().cloned()); }
        if let Some(m) = g.modifier.filter(|_| all || plan.removes(Kind::Terrain)) { drop_mods.push(m); }
        if let Some(m) = g.mask_modifier.filter(|_| all || plan.removes(Kind::Surface)) { drop_masks.push(m); }
    };
    match &plan.target {
        Target::Generator(id) => take_generator(w, *id, false, &mut objects, &mut restore, &mut drop_mods, &mut drop_masks, &mut drop_gens),
        Target::Route(id) => {
            if let Some(r) = w.routes.iter().find(|r| r.id == *id) {
                drop_route = Some(*id);
                if plan.removes(Kind::Objects) { objects.extend(&r.objects); }
                if let Some(m) = r.modifier.filter(|_| plan.removes(Kind::Terrain)) { drop_mods.push(m); }
            }
            if plan.removes(Kind::Followers) {
                let followers: Vec<u64> = w.generators.iter().filter(|g| g.target == GenTarget::Route(*id)).map(|g| g.id).collect();
                for g in followers { take_generator(w, g, true, &mut objects, &mut restore, &mut drop_mods, &mut drop_masks, &mut drop_gens); }
            }
        }
        Target::Modifier(id) => {
            drop_mods.push(*id);
            if plan.removes(Kind::Owner) {
                if let Some(r) = w.routes.iter().find(|r| r.modifier == Some(*id)) {
                    drop_route = Some(r.id);
                    objects.extend(&r.objects);
                } else if let Some(g) = w.generators.iter().find(|g| g.modifier == Some(*id)).map(|g| g.id) {
                    take_generator(w, g, true, &mut objects, &mut restore, &mut drop_mods, &mut drop_masks, &mut drop_gens);
                } else if let Some(a) = w.modifiers.iter().find(|m| m.id == *id).and_then(|m| m.attached) {
                    objects.push(a);
                }
            }
        }
        Target::MaskModifier(id) => {
            drop_masks.push(*id);
            if plan.removes(Kind::Owner) {
                if let Some(g) = w.generators.iter().find(|g| g.mask_modifier == Some(*id)).map(|g| g.id) {
                    take_generator(w, g, true, &mut objects, &mut restore, &mut drop_mods, &mut drop_masks, &mut drop_gens);
                }
            }
        }
    }
    // Objects go, and the modifiers that follow them go with them.
    let mut change = Vec::new();
    for id in objects {
        if let Some(o) = w.objects.remove(id) {
            change.push((id, Some(o), None));
            drop_mods.extend(w.modifiers.iter().filter(|m| m.attached == Some(id)).map(|m| m.id));
        }
    }
    for saved in &restore {
        let model = w.objects.intern(&saved.model);
        let o = saved.object(model);
        change.push((w.objects.insert(o), None, Some(o)));
    }
    // An object whose ground followed it stood at a height of its own; without that ground
    // it goes back down onto the terrain.
    let grounded: Vec<u32> = w.modifiers.iter().filter(|m| m.follow && drop_mods.contains(&m.id)).filter_map(|m| m.attached).collect();
    for id in grounded {
        if let Some(o) = w.objects.get(id).copied() {
            let down = crate::app::on_ground(&app.db, w.objects.model_path(&o), o);
            if down != o {
                w.objects.update(id, down);
                change.push((id, Some(o), Some(down)));
            }
        }
    }
    let objects_changed = !change.is_empty();
    // What stays forgets what was deleted, so that running it again starts afresh.
    w.generators.retain(|g| !drop_gens.contains(&g.id));
    for g in &mut w.generators {
        if g.modifier.is_some_and(|m| drop_mods.contains(&m)) { g.modifier = None; }
        if g.mask_modifier.is_some_and(|m| drop_masks.contains(&m)) { g.mask_modifier = None; }
    }
    if let Some(id) = drop_route { w.routes.retain(|r| r.id != id); }
    for r in &mut w.routes {
        // A route whose levelling was deleted no longer levels the terrain.
        if r.modifier.is_some_and(|m| drop_mods.contains(&m)) { r.modifier = None; r.grade = false; }
    }
    let terrain = w.modifiers.iter().any(|m| drop_mods.contains(&m.id));
    w.modifiers.retain(|m| !drop_mods.contains(&m.id));
    let mut mask_area = None;
    for m in &drop_masks {
        if let Some(r) = w.mask_remove(*m) { mask_area = Some(mask_area.map_or(r, |a: dz_core::terrain::Rect| a.union(r))); }
    }
    let mut steps = 0;
    if objects_changed { doc.history.push(label.clone(), Change::Objects(change)); steps += 1; doc.dirty.objects = true; }
    doc.history.push(label.clone(), Change::Generators(gens_before)); steps += 1;
    doc.history.push(label.clone(), Change::Routes(routes_before)); steps += 1;
    if terrain { doc.history.push(label.clone(), Change::Modifiers(mods_before)); steps += 1; }
    if !drop_masks.is_empty() { doc.history.push(label.clone(), Change::MaskMods(masks_before)); steps += 1; }
    doc.history.group_last(steps, label);
    if terrain {
        doc.world.recompose(None);
        app.whole_height_changed();
    }
    if let Some(r) = mask_area { app.mask_recomposed(r); }
    if let Some(d) = &app.doc {
        app.selection.retain(|i| d.world.objects.get(*i).is_some());
    }
    app.selection_rev += 1;
    if let Target::Generator(id) = &plan.target {
        if app.selected_generator == Some(*id) { app.selected_generator = None; }
    }
    if let Target::Modifier(id) = &plan.target {
        if app.selected_modifier == Some(*id) { app.selected_modifier = None; }
    }
    if drop_route.is_some() {
        app.line.selected_route = None;
        app.line.selected_point = None;
    }
}
