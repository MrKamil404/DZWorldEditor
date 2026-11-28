//! Interface translations.
//!
//! Text is written in English in the code. `assets/lang/catalog.json` holds the built-in
//! languages; any `<code>.json` file in the user's `lang` folder adds or overrides one, so a
//! new language needs no rebuild. A missing entry falls back to English.
use std::collections::HashMap;
use std::path::Path;
use std::sync::{OnceLock, RwLock};

const CATALOG: &str = include_str!("../../../assets/lang/catalog.json");

#[derive(Default)]
struct State {
    /// code -> (english -> translated)
    tables: HashMap<String, HashMap<String, String>>,
    names: Vec<(String, String)>,
    current: String,
}

fn state() -> &'static RwLock<State> {
    static S: OnceLock<RwLock<State>> = OnceLock::new();
    S.get_or_init(|| {
        let raw: HashMap<String, HashMap<String, String>> = serde_json::from_str(CATALOG).expect("catalog.json is valid");
        let mut tables: HashMap<String, HashMap<String, String>> = HashMap::new();
        for (en, per) in raw {
            for (code, text) in per {
                tables.entry(code).or_default().insert(en.clone(), text);
            }
        }
        RwLock::new(State {
            tables,
            names: vec![("pl".into(), "Polski".into()), ("en".into(), "English".into()), ("de".into(), "Deutsch".into())],
            current: "en".into(),
        })
    })
}

/// The language code of the system, such as `pl` for `pl-PL`; `None` when it cannot be read.
#[cfg(windows)]
fn system_code() -> Option<String> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetUserDefaultLocaleName(name: *mut u16, len: i32) -> i32;
    }
    let mut buf = [0u16; 85];
    let n = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
    if n <= 1 {
        return None;
    }
    let name = String::from_utf16_lossy(&buf[..n as usize - 1]);
    name.split(['-', '_']).next().map(|c| c.to_lowercase())
}

#[cfg(not(windows))]
fn system_code() -> Option<String> {
    None
}

/// The language to start with: that of the system when the editor has it, else English.
/// Call it after the user languages are loaded.
pub fn detect() -> String {
    pick(system_code().as_deref(), &languages())
}

fn pick(system: Option<&str>, available: &[(String, String)]) -> String {
    system.filter(|c| available.iter().any(|(a, _)| a == c)).unwrap_or("en").to_string()
}

/// Loads user languages: `lang/fr.json` = `{"__name__": "Français", "File": "Fichier"}`.
pub fn load_user_languages(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut s = state().write().unwrap();
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let Some(code) = p.file_stem().map(|c| c.to_string_lossy().to_string()) else { continue };
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        let Ok(mut table) = serde_json::from_str::<HashMap<String, String>>(&text) else { continue };
        let name = table.remove("__name__").unwrap_or_else(|| code.clone());
        s.tables.entry(code.clone()).or_default().extend(table);
        if !s.names.iter().any(|(c, _)| *c == code) {
            s.names.push((code, name));
        }
    }
}

pub fn languages() -> Vec<(String, String)> {
    state().read().unwrap().names.clone()
}
/// A message of the road engine (`road-core`, which writes them in Polish) in the language of
/// the interface; other text stays as it is.
pub fn road(text: &str) -> String {
    use dayz_road_tool::i18n::Language;
    let lang = match current().as_str() {
        "pl" => Language::Polish,
        "de" => Language::German,
        _ => Language::English,
    };
    lang.tr(text)
}

pub fn current() -> String {
    state().read().unwrap().current.clone()
}
pub fn set_language(code: &str) {
    let mut s = state().write().unwrap();
    if s.names.iter().any(|(c, _)| c == code) {
        s.current = code.to_string();
    }
}

/// Translated text for the current language.
pub fn t(text: &str) -> String {
    let s = state().read().unwrap();
    s.tables.get(&s.current).and_then(|m| m.get(text)).cloned().unwrap_or_else(|| text.to_string())
}

/// Translation with `{0}`, `{1}`... placeholders.
pub fn tf(text: &str, args: &[&str]) -> String {
    let mut out = t(text);
    for (i, a) in args.iter().enumerate() {
        out = out.replace(&format!("{{{i}}}"), a);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn the_system_language_is_used_only_when_it_is_available() {
        let have = vec![("pl".to_string(), "Polski".to_string()), ("en".to_string(), "English".to_string())];
        assert_eq!(pick(Some("pl"), &have), "pl");
        assert_eq!(pick(Some("fr"), &have), "en");
        assert_eq!(pick(None, &have), "en");
    }

    fn collect(dir: &Path, out: &mut BTreeSet<String>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                collect(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let text = std::fs::read_to_string(&p).unwrap();
                let bytes = text.as_bytes();
                for start in [ "t(\"", "tf(\"" ] {
                    let mut at = 0;
                    while let Some(found) = text[at..].find(start) {
                        let pos = at + found;
                        let prev = if pos == 0 { b' ' } else { bytes[pos - 1] };
                        at = pos + start.len();
                        if prev.is_ascii_alphanumeric() || prev == b'_' {
                            continue;
                        }
                        if let Some(end) = text[at..].find('"') {
                            out.insert(text[at..at + end].to_string());
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn every_interface_string_is_translated() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut keys = BTreeSet::new();
        collect(&src, &mut keys);
        // Strings that exist only inside this test's own needles.
        keys.remove("");
        let raw: HashMap<String, HashMap<String, String>> = serde_json::from_str(CATALOG).unwrap();
        let missing: Vec<_> = keys.iter().filter(|k| !raw.get(*k).is_some_and(|m| m.contains_key("pl") && m.contains_key("de"))).collect();
        assert!(missing.is_empty(), "{} untranslated: {missing:#?}", missing.len());
    }

    /// Names the interface shows from lists of the core, which the search for text in `t(`
    /// calls above cannot find.
    #[test]
    fn names_from_the_core_are_translated() {
        let raw: HashMap<String, HashMap<String, String>> = serde_json::from_str(CATALOG).unwrap();
        let built_in_objs = dz_core::library::built_in_objects();
        let built_in_pfs = dz_core::library::built_in_prefabs();
        let names = dz_core::real_terrain::SHAPE_CLASSES.iter().map(|c| c.name)
            .chain(dz_core::satgen::NATURAL_NAMES)
            .chain(dz_core::satgen::CLASS_NAMES)
            .chain(dz_core::library::BUILT_IN_OBJECT_CATEGORIES.iter().copied())
            .chain(built_in_objs.iter().map(|p| p.name.as_str()))
            .chain(built_in_pfs.iter().map(|p| p.name.as_str()))
            .chain(dz_core::gen::SURFACE_BIOMES.iter().flat_map(|b| [b.name, b.summary].into_iter().chain(b.rules.iter().map(|r| r.role))));
        let missing: Vec<&str> = names.filter(|n| !raw.get(*n).is_some_and(|m| m.contains_key("pl") && m.contains_key("de"))).collect();
        assert!(missing.is_empty(), "untranslated: {missing:?}");
    }

    #[test]
    fn placeholders_survive_translation() {
        let raw: HashMap<String, HashMap<String, String>> = serde_json::from_str(CATALOG).unwrap();
        for (en, per) in &raw {
            for (code, text) in per {
                for i in 0..4 {
                    let p = format!("{{{i}}}");
                    assert_eq!(en.contains(&p), text.contains(&p), "{code}: {en} -> {text}");
                }
            }
        }
    }
}
