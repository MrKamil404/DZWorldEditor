//! JSON files of a mission (`cfggameplay.json`, `cfgeffectarea.json`,
//! `cfgundergroundtriggers.json`, ...): the values with their keys in the order they were
//! written, and the indentation of the file.
use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct JsonDoc {
    pub value: Value,
    pub indent: String,
    pub crlf: bool,
    pub bom: bool,
}

impl JsonDoc {
    pub fn new(value: Value) -> Self {
        Self { value, indent: "    ".into(), crlf: false, bom: false }
    }
    pub fn parse(b: &[u8]) -> anyhow::Result<Self> {
        let bom = b.starts_with(&[0xEF, 0xBB, 0xBF]);
        let s = std::str::from_utf8(if bom { &b[3..] } else { b })?;
        let value: Value = serde_json::from_str(s)?;
        let indent = s.lines().map(|l| l.chars().take_while(|c| *c == ' ' || *c == '\t').collect::<String>()).find(|l| !l.is_empty()).map(|l| if l.starts_with('\t') { "\t".to_string() } else { l }).unwrap_or_else(|| "    ".into());
        Ok(Self { value, indent, crlf: s.contains("\r\n"), bom })
    }
    pub fn to_string(&self) -> String {
        let mut buf = Vec::new();
        let fmt = serde_json::ser::PrettyFormatter::with_indent(self.indent.as_bytes());
        let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
        self.value.serialize(&mut ser).expect("JSON values always serialise");
        let s = String::from_utf8(buf).unwrap_or_default();
        if self.crlf {
            s.replace('\n', "\r\n")
        } else {
            s
        }
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        if self.bom {
            out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        }
        out.extend_from_slice(self.to_string().as_bytes());
        out
    }
    /// The value at a path of keys and indexes, as `"PlayerData/StaminaData/staminaMax"`.
    pub fn get(&self, path: &str) -> Option<&Value> {
        get_path(&self.value, path)
    }
    pub fn get_mut(&mut self, path: &str) -> Option<&mut Value> {
        let mut v = &mut self.value;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            v = match v {
                Value::Object(o) => o.get_mut(part)?,
                Value::Array(a) => a.get_mut(part.parse::<usize>().ok()?)?,
                _ => return None,
            };
        }
        Some(v)
    }
}

pub fn get_path<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let mut v = v;
    for part in path.split('/').filter(|p| !p.is_empty()) {
        v = match v {
            Value::Object(o) => o.get(part)?,
            Value::Array(a) => a.get(part.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(v)
}

/// A number of a JSON value, as `f64`.
pub fn num(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64)
}

/// A JSON number for `v`: whole numbers without a fraction unless `float` asks for one.
pub fn number(v: f64, float: bool) -> Value {
    if !float && v.fract() == 0.0 && v.abs() < 9e15 {
        Value::from(v as i64)
    } else {
        serde_json::Number::from_f64(v).map(Value::Number).unwrap_or(Value::Null)
    }
}

/// Sets a number, keeping the old one (and its form) when it means the same.
pub fn set_number(slot: &mut Value, v: f64) {
    if slot.as_f64() == Some(v) {
        return;
    }
    let float = slot.is_f64();
    *slot = number(v, float);
}

/// `[x, y, z]` of a JSON array.
pub fn vec3(v: Option<&Value>) -> Option<[f64; 3]> {
    let a = v?.as_array()?;
    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?])
}

pub fn set_vec3(slot: &mut Value, v: [f64; 3]) {
    if let Value::Array(a) = slot {
        if a.len() >= 3 {
            for (i, x) in v.into_iter().enumerate() {
                set_number(&mut a[i], x);
            }
            return;
        }
    }
    *slot = Value::Array(v.iter().map(|x| number(*x, x.fract() != 0.0)).collect());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_order_and_indent() {
        let src = "{\n\t\"version\": 123,\n\t\"b\": [1, 2.5],\n\t\"a\": {\"x\": 1.0}\n}";
        let d = JsonDoc::parse(src.as_bytes()).unwrap();
        assert_eq!(d.indent, "\t");
        let out = d.to_string();
        assert!(out.find("version").unwrap() < out.find("\"b\"").unwrap());
        assert!(out.find("\"b\"").unwrap() < out.find("\"a\"").unwrap());
        assert!(out.contains("1.0"));
        let back = JsonDoc::parse(out.as_bytes()).unwrap();
        assert_eq!(back.value, d.value);
    }
}
