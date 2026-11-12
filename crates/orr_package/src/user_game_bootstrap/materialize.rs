//! Pure transformation of a verified, pinned starter snapshot. Never runs tools.
use super::Profile;
use regex::Regex;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};
const MAX_FILE: usize = 32 * 1024 * 1024;
const MAX_TOTAL: usize = 64 * 1024 * 1024;
fn ascii_json<T: Serialize>(value: &T, pretty: bool) -> Result<String, String> {
    let json = if pretty {
        serde_json::to_string_pretty(value)
    } else {
        serde_json::to_string(value)
    }
    .map_err(|e| e.to_string())?;
    let mut result = String::new();
    for c in json.chars() {
        if c >= '\u{7f}' {
            for unit in c.encode_utf16(&mut [0; 2]) {
                result.push_str(&format!("\\u{unit:04x}"));
            }
        } else {
            result.push(c);
        }
    }
    Ok(result)
}
fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("fixed bootstrap pattern")
}
fn project_manifest(
    original: &[u8],
    engine: &Path,
    name: &str,
    profile: Profile,
) -> Result<Vec<u8>, String> {
    let mut text = std::str::from_utf8(original)
        .map_err(|e| e.to_string())?
        .to_owned();
    text = text.replacen(
        "name = \"orr_static_starter\"",
        &format!("name = {}", ascii_json(&name, false)?),
        1,
    );
    for (from, to) in [
        ("version.workspace = true", "version = \"0.1.0\""),
        ("edition.workspace = true", "edition = \"2021\""),
        (
            "license.workspace = true",
            "license = \"MIT OR Apache-2.0\"",
        ),
        (
            "[lints]\nworkspace = true",
            "[lints.clippy]\nfloat_arithmetic = \"warn\"",
        ),
    ] {
        text = text.replacen(from, to, 1);
    }
    let paths = regex(r#"path = "../../crates/([a-z0-9_]+)""#);
    // Resolve all replacements before mutation so non-UTF8 host paths fail closed.
    let mut replacements = Vec::new();
    for capture in paths.captures_iter(&text) {
        let path = engine.join("crates").join(&capture[1]);
        replacements.push(format!(
            "path = {}",
            ascii_json(&path.to_str().ok_or("engine path must be UTF-8")?, false)?
        ));
    }
    let mut replacements = replacements.into_iter();
    text = paths
        .replace_all(&text, |_: &regex::Captures<'_>| {
            replacements.next().unwrap()
        })
        .into_owned();
    text = regex(r"\nstandalone-native-acceptance = \[[^\n]+\]\n")
        .replace_all(&text, "\n")
        .into_owned();
    text=text.replace("[[test]]\nname = \"standalone_x11\"\npath = \"tests/standalone_x11.rs\"\nrequired-features = [\"standalone-native-acceptance\"]\n","");
    if matches!(profile, Profile::MultiActionPreview) {
        for feature in [
            "multi-action-standalone-acceptance",
            "multi-action-editor-acceptance",
        ] {
            text = regex(&format!(r"(?m)^{} = \[[^\n]+\]\n", regex::escape(feature)))
                .replace_all(&text, "")
                .into_owned();
        }
        // Rust regex has no lookahead. This is the same non-greedy block boundary
        // as Python's (?=^\[|\Z): stop at the next line starting '[' or EOF.
        let header = regex(
            r#"(?m)^\[\[(?:bin|test)\]\]\nname = "(?:multi-action-fixture-player|multi_action_standalone_x11|multi_action_editor_x11)"\n"#,
        );
        let boundary = regex(r"(?m)^\[");
        let mut cuts = Vec::new();
        for m in header.find_iter(&text) {
            let end = boundary
                .find(&text[m.end()..])
                .map_or(text.len(), |next| m.end() + next.start());
            cuts.push(m.start()..end);
        }
        for range in cuts.into_iter().rev() {
            text.replace_range(range, "");
        }
        let parsed: toml::Value = toml::from_str(&text).map_err(|e| e.to_string())?;
        let features = parsed
            .get("features")
            .and_then(toml::Value::as_table)
            .ok_or("preview features missing")?;
        let expected = vec![
            toml::Value::String("dep:orr_input".into()),
            toml::Value::String("orr_editor?/static-game-actions".into()),
        ];
        if features
            .get("static-game-actions")
            .and_then(toml::Value::as_array)
            != Some(&expected)
            || features
                .get("default")
                .and_then(toml::Value::as_array)
                .is_some_and(|a| a.iter().any(|v| v.as_str() == Some("static-game-actions")))
        {
            return Err("preview optional action feature contract changed".into());
        }
    }
    text.push_str("\n[lib]\nname = \"orr_static_starter\"\n\n[workspace]\nresolver = \"2\"\n\n[profile.release]\ndebug = 1\ncodegen-units = 1\nlto = \"thin\"\n");
    let parsed: toml::Value = toml::from_str(&text).map_err(|e| e.to_string())?;
    let package = parsed
        .get("package")
        .and_then(toml::Value::as_table)
        .ok_or("package table missing")?;
    if package.values().any(|v| {
        v.as_table()
            .and_then(|t| t.get("workspace"))
            .is_some_and(truthy)
    }) {
        return Err("remaining package workspace inheritance".into());
    }
    Ok(text.into_bytes())
}
fn truthy(v: &toml::Value) -> bool {
    match v {
        toml::Value::Boolean(b) => *b,
        toml::Value::String(s) => !s.is_empty(),
        toml::Value::Integer(n) => *n != 0,
        toml::Value::Float(n) => *n != 0.0,
        toml::Value::Array(a) => !a.is_empty(),
        toml::Value::Table(t) => !t.is_empty(),
        toml::Value::Datetime(_) => true,
    }
}
fn validate_files(files: &BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    if files.len() > 128 {
        return Err("generated file count exceeds 128".into());
    }
    let mut total = 0usize;
    for (path, data) in files {
        if path.is_empty()
            || path.len() > 4096
            || path.contains(['\\', '\0'])
            || path
                .split('/')
                .any(|p| p.is_empty() || matches!(p, "." | ".." | ".git" | "target"))
        {
            return Err("unsafe generated relative path".into());
        }
        if data.len() > MAX_FILE {
            return Err("generated file exceeds 32 MiB".into());
        }
        total = total
            .checked_add(data.len())
            .ok_or("generated byte size overflow")?;
        if total > MAX_TOTAL {
            return Err("generated files exceed 64 MiB".into());
        }
    }
    for path in files.keys() {
        let mut prefix = path.as_str();
        while let Some((parent, _)) = prefix.rsplit_once('/') {
            if files.contains_key(parent) {
                return Err("generated file/directory collision".into());
            }
            prefix = parent;
        }
    }
    Ok(())
}
#[derive(Serialize)]
struct Receipt<'a> {
    schema: u32,
    engine_commit: &'a str,
    engine_tree: &'a str,
    engine_path: &'a str,
    project_name: &'a str,
    user_source_commit: Option<&'a str>,
    lockfile: &'a str,
    omitted_engine_only_tests: Vec<&'a str>,
    files_sha256: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    template_profile: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stability: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hosted_native_acceptance: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    action_feature: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    action_feature_default: Option<bool>,
}
pub(super) fn generated_files(
    snapshot: BTreeMap<String, Vec<u8>>,
    engine: &Path,
    name: &str,
    profile: Profile,
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    validate_files(&snapshot)?;
    let cargo = project_manifest(
        snapshot
            .get("Cargo.toml")
            .ok_or("starter Cargo.toml missing")?,
        engine,
        name,
        profile,
    )?;
    let upstream = snapshot
        .get("README.md")
        .ok_or("starter README.md missing")?
        .clone();
    let mut files: BTreeMap<_, _> = snapshot
        .into_iter()
        .filter(|(p, _)| {
            !profile.omitted().contains(&p.as_str()) && p != "Cargo.toml" && p != "README.md"
        })
        .collect();
    files.insert("Cargo.toml".into(), cargo);
    files.insert("UPSTREAM_README.md".into(), upstream);
    let mut readme = include_str!("project_readme.txt")
        .replace("{{PROJECT_NAME}}", name)
        .into_bytes();
    let preview = matches!(profile, Profile::MultiActionPreview);
    if preview {
        if !files.contains_key("tests/multi_action_contract.rs") {
            return Err("preview portable action contract missing".into());
        }
        readme.extend_from_slice(include_bytes!("preview_readme.txt"));
    }
    files.insert("README.md".into(), readme);
    files.insert(".gitignore".into(), b"/target/\n".to_vec());
    let mut omitted = profile.omitted().to_vec();
    omitted.sort();
    let receipt = Receipt {
        schema: 1,
        engine_commit: profile.commit(),
        engine_tree: profile.tree(),
        engine_path: engine.to_str().ok_or("engine path must be UTF-8")?,
        project_name: name,
        user_source_commit: None,
        lockfile: "not generated; owned by user project",
        omitted_engine_only_tests: omitted,
        files_sha256: files
            .iter()
            .map(|(p, b)| {
                (
                    p.clone(),
                    Sha256::digest(b)
                        .iter()
                        .map(|v| format!("{v:02x}"))
                        .collect(),
                )
            })
            .collect(),
        template_profile: preview.then_some(profile.name()),
        stability: preview.then_some("preview"),
        hosted_native_acceptance: preview.then_some("unverified"),
        action_feature: preview.then_some("static-game-actions"),
        action_feature_default: preview.then_some(false),
    };
    files.insert(
        "provenance.json".into(),
        (ascii_json(&receipt, true)? + "\n").into_bytes(),
    );
    validate_files(&files)?;
    Ok(files)
}
#[cfg(test)]
#[path = "materialize_tests.rs"]
mod tests;
