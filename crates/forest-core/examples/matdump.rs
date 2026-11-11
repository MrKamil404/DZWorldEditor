//! Checks the materials embedded in models against their RVMAT files.
//! `matdump <root> <folder or .p3d>...` prints every difference and a summary;
//! with `--list` it prints the embedded materials instead.
use std::path::Path;

fn walk(p: &Path, out: &mut Vec<std::path::PathBuf>) {
    if p.is_dir() {
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                walk(&e.path(), out);
            }
        }
    } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("p3d")) {
        out.push(p.to_path_buf());
    }
}

fn close(a: &[f32], b: &[f32]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let list = args.iter().any(|a| a == "--list");
    let args: Vec<String> = args.into_iter().filter(|a| a != "--list").collect();
    let root = Path::new(&args[0]);
    let mut files = Vec::new();
    for a in &args[1..] {
        walk(Path::new(a), &mut files);
    }
    let mut seen = std::collections::HashSet::new();
    let (mut same, mut differ, mut missing) = (0, 0, 0);
    for f in files {
        let Ok(b) = std::fs::read(&f) else { continue };
        let Ok(lods) = forest_core::odol::read(&b, |i, _| i == 0) else { continue };
        for m in &lods[0].materials {
            if !seen.insert(m.name.to_ascii_lowercase()) {
                continue;
            }
            if list {
                println!("{} ps={} flags={} amb={:?} dif={:?} fd={:?} em={:?} spec={:?} pow={} top={:?}", m.name, m.pixel_shader, m.render_flags, &m.ambient[..3], &m.diffuse[..3], &m.forced_diffuse[..3], &m.emissive[..3], &m.specular[..3], m.specular_power, m.top_projection);
                for (i, st) in m.stages.iter().enumerate() {
                    let t = m.transforms.get(st.transform as usize);
                    println!("   {i}: {} src={:?} xf={:?}", st.texture, t.map(|t| t.source), t.map(|t| [t.matrix[0], t.matrix[1], t.matrix[3], t.matrix[4], t.matrix[9], t.matrix[10]]));
                }
                continue;
            }
            let Some(t) = forest_core::rvmat::load(root, &m.name) else {
                missing += 1;
                continue;
            };
            let mut why = Vec::new();
            if t.pixel_shader != m.pixel_shader {
                why.push(format!("shader {} file {}", m.pixel_shader, t.pixel_shader));
            }
            if t.render_flags != m.render_flags {
                why.push(format!("flags {} file {}", m.render_flags, t.render_flags));
            }
            for (name, a, b) in [("emissive", m.emissive, t.emissive), ("ambient", m.ambient, t.ambient), ("diffuse", m.diffuse, t.diffuse), ("forced", m.forced_diffuse, t.forced_diffuse), ("specular", m.specular, t.specular)] {
                if !close(&a[..3], &b[..3]) {
                    why.push(format!("{name} {a:?} file {b:?}"));
                }
            }
            if (t.specular_power - m.specular_power).abs() > 1e-3 {
                why.push(format!("power {} file {}", m.specular_power, t.specular_power));
            }
            if t.top_projection.map(|p| p.0) != m.top_projection.map(|p| p.0) {
                why.push(format!("top {:?} file {:?}", m.top_projection, t.top_projection));
            }
            for (i, st) in t.stages.iter().enumerate().skip(1) {
                if st.texture.is_empty() {
                    continue;
                }
                match m.stages.get(i) {
                    Some(e) if e.texture.eq_ignore_ascii_case(&st.texture) => {
                        let (a, b) = (m.transforms.get(e.transform as usize), t.transforms.get(st.transform as usize));
                        if let (Some(a), Some(b)) = (a, b) {
                            if a.source != b.source || !close(&[a.matrix[0], a.matrix[1], a.matrix[3], a.matrix[4], a.matrix[9], a.matrix[10]], &[b.matrix[0], b.matrix[1], b.matrix[3], b.matrix[4], b.matrix[9], b.matrix[10]]) {
                                why.push(format!("stage {i} transform {a:?} file {b:?}"));
                            }
                        }
                    }
                    other => why.push(format!("stage {i} {:?} file {:?}", other.map(|e| &e.texture), st.texture)),
                }
            }
            if why.is_empty() {
                same += 1;
            } else {
                differ += 1;
                println!("{}: {}", m.name, why.join("; "));
            }
        }
    }
    eprintln!("same={same} differ={differ} missing={missing}");
}
