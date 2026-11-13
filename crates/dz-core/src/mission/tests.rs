use super::*;
use crate::world::WorldMeta;

fn meta() -> WorldMeta {
    WorldMeta::new("Test Map", 1024, 4.0)
}

/// Everything a file holds, in a form that does not depend on how it was written.
fn content(f: &MissionFile) -> String {
    match &f.data {
        FileData::Xml(d) => format!("{:?}{:?}{:?}{:?}", d.declaration, d.before, d.root.normalized(), d.after),
        FileData::Json(d) => d.value.to_string(),
        FileData::Text(t) => t.text.clone(),
        FileData::AreaFlags(a) => format!("{}x{} {} {}", a.width, a.height, a.usage.len(), a.value.len()),
        FileData::Disk => String::new(),
    }
}

#[test]
fn a_new_mission_writes_reads_and_validates() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = templates::new_mission(&meta());
    assert_eq!(m.name, "dayzOffline.testmap");
    let issues = validate::check(&m, Some(4096.0));
    let errors: Vec<_> = issues.iter().filter(|i| i.level == validate::Level::Error).map(|i| i.message()).collect();
    assert!(errors.is_empty(), "{errors:?}");
    let out = dir.path().join(&m.name);
    m.write(&out, true, false).unwrap();
    assert!(!m.is_dirty());
    let back = Mission::load(&out).unwrap();
    assert_eq!(back.files.len(), m.files.len());
    for (k, f) in &m.files {
        let b = back.file(k).unwrap_or_else(|| panic!("{k} missing"));
        assert_eq!(content(f), content(b), "{k}");
    }
    let a = back.areaflags().unwrap();
    assert_eq!(a.world_width, 4096);
    assert_eq!(back.limits("usageflags").len(), 17);
    // Places: the default player spawn is in all three sections.
    let p = places::collect(&back, &[places::PlaceKind::PlayerSpawn]);
    assert_eq!(p.len(), 3);
}

#[test]
fn undo_puts_everything_back() {
    let mut m = Some(Box::new(templates::new_mission(&meta())));
    let before = m.clone();
    let mm = m.as_mut().unwrap();
    let mut edits = Vec::new();
    edits.extend(insert_child(mm, "db/types.xml", &[], None, ce::TypeRow::new_element("Apple")));
    edits.extend(places::add_event_spawn(mm, "VehicleSedan", 100.0, 200.0, 45.0));
    edits.extend(places::add_player_spawn(mm, "fresh", "North", 10.0, 3000.0));
    edits.push(places::add_json(mm, "cfgeffectarea.json", "Areas", places::effect_area("Zone", 500.0, 600.0, 120.0)));
    let zone = places::zone("InfectedVillageTier1", 50.0, 60.0, 70.0, 0, 0, 3, 5);
    edits.extend(places::add_zone(mm, "env/zombie_territories.xml", 0, zone));
    let p = places::collect(mm, &places::PlaceKind::ALL);
    assert!(p.iter().any(|p| p.kind == places::PlaceKind::EventSpawn && p.x == 100.0));
    assert!(p.iter().any(|p| p.kind == places::PlaceKind::EffectArea && p.radius == 120.0));
    assert!(p.iter().any(|p| p.kind == places::PlaceKind::Territory && p.radius == 70.0));
    let spawn = p.iter().find(|p| p.kind == places::PlaceKind::EventSpawn).unwrap().clone();
    edits.extend(places::move_to(mm, &spawn.at, 150.0, 250.0, None));
    let area = p.iter().find(|p| p.kind == places::PlaceKind::EffectArea).unwrap().clone();
    edits.extend(places::move_to(mm, &area.at, 700.0, 800.0, None));
    let moved = places::collect(mm, &[places::PlaceKind::EventSpawn, places::PlaceKind::EffectArea]);
    assert!(moved.iter().any(|p| p.x == 150.0 && p.z == 250.0));
    assert!(moved.iter().any(|p| p.x == 700.0 && p.z == 800.0));
    if let Some(a) = mm.areaflags_mut() {
        let rect = [0, 0, 10, 10];
        let values = a.copy_rect(FlagLayer::Usage, 0, 0, 10, 10);
        a.paint(FlagLayer::Usage, 3, true, 0, 0, 10, 10, |_, _| true);
        edits.push(MissionEdit::Flags { layer: FlagLayer::Usage, rect, values });
    }
    // Undo is the edits in reverse.
    edits.reverse();
    toggle(&mut m, &mut edits);
    let (a, b) = (m.as_ref().unwrap(), before.as_ref().unwrap());
    for (k, f) in &b.files {
        assert_eq!(content(f), content(a.file(k).unwrap()), "{k}");
    }
    assert_eq!(a.areaflags().unwrap().usage, b.areaflags().unwrap().usage);
    // Redo brings the changes back.
    toggle(&mut m, &mut edits);
    let again = places::collect(m.as_ref().unwrap(), &[places::PlaceKind::EventSpawn]);
    assert!(again.iter().any(|p| p.x == 150.0));
}

#[test]
fn another_mission_becomes_this_maps() {
    let mut m = templates::new_mission(&meta());
    places::add_event_spawn(&mut m, "VehicleSedan", 100.0, 200.0, 45.0);
    m.put("storage_1/data/events.bin", FileData::Disk);
    let other = WorldMeta::new("Other", 2048, 4.0);
    let a = templates::adapt(m, &other);
    assert_eq!(a.name, "dayzOffline.other");
    assert!(places::collect(&a, &[places::PlaceKind::EventSpawn]).is_empty());
    assert_eq!(a.areaflags().unwrap().world_width, 8192);
    assert_eq!(a.storage_size().0, 0);
}

/// Reads every mission of a checkout of BohemiaInteractive/DayZ-Central-Economy (set
/// `DZWE_CE_DIR` to it) and checks that writing each file gives back all it held, and that an
/// export copies the files nobody changed byte for byte.
#[test]
fn vanilla_missions_round_trip() {
    let Ok(root) = std::env::var("DZWE_CE_DIR") else {
        eprintln!("DZWE_CE_DIR is not set; the vanilla missions are not checked");
        return;
    };
    let mut checked = 0;
    for e in std::fs::read_dir(&root).unwrap().flatten() {
        let dir = e.path();
        if !dir.is_dir() || !Mission::is_mission_dir(&dir) {
            continue;
        }
        let mut m = Mission::load(&dir).unwrap();
        for (k, f) in &m.files {
            let Some(bytes) = f.serialized() else { continue };
            let original = std::fs::read(f.origin.as_ref().unwrap()).unwrap();
            let again = read_file(k, &bytes);
            let first = MissionFile { data: read_file(k, &original), origin: None, dirty: true };
            let second = MissionFile { data: again, origin: None, dirty: true };
            assert_eq!(content(&first), content(&second), "{}: {k}", dir.display());
            match (&first.data, &second.data) {
                (FileData::AreaFlags(_), _) | (FileData::Text(_), _) => assert_eq!(bytes, original, "{}: {k} is not byte for byte", dir.display()),
                (FileData::Xml(a), FileData::Xml(b)) => assert_eq!(a.root.normalized(), b.root.normalized()),
                (FileData::Json(a), FileData::Json(b)) => assert_eq!(a.value, b.value),
                _ => {}
            }
            checked += 1;
        }
        // Every reader view must see the vanilla data.
        if m.file("db/types.xml").is_some() {
            assert!(!ce::all_types(&m).is_empty(), "{}", dir.display());
        }
        if m.xml("cfgeventspawns.xml").is_some_and(|d| d.root.elements().next().is_some()) {
            assert!(!places::collect(&m, &places::PlaceKind::ALL).is_empty(), "{}", dir.display());
        }
        let issues = validate::check(&m, None);
        let errors: Vec<String> = issues.iter().filter(|i| i.level == validate::Level::Error).map(|i| format!("{}: {}", i.file, i.message())).collect();
        eprintln!("{}: {} issues, {} errors {:?}", dir.display(), issues.len(), errors.len(), &errors[..errors.len().min(6)]);
        if std::env::var("DZWE_CE_VERBOSE").is_ok() {
            let mut kinds: std::collections::BTreeMap<&str, usize> = Default::default();
            for i in &issues { *kinds.entry(i.text).or_default() += 1; }
            eprintln!("{kinds:#?}");
        }
        // An export of an untouched mission is a copy.
        let out = tempfile::tempdir().unwrap();
        m.write(out.path(), false, false).unwrap();
        for (k, f) in &m.files {
            let a = std::fs::read(f.origin.as_ref().unwrap()).unwrap();
            let b = std::fs::read(out.path().join(k)).unwrap();
            assert!(a == b, "{}: {k} changed on export", dir.display());
        }
    }
    assert!(checked > 0, "no missions under {root}");
}

#[test]
fn static_events_place_groups() {
    let mut m = templates::new_mission(&meta());
    let objects = vec![
        places::GroupObject { kind: "Land_Wreck_V3S_DE".into(), x: 500.0, z: 600.0, y: 12.0, ground: 12.0, heading: 42.8 },
        places::GroupObject { kind: "StaticObj_Wreck_BRDM_DE".into(), x: 483.0, z: 571.0, y: 12.2, ground: 12.2, heading: -10.0 },
        places::GroupObject { kind: "StaticObj_Wreck_Train_742_Red_DE".into(), x: 510.0, z: 600.0, y: 14.0, ground: 12.0, heading: 0.0 },
    ];
    let (group, [x, y, z]) = places::group_from_objects("Abandoned_Test", &objects, (2, 3)).unwrap();
    let mut edits = vec![places::put_group(&mut m, group)];
    edits.extend(m.put_event_for_test("StaticMilitaryConvoy"));
    edits.extend(places::add_group_spawn(&mut m, "StaticMilitaryConvoy", "Abandoned_Test", x, z, y));
    let groups = places::event_groups(&m);
    let (_, _, children) = &groups["abandoned_test"];
    assert_eq!(children.len(), 3);
    assert_eq!((children[1].x, children[1].z), (-17.0, -29.0));
    assert_eq!(children[1].a, 350.0);
    assert!(children[0].y.is_none() && children[2].y == Some(2.0));
    assert!(children[0].deloot && children[0].loot == Some((2, 3)));
    let spawn = places::collect(&m, &[places::PlaceKind::EventSpawn]).pop().unwrap();
    assert_eq!(spawn.sub.as_deref(), Some("Abandoned_Test"));
    assert_eq!(spawn.y, Some(12.0));
    assert_eq!(children[1].world(spawn.x, spawn.z), [483.0, 571.0]);
    let issues = validate::check(&m, Some(4096.0));
    assert!(issues.iter().all(|i| !i.text.contains("group")), "{:?}", issues.iter().map(|i| i.message()).collect::<Vec<_>>());
    // Taking the group away is one edit back.
    let e = places::set_spawn_group(&mut m, &spawn.at, None, 0.0).unwrap();
    assert!(places::collect(&m, &[places::PlaceKind::EventSpawn])[0].sub.is_none());
    let mut boxed = Some(Box::new(m));
    toggle(&mut boxed, &mut vec![e]);
    assert!(places::collect(boxed.as_ref().unwrap(), &[places::PlaceKind::EventSpawn])[0].sub.is_some());
    let issues = validate::check(boxed.as_ref().unwrap(), Some(4096.0));
    let _ = edits;
    assert!(issues.iter().any(|i| i.text.contains("not placed")) == false);
}

impl Mission {
    /// An event of the kind the game's static events are, for tests.
    fn put_event_for_test(&mut self, name: &str) -> Vec<MissionEdit> {
        insert_child(self, "db/events.xml", &[], None, ce::EventRow::new_element(name)).into_iter().collect()
    }
}
