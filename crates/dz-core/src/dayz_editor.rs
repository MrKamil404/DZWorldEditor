//! Import and export of DayZ Object Spawner JSON (`cfggameplay.json`) and DayZ Editor (`.dze`)
//! object files.
//!
//! Verified against `P:\scripts\3_Game\ObjectSpawner.c` and `P:\scripts\1_Core\proto\EnMath3D.c`:
//! - `ObjectSpawnerJson` has `{ "Objects": [ { "name", "pos": [x, y, z], "ypr": [yaw, pitch, roll],
//!   "scale": 1.0, "enableCEPersistency": false } ] }`.
//! - `ObjectSpawnerHandler::SpawnObject` accepts raw P3D paths (`CreateStaticObjectUsingP3D`)
//!   **only** under `DZ\plants*` and `DZ\rocks*`; all other objects are spawned via
//!   `CreateObjectEx` using their `CfgVehicles` class name.
//! - DayZ Editor `.dze` (JSON) has `{ "MapName": "...", "EditorObjects": [ { "Type", "Position",
//!   "Orientation", "Scale", "Flags" } ] }`.
//! - Both formats store world-space model origin `[x, y, z]` (with `y` relative to sea level)
//!   and `[yaw, pitch, roll]` in degrees where `yaw` matches editor yaw while `pitch` and `roll`
//!   have opposite sign to `crate::objects::rotation`.
use crate::assets::AssetIndex;
use crate::mission::classes::{guess_stem, ClassIndex};
use crate::mission::gen::proxy_class;
use crate::objects::{Obj, ObjectStore};
use crate::world::World;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Output format for DayZ object files.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DayzObjectFormat {
    /// Vanilla `cfggameplay.json` Object Spawner (`{ "Objects": [...] }`).
    #[default]
    ObjectSpawnerJson,
    /// DayZ Editor `.dze` JSON (`{ "MapName": "...", "EditorObjects": [...] }`).
    Dze,
}

impl DayzObjectFormat {
    pub fn extension(self) -> &'static str {
        match self {
            DayzObjectFormat::ObjectSpawnerJson => "json",
            DayzObjectFormat::Dze => "dze",
        }
    }
}

/// Vanilla `ObjectSpawner.c` `VALID_PATHS` that `CreateStaticObjectUsingP3D` allows.
fn valid_spawner_p3d_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase().replace('/', "\\");
    ["dz\\plants", "dz\\rocks"].iter().any(|p| lower.contains(p))
}

#[derive(Serialize, Deserialize, Debug)]
struct SpawnerFile {
    #[serde(rename = "Objects")]
    objects: Vec<SpawnerItem>,
}

#[derive(Serialize, Deserialize, Debug)]
struct SpawnerItem {
    name: String,
    pos: [f32; 3],
    ypr: [f32; 3],
    scale: f32,
    #[serde(rename = "enableCEPersistency")]
    enable_ce_persistency: bool,
}

#[derive(Serialize, Deserialize, Debug)]
struct DzeFile {
    #[serde(rename = "MapName")]
    map_name: String,
    #[serde(rename = "CameraPosition")]
    camera_position: [f32; 3],
    #[serde(rename = "EditorObjects")]
    editor_objects: Vec<DzeObject>,
    #[serde(rename = "EditorDeletedObjects")]
    editor_deleted_objects: Vec<Value>,
}

#[derive(Serialize, Deserialize, Debug)]
struct DzeObject {
    #[serde(rename = "Type")]
    kind: String,
    #[serde(rename = "DisplayName")]
    display_name: String,
    #[serde(rename = "Position")]
    position: [f32; 3],
    #[serde(rename = "Orientation")]
    orientation: [f32; 3],
    #[serde(rename = "Scale")]
    scale: f32,
    #[serde(rename = "Flags")]
    flags: i32,
}

/// Parses a 3-float vector from either a JSON array `[x, y, z]` or a space-separated string `"x y z"`.
fn parse_vec3(v: Option<&Value>) -> Option<[f32; 3]> {
    match v? {
        Value::Array(a) if a.len() >= 3 => {
            Some([a[0].as_f64()? as f32, a[1].as_f64()? as f32, a[2].as_f64()? as f32])
        }
        Value::String(s) => {
            let nums: Vec<f32> = s.split_whitespace().filter_map(|p| p.parse().ok()).collect();
            (nums.len() >= 3).then(|| [nums[0], nums[1], nums[2]])
        }
        _ => None,
    }
}

/// Resolves a class name or P3D path to a normalised model path in the project.
fn resolve_model(name: &str, assets: &AssetIndex, classes: Option<&ClassIndex>) -> Option<String> {
    let raw = name.trim();
    if raw.is_empty() {
        return None;
    }
    if raw.contains('\\') || raw.contains('/') || raw.to_ascii_lowercase().ends_with(".p3d") {
        return assets.resolve(raw).map(str::to_string).or_else(|| {
            let norm = ObjectStore::normalize_path(raw);
            (!norm.is_empty()).then_some(norm)
        });
    }
    if let Some(m) = classes.and_then(|c| c.model_of(raw)) {
        return Some(assets.resolve(&m).map(str::to_string).unwrap_or(m));
    }
    // Strip BuilderItems (`bldr_`) as well as vanilla `Land_`/`StaticObj_`/`_DE`.
    let stripped = raw.strip_prefix("bldr_").or_else(|| raw.strip_prefix("BLDR_")).unwrap_or(raw);
    let stem = guess_stem(stripped);
    assets.resolve(&stem).or_else(|| assets.resolve(raw)).map(str::to_string)
}

/// Chooses the best DayZ name (class name or P3D path) for exporting a model.
pub fn export_name(model_path: &str, format: DayzObjectFormat, classes: Option<&ClassIndex>) -> String {
    if let Some(cls) = classes.and_then(|c| c.classes_for_model(model_path, false).into_iter().next()) {
        return cls;
    }
    if format == DayzObjectFormat::ObjectSpawnerJson && valid_spawner_p3d_path(model_path) {
        let p = model_path.replace('/', "\\");
        // `ObjectSpawnerHandler::ValidatePath` checks uppercase `"DZ\\plants"` / `"DZ\\rocks"`.
        if let Some(rest) = p.strip_prefix("dz\\") {
            return format!("DZ\\{rest}");
        }
        return p;
    }
    proxy_class(model_path)
}

/// Converts DayZ `[x, y, z]` and `[yaw, pitch, roll]` into an editor `Obj`.
pub fn from_dayz_transform(pos: [f32; 3], ypr: [f32; 3], scale: f32, model: u32, w: &World) -> Obj {
    let x = pos[0];
    let z = pos[2];
    let world_y = pos[1] + w.meta.sea_level;
    let elevation = world_y - w.height.sample(x, z);
    let scale = if scale > 0.0 && scale.is_finite() { scale } else { 1.0 };
    Obj { x, z, elevation, yaw: ypr[0], pitch: -ypr[1], roll: -ypr[2], scale, model, flags: 0 }
}

/// Converts an editor `Obj` into DayZ `[x, y, z]` and `[yaw, pitch, roll]`.
pub fn to_dayz_transform(o: &Obj, w: &World) -> ([f32; 3], [f32; 3]) {
    let y = ObjectStore::origin_y(o, &w.height) - w.meta.sea_level;
    let clean = |v: f32| if v.abs() < 1e-6 { 0.0 } else { v };
    ([clean(o.x), clean(y), clean(o.z)], [clean(o.yaw), clean(-o.pitch), clean(-o.roll)])
}

/// Imports objects from either a DayZ Object Spawner JSON (`Objects`) or a DayZ Editor `.dze`
/// JSON (`EditorObjects`). Returns the parsed objects and a summary of unresolved names.
pub fn parse_dayz_objects(bytes: &[u8], w: &mut World, assets: &AssetIndex, classes: Option<&ClassIndex>) -> Result<(Vec<Obj>, Vec<String>)> {
    let text = std::str::from_utf8(bytes).context("Not a JSON text file (binary .dze files must be saved as JSON in DayZ Editor)")?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let root: Value = serde_json::from_str(text).context("Invalid JSON")?;
    let mut out = Vec::new();
    let mut missing: BTreeMap<String, usize> = BTreeMap::new();

    if let Some(arr) = root.get("Objects").and_then(Value::as_array) {
        for item in arr {
            let Some(name) = item.get("name").and_then(Value::as_str) else { continue };
            let Some(pos) = parse_vec3(item.get("pos")) else { continue };
            let ypr = parse_vec3(item.get("ypr")).unwrap_or([0.0; 3]);
            let scale = item.get("scale").and_then(Value::as_f64).map(|v| v as f32).unwrap_or(1.0);
            match resolve_model(name, assets, classes) {
                Some(path) => {
                    let model = w.objects.intern(&path);
                    out.push(from_dayz_transform(pos, ypr, scale, model, w));
                }
                None => *missing.entry(name.to_string()).or_default() += 1,
            }
        }
    } else if let Some(arr) = root.get("EditorObjects").and_then(Value::as_array) {
        for item in arr {
            let Some(name) = item.get("Type").and_then(Value::as_str) else { continue };
            let Some(pos) = parse_vec3(item.get("Position")) else { continue };
            let ypr = parse_vec3(item.get("Orientation")).unwrap_or([0.0; 3]);
            let scale = item.get("Scale").and_then(Value::as_f64).map(|v| v as f32).unwrap_or(1.0);
            match resolve_model(name, assets, classes) {
                Some(path) => {
                    let model = w.objects.intern(&path);
                    out.push(from_dayz_transform(pos, ypr, scale, model, w));
                }
                None => *missing.entry(name.to_string()).or_default() += 1,
            }
        }
    } else {
        bail!("File has neither an \"Objects\" (Object Spawner) nor an \"EditorObjects\" (DayZ Editor) list");
    }

    let missing_list = missing.into_iter().map(|(k, n)| format!("{k} ({n})")).collect();
    Ok((out, missing_list))
}

/// Serialises objects to DayZ Object Spawner JSON or DayZ Editor `.dze` JSON.
pub fn write_dayz_objects(
    w: &World,
    objects: &[&Obj],
    format: DayzObjectFormat,
    enable_ce_persistency: bool,
    classes: Option<&ClassIndex>,
) -> Result<String> {
    match format {
        DayzObjectFormat::ObjectSpawnerJson => {
            let items: Vec<SpawnerItem> = objects
                .iter()
                .map(|o| {
                    let path = w.objects.model_path(o);
                    let (pos, ypr) = to_dayz_transform(o, w);
                    SpawnerItem {
                        name: export_name(path, format, classes),
                        pos,
                        ypr,
                        scale: o.scale,
                        enable_ce_persistency,
                    }
                })
                .collect();
            Ok(serde_json::to_string_pretty(&SpawnerFile { objects: items })?)
        }
        DayzObjectFormat::Dze => {
            let mut cam = [w.meta.size_m() * 0.5, 50.0, w.meta.size_m() * 0.5];
            let items: Vec<DzeObject> = objects
                .iter()
                .map(|o| {
                    let path = w.objects.model_path(o);
                    let (position, orientation) = to_dayz_transform(o, w);
                    let kind = export_name(path, format, classes);
                    DzeObject {
                        display_name: kind.clone(),
                        kind,
                        position,
                        orientation,
                        scale: o.scale,
                        flags: if enable_ce_persistency { 30 } else { 0 },
                    }
                })
                .collect();
            if let Some(first) = items.first() {
                cam = [first.position[0], first.position[1] + 20.0, first.position[2]];
            }
            let file = DzeFile {
                map_name: w.meta.map_id.clone(),
                camera_position: cam,
                editor_objects: items,
                editor_deleted_objects: Vec::new(),
            };
            Ok(serde_json::to_string_pretty(&file)?)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::WorldMeta;

    #[test]
    fn round_trip_object_spawner_and_dze() {
        let mut w = World::new(WorldMeta::new("TestMap", 64, 8.0), 15.0);
        let mut classes = ClassIndex::default();
        classes.add_config(
            r#"
            class CfgVehicles {
                class HouseNoDestruct;
                class Land_Mil_GuardHouse : HouseNoDestruct {
                    scope = 2;
                    model = "\DZ\structures\military\houses\mil_guardhouse.p3d";
                };
            };
            "#,
        );
        classes.finish();
        let assets = AssetIndex::default();
        let m1 = w.objects.intern("dz\\structures\\military\\houses\\mil_guardhouse.p3d");
        let m2 = w.objects.intern("dz\\plants\\tree\\t_piceaAbies_2s.p3d");
        let o1 = Obj { x: 100.0, z: 200.0, elevation: 1.5, yaw: 45.0, pitch: 10.0, roll: -5.0, scale: 1.2, model: m1, flags: 0 };
        let o2 = Obj { x: 110.0, z: 210.0, elevation: 0.0, yaw: 180.0, pitch: 0.0, roll: 0.0, scale: 0.9, model: m2, flags: 0 };

        // Object Spawner JSON: building uses class name, plant uses P3D path.
        let json = write_dayz_objects(&w, &[&o1, &o2], DayzObjectFormat::ObjectSpawnerJson, true, Some(&classes)).unwrap();
        assert!(json.contains("\"Land_Mil_GuardHouse\""), "{json}");
        assert!(json.contains("DZ\\\\plants\\\\tree\\\\t_piceaabies_2s.p3d"), "{json}");
        assert!(json.contains("\"enableCEPersistency\": true"), "{json}");

        let (back, missing) = parse_dayz_objects(json.as_bytes(), &mut w, &assets, Some(&classes)).unwrap();
        assert!(missing.is_empty(), "{missing:?}");
        assert_eq!(back.len(), 2);
        assert!((back[0].x - 100.0).abs() < 1e-4 && (back[0].z - 200.0).abs() < 1e-4);
        assert!((back[0].elevation - 1.5).abs() < 1e-4);
        assert!((back[0].yaw - 45.0).abs() < 1e-4 && (back[0].pitch - 10.0).abs() < 1e-4 && (back[0].roll - (-5.0)).abs() < 1e-4);
        assert!((back[0].scale - 1.2).abs() < 1e-4);

        // DayZ Editor .dze round-trip.
        let dze = write_dayz_objects(&w, &[&o1], DayzObjectFormat::Dze, false, Some(&classes)).unwrap();
        assert!(dze.contains("\"EditorObjects\"") && dze.contains("\"MapName\": \"TestMap\""), "{dze}");
        let (back_dze, missing_dze) = parse_dayz_objects(dze.as_bytes(), &mut w, &assets, Some(&classes)).unwrap();
        assert!(missing_dze.is_empty(), "{missing_dze:?}");
        assert_eq!(back_dze.len(), 1);
        assert!((back_dze[0].elevation - 1.5).abs() < 1e-4);
        assert!((back_dze[0].pitch - 10.0).abs() < 1e-4 && (back_dze[0].roll - (-5.0)).abs() < 1e-4);
    }
}
