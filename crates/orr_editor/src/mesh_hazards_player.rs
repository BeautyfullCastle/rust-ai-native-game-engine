use crate::{
    app::{EditorApp, ScreenshotJob},
    editor::Editor,
    game::EditorGame,
};
use egui::Ui;
use orr_remote::mesh_navigation::mesh_from_view;
use sha2::{Digest, Sha256};
use std::{
    env,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_PLAYER_EXE_BYTES: u64 = 256 << 20;
const MAX_STARTUP_TICKS: u32 = 10_000;
const MAX_CAPTURE_PIXELS: u64 = 1_048_576;
const USAGE: &str = "orr_mesh_hazards_player [--bundle DIR] [--ticks N] [--screenshot PNG [--frames N]] [--size WxH]\n       orr_mesh_hazards_player [--bundle DIR] --verify-headless --ticks N\n       orr_mesh_hazards_player --capabilities\n\nWithout --bundle, the validated bundle beside this executable is used.\n--ticks N starts Play and leaves the admitted simulation paused at tick N.\n--screenshot writes a bounded window PNG; --frames defaults to 30.\n--size sets the initial window dimensions in points (default 960x600).";

const CAPABILITIES: &str = "ORR_MESH_HAZARDS_PLAYER_V1\nORR_MESH_INPUT_V1\nORR_MESH_HAZARDS_CAMERA_V1\nORR_MESH_HAZARDS_ART_V1\nORR_MESH_HAZARDS_LIGHTING_V1\nORR_MESH_HAZARDS_HUD_V1\nORR_MESH_HAZARDS_PROPS_V1";

enum Request {
    Help,
    Capabilities,
    Launch(Options),
}

struct Options {
    verify_headless: bool,
    bundle: Option<PathBuf>,
    ticks: Option<u32>,
    screenshot: Option<PathBuf>,
    frames: u64,
    size: (u32, u32),
}

impl Options {
    fn parse(arguments: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Request, String> {
        let mut arguments = arguments.into_iter();
        let mut options = Self {
            verify_headless: false,
            bundle: None,
            ticks: None,
            screenshot: None,
            frames: 30,
            size: (960, 600),
        };
        let mut frames_was_set = false;
        let mut size_was_set = false;
        let mut capabilities = false;
        let mut help = false;
        while let Some(argument) = arguments.next() {
            let flag = argument
                .to_str()
                .ok_or_else(|| format!("arguments must be valid UTF-8\n\n{USAGE}"))?;
            match flag {
                "--verify-headless" => {
                    if options.verify_headless {
                        return Err("--verify-headless may only be supplied once".into());
                    }
                    options.verify_headless = true;
                }
                "--bundle" => {
                    if options.bundle.is_some() {
                        return Err("--bundle may only be supplied once".into());
                    }
                    let path = arguments
                        .next()
                        .ok_or_else(|| format!("--bundle needs a directory\n\n{USAGE}"))?;
                    if path.is_empty() {
                        return Err("--bundle needs a nonempty directory".into());
                    }
                    options.bundle = Some(PathBuf::from(path));
                }
                "--ticks" => {
                    if options.ticks.is_some() {
                        return Err("--ticks may only be supplied once".into());
                    }
                    let value = next_text(&mut arguments, "--ticks")?;
                    let ticks = value
                        .parse::<u32>()
                        .map_err(|_| "--ticks needs a nonnegative whole number".to_owned())?;
                    if ticks > MAX_STARTUP_TICKS {
                        return Err(format!(
                            "--ticks exceeds the {MAX_STARTUP_TICKS} tick limit"
                        ));
                    }
                    options.ticks = Some(ticks);
                }
                "--screenshot" => {
                    if options.screenshot.is_some() {
                        return Err("--screenshot may only be supplied once".into());
                    }
                    let path = arguments
                        .next()
                        .ok_or_else(|| format!("--screenshot needs a file path\n\n{USAGE}"))?;
                    if path.is_empty() {
                        return Err("--screenshot needs a nonempty file path".into());
                    }
                    options.screenshot = Some(PathBuf::from(path));
                }
                "--frames" => {
                    if frames_was_set {
                        return Err("--frames may only be supplied once".into());
                    }
                    let value = next_text(&mut arguments, "--frames")?;
                    options.frames = value
                        .parse::<u64>()
                        .map_err(|_| "--frames needs a positive whole number".to_owned())?;
                    if !(1..=10_000).contains(&options.frames) {
                        return Err("--frames must be between 1 and 10000".into());
                    }
                    frames_was_set = true;
                }
                "--size" => {
                    if size_was_set {
                        return Err("--size may only be supplied once".into());
                    }
                    let value = next_text(&mut arguments, "--size")?;
                    options.size = parse_size(&value)?;
                    size_was_set = true;
                }
                "--capabilities" => {
                    if capabilities {
                        return Err("--capabilities may only be supplied once".into());
                    }
                    capabilities = true;
                }
                "--help" | "-h" => help = true,
                unknown => return Err(format!("unknown argument '{unknown}'\n\n{USAGE}")),
            }
        }
        if capabilities {
            if options.verify_headless
                || options.bundle.is_some()
                || options.ticks.is_some()
                || options.screenshot.is_some()
                || frames_was_set
                || size_was_set
                || help
            {
                return Err("--capabilities cannot be combined with other options".into());
            }
            return Ok(Request::Capabilities);
        }
        if help {
            if options.verify_headless
                || options.bundle.is_some()
                || options.ticks.is_some()
                || options.screenshot.is_some()
                || frames_was_set
                || size_was_set
            {
                return Err("--help cannot be combined with other options".into());
            }
            return Ok(Request::Help);
        }
        if options.verify_headless
            && (options.ticks.is_none()
                || options.screenshot.is_some()
                || frames_was_set
                || size_was_set)
        {
            return Err("--verify-headless requires --ticks and cannot combine with screenshot, frames or size".into());
        }
        if frames_was_set && options.screenshot.is_none() {
            return Err("--frames requires --screenshot".into());
        }
        Ok(Request::Launch(options))
    }
}

fn next_text(
    arguments: &mut impl Iterator<Item = std::ffi::OsString>,
    flag: &str,
) -> Result<String, String> {
    arguments
        .next()
        .ok_or_else(|| format!("{flag} needs a value\n\n{USAGE}"))?
        .into_string()
        .map_err(|_| format!("{flag} value must be valid UTF-8"))
}

fn parse_size(value: &str) -> Result<(u32, u32), String> {
    let (width, height) = value
        .split_once('x')
        .ok_or_else(|| "--size needs WxH, for example 960x600".to_owned())?;
    let width = width
        .parse::<u32>()
        .map_err(|_| "--size width must be a whole number".to_owned())?;
    let height = height
        .parse::<u32>()
        .map_err(|_| "--size height must be a whole number".to_owned())?;
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| "--size dimensions overflow".to_owned())?;
    if !(320..=2048).contains(&width)
        || !(240..=2048).contains(&height)
        || pixels > MAX_CAPTURE_PIXELS
    {
        return Err(format!(
            "--size must be 320..=2048 by 240..=2048 and at most {MAX_CAPTURE_PIXELS} pixels"
        ));
    }
    Ok((width, height))
}

fn sha256_file_bounded(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("open runtime executable: {error}"))?;
    let initial = file
        .metadata()
        .map_err(|error| format!("inspect runtime executable: {error}"))?;
    if !initial.is_file() || initial.len() == 0 || initial.len() > MAX_PLAYER_EXE_BYTES {
        return Err("runtime executable has an invalid or over-limit size".into());
    }
    let mut remaining = initial.len();
    let mut buffer = [0_u8; 64 * 1024];
    let mut digest = Sha256::new();
    while remaining > 0 {
        let limit = buffer.len().min(remaining as usize);
        let count = file
            .read(&mut buffer[..limit])
            .map_err(|error| format!("read runtime executable: {error}"))?;
        if count == 0 {
            return Err("runtime executable changed while hashing".into());
        }
        digest.update(&buffer[..count]);
        remaining -= count as u64;
    }
    let mut trailing = [0_u8; 1];
    if file
        .read(&mut trailing)
        .map_err(|error| format!("check runtime executable size: {error}"))?
        != 0
        || file
            .metadata()
            .map_err(|error| format!("recheck runtime executable: {error}"))?
            .len()
            != initial.len()
    {
        return Err("runtime executable changed while hashing".into());
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn screenshot_path(path: &Path, bundle_root: &Path) -> Result<PathBuf, String> {
    let requested = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .map_err(|error| format!("resolve screenshot path: {error}"))?
            .join(path)
    };
    let name = requested
        .file_name()
        .ok_or_else(|| "screenshot path must name a file".to_owned())?;
    let parent = requested
        .parent()
        .ok_or_else(|| "screenshot path needs an existing parent directory".to_owned())?
        .canonicalize()
        .map_err(|error| format!("resolve screenshot parent: {error}"))?;
    let output = parent.join(name);
    let bundle_root = bundle_root
        .canonicalize()
        .map_err(|error| format!("resolve bundle directory: {error}"))?;
    if output.starts_with(&bundle_root) {
        return Err("screenshot output must be outside the read-only game bundle".into());
    }
    match std::fs::symlink_metadata(&output) {
        Ok(_) => return Err("screenshot output already exists".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("inspect screenshot output: {error}")),
    }
    Ok(output)
}

struct PlayerApp {
    inner: EditorApp,
    session: crate::app::mesh_hazards_player::State,
}

impl eframe::App for PlayerApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.inner.mesh_hazards_player_ui(ui, &mut self.session);
    }
}

pub fn run() -> Result<(), String> {
    match Options::parse(env::args_os().skip(1))? {
        Request::Help => {
            println!("{USAGE}");
            Ok(())
        }
        Request::Capabilities => {
            println!("{CAPABILITIES}");
            #[cfg(feature = "mesh-animated-models")]
            println!("ORR_MESH_HAZARDS_ANIMATION_V1");
            Ok(())
        }
        Request::Launch(options) => launch(options),
    }
}

#[cfg(test)]
#[path = "mesh_hazards_player/tests.rs"]
mod tests;

fn launch(options: Options) -> Result<(), String> {
    let executable =
        env::current_exe().map_err(|error| format!("locate player executable: {error}"))?;
    let bundle_root = match options.bundle {
        Some(root) => root,
        None => executable
            .parent()
            .ok_or_else(|| "player executable has no parent directory".to_owned())?
            .to_path_buf(),
    };
    let bundle = crate::mesh_hazards_export::open_bundle(&bundle_root)?;
    verify_executable(&executable, &bundle)?;
    let screenshot = options
        .screenshot
        .as_deref()
        .map(|path| screenshot_path(path, &bundle_root))
        .transpose()?;

    let mut editor = Editor::open_game(&bundle.scene, EditorGame::MeshHazardsV1)?;
    editor.sync();
    let snapshot = editor
        .snapshot()
        .ok_or_else(|| "mesh host has not published an admitted frame".to_owned())?;
    let view = mesh_from_view(snapshot.predicted())
        .map_err(|error| format!("runtime scene admission failed: {error}"))?;
    if editor.checksum() != bundle.checksum || view.graph.revision() != bundle.graph_revision {
        return Err("runtime frame differs from the admitted game bundle".into());
    }
    let mut panel = crate::mesh_navigation::Panel::default();
    panel.sync(&editor);
    let mut camera_panel = crate::mesh_camera_panel::Panel::default();
    camera_panel.sync(&mut editor);
    verify_loaded_camera_bundle(&bundle_root, &bundle, &camera_panel)?;
    verify_loaded_art_bundle(&bundle_root, &bundle, &panel)?;
    verify_loaded_props_bundle(&bundle_root, &bundle, &panel, &editor)?;
    let mut lighting_panel = crate::mesh_lighting_panel::Panel::default();
    lighting_panel.sync(&editor);
    verify_loaded_lighting_bundle(&bundle_root, &bundle, &lighting_panel)?;
    let mut hud_panel = crate::mesh_hud_panel::Panel::default();
    hud_panel.sync(&editor);
    verify_loaded_hud_bundle(&bundle_root, &bundle, &hud_panel)?;
    #[cfg(feature = "mesh-animated-models")]
    let mut animation_panel = crate::mesh_animation_panel::Panel::default();
    #[cfg(feature = "mesh-animated-models")]
    animation_panel.sync(&editor);
    #[cfg(feature = "mesh-animated-models")]
    let animation_blocker = animation_panel.play_blocker(&editor);
    #[cfg(feature = "mesh-animated-models")]
    verify_loaded_animation_bundle(&bundle_root, &bundle, &animation_panel)?;
    #[cfg(feature = "mesh-animated-models")]
    let initial_animation_pose = animation_panel.initial_play_placement(&editor)?
        .as_ref().map(animation_pose_digest);
    #[cfg(not(feature = "mesh-animated-models"))]
    let animation_blocker = match crate::mesh_animation_store::has_sidecar(&bundle.scene) {
        Ok(false) => None,
        Ok(true) => Some(
            "Mesh animation sidecar is present; rebuild with mesh-animated-models support".into(),
        ),
        Err(error) => Some(error),
    };
    editor.set_mesh_navigation_blocker(
        panel
            .play_blocker(&editor)
            .or_else(|| camera_panel.play_blocker(&editor))
            .or_else(|| lighting_panel.play_blocker(&editor))
            .or_else(|| hud_panel.play_blocker(&editor))
            .or(animation_blocker),
    );
    start_playback(&mut editor, options.ticks, screenshot.is_some())?;
    if options.verify_headless {
        return verify_headless_session(
            editor,
            panel,
            camera_panel,
            lighting_panel,
            hud_panel,
            #[cfg(feature = "mesh-animated-models")]
            (animation_panel, initial_animation_pose),
            &bundle,
        );
    }

    let (width, height) = options.size;
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([width as f32, height as f32])
            .with_min_inner_size([320.0, 240.0]),
        ..Default::default()
    };
    let frames = options.frames;
    let mesh_input_allowed = true;
    eframe::run_native(
        "Orrery Mesh Hazards Player",
        native_options,
        Box::new(move |creation_context| {
            let mut inner = EditorApp::new(editor, creation_context.wgpu_render_state.clone());
            inner.install_validated_hazards_camera(camera_panel);
            inner.install_validated_hazards_navigation(panel);
            inner.install_validated_hazards_lighting(lighting_panel);
            inner.install_validated_hazards_hud(hud_panel);
            #[cfg(feature = "mesh-animated-models")]
            inner.install_validated_hazards_animation(animation_panel);
            inner.set_mesh_player_input_allowed(mesh_input_allowed);
            if let Some(path) = screenshot {
                inner = inner.with_screenshot(ScreenshotJob::new(path, frames).with_settle());
            }
            Ok(Box::new(PlayerApp {
                inner,
                session: Default::default(),
            }))
        }),
    )
    .map_err(|error| format!("Mesh Hazards Player window: {error}"))
}

fn verify_loaded_camera_bundle(
    bundle_root: &Path,
    bundle: &crate::mesh_hazards_export::PreparedBundle,
    camera_panel: &crate::mesh_camera_panel::Panel,
) -> Result<(), String> {
    if !camera_panel.matches_saved_sidecar(bundle.camera_sha256.as_deref()) {
        return Err("loaded game camera sidecar differs from the validated bundle".into());
    }
    // Re-read and validate the manifest + bounded file inventory after the
    // camera has been installed. This detects ordinary edits between initial
    // admission and launch; it does not claim immunity to an adversarial writer.
    let current = crate::mesh_hazards_export::open_bundle(bundle_root).map_err(|error| {
        format!("game bundle changed while the authored camera was loading: {error}")
    })?;
    if current != *bundle {
        return Err("game bundle changed while the authored camera was loading".into());
    }
    Ok(())
}

fn verify_loaded_art_bundle(
    bundle_root: &Path,
    bundle: &crate::mesh_hazards_export::PreparedBundle,
    panel: &crate::mesh_navigation::Panel,
) -> Result<(), String> {
    if !panel.art_matches_saved_sidecar(bundle.art_sha256.as_deref()) {
        return Err("loaded game agent art differs from the validated bundle".into());
    }
    let current = crate::mesh_hazards_export::open_bundle(bundle_root)
        .map_err(|error| format!("game bundle changed while agent art was loading: {error}"))?;
    if current != *bundle {
        return Err("game bundle changed while agent art was loading".into());
    }
    Ok(())
}

fn verify_loaded_props_bundle(
    bundle_root: &Path,
    bundle: &crate::mesh_hazards_export::PreparedBundle,
    panel: &crate::mesh_navigation::Panel,
    editor: &Editor,
) -> Result<(), String> {
    if !panel.props_matches_saved_sidecar(editor, bundle.props_sha256.as_deref()) {
        return Err("loaded game props differ from the validated bundle".into());
    }
    let current = crate::mesh_hazards_export::open_bundle(bundle_root)
        .map_err(|error| format!("game bundle changed while props were loading: {error}"))?;
    if current != *bundle {
        return Err("game bundle changed while props were loading".into());
    }
    Ok(())
}

fn verify_loaded_lighting_bundle(
    bundle_root: &Path,
    bundle: &crate::mesh_hazards_export::PreparedBundle,
    panel: &crate::mesh_lighting_panel::Panel,
) -> Result<(), String> {
    if !panel.matches_saved_sidecar(bundle.lighting_sha256.as_deref()) {
        return Err("loaded game lighting differs from the validated bundle".into());
    }
    let current = crate::mesh_hazards_export::open_bundle(bundle_root)
        .map_err(|error| format!("game bundle changed while lighting was loading: {error}"))?;
    if current != *bundle {
        return Err("game bundle changed while lighting was loading".into());
    }
    Ok(())
}

fn verify_loaded_hud_bundle(
    bundle_root: &Path,
    bundle: &crate::mesh_hazards_export::PreparedBundle,
    panel: &crate::mesh_hud_panel::Panel,
) -> Result<(), String> {
    if !panel.matches_saved_sidecar(bundle.hud_sha256.as_deref()) {
        return Err("loaded game HUD differs from the validated bundle".into());
    }
    let current = crate::mesh_hazards_export::open_bundle(bundle_root)
        .map_err(|error| format!("game bundle changed while HUD was loading: {error}"))?;
    if current != *bundle {
        return Err("game bundle changed while HUD was loading".into());
    }
    Ok(())
}

#[cfg(feature = "mesh-animated-models")]
fn verify_loaded_animation_bundle(
    root: &Path,
    bundle: &crate::mesh_hazards_export::PreparedBundle,
    panel: &crate::mesh_animation_panel::Panel,
) -> Result<(), String> {
    if !panel.matches_saved_sidecar(bundle.animation_sha256.as_deref()) {
        return Err("loaded game animation differs from the validated bundle".into());
    }
    if crate::mesh_hazards_export::open_bundle(root)? != *bundle {
        return Err("game bundle changed while animation was loading".into());
    }
    Ok(())
}

#[cfg(feature = "mesh-animated-models")]
pub(crate) fn animation_pose_digest(placement: &crate::viewport3d::AnimatedPlacement) -> String {
    let mut digest = Sha256::new();
    // Local pose excludes world movement: a changing digest proves skeletal
    // animation, not merely translation of the agent's instance.
    for matrix in placement.pose.global() {
        for row in matrix {
            for value in row { digest.update(value.to_bits().to_le_bytes()); }
        }
    }
    digest.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
}

fn verify_headless_session(
    editor: Editor,
    navigation: crate::mesh_navigation::Panel,
    camera: crate::mesh_camera_panel::Panel,
    lighting: crate::mesh_lighting_panel::Panel,
    hud: crate::mesh_hud_panel::Panel,
    #[cfg(feature = "mesh-animated-models")] animation_state: (crate::mesh_animation_panel::Panel, Option<String>),
    bundle: &crate::mesh_hazards_export::PreparedBundle,
) -> Result<(), String> {
    #[cfg(feature = "mesh-animated-models")]
    let (mut animation, initial_animation_pose) = animation_state;
    let tick = editor.sim().head_tick;
    let checksum = editor.checksum();
    let snapshot = editor.snapshot().ok_or("verification has no frame")?;
    let view = mesh_from_view(snapshot.predicted())?;
    let position = view.agent.position.map(|value| value.raw());
    if bundle.art_sha256.is_some() && navigation.art_model(&editor).is_none() {
        return Err("validated agent art has no playback placement".into());
    }
    let expected_props = navigation.props_document_bytes(&editor)?;
    let prop_count = navigation.prop_models(&editor).len();
    let expected_lighting = lighting.applied_lighting(&editor);
    let expected_post_process = lighting.applied_post_process(&editor);
    if bundle.lighting_sha256.is_some() && expected_lighting.is_none() {
        return Err("validated game lighting is not applied during playback".into());
    }
    let expected_hud = hud.applied_document(&editor);
    if bundle.hud_sha256.is_some() && expected_hud.is_none() {
        return Err("validated game HUD is not applied during playback".into());
    }
    #[cfg(feature = "mesh-animated-models")]
    let played_animation_pose = animation.placement(&editor)?.as_ref().map(animation_pose_digest);
    #[cfg(feature = "mesh-animated-models")]
    if played_animation_pose.is_some() != bundle.animation_sha256.is_some() {
        return Err("validated animation has no playback placement".into());
    }
    let mut app = EditorApp::new(editor, None);
    app.install_validated_hazards_navigation(navigation);
    app.install_validated_hazards_camera(camera);
    app.install_validated_hazards_lighting(lighting);
    app.install_validated_hazards_hud(hud);
    #[cfg(feature = "mesh-animated-models")]
    app.install_validated_hazards_animation(animation);
    app.verify_hazards_restart()?;
    if !app.hazards_props_match(&bundle.props_sha256, &expected_props) {
        return Err("headless restart changed the admitted game props".into());
    }
    if !app.hazards_lighting_matches(
        &bundle.lighting_sha256,
        expected_lighting,
        expected_post_process,
    ) {
        return Err("headless restart changed the admitted game lighting".into());
    }
    if !app.hazards_hud_matches(&bundle.hud_sha256, &expected_hud) {
        return Err("headless restart changed the admitted game HUD".into());
    }
    #[cfg(feature = "mesh-animated-models")]
    let restarted_animation_pose = app.hazards_animation_pose_digest()?;
    #[cfg(feature = "mesh-animated-models")]
    if !app.hazards_animation_matches(bundle.animation_sha256.as_deref())
        || restarted_animation_pose != initial_animation_pose {
        return Err("headless restart changed the admitted animation pose".into());
    }
    if app.editor.checksum() != bundle.checksum {
        return Err("headless restart differs from the admitted initial frame".into());
    }
    #[allow(unused_mut)]
    let mut report = serde_json::json!({
            "tick": tick, "checksum": format!("{checksum:016x}"),
            "agent_position_raw": position, "art_sha256": bundle.art_sha256,
            "lighting_sha256": bundle.lighting_sha256, "lighting_restart_parity": true,
            "hud_sha256": bundle.hud_sha256, "hud_restart_parity": true,
            "props_sha256": bundle.props_sha256, "props_restart_parity": true,
            "prop_count": prop_count,
            "restarted_tick": app.editor.sim().head_tick,
            "restarted_checksum": format!("{:016x}", app.editor.checksum()),
            "restart_parity": true
        });
    #[cfg(feature = "mesh-animated-models")]
    {
        report["animation_sha256"] = serde_json::json!(bundle.animation_sha256);
        report["animation_pose_sha256"] = serde_json::json!(played_animation_pose);
        report["animation_initial_pose_sha256"] = serde_json::json!(initial_animation_pose);
        report["animation_restarted_pose_sha256"] = serde_json::json!(restarted_animation_pose);
        report["animation_restart_parity"] = serde_json::json!(true);
    }
    println!("{report}");
    Ok(())
}

fn start_playback(editor: &mut Editor, ticks: Option<u32>, capturing: bool) -> Result<(), String> {
    if !editor.start_play() {
        return Err(editor
            .status()
            .map(|message| message.text.clone())
            .unwrap_or_else(|| "mesh scene could not start".into()));
    }
    editor.set_mesh_play_input_enabled(false);
    editor.sync();
    crate::app::mesh_hazards_player::validate_fresh_session(editor)?;
    match ticks {
        Some(count) if count > 0 => editor.step(count),
        None if !capturing => editor.play(),
        _ => {}
    }
    editor.sync();
    if let Some(message) = editor.status().filter(|message| message.error) {
        return Err(message.text.clone());
    }
    let timeline = editor
        .timeline()
        .ok_or("Mesh Hazards Player did not enter Play")?;
    match ticks {
        Some(count) if timeline.tick != u64::from(count) || timeline.playing => {
            Err("tick-limited session did not remain paused at the requested tick".into())
        }
        None if capturing && (timeline.tick != 0 || timeline.playing) => {
            Err("screenshot session did not remain paused at tick zero".into())
        }
        None if !capturing && !timeline.playing => {
            Err("Mesh Hazards Player could not start the admitted scene".into())
        }
        _ => Ok(()),
    }
}

fn verify_executable(
    executable: &Path,
    bundle: &crate::mesh_hazards_export::PreparedBundle,
) -> Result<(), String> {
    if !bundle.platform.is_current() {
        return Err("bundle runtime platform does not match this player process".into());
    }
    // open_bundle has validated the runtime's container, capability section and
    // digest. Compare actual current_exe bytes, never execute the supplied path.
    if sha256_file_bounded(executable)? != bundle.runtime_sha256 {
        return Err("running player executable hash does not match the admitted bundle".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "mesh_hazards_player/acceptance.rs"]
mod acceptance;

#[cfg(test)]
#[path = "mesh_hazards_player/art_acceptance.rs"]
mod art_acceptance;

#[cfg(test)]
#[path = "mesh_hazards_player/lighting_acceptance.rs"]
mod lighting_acceptance;

#[cfg(test)]
#[path = "mesh_hazards_player/hud_acceptance.rs"]
mod hud_acceptance;

#[cfg(test)]
#[path = "mesh_hazards_player/props_acceptance.rs"]
mod props_acceptance;

#[cfg(all(test, feature = "mesh-animated-models"))]
#[path = "mesh_hazards_player/animation_acceptance.rs"]
mod animation_acceptance;
