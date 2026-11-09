//! Generates a small map, exports it and builds its PBO, without the editor.
use dz_core::*;
use std::time::Instant;
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let id = args.get(1).cloned().unwrap_or("DZWETest".into());
    let paths = build::detect();
    println!("{paths:?}\n{:?}", paths.check());
    let job = Job::default();
    let mut w = world::World::new(world::WorldMeta::new(&id, 1024, 6.0), 60.0);
    w.meta.author = "DZ World Editor".into();
    gen::generate_terrain(&mut w.base, &gen::TerrainParams { kind: gen::TerrainKind::Island, ..Default::default() }, &job);
    w.recompose(None);
    let rules = gen::default_paint_rules(&w);
    w.mask.data = gen::auto_paint(&w, &rules, 1, None, &job);
    let db = model_info::ModelDb::new(paths.work_drive.clone());
    println!("objects {}", w.objects.len());
    let t = Instant::now();
    let r = export::run(&w, &db, &export::ExportOptions { root: paths.work_drive.clone(), layers: true, world: true, config: true, source: false }, &job)?;
    println!("export {:?} in {:?}", r, t.elapsed());
    let t = Instant::now();
    let res = build::build_pbo(&paths, &w.meta.map_id, &job);
    for l in job.take_log() { println!("{l}"); }
    println!("{:?} in {:?}", res, t.elapsed());
    Ok(())
}
