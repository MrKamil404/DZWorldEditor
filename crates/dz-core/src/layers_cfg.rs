//! Reads and writes Terrain Builder's `layers.cfg`.
use crate::world::SurfaceLayer;
use anyhow::{ensure, Result};
use std::collections::HashMap;

fn strip_comments(text: &str) -> String {
    text.lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n")
}

/// Text inside the first `class <name> { ... }` block.
fn block<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let lower = text.to_ascii_lowercase();
    let key = format!("class {}", name.to_ascii_lowercase());
    let mut from = 0;
    loop {
        let at = lower[from..].find(&key)? + from;
        let after = at + key.len();
        let next = lower[after..].chars().next()?;
        if next.is_alphanumeric() || next == '_' {
            from = after;
            continue;
        }
        let open = lower[after..].find('{')? + after;
        let mut depth = 0;
        for (i, c) in text[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&text[open + 1..open + i]);
                    }
                }
                _ => {}
            }
        }
        return None;
    }
}

/// Direct child classes of a block as (name, body).
fn children(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("class ") {
        let after = &rest[at + 6..];
        let name: String = after.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        let Some(open) = after.find('{') else { break };
        let mut depth = 0;
        let mut end = None;
        for (i, c) in after[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(open + i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else { break };
        out.push((name, after[open + 1..end].to_string()));
        rest = &after[end..];
    }
    out
}

fn field(body: &str, key: &str) -> Option<String> {
    for stmt in body.split(';') {
        let Some((k, v)) = stmt.split_once('=') else { continue };
        if k.trim().eq_ignore_ascii_case(key) {
            return Some(v.trim().trim_matches('"').trim_start_matches('\\').to_string());
        }
    }
    None
}

pub fn parse(text: &str) -> Result<Vec<SurfaceLayer>> {
    let text = strip_comments(text);
    let layers = block(&text, "Layers").unwrap_or(&text);
    let mut colors: HashMap<String, [u8; 3]> = HashMap::new();
    if let Some(c) = block(&text, "Legend").and_then(|l| block(l, "Colors")) {
        for stmt in c.split(';') {
            let Some((k, v)) = stmt.split_once('=') else { continue };
            let name = k.trim().trim_end_matches("[]").trim().to_ascii_lowercase();
            let digits: Vec<u8> = v.split(|ch: char| !ch.is_ascii_digit()).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect();
            if digits.len() >= 3 {
                colors.insert(name, [digits[0], digits[1], digits[2]]);
            }
        }
    }
    let mut out = Vec::new();
    for (i, (name, body)) in children(layers).into_iter().enumerate() {
        if name.eq_ignore_ascii_case("Legend") {
            continue;
        }
        let (Some(texture), Some(material)) = (field(&body, "texture"), field(&body, "material")) else { continue };
        let color = colors.get(&name.to_ascii_lowercase()).copied().unwrap_or_else(|| {
            forest_core::species::hsv_to_rgb(i as f64 * 47.0 % 360.0, 0.55, 0.75)
        });
        out.push(SurfaceLayer { name, texture, material, color });
    }
    ensure!(!out.is_empty(), "layers.cfg has no usable layers");
    Ok(out)
}

pub fn write(layers: &[SurfaceLayer], legend_picture: &str) -> String {
    let mut s = String::from("class Layers\n{\n");
    for l in layers {
        s += &format!("\tclass {}\n\t{{\n\t\ttexture = \"{}\";\n\t\tmaterial = \"{}\";\n\t}};\n", l.name, l.texture, l.material);
    }
    s += &format!("}};\nclass Legend\n{{\n\tpicture=\"{legend_picture}\";\n\tclass Colors\n\t{{\n");
    for l in layers {
        s += &format!("\t\t{}[]={{{{{},{},{}}}}};\n", l.name, l.color[0], l.color[1], l.color[2]);
    }
    s += "\t};\n};\n";
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    const SAMPLE: &str = r#"class Layers
{
	class grass // comment
	{
		texture = "DZ\surfaces\data\terrain\cp_grass_ca.paa";
		material = "DZ\surfaces\data\terrain\cp_grass.rvmat";
	};
	class rock
	{
		texture = "DZ\surfaces\data\terrain\cp_rock_ca.paa";
		material = "DZ\surfaces\data\terrain\cp_rock.rvmat";
	};
};
class Legend
{
	picture="x.png";
	class Colors
	{
		grass[]={{0,255,0}};
		rock[]={{255,0,0}};
	};
};"#;
    #[test]
    fn parses_layers_and_colours() {
        let l = parse(SAMPLE).unwrap();
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].name, "grass");
        assert_eq!(l[1].color, [255, 0, 0]);
        assert!(l[1].material.ends_with("cp_rock.rvmat"));
    }
    #[test]
    fn written_file_parses_back() {
        let l = parse(SAMPLE).unwrap();
        assert_eq!(parse(&write(&l, "a.png")).unwrap(), l);
    }
}
