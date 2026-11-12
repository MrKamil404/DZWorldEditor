/// Road parts are source (MLOD) models whose length runs along +z from their origin; baked
/// into the satellite they lie where they stand on the map, when P: is mounted.
#[test]
fn a_baked_road_part_lies_where_it_stands() {
    if !std::path::Path::new("P:/dz/structures/roads/parts/asf2_12.p3d").exists() { return; }
    let db = dz_core::model_info::ModelDb::new("P:/");
    for yaw in [0.0f32, 30.0, 94.57, 274.57] {
        let o = dz_core::objects::Obj { x: 128.0, z: 128.0, elevation: 0.0, yaw, pitch: 0.0, roll: 0.0, scale: 1.0, model: 0, flags: 0 };
        let b = dz_core::satmod::BakeObject { object: o, origin_y: 0.0, model: "dz/structures/roads/parts/asf2_12.p3d".into() };
        let tiles = dz_core::satmod::bake_objects(&[b], &db, 256, 256, 256.0, |_| false, &dz_core::Job::default());
        let (mut sx, mut sz, mut n) = (0.0f64, 0.0f64, 0.0f64);
        for ((tx, ty), t) in &tiles {
            for y in 0..128 { for x in 0..128 {
                if t[(y * 128 + x) * 4 + 3] > 0 {
                    let (px, py) = (*tx as usize * 128 + x, *ty as usize * 128 + y);
                    sx += px as f64 + 0.5; sz += 256.0 - (py as f64 + 0.5); n += 1.0;
                }
            } }
        }
        let f = [yaw.to_radians().sin() as f64, yaw.to_radians().cos() as f64];
        let (ex, ez) = (128.0 + f[0] * 6.25, 128.0 + f[1] * 6.25);
        assert!(n > 50.0 && (sx / n - ex).hypot(sz / n - ez) < 0.5, "yaw {yaw}: baked middle ({:.2}, {:.2}) of {n} px, expected ({ex:.2}, {ez:.2})", sx / n, sz / n);
    }
}

/// A tree baked into the satellite leaves its crown in the image, when P: is mounted.
#[test]
fn a_baked_tree_shows_from_above() {
    if !std::path::Path::new("P:/dz/plants/tree/t_betulapendula_3s.p3d").exists() { return; }
    let db = dz_core::model_info::ModelDb::new("P:/");
    let o = dz_core::objects::Obj { x: 128.0, z: 128.0, elevation: 0.0, yaw: 0.0, pitch: 0.0, roll: 0.0, scale: 1.0, model: 0, flags: 0 };
    let b = dz_core::satmod::BakeObject { object: o, origin_y: 0.0, model: "dz/plants/tree/t_betulapendula_3s.p3d".into() };
    let tiles = dz_core::satmod::bake_objects(&[b], &db, 256, 256, 256.0, |_| true, &dz_core::Job::default());
    let n: usize = tiles.values().map(|t| t.chunks(4).filter(|p| p[3] > 0).count()).sum();
    assert!(n > 20, "{n} px");
}

/// A gate of the same set as its fence lies along the fence, centred on its line, when P: is mounted.
#[test]
fn a_real_gate_lies_along_its_fence() {
    use dz_core::library::{FenceGen, FenceOpening};
    if !std::path::Path::new("P:/dz/structures/walls/Wall_Fen2_6.p3d").exists() { return; }
    let db = dz_core::model_info::ModelDb::new("P:/");
    for (wall, gate, two) in [("dz/structures/walls/wall_fen2_6.p3d", "dz/structures/walls/wall_gate_fen2_l.p3d", true), ("dz/structures/walls/wall_feng_3.p3d", "dz/structures/walls/wall_gate_feng.p3d", false), ("dz/structures/walls/wall_feng_3.p3d", "dz/structures/walls/wall_gate_feng_open.p3d", false)] {
        let mut w = dz_core::world::World::new(dz_core::world::WorldMeta::new("g", 64, 4.0), 10.0);
        let opening = if two {
            FenceOpening { at: 50.0, gate: true, two_leaves: true, left: gate.into(), right: gate.replace("_l.", "_r."), ..Default::default() }
        } else {
            FenceOpening { at: 50.0, gate: true, model: gate.into(), ..Default::default() }
        };
        let g = FenceGen { model: wall.into(), openings: vec![opening], ..Default::default() };
        let objects = dz_core::gen::fence_objects(&mut w, &db, &[[0.0, 100.0], [100.0, 100.0]], &g);
        let wall_path = dz_core::objects::ObjectStore::normalize_path(wall);
        let named = |o: &&dz_core::objects::Obj| dz_core::objects::ObjectStore::normalize_path(&w.objects.models[o.model as usize].path) == wall_path;
        let walls: Vec<_> = objects.iter().filter(|o| named(o)).collect();
        let gates: Vec<_> = objects.iter().filter(|o| !named(o)).collect();
        assert!(!walls.is_empty(), "{wall}");
        assert!(!gates.is_empty(), "{gate}");
        for o in &gates {
            // Turned as the fence pieces are, and with the model's middle on the line.
            assert!(((o.yaw - walls[0].yaw).rem_euclid(180.0)).min(180.0 - (o.yaw - walls[0].yaw).rem_euclid(180.0)) < 0.5, "{gate}: yaw {} vs {}", o.yaw, walls[0].yaw);
            let i = db.get(&w.objects.models[o.model as usize].path);
            let m = dz_core::objects::rotation(o.yaw, o.pitch, o.roll, 1.0);
            let c = [(i.min[0] + i.max[0]) * 0.5, (i.min[2] + i.max[2]) * 0.5];
            let mid_z = o.z + m[0][2] * c[0] + m[2][2] * c[1];
            assert!((mid_z - 100.0).abs() < 0.3, "{gate}: middle {mid_z}");
        }
    }
}
