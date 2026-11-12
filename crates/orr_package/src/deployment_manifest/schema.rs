//! Pure deployment v1 admission and launch-plan data. Never launches a program.
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
pub(super) const FORMAT: &str = "orr.deployment/1";
const MAX_MANIFEST: usize = 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct BinaryRecord {
    pub label: String,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Provenance {
    // deserialize_with deliberately prevents serde's implicit missing Option default.
    #[serde(deserialize_with = "nullable_revision")]
    pub source_revision: Option<String>,
    pub binaries: Vec<BinaryRecord>,
}
fn nullable_revision<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(d)
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    pub format: String,
    pub game: String,
    pub game_code_identity: String,
    pub game_code_id: String,
    pub frame_format_version: u32,
    pub build_id: String,
    pub provenance: Provenance,
}
pub(super) fn trim_python(value: &str) -> &str {
    value.trim_matches(|c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}'))
}
fn text(value: &str, field: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 4096 || trim_python(value) != value {
        return Err(format!(
            "{field} must be a nonempty trimmed string of at most 4096 UTF-8 bytes"
        ));
    }
    Ok(())
}
fn number(value: &str, hex: bool) -> Result<u64, String> {
    let n = if hex && (value.starts_with("0x") || value.starts_with("0X")) {
        let digits = &value[2..];
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("invalid hexadecimal u64 string".into());
        }
        u64::from_str_radix(digits, 16)
    } else {
        if value.is_empty()
            || !value.bytes().all(|b| b.is_ascii_digit())
            || (value.len() > 1 && value.starts_with('0'))
        {
            return Err("expected canonical decimal u64 string".into());
        }
        value.parse::<u64>()
    }
    .map_err(|_| "u64 string outside range")?;
    if n == 0 {
        return Err("identity must be nonzero".into());
    }
    Ok(n)
}
fn core_version() -> Result<(), String> {
    if orr_ecs::FRAME_FORMAT_VERSION != 2 {
        return Err("deployment v1 requires core frame format 2".into());
    }
    Ok(())
}
fn validate(m: &Manifest, game: Option<&str>, identity: Option<&str>) -> Result<(), String> {
    core_version()?;
    if m.format != FORMAT {
        return Err("unsupported deployment format".into());
    }
    if !matches!(m.game.as_str(), "arena" | "physics") {
        return Err("game must be arena or physics".into());
    }
    text(&m.game_code_identity, "game_code_identity")?;
    if m.frame_format_version != 2 {
        return Err("frame_format_version must be integer 2".into());
    }
    let id = number(&m.game_code_id, false)?;
    if number(&m.build_id, false)? != orr_sim::frame_build_id(id) {
        return Err("build_id does not match core frame_build_id".into());
    }
    if let Some(revision) = &m.provenance.source_revision {
        text(revision, "source_revision")?;
    }
    if m.provenance.binaries.len() > 32 {
        return Err("at most 32 binary records are allowed".into());
    }
    let mut seen = BTreeSet::new();
    for binary in &m.provenance.binaries {
        text(&binary.label, "binary label")?;
        if !seen.insert(&binary.label) {
            return Err("duplicate binary label".into());
        }
        if binary.sha256.len() != 64
            || !binary
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("binary sha256 must be lowercase SHA-256 hex".into());
        }
    }
    if game.is_some_and(|g| g != m.game) {
        return Err("game identity mismatch".into());
    }
    if identity.is_some_and(|i| i != m.game_code_identity) {
        return Err("game_code_identity mismatch".into());
    }
    Ok(())
}
pub(super) fn create(
    game: &str,
    identity: &str,
    id: &str,
    revision: Option<String>,
    binaries: Vec<BinaryRecord>,
) -> Result<Manifest, String> {
    core_version()?;
    let id = number(id, true)?;
    let m = Manifest {
        format: FORMAT.into(),
        game: game.into(),
        game_code_identity: identity.into(),
        game_code_id: id.to_string(),
        frame_format_version: 2,
        build_id: orr_sim::frame_build_id(id).to_string(),
        provenance: Provenance {
            source_revision: revision,
            binaries,
        },
    };
    validate(&m, None, None)?;
    Ok(m)
}
pub(super) fn load(
    bytes: &[u8],
    expected_game: Option<&str>,
    expected_identity: Option<&str>,
) -> Result<Manifest, String> {
    if bytes.len() > MAX_MANIFEST {
        return Err("manifest exceeds 1 MiB".into());
    }
    // Derived struct visitors reject duplicate fields at every schema object level.
    let m: Manifest = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    validate(&m, expected_game, expected_identity)?;
    Ok(m)
}
pub(super) fn check_previous(manifest: &Manifest, previous: &Manifest) -> Result<(), String> {
    validate(manifest, None, None)?;
    validate(previous, None, None)?;
    if previous.game_code_id == manifest.game_code_id
        && (previous.game != manifest.game
            || previous.game_code_identity != manifest.game_code_identity)
    {
        return Err("game_code_id was reused for a changed game or game_code_identity; release owner must assign a new game_code_id".into());
    }
    Ok(())
}
pub(super) fn export(m: &Manifest) -> Result<Value, String> {
    validate(m, None, None)?;
    let browser = if m.game == "arena" { "arena" } else { "phys" };
    Ok(json!({
        "format": FORMAT, "game": m.game, "game_code_id": m.game_code_id,
        "frame_format_version": m.frame_format_version, "build_id": m.build_id,
        "native_host": {"program":"orr_remote_host", "args":["--game",m.game,"--build-id",m.build_id]},
        "relay": {"program":"orr_server", "args":["--game",m.game,"--build-id",m.build_id]},
        "native_client": {"program":m.game,"args":["--build-id",m.build_id]},
        "browser":{"opts":{"game":browser,"build_id":m.build_id},"query":{"game":browser,"build":m.build_id}}
    }))
}
/// Python json.dumps(indent=2, sort_keys=True, ensure_ascii=True), plus newline.
/// Object keys are sorted by Unicode scalar order; array order is retained.
pub(super) fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    fn quoted(s: &str) -> String {
        let json = serde_json::to_string(s).expect("string serialization cannot fail");
        let mut out = String::new();
        for c in json.chars() {
            if c >= '\u{7f}' {
                for unit in c.encode_utf16(&mut [0; 2]) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            } else {
                out.push(c);
            }
        }
        out
    }
    fn write(v: &Value, depth: usize, out: &mut String) {
        let pad = "  ".repeat(depth + 1);
        match v {
            Value::Object(map) if !map.is_empty() => {
                out.push_str("{\n");
                let mut keys: Vec<_> = map.keys().collect();
                keys.sort();
                for (i, key) in keys.iter().enumerate() {
                    if i != 0 {
                        out.push_str(",\n");
                    }
                    out.push_str(&pad);
                    out.push_str(&quoted(key));
                    out.push_str(": ");
                    write(&map[*key], depth + 1, out);
                }
                out.push('\n');
                out.push_str(&"  ".repeat(depth));
                out.push('}');
            }
            Value::Array(array) if !array.is_empty() => {
                out.push_str("[\n");
                for (i, item) in array.iter().enumerate() {
                    if i != 0 {
                        out.push_str(",\n");
                    }
                    out.push_str(&pad);
                    write(item, depth + 1, out);
                }
                out.push('\n');
                out.push_str(&"  ".repeat(depth));
                out.push(']');
            }
            Value::String(s) => out.push_str(&quoted(s)),
            _ => out.push_str(&v.to_string()),
        }
    }
    let value = serde_json::to_value(value).map_err(|e| e.to_string())?;
    let mut out = String::new();
    write(&value, 0, &mut out);
    out.push('\n');
    if out.len() > MAX_MANIFEST {
        return Err("encoded manifest exceeds 1 MiB".into());
    }
    Ok(out.into_bytes())
}
#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
