//! Plik towarzyszący projektu (`<nazwa>.objects.json`): zapis i odczyt
//! wygenerowanych obiektów razem z projektem.
//!
//! Obiekty trzymane są W OSOBNYM pliku obok JSON-a projektu (nie w nim),
//! żeby projekt pozostał lekki i kompatybilny wstecz — a duże generacje
//! (dziesiątki tysięcy obiektów) nie spowalniały zapisu/odczytu ustawień.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::scatter::{GenStats, PlacedObject};

const SIDECAR_SUFFIX: &str = ".objects.json";
const SIDECAR_VERSION: u32 = 1;

/// Zawartość pliku towarzyszącego: obiekty + statystyki z generowania.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedObjects {
    pub version: u32,
    pub objects: Vec<PlacedObject>,
    pub stats: GenStats,
}

/// Ścieżka pliku towarzyszącego dla danego pliku projektu:
/// `projekt_lasu.json` -> `projekt_lasu.objects.json`.
pub fn sidecar_path(project_path: &Path) -> PathBuf {
    let stem = project_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "projekt".into());
    project_path.with_file_name(format!("{stem}{SIDECAR_SUFFIX}"))
}

/// Zapisz obiekty + statystyki do pliku towarzyszącego projektu.
pub fn save_sidecar(
    project_path: &Path,
    objects: &[PlacedObject],
    stats: &GenStats,
) -> Result<PathBuf, String> {
    let path = sidecar_path(project_path);
    let data = SavedObjects {
        version: SIDECAR_VERSION,
        objects: objects.to_vec(),
        stats: stats.clone(),
    };
    let json =
        serde_json::to_string(&data).map_err(|e| format!("Serializacja obiektów: {e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("Zapis obiektów: {e}"))?;
    Ok(path)
}

/// Wczytaj obiekty + statystyki z pliku towarzyszącego (None = brak pliku).
/// Niezgodna wersja lub uszkodzony plik zgłasza błąd tekstowy.
pub fn load_sidecar(project_path: &Path) -> Result<Option<SavedObjects>, String> {
    let path = sidecar_path(project_path);
    if !path.exists() {
        return Ok(None);
    }
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("Odczyt obiektów: {e}"))?;
    let data: SavedObjects =
        serde_json::from_str(&text).map_err(|e| format!("Parse obiektów: {e}"))?;
    if data.version != SIDECAR_VERSION {
        return Err(format!(
            "Niezgodna wersja pliku obiektów ({}) — wygeneruj las ponownie",
            data.version
        ));
    }
    Ok(Some(data))
}

/// Usuń plik towarzyszący, jeśli istnieje (np. zapis projektu bez obiektów).
pub fn remove_sidecar(project_path: &Path) {
    let path = sidecar_path(project_path);
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_path_appends_suffix() {
        let p = Path::new("C:/proj/projekt_lasu.json");
        assert_eq!(
            sidecar_path(p),
            PathBuf::from("C:/proj/projekt_lasu.objects.json")
        );
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = std::env::temp_dir().join("dzf_sidecar_test");
        let _ = std::fs::create_dir_all(&dir);
        let proj = dir.join("proj.json");
        let objects = vec![PlacedObject {
            model: "a_1f".into(),
            x: 1.0,
            y: 2.0,
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            scale: 1.0,
            elevation: 0.0,
            zone_index: 0,
            species_index: 0,
        }];
        let stats = GenStats {
            total: 1,
            ..Default::default()
        };
        let path = save_sidecar(&proj, &objects, &stats).unwrap();
        assert!(path.exists());
        let back = load_sidecar(&proj).unwrap().expect("sidecar");
        assert_eq!(back.objects.len(), 1);
        assert_eq!(back.objects[0].model, "a_1f");
        assert_eq!(back.stats.total, 1);
        remove_sidecar(&proj);
        assert!(load_sidecar(&proj).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
