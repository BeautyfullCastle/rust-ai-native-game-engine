//! Deterministic, paused headless playback. No interactive or hardware-portability claim.
use crate::{editor::Editor, game::EditorGame};
use orr_remote::mesh_navigation::mesh_from_view;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    io::Write,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Trajectory {
    pub tick: u64,
    pub verified_tick: u64,
    pub checksum: u64,
    pub agent_position_raw: [i64; 3],
    pub status: String,
    pub graph_revision: [u8; 32],
}

struct Options {
    bundle: PathBuf,
    ticks: u32,
    size: (u32, u32),
    capture: Option<PathBuf>,
    restart: bool,
}
enum Request {
    Capabilities,
    Help,
    Run(Options),
}
fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Request, String> {
    let mut bounded = Vec::new();
    let mut bytes = 0usize;
    for arg in args {
        bytes = bytes
            .checked_add(arg.as_bytes().len())
            .ok_or("argument size overflow")?;
        if bounded.len() >= 16 || arg.as_bytes().len() > 4096 || bytes > 16384 {
            return Err("argument count or byte limit exceeded".into());
        }
        bounded.push(arg);
    }
    let args = bounded;
    if args == [OsString::from("--help")] {
        return Ok(Request::Help);
    }
    if args == [OsString::from("--capabilities")] {
        return Ok(Request::Capabilities);
    }
    let mut it = args.into_iter();
    let mut seen = BTreeSet::new();
    let mut bundle = None;
    let mut ticks = 0;
    let mut size = (640, 480);
    let mut capture = None;
    let mut restart = false;
    while let Some(flag) = it.next() {
        let flag = flag.to_str().ok_or("option must be UTF-8")?;
        if !seen.insert(flag.to_owned()) {
            return Err(format!("duplicate option {flag}"));
        }
        match flag {
            "--restart-check" => restart = true,
            "--no-capture" => {}
            "--bundle" | "--capture" | "--ticks" | "--size" => {
                let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
                if value.is_empty() {
                    return Err(format!("{flag} needs a nonempty value"));
                }
                match flag {
                    "--bundle" => bundle = Some(PathBuf::from(value)),
                    "--capture" => capture = Some(PathBuf::from(value)),
                    "--ticks" => {
                        ticks = value
                            .to_str()
                            .ok_or("ticks must be UTF-8")?
                            .parse::<u32>()
                            .map_err(|_| "invalid tick count")?;
                        if ticks > 6000 {
                            return Err("ticks exceeds 6000".into());
                        }
                    }
                    _ => {
                        let value = value.to_str().ok_or("size must be UTF-8")?;
                        let (w, h) = value.split_once('x').ok_or("size must be WxH")?;
                        let w = w.parse::<u32>().map_err(|_| "invalid width")?;
                        let h = h.parse::<u32>().map_err(|_| "invalid height")?;
                        if !(64..=2048).contains(&w)
                            || !(64..=2048).contains(&h)
                            || u64::from(w) * u64::from(h) > 1_048_576
                        {
                            return Err(
                                "size must be 64..2048 per axis and at most 1048576 pixels".into(),
                            );
                        }
                        size = (w, h);
                    }
                }
            }
            _ => return Err(format!("unknown or non-standalone option {flag}")),
        }
    }
    if capture.is_some() && seen.contains("--no-capture") {
        return Err("--capture conflicts with --no-capture".into());
    }
    Ok(Request::Run(Options {
        bundle: bundle.ok_or("--bundle DIR is required")?,
        ticks,
        size,
        capture,
        restart,
    }))
}

fn fail_status(editor: &Editor) -> Result<(), String> {
    if let Some(s) = editor.status().filter(|s| s.error) {
        Err(s.text.clone())
    } else {
        Ok(())
    }
}
fn advance(editor: &mut Editor, ticks: u32) -> Result<Trajectory, String> {
    if ticks > 6000 {
        return Err("ticks exceeds 6000".into());
    }
    if !editor.start_play() {
        return Err(editor
            .status()
            .map(|s| s.text.clone())
            .unwrap_or_else(|| "cannot start mesh host".into()));
    }
    if ticks > 0 {
        editor.step(ticks);
    }
    editor.sync();
    fail_status(editor)?;
    let timeline = editor.timeline().ok_or("missing play timeline")?;
    if timeline.playing || timeline.tick != u64::from(ticks) {
        return Err("host did not remain paused at requested tick".into());
    }
    let snapshot = editor.snapshot().ok_or("missing host snapshot")?;
    let frame = snapshot.verified().ok_or("missing verified frame")?;
    if snapshot.tick() != u64::from(ticks)
        || snapshot.verified_tick() != u64::from(ticks)
        || frame.tick() != u64::from(ticks)
        || snapshot.predicted().checksum() != frame.checksum()
        || editor.checksum() != frame.checksum()
    {
        return Err("incoherent host snapshot".into());
    }
    let view = mesh_from_view(frame)?;
    Ok(Trajectory {
        tick: frame.tick(),
        verified_tick: snapshot.verified_tick(),
        checksum: frame.checksum(),
        agent_position_raw: view.agent.position.map(|v| v.raw()),
        status: format!("{:?}", view.agent.status),
        graph_revision: view.graph.revision(),
    })
}
fn open(source: &super::SavedSource) -> Result<Editor, String> {
    source.recheck()?;
    let mut editor = Editor::open_game(source.scene_path(), EditorGame::MeshNavigation)?;
    editor.sync();
    fail_status(&editor)?;
    let snapshot = editor.snapshot().ok_or("missing initial mesh snapshot")?;
    let view = mesh_from_view(snapshot.predicted())?;
    if editor.checksum() != source.initial_checksum()
        || snapshot.predicted().checksum() != source.initial_checksum()
        || view.graph.revision() != source.graph_revision()
    {
        return Err("host admission differs from saved source".into());
    }
    source.recheck()?;
    Ok(editor)
}
pub(crate) fn trajectory(scene: &Path, ticks: u32) -> Result<Trajectory, String> {
    let source = super::SavedSource::capture(scene)?;
    let mut editor = open(&source)?;
    let result = advance(&mut editor, ticks)?;
    source.recheck()?;
    Ok(result)
}

struct CaptureTarget {
    path: PathBuf,
    parent: PathBuf,
    directory: std::fs::File,
}
fn capture_path(path: &Path, root: &Path) -> Result<CaptureTarget, String> {
    let path = super::io::checked_path(path)?;
    if path.extension().and_then(|v| v.to_str()) != Some("png") {
        return Err("capture must name a new .png file".into());
    }
    let parent = path.parent().ok_or("capture parent missing")?.to_owned();
    let directory = super::io::directory(&parent)?;
    let root = super::io::checked_path(root)?;
    if path.starts_with(&root) {
        return Err("capture must be outside bundle".into());
    }
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
        Ok(_) => return Err("capture already exists".into()),
    }
    super::io::recheck_directory(&parent, &directory)?;
    Ok(CaptureTarget {
        path,
        parent,
        directory,
    })
}

fn render(
    editor: &Editor,
    scene: &Path,
    revision: [u8; 32],
    size: (u32, u32),
) -> Result<(Vec<u8>, String, serde_json::Value), String> {
    let mut panel = crate::mesh_navigation::Panel::default();
    panel.sync(editor);
    if let Some(error) = panel.play_blocker(editor) {
        return Err(error);
    }
    let model = panel.model().ok_or("navigation base model unavailable")?;
    let props = crate::mesh_props::load(scene, revision)?
        .ok_or("saved props missing")?
        .0;
    let mut models = props.placements()?;
    models.insert(
        0,
        crate::viewport3d::SceneModel {
            model,
            instance: Default::default(),
            shading: None,
        },
    );
    let snapshot = editor.snapshot().ok_or("missing capture snapshot")?;
    let frame = snapshot
        .verified()
        .ok_or("missing verified capture frame")?;
    let view = mesh_from_view(frame)?;
    let camera = match crate::mesh_camera_store::load(scene, revision)? {
        Some((doc, _)) => doc.camera_for_view(&doc.orbit(), size, &view)?,
        None => editor.camera3d.camera(),
    };
    let mut list = orr_render::RenderList3D::default();
    let settings = match crate::mesh_lighting_store::load(scene, revision)? {
        Some((doc, _)) => {
            list.lighting = doc.lighting()?;
            doc.post_process_settings()
        }
        None => {
            list.lighting = Default::default();
            Default::default()
        }
    };
    let rhi =
        orr_rhi::Wgpu::headless(Default::default()).map_err(|e| format!("headless GPU: {e}"))?;
    let mut gpu = crate::viewport3d::Viewport3dGpu::new(&rhi, size);
    gpu.render_scene(
        size,
        &list,
        &camera,
        &[],
        &models,
        #[cfg(feature = "animated-models")]
        &[],
        settings,
        #[cfg(feature = "irradiance-probes")]
        None,
    )?;
    let pixels = gpu.read_rgba8();
    if pixels.len() != size.0 as usize * size.1 as usize * 4 {
        return Err("invalid capture byte count".into());
    }
    let projection = match camera.projection {
        orr_render::Projection::Perspective { fov_y, near, far } => {
            serde_json::json!({"kind":"perspective","fov_y_radians":fov_y,"near":near,"far":far})
        }
        orr_render::Projection::Orthographic {
            half_height,
            near,
            far,
        } => {
            serde_json::json!({"kind":"orthographic","half_height":half_height,"near":near,"far":far})
        }
    };
    let applied_camera = serde_json::json!({"eye":camera.eye,"target":camera.target,"up":camera.up,"projection":projection});
    Ok((pixels, gpu.adapter_name(), applied_camera))
}

pub fn run_cli(args: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let options = match parse(args)? {
        Request::Help => {
            println!("orr_mesh_prop_player --bundle DIR [--ticks 0..6000] [--size WxH] [--capture NEW.png | --no-capture] [--restart-check]\nHeadless paused playback; defaults: tick 0, size 640x480, no capture.\norr_mesh_prop_player --capabilities");
            return Ok(());
        }
        Request::Capabilities => {
            println!("ORR_MESH_PROP_HEADLESS_V1");
            return Ok(());
        }
        Request::Run(options) => options,
    };
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let bundle = super::verify_bundle(&options.bundle, &exe)?;
    let output = options
        .capture
        .as_deref()
        .map(|p| capture_path(p, &options.bundle))
        .transpose()?;
    let source = super::SavedSource::capture(&bundle.scene)?;
    let mut editor = open(&source)?;
    let first = advance(&mut editor, options.ticks)?;
    let first_agent = mesh_from_view(
        editor
            .snapshot()
            .ok_or("missing first snapshot")?
            .predicted(),
    )?
    .agent;
    if first.graph_revision != bundle.manifest.graph_revision {
        return Err("play graph differs from manifest".into());
    }
    bundle.recheck()?;
    let rendered = if output.is_some() {
        Some(render(
            &editor,
            &bundle.scene,
            first.graph_revision,
            options.size,
        )?)
    } else {
        None
    };
    bundle.recheck()?;
    if options.restart {
        if editor.stop().is_none() {
            return Err("host stop failed".into());
        }
        if !editor.restart() {
            return Err("host restart failed".into());
        }
        editor.sync();
        if editor.checksum() != source.initial_checksum() {
            return Err("restarted host initial checksum differs".into());
        }
        let restarted = advance(&mut editor, options.ticks)?;
        if restarted != first
            || mesh_from_view(
                editor
                    .snapshot()
                    .ok_or("missing restarted snapshot")?
                    .predicted(),
            )?
            .agent
                != first_agent
        {
            return Err("restarted trajectory differs".into());
        }
        bundle.recheck()?;
        let mut fresh = open(&source)?;
        let repeated = advance(&mut fresh, options.ticks)?;
        if repeated != first
            || mesh_from_view(
                fresh
                    .snapshot()
                    .ok_or("missing fresh snapshot")?
                    .predicted(),
            )?
            .agent
                != first_agent
        {
            return Err("fresh host trajectory differs".into());
        }
        if let Some((pixels, _, _)) = &rendered {
            let (again, _, _) = render(&fresh, &bundle.scene, first.graph_revision, options.size)?;
            if &again != pixels {
                return Err("fresh host rendered pixels differ".into());
            }
        }
        bundle.recheck()?;
    }
    let capture = if let (Some(target), Some((pixels, adapter, applied_camera))) =
        (output, rendered)
    {
        bundle.recheck()?;
        super::io::recheck_directory(&target.parent, &target.directory)?;
        let mut png_bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_bytes, options.size.0, options.size.1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
            writer
                .write_image_data(&pixels)
                .map_err(|e| e.to_string())?;
            writer.finish().map_err(|e| e.to_string())?;
        }
        let mut temporary =
            tempfile::NamedTempFile::new_in(&target.parent).map_err(|e| e.to_string())?;
        temporary.write_all(&png_bytes).map_err(|e| e.to_string())?;
        temporary.as_file().sync_all().map_err(|e| e.to_string())?;
        super::io::recheck_directory(&target.parent, &target.directory)?;
        bundle.recheck()?;
        source.recheck()?;
        let published_file=temporary.persist_noclobber(&target.path).map_err(|e|format!("capture publication failed; publication may be uncertain, do not retry blindly: {e}"))?;
        let check_published_identity = || -> Result<(), String> {
            let retained = published_file.metadata().map_err(|e| e.to_string())?;
            let named = rustix::fs::statat(
                &target.directory,
                target.path.file_name().ok_or("capture filename missing")?,
                rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
            )
            .map_err(|e| e.to_string())?;
            if rustix::fs::FileType::from_raw_mode(named.st_mode)
                != rustix::fs::FileType::RegularFile
                || named.st_dev != retained.dev()
                || named.st_ino != retained.ino()
                || named.st_mode != retained.mode()
                || named.st_size as u64 != retained.len()
            {
                return Err("published capture file identity changed".into());
            }
            Ok(())
        };
        let after = || -> Result<(), String> {
            check_published_identity()?;
            super::io::recheck_directory(&target.parent, &target.directory)?;
            target.directory.sync_all().map_err(|e| e.to_string())?;
            let published = super::io::Snapshot::read(&target.path, 8 * 1024 * 1024)?;
            if published.bytes != png_bytes {
                return Err("published capture bytes differ".into());
            }
            published.recheck()?;
            check_published_identity()?;
            bundle.recheck()?;
            source.recheck()?;
            super::io::recheck_directory(&target.parent, &target.directory)
        };
        after().map_err(|e| {
            format!(
                "capture published at {}; durability or identity uncertain: {e}; do not retry",
                target.path.display()
            )
        })?;
        let sidecar = |suffix: &str, default: &str| {
            let name = format!("{}{}", bundle.manifest.scene, suffix);
            match bundle.manifest.files.iter().find(|f| f.name == name) {
                Some(f) => serde_json::json!({"source":"saved_sidecar","sha256":f.sha256}),
                None => serde_json::json!({"source":"default","policy":default}),
            }
        };
        Some(
            serde_json::json!({"path":target.path,"width":options.size.0,"height":options.size.1,
            "rgba_sha256":super::manifest::hash(&pixels),"png_sha256":super::manifest::hash(&png_bytes),"adapter":adapter,
            "camera":{"provenance":sidecar(".mesh-camera.json","Editor::camera3d"),"applied":applied_camera},
            "lighting":sidecar(".mesh-lighting.json","Lighting::default"),
            "post_process":sidecar(".mesh-lighting.json","PostProcessSettings::default")}),
        )
    } else {
        bundle.recheck()?;
        source.recheck()?;
        None
    };
    println!(
        "{}",
        serde_json::json!({"trajectory":first,"manifest_sha256":bundle.manifest_sha256,"restart_checked":options.restart,"capture":capture})
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(s: &[&str]) -> Vec<OsString> {
        s.iter().map(OsString::from).collect()
    }
    #[test]
    fn cli_rejects_ambiguous_or_unbounded_requests() {
        for a in [
            vec![],
            vec!["--capabilities", "--bundle", "x"],
            vec!["--bundle", "x", "--ticks", "6001"],
            vec!["--bundle", "x", "--ticks", "0", "--ticks", "1"],
            vec!["--bundle", "x", "--size", "2048x2048"],
            vec!["--bundle", "x", "--capture", "x.png", "--no-capture"],
            vec!["--bundle", "x", "--size", "63x64"],
        ] {
            assert!(parse(args(&a)).is_err(), "{a:?}");
        }
    }
    #[test]
    fn cli_accepts_explicit_headless_modes() {
        assert!(matches!(
            parse(args(&["--capabilities"])).unwrap(),
            Request::Capabilities
        ));
        assert!(matches!(
            parse(args(&[
                "--bundle",
                "x",
                "--ticks",
                "6000",
                "--size",
                "64x64",
                "--restart-check",
                "--no-capture"
            ]))
            .unwrap(),
            Request::Run(_)
        ));
    }
    #[test]
    fn arguments_are_bounded_before_launch() {
        assert!(parse(std::iter::repeat_n(OsString::from("--no-capture"), 17)).is_err());
        assert!(parse([OsString::from("--bundle"), OsString::from("x".repeat(4097))]).is_err());
        assert!(matches!(parse(args(&["--help"])).unwrap(), Request::Help));
        assert!(parse(args(&["--help", "--bundle", "x"])).is_err());
    }
    #[test]
    fn capture_rejects_bundle_existing_and_symlink_ancestry() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("bundle");
        std::fs::create_dir(&root).unwrap();
        assert!(capture_path(&root.join("inside.png"), &root).is_err());
        let present = dir.path().join("present.png");
        std::fs::write(&present, b"keep").unwrap();
        assert!(capture_path(&present, &root).is_err());
        let link = dir.path().join("link");
        symlink(dir.path(), &link).unwrap();
        assert!(capture_path(&link.join("outside.png"), &root).is_err());
        let output = dir.path().join("out");
        std::fs::create_dir(&output).unwrap();
        let target = capture_path(&output.join("new.png"), &root).unwrap();
        std::fs::rename(&output, dir.path().join("old")).unwrap();
        std::fs::create_dir(&output).unwrap();
        assert!(super::super::io::recheck_directory(&target.parent, &target.directory).is_err());
    }
}
