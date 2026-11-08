//! Eksport obiektów do pliku TXT dla Terrain Buildera.
//!
//! Format wiersza (potwierdzony dla TB / DayZ Tools, flaga underground
//! wymagana od pełnej aktualizacji DayZ):
//!   "nazwa_modelu";X;Y;Yaw;Pitch;Roll;Scale;Elevation;Underground;
//! Underground jest zawsze eksportowane jako 0.
//! X zawiera offset easting (klasycznie +200000). Nazwa modelu musi odpowiadać
//! wpisowi w Template Library. Przy imporcie wybierz odpowiednio
//! "relative" (Elevation=0) lub "absolute" (wysokość z heightmapy).

use std::io::Write;

use crate::mask::MaskImage;
use crate::preset::{ForestProject, PngMode, PngShape};
use crate::scatter::{hash2, PlacedObject};

/// Wzorzec dysku (koła) do rysowania obiektów - identyczny jak w podglądzie.
fn dot_stencil(radius: f32) -> Vec<(i64, i64)> {
    let ri = radius.ceil() as i64;
    let r2 = radius * radius;
    let mut v = Vec::new();
    for dy in -ri..=ri {
        for dx in -ri..=ri {
            if ((dx * dx + dy * dy) as f32) <= r2 + 0.25 {
                v.push((dx, dy));
            }
        }
    }
    v
}

pub fn write_tb_txt(
    objects: &[PlacedObject],
    project: &ForestProject,
    out: &mut dyn Write,
) -> std::io::Result<usize> {
    let mut n = 0usize;
    for o in objects {
        writeln!(
            out,
            "\"{}\";{:.6};{:.6};{:.6};{:.6};{:.6};{:.6};{:.6};0;",
            o.model,
            o.x + project.easting_offset,
            o.y + project.northing_offset,
            o.yaw,
            o.pitch,
            o.roll,
            o.scale,
            o.elevation
        )?;
        n += 1;
    }
    Ok(n)
}

pub fn write_tb_file(
    objects: &[PlacedObject],
    project: &ForestProject,
    path: impl AsRef<std::path::Path>,
) -> Result<usize, String> {
    let file = std::fs::File::create(path)
        .map_err(|e| format!("Nie można utworzyć pliku wyjściowego: {e}"))?;
    let mut buf = std::io::BufWriter::new(file);
    write_tb_txt(objects, project, &mut buf).map_err(|e| format!("Zapis TXT: {e}"))
}

/// Wynik importu pliku TB TXT (tego samego formatu, który produkuje eksport).
#[derive(Clone, Debug, Default)]
pub struct TbImport {
    pub objects: Vec<PlacedObject>,
    /// Pominięte wiersze: (numer wiersza, powód).
    pub skipped: Vec<(usize, String)>,
    /// Obiekty o modelu spoza biblioteki gatunków projektu (białe na podglądzie,
    /// eksportują się z powrotem bez zmian).
    pub unknown_models: usize,
}

/// Parsuje plik TB TXT do obiektów (odwrotność `write_tb_txt`):
/// X/Y są w układzie mapy (z offsetami TB) — odejmujemy offsety projektu.
/// Gatunek rozpoznawany po nazwie modelu (dopasowanie znormalizowane:
/// bez `.p3d`/ścieżki, bez względu na wielkość liter); nieznane modele
/// dostają `species_index = usize::MAX` (bezpieczne: podgląd i eksport
/// używają fallbacków, a generowanie i tak buduje obiekty od nowa).
pub fn parse_tb_txt(text: &str, project: &ForestProject) -> Result<TbImport, String> {
    use crate::scatter::MAX_TOTAL_OBJECTS;
    use crate::species::find_species_in;
    let mut out = TbImport::default();
    for (li, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let no = li + 1;
        let mut fail = |why: &str| out.skipped.push((no, why.into()));
        // "model";X;Y;yaw;pitch;roll;scale;elev;underground;(końcowy pusty)
        let parts: Vec<&str> = line.split(';').collect();
        if parts.len() < 9 {
            fail("za mało pól (oczekiwano 9)");
            continue;
        }
        let model = parts[0].trim().trim_matches('"');
        if model.is_empty() {
            fail("pusta nazwa modelu");
            continue;
        }
        let mut nums = [0.0f64; 7];
        let mut bad: Option<usize> = None;
        for (k, v) in parts[1..8].iter().enumerate() {
            match v.trim().parse::<f64>() {
                Ok(n) => nums[k] = n,
                Err(_) => {
                    bad = Some(k + 1);
                    break;
                }
            }
        }
        if let Some(k) = bad {
            fail(&format!("pole {k} nie jest liczbą"));
            continue;
        }
        if out.objects.len() >= MAX_TOTAL_OBJECTS {
            return Err(format!(
                "Plik ma więcej niż {MAX_TOTAL_OBJECTS} obiektów — limit jak przy generowaniu"
            ));
        }
        let species_index = find_species_in(&project.species, model).unwrap_or(usize::MAX);
        if species_index == usize::MAX {
            out.unknown_models += 1;
        }
        out.objects.push(PlacedObject {
            model: model.into(),
            x: nums[0] - project.easting_offset,
            y: nums[1] - project.northing_offset,
            yaw: nums[2],
            pitch: nums[3],
            roll: nums[4],
            scale: nums[5],
            elevation: nums[6],
            zone_index: 0,
            species_index,
        });
    }
    if out.objects.is_empty() {
        return Err("Brak poprawnych wierszy obiektów w pliku".into());
    }
    Ok(out)
}

/// Eksportuje warstwę drzew jako przezroczysty PNG (rozdzielczość `res`x`res`).
/// `colors[i]` = kolor gatunku o indeksie `i`. Zwraca liczbę narysowanych obiektów.
pub fn export_trees_png(
    objects: &[PlacedObject],
    project: &ForestProject,
    colors: &[[u8; 3]],
    res: u32,
    path: impl AsRef<std::path::Path>,
) -> Result<u32, String> {
    if res == 0 {
        return Err("Rozdzielczość musi być > 0".into());
    }
    let mut rgba = vec![0u8; (res as usize) * (res as usize) * 4];
    let map = project.map_size_m;
    let sx = f64::from(res) / map;
    let sy = f64::from(res) / map;

    // Tryb Preview: identyczny rendering jak podgląd w programie (koło 2px)
    if project.png_settings.mode == PngMode::Preview {
        let stencil = dot_stencil(2.0);
        let mut drawn: u32 = 0;
        for o in objects {
            let px = (o.x * sx) as i64;
            let py = ((map - o.y) * sy) as i64;
            let c = colors.get(o.species_index).copied().unwrap_or([255, 255, 255]);
            for &(dx, dy) in &stencil {
                let x = px + dx;
                let y = py + dy;
                if x >= 0 && y >= 0 && (x as u32) < res && (y as u32) < res {
                    let idx = ((y as usize) * (res as usize) + (x as usize)) * 4;
                    rgba[idx] = c[0];
                    rgba[idx + 1] = c[1];
                    rgba[idx + 2] = c[2];
                    rgba[idx + 3] = 255;
                }
            }
            drawn += 1;
        }
        image::save_buffer(path, &rgba, res, res, image::ColorType::Rgba8)
            .map_err(|e| format!("Zapis PNG: {e}"))?;
        return Ok(drawn);
    }

    let base_r = (project.png_settings.dot_size_m * sx).max(1.0);
    let mut drawn: u32 = 0;

    for o in objects {
        let px = (o.x * sx) as i64;
        let py = ((map - o.y) * sy) as i64;
        let c = colors.get(o.species_index).copied().unwrap_or([255, 255, 255]);

        // randomizacja rozmiaru
        let size_mult = if project.png_settings.randomize_size > 0.0 {
            let rng = hash2(o.x, o.y, project.seed);
            1.0 + (rng - 0.5) * 2.0 * project.png_settings.randomize_size
        } else {
            1.0
        };
        let r = (base_r * size_mult).max(0.5);
        let ri = r.ceil() as i64;

        // randomizacja rotacji (dla Square/Diamond)
        let angle = if project.png_settings.randomize_rotation {
            let rng = hash2(o.x + 1000.0, o.y + 1000.0, project.seed);
            rng * std::f64::consts::TAU
        } else {
            match project.png_settings.shape {
                PngShape::Diamond => std::f64::consts::FRAC_PI_4,
                _ => 0.0,
            }
        };

        let cos_a = angle.cos();
        let sin_a = angle.sin();

        for dy in -ri..=ri {
            for dx in -ri..=ri {
                // obróć punkt
                let rx = dx as f64 * cos_a - dy as f64 * sin_a;
                let ry = dx as f64 * sin_a + dy as f64 * cos_a;

                let inside = match project.png_settings.shape {
                    PngShape::Circle => (rx * rx + ry * ry) <= r * r,
                    PngShape::Square => rx.abs() <= r && ry.abs() <= r,
                    PngShape::Diamond => rx.abs() + ry.abs() <= r,
                    PngShape::Blob => {
                        let dist = (rx * rx + ry * ry).sqrt();
                        // nieregularna plama - promień zależny od kąta
                        let angle = ry.atan2(rx);
                        let phase1 = hash2(o.x + 11.0, o.y + 17.0, project.seed) * std::f64::consts::TAU;
                        let phase2 = hash2(o.x + 91.0, o.y + 33.0, project.seed) * std::f64::consts::TAU;
                        let wobble = 0.30 * (angle * 3.0 + phase1).sin()
                            + 0.18 * (angle * 5.0 + phase2).cos()
                            + 0.12 * (angle * 7.0 - phase1 * 0.5).sin();
                        let r_eff = r * (1.0 + wobble).clamp(0.65, 1.35);
                        dist <= r_eff
                    }
                };

                if inside {
                    let x = px + dx;
                    let y = py + dy;
                    if x >= 0 && y >= 0 && (x as u32) < res && (y as u32) < res {
                        let idx = ((y as usize) * (res as usize) + (x as usize)) * 4;
                        rgba[idx] = c[0];
                        rgba[idx + 1] = c[1];
                        rgba[idx + 2] = c[2];
                        rgba[idx + 3] = 255;
                    }
                }
            }
        }
        drawn += 1;
    }

    image::save_buffer(
        path,
        &rgba,
        res,
        res,
        image::ColorType::Rgba8,
    )
    .map_err(|e| format!("Zapis PNG: {e}"))?;
    Ok(drawn)
}

/// Eksportuje warstwę drzew jako PNG z kolorami warstw (layers.cfg).
/// `species_layer_map` = mapa znormalizowany model TB → nazwa warstwy
/// (kluczowanie modelem, nie indeksem — odporne na import/dodawanie gatunków).
/// `layer_colors` = mapa nazwa warstwy → kolor RGB.
/// Zwraca liczbę narysowanych obiektów.
pub fn export_trees_png_with_layers(
    objects: &[PlacedObject],
    project: &ForestProject,
    species_layer_map: &std::collections::HashMap<String, String>,
    layer_colors: &std::collections::HashMap<String, [u8; 3]>,
    res: u32,
    path: impl AsRef<std::path::Path>,
) -> Result<u32, String> {
    // Tryb Zones: renderuj strefy jako duże plamy z kolorami warstw
    if project.png_settings.mode == PngMode::Zones {
        return export_zones_png_with_layers(project, species_layer_map, layer_colors, res, path);
    }
    // Tryb Preview: identyczny jak podgląd (koło 2px, bez randomizacji) ale z kolorami warstw
    if project.png_settings.mode == PngMode::Preview {
        if res == 0 {
            return Err("Rozdzielczość musi być > 0".into());
        }
        let mut rgba = vec![0u8; (res as usize) * (res as usize) * 4];
        let map = project.map_size_m;
        let sx = f64::from(res) / map;
        let sy = f64::from(res) / map;
        let stencil = dot_stencil(2.0);
        let mut drawn: u32 = 0;
        for o in objects {
            let px = (o.x * sx) as i64;
            let py = ((map - o.y) * sy) as i64;
            // warstwa po MODELU TB (nie po indeksie gatunku)
            let c = species_layer_map
                .get(&crate::species::normalize_model_name(&o.model))
                .and_then(|layer_name| layer_colors.get(layer_name))
                .copied()
                .unwrap_or([255, 255, 255]);
            for &(dx, dy) in &stencil {
                let x = px + dx;
                let y = py + dy;
                if x >= 0 && y >= 0 && (x as u32) < res && (y as u32) < res {
                    let idx = ((y as usize) * (res as usize) + (x as usize)) * 4;
                    rgba[idx] = c[0];
                    rgba[idx + 1] = c[1];
                    rgba[idx + 2] = c[2];
                    rgba[idx + 3] = 255;
                }
            }
            drawn += 1;
        }
        image::save_buffer(path, &rgba, res, res, image::ColorType::Rgba8)
            .map_err(|e| format!("Zapis PNG: {e}"))?;
        return Ok(drawn);
    }

    if res == 0 {
        return Err("Rozdzielczość musi być > 0".into());
    }
    let mut rgba = vec![0u8; (res as usize) * (res as usize) * 4];
    let map = project.map_size_m;
    let sx = f64::from(res) / map;
    let sy = f64::from(res) / map;

    let base_r = (project.png_settings.dot_size_m * sx).max(1.0);
    let mut drawn: u32 = 0;

    for o in objects {
        let px = (o.x * sx) as i64;
        let py = ((map - o.y) * sy) as i64;

        // pobierz kolor z warstwy przypisanej do MODELU TB obiektu
        let c = species_layer_map
            .get(&crate::species::normalize_model_name(&o.model))
            .and_then(|layer_name| layer_colors.get(layer_name))
            .copied()
            .unwrap_or([255, 255, 255]);

        // randomizacja rozmiaru
        let size_mult = if project.png_settings.randomize_size > 0.0 {
            let rng = hash2(o.x, o.y, project.seed);
            1.0 + (rng - 0.5) * 2.0 * project.png_settings.randomize_size
        } else {
            1.0
        };
        let r = (base_r * size_mult).max(0.5);
        let ri = r.ceil() as i64;

        // randomizacja rotacji (dla Square/Diamond)
        let angle = if project.png_settings.randomize_rotation {
            let rng = hash2(o.x + 1000.0, o.y + 1000.0, project.seed);
            rng * std::f64::consts::TAU
        } else {
            match project.png_settings.shape {
                PngShape::Diamond => std::f64::consts::FRAC_PI_4,
                _ => 0.0,
            }
        };

        let cos_a = angle.cos();
        let sin_a = angle.sin();

        for dy in -ri..=ri {
            for dx in -ri..=ri {
                // obróć punkt
                let rx = dx as f64 * cos_a - dy as f64 * sin_a;
                let ry = dx as f64 * sin_a + dy as f64 * cos_a;

                let inside = match project.png_settings.shape {
                    PngShape::Circle => (rx * rx + ry * ry) <= r * r,
                    PngShape::Square => rx.abs() <= r && ry.abs() <= r,
                    PngShape::Diamond => rx.abs() + ry.abs() <= r,
                    PngShape::Blob => {
                        let dist = (rx * rx + ry * ry).sqrt();
                        let angle = ry.atan2(rx);
                        let phase1 = hash2(o.x + 11.0, o.y + 17.0, project.seed) * std::f64::consts::TAU;
                        let phase2 = hash2(o.x + 91.0, o.y + 33.0, project.seed) * std::f64::consts::TAU;
                        let wobble = 0.30 * (angle * 3.0 + phase1).sin()
                            + 0.18 * (angle * 5.0 + phase2).cos()
                            + 0.12 * (angle * 7.0 - phase1 * 0.5).sin();
                        let r_eff = r * (1.0 + wobble).clamp(0.65, 1.35);
                        dist <= r_eff
                    }
                };

                if inside {
                    let x = px + dx;
                    let y = py + dy;
                    if x >= 0 && y >= 0 && (x as u32) < res && (y as u32) < res {
                        let idx = ((y as usize) * (res as usize) + (x as usize)) * 4;
                        rgba[idx] = c[0];
                        rgba[idx + 1] = c[1];
                        rgba[idx + 2] = c[2];
                        rgba[idx + 3] = 255;
                    }
                }
            }
        }
        drawn += 1;
    }

    image::save_buffer(
        path,
        &rgba,
        res,
        res,
        image::ColorType::Rgba8,
    )
    .map_err(|e| format!("Zapis PNG: {e}"))?;
    Ok(drawn)
}

/// Eksport warstw stref jako dużych plam z kolorami warstw (tryb Zones dla warstw).
fn export_zones_png_with_layers(
    project: &ForestProject,
    species_layer_map: &std::collections::HashMap<String, String>,
    layer_colors: &std::collections::HashMap<String, [u8; 3]>,
    res: u32,
    path: impl AsRef<std::path::Path>,
) -> Result<u32, String> {
    if res == 0 {
        return Err("Rozdzielczość musi być > 0".into());
    }
    // Zbuduj kolory stref z warstw: kolor strefy = kolor warstwy pierwszego gatunku strefy
    let zone_colors: Vec<[u8; 3]> = project
        .zones
        .iter()
        .map(|z| {
            z.species_weights
                .first()
                .and_then(|(si, _)| project.species.get(*si))
                .and_then(|sp| {
                    species_layer_map.get(&crate::species::normalize_model_name(&sp.model))
                })
                .and_then(|ln| layer_colors.get(ln))
                .copied()
                .unwrap_or([255, 255, 255])
        })
        .collect();

    // Jeśli brak maski, nie możemy renderować stref – zwróć błąd
    // Szukamy maski w pliku: spróbuj wczytać z project.paths.mask jeśli istnieje
    if let Some(mask_path) = &project.paths.mask {
        if let Ok(mask) = MaskImage::load(mask_path) {
            return export_zones_png(&mask, project, &zone_colors, res, path);
        }
    }
    Err("Brak maski — tryb Strefy wymaga wczytanej maski.".into())
}

/// Eksportuje warstwę stref lasu jako PNG (tryb Zones).
/// Renderuje maskę z kolorami stref zamiast pojedynczych drzew.
pub fn export_zones_png(
    mask: &MaskImage,
    project: &ForestProject,
    colors: &[[u8; 3]],
    res: u32,
    path: impl AsRef<std::path::Path>,
) -> Result<u32, String> {
    if res == 0 {
        return Err("Rozdzielczość musi być > 0".into());
    }

    let mut rgba = vec![0u8; (res as usize) * (res as usize) * 4];
    let map = project.map_size_m;
    let sx = f64::from(res) / map;
    let sy = f64::from(res) / map;

    // Skalowanie z maski do wyjściowej rozdzielczości
    let mask_sx = f64::from(mask.width) / map;
    let mask_sy = f64::from(mask.height) / map;

    let mut drawn: u32 = 0;

    for py in 0..res {
        for px in 0..res {
            // Mapuj piksel wyjściowy na współrzędne świata
            let wx = f64::from(px) / sx;
            let wy = map - f64::from(py) / sy;

            // Mapuj na piksel maski
            let mx = (wx * mask_sx) as u32;
            let my = ((map - wy) * mask_sy) as u32;

            if mx < mask.width && my < mask.height {
                let pixel = mask.pixel(mx, my);
                // Znajdź strefę dla tego koloru
                if let Some(zone_idx) = project.zones.iter().position(|z| {
                    let dr = (pixel.0[0] as i16 - z.color.0[0] as i16).abs();
                    let dg = (pixel.0[1] as i16 - z.color.0[1] as i16).abs();
                    let db = (pixel.0[2] as i16 - z.color.0[2] as i16).abs();
                    (dr + dg + db) <= project.color_tolerance as i16
                }) {
                    let c = colors.get(zone_idx).copied().unwrap_or([255, 255, 255]);
                    let out_idx = (py as usize * res as usize + px as usize) * 4;
                    rgba[out_idx] = c[0];
                    rgba[out_idx + 1] = c[1];
                    rgba[out_idx + 2] = c[2];
                    rgba[out_idx + 3] = 255;
                    drawn += 1;
                }
            }
        }
    }

    image::save_buffer(
        path,
        &rgba,
        res,
        res,
        image::ColorType::Rgba8,
    )
    .map_err(|e| format!("Zapis PNG: {e}"))?;
    Ok(drawn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preset::{ForestProject, ZoneDef};
    use crate::mask::Rgb8;

    #[test]
    fn line_format_matches_tb_spec() {
        let proj = ForestProject {
            easting_offset: 200_000.0,
            northing_offset: 0.0,
            zones: vec![ZoneDef { color: Rgb8([0,0,0]), ..Default::default() }],
            ..Default::default()
        };
        let obj = PlacedObject {
            model: "t_BetulaPendula_2f".into(),
            x: 456.296875,
            y: 1032.300049,
            yaw: 250.0,
            pitch: -1.5,
            roll: 2.25,
            scale: 1.0,
            elevation: 5.686351,
            zone_index: 0,
            species_index: 0,
        };
        let mut buf: Vec<u8> = Vec::new();
        let n = write_tb_txt(&[obj], &proj, &mut buf).unwrap();
        assert_eq!(n, 1);
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(
            s,
            "\"t_BetulaPendula_2f\";200456.296875;1032.300049;250.000000;-1.500000;2.250000;1.000000;5.686351;0;\n"
        );
    }

    #[test]
    fn northing_offset_applied() {
        let proj = ForestProject {
            easting_offset: 200_000.0,
            northing_offset: 500_000.0,
            zones: vec![ZoneDef { color: Rgb8([0,0,0]), ..Default::default() }],
            ..Default::default()
        };
        let obj = PlacedObject {
            model: "x".into(),
            x: 1.0,
            y: 2.0,
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            scale: 1.0,
            elevation: 0.0,
            zone_index: 0,
            species_index: 0,
        };
        let mut buf: Vec<u8> = Vec::new();
        write_tb_txt(&[obj], &proj, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        let parts: Vec<&str> = s.trim_end().split(';').collect();
        assert_eq!(parts[1], "200001.000000");
        assert_eq!(parts[2], "500002.000000");
    }

    #[test]
    fn trees_png_transparent_with_colored_dots() {
        let proj = ForestProject {
            map_size_m: 1000.0,
            zones: vec![ZoneDef { color: Rgb8([0,0,0]), ..Default::default() }],
            ..Default::default()
        };
        let objs = vec![
            PlacedObject { model: "a".into(), x: 500.0, y: 500.0, yaw: 0.0, pitch: 0.0, roll: 0.0, scale: 1.0, elevation: 0.0, zone_index: 0, species_index: 0 },
        ];
        let colors = [[255, 0, 0]];
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trees.png");
        let n = export_trees_png(&objs, &proj, &colors, 100, &path).unwrap();
        assert_eq!(n, 1);

        let img = image::open(&path).unwrap().to_rgba8();
        assert_eq!((img.width(), img.height()), (100, 100));
        // środek ma czerwoną kropkę, a daleki róg jest przezroczysty (alpha=0)
        let center = img.get_pixel(50, 50);
        assert_eq!([center[0], center[1], center[2], center[3]], [255, 0, 0, 255]);
        let corner = img.get_pixel(5, 5);
        assert_eq!(corner[3], 0);
    }

    #[test]
    fn export_import_roundtrip() {
        use crate::species::SpeciesDef;
        let proj = ForestProject {
            easting_offset: 200_000.0,
            northing_offset: 0.0,
            species: vec![SpeciesDef::new("Brzoza", "t_BetulaPendula_2f", 0.9, 1.1)],
            ..Default::default()
        };
        let objs = vec![
            PlacedObject {
                model: "t_BetulaPendula_2f".into(),
                x: 456.5,
                y: 1032.25,
                yaw: 10.0,
                pitch: 0.0,
                roll: 0.0,
                scale: 1.0,
                elevation: 3.5,
                zone_index: 0,
                species_index: 0,
            },
            PlacedObject {
                model: "p_UnknownTree".into(),
                x: 100.0,
                y: 200.0,
                yaw: 0.0,
                pitch: 0.0,
                roll: 0.0,
                scale: 1.0,
                elevation: 0.0,
                zone_index: 0,
                species_index: usize::MAX,
            },
        ];
        let mut buf: Vec<u8> = Vec::new();
        write_tb_txt(&objs, &proj, &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        let imp = parse_tb_txt(&text, &proj).unwrap();
        assert_eq!(imp.objects.len(), 2);
        assert_eq!(imp.unknown_models, 1);
        assert!(imp.skipped.is_empty());
        // offsety zdjęte z powrotem (z dokładnością formatu %.6f)
        assert!((imp.objects[0].x - 456.5).abs() < 1e-6);
        assert!((imp.objects[0].y - 1032.25).abs() < 1e-6);
        assert_eq!(imp.objects[0].species_index, 0);
        assert_eq!(imp.objects[1].species_index, usize::MAX);
        assert_eq!(imp.objects[0].elevation, 3.5);
    }

    #[test]
    fn import_skips_garbage_but_keeps_good_rows() {
        let proj = ForestProject {
            easting_offset: 200_000.0,
            ..Default::default()
        };
        let text = "śmieci bez średników\n\
            \"x\";200001.000000;2.000000;0;0;0;1;0;0;\n\
            \"y\";nie-liczba;2;0;0;0;1;0;0;\n\
            \n";
        let imp = parse_tb_txt(text, &proj).unwrap();
        assert_eq!(imp.objects.len(), 1);
        assert_eq!(imp.objects[0].model, "x");
        assert_eq!(imp.skipped.len(), 2);
    }

    #[test]
    fn import_empty_is_error() {
        let proj = ForestProject::default();
        assert!(parse_tb_txt("", &proj).is_err());
        assert!(parse_tb_txt("   \n\n", &proj).is_err());
    }

    #[test]
    fn import_matches_despite_case_p3d_and_path() {
        use crate::species::SpeciesDef;
        let proj = ForestProject {
            easting_offset: 0.0,
            northing_offset: 0.0,
            species: vec![SpeciesDef::new("Brzoza", "t_BetulaPendula_2f", 0.9, 1.1)],
            ..Default::default()
        };
        let text = "\"T_BETULAPENDULA_2F.P3D\";1;2;0;0;0;1;0;0;\n\
            \"dz\\plants\\tree\\t_betulapendula_2f\";3;4;0;0;0;1;0;0;\n";
        let imp = parse_tb_txt(text, &proj).unwrap();
        assert_eq!(imp.objects.len(), 2);
        assert_eq!(imp.unknown_models, 0);
        assert!(imp.objects.iter().all(|o| o.species_index == 0));
    }
}
