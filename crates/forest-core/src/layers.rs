//! Parser pliku `layers.cfg` z projektu DayZ (Terrain Builder).
//!
//! Format:
//! ```text
//! class Legend {
//!     class Colors {
//!         layer_name[]={{R,G,B}};
//!         ...
//!     };
//! };
//! ```
//!
//! Wyciąga mapę nazwa warstwy → kolor RGB.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::mask::Rgb8;

/// Pojedyncza warstwa z `layers.cfg`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LayerDef {
    pub name: String,
    pub color: Rgb8,
}

/// Zaimportowane warstwy z `layers.cfg`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LayerLibrary {
    pub layers: Vec<LayerDef>,
}

impl LayerLibrary {
    /// Parsuj plik `layers.cfg` i wyciągnij kolory z sekcji `Legend > Colors`.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let text = std::fs::read_to_string(path.as_ref())
            .map_err(|e| format!("Odczyt layers.cfg: {e}"))?;
        Self::parse(&text)
    }

    /// Parsuj zawartość pliku `layers.cfg` (tekst).
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut layers = Vec::new();

        // znajdź sekcję class Colors { ... } wewnątrz class Legend
        let colors_start = text
            .find("class Colors")
            .ok_or("Brak sekcji 'class Colors' w layers.cfg")?;
        let brace_start = text[colors_start..]
            .find('{')
            .map(|i| colors_start + i + 1)
            .ok_or("Brak '{' po 'class Colors'")?;

        // znajdź pasujący '}'
        let mut depth = 1;
        let mut pos = brace_start;
        let bytes = text.as_bytes();
        while pos < bytes.len() && depth > 0 {
            match bytes[pos] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            pos += 1;
        }
        let colors_end = pos - 1;
        let colors_body = &text[brace_start..colors_end];

        // parsuj linie typu: layer_name[]={{R,G,B}};
        for line in colors_body.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("//") {
                continue;
            }
            // oczekiwany format: name[]={{R,G,B}};
            if let Some((name, color)) = parse_color_line(line) {
                layers.push(LayerDef { name, color });
            }
        }

        if layers.is_empty() {
            return Err("Nie znaleziono żadnych warstw w sekcji Colors".into());
        }

        Ok(Self { layers })
    }

    /// Mapa nazwa → kolor.
    pub fn color_map(&self) -> HashMap<&str, Rgb8> {
        self.layers.iter().map(|l| (l.name.as_str(), l.color)).collect()
    }

    /// Zapisz do pliku w formacie layers.cfg (tylko sekcja Colors).
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let mut text = String::new();
        text.push_str("class Legend\n{\n");
        text.push_str("  class Colors\n  {\n");
        for layer in &self.layers {
            let [r, g, b] = layer.color.0;
            text.push_str(&format!(
                "    {}[]={{{{ {},{},{} }}}};\n",
                layer.name, r, g, b
            ));
        }
        text.push_str("  };\n");
        text.push_str("};\n");
        std::fs::write(path, text).map_err(|e| format!("Zapis layers.cfg: {e}"))
    }
}

/// Parsuj jedną linię: `name[]={{R,G,B}};` → (name, Rgb8)
fn parse_color_line(line: &str) -> Option<(String, Rgb8)> {
    // usuń średnik na końcu
    let line = line.strip_suffix(';').unwrap_or(line).trim();

    // znajdź name[]={{...}}
    let eq_pos = line.find("[]={{")?;
    let name = line[..eq_pos].trim().to_string();
    if name.is_empty() {
        return None;
    }

    // wyciągnij R,G,B z {{R,G,B}}
    let rest = &line[eq_pos + 5..]; // po "[]={{"
    let end = rest.find("}}")?;
    let rgb_str = &rest[..end];

    let parts: Vec<&str> = rgb_str.split(',').collect();
    if parts.len() != 3 {
        return None;
    }

    let r: u8 = parts[0].trim().parse().ok()?;
    let g: u8 = parts[1].trim().parse().ok()?;
    let b: u8 = parts[2].trim().parse().ok()?;

    Some((name, Rgb8([r, g, b])))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
class Layers
{
    class broadleaf_dense1
    {
        texture = "DZ\surfaces\data\terrain\cp_broadleaf_dense1_ca.paa";
        material = "DZ\surfaces\data\terrain\cp_broadleaf_dense1.rvmat";
    };
};
class Legend
{
    picture="BalticrusSource\mapLegend.png";
    class Colors
    {
        broadleaf_dense1[]={{0,64,100}};
        broadleaf_dense2[]={{0,32,50}};
        conifer_common1[]={{0,0,0}};
        grass[]={{0,255,0}};
    };
};
"#;

    #[test]
    fn parses_legend_colors() {
        let lib = LayerLibrary::parse(SAMPLE).unwrap();
        assert_eq!(lib.layers.len(), 4);
        assert_eq!(lib.layers[0].name, "broadleaf_dense1");
        assert_eq!(lib.layers[0].color.0, [0, 64, 100]);
        assert_eq!(lib.layers[2].name, "conifer_common1");
        assert_eq!(lib.layers[2].color.0, [0, 0, 0]);
    }

    #[test]
    fn color_map_works() {
        let lib = LayerLibrary::parse(SAMPLE).unwrap();
        let map = lib.color_map();
        assert_eq!(map.get("grass"), Some(&Rgb8([0, 255, 0])));
        assert_eq!(map.get("nonexistent"), None);
    }

    #[test]
    fn save_and_reload_roundtrip() {
        let lib = LayerLibrary::parse(SAMPLE).unwrap();
        let tmp = std::env::temp_dir().join("test_layers.cfg");
        lib.save(&tmp).unwrap();
        let loaded = LayerLibrary::load(&tmp).unwrap();
        assert_eq!(loaded.layers.len(), lib.layers.len());
        for (a, b) in loaded.layers.iter().zip(lib.layers.iter()) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.color, b.color);
        }
        let _ = std::fs::remove_file(&tmp);
    }
}
