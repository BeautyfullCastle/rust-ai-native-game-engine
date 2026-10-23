#![allow(clippy::disallowed_types)]
//! Local-only explicit MeshHazardsV1 authoring entry point.
use bytemuck::Zeroable;
use orr_bridge::FrameView;
use orr_edit::{Op, Origin};
use orr_fp::FP;
use orr_remote::{
    mesh_navigation::{
        hazards::{self, HazardSphere, MeshHazards, HAZARDS_NAME, MESH_HAZARDS_GAME},
        mesh_doc_from_path, MeshScenePin,
    },
    Auth, ServerConfig,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::PathBuf,
    process::ExitCode,
    time::{Duration, Instant},
};

const HELP: &str = "orr_mesh_hazards upgrade --scene ABS.scene.yaml --output NEW.scene.yaml --spheres 'X,Y,Z,R[;X,Y,Z,R...]'\norr_mesh_hazards inspect --scene ABS.scene.yaml\norr_mesh_hazards serve --scene ABS.scene.yaml --dev-no-auth [--bind 127.0.0.1:7777] [--stop-after-ms N]\nExplicit opt-in MeshHazardsV1; upgrade never overwrites. 1..=8 fixed-point spheres. Local development only.";
type Options = BTreeMap<String, String>;

fn main() -> ExitCode {
    let args = match std::env::args_os()
        .skip(1)
        .take(18)
        .map(|arg| arg.into_string())
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(args) => args,
        Err(_) => {
            eprintln!("arguments must be UTF-8");
            return ExitCode::from(2);
        }
    };
    if args == ["--help"] || args == ["-h"] {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let (command, options) = match parse(&args) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("error: {error}\n{HELP}");
            return ExitCode::from(2);
        }
    };
    match execute(command, &options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn parse(args: &[String]) -> Result<(&str, Options), String> {
    if args.len() > 16 || args.iter().any(|arg| arg.len() > 4096) {
        return Err("argument limit exceeded".into());
    }
    let command = args.first().ok_or("expected command")?.as_str();
    let allowed: &[&str] = match command {
        "upgrade" => &["--scene", "--output", "--spheres"],
        "inspect" => &["--scene"],
        "serve" => &["--scene", "--bind", "--dev-no-auth", "--stop-after-ms"],
        _ => return Err(format!("unknown command '{command}'")),
    };
    let mut values = Options::new();
    let mut args = args[1..].iter();
    while let Some(flag) = args.next() {
        if !allowed.contains(&flag.as_str()) || values.contains_key(flag) {
            return Err(format!("unknown or duplicate option '{flag}'"));
        }
        let value = if flag == "--dev-no-auth" {
            "true".to_string()
        } else {
            let value = args
                .next()
                .filter(|value| !value.starts_with("--"))
                .ok_or_else(|| format!("{flag} needs a value"))?;
            value.clone()
        };
        values.insert(flag.clone(), value);
    }
    path(&values, "--scene")?;
    if command == "upgrade" {
        path(&values, "--output")?;
        spheres(required(&values, "--spheres")?)?;
    }
    if command == "serve" {
        if !values.contains_key("--dev-no-auth") {
            return Err("serve requires explicit --dev-no-auth (local development only)".into());
        }
        bind(&values)?;
        stop_after(&values)?;
    }
    Ok((command, values))
}

fn execute(command: &str, values: &Options) -> Result<(), String> {
    let scene = path(values, "--scene")?;
    match command {
        "upgrade" => {
            let document = upgraded(&scene, &spheres(required(values, "--spheres")?)?)?;
            let yaml = document.to_yaml();
            let reopened =
                hazards::hazards_doc_from_yaml(&yaml).map_err(|error| error.to_string())?;
            if reopened.frame().to_bytes() != document.frame().to_bytes() {
                return Err("upgraded scene failed exact round-trip".into());
            }
            orr_remote::mesh_navigation::authoring::create_new_scene_file(
                &path(values, "--output")?,
                &yaml,
            )?;
            println!("{}", readback(&reopened)?);
        }
        "inspect" => println!("{}", readback(&hazards::hazards_doc_from_path(&scene)?)?),
        "serve" => {
            let mut config = ServerConfig::new(Auth::DevNoAuth);
            config.bind = bind(values)?;
            let host = hazards::spawn_mesh_hazards_scene_host(scene, config)?;
            println!(
                "{}",
                json!({"game": MESH_HAZARDS_GAME, "url": host.url(), "auth": "explicit loopback development only"})
            );
            let deadline = stop_after(values)?.map(|duration| Instant::now() + duration);
            while deadline.is_none_or(|deadline| Instant::now() < deadline) {
                if !host.is_running() {
                    return Err("hazard host stopped unexpectedly".into());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        _ => return Err("unknown command".into()),
    }
    Ok(())
}

fn upgraded(
    source: &std::path::Path,
    spheres: &[HazardSphere],
) -> Result<orr_edit::EditorDoc, String> {
    // The legacy loader rejects hazard fields and nonzero runtime authoring.
    // Never reinterpret a live frame or silently select a new profile.
    let legacy = mesh_doc_from_path(source)?;
    let mut document =
        hazards::hazards_doc_from_yaml(&legacy.to_yaml()).map_err(|error| error.to_string())?;
    let mut config = MeshHazards::zeroed();
    config.enabled = 1;
    config.count = u32::try_from(spheres.len()).map_err(|_| "too many spheres")?;
    if spheres.is_empty() || spheres.len() > config.spheres.len() {
        return Err("expected 1..=8 spheres".into());
    }
    config.spheres[..spheres.len()].copy_from_slice(spheres);
    config.graph_revision = document.frame().singleton::<MeshScenePin>().graph_revision;
    orr_games::mesh_navigation_game::hazards::validate_config(
        &config,
        document.frame().singleton::<MeshScenePin>(),
    )?;
    let value = document
        .types()
        .get(HAZARDS_NAME)
        .ok_or("missing hazard reflection")?
        .read(bytemuck::bytes_of(&config));
    document
        .apply(
            Op::SetSingletonField {
                singleton: HAZARDS_NAME.into(),
                path: String::new(),
                value,
            },
            Origin::User,
        )
        .map_err(|error| error.to_string())?;
    Ok(document)
}

fn readback(document: &orr_edit::EditorDoc) -> Result<Value, String> {
    let frame = document.frame();
    let view = hazards::from_view(FrameView::of(frame))?;
    Ok(
        json!({"game": MESH_HAZARDS_GAME, "tick": frame.tick(), "checksum": format!("0x{:016x}", frame.checksum()),
            "enabled": view.is_some(), "count": view.as_ref().map_or(0, |view| view.config.count),
            "spheres": view.as_ref().map(|view| view.config.spheres[..view.config.count as usize].iter().map(|sphere| json!({"center_raw": sphere.center.map(|value| value.raw()), "radius_raw": sphere.radius.raw()})).collect::<Vec<_>>()).unwrap_or_default(),
            "lost": view.as_ref().map_or(0, |view| view.run.lost), "hit_index": view.as_ref().map_or(0, |view| view.run.hit_index), "lost_tick": view.as_ref().map_or(0, |view| view.run.lost_tick)
        }),
    )
}
fn required<'a>(values: &'a Options, flag: &str) -> Result<&'a str, String> {
    values
        .get(flag)
        .map(String::as_str)
        .ok_or_else(|| format!("missing {flag}"))
}
fn path(values: &Options, flag: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(required(values, flag)?);
    if !path.is_absolute() {
        return Err(format!("{flag} must be absolute"));
    }
    Ok(path)
}
fn spheres(value: &str) -> Result<Vec<HazardSphere>, String> {
    let parts: Vec<_> = value.split(';').collect();
    if !(1..=8).contains(&parts.len()) {
        return Err("expected 1..=8 spheres".into());
    }
    parts
        .into_iter()
        .map(|part| {
            let coords = part
                .split(',')
                .map(|value| {
                    FP::parse(value)
                        .map_err(|error| format!("invalid fixed-point coordinate: {error:?}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            if coords.len() != 4 || coords[3] <= FP::ZERO {
                return Err("sphere must be X,Y,Z,R with positive radius".into());
            }
            Ok(HazardSphere {
                center: [coords[0], coords[1], coords[2]],
                radius: coords[3],
            })
        })
        .collect()
}
fn bind(values: &Options) -> Result<SocketAddr, String> {
    let address = values
        .get("--bind")
        .map_or("127.0.0.1:7777", String::as_str)
        .parse::<SocketAddr>()
        .map_err(|error| error.to_string())?;
    if !address.ip().is_loopback() {
        return Err("serve only supports loopback".into());
    }
    Ok(address)
}
fn stop_after(values: &Options) -> Result<Option<Duration>, String> {
    values
        .get("--stop-after-ms")
        .map(|value| {
            let milliseconds = value
                .parse::<u64>()
                .map_err(|_| "--stop-after-ms must be a whole number")?;
            if !(1..=600_000).contains(&milliseconds) {
                return Err("--stop-after-ms must be 1..=600000".into());
            }
            Ok(Duration::from_millis(milliseconds))
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_fp::fp;
    use orr_navigation::{mesh::StaticMeshGraph, AgentProfile};
    use orr_remote::mesh_navigation::{authoring::create_mesh_scene, AgentSpec};
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    fn parse_args(args: &[&str]) -> bool {
        parse(
            &args
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>(),
        )
        .is_ok()
    }
    #[test]
    fn rejects_bad_flags_and_nonlocal_unauthenticated_serve() {
        let directory = std::env::temp_dir();
        let scene = directory.join("a.scene.yaml");
        let output = directory.join("b.scene.yaml");
        assert!(scene.is_absolute());
        assert!(output.is_absolute());
        let scene = scene.to_str().expect("test temp path must be UTF-8");
        let output = output.to_str().expect("test temp path must be UTF-8");
        assert!(parse_args(&["inspect", "--scene", scene]));
        for args in [
            vec!["inspect", "--scene", "relative"],
            vec!["inspect", "--scene", scene, "--scene", output],
            vec!["inspect", "--scene", "--unknown"],
            vec!["serve", "--scene", scene],
            vec![
                "serve",
                "--scene",
                scene,
                "--dev-no-auth",
                "--bind",
                "0.0.0.0:7777",
            ],
            vec![
                "serve",
                "--scene",
                scene,
                "--dev-no-auth",
                "--stop-after-ms",
                "0",
            ],
            vec!["upgrade", "--scene", scene, "--output", output],
        ] {
            assert!(!parse_args(&args), "{args:?}");
        }
        assert!(parse_args(&[
            "serve",
            "--scene",
            scene,
            "--dev-no-auth",
            "--bind",
            "[::1]:7777"
        ]));
    }
    #[test]
    fn rejects_bad_spheres() {
        for value in ["", "0,0,0", "0,0,0,0", "0,0,0,-1", "NaN,0,0,1", "0,0,0,1;"] {
            assert!(spheres(value).is_err(), "{value}");
        }
        assert!(spheres(&["0,0,0,1"; 9].join(";")).is_err());
        assert_eq!(spheres("-1,0,2,0.5;3,0,1,1").unwrap().len(), 2);
    }
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "orr-hazard-cli-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(fs::canonicalize(path).unwrap())
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn upgrade_preserves_source_and_reopens_zero_runtime_without_overwrite() {
        let fixture = Fixture::new();
        let source = fixture.0.join("legacy.scene.yaml");
        let output = fixture.0.join("hazards.scene.yaml");
        let graph = StaticMeshGraph::bake(
            &[
                [FP::ZERO; 3],
                [fp!(4), FP::ZERO, FP::ZERO],
                [FP::ZERO, FP::ZERO, fp!(4)],
                [fp!(4), FP::ZERO, fp!(4)],
            ],
            &[[0, 2, 3], [0, 3, 1]],
            AgentProfile::default(),
        )
        .unwrap();
        let mut document = create_mesh_scene(
            &graph,
            AgentSpec {
                start: [fp!(0.5), FP::ZERO, fp!(0.5)],
                goal: [fp!(3.5), FP::ZERO, fp!(3.5)],
                max_vertical: FP::ZERO,
                distance_per_tick: fp!(0.25),
            },
        )
        .unwrap();
        use orr_remote::mesh_navigation::mission::{
            MeshMission, MeshMissionRun, MissionPoint, MISSION_NAME,
        };
        let mut mission = MeshMission::zeroed();
        mission.enabled = 1;
        mission.count = 1;
        mission.graph_revision = graph.revision();
        mission.points[0] = MissionPoint {
            position: [fp!(1), FP::ZERO, fp!(1)],
            radius: fp!(0.25),
        };
        mission.exit = MissionPoint {
            position: [fp!(3), FP::ZERO, fp!(3)],
            radius: fp!(0.25),
        };
        let value = document
            .types()
            .get(MISSION_NAME)
            .unwrap()
            .read(bytemuck::bytes_of(&mission));
        document
            .apply(
                Op::SetSingletonField {
                    singleton: MISSION_NAME.into(),
                    path: String::new(),
                    value,
                },
                Origin::User,
            )
            .unwrap();
        let original = document.to_yaml();
        fs::write(&source, &original).unwrap();
        let args = vec![
            "upgrade".into(),
            "--scene".into(),
            source.display().to_string(),
            "--output".into(),
            output.display().to_string(),
            "--spheres".into(),
            "2,0,2,0.5".into(),
        ];
        let (command, options) = parse(&args).unwrap();
        execute(command, &options).unwrap();
        assert_eq!(fs::read_to_string(&source).unwrap(), original);
        let saved = fs::read(&output).unwrap();
        assert!(execute(command, &options).is_err());
        assert_eq!(fs::read(&output).unwrap(), saved);
        let reopened = hazards::hazards_doc_from_path(&output).unwrap();
        assert_eq!(*reopened.frame().singleton::<MeshMission>(), mission);
        assert_eq!(
            *reopened.frame().singleton::<MeshMissionRun>(),
            MeshMissionRun::zeroed()
        );
        let read = readback(&reopened).unwrap();
        assert_eq!(read["count"], 1);
        assert_eq!(read["lost"], 0);
        assert_eq!(read["lost_tick"], 0);
        assert_eq!(reopened.frame().tick(), 0);
        assert!(mesh_doc_from_path(&output).is_err());
        assert!(upgraded(&output, &spheres("2,0,2,0.5").unwrap()).is_err());
        let same = Options::from([
            ("--scene".into(), source.display().to_string()),
            ("--output".into(), source.display().to_string()),
            ("--spheres".into(), "2,0,2,0.5".into()),
        ]);
        assert!(execute("upgrade", &same).is_err());
        assert_eq!(fs::read_to_string(&source).unwrap(), original);
    }
    #[test]
    fn safe_output_rejects_wrong_extension_and_parent_traversal() {
        let fixture = Fixture::new();
        // PathBuf::join normalizes `..` for Windows verbatim paths returned
        // by canonicalize. Append raw path text so validation sees traversal.
        let mut traversal = fixture.0.as_os_str().to_os_string();
        traversal.push(format!("{0}..{0}bad.scene.yaml", std::path::MAIN_SEPARATOR));
        let traversal = PathBuf::from(traversal);
        assert!(traversal
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir)));
        for output in [fixture.0.join("bad.yaml"), traversal] {
            assert!(
                orr_remote::mesh_navigation::authoring::create_new_scene_file(&output, "").is_err()
            );
        }
    }
    #[cfg(unix)]
    #[test]
    fn safe_output_rejects_symlink_parent_and_leaf() {
        let fixture = Fixture::new();
        let link = fixture.0.join("linked");
        std::os::unix::fs::symlink(&fixture.0, &link).unwrap();
        assert!(
            orr_remote::mesh_navigation::authoring::create_new_scene_file(
                &link.join("out.scene.yaml"),
                ""
            )
            .is_err()
        );
        let target = fixture.0.join("target.scene.yaml");
        fs::write(&target, "preserve").unwrap();
        let leaf = fixture.0.join("leaf.scene.yaml");
        std::os::unix::fs::symlink(&target, &leaf).unwrap();
        assert!(orr_remote::mesh_navigation::authoring::create_new_scene_file(&leaf, "").is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "preserve");
    }
}
