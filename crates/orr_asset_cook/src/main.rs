use orr_asset::AssetRef;
use orr_asset_cook::authoring::AssetType;
use orr_asset_cook::{
    allocate_id, cook, hex, inspect_bundle, load_index, write_new_index, CookError, CookOptions,
    Result,
};
use std::collections::BTreeMap;
use std::path::PathBuf;

const HELP: &str = "orr_asset_cook: bounded ORAM v1 authoring tool\n\n\
  cook --index FILE --out NEW_DIR [--cache DIR] [--root a_...]... [--check]\n\
  inspect --bundle DIR\n\
  register --index FILE --out-index NEW_FILE --type TYPE --source PATH [--id a_...]\n\
  clone --index FILE --out-index NEW_FILE --from a_... --source PATH [--id a_...]\n\
  move --index FILE --out-index NEW_FILE --id a_... --source PATH\n\
  tombstone --index FILE --out-index NEW_FILE --id a_... [--root a_...]...\n\n\
TYPE is sim.motion_profile or view.impact_pcm16. Omitted new IDs use OS randomness.\n\
Paths in an index are relative to its directory. Authoring commands always write\n\
a new index; use the same directory to preserve relative source meaning. Roots\n\
are explicit fixture references, not a scan of arbitrary documents. --check is\n\
read-only and recomputes all output without cache use. Exit: 0 success, 1 error.";

struct Args {
    values: BTreeMap<String, String>,
    roots: Vec<AssetRef>,
    check: bool,
}
impl Args {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let mut parsed = Self {
            values: BTreeMap::new(),
            roots: Vec::new(),
            check: false,
        };
        let mut count = 0;
        while let Some(flag) = args.next() {
            count += 1;
            if count > 100 {
                return Err(CookError::invalid("too many options"));
            }
            if flag == "--check" {
                if parsed.check {
                    return Err(CookError::invalid("duplicate --check"));
                }
                parsed.check = true;
                continue;
            }
            let value = args
                .next()
                .ok_or_else(|| CookError::invalid(format!("missing value for {flag}")))?;
            if !flag.starts_with("--") || value.starts_with("--") || value.len() > 4096 {
                return Err(CookError::invalid("invalid option/value"));
            }
            if flag == "--root" {
                parsed.roots.push(value.parse()?);
            } else if parsed.values.insert(flag.clone(), value).is_some() {
                return Err(CookError::invalid(format!("duplicate option: {flag}")));
            }
        }
        Ok(parsed)
    }
    fn required(&mut self, flag: &str) -> Result<String> {
        self.values
            .remove(flag)
            .ok_or_else(|| CookError::invalid(format!("missing {flag}")))
    }
    fn optional(&mut self, flag: &str) -> Option<String> {
        self.values.remove(flag)
    }
    fn finish(&self, allow_roots: bool, allow_check: bool) -> Result<()> {
        if !self.values.is_empty()
            || (!allow_roots && !self.roots.is_empty())
            || (!allow_check && self.check)
        {
            return Err(CookError::invalid("unknown or inapplicable option"));
        }
        Ok(())
    }
}
fn run() -> Result<()> {
    let args = std::env::args_os()
        .skip(1)
        .map(|arg| {
            arg.into_string()
                .map_err(|_| CookError::invalid("arguments must be UTF-8"))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut args = args.into_iter();
    let command = args.next().unwrap_or_else(|| "--help".into());
    if command == "--help" || command == "help" {
        if args.next().is_some() {
            return Err(CookError::invalid("unexpected help argument"));
        }
        println!("{HELP}");
        return Ok(());
    }
    let mut args = Args::parse(args)?;
    if command == "cook" {
        let index = PathBuf::from(args.required("--index")?);
        let out = PathBuf::from(args.required("--out")?);
        let cache = args.optional("--cache").map(PathBuf::from);
        args.finish(true, true)?;
        let summary = cook(&CookOptions {
            index,
            out,
            cache,
            roots: args.roots,
            check: args.check,
        })?;
        println!(
            "sim {}\nview {}\nassets {} cache_hits {}",
            hex(&summary.sim_manifest_sha256),
            hex(&summary.view_manifest_sha256),
            summary.asset_count,
            summary.cache_hits
        );
        return Ok(());
    }
    if command == "inspect" {
        let path = PathBuf::from(args.required("--bundle")?);
        args.finish(false, false)?;
        use std::io::Write;
        std::io::stdout().write_all(&inspect_bundle(&path)?)?;
        return Ok(());
    }
    if !matches!(
        command.as_str(),
        "register" | "clone" | "move" | "tombstone"
    ) {
        return Err(CookError::invalid("unknown command; use --help"));
    }
    let input = PathBuf::from(args.required("--index")?);
    let output = PathBuf::from(args.required("--out-index")?);
    let mut index = load_index(&input)?;
    let id = match args.optional("--id") {
        Some(value) => value.parse()?,
        None if command == "register" || command == "clone" => allocate_id(&index)?,
        None => return Err(CookError::invalid("missing --id")),
    };
    match command.as_str() {
        "register" => {
            let asset_type = match args.required("--type")?.as_str() {
                "sim.motion_profile" => AssetType::Motion,
                "view.impact_pcm16" => AssetType::Impact,
                _ => return Err(CookError::invalid("unsupported asset type")),
            };
            let source = args.required("--source")?;
            args.finish(false, false)?;
            index.register(id, asset_type, source)?;
        }
        "clone" => {
            let from = args.required("--from")?.parse()?;
            let source = args.required("--source")?;
            args.finish(false, false)?;
            index.clone_entry(from, id, source)?;
        }
        "move" => {
            let source = args.required("--source")?;
            args.finish(false, false)?;
            index.move_source(id, source)?;
        }
        "tombstone" => {
            args.finish(true, false)?;
            index.tombstone(id, &args.roots)?;
        }
        _ => unreachable!(),
    }
    write_new_index(&output, &index)?;
    println!("{id}");
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
