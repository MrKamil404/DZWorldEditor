//! Presety użytkownika — edytowalne kopie szablonów wbudowanych oraz własne
//! strefy lasów, zapisywane do pliku JSON obok programu.
//!
//! Wagi gatunków przechowywane są po NAZWIE MODELU (nie po indeksie), więc
//! presety są przenośne między projektami o różnej liście gatunków.

use serde::{Deserialize, Serialize};

use crate::mask::Rgb8;
use crate::preset::ZoneDef;
use crate::species::{vanilla_library, SpeciesDef, ZonePreset};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserPreset {
    pub group: String,
    pub name: String,
    pub density_per_ha: f32,
    /// (nazwa modelu TB, waga > 0)
    pub weights: Vec<(String, f32)>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UserPresetLibrary {
    #[serde(default)]
    pub presets: Vec<UserPreset>,
}

/// Domyślna ścieżka pliku z presetami użytkownika (katalog roboczy).
pub const DEFAULT_PRESETS_FILE: &str = "presets_user.json";

impl UserPresetLibrary {
    /// Wczytuje plik; brak pliku = pusta biblioteka (bez błędu).
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self, String> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("Odczyt presetów: {e}"))?;
        serde_json::from_str(&text).map_err(|e| format!("Parse presetów: {e}"))
    }

    pub fn save(&self, path: impl AsRef<std::path::Path>) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Serializacja presetów: {e}"))?;
        std::fs::write(path, json).map_err(|e| format!("Zapis presetów: {e}"))
    }
}

impl UserPreset {
    /// Kopiuje preset wbudowany do biblioteki użytkownika (modele wg nazw).
    pub fn from_builtin(p: &ZonePreset) -> Self {
        let lib = vanilla_library();
        let weights = p
            .weights
            .iter()
            .filter_map(|(i, w)| lib.get(*i).map(|s| (s.model.clone(), *w)))
            .collect();
        Self {
            group: p.group.to_string(),
            name: p.name.to_string(),
            density_per_ha: p.density_per_ha,
            weights,
        }
    }

    /// Mapuje nazwy modeli na indeksy gatunków projektu. Nieznane modele są
    /// pomijane; zwraca None, gdy żaden nie pasuje.
    pub fn to_zone_def_in(
        &self,
        color: Rgb8,
        species: &[SpeciesDef],
    ) -> Option<ZoneDef> {
        let mut zw = Vec::new();
        for (model, w) in &self.weights {
            if let Some(idx) = species
                .iter()
                .position(|s| s.model.eq_ignore_ascii_case(model))
            {
                zw.push((idx, *w));
            }
        }
        if zw.is_empty() {
            return None;
        }
        Some(ZoneDef {
            color,
            label: self.name.clone(),
            density_per_ha: self.density_per_ha,
            species_weights: zw,
            preset_mix: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_builtin_resolves_all_models() {
        let lib = vanilla_library();
        for p in zone_presets_all() {
            let up = UserPreset::from_builtin(&p);
            assert_eq!(up.weights.len(), p.weights.len(), "{}", p.name);
            for (model, _) in &up.weights {
                assert!(lib.iter().any(|s| &s.model == model), "{model}");
            }
        }
    }

    fn zone_presets_all() -> Vec<ZonePreset> {
        crate::species::zone_presets()
    }

    #[test]
    fn to_zone_def_maps_models_and_skips_unknown() {
        let species = vec![
            SpeciesDef::new("A", "a_1f", 1.0, 1.0),
            SpeciesDef::new("B", "b_2f", 1.0, 1.0),
        ];
        let up = UserPreset {
            group: "G".into(),
            name: "Test".into(),
            density_per_ha: 100.0,
            weights: vec![
                ("a_1f".into(), 3.0),
                ("nie_ma_takiego".into(), 5.0),
                ("b_2f".into(), 1.0),
            ],
        };
        let z = up.to_zone_def_in(Rgb8([9, 9, 9]), &species).unwrap();
        assert_eq!(z.color, Rgb8([9, 9, 9]));
        assert_eq!(z.label, "Test");
        assert_eq!(z.species_weights, vec![(0, 3.0), (1, 1.0)]);

        let up_bad = UserPreset {
            weights: vec![("brak".into(), 1.0)],
            ..up.clone()
        };
        assert!(up_bad.to_zone_def_in(Rgb8([0, 0, 0]), &species).is_none());
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presets.json");
        let lib = UserPresetLibrary {
            presets: vec![UserPreset {
                group: "Moje".into(),
                name: "Zagajnik".into(),
                density_per_ha: 210.0,
                weights: vec![("t_BetulaPendula_2f".into(), 2.0)],
            }],
        };
        lib.save(&path).unwrap();
        let back = UserPresetLibrary::load(&path).unwrap();
        assert_eq!(back.presets.len(), 1);
        assert_eq!(back.presets[0].name, "Zagajnik");
        assert_eq!(back.presets[0].density_per_ha, 210.0);
    }

    #[test]
    fn load_missing_file_is_empty_ok() {
        let dir = tempfile::tempdir().unwrap();
        let lib = UserPresetLibrary::load(dir.path().join("nic.json")).unwrap();
        assert!(lib.presets.is_empty());
    }
}
