#![allow(
    clippy::disallowed_types,
    reason = "Native acceptance uses bounded host-side deadlines, not simulation time"
)]

#[path = "../../static-game-starter/tests/support/standalone_x11.rs"]
#[allow(
    dead_code,
    reason = "Native acceptance reuses the starter's X11 helper without modifying it"
)]
mod x11;

use orr_collect_delivery_game::game::{
    CollectDeliveryGame, CollectDeliveryState, MODE_NAVIGATION_PULSE, MODE_PRIMARY, MODE_RELEASED,
};
use orr_editor::{Editor, registered_project::PreparedRegisteredProject};
use orr_session::ReplayReader;
use orr_sim::PlayerSlot;
use orr_static_starter::game_contract::Starter;
use orr_static_starter::presentation::StandaloneGame;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::Duration,
};

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;
const SCHEMA: &str = "orr.collect-delivery-native/1";
const RIGHT: u32 = 0xff53;
const LEFT: u32 = 0xff51;
const UP: u32 = 0xff52;
const SPACE: u32 = 0x20;
const ENTER: u32 = 0xff0d;
const ESCAPE: u32 = 0xff1b;
const WAIT: Duration = Duration::from_secs(15);
const MAX_LOG: u64 = 8 << 20;
const MAX_OBSERVATIONS: u64 = 32 << 20;

fn error(message: impl Into<String>) -> Box<dyn Error> {
    Box::new(std::io::Error::other(message.into()))
}
fn require(ok: bool, message: impl Into<String>) -> Result {
    if ok { Ok(()) } else { Err(error(message)) }
}
fn write_json(path: &Path, value: &Value) -> Result {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}
fn hash(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut bytes = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        digest.update(&bytes[..count]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    require(
        metadata.is_file() && metadata.len() <= limit,
        "missing/linked/oversized output",
    )?;
    Ok(fs::read(path)?)
}
fn source_sha() -> Result<&'static str> {
    let sha = option_env!("ORR_STATIC_STANDALONE_BUILD_SHA")
        .ok_or_else(|| error("missing compile-bound source SHA"))?;
    require(
        sha.len() == 40
            && sha
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid compile-bound source SHA",
    )?;
    Ok(sha)
}

struct OwnedChild {
    child: Child,
    stdout: PathBuf,
    stderr: PathBuf,
}
impl OwnedChild {
    fn spawn(mut command: Command, directory: &Path, name: &str) -> Result<Self> {
        let stdout = directory.join(format!("{name}.stdout"));
        let stderr = directory.join(format!("{name}.stderr"));
        let stdout_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&stdout)?;
        let stderr_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&stderr)?;
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout_file))
            .stderr(Stdio::from(stderr_file))
            .spawn()?;
        Ok(Self {
            child,
            stdout,
            stderr,
        })
    }
    fn check_logs(&self) -> Result {
        require(
            fs::metadata(&self.stdout)?.len() <= MAX_LOG
                && fs::metadata(&self.stderr)?.len() <= MAX_LOG,
            "owned child output exceeded its bound",
        )
    }
    fn alive(&mut self) -> Result {
        self.check_logs()?;
        if let Some(status) = self.child.try_wait()? {
            let bytes = read_bounded(&self.stderr, MAX_LOG)?;
            let message = String::from_utf8_lossy(&bytes);
            return Err(error(format!(
                "owned child exited before its expected event ({status}): {message}"
            )));
        }
        Ok(())
    }
    fn wait(&mut self) -> Result<Vec<u8>> {
        let deadline = x11::Deadline::after(WAIT);
        loop {
            self.check_logs()?;
            if let Some(status) = self.child.try_wait()? {
                if !status.success() {
                    let bytes = read_bounded(&self.stderr, MAX_LOG)?;
                    let message = String::from_utf8_lossy(&bytes);
                    return Err(error(format!("owned child failed ({status}): {message}")));
                }
                return read_bounded(&self.stdout, MAX_LOG);
            }
            if deadline.expired() {
                let _ = self.child.kill();
                let _ = self.child.wait();
                return Err(error("owned child timed out"));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Default)]
struct Observations {
    path: PathBuf,
    values: Vec<Value>,
}
impl Observations {
    fn refresh(&mut self) -> Result {
        if !self.path.exists() {
            return Ok(());
        }
        let meta = fs::symlink_metadata(&self.path)?;
        require(
            meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= MAX_OBSERVATIONS,
            "unsafe/oversized observation stream",
        )?;
        let bytes = fs::read(&self.path)?;
        let end = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |n| n + 1);
        let parsed = bytes[..end]
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
            .map(serde_json::from_slice)
            .collect::<std::result::Result<Vec<Value>, _>>()?;
        require(
            parsed.len() >= self.values.len() && parsed[..self.values.len()] == self.values,
            "observation prefix was rewritten",
        )?;
        for pair in parsed.windows(2) {
            require(
                pair[1]["sequence"].as_u64()
                    == pair[0]["sequence"].as_u64().and_then(|n| n.checked_add(1))
                    && pair[1]["tick"].as_u64() >= pair[0]["tick"].as_u64(),
                "observation sequence/tick is not monotone",
            )?;
        }
        self.values = parsed;
        Ok(())
    }
    fn latest_sequence(&self) -> u64 {
        self.values
            .last()
            .and_then(|v| v["sequence"].as_u64())
            .unwrap_or(0)
    }
    fn latest(&self) -> Result<&Value> {
        self.values
            .last()
            .ok_or_else(|| error("runtime emitted no rendered observation"))
    }
    fn until(
        &mut self,
        process: &mut OwnedChild,
        after: u64,
        predicate: impl Fn(&Value) -> bool,
    ) -> Result<Value> {
        let deadline = x11::Deadline::after(WAIT);
        loop {
            process.alive()?;
            self.refresh()?;
            if let Some(value) = self
                .values
                .iter()
                .find(|v| v["sequence"].as_u64().is_some_and(|n| n > after) && predicate(v))
            {
                return Ok(value.clone());
            }
            if deadline.expired() {
                return Err(error("required presentation timed out"));
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
}

fn consumed(
    observation: &Value,
    dx: i32,
    dy: i32,
    action: u32,
    predicate: impl Fn(&Value) -> bool,
) -> bool {
    observation["consumed"].as_array().is_some_and(|entries| {
        entries.iter().any(|entry| {
            entry["input"] == json!({"dx":dx,"dy":dy,"action":action}) && predicate(&entry["state"])
        })
    })
}
fn release_key(
    x11: &x11::Observer,
    process: &mut OwnedChild,
    observations: &mut Observations,
    key: u32,
    after: u64,
    position: [i64; 2],
) -> Result<Value> {
    x11.key(key, false)?;
    observations.until(process, after, |observation| {
        consumed(observation, 0, 0, MODE_RELEASED, |state| {
            state["action_held"] == MODE_RELEASED && state["position"] == json!(position)
        })
    })
}
fn move_one(
    x11: &x11::Observer,
    process: &mut OwnedChild,
    observations: &mut Observations,
    key: u32,
    dx: i32,
    dy: i32,
    position: [i64; 2],
) -> Result<Value> {
    let before = observations.latest_sequence();
    x11.key(key, true)?;
    let first = observations.until(process, before, |observation| {
        consumed(observation, dx, dy, MODE_NAVIGATION_PULSE, |state| {
            state["position"] == json!(position)
        })
    })?;
    let sequence = first["sequence"]
        .as_u64()
        .ok_or("missing presentation sequence")?;
    let first_tick = first["tick"].as_u64().ok_or("missing simulation tick")?;
    // Keep the real key down for two further ticks: a held key must not repeat a grid move.
    let stable = observations.until(process, sequence, |observation| {
        observation["tick"]
            .as_u64()
            .is_some_and(|tick| tick >= first_tick + 2)
            && observation["state"]["position"] == json!(position)
            && observation["state"]["action_held"] == MODE_NAVIGATION_PULSE
    })?;
    release_key(
        x11,
        process,
        observations,
        key,
        stable["sequence"].as_u64().ok_or("missing sequence")?,
        position,
    )
}
fn press_space(
    x11: &x11::Observer,
    process: &mut OwnedChild,
    observations: &mut Observations,
    expected: impl Fn(&Value) -> bool,
    position: [i64; 2],
) -> Result<Value> {
    let before = observations.latest_sequence();
    x11.key(SPACE, true)?;
    let pressed = observations.until(process, before, |observation| {
        consumed(observation, 0, 0, MODE_PRIMARY, &expected)
    })?;
    release_key(
        x11,
        process,
        observations,
        SPACE,
        pressed["sequence"].as_u64().ok_or("missing sequence")?,
        position,
    )
}
fn resume(
    x11: &x11::Observer,
    process: &mut OwnedChild,
    observations: &mut Observations,
) -> Result<Value> {
    let before = observations.latest_sequence();
    x11.key(ENTER, true)?;
    x11.key(ENTER, false)?;
    observations.until(process, before, |v| {
        v["paused"] == false
            && v["neutral_consumed_tick"]
                .as_u64()
                .is_some_and(|tick| tick > v["resume_from_tick"].as_u64().unwrap_or(u64::MAX))
    })
}
fn pause(
    x11: &x11::Observer,
    process: &mut OwnedChild,
    observations: &mut Observations,
) -> Result<Value> {
    let before = observations.latest_sequence();
    x11.key(ESCAPE, true)?;
    x11.key(ESCAPE, false)?;
    observations.until(process, before, |v| v["paused"] == true)
}
fn pos(state: &Value, key: &str) -> Result<[f64; 2]> {
    let pair = state[key]
        .as_array()
        .ok_or("missing fixed-point coordinate pair")?;
    require(pair.len() == 2, "invalid coordinate pair")?;
    let convert = |v: &Value| {
        v.as_i64()
            .map(|n| n as f64 / 65536.0)
            .ok_or_else(|| error("invalid fixed-point coordinate"))
    };
    Ok([convert(&pair[0])?, convert(&pair[1])?])
}
fn capture_geometry(
    image: &x11::Capture,
    observation: &Value,
) -> Result<(usize, usize, f64, [f64; 2])> {
    let size = &observation["size"];
    let width = usize::try_from(size[0].as_u64().ok_or("missing frame width")?)?;
    let height = usize::try_from(size[1].as_u64().ok_or("missing frame height")?)?;
    require(
        width == usize::from(image.spec.width) && height == usize::from(image.spec.height),
        "X11 capture dimensions differ from the presented observation",
    )?;
    require(
        width > 0 && height > 0 && width <= 2048 && height <= 2048,
        "invalid X11 capture dimensions",
    )?;
    let expected = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(4))
        .ok_or("capture RGBA length overflow")?;
    require(
        image.rgba.len() == expected,
        "capture RGBA buffer length mismatch",
    )?;
    let center = [
        observation["camera"]["center"][0]
            .as_f64()
            .ok_or("missing camera x")?,
        observation["camera"]["center"][1]
            .as_f64()
            .ok_or("missing camera y")?,
    ];
    let half = observation["camera"]["half_extent"]
        .as_f64()
        .filter(|n| n.is_finite() && *n > 0.0)
        .ok_or("invalid camera extent")?;
    let scale = (width.min(height) as f64) / (2.0 * half);
    require(
        scale.is_finite() && scale > 0.0 && center.iter().all(|n| n.is_finite()),
        "invalid camera transform",
    )?;
    Ok((width, height, scale, center))
}
fn color_matches(rgb: [u8; 3], class: &str) -> Result<bool> {
    let [r, g, b] = rgb;
    match class {
        "blue" => Ok(b.saturating_sub(r) > 25 && b.saturating_sub(g) > 10),
        "gold" => Ok(r > 200 && g > 150 && b < 180),
        "green" => Ok(g.saturating_sub(r) > 40 && g.saturating_sub(b) > 40),
        "red" => Ok(r.saturating_sub(g) > 40 && r.saturating_sub(b) > 40),
        "purple" => Ok(r.saturating_sub(g) > 30 && b.saturating_sub(g) > 30),
        _ => Err(error("unknown screenshot color class")),
    }
}
fn region_rgb(
    image: &x11::Capture,
    observation: &Value,
    world: [f64; 2],
    inner: f64,
    outer: f64,
) -> Result<Vec<[u8; 3]>> {
    let (width, height, scale, center) = capture_geometry(image, observation)?;
    require(
        inner.is_finite() && outer.is_finite() && inner >= 0.0 && outer > inner,
        "invalid pixel-oracle radius",
    )?;
    let cx = width as f64 * 0.5 + (world[0] - center[0]) * scale;
    let cy = height as f64 * 0.5 - (world[1] - center[1]) * scale;
    let radius = (outer * scale).ceil() as i32;
    require(
        cx.is_finite()
            && cy.is_finite()
            && radius > 0
            && cx >= radius as f64
            && cy >= radius as f64
            && cx < width as f64 - radius as f64
            && cy < height as f64 - radius as f64,
        "pixel-oracle point outside image",
    )?;
    let inner_sq = inner * inner;
    let outer_sq = outer * outer;
    let mut pixels = Vec::new();
    for y in (cy as i32 - radius)..=(cy as i32 + radius) {
        for x in (cx as i32 - radius)..=(cx as i32 + radius) {
            let dx = (x as f64 + 0.5 - cx) / scale;
            let dy = (cy - (y as f64 + 0.5)) / scale;
            let distance_sq = dx * dx + dy * dy;
            if distance_sq < inner_sq || distance_sq > outer_sq {
                continue;
            }
            let at = (y as usize * width + x as usize) * 4;
            pixels.push([image.rgba[at], image.rgba[at + 1], image.rgba[at + 2]]);
        }
    }
    require(!pixels.is_empty(), "pixel-oracle sample region is empty")?;
    Ok(pixels)
}
fn color_count(
    image: &x11::Capture,
    observation: &Value,
    world: [f64; 2],
    inner: f64,
    outer: f64,
    class: &str,
) -> Result<usize> {
    let mut count = 0;
    for rgb in region_rgb(image, observation, world, inner, outer)? {
        count += usize::from(color_matches(rgb, class)?);
    }
    Ok(count)
}
fn mean_rgb(pixels: &[[u8; 3]]) -> [f64; 3] {
    let mut mean = [0.0; 3];
    for pixel in pixels {
        for channel in 0..3 {
            mean[channel] += f64::from(pixel[channel]);
        }
    }
    for channel in &mut mean {
        *channel /= pixels.len() as f64;
    }
    mean
}
fn pixel_oracle(image: &x11::Capture, observation: &Value, phase: &str) -> Result {
    let state = &observation["state"];
    let courier = pos(state, "position")?;
    let pickup = pos(state, "pickup")?;
    let destination = pos(state, "destination")?;
    let hazard = pos(state, "hazard")?;
    // Objective radii are 0.34/0.42 and the courier radius is 0.30. Sampling
    // the exposed outer band distinguishes an objective from a courier drawn
    // over its center, while avoiding the antialiased silhouette edge.
    require(
        color_count(image, observation, destination, 0.31, 0.40, "green")? > 20,
        "destination pixels missing",
    )?;
    require(
        color_count(image, observation, hazard, 0.31, 0.40, "red")? > 20,
        "hazard pixels missing",
    )?;
    match phase {
        "initial" | "restarted" => {
            require(
                color_count(image, observation, pickup, 0.05, 0.25, "gold")? > 20
                    && color_count(image, observation, courier, 0.05, 0.20, "blue")? > 10,
                "idle game pixels missing",
            )?;
        }
        "wrong_order" => {
            // The blue courier covers the destination center. Its green outer
            // band is independently checked above; the pickup remains visible.
            require(
                color_count(image, observation, pickup, 0.05, 0.25, "gold")? > 20
                    && color_count(image, observation, courier, 0.05, 0.20, "blue")? > 10,
                "wrong-order state pixels missing",
            )?;
        }
        "pickup" => {
            require(
                color_count(image, observation, pickup, 0.305, 0.335, "gold")? == 0
                    && color_count(image, observation, courier, 0.05, 0.20, "purple")? > 10,
                "pickup/carrying pixels mismatch",
            )?;
        }
        "loss" => {
            let courier_center = mean_rgb(&region_rgb(image, observation, courier, 0.0, 0.12)?);
            let hazard_ring = mean_rgb(&region_rgb(image, observation, hazard, 0.34, 0.40)?);
            require(
                color_count(image, observation, courier, 0.0, 0.14, "red")? > 10
                    && courier_center[1] > hazard_ring[1] + 6.0
                    && courier_center[2] > hazard_ring[2] + 6.0,
                "loss courier color is absent or indistinguishable from the red hazard",
            )?;
        }
        "win" => {
            let courier_center = mean_rgb(&region_rgb(image, observation, courier, 0.0, 0.12)?);
            let destination_ring =
                mean_rgb(&region_rgb(image, observation, destination, 0.34, 0.40)?);
            require(
                color_count(image, observation, courier, 0.0, 0.14, "green")? > 10
                    && courier_center[0] + 6.0 < destination_ring[0]
                    && courier_center[1] > destination_ring[1] + 6.0
                    && courier_center[2] > destination_ring[2] + 6.0,
                "win courier color is absent or indistinguishable from the green destination",
            )?;
        }
        _ => return Err(error("unknown capture phase")),
    }
    Ok(())
}

#[cfg(test)]
mod pixel_oracle_tests {
    use super::*;

    const WIDTH: u16 = 960;
    const HEIGHT: u16 = 720;
    const CENTER: [f64; 2] = [2.5, 0.5];
    const SCALE: f64 = 60.0;
    type Circle = ([f64; 2], f64, [u8; 3]);

    fn observation(position: [i64; 2]) -> Value {
        json!({
            "size":[WIDTH, HEIGHT],
            "camera":{"center":CENTER,"half_extent":6.0},
            "state":{
                "position":position,
                "pickup":[2 << 16,0],
                "destination":[5 << 16,0],
                "hazard":[2 << 16,1 << 16]
            }
        })
    }

    fn capture(shapes: &[Circle]) -> x11::Capture {
        let width = usize::from(WIDTH);
        let height = usize::from(HEIGHT);
        let mut rgba = vec![18u8; width * height * 4];
        let (pixels, _) = rgba.as_chunks_mut::<4>();
        for pixel in pixels {
            pixel[3] = 255;
        }
        for (world, radius, color) in shapes {
            let cx = width as f64 * 0.5 + (world[0] - CENTER[0]) * SCALE;
            let cy = height as f64 * 0.5 - (world[1] - CENTER[1]) * SCALE;
            let radius_px = radius * SCALE;
            for y in (cy - radius_px - 1.0).floor() as i32..=(cy + radius_px + 1.0).ceil() as i32 {
                for x in
                    (cx - radius_px - 1.0).floor() as i32..=(cx + radius_px + 1.0).ceil() as i32
                {
                    if x < 0 || y < 0 || x >= i32::from(WIDTH) || y >= i32::from(HEIGHT) {
                        continue;
                    }
                    let dx = (x as f64 + 0.5 - cx) / SCALE;
                    let dy = (cy - (y as f64 + 0.5)) / SCALE;
                    if dx * dx + dy * dy > radius * radius {
                        continue;
                    }
                    let at = (y as usize * width + x as usize) * 4;
                    rgba[at..at + 3].copy_from_slice(color);
                }
            }
        }
        x11::Capture {
            spec: x11::PixelSpec {
                width: WIDTH,
                height: HEIGHT,
                depth: 24,
                bits: 32,
                pad: 32,
                lsb: true,
                masks: [0xff0000, 0xff00, 0xff],
            },
            visual: 1,
            rgba,
        }
    }

    fn scene(position: [f64; 2], include_pickup: bool, courier: Option<[u8; 3]>) -> x11::Capture {
        let pickup = [2.0, 0.0];
        let destination = [5.0, 0.0];
        let hazard = [2.0, 1.0];
        let mut shapes = vec![
            (destination, 0.42, [48, 217, 64]),
            (hazard, 0.42, [220, 38, 31]),
        ];
        if include_pickup {
            shapes.push((pickup, 0.34, [255, 210, 40]));
        }
        if let Some(color) = courier {
            shapes.push((position, 0.30, color));
        }
        capture(&shapes)
    }

    #[test]
    fn visible_objective_bands_survive_a_courier_over_their_centers() {
        let image = scene([5.0, 0.0], true, Some([64, 128, 240]));
        assert!(pixel_oracle(&image, &observation([5 << 16, 0]), "wrong_order").is_ok());
        let without_destination = capture(&[
            ([2.0, 1.0], 0.42, [220, 38, 31]),
            ([2.0, 0.0], 0.34, [255, 210, 40]),
            ([5.0, 0.0], 0.30, [64, 128, 240]),
        ]);
        assert!(
            pixel_oracle(
                &without_destination,
                &observation([5 << 16, 0]),
                "wrong_order"
            )
            .is_err()
        );
    }

    #[test]
    fn pickup_absence_is_checked_in_the_exposed_item_band() {
        let no_item = scene([2.0, 0.0], false, Some([220, 55, 240]));
        assert!(pixel_oracle(&no_item, &observation([2 << 16, 0]), "pickup").is_ok());
        let stale_item = scene([2.0, 0.0], true, Some([220, 55, 240]));
        assert!(pixel_oracle(&stale_item, &observation([2 << 16, 0]), "pickup").is_err());
    }

    #[test]
    fn terminal_courier_colors_are_distinct_from_objective_colors() {
        let loss = scene([2.0, 1.0], false, Some([242, 95, 95]));
        assert!(pixel_oracle(&loss, &observation([2 << 16, 1 << 16]), "loss").is_ok());
        let no_loss_courier = scene([2.0, 1.0], false, None);
        assert!(pixel_oracle(&no_loss_courier, &observation([2 << 16, 1 << 16]), "loss").is_err());
        let wrong_loss_color = scene([2.0, 1.0], false, Some([64, 128, 240]));
        assert!(pixel_oracle(&wrong_loss_color, &observation([2 << 16, 1 << 16]), "loss").is_err());

        let win = scene([5.0, 0.0], false, Some([38, 242, 76]));
        assert!(pixel_oracle(&win, &observation([5 << 16, 0]), "win").is_ok());
        let no_win_courier = scene([5.0, 0.0], false, None);
        assert!(pixel_oracle(&no_win_courier, &observation([5 << 16, 0]), "win").is_err());
        let wrong_win_color = scene([5.0, 0.0], false, Some([64, 128, 240]));
        assert!(pixel_oracle(&wrong_win_color, &observation([5 << 16, 0]), "win").is_err());
    }

    #[test]
    fn capture_geometry_must_match_the_presented_frame() {
        let image = scene([0.0, 0.0], true, Some([64, 128, 240]));
        let mut frame = observation([0, 0]);
        frame["size"] = json!([WIDTH + 1, HEIGHT]);
        assert!(pixel_oracle(&image, &frame, "initial").is_err());
        let mut short = image;
        short.rgba.pop();
        assert!(pixel_oracle(&short, &observation([0, 0]), "initial").is_err());
    }
}
fn capture_phase(
    x11: &x11::Observer,
    window: u32,
    process: &mut OwnedChild,
    observations: &mut Observations,
    evidence: &Path,
    phase: &str,
) -> Result<Value> {
    if observations.latest()?["paused"] != true {
        pause(x11, process, observations)?;
    }
    let before = observations.latest()?.clone();
    let sequence = before["sequence"].as_u64().ok_or("missing sequence")?;
    let first = x11.capture(window)?;
    observations.until(process, sequence + 1, |v| {
        v["paused"] == true && v["tick"] == before["tick"] && v["state"] == before["state"]
    })?;
    let second = x11.capture(window)?;
    require(
        first.rgba == second.rgba
            && first.visual == second.visual
            && first.spec.width == second.spec.width
            && first.spec.height == second.spec.height,
        "native screenshot did not stabilize",
    )?;
    let path = evidence.join(format!("{phase}.png"));
    first.png(&path)?;
    pixel_oracle(&first, &before, phase)?;
    Ok(json!({
        "phase":phase,
        "sequence":before["sequence"],
        "tick":before["tick"],
        "state":before["state"],
        "png":format!("{phase}.png"),
        "sha256":hash(&path)?,
        "x11":{"width":first.spec.width,"height":first.spec.height,"depth":first.spec.depth,
            "bits_per_pixel":first.spec.bits,"scanline_pad":first.spec.pad,"lsb_first":first.spec.lsb,
            "masks":first.spec.masks,"visual":first.visual}
    }))
}

fn setup_project(out: &Path) -> Result<(PathBuf, PathBuf, String)> {
    let project = out.join("original project");
    fs::create_dir(&project)?;
    let manifest = project.join("collect-delivery.project.json");
    let scene = project.join("collect-delivery.scene.yaml");
    fs::write(
        &manifest,
        include_str!("../scenes/collect-delivery.project.json"),
    )?;
    fs::write(&scene, CollectDeliveryGame::SCENE)?;
    let prepared = PreparedRegisteredProject::open(
        &manifest,
        orr_collect_delivery_game::registration::registry(),
    )?;
    let mut editor = Editor::open_registered_project(prepared)?;
    editor.sync();
    editor.host_call(
        "world.singleton.patch",
        json!({"name":"CollectDeliveryState","path":"destination","value":[5,0]}),
    )?;
    require(editor.save(), "editor save failed")?;
    let authored = fs::read_to_string(&scene)?;
    require(
        authored.contains("destination: [5, 0]") && authored != CollectDeliveryGame::SCENE,
        "saved scene did not retain its non-default destination",
    )?;
    drop(editor);
    let reopened = PreparedRegisteredProject::open(
        &manifest,
        orr_collect_delivery_game::registration::registry(),
    )?;
    require(
        reopened.scene_text() == authored,
        "save/reopen changed the scene",
    )?;
    Ok((project, scene, authored))
}

fn package(out: &Path, scene: &Path, runtime_sha: &str, source: &str) -> Result<Value> {
    let built = Path::new(env!("CARGO_BIN_EXE_collect-delivery-player"));
    let trusted = out.join("trusted runtime copy");
    fs::copy(built, &trusted)?;
    fs::set_permissions(&trusted, fs::Permissions::from_mode(0o755))?;
    require(
        fs::metadata(&trusted)?.nlink() == 1 && hash(&trusted)? == runtime_sha,
        "runtime copy is not byte-identical and single-link",
    )?;
    let provenance = {
        let mut command = Command::new(&trusted);
        command
            .args(["--provenance"])
            .env_clear()
            .env("PATH", "/usr/bin:/bin");
        OwnedChild::spawn(command, out, "runtime-provenance")?.wait()?
    };
    require(
        serde_json::from_slice::<Value>(&provenance)? == json!({"source_sha":source}),
        "compiled runtime provenance mismatch",
    )?;
    let bundle_path = out.join("bundle");
    let tool = Path::new(env!("CARGO_BIN_EXE_collect-delivery-bundle"));
    let args = [
        ("--runtime", trusted.to_string_lossy().into_owned()),
        ("--runtime-sha256", hash(&trusted)?),
        ("--scene", scene.to_string_lossy().into_owned()),
        ("--output", bundle_path.to_string_lossy().into_owned()),
        ("--source-sha", source.to_owned()),
    ];
    let mut command = Command::new(tool);
    for (key, value) in args {
        command.arg(key).arg(value);
    }
    command.arg("--trusted-runtime");
    let output = OwnedChild::spawn(command, out, "bundle-cli")?.wait()?;
    let receipt: Value = serde_json::from_slice(&output)?;
    require(
        receipt["source_sha"] == source
            && receipt["runtime_sha256"] == runtime_sha
            && receipt["runtime_sha256"] == hash(&bundle_path.join("bin/standalone"))?,
        "bundle receipt/runtime provenance mismatch",
    )?;
    require(
        fs::read_to_string(bundle_path.join("scene/game.scene.yaml"))?
            == fs::read_to_string(scene)?,
        "bundle did not retain the saved scene",
    )?;
    fs::remove_file(&trusted)?;
    require(!trusted.exists(), "trusted runtime copy still exists")?;
    Ok(receipt)
}

fn sandbox(bundle: &Path, evidence: &Path, display: &str, hidden: &[PathBuf]) -> Result<Command> {
    let socket = format!(
        "/tmp/.X11-unix/X{}",
        display.strip_prefix(':').ok_or("invalid display")?
    );
    let mut command = Command::new("/usr/bin/bwrap");
    command.env_clear().args([
        "--die-with-parent",
        "--new-session",
        "--unshare-pid",
        "--unshare-net",
        "--unshare-ipc",
        "--clearenv",
        "--ro-bind",
        "/usr",
        "/usr",
        "--ro-bind",
        "/bin",
        "/bin",
        "--ro-bind",
        "/lib",
        "/lib",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
        "--dir",
        "/tmp/.X11-unix",
        "--ro-bind",
        &socket,
        &socket,
        "--dir",
        "/tmp/runtime",
        "--dir",
        "/empty",
        "--setenv",
        "PATH",
        "/usr/bin:/bin",
        "--setenv",
        "DISPLAY",
        display,
        "--setenv",
        "WGPU_BACKEND",
        "vulkan",
        "--setenv",
        "XDG_RUNTIME_DIR",
        "/tmp/runtime",
    ]);
    if Path::new("/lib64").exists() {
        command.args(["--ro-bind", "/lib64", "/lib64"]);
    }
    if Path::new("/etc/ld.so.cache").exists() {
        command.args(["--ro-bind", "/etc/ld.so.cache", "/etc/ld.so.cache"]);
    }
    let script = r#"for path do test ! -e "$path" || { echo "source path visible: $path" >&2; exit 91; }; done
test "$PWD" = /empty
test ! -e /usr/bin/cargo
test ! -e /usr/bin/rustc
cat /proc/self/mountinfo > /evidence/mountinfo.txt
cat /proc/$$/status > /evidence/process-status.txt
cat /proc/net/dev > /evidence/network-interfaces.txt
printf '%s\n' source-hidden > /evidence/isolation.txt
exec '/bundle with spaces/bin/standalone' --scene '/bundle with spaces/scene/game.scene.yaml' --record /evidence/record.orrp --observe /evidence/observations.ndjson"#;
    command
        .arg("--ro-bind")
        .arg(bundle)
        .arg("/bundle with spaces")
        .arg("--bind")
        .arg(evidence)
        .arg("/evidence")
        .args([
            "--remount-ro",
            "/",
            "--chdir",
            "/empty",
            "/bin/sh",
            "-eu",
            "-c",
            script,
            "collect-native-preflight",
        ])
        .args(hidden)
        .stdin(Stdio::null());
    Ok(command)
}

fn check_sandbox(evidence: &Path) -> Result {
    require(
        fs::read_to_string(evidence.join("isolation.txt"))? == "source-hidden\n",
        "source-hidden preflight missing",
    )?;
    let mountinfo = fs::read_to_string(evidence.join("mountinfo.txt"))?;
    let mut mounts = std::collections::BTreeMap::new();
    for line in mountinfo.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        require(
            fields.len() >= 10 && fields.contains(&"-"),
            "invalid mountinfo",
        )?;
        let path = fields[4].replace("\\040", " ");
        require(!mounts.contains_key(&path), "stacked mount in test sandbox")?;
        mounts.insert(path, fields[5].to_owned());
    }
    require(
        mounts
            .get("/bundle with spaces")
            .is_some_and(|v| v.split(',').any(|f| f == "ro"))
            && mounts
                .get("/evidence")
                .is_some_and(|v| v.split(',').any(|f| f == "rw"))
            && mounts
                .get("/")
                .is_some_and(|v| v.split(',').any(|f| f == "ro"))
            && mounts.contains_key("/proc")
            && mounts.contains_key("/tmp"),
        "read-only package/writable evidence sandbox contract failed",
    )?;
    let interfaces = fs::read_to_string(evidence.join("network-interfaces.txt"))?;
    let devices: Vec<_> = interfaces
        .lines()
        .skip(2)
        .map(|line| line.split(':').next().unwrap_or("").trim())
        .collect();
    require(
        devices == ["lo"],
        "sandbox has a non-loopback network interface",
    )?;
    require(
        fs::read_to_string(evidence.join("process-status.txt"))?
            .lines()
            .any(|l| l.starts_with("NSpid:")),
        "PID namespace evidence missing",
    )?;
    Ok(())
}

fn verify_replay(scene: &str, evidence: &Path, observations: &[Value]) -> Result<Value> {
    let bytes = read_bounded(&evidence.join("record.orrp"), 4 << 20)?;
    require(bytes.starts_with(b"ORRP"), "recorded output is not ORRP")?;
    let admitted = orr_static_starter::game_contract::admit::<CollectDeliveryGame>(scene)?;
    admitted.open_replay(&bytes)?;
    let reader = ReplayReader::<CollectDeliveryGame>::parse(&bytes)?;
    require(
        reader.header.seed == 42
            && reader.header.tick_rate == 60
            && reader.header.player_count == 1
            && reader.last_tick() > 0
            && reader.last_tick() <= 3600
            && reader.tick_count() as u64 == reader.last_tick(),
        "replay header/tick bounds mismatch",
    )?;
    let mut presented = std::collections::BTreeMap::new();
    for observation in observations {
        for tick in observation["consumed"]
            .as_array()
            .ok_or("missing consumed ticks")?
        {
            let id = tick["tick"].as_u64().ok_or("missing tick")?;
            require(
                presented.insert(id, tick.clone()).is_none(),
                "duplicate presented replay tick",
            )?;
        }
    }
    require(
        presented.len() as u64 == reader.last_tick(),
        "observation stream does not cover replay",
    )?;
    let mut session = orr_static_starter::game_contract::player::<CollectDeliveryGame>(scene)?;
    for tick in 1..=reader.last_tick() {
        let (inputs, commands) = reader.tick(tick).ok_or("replay tick gap")?;
        require(
            inputs.len() == 1
                && CollectDeliveryGame::valid_input(&inputs[0])
                && commands.is_empty(),
            "replay contains invalid input, command or player count",
        )?;
        session.set_input(PlayerSlot(0), inputs[0]);
        session.step_now().ok_or("independent replay stopped")?;
        let record = presented
            .get(&tick)
            .ok_or("presentation omitted replay tick")?;
        let frame = session.frame();
        let state = CollectDeliveryGame::state_value(orr_bridge::FrameView::of(frame));
        let input = CollectDeliveryGame::input_value(&inputs[0]);
        let checksum = format!("0x{:016x}", frame.checksum());
        require(
            record["input"] == input && record["state"] == state && record["checksum"] == checksum,
            format!("source replay differs at tick {tick}"),
        )?;
    }
    require(
        session.frame().singleton::<CollectDeliveryState>().won == 1,
        "replayed run did not finish won",
    )?;
    Ok(json!({"sha256":hash(&evidence.join("record.orrp"))?,
        "ticks":reader.tick_count(),"final_checksum":format!("0x{:016x}",session.frame().checksum()),
        "final_state":CollectDeliveryGame::state_value(orr_bridge::FrameView::of(session.frame()))}))
}

fn run_native(out: &Path) -> Result<Value> {
    let sha = source_sha()?;
    let (project, scene, authored) = setup_project(out)?;
    require(
        authored.contains("destination: [5, 0]") && !CollectDeliveryGame::SCENE.contains("[5, 0]"),
        "acceptance scene must differ from compiled default",
    )?;
    let player = Path::new(env!("CARGO_BIN_EXE_collect-delivery-player"));
    let runtime_sha = hash(player)?;
    let mut display = x11::OwnedDisplay::spawn(&out.join("xvfb.stderr"))?;
    let observer = display.connect()?;
    let receipt = package(out, &scene, &runtime_sha, sha)?;
    fs::remove_dir_all(&project)?;
    require(
        !project.exists() && !scene.exists() && !out.join("trusted runtime copy").exists(),
        "project scene or trusted runtime copy remains",
    )?;
    let bundle = out.join("bundle");
    let packaged = bundle.join("bin/standalone");
    let identity = {
        let mut command = Command::new(&packaged);
        command
            .arg("--identity")
            .arg("--scene")
            .arg(bundle.join("scene/game.scene.yaml"))
            .env_clear()
            .env("PATH", "/usr/bin:/bin");
        let bytes = OwnedChild::spawn(command, out, "identity")?.wait()?;
        serde_json::from_slice::<Value>(&bytes)?
    };
    require(
        identity["game"] == CollectDeliveryGame::ID
            && identity["build_id"] == format!("0x{:016x}", CollectDeliveryGame::BUILD)
            && identity["seed"] == 42
            && identity["tick_rate"] == 60
            && identity["player_count"] == 1,
        "packaged identity mismatch",
    )?;
    require(
        fs::read_to_string(bundle.join("scene/game.scene.yaml"))? == authored,
        "packaged runtime did not receive the exact editor-saved scene",
    )?;

    let evidence = out.join("execution");
    fs::create_dir(&evidence)?;
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .ok_or("missing source root")?
        .canonicalize()?;
    let test_binary = std::env::current_exe()?.canonicalize()?;
    let mut hidden = vec![
        source_root,
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).canonicalize()?,
        project.clone(),
        scene.clone(),
        test_binary,
        player.canonicalize()?,
        packaged.canonicalize()?,
        out.join("trusted runtime copy"),
    ];
    for key in [
        "CARGO_TARGET_DIR",
        "CARGO_HOME",
        "RUSTUP_HOME",
        "ORR_STANDALONE_TOOLCHAIN_ROOT",
    ] {
        if let Some(value) = std::env::var_os(key) {
            let path = PathBuf::from(value);
            hidden.push(path.canonicalize().unwrap_or(path));
        }
    }
    hidden.sort();
    hidden.dedup();
    let display = display.name.clone();
    let mut command = sandbox(
        &bundle.canonicalize()?,
        &evidence.canonicalize()?,
        &display,
        &hidden,
    )?;
    let stdout = out.join("player.stdout");
    let stderr = out.join("player.stderr");
    let stdout_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&stdout)?;
    let stderr_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&stderr)?;
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut process = OwnedChild {
        child: command
            .stdout(Stdio::from(stdout_file))
            .stderr(Stdio::from(stderr_file))
            .spawn()?,
        stdout,
        stderr,
    };
    let mut observations = Observations {
        path: evidence.join("observations.ndjson"),
        values: Vec::new(),
    };
    let initial = observations.until(&mut process, 0, |v| v["paused"] == true)?;
    let pid = u32::try_from(initial["process_id"].as_u64().ok_or("missing process id")?)?;
    require(
        initial["identity"] == identity
            && initial["source_sha"] == sha
            && initial["adapter"]["backend"] == "Vulkan"
            && initial["adapter"]["device_type"] == "Cpu"
            && !initial["adapter"]
                .to_string()
                .to_lowercase()
                .contains("swiftshader"),
        "native runtime provenance or CPU Vulkan adapter mismatch",
    )?;
    let window = observer.window(pid, CollectDeliveryGame::ID)?;
    observer.focus(window)?;
    let focused = observations.until(
        &mut process,
        initial["sequence"].as_u64().ok_or("missing sequence")?,
        |v| v["focused"] == true && v["paused"] == true,
    )?;
    let mut captures = Vec::new();
    captures.push(capture_phase(
        &observer,
        window,
        &mut process,
        &mut observations,
        &evidence,
        "initial",
    )?);

    resume(&observer, &mut process, &mut observations)?;
    let mut x = 0;
    for _ in 0..5 {
        x += 1;
        move_one(
            &observer,
            &mut process,
            &mut observations,
            RIGHT,
            1,
            0,
            [x << 16, 0],
        )?;
    }
    press_space(
        &observer,
        &mut process,
        &mut observations,
        |s| {
            s["position"] == json!([5 << 16, 0])
                && s["won"] == 0
                && s["carrying"] == 0
                && s["item_available"] == 1
        },
        [5 << 16, 0],
    )?;
    captures.push(capture_phase(
        &observer,
        window,
        &mut process,
        &mut observations,
        &evidence,
        "wrong_order",
    )?);
    resume(&observer, &mut process, &mut observations)?;
    for _ in 0..3 {
        x -= 1;
        move_one(
            &observer,
            &mut process,
            &mut observations,
            LEFT,
            -1,
            0,
            [x << 16, 0],
        )?;
    }
    press_space(
        &observer,
        &mut process,
        &mut observations,
        |s| {
            s["position"] == json!([2 << 16, 0])
                && s["item_available"] == 0
                && s["carrying"] == 1
                && s["won"] == 0
                && s["lost"] == 0
        },
        [2 << 16, 0],
    )?;
    captures.push(capture_phase(
        &observer,
        window,
        &mut process,
        &mut observations,
        &evidence,
        "pickup",
    )?);
    resume(&observer, &mut process, &mut observations)?;
    move_one(
        &observer,
        &mut process,
        &mut observations,
        UP,
        0,
        1,
        [2 << 16, 1 << 16],
    )?;
    require(
        observations.latest()?["state"]["lost"] == 1 && observations.latest()?["state"]["won"] == 0,
        "hazard did not produce loss",
    )?;
    captures.push(capture_phase(
        &observer,
        window,
        &mut process,
        &mut observations,
        &evidence,
        "loss",
    )?);
    resume(&observer, &mut process, &mut observations)?;
    press_space(
        &observer,
        &mut process,
        &mut observations,
        |s| {
            s["position"] == json!([0, 0])
                && s["item_available"] == 1
                && s["carrying"] == 0
                && s["won"] == 0
                && s["lost"] == 0
        },
        [0, 0],
    )?;
    captures.push(capture_phase(
        &observer,
        window,
        &mut process,
        &mut observations,
        &evidence,
        "restarted",
    )?);
    resume(&observer, &mut process, &mut observations)?;
    move_one(
        &observer,
        &mut process,
        &mut observations,
        RIGHT,
        1,
        0,
        [1 << 16, 0],
    )?;
    move_one(
        &observer,
        &mut process,
        &mut observations,
        RIGHT,
        1,
        0,
        [2 << 16, 0],
    )?;
    press_space(
        &observer,
        &mut process,
        &mut observations,
        |s| s["carrying"] == 1 && s["item_available"] == 0,
        [2 << 16, 0],
    )?;
    for x in [3, 4, 5] {
        move_one(
            &observer,
            &mut process,
            &mut observations,
            RIGHT,
            1,
            0,
            [x << 16, 0],
        )?;
    }
    press_space(
        &observer,
        &mut process,
        &mut observations,
        |s| {
            s["position"] == json!([5 << 16, 0])
                && s["carrying"] == 1
                && s["won"] == 1
                && s["lost"] == 0
        },
        [5 << 16, 0],
    )?;
    captures.push(capture_phase(
        &observer,
        window,
        &mut process,
        &mut observations,
        &evidence,
        "win",
    )?);

    observer.close(window)?;
    process.wait()?;
    observations.refresh()?;
    require(
        fs::read_to_string(evidence.join("isolation.txt"))? == "source-hidden\n",
        "source hidden sandbox preflight absent",
    )?;
    check_sandbox(&evidence)?;
    let replay = verify_replay(&authored, &evidence, &observations.values)?;
    require(
        observations.values.iter().all(|v| {
            v["schema"] == "orr.static-standalone-observation/1"
                && v["source_sha"] == sha
                && v["identity"] == identity
                && v["process_id"] == pid
        }),
        "native observation provenance changed",
    )?;
    Ok(json!({
        "schema":SCHEMA,"status":"passed","game":CollectDeliveryGame::ID,"source_sha":sha,
        "scene_sha256":hash(&bundle.join("scene/game.scene.yaml"))?,
        "runtime_sha256":receipt["runtime_sha256"],"identity":identity,
        "input_source":"XTEST","framebuffer_source":"XGetImage","physical_gpu":false,
        "windowed_native":true,"source_hidden":true,"network_disabled":true,
        "initial_focused_sequence":focused["sequence"],"captures":captures,"replay":replay,
        "observation_sha256":hash(&evidence.join("observations.ndjson"))?,
        "bundle_manifest_sha256":hash(&bundle.join("bundle.json"))?,"bundle_receipt":receipt
    }))
}

#[test]
fn collect_delivery_native_windowed_source_free_journey() -> Result {
    require(
        std::env::var("ORR_REQUIRE_COLLECT_NATIVE").as_deref() == Ok("1"),
        "selected native acceptance requires ORR_REQUIRE_COLLECT_NATIVE=1; it never skips",
    )?;
    let out = PathBuf::from(
        std::env::var_os("ORR_STANDALONE_ARTIFACT_DIR")
            .ok_or_else(|| error("missing ORR_STANDALONE_ARTIFACT_DIR"))?,
    );
    require(out.is_absolute(), "artifact directory must be absolute")?;
    fs::create_dir(&out)?;
    let sha = source_sha()?;
    write_json(
        &out.join("attempt.json"),
        &json!({"schema":SCHEMA,"status":"started","source_sha":sha}),
    )?;
    match run_native(&out) {
        Ok(report) => {
            write_json(&out.join("result.json"), &report)?;
            Ok(())
        }
        Err(problem) => {
            let _ = write_json(
                &out.join("result.json"),
                &json!({"schema":SCHEMA,"status":"failed","source_sha":sha,"error":problem.to_string()}),
            );
            Err(problem)
        }
    }
}
