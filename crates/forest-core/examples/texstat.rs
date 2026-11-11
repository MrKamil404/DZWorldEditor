//! Lists what each visual LOD of the given models is made of: its textures and, per material,
//! the faces using it. With `--png <dir>` the textures of the last LOD are saved as images,
//! which shows the look the game bakes for the model seen from afar.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = args.iter().position(|a| a == "--png").and_then(|i| args.get(i + 1)).cloned();
    for a in args.iter().filter(|a| a.to_ascii_lowercase().ends_with(".p3d")) {
        let Ok(b) = std::fs::read(a) else { continue };
        let Ok(lods) = forest_core::odol::read(&b, |_, r| r < 1000.0) else {
            println!("{a}: unreadable");
            continue;
        };
        for (k, l) in lods.iter().enumerate() {
            println!("{a} lod {}: vertices {} normals {}", l.resolution, l.vertices.len(), l.normals.len());
            let mut uses = std::collections::BTreeMap::new();
            for (t, m) in l.face_textures.iter().zip(&l.face_materials) {
                *uses.entry((*t, *m)).or_insert(0usize) += 1;
            }
            for ((t, m), n) in uses {
                let texture = l.textures.get(t as usize).map(String::as_str).unwrap_or("-");
                let material = usize::try_from(m).ok().and_then(|i| l.materials.get(i)).map(|m| m.name.as_str()).unwrap_or("-");
                println!("   {n:6} faces  {texture}  |  {material}");
            }
            if let (Some(d), true) = (&dir, k + 1 == lods.len()) {
                for t in &l.textures {
                    let file = format!("P:/{}", t.replace('\\', "/"));
                    let Some(img) = std::fs::read(&file).ok().and_then(|b| forest_core::paa::decode(&b, 512).ok()) else { continue };
                    let rgb: Vec<u8> = img.rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
                    let name = std::path::Path::new(&file).file_stem().unwrap().to_string_lossy().to_string();
                    let _ = image::save_buffer(format!("{d}/{name}.png"), &rgb, img.width, img.height, image::ColorType::Rgb8);
                }
            }
        }
    }
}
