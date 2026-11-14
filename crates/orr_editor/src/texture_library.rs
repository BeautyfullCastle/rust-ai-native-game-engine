//! Images of the open scene that materials pick for their shader's textures.
//! A material names a PNG path from the scene's folder; each image is read
//! when it is first needed and again when the file changes, so a person can
//! repaint it in an image editor and see each save in the viewport.
#![allow(clippy::disallowed_types)]

use orr_render::{SurfaceTexture, TextureKind};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

/// How often each file is looked at again.
const CHECK_INTERVAL: Duration = Duration::from_millis(500);

/// Versions are unique in the process, so a viewport never confuses two images.
static NEXT_VERSION: AtomicU64 = AtomicU64::new(1);

/// One image file read for one texture kind, and the result.
#[derive(Clone, Debug)]
pub struct Entry {
    /// Changes every time the file is read again.
    pub version: u64,
    /// The prepared image, or why it cannot be used, with its path.
    pub result: Result<Arc<SurfaceTexture>, String>,
    stamp: Option<(SystemTime, u64)>,
    checked: Instant,
}

#[derive(Default)]
pub struct TextureLibrary {
    root: Option<PathBuf>,
    entries: BTreeMap<(String, TextureKind), Entry>,
}

fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// The pixels of a PNG file as RGBA8 rows from top to bottom.
fn decode_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut decoder = png::Decoder::new(bytes);
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| format!("not a PNG image: {e}"))?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).map_err(|e| format!("the PNG image cannot be read: {e}"))?;
    let pixels = &buffer[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => pixels.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => pixels.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => pixels.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
        png::ColorType::Indexed => return Err("the PNG palette could not be expanded".into()),
    };
    Ok((info.width, info.height, rgba))
}

impl TextureLibrary {
    /// Points the library at the folder of the scene file `scene`. Another
    /// scene folder forgets every loaded image.
    pub fn set_scene(&mut self, scene: Option<&Path>) {
        let root = scene.and_then(Path::parent).map(Path::to_path_buf);
        if root != self.root {
            self.root = root;
            self.entries.clear();
        }
    }

    /// Reads the images in `wanted` that are not loaded yet, and those whose
    /// file changed, looking at each file at most every half second. An empty
    /// path means "the shader's default" and is skipped.
    pub fn refresh<'a>(&mut self, wanted: impl IntoIterator<Item = (&'a str, TextureKind)>) {
        let now = Instant::now();
        for (path, kind) in wanted {
            if path.is_empty() {
                continue;
            }
            let key = (path.to_string(), kind);
            if self.entries.get(&key).is_some_and(|entry| now.duration_since(entry.checked) < CHECK_INTERVAL) {
                continue;
            }
            let file = self.root.as_ref().filter(|_| valid_path(path)).map(|root| root.join(path));
            let current = file.as_deref().and_then(stamp);
            // An unchanged file, or a file still missing, keeps its entry.
            if let Some(entry) = self.entries.get_mut(&key).filter(|entry| entry.stamp == current && (current.is_some() || entry.result.is_err())) {
                entry.checked = now;
                continue;
            }
            let result = self.load(path, kind, file.as_deref()).map(Arc::new);
            let version = NEXT_VERSION.fetch_add(1, Ordering::Relaxed);
            self.entries.insert(key, Entry { version, result, stamp: current, checked: now });
        }
    }

    fn load(&self, path: &str, kind: TextureKind, file: Option<&Path>) -> Result<SurfaceTexture, String> {
        if !valid_path(path) {
            return Err(format!("'{path}' is not a PNG path next to the scene"));
        }
        let file = file.ok_or("Save the scene to a file first: textures are read from its folder")?;
        let bytes = std::fs::read(file).map_err(|e| format!("{path}: {e}"))?;
        let (width, height, rgba) = decode_png(&bytes).map_err(|e| format!("{path}: {e}"))?;
        SurfaceTexture::from_rgba8(kind, width, height, &rgba).map_err(|e| format!("{path}: {e}"))
    }

    pub fn get(&self, path: &str, kind: TextureKind) -> Option<&Entry> {
        self.entries.get(&(path.to_string(), kind))
    }
}

/// Whether `path` names a PNG inside the scene's folder. The path comes from
/// the frame, where simulation systems can write any bytes.
fn valid_path(path: &str) -> bool {
    orr_reflect::TypeDesc::asset(orr_gfx::MATERIAL_TEXTURE_PATH_BYTES, "texture").check(&orr_reflect::Value::Text(path.to_string()), true).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_png(path: &Path, width: u32, height: u32, rgb: [u8; 3]) {
        let file = std::fs::File::create(path).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
        encoder.set_color(png::ColorType::Rgb);
        let mut writer = encoder.write_header().unwrap();
        let data: Vec<u8> = (0..width * height).flat_map(|_| rgb).collect();
        writer.write_image_data(&data).unwrap();
    }

    #[test]
    fn images_load_from_the_scene_folder_and_reload_when_the_file_changes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("textures")).unwrap();
        let file = dir.path().join("textures").join("grass.png");
        write_png(&file, 4, 2, [10, 200, 30]);
        let mut library = TextureLibrary::default();
        library.set_scene(Some(&dir.path().join("level.scene.yaml")));
        library.refresh([("textures/grass.png", TextureKind::Color), ("missing.png", TextureKind::Color), ("../outside.png", TextureKind::Data), ("", TextureKind::Color)]);
        let first = library.get("textures/grass.png", TextureKind::Color).unwrap().clone();
        assert_eq!(first.result.as_ref().unwrap().size(), (4, 2));
        assert!(library.get("missing.png", TextureKind::Color).unwrap().result.as_ref().unwrap_err().starts_with("missing.png"));
        assert!(library.get("../outside.png", TextureKind::Data).unwrap().result.as_ref().unwrap_err().contains("not a PNG path"));
        assert!(library.get("", TextureKind::Color).is_none(), "an empty path means the shader's default");

        // A new save is read again after the check interval.
        write_png(&file, 8, 8, [0, 0, 0]);
        for entry in library.entries.values_mut() {
            entry.checked -= CHECK_INTERVAL;
        }
        library.refresh([("textures/grass.png", TextureKind::Color)]);
        let second = library.get("textures/grass.png", TextureKind::Color).unwrap();
        assert!(second.version > first.version);
        assert_eq!(second.result.as_ref().unwrap().size(), (8, 8));
    }

    #[test]
    fn a_scene_without_a_file_cannot_load_images() {
        let mut library = TextureLibrary::default();
        library.set_scene(None);
        library.refresh([("grass.png", TextureKind::Color)]);
        assert!(library.get("grass.png", TextureKind::Color).unwrap().result.as_ref().unwrap_err().contains("Save the scene"));
    }
}
