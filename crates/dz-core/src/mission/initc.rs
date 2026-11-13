//! Settings the editor finds in `init.c` and changes there, leaving the rest of the script as
//! written.

/// Start of the lines the editor adds, so it finds and removes them again.
pub const MARK: &str = "// DZWE:";

/// The month and day the server date is reset to (`int reset_month = 9, reset_day = 20;`).
pub fn reset_date(text: &str) -> Option<(i32, i32)> {
    Some((int_after(text, "reset_month")?, int_after(text, "reset_day")?))
}

fn int_after(text: &str, name: &str) -> Option<i32> {
    let at = text.find(&format!("{name} ="))?;
    let rest = &text[at + name.len()..];
    let rest = rest.trim_start().strip_prefix('=')?.trim_start();
    let n: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect();
    n.parse().ok()
}

fn set_int_after(text: &str, name: &str, v: i32) -> String {
    let Some(at) = text.find(&format!("{name} =")) else { return text.to_string() };
    let start = at + name.len();
    let eq = text[start..].find('=').map(|p| start + p + 1).unwrap_or(start);
    let num_start = eq + text[eq..].len() - text[eq..].trim_start().len();
    let num_end = num_start + text[num_start..].chars().take_while(|c| c.is_ascii_digit() || *c == '-').count();
    format!("{}{}{}", &text[..num_start], v, &text[num_end..])
}

pub fn set_reset_date(text: &str, month: i32, day: i32) -> String {
    set_int_after(&set_int_after(text, "reset_month", month), "reset_day", day)
}

/// The range of the random health of the starting items (`Math.RandomFloat( 0.45, 0.65 )`).
pub fn start_health(text: &str) -> Option<(f32, f32)> {
    let at = text.find("RandomFloat(")?;
    let rest = &text[at + "RandomFloat(".len()..];
    let end = rest.find(')')?;
    let mut it = rest[..end].split(',').map(|s| s.trim().trim_end_matches('f').parse::<f32>());
    Some((it.next()?.ok()?, it.next()?.ok()?))
}

pub fn set_start_health(text: &str, lo: f32, hi: f32) -> String {
    let Some(at) = text.find("RandomFloat(") else { return text.to_string() };
    let open = at + "RandomFloat(".len();
    let Some(end) = text[open..].find(')') else { return text.to_string() };
    format!("{} {lo:.2}, {hi:.2} {}", &text[..open], &text[open + end..])
}

/// Items the script puts on new characters (`CreateInInventory( "X" )` and the like).
pub fn start_items(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for key in ["CreateInInventory(", "CreateAttachment(", "CreateInCargo("] {
        let mut rest = text;
        while let Some(at) = rest.find(key) {
            rest = &rest[at + key.len()..];
            let arg = rest.trim_start();
            if let Some(s) = arg.strip_prefix('"') {
                if let Some(end) = s.find('"') {
                    out.push(s[..end].to_string());
                }
            }
        }
    }
    // Items of string arrays such as the chemlights.
    let mut rest = text;
    while let Some(at) = rest.find("[] = {") {
        rest = &rest[at + 6..];
        if let Some(end) = rest.find('}') {
            for part in rest[..end].split(',') {
                let p = part.trim().trim_matches('"');
                if !p.is_empty() && !out.iter().any(|o| o == p) {
                    out.push(p.to_string());
                }
            }
        }
    }
    out
}

/// What `GetCEApi()` can write out on the server, to make economy files for a new map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CeExport {
    /// `mapgroupproto.xml` of the buildings with loot points (`ExportProxyProto`).
    ProxyProto,
    /// `mapgrouppos.xml` of the buildings on the map (`ExportProxyData`).
    ProxyData,
    /// `mapgroupcluster.xml` of the fruit trees and stones (`ExportClusterData`).
    ClusterData,
    /// Player spawn points made by the generator (`ExportSpawnData`).
    SpawnData,
}

impl CeExport {
    pub const ALL: [CeExport; 4] = [CeExport::ProxyProto, CeExport::ProxyData, CeExport::ClusterData, CeExport::SpawnData];
    pub fn call(self, center: [f32; 2], radius: f32) -> String {
        match self {
            CeExport::ProxyProto => "GetCEApi().ExportProxyProto();".into(),
            CeExport::ProxyData => format!("GetCEApi().ExportProxyData(Vector({:.0}, 0, {:.0}), {:.0});", center[0], center[1], radius),
            CeExport::ClusterData => "GetCEApi().ExportClusterData();".into(),
            CeExport::SpawnData => "GetCEApi().ExportSpawnData();".into(),
        }
    }
    fn needle(self) -> &'static str {
        match self {
            CeExport::ProxyProto => "ExportProxyProto(",
            CeExport::ProxyData => "ExportProxyData(",
            CeExport::ClusterData => "ExportClusterData(",
            CeExport::SpawnData => "ExportSpawnData(",
        }
    }
}

/// Whether the script calls that export (outside comments).
pub fn has_export(text: &str, e: CeExport) -> bool {
    text.lines().any(|l| {
        let code = l.split("//").next().unwrap_or("");
        code.contains(e.needle())
    })
}

/// Adds the export after the economy starts, or removes it.
pub fn set_export(text: &str, e: CeExport, on: bool, center: [f32; 2], radius: f32) -> String {
    if has_export(text, e) == on {
        return text.to_string();
    }
    if !on {
        let lines: Vec<&str> = text.lines().collect();
        let calls = |l: &str| l.split("//").next().unwrap_or("").contains(e.needle());
        let mut out = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            if calls(l) || (l.trim_start().starts_with(MARK) && lines.get(i + 1).is_some_and(|n| calls(n))) {
                continue;
            }
            out.push(*l);
        }
        return out.join("\n") + if text.ends_with('\n') { "\n" } else { "" };
    }
    let line = format!("\t{MARK} remove after the files are written\n\t{}\n", e.call(center, radius));
    // After `ce.InitOffline();` where the script has it, else at the start of main().
    let anchor = text.find("InitOffline();").and_then(|p| text[p..].find('\n').map(|n| p + n + 1)).or_else(|| text.find("void main()").and_then(|p| text[p..].find('{').and_then(|b| text[p + b..].find('\n').map(|n| p + b + n + 1))));
    match anchor {
        Some(at) => format!("{}{}{}", &text[..at], line, &text[at..]),
        None => format!("{text}\n{line}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INIT: &str = "void main()\n{\n\tHive ce = CreateHive();\n\tif ( ce )\n\t\tce.InitOffline();\n\n\tint reset_month = 9, reset_day = 20;\n}\nclass M {\n\tfloat rndHlt = Math.RandomFloat( 0.45, 0.65 );\n\titemEnt = player.GetInventory().CreateInInventory( \"Apple\" );\n\tstring chemlightArray[] = { \"Chemlight_White\", \"Chemlight_Red\" };\n};\n";

    #[test]
    fn reads_and_changes_settings() {
        assert_eq!(reset_date(INIT), Some((9, 20)));
        let t = set_reset_date(INIT, 6, 1);
        assert_eq!(reset_date(&t), Some((6, 1)));
        assert_eq!(start_health(INIT), Some((0.45, 0.65)));
        let t = set_start_health(&t, 0.7, 1.0);
        assert_eq!(start_health(&t), Some((0.7, 1.0)));
        assert_eq!(start_items(INIT), vec!["Apple", "Chemlight_White", "Chemlight_Red"]);
        let t2 = set_export(&t, CeExport::ProxyData, true, [7680.0, 7680.0], 20000.0);
        assert!(has_export(&t2, CeExport::ProxyData));
        assert!(t2.find("ExportProxyData").unwrap() > t2.find("InitOffline").unwrap());
        let t3 = set_export(&t2, CeExport::ProxyData, false, [0.0; 2], 0.0);
        assert!(!has_export(&t3, CeExport::ProxyData));
        assert_eq!(reset_date(&t3), Some((6, 1)));
    }
}
