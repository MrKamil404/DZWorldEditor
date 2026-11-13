use super::*;
use crate::mission::read_file;
use crate::satmod::SatKind;
use crate::terrain::RgbImage;
use crate::world::WorldMeta;
use std::sync::Arc;

fn world() -> World {
    let mut w = World::new(WorldMeta::new("Team", 128, 8.0), 10.0);
    let m = w.objects.intern("dz\\a.p3d");
    for i in 0..4 {
        w.objects.insert(Obj { x: 100.0 + i as f32 * 50.0, z: 100.0, elevation: 0.0, yaw: 0.0, pitch: 0.0, roll: 0.0, scale: 1.0, model: m, flags: 0 });
    }
    w
}
fn header() -> PatchHeader {
    PatchHeader { format: FORMAT, base: 1, result: 2, author: "Ania".into(), time: "2026-11-13_18-00-00".into(), map_id: "Team".into(), summary: Summary::default() }
}
fn id_at(w: &World, x: f32) -> u32 {
    w.objects.iter().find(|(_, o)| o.x == x).unwrap().0
}
/// Sends `theirs` as a patch through bytes and merges it into `mine`.
fn merge_into(base: &World, theirs: &World, mine: &mut World, choices: &HashMap<ConflictKey, Choice>) -> Merged {
    let p = Patch::from_bytes(&diff(base, None, theirs, None, header()).unwrap().to_bytes().unwrap()).unwrap();
    merge(base, &p, mine, Dirs::default(), choices).unwrap()
}
fn sat_tile(rgba: [u8; 4]) -> HashMap<crate::satmod::Key, crate::satmod::Tile> {
    HashMap::from([((0u16, 0u16), Arc::new(rgba.repeat(SAT_TILE * SAT_TILE)))])
}
fn mission(files: &[(&str, &str)]) -> Mission {
    let mut m = Mission { name: "dayzOffline.team".into(), ..Default::default() };
    for (rel, text) in files {
        m.put(rel, read_file(rel, text.as_bytes()));
    }
    m
}
fn file(w: &World, rel: &str) -> String {
    String::from_utf8(w.mission.as_ref().unwrap().file(rel).unwrap().bytes().unwrap()).unwrap()
}

#[test]
fn work_on_different_things_is_all_kept() {
    let base = world();
    let (mut ania, mut bartek) = (base.clone(), base.clone());
    // Ania sculpts in the west, turns an object and adds one.
    ania.base.set(3, 3, 25.0);
    let a = id_at(&ania, 100.0);
    let mut o = *ania.objects.get(a).unwrap();
    o.yaw = 45.0;
    ania.objects.update(a, o);
    let m = ania.objects.intern("dz\\new.p3d");
    ania.objects.insert(Obj { x: 600.0, model: m, ..o });
    // Bartek paints, removes an object and moves the one Ania turned.
    bartek.base.set(100, 100, 3.0);
    bartek.mask.data[5] = 2;
    let gone = id_at(&bartek, 200.0);
    bartek.objects.remove(gone);
    let b = id_at(&bartek, 100.0);
    let mut o = *bartek.objects.get(b).unwrap();
    o.x = 120.0;
    bartek.objects.update(b, o);
    let r = merge_into(&base, &ania, &mut bartek, &HashMap::new());
    assert!(r.conflicts.is_empty(), "{:?}", r.conflicts);
    assert_eq!((bartek.base.at(3, 3), bartek.base.at(100, 100)), (25.0, 3.0));
    assert_eq!(bartek.mask.data[5], 2);
    assert_eq!(bartek.objects.len(), 4);
    let moved = bartek.objects.get(b).unwrap();
    assert_eq!((moved.x, moved.yaw), (120.0, 45.0), "a move and a turn of one object both stay");
    assert!(bartek.objects.iter().any(|(_, o)| bartek.objects.model_path(o) == "dz\\new.p3d"));
}

#[test]
fn both_sculpting_the_same_place_adds_up_unless_chosen() {
    let base = world();
    let (mut ania, mut bartek) = (base.clone(), base.clone());
    ania.base.set(10, 10, 15.0);
    bartek.base.set(10, 10, 12.0);
    let mut mine = bartek.clone();
    let r = merge_into(&base, &ania, &mut mine, &HashMap::new());
    assert_eq!(r.conflicts.len(), 1);
    assert_eq!(r.conflicts[0].kind, ConflictKind::Terrain);
    assert_eq!(mine.base.at(10, 10), 17.0, "+5 and +2 on 10");
    let key = r.conflicts[0].key;
    let mut mine = bartek.clone();
    merge_into(&base, &ania, &mut mine, &HashMap::from([(key, Choice::Theirs)]));
    assert_eq!(mine.base.at(10, 10), 15.0);
}

#[test]
fn the_same_object_changed_twice_is_a_conflict() {
    let base = world();
    let (mut ania, mut bartek) = (base.clone(), base.clone());
    let id = id_at(&base, 150.0);
    let mut o = *base.objects.get(id).unwrap();
    o.x = 160.0;
    ania.objects.update(id, o);
    o.x = 170.0;
    bartek.objects.update(id, o);
    let mut mine = bartek.clone();
    let r = merge_into(&base, &ania, &mut mine, &HashMap::new());
    assert_eq!(r.conflicts.len(), 1);
    assert_eq!(mine.objects.get(id).unwrap().x, 170.0);
    let mut mine = bartek.clone();
    merge_into(&base, &ania, &mut mine, &HashMap::from([(r.conflicts[0].key, Choice::Theirs)]));
    assert_eq!(mine.objects.get(id).unwrap().x, 160.0);
}

#[test]
fn layers_are_matched_by_name() {
    let base = world();
    let (mut ania, mut bartek) = (base.clone(), base.clone());
    bartek.add_object_layer("Roads");
    let rocks = ania.add_object_layer("Rocks");
    let id = id_at(&ania, 100.0);
    let mut o = *ania.objects.get(id).unwrap();
    o.set_layer(rocks);
    ania.objects.update(id, o);
    merge_into(&base, &ania, &mut bartek, &HashMap::new());
    let o = bartek.objects.get(id).unwrap();
    assert_eq!(bartek.object_layers[o.layer() as usize].name, "Rocks");
    assert_eq!(bartek.object_layers.len(), 3);
}

#[test]
fn roads_follow_their_objects() {
    let base = world();
    let mut ania = base.clone();
    let mut bartek = base.clone();
    // Bartek has more objects, so the same object has another id in his copy.
    let m = bartek.objects.intern("dz\\x.p3d");
    for i in 0..10 {
        bartek.objects.insert(Obj { x: 900.0, z: i as f32, elevation: 0.0, yaw: 0.0, pitch: 0.0, roll: 0.0, scale: 1.0, model: m, flags: 0 });
    }
    let am = ania.objects.intern("dz\\x.p3d");
    let road = ania.objects.insert(Obj { x: 700.0, z: 700.0, elevation: 0.0, yaw: 0.0, pitch: 0.0, roll: 0.0, scale: 1.0, model: am, flags: 0 });
    ania.routes.push(crate::world::RoadRoute { name: "r".into(), family: "asf2".into(), points: vec![[700.0, 700.0]], centerline: vec![], objects: vec![road], id: 77, grading: Default::default(), profile: Vec::new(), decks: vec![], terrain_route: true, settings: Default::default(), grade: false, grade_width: 9.0, grade_falloff: 12.0, modifier: None, parts: Vec::new(), fitted: Vec::new(), method: Default::default() });
    merge_into(&base, &ania, &mut bartek, &HashMap::new());
    assert_eq!(bartek.routes.len(), 1);
    assert_eq!(bartek.objects.get(bartek.routes[0].objects[0]).unwrap().x, 700.0);
}

#[test]
fn deleting_what_the_other_changed_is_asked() {
    let base = world();
    let (mut ania, mut bartek) = (base.clone(), base.clone());
    let id = id_at(&base, 250.0);
    ania.objects.remove(id);
    let mut o = *bartek.objects.get(id).unwrap();
    o.scale = 2.0;
    bartek.objects.update(id, o);
    let mut mine = bartek.clone();
    let r = merge_into(&base, &ania, &mut mine, &HashMap::new());
    assert_eq!(r.conflicts[0].kind, ConflictKind::DeletedThere);
    assert!(mine.objects.get(id).is_some(), "by default what you changed stays");
}

#[test]
fn mask_modifiers_of_both_stay_over_the_merged_painting() {
    let base = world();
    let (mut ania, mut bartek) = (base.clone(), base.clone());
    // Ania paints, Bartek lays a modifier over another place.
    ania.mask.data[0] = 3;
    let id = bartek.mask_mods.add(crate::maskmod::MaskKind::Paint, "forest".into());
    let mut tile = vec![crate::maskmod::NONE; crate::maskmod::TILE * crate::maskmod::TILE];
    tile[crate::maskmod::TILE * 2 + 2] = 5;
    bartek.mask_replace_tiles(id, HashMap::from([((0u16, 0u16), Arc::new(tile))]));
    let w = bartek.mask.width;
    assert_eq!(bartek.mask.data[2 * w + 2], 5);
    let mut mine = bartek.clone();
    let r = merge_into(&base, &ania, &mut mine, &HashMap::new());
    assert!(r.conflicts.is_empty());
    assert_eq!((mine.mask.data[0], mine.mask.data[2 * w + 2]), (3, 5));
    // And the other way: the modifier travels to Ania and is laid over her painting.
    let mut mine = ania.clone();
    merge_into(&base, &bartek, &mut mine, &HashMap::new());
    assert_eq!((mine.mask.data[0], mine.mask.data[2 * w + 2]), (3, 5));
    assert_eq!(mine.mask_mods.modifiers.len(), 1);
    // Hiding the modifier brings back the painting under it.
    mine.mask_set_enabled(id, false);
    assert_eq!(mine.mask.data[2 * w + 2], base.mask.data[2 * w + 2]);
}

#[test]
fn satellite_pixels_and_modifiers_are_merged() {
    let mut base = world();
    base.satellite = Some(RgbImage::new(300, 300, [10, 20, 30]));
    let (mut ania, mut bartek) = (base.clone(), base.clone());
    // Ania paints the image in the corner far from (0, 0); Bartek bakes a modifier at (0, 0).
    let s = ania.satellite.as_mut().unwrap();
    let i = (299 * 300 + 299) * 3;
    s.data[i..i + 3].copy_from_slice(&[200, 0, 0]);
    let id = bartek.sat.add(SatKind::Bake, "bake".into());
    bartek.sat_add_tiles(id, sat_tile([0, 255, 0, 255]));
    let mut mine = bartek.clone();
    let r = merge_into(&base, &ania, &mut mine, &HashMap::new());
    assert!(r.conflicts.is_empty());
    let s = mine.satellite.as_ref().unwrap();
    assert_eq!((&s.data[i..i + 3], &s.data[0..3]), (&[200u8, 0, 0][..], &[0u8, 255, 0][..]));
    // The modifier travels to Ania with the pixels under it.
    let mut mine = ania.clone();
    merge_into(&base, &bartek, &mut mine, &HashMap::new());
    assert_eq!(&mine.satellite.as_ref().unwrap().data[0..3], &[0, 255, 0]);
    mine.sat_remove(id);
    assert_eq!(&mine.satellite.as_ref().unwrap().data[0..3], &[10, 20, 30]);
    assert_eq!(&mine.satellite.as_ref().unwrap().data[i..i + 3], &[200, 0, 0]);
}

#[test]
fn the_same_satellite_pixels_painted_twice_are_asked() {
    let mut base = world();
    base.satellite = Some(RgbImage::new(64, 64, [0, 0, 0]));
    let (mut ania, mut bartek) = (base.clone(), base.clone());
    ania.satellite.as_mut().unwrap().data[0] = 100;
    bartek.satellite.as_mut().unwrap().data[0] = 50;
    let mut mine = bartek.clone();
    let r = merge_into(&base, &ania, &mut mine, &HashMap::new());
    assert_eq!(r.conflicts[0].kind, ConflictKind::Satellite);
    assert_eq!(mine.satellite.as_ref().unwrap().data[0], 50);
    let mut mine = bartek.clone();
    merge_into(&base, &ania, &mut mine, &HashMap::from([(r.conflicts[0].key, Choice::Theirs)]));
    assert_eq!(mine.satellite.as_ref().unwrap().data[0], 100);
}

#[test]
fn mission_files_merge_line_by_line() {
    let mut base = world();
    base.mission = Some(Box::new(mission(&[("init.c", "void main()\n{\n\tint a = 1;\n\tint b = 2;\n}\n")])));
    let (mut ania, mut bartek) = (base.clone(), base.clone());
    ania.mission = Some(Box::new(mission(&[("init.c", "void main()\n{\n\tint a = 10;\n\tint b = 2;\n}\n"), ("custom/new.c", "x\n")])));
    bartek.mission = Some(Box::new(mission(&[("init.c", "void main()\n{\n\tint a = 1;\n\tint b = 20;\n}\n")])));
    let mut mine = bartek.clone();
    let r = merge_into(&base, &ania, &mut mine, &HashMap::new());
    assert!(r.conflicts.is_empty(), "{:?}", r.conflicts);
    assert_eq!(file(&mine, "init.c"), "void main()\n{\n\tint a = 10;\n\tint b = 20;\n}\n");
    assert_eq!(file(&mine, "custom/new.c"), "x\n");
    // The same line changed by both is asked, file by file.
    ania.mission = Some(Box::new(mission(&[("init.c", "void main()\n{\n\tint a = 1;\n\tint b = 30;\n}\n")])));
    let mut mine = bartek.clone();
    let r = merge_into(&base, &ania, &mut mine, &HashMap::new());
    assert_eq!(r.conflicts[0].kind, ConflictKind::Lines);
    assert!(file(&mine, "init.c").contains("b = 20"));
    let mut mine = bartek.clone();
    merge_into(&base, &ania, &mut mine, &HashMap::from([(r.conflicts[0].key, Choice::Theirs)]));
    assert!(file(&mine, "init.c").contains("b = 30"));
}

#[test]
fn map_settings_merge_setting_by_setting() {
    let base = world();
    let (mut ania, mut bartek) = (base.clone(), base.clone());
    ania.meta.name = "New name".into();
    ania.meta.config.description = "Ania".into();
    bartek.meta.sea_level = 4.0;
    bartek.meta.config.description = "Bartek".into();
    let mut mine = bartek.clone();
    let r = merge_into(&base, &ania, &mut mine, &HashMap::new());
    assert_eq!((mine.meta.name.as_str(), mine.meta.sea_level), ("New name", 4.0));
    assert_eq!(r.conflicts.len(), 1);
    assert_eq!(r.conflicts[0].what, "config.description");
    assert_eq!(mine.meta.config.description, "Bartek");
}

#[test]
fn modifiers_get_ids_others_do_not_make() {
    let base = world();
    let (mut ania, mut bartek) = (base.clone(), base.clone());
    let a = ania.sat.add(SatKind::Bake, "a".into());
    let b = bartek.sat.add(SatKind::Bake, "b".into());
    assert_ne!(a, b);
    assert_ne!(ania.new_modifier_id(), bartek.new_modifier_id());
}

#[test]
fn a_day_of_two_people_through_the_team_folder() {
    let d = tempfile::tempdir().unwrap();
    let folder = d.path().join("team");
    std::fs::create_dir_all(&folder).unwrap();
    let (pa, pb) = (d.path().join("ania"), d.path().join("bartek"));
    let mut start_world = world();
    start_world.satellite = Some(RgbImage::new(200, 200, [1, 2, 3]));
    start_world.mission = Some(Box::new(mission(&[("init.c", "a\nb\nc\n")])));
    project::save(&start_world, &pa, SaveParts::ALL).unwrap();
    project::save_mission(&mut start_world, &pa).unwrap();
    start(&pa, &start_world, &folder.to_string_lossy()).unwrap();
    // Bartek gets a full copy; the satellite image is not sent twice and the team folder comes along.
    let full = send_full(&pa, "Team", "2026-11-13_08-00-00").unwrap();
    let base_id = TeamState::load(&pa).base;
    assert!(!version_dir(&full, base_id).join(SATELLITE).exists());
    take_full(&full, &pb).unwrap();
    assert_eq!(TeamState::load(&pb).folder, folder.display().to_string());
    assert!(version_dir(&pb, base_id).join(SATELLITE).is_file());
    let mut ania = project::load(&pa).unwrap();
    let mut bartek = project::load(&pb).unwrap();
    // The day.
    ania.base.set(5, 5, 30.0);
    ania.satellite.as_mut().unwrap().data[0] = 99;
    ania.mission.as_mut().unwrap().put("init.c", read_file("init.c", b"A\nb\nc\n"));
    bartek.base.set(90, 90, 1.0);
    bartek.mission.as_mut().unwrap().put("init.c", read_file("init.c", b"a\nb\nC\n"));
    let id = id_at(&bartek, 100.0);
    bartek.objects.remove(id);
    let sid = bartek.sat.add(SatKind::Bake, "bake".into());
    bartek.sat_add_tiles(sid, sat_tile([0, 0, 255, 255]));
    for (w, p) in [(&mut ania, &pa), (&mut bartek, &pb)] {
        project::save(w, p, SaveParts::ALL).unwrap();
        project::save_mission(w, p).unwrap();
    }
    // Evening: Ania sends, Bartek merges and sends back, Ania takes it.
    send(&pa, &ania, "Ania", "2026-11-13_18-00-00").unwrap();
    let list = incoming(&pb, "Team");
    assert_eq!(list.len(), 1);
    let (p, merged) = prepare(&pb, &list[0].0).unwrap();
    assert!(merged.conflicts.is_empty(), "{:?}", merged.conflicts);
    finish(&pb, p, &HashMap::new(), "2026-11-13_18-10-00").unwrap();
    assert!(incoming(&pb, "Team").is_empty());
    let bartek = project::load(&pb).unwrap();
    assert_eq!((bartek.base.at(5, 5), bartek.base.at(90, 90), bartek.objects.len()), (30.0, 1.0, 3));
    assert_eq!(file(&bartek, "init.c"), "A\nb\nC\n");
    let s = &bartek.satellite.as_ref().unwrap().data;
    assert_eq!((s[0], s[1], s[2]), (0, 0, 255), "Bartek's modifier lies over Ania's painting");
    send(&pb, &bartek, "Bartek", "2026-11-13_18-20-00").unwrap();
    let list = incoming(&pa, "Team");
    assert_eq!(list.len(), 1, "Ania's own patch is not offered back to her");
    let (p, merged) = prepare(&pa, &list[0].0).unwrap();
    assert!(merged.conflicts.is_empty(), "{:?}", merged.conflicts);
    finish(&pa, p, &HashMap::new(), "2026-11-13_18-30-00").unwrap();
    let mut ania = project::load(&pa).unwrap();
    assert_eq!(ania.base.data, bartek.base.data);
    assert_eq!(ania.satellite.as_ref().unwrap().data, bartek.satellite.as_ref().unwrap().data);
    assert_eq!(file(&ania, "init.c"), "A\nb\nC\n");
    assert_eq!(ania.objects.len(), 3);
    // Hiding the modifier in Ania's copy shows her painting under it.
    ania.sat_set_enabled(sid, false);
    assert_eq!(ania.satellite.as_ref().unwrap().data[0], 99);
    // Both now count from the same version, so tomorrow starts clean.
    let ania = project::load(&pa).unwrap();
    let base = TeamState::load(&pa).base;
    assert_eq!(base, TeamState::load(&pb).base);
    let empty = diff(&load_version(&pa, base).unwrap(), Some(&version_dir(&pa, base)), &ania, None, header()).unwrap();
    assert!(empty.header.summary.is_empty(), "{:?}", empty.header.summary);
}

/// A copy worked on by `name`, with team work on.
fn as_person(w: &World, name: &str) -> World {
    let mut w = w.clone();
    w.team_me = name.into();
    w.note_authors();
    w
}
fn header_by(author: &str) -> PatchHeader {
    PatchHeader { author: author.into(), ..header() }
}
fn merge_from(base: &World, theirs: &World, author: &str, mine: &mut World, choices: &HashMap<ConflictKey, Choice>) -> Merged {
    let p = Patch::from_bytes(&diff(base, None, theirs, None, header_by(author)).unwrap().to_bytes().unwrap()).unwrap();
    merge(base, &p, mine, Dirs::default(), choices).unwrap()
}

#[test]
fn layers_of_others_are_left_alone_unless_unlocked() {
    let mut w = as_person(&world(), "Bartek");
    let mine = w.add_object_layer("Bartek's");
    w.team_me = "Ania".into();
    let hers = w.add_object_layer("Ania's");
    w.team_me = "Bartek".into();
    assert_eq!((w.object_layers[mine as usize].owner.as_str(), w.object_layers[hers as usize].owner.as_str()), ("Bartek", "Ania"));
    let id = id_at(&w, 100.0);
    let mut o = *w.objects.get(id).unwrap();
    o.set_layer(hers);
    w.objects.update(id, o);
    assert!(!w.object_interactive(&o) && !w.layer_interactive(hers) && w.layer_foreign(hers));
    assert!(w.layer_interactive(mine) && w.layer_interactive(0), "layers of no one stay open");
    w.team_unlocked.insert("Ania's".into());
    assert!(w.object_interactive(&o));
    // Without team work every layer is open.
    w.team_unlocked.clear();
    w.team_me.clear();
    assert!(w.object_interactive(&o));
}

#[test]
fn the_owner_of_a_layer_wins_conflicts_by_default() {
    let mut base = as_person(&world(), "Ania");
    let layer = base.add_object_layer("Ania's forest");
    let id = id_at(&base, 150.0);
    let mut o = *base.objects.get(id).unwrap();
    o.set_layer(layer);
    base.objects.update(id, o);
    let (mut ania, mut bartek) = (base.clone(), as_person(&base, "Bartek"));
    let (mut a, mut b) = (o, o);
    a.x = 160.0;
    b.x = 170.0;
    ania.objects.update(id, a);
    bartek.objects.update(id, b);
    // Bartek merges Ania's changes: her layer, her version stays unless he chooses.
    let mut mine = bartek.clone();
    let r = merge_from(&base, &ania, "Ania", &mut mine, &HashMap::new());
    assert_eq!((r.conflicts[0].default, mine.objects.get(id).unwrap().x), (Choice::Theirs, 160.0));
    // Ania merges Bartek's changes: still her layer, so hers stays.
    let mut mine = ania.clone();
    let r = merge_from(&base, &bartek, "Bartek", &mut mine, &HashMap::new());
    assert_eq!((r.conflicts[0].default, mine.objects.get(id).unwrap().x), (Choice::Mine, 160.0));
}

#[test]
fn owners_and_authors_travel_with_the_changes() {
    let base = as_person(&world(), "Ania");
    let mut ania = base.clone();
    let mut bartek = as_person(&base, "Bartek");
    // An empty layer of Ania and a modifier she makes.
    ania.add_object_layer("Ania's villages");
    let mid = ania.new_modifier_id();
    ania.modifiers.push(crate::world::TerrainModifier { id: mid, name: "pad".into(), enabled: true, x: 400.0, z: 400.0, yaw: 0.0, shape: crate::world::ModShape::Circle { radius: 16.0 }, mode: crate::world::ModMode::Offset, target: None, amount: 3.0, falloff: 8.0, attached: None, follow: false });
    ania.note_authors();
    assert_eq!(ania.author(mid), "Ania");
    merge_from(&base, &ania, "Ania", &mut bartek, &HashMap::new());
    let l = bartek.object_layers.iter().find(|l| l.name == "Ania's villages").unwrap();
    assert_eq!(l.owner, "Ania");
    assert_eq!(bartek.author(mid), "Ania", "Bartek does not become the author of what he got");
    bartek.note_authors();
    assert_eq!(bartek.author(mid), "Ania");
    // Both change the modifier: Ania made it, so hers stays by default in Bartek's copy.
    let shared = bartek.clone();
    let (mut a2, mut b2) = (as_person(&shared, "Ania"), shared.clone());
    a2.modifiers[0].amount = 5.0;
    b2.modifiers[0].amount = 7.0;
    let r = merge_from(&shared, &a2, "Ania", &mut b2, &HashMap::new());
    assert_eq!((r.conflicts[0].default, b2.modifiers[0].amount), (Choice::Theirs, 5.0));
}

#[test]
fn what_was_there_when_team_work_started_has_no_author() {
    let mut w = world();
    let mid = w.new_modifier_id();
    w.modifiers.push(crate::world::TerrainModifier { id: mid, name: "old".into(), enabled: true, x: 0.0, z: 0.0, yaw: 0.0, shape: crate::world::ModShape::Circle { radius: 1.0 }, mode: crate::world::ModMode::Offset, target: None, amount: 1.0, falloff: 1.0, attached: None, follow: false });
    let w = as_person(&w, "Ania");
    assert_eq!(w.author(mid), "");
    let d = tempfile::tempdir().unwrap();
    project::save(&w, d.path(), SaveParts::ALL).unwrap();
    assert_eq!(project::load(d.path()).unwrap().authors, w.authors);
}
