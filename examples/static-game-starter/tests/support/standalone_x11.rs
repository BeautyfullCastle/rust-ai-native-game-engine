//! Dev-only observer. Never connects to the caller's DISPLAY or mutates game state.
use std::{
    error::Error,
    io::{BufRead, BufReader, Read},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::Duration,
};
use x11rb::{
    connection::Connection,
    protocol::{xproto::*, xtest},
    rust_connection::RustConnection,
    COPY_DEPTH_FROM_PARENT, CURRENT_TIME,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const WAIT: Duration = Duration::from_secs(15);

// Host-only watchdog clock, never simulation time or an input source.
#[allow(
    clippy::disallowed_types,
    reason = "Dev-only process/window watchdogs need monotonic host deadlines; no simulation state uses this clock"
)]
mod host_clock {
    use std::time::{Duration, Instant};

    pub struct Deadline(Instant);

    impl Deadline {
        pub fn after(duration: Duration) -> Self {
            Self(Instant::now() + duration)
        }

        pub fn expired(&self) -> bool {
            Instant::now() >= self.0
        }
    }
}

pub use host_clock::Deadline;

/// Only this owned child is killed, including on any initialization failure.
pub struct OwnedDisplay {
    child: Child,
    pub name: String,
}
impl OwnedDisplay {
    pub fn spawn(log: &Path) -> Result<Self> {
        let stderr = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(log)?;
        let child = Command::new("/usr/bin/Xvfb")
            .args([
                "-displayfd",
                "1",
                "-screen",
                "0",
                "1024x768x24",
                "-nolisten",
                "tcp",
                "-noreset",
                "-ac",
            ])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(stderr)
            .spawn()?;
        let mut owned = Self {
            child,
            name: String::new(),
        };
        let stdout = owned
            .child
            .stdout
            .take()
            .ok_or("missing Xvfb display pipe")?;
        let (send, receive) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut line = String::new();
            let result = BufReader::new(stdout)
                .take(32)
                .read_line(&mut line)
                .map(|_| line);
            let _ = send.send(result);
        });
        let line = receive.recv_timeout(WAIT)??;
        let number = line.trim();
        if number.is_empty() || number.len() > 5 || !number.bytes().all(|b| b.is_ascii_digit()) {
            return Err("invalid owned Xvfb display number".into());
        }
        owned.name = format!(":{number}");
        if owned.child.try_wait()?.is_some() {
            return Err("owned Xvfb exited".into());
        }
        Ok(owned)
    }
    pub fn connect(&mut self) -> Result<Observer> {
        if self.child.try_wait()?.is_some() {
            return Err("owned Xvfb exited".into());
        }
        let (connection, screen) = x11rb::connect(Some(&self.name))?;
        xtest::get_version(&connection, 2, 2)?.reply()?;
        let root = connection.setup().roots[screen].root;
        let sink = connection.generate_id()?;
        connection
            .create_window(
                COPY_DEPTH_FROM_PARENT,
                sink,
                root,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_ONLY,
                0,
                &CreateWindowAux::new(),
            )?
            .check()?;
        connection.map_window(sink)?.check()?;
        connection.flush()?;
        Ok(Observer {
            connection,
            screen,
            sink,
        })
    }
}
impl Drop for OwnedDisplay {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub struct Observer {
    connection: RustConnection,
    screen: usize,
    sink: Window,
}
impl Observer {
    /// Match the actual OS process and exact title prefix, not the first window.
    pub fn window(&self, pid: u32, title_prefix: &str) -> Result<Window> {
        if pid == 0 || title_prefix.is_empty() {
            return Err("unbounded window selector".into());
        }
        let atom_pid = self
            .connection
            .intern_atom(false, b"_NET_WM_PID")?
            .reply()?
            .atom;
        let atom_title = self
            .connection
            .intern_atom(false, b"_NET_WM_NAME")?
            .reply()?
            .atom;
        let utf8 = self
            .connection
            .intern_atom(false, b"UTF8_STRING")?
            .reply()?
            .atom;
        let root = self.connection.setup().roots[self.screen].root;
        let until = Deadline::after(WAIT);
        loop {
            let mut matches = Vec::new();
            let mut queue = vec![root];
            let mut inspected = 0;
            while let Some(parent) = queue.pop() {
                inspected += 1;
                if inspected > 256 {
                    return Err("too many windows on owned display".into());
                }
                for child in self.connection.query_tree(parent)?.reply()?.children {
                    queue.push(child);
                    let attrs = self.connection.get_window_attributes(child)?.reply()?;
                    if attrs.map_state != MapState::VIEWABLE
                        || attrs.class != WindowClass::INPUT_OUTPUT
                    {
                        continue;
                    }
                    let property = self
                        .connection
                        .get_property(false, child, atom_pid, AtomEnum::CARDINAL, 0, 1)?
                        .reply()?;
                    if property.value32().and_then(|mut values| values.next()) != Some(pid) {
                        continue;
                    }
                    let title = self
                        .connection
                        .get_property(false, child, atom_title, utf8, 0, 256)?
                        .reply()?;
                    if title.bytes_after != 0 {
                        return Err("oversize window title".into());
                    }
                    if std::str::from_utf8(&title.value)?.starts_with(title_prefix) {
                        matches.push(child);
                    }
                }
            }
            match matches.as_slice() {
                [window] => return Ok(*window),
                [] => {}
                _ => return Err("ambiguous player windows".into()),
            }
            if until.expired() {
                return Err("player did not map its own window".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    pub fn focus(&self, window: Window) -> Result<()> {
        self.connection
            .set_input_focus(InputFocus::PARENT, window, CURRENT_TIME)?
            .check()?;
        self.connection.flush()?;
        if self.connection.get_input_focus()?.reply()?.focus != window {
            return Err("focus not delivered".into());
        }
        Ok(())
    }
    pub fn blur(&self) -> Result<()> {
        self.focus(self.sink)
    }
    /// Resolve actual server keyboard mapping; XTEST generates normal server events,
    /// not XSendEvent synthetic events or direct game-controller calls.
    pub fn key(&self, keysym: u32, pressed: bool) -> Result<()> {
        let setup = self.connection.setup();
        let count = setup
            .max_keycode
            .checked_sub(setup.min_keycode)
            .and_then(|v| v.checked_add(1))
            .ok_or("keyboard range")?;
        let mapping = self
            .connection
            .get_keyboard_mapping(setup.min_keycode, count)?
            .reply()?;
        let width = usize::from(mapping.keysyms_per_keycode);
        if width == 0 {
            return Err("empty keyboard mapping".into());
        }
        let offset = mapping
            .keysyms
            .chunks_exact(width)
            .position(|symbols| symbols[0] == keysym)
            .ok_or("required unmodified keysym unavailable")?;
        let code = setup
            .min_keycode
            .checked_add(u8::try_from(offset)?)
            .ok_or("keycode overflow")?;
        xtest::fake_input(
            &self.connection,
            if pressed {
                KEY_PRESS_EVENT
            } else {
                KEY_RELEASE_EVENT
            },
            code,
            CURRENT_TIME,
            setup.roots[self.screen].root,
            0,
            0,
            0,
        )?
        .check()?;
        self.connection.flush()?;
        Ok(())
    }
    /// Deliver a real server-side mouse click inside the selected owned window.
    #[allow(
        dead_code,
        reason = "Used only by the separate editor XTEST acceptance target"
    )]
    pub fn click(&self, window: Window, x: i16, y: i16) -> Result<()> {
        let geometry = self.connection.get_geometry(window)?.reply()?;
        if x < 0 || y < 0 || x as u16 >= geometry.width || y as u16 >= geometry.height {
            return Err("click is outside the owned window".into());
        }
        let root = self.connection.setup().roots[self.screen].root;
        let translated = self
            .connection
            .translate_coordinates(window, root, x, y)?
            .reply()?;
        if !translated.same_screen {
            return Err("click crosses screens".into());
        }
        xtest::fake_input(
            &self.connection,
            MOTION_NOTIFY_EVENT,
            0,
            CURRENT_TIME,
            root,
            translated.dst_x,
            translated.dst_y,
            0,
        )?
        .check()?;
        for kind in [BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT] {
            xtest::fake_input(&self.connection, kind, 1, CURRENT_TIME, root, 0, 0, 0)?.check()?;
        }
        self.connection.flush()?;
        self.connection.get_input_focus()?.reply()?;
        Ok(())
    }
    pub fn close(&self, window: Window) -> Result<()> {
        let protocols = self
            .connection
            .intern_atom(false, b"WM_PROTOCOLS")?
            .reply()?
            .atom;
        let delete = self
            .connection
            .intern_atom(false, b"WM_DELETE_WINDOW")?
            .reply()?
            .atom;
        let event = ClientMessageEvent::new(32, window, protocols, [delete, CURRENT_TIME, 0, 0, 0]);
        self.connection
            .send_event(false, window, EventMask::NO_EVENT, event)?
            .check()?;
        self.connection.flush()?;
        Ok(())
    }
    pub fn capture(&self, window: Window) -> Result<Capture> {
        let attrs = self.connection.get_window_attributes(window)?.reply()?;
        if attrs.map_state != MapState::VIEWABLE {
            return Err("capture window is not mapped".into());
        }
        let geometry = self.connection.get_geometry(window)?.reply()?;
        let image = self
            .connection
            .get_image(
                ImageFormat::Z_PIXMAP,
                window,
                0,
                0,
                geometry.width,
                geometry.height,
                u32::MAX,
            )?
            .reply()?;
        if image.depth != geometry.depth || image.visual != attrs.visual {
            return Err("image visual/depth mismatch".into());
        }
        let setup = self.connection.setup();
        let format = setup
            .pixmap_formats
            .iter()
            .find(|f| f.depth == image.depth)
            .ok_or("missing pixmap format")?;
        let visual = setup.roots[self.screen]
            .allowed_depths
            .iter()
            .filter(|d| d.depth == image.depth)
            .flat_map(|d| &d.visuals)
            .find(|v| v.visual_id == image.visual)
            .ok_or("missing image visual")?;
        if visual.class != VisualClass::TRUE_COLOR {
            return Err("only TrueColor capture supported".into());
        }
        let spec = PixelSpec {
            width: geometry.width,
            height: geometry.height,
            depth: image.depth,
            bits: format.bits_per_pixel,
            pad: format.scanline_pad,
            lsb: setup.image_byte_order == ImageOrder::LSB_FIRST,
            masks: [visual.red_mask, visual.green_mask, visual.blue_mask],
        };
        let rgba = decode(spec, &image.data)?;
        Ok(Capture {
            spec,
            visual: image.visual,
            rgba,
        })
    }
}
#[derive(Clone, Copy, Debug)]
pub struct PixelSpec {
    pub width: u16,
    pub height: u16,
    pub depth: u8,
    pub bits: u8,
    pub pad: u8,
    pub lsb: bool,
    pub masks: [u32; 3],
}
pub struct Capture {
    pub spec: PixelSpec,
    pub visual: u32,
    pub rgba: Vec<u8>,
}
impl Capture {
    pub fn png(&self, path: &Path) -> Result<()> {
        let output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        let mut encoder = png::Encoder::new(
            output,
            u32::from(self.spec.width),
            u32::from(self.spec.height),
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&self.rgba)?;
        Ok(())
    }
}

fn decode(spec: PixelSpec, raw: &[u8]) -> Result<Vec<u8>> {
    let PixelSpec {
        width,
        height,
        depth,
        bits,
        pad,
        lsb,
        masks,
    } = spec;
    if width == 0
        || height == 0
        || width > 2048
        || height > 2048
        || depth == 0
        || depth > bits
        || ![8, 16, 24, 32].contains(&bits)
        || ![8, 16, 32].contains(&pad)
    {
        return Err("invalid capture dimensions/format".into());
    }
    let used = masks.iter().fold(0, |a, b| a | b);
    if used.count_ones() > u32::from(depth)
        || (bits < 32 && used >> bits != 0)
        || masks.iter().any(|m| {
            *m == 0
                || (*m >> m.trailing_zeros()).count_ones()
                    != 32 - (*m >> m.trailing_zeros()).leading_zeros()
        })
        || masks[0] & masks[1] != 0
        || masks[0] & masks[2] != 0
        || masks[1] & masks[2] != 0
    {
        return Err("invalid/overlapping/noncontiguous visual masks".into());
    }
    let bytes = usize::from(bits / 8);
    let stride =
        (usize::from(width) * usize::from(bits)).div_ceil(usize::from(pad)) * usize::from(pad / 8);
    let length = stride * usize::from(height);
    // X11 reply padding is to a four-byte boundary, beyond scanline padding.
    if raw.len() != length.div_ceil(4) * 4 {
        return Err("invalid image byte count".into());
    }
    let mut out = Vec::with_capacity(usize::from(width) * usize::from(height) * 4);
    for row in raw[..length].chunks_exact(stride) {
        for pixel in row[..usize::from(width) * bytes].chunks_exact(bytes) {
            let mut value = 0u32;
            for (at, byte) in pixel.iter().enumerate() {
                let shift = if lsb { at } else { bytes - 1 - at } * 8;
                value |= u32::from(*byte) << shift;
            }
            for mask in masks {
                let max = mask >> mask.trailing_zeros();
                let component = (value & mask) >> mask.trailing_zeros();
                out.push(u8::try_from(
                    (u64::from(component) * 255 + u64::from(max) / 2) / u64::from(max),
                )?);
            }
            out.push(255);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spec() -> PixelSpec {
        PixelSpec {
            width: 1,
            height: 1,
            depth: 24,
            bits: 32,
            pad: 32,
            lsb: true,
            masks: [0xff0000, 0xff00, 0xff],
        }
    }
    #[test]
    fn little_and_big_endian_pixels() {
        assert_eq!(decode(spec(), &[3, 17, 240, 0]).unwrap(), [240, 17, 3, 255]);
        assert_eq!(
            decode(
                PixelSpec {
                    lsb: false,
                    ..spec()
                },
                &[0, 240, 17, 3]
            )
            .unwrap(),
            [240, 17, 3, 255]
        );
    }
    #[test]
    fn rgb565_masks_and_scanline_padding() {
        let s = PixelSpec {
            width: 1,
            height: 2,
            depth: 16,
            bits: 16,
            masks: [0xf800, 0x7e0, 0x1f],
            ..spec()
        };
        assert_eq!(
            decode(s, &[0, 248, 99, 99, 224, 7, 99, 99]).unwrap(),
            [255, 0, 0, 255, 0, 255, 0, 255]
        );
    }
    #[test]
    fn swapped_masks_and_packed24() {
        let s = PixelSpec {
            bits: 24,
            masks: [0xff, 0xff00, 0xff0000],
            ..spec()
        };
        assert_eq!(decode(s, &[240, 17, 3, 99]).unwrap(), [240, 17, 3, 255]);
    }
    #[test]
    fn reject_bad_bounds_masks_and_lengths() {
        for s in [
            PixelSpec { width: 0, ..spec() },
            PixelSpec {
                height: 2049,
                ..spec()
            },
            PixelSpec {
                depth: 33,
                ..spec()
            },
            PixelSpec { pad: 7, ..spec() },
            PixelSpec {
                masks: [0, 0xff00, 0xff],
                ..spec()
            },
            PixelSpec {
                masks: [0xff, 0xff, 0xff0000],
                ..spec()
            },
            PixelSpec {
                masks: [0xa00000, 0xff00, 0xff],
                ..spec()
            },
        ] {
            assert!(decode(s, &[0; 4]).is_err(), "{s:?}");
        }
        assert!(decode(spec(), &[0; 3]).is_err());
        assert!(decode(spec(), &[0; 8]).is_err());
    }
}
