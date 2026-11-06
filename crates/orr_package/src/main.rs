use orr_package::{inspect_for_install, Project, Runtime};
use std::{env, path::PathBuf};
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" {
        println!("orr_pkg <install PROJECT [--dry-run] --path SOURCE [--dependency-path SOURCE]... | remove PROJECT NAME [--dry-run] | restore PROJECT NAME DIGEST [--dry-run] | list PROJECT | explain PROJECT NAME | verify PROJECT [--package NAME] | inspect SOURCE>\nOptional Linux authoring: pack-coop-native --help (requires coop-native-authoring, trusted native runtime); create-coop-native-scene --help (requires coop-native-authoring feature, no Node/WASM); orr_pkg create-assembled/create-uv-model --help (requires model-authoring feature); create-animated-sequence --help (requires animation-authoring feature); create-sprite --help (requires sprite-authoring feature); create-coop-audio-bank[-from-wav] --help (requires audio-authoring feature); deployment --help (requires deployment-authoring feature); bootstrap-user-game --help (requires bootstrap-authoring feature); build-coop-recipe/copy-coop-web-assets/pack-coop-offline/create-coop-scene/create-coop-animation/compose-coop-animations --help (requires coop-web-authoring feature).\nOffline content only. Runtime capabilities must be validated by the consuming host.");
        return Ok(());
    }
    let engine = Runtime::content_only().engine_version;
    if args[0] == "inspect" && args.len() == 2 {
        println!(
            "{}",
            serde_json::to_string_pretty(&inspect_for_install(&args[1], engine)?)?
        );
    } else {
        let root = args.get(1).ok_or("project directory required")?;
        let project = Project::open_for_install(root, engine)?;
        if args[0] == "verify" && args.len() == 4 && args[2] == "--package" {
            println!(
                "{}",
                serde_json::to_string_pretty(&project.verify_package(&args[3])?)?
            );
            eprintln!("Scoped verification: complete lock structure admitted; only the requested package and its dependencies had installed bytes verified. Other active packages are listed in unverified_packages and their payloads were not checked. This is not whole-project or consuming-host validation; compiled capability validation is deferred to the consuming host.");
            return Ok(());
        }
        if args[0] == "explain" && args.len() == 3 {
            println!(
                "{}",
                serde_json::to_string_pretty(&project.explain(&args[2])?)?
            );
            eprintln!("Lock metadata only; installed bytes were not verified. Compiled capability validation is deferred to the consuming host.");
            return Ok(());
        }
        let lock = match args[0].as_str() {
            "install" if args.len() >= 4 => {
                let mut sources = Vec::new();
                let mut candidates = Vec::new();
                let dry_run = args.get(2).is_some_and(|arg| arg == "--dry-run");
                let start = if dry_run { 3 } else { 2 };
                let mut rest = args[start..].chunks_exact(2);
                for pair in &mut rest {
                    match pair[0].as_str() {
                        "--path" => sources.push(PathBuf::from(&pair[1])),
                        "--dependency-path" => candidates.push(PathBuf::from(&pair[1])),
                        _ => return Err("install expects --path or --dependency-path".into()),
                    }
                }
                if !rest.remainder().is_empty() {
                    return Err("source path missing".into());
                }
                if dry_run {
                    println!("{}", serde_json::to_string_pretty(
                        &project.preview_install_with_dependencies(&sources, &candidates)?
                    )?);
                    preview_limits();
                    return Ok(());
                }
                project.install_with_dependencies(&sources, &candidates)?
            }
            "remove" if args.len() == 4 && args[3] == "--dry-run" => {
                println!("{}", serde_json::to_string_pretty(&project.preview_remove(&args[2])?)?);
                preview_limits();
                return Ok(());
            }
            "remove" if args.len() == 3 => project.remove(&args[2])?,
            "restore" if args.len() == 5 && args[4] == "--dry-run" => {
                println!("{}", serde_json::to_string_pretty(
                    &project.preview_restore_cached(&args[2], &args[3])?
                )?);
                preview_limits();
                return Ok(());
            }
            "restore" if args.len() == 4 => {
                project.restore_cached(&args[2], &args[3])?
            }
            "list" if args.len() == 2 => project.list()?,
            "verify" if args.len() == 2 => project.verify()?,
            _ => return Err("invalid command/arguments; use --help".into()),
        };
        println!("{}", serde_json::to_string_pretty(&lock)?);
    }
    eprintln!("Content/engine validation only; compiled capability validation is deferred to the consuming host.");
    Ok(())
}
fn preview_limits() {
    eprintln!("Read-only content/graph preview; no activation or writer reservation. Future write permissions, writer availability and inactive destination objects are not guaranteed. Compiled capability validation is deferred to the consuming host.");
}
fn main() {
    if env::args_os().nth(1).is_some_and(|arg| arg == "pack-coop-native") {
        #[cfg(all(feature = "coop-native-authoring", target_os = "linux", target_arch = "x86_64"))]
        std::process::exit(i32::from(orr_package::coop_web_authoring::run_native_bundle_cli(env::args_os().skip(2))));
        #[cfg(not(all(feature = "coop-native-authoring", target_os = "linux", target_arch = "x86_64")))]
        { eprintln!("Native bundling requires Linux x86-64 with --features coop-native-authoring; no input/output opened"); std::process::exit(2); }
    }

    if env::args_os().nth(1).is_some_and(|arg| arg == "create-coop-native-scene") {
        #[cfg(all(feature = "coop-native-authoring", target_os = "linux"))]
        std::process::exit(i32::from(orr_package::coop_web_authoring::run_native_scene_cli(env::args_os().skip(2))));
        #[cfg(not(all(feature = "coop-native-authoring", target_os = "linux")))]
        { eprintln!("Native Scene authoring requires Linux with --features coop-native-authoring; no input/output opened"); std::process::exit(2); }
    }

    if env::args_os().nth(1).is_some_and(|arg| arg == "build-coop-recipe" || arg == "copy-coop-web-assets") {
        #[cfg(all(feature="coop-web-authoring",target_os="linux"))]
        {
            let recipe=env::args_os().nth(1).is_some_and(|arg|arg=="build-coop-recipe");
            let code=if recipe { orr_package::coop_web_authoring::run_recipe_cli(env::args_os().skip(2)) } else { orr_package::coop_web_authoring::run_copy_cli(env::args_os().skip(2)) };
            std::process::exit(i32::from(code));
        }
        #[cfg(not(all(feature="coop-web-authoring",target_os="linux")))]
        { eprintln!("Web recipe/copy requires Linux with coop-web-authoring; no input/output opened"); std::process::exit(2); }
    }

    if env::args_os().nth(1).is_some_and(|arg| arg == "pack-coop-offline") {
        #[cfg(all(feature="coop-web-authoring",target_os="linux"))]
        std::process::exit(i32::from(orr_package::coop_web_authoring::run_offline_cli(env::args_os().skip(2))));
        #[cfg(not(all(feature="coop-web-authoring",target_os="linux")))]
        { eprintln!("Offline web packing requires Linux with coop-web-authoring; no input/output opened"); std::process::exit(2); }
    }

    if env::args_os().nth(1).is_some_and(|arg| arg == "create-coop-scene") {
        #[cfg(all(feature = "coop-web-authoring", target_os = "linux"))]
        std::process::exit(i32::from(orr_package::coop_web_authoring::run_scene_cli(env::args_os().skip(2))));
        #[cfg(not(all(feature = "coop-web-authoring", target_os = "linux")))]
        { eprintln!("Web Scene authoring requires Linux with --features coop-web-authoring; no input/output opened"); std::process::exit(2); }
    }

    if env::args_os().nth(1).is_some_and(|arg| arg == "create-coop-animation" || arg == "compose-coop-animations") {
        #[cfg(all(feature = "coop-web-authoring", target_os = "linux"))]
        {
            let compose=env::args_os().nth(1).is_some_and(|arg|arg=="compose-coop-animations");
            std::process::exit(i32::from(orr_package::coop_web_authoring::run_cli(env::args_os().skip(2),compose)));
        }
        #[cfg(not(all(feature = "coop-web-authoring", target_os = "linux")))]
        { eprintln!("Web animation authoring requires Linux with --features coop-web-authoring; no input/output opened"); std::process::exit(2); }
    }

    if env::args_os().nth(1).is_some_and(|arg| arg == "bootstrap-user-game") {
        #[cfg(all(feature = "bootstrap-authoring", target_os = "linux"))]
        std::process::exit(i32::from(orr_package::user_game_bootstrap::run_cli(env::args_os().skip(2))));
        #[cfg(not(all(feature = "bootstrap-authoring", target_os = "linux")))]
        {
            eprintln!("Bootstrap requires Linux with --features bootstrap-authoring; no input/output opened");
            std::process::exit(2);
        }
    }

    if env::args_os().nth(1).is_some_and(|arg| arg == "deployment") {
        #[cfg(all(feature = "deployment-authoring", target_os = "linux"))]
        std::process::exit(i32::from(orr_package::deployment_manifest::run_cli(env::args_os().skip(2))));
        #[cfg(not(all(feature = "deployment-authoring", target_os = "linux")))]
        {
            eprintln!("Deployment authoring requires a Linux build with --features deployment-authoring; no input or output opened");
            std::process::exit(2);
        }
    }

    if env::args_os().nth(1).is_some_and(|arg| arg == "create-coop-audio-bank" || arg == "create-coop-audio-bank-from-wav") {
        #[cfg(all(feature = "audio-authoring", target_os = "linux"))]
        {
            let kind = if env::args_os().nth(1).is_some_and(|arg| arg == "create-coop-audio-bank-from-wav") { orr_package::coop_audio_bank::InputKind::Wav } else { orr_package::coop_audio_bank::InputKind::Pcm };
            std::process::exit(i32::from(orr_package::coop_audio_bank::run_cli(env::args_os().skip(2), kind)));
        }
        #[cfg(not(all(feature = "audio-authoring", target_os = "linux")))]
        {
            eprintln!("Audio bank authoring requires a Linux build with --features audio-authoring; no input or output opened");
            std::process::exit(2);
        }
    }

    if env::args_os().nth(1).is_some_and(|arg| arg == "create-sprite") {
        #[cfg(all(feature = "sprite-authoring", target_os = "linux"))]
        std::process::exit(i32::from(orr_package::sprite_package::run_cli(env::args_os().skip(2))));
        #[cfg(not(all(feature = "sprite-authoring", target_os = "linux")))]
        {
            eprintln!("orr_pkg create-sprite requires a Linux build with --features sprite-authoring; no input or output opened");
            std::process::exit(2);
        }
    }

    if env::args_os().nth(1).is_some_and(|arg| arg == "create-animated-sequence") {
        #[cfg(all(feature = "animation-authoring", target_os = "linux"))]
        std::process::exit(i32::from(orr_package::animated_sequence_package::run_cli(env::args_os().skip(2))));
        #[cfg(not(all(feature = "animation-authoring", target_os = "linux")))]
        {
            eprintln!("orr_pkg create-animated-sequence requires a Linux build with --features animation-authoring; no input or output opened");
            std::process::exit(2);
        }
    }

    if env::args_os().nth(1).is_some_and(|arg| arg == "create-assembled") {
        #[cfg(all(feature = "model-authoring", target_os = "linux"))]
        std::process::exit(i32::from(orr_package::assembled_package::run_cli(env::args_os().skip(2))));
        #[cfg(not(all(feature = "model-authoring", target_os = "linux")))]
        {
            eprintln!("orr_pkg create-assembled requires a Linux build with --features model-authoring; no input or output opened");
            std::process::exit(2);
        }
    }

    if env::args_os().nth(1).is_some_and(|arg| arg == "create-uv-model") {
        #[cfg(all(feature = "model-authoring", target_os = "linux"))]
        std::process::exit(i32::from(orr_package::uv_model_package::run_cli(env::args_os().skip(2))));
        #[cfg(not(all(feature = "model-authoring", target_os = "linux")))]
        {
            eprintln!("orr_pkg create-uv-model requires a Linux build with --features model-authoring; no input or output opened");
            std::process::exit(2);
        }
    }
    if let Err(e) = run() {
        eprintln!("orr_pkg: {e}");
        std::process::exit(1);
    }
}
