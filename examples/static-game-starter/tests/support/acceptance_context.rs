//! Acceptance-only provenance adapter. Never imported by a delivered runtime.
#![allow(dead_code)]
use serde_json::Value;
use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const SCENES: [&str; 4] = [
    "delivery",
    "switchboard",
    "editor_delivery",
    "editor_switchboard",
];

pub fn compiled_digest() -> Option<&'static str> {
    option_env!("ORR_ACCEPTANCE_CONTEXT_SHA256")
}

pub struct Context {
    manifest: PathBuf,
    generated: Option<Generated>,
}
struct Generated {
    source_sha: String,
    digest: String,
    hidden: Vec<PathBuf>,
    scenes: BTreeMap<String, PathBuf>,
}
fn require(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(message.into())
    }
}
fn hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn canonical(path: &str, directory: bool) -> Result<PathBuf> {
    let path = PathBuf::from(path);
    require(path.is_absolute(), "acceptance paths must be absolute")?;
    require(
        path.canonicalize()? == path,
        "acceptance paths must be canonical",
    )?;
    let metadata = fs::symlink_metadata(&path)?;
    require(
        if directory {
            metadata.is_dir()
        } else {
            metadata.is_file()
        },
        "unexpected acceptance path type",
    )?;
    Ok(path)
}
fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .ok_or_else(|| format!("missing string acceptance field {key}").into())
}
fn sha256(path: &Path) -> Result<String> {
    let output = Command::new("sha256sum").arg(path).output()?;
    require(output.status.success(), "cannot hash acceptance context")?;
    let text = String::from_utf8(output.stdout)?;
    let digest = text
        .split_whitespace()
        .next()
        .ok_or("missing acceptance hash")?;
    require(hex(digest, 64), "invalid acceptance hash")?;
    Ok(digest.to_owned())
}
impl Context {
    pub fn load(manifest: &str) -> Result<Self> {
        Self::load_bound(
            manifest,
            std::env::var_os("ORR_ACCEPTANCE_CONTEXT").map(PathBuf::from),
            compiled_digest(),
            &std::env::var("GITHUB_SHA")?,
        )
    }
    fn load_bound(
        manifest: &str,
        path: Option<PathBuf>,
        bound: Option<&str>,
        gate_sha: &str,
    ) -> Result<Self> {
        let manifest = Path::new(manifest).canonicalize()?;
        match (path, bound) {
            (None, None) => Ok(Self {
                manifest,
                generated: None,
            }),
            (Some(path), Some(digest)) => {
                require(hex(digest, 64), "invalid compile-bound acceptance digest")?;
                let path = canonical(path.to_str().ok_or("non-UTF8 context path")?, false)?;
                require(
                    fs::metadata(&path)?.len() <= 1024 * 1024,
                    "acceptance context exceeds 1 MiB",
                )?;
                require(
                    sha256(&path)? == digest,
                    "acceptance context is not compile-bound",
                )?;
                let bytes = fs::read(&path)?;
                let value: Value = serde_json::from_slice(&bytes)?;
                let mut generated = Self::validate(&manifest, &value, gate_sha)?;
                // Detect a concurrent replacement between hashing and parsing.
                require(
                    fs::read(&path)? == bytes && sha256(&path)? == digest,
                    "acceptance context changed during admission",
                )?;
                generated.digest = digest.to_owned();
                generated.hidden.push(path);
                Ok(Self {
                    manifest,
                    generated: Some(generated),
                })
            }
            _ => Err(
                "generated acceptance context and compile-bound digest must occur together".into(),
            ),
        }
    }
    fn validate(manifest: &Path, value: &Value, gate_sha: &str) -> Result<Generated> {
        let object = value
            .as_object()
            .ok_or("acceptance context must be an object")?;
        let allowed = [
            "schema",
            "mode",
            "user_root",
            "engine_root",
            "gate_root",
            "user_source_commit",
            "hosted_gate_commit",
            "engine_commit",
            "hidden_source_roots",
            "acceptance_scenes",
        ];
        require(
            object.keys().all(|key| allowed.contains(&key.as_str())),
            "unknown acceptance context field",
        )?;
        require(
            value["schema"] == "orr.native-execution-context/1"
                && value["mode"] == "generated_user_project",
            "unsupported acceptance context schema or mode",
        )?;
        for key in ["user_source_commit", "hosted_gate_commit", "engine_commit"] {
            require(
                hex(field(value, key)?, 40),
                "acceptance commits must be lowercase 40-hex",
            )?;
        }
        require(
            field(value, "hosted_gate_commit")? == gate_sha,
            "hosted gate identity mismatch; GITHUB_SHA must remain unchanged",
        )?;
        let roots = [
            canonical(field(value, "user_root")?, true)?,
            canonical(field(value, "engine_root")?, true)?,
            canonical(field(value, "gate_root")?, true)?,
        ];
        require(
            roots[0] == manifest,
            "acceptance user root must match compiled manifest directory",
        )?;
        for (index, root) in roots.iter().enumerate() {
            require(
                roots.iter().enumerate().all(|(other, p)| {
                    index == other || (!p.starts_with(root) && !root.starts_with(p))
                }),
                "acceptance source roots must be distinct and nonnested",
            )?;
        }
        let hidden = value["hidden_source_roots"]
            .as_array()
            .ok_or("missing hidden source roots")?
            .iter()
            .map(|v| canonical(v.as_str().ok_or("hidden root must be a string")?, true))
            .collect::<Result<Vec<_>>>()?;
        require(
            roots.iter().all(|root| hidden.contains(root)),
            "hidden roots must include user, engine and gate roots",
        )?;
        let scenes = value["acceptance_scenes"]
            .as_object()
            .ok_or("missing acceptance scenes")?;
        require(
            scenes.len() == SCENES.len() && SCENES.iter().all(|key| scenes.contains_key(*key)),
            "exact four acceptance fixtures required",
        )?;
        let mut admitted = BTreeMap::new();
        for key in SCENES {
            let path = canonical(
                scenes[key].as_str().ok_or("scene path must be a string")?,
                false,
            )?;
            require(
                path.starts_with(&roots[0]),
                "acceptance scene must belong to user project",
            )?;
            admitted.insert(key.to_owned(), path);
        }
        Ok(Generated {
            source_sha: field(value, "user_source_commit")?.to_owned(),
            digest: String::new(),
            hidden,
            scenes: admitted,
        })
    }
    pub fn source_sha(&self) -> Result<String> {
        match &self.generated {
            Some(context) => Ok(context.source_sha.clone()),
            None => Ok(std::env::var("GITHUB_SHA")?),
        }
    }
    pub fn digest(&self) -> Option<&str> {
        self.generated
            .as_ref()
            .map(|context| context.digest.as_str())
    }
    pub fn hidden_roots(&self) -> Result<Vec<PathBuf>> {
        match &self.generated {
            Some(context) => Ok(context.hidden.clone()),
            None => Ok(vec![self.manifest.join("../..").canonicalize()?]),
        }
    }
    pub fn write_scene(&self, key: &str, expected: &str, output: &Path) -> Result<()> {
        let bytes = match &self.generated {
            Some(context) => {
                let path = context.scenes.get(key).ok_or("unknown acceptance scene")?;
                let bytes = fs::read(path)?;
                require(
                    bytes == expected.as_bytes(),
                    "committed acceptance scene differs from compiled canonical fixture",
                )?;
                bytes
            }
            None => expected.as_bytes().to_vec(),
        };
        fs::write(output, bytes)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> (tempfile::TempDir, Value) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        for name in ["user", "engine", "gate"] {
            fs::create_dir(root.join(name)).unwrap();
        }
        let mut scenes = serde_json::Map::new();
        for key in SCENES {
            let p = root.join("user").join(key);
            fs::write(&p, "fixture").unwrap();
            scenes.insert(key.into(), json!(p));
        }
        let value = json!({"schema":"orr.native-execution-context/1","mode":"generated_user_project","user_root":root.join("user"),"engine_root":root.join("engine"),"gate_root":root.join("gate"),"user_source_commit":"a".repeat(40),"hosted_gate_commit":"b".repeat(40),"engine_commit":"c".repeat(40),"hidden_source_roots":[root.join("user"),root.join("engine"),root.join("gate")],"acceptance_scenes":scenes});
        (temp, value)
    }
    fn validate(v: &Value) -> Result<Generated> {
        Context::validate(
            Path::new(v["user_root"].as_str().unwrap()),
            v,
            &"b".repeat(40),
        )
    }
    #[test]
    fn distinct_user_identity_and_complete_roots_are_accepted() {
        let (_temp, value) = fixture();
        assert_eq!(validate(&value).unwrap().source_sha, "a".repeat(40));
    }
    #[test]
    fn rejects_bad_provenance_roots_and_scene_escape() {
        let (_temp, value) = fixture();
        for (key, replacement) in [
            ("hosted_gate_commit", json!("d".repeat(40))),
            ("user_source_commit", json!("A".repeat(40))),
            ("engine_root", value["user_root"].clone()),
            ("hidden_source_roots", json!([])),
            ("schema", json!("unknown")),
            ("typo", json!(true)),
        ] {
            let mut bad = value.clone();
            bad[key] = replacement;
            assert!(validate(&bad).is_err(), "{key}");
        }
        let mut bad = value.clone();
        let outside = Path::new(value["gate_root"].as_str().unwrap()).join("outside.scene");
        fs::write(&outside, "fixture").unwrap();
        bad["acceptance_scenes"]["delivery"] = json!(outside);
        assert!(validate(&bad).is_err());
        let mut bad = value.clone();
        bad["acceptance_scenes"]
            .as_object_mut()
            .unwrap()
            .remove("switchboard");
        assert!(validate(&bad).is_err());
    }
    #[test]
    fn rejects_unbound_injected_omitted_and_modified_context() {
        let (_temp, value) = fixture();
        let user = value["user_root"].as_str().unwrap();
        let path = Path::new(user).join("context.json");
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let digest = sha256(&path).unwrap();
        assert!(Context::load_bound(user, Some(path.clone()), None, &"b".repeat(40)).is_err());
        assert!(Context::load_bound(user, None, Some(&digest), &"b".repeat(40)).is_err());
        let context =
            Context::load_bound(user, Some(path.clone()), Some(&digest), &"b".repeat(40)).unwrap();
        assert!(context.hidden_roots().unwrap().contains(&path));
        assert!(context
            .write_scene(
                "delivery",
                "different fixture",
                &Path::new(user).join("out")
            )
            .is_err());
        fs::write(&path, "{}").unwrap();
        assert!(Context::load_bound(user, Some(path), Some(&digest), &"b".repeat(40)).is_err());
        assert!(Context::load_bound(user, None, None, &"b".repeat(40))
            .unwrap()
            .digest()
            .is_none());
    }
}
