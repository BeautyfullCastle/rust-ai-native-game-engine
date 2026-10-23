#[cfg(feature = "navigation")]
#[path = "../mesh_model.rs"]
mod mesh_model;

const USAGE: &str = "Usage: orr_mesh_cook (--source <absolute .glb|.gltf|.orrmodel> | --project <absolute directory> --package <name> --asset <portable path>) --output <absolute .orrmesh> [--max-slope <fixed-decimal>]\n\nCook a static mesh for point-agent, open-sky navigation. Maximum slope defaults to 1 and must be in 0..=4. Package mode loads verified installed static models with the compiled models capability, never unchecked object paths.";

#[cfg(not(feature = "navigation"))]
fn main() {
    use std::ffi::OsString;

    let arguments: Vec<OsString> = std::env::args_os().skip(1).take(13).collect();
    if arguments.len() > 12 {
        eprintln!("at most 12 arguments are accepted\n\n{USAGE}");
        std::process::exit(2);
    }
    if arguments.len() == 1 && (arguments[0] == "--help" || arguments[0] == "-h") {
        println!("{USAGE}\n\nBuild with: cargo run -p orr_editor --features navigation --bin orr_mesh_cook -- --help");
        return;
    }
    eprintln!("orr_mesh_cook requires the navigation feature. Build with: cargo run -p orr_editor --features navigation --bin orr_mesh_cook -- --help\n\n{USAGE}");
    std::process::exit(1);
}

#[cfg(feature = "navigation")]
fn main() {
    match run(std::env::args_os().skip(1)) {
        Ok(()) => {}
        Err((code, message)) => {
            if !message.is_empty() {
                eprintln!("{message}");
            }
            std::process::exit(code);
        }
    }
}

#[cfg(feature = "navigation")]
fn run(arguments: impl IntoIterator<Item = std::ffi::OsString>) -> Result<(), (i32, String)> {
    use orr_fp::FP;
    use orr_navigation::AgentProfile;
    use std::fs;

    let options = match parse_arguments(arguments) {
        Ok(ParseResult::Help) => {
            println!("{USAGE}");
            return Ok(());
        }
        Ok(ParseResult::Options(options)) => options,
        Err(message) => return Err((2, format!("{message}\n\n{USAGE}"))),
    };

    let max_slope = orr_reflect::decimal::parse_fp(&options.max_slope)
        .map_err(|error| (2, format!("invalid --max-slope: {error:?}")))?;
    if max_slope.raw() < 0 || max_slope > FP::from_int(4) {
        return Err((2, "--max-slope must be in the canonical range 0..=4".into()));
    }

    if let Ok(metadata) = fs::symlink_metadata(&options.output) {
        let kind = if metadata.file_type().is_symlink() {
            "link"
        } else {
            "file"
        };
        return Err((
            1,
            format!("output {kind} already exists; refusing to overwrite it"),
        ));
    }

    let profile = AgentProfile {
        max_slope,
        ..AgentProfile::default()
    };
    let (graph, package_identity) = match options.source {
        Source::File(source) => (
            mesh_model::import_model(&source, profile)
                .map_err(|error| (1, format!("cannot cook source mesh: {error}")))?,
            None,
        ),
        Source::Package {
            project,
            package,
            asset,
        } => {
            use orr_model_bindings::model_bindings::{load_asset_from_project, ModelKind};

            mesh_model::check_existing_components(&project)
                .map_err(|error| (1, format!("invalid project path: {error}")))?;
            let mut runtime = orr_package::Runtime::content_only();
            runtime.capabilities.insert("models".into());
            let project = orr_package::Project::open(&project, runtime)
                .map_err(|error| (1, format!("cannot open model project: {error}")))?;
            let loaded = load_asset_from_project(&project, &package, &asset, ModelKind::Static)
                .map_err(|error| (1, format!("cannot load installed model: {error}")))?;
            let model = loaded
                .static_model()
                .ok_or_else(|| (1, "installed asset is not a static model".into()))?;
            let graph = mesh_model::cook_model(model, profile)
                .map_err(|error| (1, format!("cannot cook installed model: {error}")))?;
            (
                graph,
                Some((
                    loaded.package_digest().to_owned(),
                    loaded.source_hash().to_owned(),
                )),
            )
        }
    };
    mesh_model::save_graph(&options.output, &graph)
        .map_err(|error| (1, format!("cannot save cooked mesh: {error}")))?;

    let bytes = fs::metadata(&options.output)
        .map_err(|error| (1, format!("cannot inspect cooked mesh: {error}")))?
        .len();
    println!(
        "cooked mesh revision={} triangles={} bytes={bytes}",
        hex_revision(graph.revision()),
        graph.triangles().len()
    );
    if let Some((package_digest, source_hash)) = package_identity {
        println!("verified package digest={package_digest} source_hash={source_hash}");
    }
    println!("Navigation supports point agents on open-sky triangles only; radius, headroom, step height, physics and collision clearance are not represented.");
    Ok(())
}

#[cfg(feature = "navigation")]
struct Options {
    source: Source,
    output: std::path::PathBuf,
    max_slope: String,
}

#[cfg(feature = "navigation")]
enum Source {
    File(std::path::PathBuf),
    Package {
        project: std::path::PathBuf,
        package: String,
        asset: String,
    },
}

#[cfg(feature = "navigation")]
enum ParseResult {
    Help,
    Options(Options),
}

#[cfg(feature = "navigation")]
fn parse_arguments(
    arguments: impl IntoIterator<Item = std::ffi::OsString>,
) -> Result<ParseResult, String> {
    use std::path::PathBuf;

    let raw_arguments: Vec<_> = arguments.into_iter().take(13).collect();
    if raw_arguments.len() > 12 {
        return Err("at most 12 arguments are accepted".into());
    }
    let arguments = raw_arguments
        .into_iter()
        .map(|argument| {
            let text = argument
                .into_string()
                .map_err(|_| "arguments must be valid UTF-8".to_string())?;
            if text.len() > 4096 {
                return Err("arguments and paths must not exceed 4096 bytes".into());
            }
            Ok(text)
        })
        .collect::<Result<Vec<_>, String>>()?;
    if arguments
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        return Ok(ParseResult::Help);
    }

    let mut source = None;
    let mut project = None;
    let mut package = None;
    let mut asset = None;
    let mut output = None;
    let mut max_slope = None;
    let mut index = 0;
    while index < arguments.len() {
        let flag = arguments[index].as_str();
        index += 1;
        let value = arguments
            .get(index)
            .ok_or_else(|| format!("missing value for {flag}"))?;
        index += 1;
        match flag {
            "--source" => {
                if source.replace(PathBuf::from(value)).is_some() {
                    return Err("--source may be specified only once".into());
                }
            }
            "--output" => {
                if output.replace(PathBuf::from(value)).is_some() {
                    return Err("--output may be specified only once".into());
                }
            }
            "--project" => {
                if project.replace(PathBuf::from(value)).is_some() {
                    return Err("--project may be specified only once".into());
                }
            }
            "--package" => {
                if package.replace(value.clone()).is_some() {
                    return Err("--package may be specified only once".into());
                }
            }
            "--asset" => {
                if asset.replace(value.clone()).is_some() {
                    return Err("--asset may be specified only once".into());
                }
            }
            "--max-slope" => {
                if max_slope.replace(value.clone()).is_some() {
                    return Err("--max-slope may be specified only once".into());
                }
            }
            _ => return Err(format!("unknown argument '{flag}'")),
        }
    }

    let source = match (source, project, package, asset) {
        (Some(source), None, None, None) => {
            if !source.is_absolute() {
                return Err("--source must be an absolute path".into());
            }
            if !source
                .extension()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|extension| {
                    ["glb", "gltf", "orrmodel"]
                        .iter()
                        .any(|allowed| extension.eq_ignore_ascii_case(allowed))
                })
            {
                return Err("--source must use the .glb, .gltf or .orrmodel extension".into());
            }
            Source::File(source)
        }
        (None, Some(project), Some(package), Some(asset)) => {
            if !project.is_absolute() {
                return Err("--project must be an absolute path".into());
            }
            if package.is_empty() || package.len() > 64 || asset.is_empty() || asset.len() > 240 {
                return Err(
                    "--package and --asset must be nonempty and within 64/240 bytes".into(),
                );
            }
            Source::Package {
                project,
                package,
                asset,
            }
        }
        _ => return Err("choose --source alone or all of --project, --package and --asset".into()),
    };
    let output = output.ok_or_else(|| "missing required --output".to_string())?;
    let max_slope = max_slope.unwrap_or_else(|| "1".into());
    if !output.is_absolute() {
        return Err("--output must be an absolute path".into());
    }
    if !output
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("orrmesh"))
    {
        return Err("--output must use the .orrmesh extension".into());
    }
    if max_slope.len() > 4096 {
        return Err("arguments and paths must not exceed 4096 bytes".into());
    }

    Ok(ParseResult::Options(Options {
        source,
        output,
        max_slope,
    }))
}

#[cfg(feature = "navigation")]
fn hex_revision(revision: [u8; 32]) -> String {
    revision.iter().map(|byte| format!("{byte:02x}")).collect()
}
