//! Makes a project of a real place without the editor.
//!
//! cargo run --release -p dz-core --example real_terrain -- <lat> <lon> <vertices> <cell> <out dir> [source index] [metres per pixel]
use dz_core::real_terrain::{build_world, satellite_sources, Fetcher, OsmParts, RealMap};
use dz_core::Job;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 6 {
        anyhow::bail!("usage: real_terrain <lat> <lon> <vertices> <cell> <out dir> [source index] [metres per pixel]");
    }
    let source = a.get(6).and_then(|s| s.parse().ok()).unwrap_or(0usize);
    let mpp = a.get(7).and_then(|s| s.parse().ok()).unwrap_or(1.0f32);
    let out = PathBuf::from(&a[5]);
    let m = RealMap {
        name: "RealTest".into(),
        lat: a[1].parse()?,
        lon: a[2].parse()?,
        terrain_size: a[3].parse()?,
        cell: a[4].parse()?,
        satellite: Some((satellite_sources()[source].clone(), mpp)),
        heights: true,
        osm: Some(OsmParts::default()),
        mask: true,
    };
    let f = Fetcher::new(std::env::var_os("DZWE_TILE_CACHE").map(PathBuf::from))?;
    let job = Job::default();
    let t = std::time::Instant::now();
    let w = build_world(&m, &f, &job)?;
    for l in job.take_log() {
        println!("{l}");
    }
    println!("built in {:.1} s", t.elapsed().as_secs_f32());
    let (lo, hi) = w.height.min_max();
    println!("heights {lo:.1} .. {hi:.1} m");
    for l in &w.shape_layers {
        println!("{}: {} shapes", l.name, l.shapes.len());
    }
    let mut count = vec![0usize; w.layers.len()];
    for v in &w.mask.data {
        count[*v as usize] += 1;
    }
    for (l, c) in w.layers.iter().zip(&count) {
        println!("{:12} {:5.1} %", l.name, *c as f32 * 100.0 / w.mask.data.len() as f32);
    }
    std::fs::create_dir_all(&out)?;
    if let Some(s) = &w.satellite {
        s.save_png(&out.join("satellite.png"))?;
    }
    let mut mask = dz_core::terrain::RgbImage::new(w.mask.width, w.mask.height, [0, 0, 0]);
    for (i, v) in w.mask.data.iter().enumerate() {
        mask.data[i * 3..i * 3 + 3].copy_from_slice(&w.layers[*v as usize].color);
    }
    mask.save_png(&out.join("mask.png"))?;
    let n = w.height.cols;
    let mut hm = dz_core::terrain::RgbImage::new(n, n, [0, 0, 0]);
    for j in 0..n {
        for i in 0..n {
            let v = ((w.height.at(i, j) - lo) / (hi - lo).max(1.0) * 255.0) as u8;
            let p = ((n - 1 - j) * n + i) * 3;
            hm.data[p..p + 3].copy_from_slice(&[v, v, v]);
        }
    }
    hm.save_png(&out.join("heights.png"))?;
    dz_core::project::save(&w, &out.join("project"), dz_core::project::SaveParts::ALL)?;
    Ok(())
}
