//! Synchronous publication pins memory independently of the saved backing files.
use super::{Editor, Fields, Recipe, TerrainPanel, TerrainSource};
use std::{
    fs::{self, File, Metadata},
    io::Read,
    path::{Component, Path, PathBuf},
};

const MAX_SCENE_BYTES: usize = 1024 * 1024;
struct Snapshot {
    fields: Fields,
    output: String,
    recipe: Recipe,
    cook: Vec<u8>,
    source: TerrainSource,
    scene: PathBuf,
    epoch: u64,
    target: PathBuf,
    scene_bytes: Vec<u8>,
    terrain_bytes: Vec<u8>,
}
impl Snapshot {
    fn capture(panel: &TerrainPanel, editor: &Editor) -> Result<Self, String> {
        panel.raw16_context(editor)?;
        let source = panel
            .document
            .source()
            .ok_or("Missing terrain source")?
            .clone();
        let scene = panel
            .document
            .scene()
            .ok_or("Missing terrain scene")?
            .to_owned();
        let TerrainSource::Local { relative_path } = &source else {
            return Err("Export needs writable scene terrain".into());
        };
        let target = scene
            .parent()
            .ok_or("Missing scene directory")?
            .join(relative_path);
        let snapshot = Self {
            fields: panel.raw16.fields.clone(),
            output: panel.raw16.output_path.clone(),
            recipe: panel.raw16_recipe()?,
            cook: panel.document.bytes().ok_or("Missing terrain")?.to_vec(),
            source,
            scene_bytes: read_bounded(&scene, MAX_SCENE_BYTES)?,
            terrain_bytes: read_bounded(&target, orr_terrain::MAX_FILE_BYTES)?,
            scene,
            target,
            epoch: editor.scene_load_epoch(),
        };
        snapshot.check(panel, editor)?;
        Ok(snapshot)
    }
    fn check(&self, panel: &TerrainPanel, editor: &Editor) -> Result<(), String> {
        panel.raw16_context(editor)?;
        if panel.raw16.fields != self.fields
            || panel.raw16.output_path != self.output
            || panel.raw16_recipe()? != self.recipe
            || panel.document.bytes() != Some(self.cook.as_slice())
            || panel.document.source() != Some(&self.source)
            || panel.document.scene() != Some(self.scene.as_path())
            || editor.scene_load_epoch() != self.epoch
            || read_bounded(&self.scene, MAX_SCENE_BYTES)? != self.scene_bytes
            || read_bounded(&self.target, orr_terrain::MAX_FILE_BYTES)? != self.terrain_bytes
        {
            return Err("RAW16 export context or backing files changed; nothing published".into());
        }
        Ok(())
    }
}
impl TerrainPanel {
    pub fn export_raw16(&mut self, editor: &Editor) -> Result<(), String> {
        self.raw16.export_status = None;
        let result = (|| {
            let output = Path::new(&self.raw16.output_path);
            if !output.is_absolute()
                || self.raw16.output_path.len() > 4096
                || output.extension().and_then(|s| s.to_str()) != Some("raw")
            {
                return Err("RAW16 output must be an explicit absolute .raw path of at most 4096 UTF-8 bytes".into());
            }
            let snapshot = Snapshot::capture(self, editor)?;
            let terrain = self.document.terrain().ok_or("Open terrain first")?;
            let published = orr_terrain_view::raw16::output::export_new_checked(
                output,
                terrain,
                &snapshot.recipe,
                || snapshot.check(self, editor),
            )
            .map_err(|e| e.to_string())?;
            let mut status = format!("Published {} RAW16 bytes with exact current heights; terrain and Undo history unchanged.", published.bytes);
            if let Some(warning) = published.durability_warning {
                status.push_str(&format!(" Published, but durability is uncertain; do not retry this destination: {warning}"));
            }
            if let Some(warning) = published.cleanup_warning {
                status.push_str(&format!(" Published, but cleanup needs attention; do not retry this destination: {warning}"));
            }
            self.raw16.export_status = Some(status);
            Ok(())
        })();
        self.report(result)
    }
}
// Trusted stable local ancestry, like the RAW input reader; not a hostile
// concurrent-filesystem sandbox. Bound allocation before opening and after read.
fn checked_metadata(path: &Path) -> Result<Metadata, String> {
    if !path.is_absolute() || path.components().any(|p| matches!(p, Component::ParentDir)) {
        return Err("Export backing paths must be absolute without traversal".into());
    }
    let mut current = PathBuf::new();
    for part in path.components() {
        if matches!(part, Component::CurDir) {
            continue;
        }
        current.push(part.as_os_str());
        if matches!(part, Component::Prefix(_)) {
            continue;
        }
        if fs::symlink_metadata(&current)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            return Err("Export backing paths must not contain symlinks".into());
        }
    }
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("Export backing source must be a regular file".into());
    }
    Ok(metadata)
}
fn same(a: &Metadata, b: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if a.dev() != b.dev() || a.ino() != b.ino() {
            return false;
        }
    }
    a.len() == b.len() && a.modified().ok() == b.modified().ok()
}
fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let before = checked_metadata(path)?;
    if before.len() > limit as u64 {
        return Err("Export backing source exceeds byte limit".into());
    }
    let file = File::open(path).map_err(|e| e.to_string())?;
    let opened = file.metadata().map_err(|e| e.to_string())?;
    if !opened.is_file() || !same(&before, &opened) || !same(&opened, &checked_metadata(path)?) {
        return Err("Export backing source changed during open".into());
    }
    let mut bytes = Vec::new();
    (&file)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit
        || bytes.len() as u64 != opened.len()
        || !same(&opened, &file.metadata().map_err(|e| e.to_string())?)
        || !same(&opened, &checked_metadata(path)?)
    {
        return Err("Export backing source changed during read".into());
    }
    Ok(bytes)
}
#[cfg(test)]
mod tests;
