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
