//! Pure web animation admission. No assets are opened or decoded here.
use serde::{
    de::{self, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer, Serialize,
};
use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fmt};
const MAX_BYTES: usize = 65536;
struct Strict(Value);
impl<'de> Deserialize<'de> for Strict {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Strict;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("strict JSON without duplicate keys")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Strict, E> {
                Ok(Strict(Value::Bool(v)))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Strict, E> {
                Number::from_f64(v)
                    .map(|n| Strict(Value::Number(n)))
                    .ok_or_else(|| E::custom("nonfinite JSON value"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Strict, E> {
                Ok(Strict(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Strict, A::Error> {
                let mut result = Vec::new();
                while let Some(Strict(v)) = a.next_element()? {
                    result.push(v);
                }
                Ok(Strict(Value::Array(result)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Strict, A::Error> {
                let mut result = Map::new();
                let mut seen = BTreeSet::new();
                while let Some(key) = a.next_key::<String>()? {
                    if !seen.insert(key.clone()) {
                        return Err(de::Error::custom("duplicate JSON key"));
                    }
                    let Strict(v) = a.next_value()?;
                    result.insert(key, v);
                }
                Ok(Strict(Value::Object(result)))
            }
        }
        d.deserialize_any(V)
    }
}
pub(super) fn parse(bytes: &[u8]) -> Result<Value, String> {
    if bytes.len() > MAX_BYTES {
        return Err("animation manifest exceeds 64 KiB".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    serde_json::from_str::<Strict>(text)
        .map(|s| s.0)
        .map_err(|e| e.to_string())
}
fn keys<'a>(
    v: &'a Value,
    required: &[&str],
    optional: &[&str],
) -> Result<&'a Map<String, Value>, String> {
    let m = v.as_object().ok_or("invalid animation object fields")?;
    if required.iter().any(|k| !m.contains_key(*k))
        || m.keys()
            .any(|k| !required.contains(&k.as_str()) && !optional.contains(&k.as_str()))
    {
        return Err("invalid animation object fields".into());
    }
    Ok(m)
}
fn number(v: &Value, low: f64, high: f64, positive: bool) -> bool {
    v.as_f64()
        .is_some_and(|n| n.is_finite() && n >= low && n <= high && (!positive || n > 0.0))
}
fn pair(v: &Value, low: f64, high: f64, positive: bool) -> bool {
    v.as_array()
        .is_some_and(|a| a.len() == 2 && a.iter().all(|n| number(n, low, high, positive)))
}
pub(super) fn names(value: &Value) -> Result<Vec<String>, String> {
    let roles = value
        .as_object()
        .filter(|m| (1..=11).contains(&m.len()))
        .ok_or("animation manifest must contain 1..11 roles")?;
    let role_pattern = regex::Regex::new(r"\A(?:unit:[0-5]|enemy:[0-4])\z").unwrap();
    let path_part = regex::Regex::new(r"\A[A-Za-z0-9_-][A-Za-z0-9_.-]*\z").unwrap();
    let mut images = BTreeSet::new();
    for (role, definition) in roles {
        if !role_pattern.is_match(role) {
            return Err("invalid animation role".into());
        }
        let d = keys(
            definition,
            &["image", "clips"],
            &["anchor", "pivot", "logicalSize"],
        )?;
        let name = d["image"].as_str().ok_or("invalid animation image path")?;
        if name.len() > 128
            || !name.ends_with(".png")
            || name.split('/').any(|p| !path_part.is_match(p))
        {
            return Err("animation image must be a bounded relative PNG path".into());
        }
        images.insert(name.to_owned());
        for (field, low, high, positive) in [
            ("anchor", -16.0, 16.0, false),
            ("pivot", -4096.0, 4096.0, false),
            ("logicalSize", 0.0, 4096.0, true),
        ] {
            if d.get(field).is_some_and(|v| !pair(v, low, high, positive)) {
                return Err(format!("invalid animation {field}"));
            }
        }
        let clips = d["clips"]
            .as_object()
            .filter(|m| (1..=5).contains(&m.len()))
            .ok_or("invalid animation clips")?;
        for (clip_name, clip) in clips {
            if !["idle", "move", "attack", "hit", "death"].contains(&clip_name.as_str()) {
                return Err("invalid animation clip name".into());
            }
            let mut optional = vec!["fps", "loop", "pivots"];
            if clip_name == "attack" {
                optional.extend(["windupMs", "releaseMs", "recoveryMs"]);
            }
            let c = keys(clip, &["frames"], &optional)?;
            let frames = c["frames"]
                .as_array()
                .filter(|a| (1..=256).contains(&a.len()))
                .ok_or("animation clip requires 1..256 frames")?;
            for rect in frames {
                let r = rect
                    .as_array()
                    .filter(|a| a.len() == 4)
                    .ok_or("invalid animation frame rectangle")?;
                if !r.iter().all(|v| number(v, 0.0, 4096.0, false))
                    || !number(&r[2], 0.0, 4096.0, true)
                    || !number(&r[3], 0.0, 4096.0, true)
                {
                    return Err("invalid animation frame rectangle".into());
                }
            }
            if c.get("fps").is_some_and(|v| !number(v, 1.0, 120.0, false)) {
                return Err("invalid animation fps".into());
            }
            if c.get("loop").is_some_and(|v| !v.is_boolean()) {
                return Err("invalid animation loop".into());
            }
            if c.get("pivots").is_some_and(|v| {
                !v.as_array().is_some_and(|a| {
                    a.len() == frames.len() && a.iter().all(|p| pair(p, -4096.0, 4096.0, false))
                })
            }) {
                return Err("invalid animation pivots".into());
            }
            if clip_name == "attack"
                && ["windupMs", "releaseMs", "recoveryMs"].iter().any(|field| {
                    c.get(*field)
                        .is_some_and(|v| !number(v, 1.0, 2000.0, false))
                })
            {
                return Err("invalid animation attack timing".into());
            }
        }
    }
    Ok(images.into_iter().collect())
}
pub(super) fn encode(value: &Value, pretty: bool) -> Result<Vec<u8>, String> {
    fn quoted(s: &str) -> String {
        let raw = serde_json::to_string(s).unwrap();
        let mut out = String::new();
        for c in raw.chars() {
            if c >= '\u{7f}' {
                for u in c.encode_utf16(&mut [0; 2]) {
                    out.push_str(&format!("\\u{u:04x}"));
                }
            } else {
                out.push(c);
            }
        }
        out
    }
    fn emit(v: &Value, p: bool, depth: usize, out: &mut String) {
        let (open, close, items) = match v {
            Value::Object(m) => {
                let mut keys: Vec<_> = m.keys().collect();
                keys.sort();
                (
                    '{',
                    '}',
                    keys.into_iter()
                        .map(|k| (Some(k.as_str()), &m[k]))
                        .collect::<Vec<_>>(),
                )
            }
            Value::Array(a) => ('[', ']', a.iter().map(|v| (None, v)).collect()),
            Value::String(s) => {
                out.push_str(&quoted(s));
                return;
            }
            _ => {
                out.push_str(&v.to_string());
                return;
            }
        };
        out.push(open);
        for (i, (key, item)) in items.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            if p {
                out.push('\n');
                out.push_str(&"  ".repeat(depth + 1));
            }
            if let Some(key) = key {
                out.push_str(&quoted(key));
                out.push(':');
                if p {
                    out.push(' ');
                }
            }
            emit(item, p, depth + 1, out);
        }
        if p && !items.is_empty() {
            out.push('\n');
            out.push_str(&"  ".repeat(depth));
        }
        out.push(close);
    }
    let mut out = String::new();
    emit(value, pretty, 0, &mut out);
    out.push('\n');
    if out.len() > MAX_BYTES {
        return Err("animation manifest exceeds 64 KiB".into());
    }
    Ok(out.into_bytes())
}
fn finite(v: f64) -> Result<Value, String> {
    Number::from_f64(v)
        .map(Value::Number)
        .ok_or("nonfinite authored number".into())
}
pub(super) fn create(
    role: &str,
    image: &str,
    frames: &[(String, [f64; 4])],
    clips: &[(String, f64, bool)],
    pivot: Option<[f64; 2]>,
) -> Result<Vec<u8>, String> {
    if frames.len() > 5 * 256 || clips.len() > 5 {
        return Err("animation input count exceeds bounds".into());
    }
    let mut definitions = Map::new();
    for (name, rect) in frames {
        let clip = definitions
            .entry(name.clone())
            .or_insert_with(|| serde_json::json!({"frames":[]}));
        let rect = rect
            .iter()
            .map(|v| finite(*v))
            .collect::<Result<Vec<_>, _>>()?;
        clip["frames"]
            .as_array_mut()
            .unwrap()
            .push(Value::Array(rect));
    }
    let mut configured = BTreeSet::new();
    for (name, fps, looping) in clips {
        if !configured.insert(name) || !definitions.contains_key(name) {
            return Err("clip settings require a unique clip with authored frames".into());
        }
        let c = definitions.get_mut(name).unwrap().as_object_mut().unwrap();
        c.insert("fps".into(), finite(*fps)?);
        c.insert("loop".into(), Value::Bool(*looping));
    }
    let mut d = Map::new();
    d.insert("image".into(), image.into());
    d.insert("clips".into(), Value::Object(definitions));
    if let Some(p) = pivot {
        d.insert(
            "pivot".into(),
            Value::Array(p.iter().map(|v| finite(*v)).collect::<Result<_, _>>()?),
        );
    }
    let v = Value::Object(Map::from_iter([(role.into(), Value::Object(d))]));
    names(&v)?;
    encode(&v, true)
}
#[derive(Debug, Serialize)]
pub(super) struct SourceReceipt {
    pub bytes: usize,
    pub sha256: String,
}
pub(super) fn compose(inputs: &[Vec<u8>]) -> Result<(Vec<u8>, Vec<SourceReceipt>), String> {
    if !(1..=11).contains(&inputs.len()) {
        return Err("composition requires 1..11 input manifests".into());
    }
    let mut combined = Map::new();
    let mut receipts = Vec::new();
    for bytes in inputs {
        let v = parse(bytes)?;
        names(&v)?;
        for (role, definition) in v.as_object().unwrap() {
            if combined.insert(role.clone(), definition.clone()).is_some() {
                return Err("duplicate animation role across manifests".into());
            }
        }
        receipts.push(SourceReceipt {
            bytes: bytes.len(),
            sha256: Sha256::digest(bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
        });
    }
    let v = Value::Object(combined);
    names(&v)?;
    Ok((encode(&v, false)?, receipts))
}
#[cfg(test)]
#[path = "manifest_tests.rs"]
mod tests;
