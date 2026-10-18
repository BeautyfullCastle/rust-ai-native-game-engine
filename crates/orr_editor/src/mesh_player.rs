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
const USAGE: &str = "orr_mesh_player [--bundle DIR] [--ticks N] [--screenshot PNG [--frames N]] [--size WxH]\n       orr_mesh_player --capabilities\n\nWithout --bundle, the validated bundle beside this executable is used.\n--ticks N starts Play and leaves the admitted simulation paused at tick N.\n--screenshot writes a bounded window PNG; --frames defaults to 30.\n--size sets the initial window dimensions in points (default 960x600).";

const CAPABILITIES: &str = "ORR_MESH_PLAYER_V1\nORR_MESH_INPUT_V1";

enum Request {
    Help,
    Capabilities,
    Launch(Options),
}

struct Options {
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
            if options.bundle.is_some()
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
            if options.bundle.is_some()
                || options.ticks.is_some()
                || options.screenshot.is_some()
                || frames_was_set
                || size_was_set
            {
                return Err("--help cannot be combined with other options".into());
            }
            return Ok(Request::Help);
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
}

impl eframe::App for PlayerApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.inner.mesh_player_ui(ui);
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
            Ok(())
        }
        Request::Launch(options) => launch(options),
    }
}

#[cfg(test)]
#[path = "mesh_player/tests.rs"]
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
    let bundle = crate::mesh_export::open_bundle(&bundle_root)?;
    let runtime_sha256 = sha256_file_bounded(&executable)?;
    if runtime_sha256 != bundle.runtime_sha256 {
        return Err("running player executable hash does not match the admitted bundle".into());
    }
    let screenshot = options
        .screenshot
        .as_deref()
        .map(|path| screenshot_path(path, &bundle_root))
        .transpose()?;

    let mut editor = Editor::open_game(&bundle.scene, EditorGame::MeshNavigation)?;
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
    let mut lighting_panel = crate::mesh_lighting_panel::Panel::default();
    lighting_panel.sync(&editor);
    #[cfg(feature = "mesh-animated-models")]
    let mut animation_panel = crate::mesh_animation_panel::Panel::default();
    #[cfg(feature = "mesh-animated-models")]
    animation_panel.sync(&editor);
    #[cfg(feature = "mesh-animated-models")]
    let animation_blocker = animation_panel.play_blocker(&editor);
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
            .or(animation_blocker),
    );
    start_playback(&mut editor, options.ticks, screenshot.is_some())?;

    let (width, height) = options.size;
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([width as f32, height as f32])
            .with_min_inner_size([320.0, 240.0]),
        ..Default::default()
    };
    let frames = options.frames;
    let mesh_input_allowed =
        bundle.input_profile == Some(crate::mesh_export::InputProfile::MeshClickToMoveV1);
    eframe::run_native(
        "Orrery Mesh Player",
        native_options,
        Box::new(move |creation_context| {
            let mut inner = EditorApp::new(editor, creation_context.wgpu_render_state.clone());
            inner.set_mesh_player_input_allowed(mesh_input_allowed);
            if let Some(path) = screenshot {
                inner = inner.with_screenshot(ScreenshotJob::new(path, frames).with_settle());
            }
            Ok(Box::new(PlayerApp { inner }))
        }),
    )
    .map_err(|error| format!("Mesh Player window: {error}"))
}

fn start_playback(editor: &mut Editor, ticks: Option<u32>, capturing: bool) -> Result<(), String> {
    if !editor.start_play() {
        return Err(editor
            .status()
            .map(|message| message.text.clone())
            .unwrap_or_else(|| "mesh scene could not start".into()));
    }
    match ticks {
        Some(count) if count > 0 => editor.step(count),
        None if !capturing => editor.play(),
        _ => {}
    }
    editor.sync();
    if let Some(message) = editor.status().filter(|message| message.error) {
        return Err(message.text.clone());
    }
    let timeline = editor.timeline().ok_or("Mesh Player did not enter Play")?;
    match ticks {
        Some(count) if timeline.tick != u64::from(count) || timeline.playing => {
            Err("tick-limited session did not remain paused at the requested tick".into())
        }
        None if capturing && (timeline.tick != 0 || timeline.playing) => {
            Err("screenshot session did not remain paused at tick zero".into())
        }
        None if !capturing && !timeline.playing => {
            Err("Mesh Player could not start the admitted scene".into())
        }
        _ => Ok(()),
    }
}
