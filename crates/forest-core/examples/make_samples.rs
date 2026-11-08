//! Generuje przykładowe pliki do assets/samples/:
//! maska PNG + heightmapa ASC + wykluczenia GeoJSON + gotowy projekt JSON.
//!
//! Uruchomienie z katalogu głównego repo: cargo run -p forest-core --example make_samples

use forest_core::mask::Rgb8;
use forest_core::preset::{ElevationMode, EdgeSettings, ForestProject, ProjectPaths, ZoneDef};
use forest_core::species::vanilla_library;

const MASK_PX: u32 = 512;
const MAP_M: f64 = 15360.0;
const ASC_N: usize = 128;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = std::path::Path::new("assets/samples");
    std::fs::create_dir_all(out_dir)?;

    // --- maska ---------------------------------------------------------------
    let mut img = image::RgbaImage::from_pixel(MASK_PX, MASK_PX, image::Rgba([150, 150, 150, 255]));

    let conifer = Rgb8([0, 120, 60]);
    let mixed = Rgb8([0, 220, 80]);
    let road = Rgb8([25, 25, 25]);
    let lake = Rgb8([0, 90, 255]);

    for y in 0..MASK_PX {
        for x in 0..MASK_PX {
            let fx = f64::from(x);
            let fy = f64::from(y);

            // droga: pozioma pas + pionowa kręta
            let road_h = (fy - 300.0).abs() < 4.0 + 2.0 * (fx * 0.05).sin();
            let road_v = (fx - (180.0 + 40.0 * (fy * 0.03).sin())).abs() < 4.0;
            if road_h || road_v {
                img.put_pixel(x, y, image::Rgba([road.r(), road.g(), road.b(), 255]));
                continue;
            }

            // jezioro
            let dl = ((fx - 330.0).powi(2) + (fy - 150.0).powi(2)).sqrt();
            if dl < 45.0 + 6.0 * (fx * 0.08).sin() * (fy * 0.06).cos() {
                img.put_pixel(x, y, image::Rgba([lake.r(), lake.g(), lake.b(), 255]));
                continue;
            }

            // las iglasty: masyw NE z organically poszarpaną krawędzią
            let dc = ((fx - 380.0).powi(2) + (fy - 110.0).powi(2)).sqrt();
            let wob_c = 18.0 * (fx * 0.04 + fy * 0.03).sin() + 10.0 * (fx * 0.11).cos();
            if dc < 170.0 + wob_c {
                img.put_pixel(x, y, image::Rgba([conifer.r(), conifer.g(), conifer.b(), 255]));
                continue;
            }

            // las mieszany: kilka płatów SW
            let dm1 = ((fx - 110.0).powi(2) + (fy - 380.0).powi(2)).sqrt();
            let dm2 = ((fx - 260.0).powi(2) + (fy - 430.0).powi(2)).sqrt();
            let wob_m = 14.0 * (fx * 0.05 - fy * 0.04).sin();
            if dm1 < 95.0 + wob_m || dm2 < 70.0 + wob_m {
                img.put_pixel(x, y, image::Rgba([mixed.r(), mixed.g(), mixed.b(), 255]));
            }
        }
    }
    let mask_path = out_dir.join("sample_mask.png");
    img.save(&mask_path)?;
    println!("OK {}", mask_path.display());

    // --- heightmapa ASC --------------------------------------------------------
    let cellsize = MAP_M / ASC_N as f64; // 120 m
    let mut asc = String::new();
    asc.push_str(&format!("ncols {ASC_N}\n"));
    asc.push_str(&format!("nrows {ASC_N}\n"));
    asc.push_str("xllcorner 0\nyllcorner 0\n");
    asc.push_str(&format!("cellsize {cellsize}\n"));
    asc.push_str("NODATA_value -9999\n");
    for row in 0..ASC_N {
        // row 0 = północ => y świata największe
        let wy = MAP_M - row as f64 * cellsize;
        let mut line = String::new();
        for col in 0..ASC_N {
            let wx = col as f64 * cellsize;
            let h = 85.0
                + 55.0 * (wx * 0.00021).sin() * (wy * 0.00017).cos()
                + 22.0 * (wx * 0.0007 + wy * 0.0005).sin()
                + 6.0 * (wx * 0.003).sin() * (wy * 0.0027).cos();
            line.push_str(&format!("{h:.2} "));
        }
        asc.push_str(line.trim_end());
        asc.push('\n');
    }
    let asc_path = out_dir.join("sample_heightmap.asc");
    std::fs::write(&asc_path, asc)?;
    println!("OK {}", asc_path.display());

    // --- wykluczenia GeoJSON ----------------------------------------------------
    let gj = r#"{
  "type": "FeatureCollection",
  "features": [
    { "type": "Feature", "properties": { "name": "strefa zerowania" },
      "geometry": { "type": "Polygon", "coordinates": [[
        [7200, 9000], [8600, 9000], [8600, 10200], [7200, 10200], [7200, 9000]
      ]] } },
    { "type": "Feature", "properties": { "name": "lotnisko" },
      "geometry": { "type": "Polygon", "coordinates": [[
        [2400, 2400], [4200, 2100], [4400, 3100], [2600, 3400], [2400, 2400]
      ]] } }
  ]
}"#;
    let gj_path = out_dir.join("sample_exclusions.geojson");
    std::fs::write(&gj_path, gj)?;
    println!("OK {}", gj_path.display());

    // --- projekt ------------------------------------------------------------------
    let mut project = ForestProject {
        version: 1,
        map_size_m: MAP_M,
        easting_offset: 200_000.0,
        northing_offset: 0.0,
        seed: 20_260_821,
        spacing_multiplier: 1.0,
        scale_min: 1.0,
        scale_max: 1.0,
        clearing_scale_m: 420.0,
        clearing_strength: 0.35,
        min_altitude: Some(35.0),
        max_altitude: None,
        max_slope_deg: Some(32.0),
        edge_padding_m: 24.0,
        color_tolerance: 12,
        exclusion_colors: vec![lake, road],
        cut_zones: vec![
            // demo: wycina drzewa 8 m od szarej drogi
            forest_core::preset::CutZone { color: road, margin_m: 8.0 },
        ],
        elevation_mode: ElevationMode::RelativeZero,
        species: vanilla_library(),
        zones: vec![
            ZoneDef {
                color: conifer,
                label: "Las iglasty".into(),
                density_per_ha: 150.0,
                // indeksy wg posortowanej vanilla_library(): świerk wys. (49),
                // świerk (26), sosna wys. (22), świerk mł. (48), leszczyna (152)
                species_weights: vec![(49, 5.0), (26, 3.0), (22, 2.0), (48, 2.0), (152, 1.0)],
                preset_mix: Vec::new(),
            },
            ZoneDef {
                color: mixed,
                label: "Las mieszany".into(),
                density_per_ha: 110.0,
                // brzoza (50), dąb (93), świerk (26), buk (64), bez (143)
                species_weights: vec![(50, 3.0), (93, 2.0), (26, 2.0), (64, 1.0), (143, 1.0)],
                preset_mix: Vec::new(),
            },
        ],
        areas: Vec::new(),
        use_mask_zones: true,
        use_areas: true,
        // demo pasa granicznego: krzewy wzdłuż krawędzi lasu, wtapiane i poszarpane
        edges: EdgeSettings {
            enabled: true,
            band_width_m: 18.0,
            density_per_ha: 140.0,
            species_weights: vec![(152, 3.0), (154, 3.0), (143, 2.0), (156, 1.0)],
            blend: true,
            jagged_m: 30.0,
            blend_inside_m: 45.0,
            preset_mix: Vec::new(),
        },
        paths: ProjectPaths {
            mask: Some(mask_path.to_string_lossy().to_string()),
            satellite: None,
            heightmap_asc: Some(asc_path.to_string_lossy().to_string()),
            exclusions_geojson: Some(gj_path.to_string_lossy().to_string()),
        },
        layer_library: Default::default(),
        species_layers: Default::default(),
        group_layers: Default::default(),
        png_settings: Default::default(),
        generation_order: vec![
            forest_core::preset::GenStep::Mask,
            forest_core::preset::GenStep::Cut,
        ],
    };
    project.sanitize();

    let proj_path = out_dir.join("sample_project.json");
    project.save(&proj_path)?;
    println!("OK {}", proj_path.display());
    Ok(())
}
