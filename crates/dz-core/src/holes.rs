//! Terrain holes: heightmap cells the game does not draw, kept in `holes.cfg`.
use std::collections::BTreeSet;

/// (column, row) of a heightmap cell, counted from the south-west corner.
pub type Holes = BTreeSet<[u32; 2]>;

/// Touching cells end up in one class each, which keeps the file readable.
fn groups(holes: &Holes) -> Vec<Vec<[u32; 2]>> {
    let mut left = holes.clone();
    let mut out = Vec::new();
    while let Some(first) = left.pop_first() {
        let mut group = vec![first];
        let mut k = 0;
        while k < group.len() {
            let [x, y] = group[k];
            for n in [[x + 1, y], [x.wrapping_sub(1), y], [x, y + 1], [x, y.wrapping_sub(1)]] {
                if left.remove(&n) {
                    group.push(n);
                }
            }
            k += 1;
        }
        group.sort_unstable();
        out.push(group);
    }
    out
}

/// The `class Holes` block, indented for where it goes.
pub fn class_text(holes: &Holes, indent: &str) -> String {
    let mut text = format!("{indent}class Holes\n{indent}{{\n");
    for (i, g) in groups(holes).iter().enumerate() {
        let tiles: Vec<String> = g.iter().map(|t| format!("{{{},{}}}", t[0], t[1])).collect();
        text += &format!("{indent}\tclass Hole_{}\n{indent}\t{{\n{indent}\t\ttiles[]={{{}}};\n{indent}\t}};\n", i + 1, tiles.join(","));
    }
    text + &format!("{indent}}};\n")
}

/// Reads every `{x,y}` pair of the `tiles[]` arrays.
pub fn parse(text: &str) -> Holes {
    let mut out = Holes::new();
    let mut rest = text;
    while let Some(at) = rest.find("tiles[]") {
        rest = &rest[at + 7..];
        let Some(open) = rest.find('{') else { break };
        // The array ends where its braces balance.
        let (mut depth, mut end) = (0, rest.len());
        for (i, c) in rest[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let numbers: Vec<u32> = rest[open..end].split(|c: char| !c.is_ascii_digit()).filter_map(|v| v.parse().ok()).collect();
        out.extend(numbers.chunks_exact(2).map(|p| [p[0], p[1]]));
        rest = &rest[end..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holes_survive_writing_and_reading() {
        let holes = Holes::from([[10, 20], [11, 20], [11, 21], [500, 7]]);
        let text = class_text(&holes, "");
        assert!(text.contains("class Hole_1") && text.contains("class Hole_2"));
        assert!(text.contains("tiles[]={{10,20},{11,20},{11,21}};"));
        assert_eq!(parse(&text), holes);
    }
}
