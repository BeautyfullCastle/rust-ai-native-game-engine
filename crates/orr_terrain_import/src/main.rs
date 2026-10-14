use orr_fp::FP;
use orr_terrain_import::{publish::import_file, Endian, Recipe};
use std::{collections::BTreeMap, ffi::OsString, io::Write, path::PathBuf};
const HELP: &str = "Unsigned RAW16 to canonical .orrt (no install or overwrite)\n\
Usage: orr_terrain_import --input FILE --output NEW_FILE --width N --depth N \\\n\
  --endian le|be --height-offset DECIMAL --height-scale DECIMAL \\\n\
  --origin-x DECIMAL --origin-z DECIMAL --spacing DECIMAL --asset-id ID [--flip-z]\n\
Required: dimensions 2..129; scale >= 0; spacing > 0; exact byte length; exact f32 display coordinates\n\
Input and output parents must be trusted, stable and link-free. Output parent must already exist.\n\
Exit 0: published/help; 2: not published/error; 3: published with durability/cleanup warning.\n";

fn run(args: Vec<OsString>) -> Result<u8, String> {
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        std::io::stdout()
            .write_all(HELP.as_bytes())
            .map_err(|e| e.to_string())?;
        return Ok(0);
    }
    if args.len() > 23 {
        return Err("too many arguments".into());
    }
    let mut values = BTreeMap::new();
    let mut flip_z = false;
    let mut iter = args.into_iter();
    while let Some(flag) = iter.next() {
        let flag = flag
            .into_string()
            .map_err(|_| "option names must be UTF-8")?;
        if flag == "--flip-z" {
            if flip_z {
                return Err("duplicate --flip-z".into());
            }
            flip_z = true;
            continue;
        }
        if ![
            "--input",
            "--output",
            "--width",
            "--depth",
            "--endian",
            "--height-offset",
            "--height-scale",
            "--origin-x",
            "--origin-z",
            "--spacing",
            "--asset-id",
        ]
        .contains(&flag.as_str())
        {
            return Err(format!("unknown option {flag:?}; use --help"));
        }
        let value = iter
            .next()
            .ok_or_else(|| format!("missing value for {flag}"))?;
        if values.insert(flag.clone(), value).is_some() {
            return Err(format!("duplicate option {flag}"));
        }
    }
    let get = |name: &str| -> Result<&OsString, String> {
        values
            .get(name)
            .ok_or_else(|| format!("missing {name}; use --help"))
    };
    let text = |name: &str| -> Result<&str, String> {
        let value = get(name)?
            .to_str()
            .ok_or_else(|| format!("{name} must be UTF-8"))?;
        if value.len() > 1024 {
            return Err(format!("{name} value is too long"));
        }
        Ok(value)
    };
    let fixed = |name: &str| -> Result<FP, String> {
        FP::parse(text(name)?).map_err(|e| format!("invalid {name} decimal: {e:?}"))
    };
    let dimension = |name: &str| -> Result<u32, String> {
        let value = text(name)?;
        if value.is_empty() || !value.bytes().all(|v| v.is_ascii_digit()) {
            return Err(format!("invalid {name}: expected unsigned integer"));
        }
        value
            .parse()
            .map_err(|_| format!("invalid {name}: integer overflow"))
    };
    let recipe = Recipe {
        asset_id: text("--asset-id")?.into(),
        width: dimension("--width")?,
        depth: dimension("--depth")?,
        endian: match text("--endian")? {
            "le" => Endian::Little,
            "be" => Endian::Big,
            _ => return Err("--endian must be le or be".into()),
        },
        height_offset: fixed("--height-offset")?,
        height_scale: fixed("--height-scale")?,
        origin: [fixed("--origin-x")?, fixed("--origin-z")?],
        spacing: fixed("--spacing")?,
        flip_z,
    };
    let input = PathBuf::from(get("--input")?);
    let output = PathBuf::from(get("--output")?);
    let result = import_file(&input, &output, &recipe).map_err(|e| e.to_string())?;
    let revision: String = result.revision.iter().map(|b| format!("{b:02x}")).collect();
    // Publication is already committed. A broken stdout is a post-commit warning.
    let printed = writeln!(
        std::io::stdout(),
        "published {} bytes; revision {revision}",
        result.bytes
    );
    for warning in [&result.durability_warning, &result.cleanup_warning]
        .into_iter()
        .flatten()
    {
        let _ = writeln!(std::io::stderr(), "warning: {warning}");
    }
    if let Err(e) = &printed {
        let _ = writeln!(
            std::io::stderr(),
            "warning: published, but status output failed: {e}"
        );
    }
    Ok(if result.has_warnings() || printed.is_err() {
        3
    } else {
        0
    })
}
fn main() {
    let code = match run(std::env::args_os().skip(1).take(25).collect()) {
        Ok(code) => code,
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "error: {error}");
            2
        }
    };
    std::process::exit(i32::from(code));
}
