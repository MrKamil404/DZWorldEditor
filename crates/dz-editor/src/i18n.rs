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
            current: "pl".into(),
        })
    })
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
