//! Optional deployment records and pure launch plans. Never launches programs.
use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};
mod io;
mod schema;
use schema::BinaryRecord;
const USAGE:&str="orr_pkg deployment create --game arena|physics --game-code-identity TEXT --game-code-id NONZERO_U64 [--source-revision TEXT] [--binary LABEL=PATH]... [--previous FILE] --output NEW_FILE\norr_pkg deployment validate|export|id FILE [--expect-game arena|physics] [--expect-game-code-identity TEXT]\nOptional Linux deployment-authoring. Pure configuration/provenance records, no launches/network/signatures. No overwrite; bounded no-follow files. Legacy Python retained.";
fn text(value: OsString) -> Result<String, String> {
    let v = value
        .into_string()
        .map_err(|_| "text option must be UTF-8")?;
    if v.len() > 4096 || v.as_bytes().contains(&0) {
        return Err("text exceeds4096 bytes or contains NUL".into());
    }
    Ok(v)
}
fn bounded(arguments: impl IntoIterator<Item = OsString>) -> Result<Vec<OsString>, String> {
    let mut args = Vec::new();
    let mut bytes = 0usize;
    for a in arguments {
        bytes = bytes
            .checked_add(a.as_encoded_bytes().len() + 1)
            .ok_or("argv overflow")?;
        if bytes > 64 * 1024 || args.len() >= 80 {
            return Err("argv exceeds64KiB/80 arguments".into());
        }
        args.push(a);
    }
    Ok(args)
}
fn run(args: Vec<OsString>) -> Result<(), String> {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let command = args
        .first()
        .and_then(|a| a.to_str())
        .ok_or("missing deployment subcommand")?;
    if command == "create" {
        let mut unique = BTreeMap::new();
        let mut binaries = Vec::new();
        let mut iter = args[1..].chunks_exact(2);
        for pair in &mut iter {
            let key = pair[0].to_str().ok_or("invalid option")?;
            match key {
                "--binary" => {
                    if binaries.len() >= 32 {
                        return Err("at most32 binary provenance records".into());
                    }
                    let raw = pair[1].as_bytes();
                    let n = raw
                        .iter()
                        .position(|b| *b == b'=')
                        .ok_or("binary requires LABEL=PATH")?;
                    let label = text(OsString::from_vec(raw[..n].to_vec()))?;
                    let label = label
                        .trim_matches(|c: char| {
                            c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}')
                        })
                        .to_owned();
                    if label.is_empty() {
                        return Err("empty binary label".into());
                    }
                    let path = PathBuf::from(OsString::from_vec(raw[n + 1..].to_vec()));
                    binaries.push((label, path));
                }
                "--game"
                | "--game-code-identity"
                | "--game-code-id"
                | "--source-revision"
                | "--previous"
                | "--output" => {
                    if unique.insert(key.to_owned(), pair[1].clone()).is_some() {
                        return Err(format!("duplicate option {key}"));
                    }
                }
                _ => return Err(format!("unknown option {key}")),
            }
        }
        if !iter.remainder().is_empty() {
            return Err("missing option value".into());
        }
        let mut take = |k: &str| unique.remove(k).ok_or_else(|| format!("missing {k}"));
        let game = text(take("--game")?)?;
        let identity = text(take("--game-code-identity")?)?;
        let id = text(take("--game-code-id")?)?;
        let output = PathBuf::from(take("--output")?);
        let revision = unique.remove("--source-revision").map(text).transpose()?;
        let previous = unique.remove("--previous").map(PathBuf::from);
        let records = binaries
            .into_iter()
            .map(|(label, path)| {
                Ok(BinaryRecord {
                    label,
                    sha256: io::binary_hash(&path)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let manifest = schema::create(&game, &identity, &id, revision, records)?;
        if let Some(p) = previous {
            schema::check_previous(&manifest, &schema::load(&io::manifest(&p)?, None, None)?)?;
        }
        let warning = io::write_new(&output, &schema::encode(&manifest)?)?;
        println!("{}", output.display());
        if let Some(w) = warning {
            eprintln!("warning: complete manifest exists; durability uncertain: {w}; do not retry/overwrite/remove blindly");
        }
        Ok(())
    } else if ["validate", "export", "id"].contains(&command) {
        let path = PathBuf::from(args.get(1).ok_or("missing manifest path")?);
        let mut opts = BTreeMap::new();
        let mut iter = args[2..].chunks_exact(2);
        for p in &mut iter {
            let key = p[0].to_str().ok_or("invalid option")?;
            if !["--expect-game", "--expect-game-code-identity"].contains(&key)
                || opts.insert(key, text(p[1].clone())?).is_some()
            {
                return Err("unknown/duplicate expectation".into());
            }
        }
        if !iter.remainder().is_empty() {
            return Err("missing expectation value".into());
        }
        let m = schema::load(
            &io::manifest(&path)?,
            opts.get("--expect-game").map(String::as_str),
            opts.get("--expect-game-code-identity").map(String::as_str),
        )?;
        match command {
            "validate" => println!("valid"),
            "id" => println!("{}", m.build_id),
            _ => {
                use std::io::Write;
                std::io::stdout()
                    .write_all(&schema::encode(&schema::export(&m)?)?)
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    } else {
        Err("unknown deployment subcommand".into())
    }
}
fn valid_usage(args: &[OsString]) -> bool {
    let Some(command) = args.first().and_then(|a| a.to_str()) else {
        return false;
    };
    let (start, allowed, required): (usize, &[&str], &[&str]) = match command {
        "create" => (
            1,
            &[
                "--game",
                "--game-code-identity",
                "--game-code-id",
                "--source-revision",
                "--binary",
                "--previous",
                "--output",
            ],
            &[
                "--game",
                "--game-code-identity",
                "--game-code-id",
                "--output",
            ],
        ),
        "validate" | "export" | "id" if args.len() >= 2 => {
            (2, &["--expect-game", "--expect-game-code-identity"], &[])
        }
        _ => return false,
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut pairs = args[start..].chunks_exact(2);
    for p in &mut pairs {
        let Some(key) = p[0].to_str() else {
            return false;
        };
        if !allowed.contains(&key) || (key != "--binary" && !seen.insert(key)) {
            return false;
        }
        if ["--game", "--expect-game"].contains(&key)
            && !matches!(p[1].to_str(), Some("arena" | "physics"))
        {
            return false;
        }
    }
    pairs.remainder().is_empty() && required.iter().all(|r| seen.contains(r))
}
pub fn run_cli(arguments: impl IntoIterator<Item = OsString>) -> u8 {
    let args = match bounded(arguments) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("deployment usage: {e}");
            return 2;
        }
    };
    if args.len() == 1 && args[0] == "--help" {
        println!("{USAGE}");
        return 0;
    }
    if !valid_usage(&args) {
        eprintln!("invalid deployment arguments; {USAGE}");
        return 2;
    }
    match run(args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closed_usage_and_argv_bounds() {
        assert!(!valid_usage(&[]));
        assert!(!valid_usage(&["id".into()]));
        assert!(valid_usage(&["id".into(), "file".into()]));
        assert!(!valid_usage(&[
            "id".into(),
            "file".into(),
            "--command".into(),
            "x".into()
        ]));
        assert!(bounded(std::iter::repeat(OsString::from("x"))).is_err());
        assert!(bounded([OsString::from("x".repeat(65536))]).is_err());
    }
}
