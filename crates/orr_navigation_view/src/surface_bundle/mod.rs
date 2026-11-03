//! Closed Linux x86_64 saved-scene point-navigation delivery verification.
//! No signature, license verification or filesystem sandbox is claimed.
pub mod export;
mod source;
pub use source::SavedSource;
pub(crate) use crate::bundle::io;
pub(crate) mod manifest;
use crate::surface_player::SurfacePlayer;
use io::{checked_path, Snapshot};
use manifest::{hash, hex_revision, Manifest};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
pub(crate) const MANIFEST_PATH: &str = "navigation.surface.bundle.json";
pub(crate) const SCENE_PATH: &str = "project/game.scene.yaml";
pub(crate) const RUNTIME_PATH: &str = "bin/navigation_surface_player";
pub(crate) const LAUNCHER_PATH: &str = "run-navigation-surface";
pub(crate) const MAX_MANIFEST: usize = 64 * 1024;
pub(crate) const MAX_SCENE: usize = 1024 * 1024;
pub(crate) const MAX_RUNTIME: usize = 64 * 1024 * 1024;
pub(crate) const MAX_SURFACE: usize = 8 * 1024 * 1024;
pub(crate) const MAX_LAUNCHER: usize = 4096;
pub(crate) const LAUNCHER:&[u8]=b"#!/bin/sh\nset -eu\nROOT=$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd -P)\nexec \"$ROOT/bin/navigation_surface_player\" --bundle \"$ROOT\" \"$@\"\n";

/// `None` is only for export-stage internal verification. Production bundle-open
/// must supply current_exe and prove both its exact path and bytes are included.
pub(crate) fn verify_bundle(
    root: &Path,
    expected_exe: Option<&Path>,
) -> Result<SurfacePlayer, String> {
    let root = checked_path(root)?;
    let manifest_snapshot = Snapshot::read(&root.join(MANIFEST_PATH), MAX_MANIFEST)?;
    if manifest_snapshot.mode != 0o644 {
        return Err("unexpected bundle manifest mode".into());
    }
    let manifest = Manifest::parse(&manifest_snapshot.bytes)?;
    let files: BTreeSet<PathBuf> = manifest
        .payload
        .files
        .iter()
        .map(|f| PathBuf::from(&f.path))
        .chain(std::iter::once(MANIFEST_PATH.into()))
        .collect();
    io::inventory(&root, &files)?;
    let limits = [
        MAX_SCENE,
        orr_terrain::MAX_FILE_BYTES,
        MAX_SURFACE,
        MAX_RUNTIME,
        MAX_LAUNCHER,
    ];
    let mut snapshots = Vec::new();
    for (descriptor, limit) in manifest.payload.files.iter().zip(limits) {
        let snapshot = Snapshot::read(&root.join(&descriptor.path), limit)?;
        if snapshot.mode != descriptor.mode
            || snapshot.bytes.len() != descriptor.bytes
            || hash(&snapshot.bytes) != descriptor.sha256
        {
            return Err(format!("bundle descriptor mismatch: {}", descriptor.path));
        }
        snapshots.push(snapshot);
    }
    if snapshots[4].bytes != LAUNCHER {
        return Err("bundle launcher is not the fixed contract".into());
    }
    if let Some(executable) = expected_exe {
        let executable = checked_path(executable)?;
        if executable != root.join(RUNTIME_PATH) {
            return Err("running executable is outside this bundle".into());
        }
        let running = Snapshot::read(&executable, MAX_RUNTIME)?;
        if running.bytes != snapshots[3].bytes || running.mode != 0o755 {
            return Err("running executable differs from bundle runtime".into());
        }
        running.recheck()?;
    }
    let authored = crate::authored::AuthoredPlayer::open(&root.join(SCENE_PATH))?;
    // Surface bytes come exclusively from the verified descriptor snapshot.
    let surface = orr_terrain_view::surface::Surface::from_bytes(&snapshots[2].bytes)
        .map_err(|e| e.to_string())?;
    let player = SurfacePlayer::from_verified(authored, surface)?;
    let pin = player
        .frame()
        .singleton::<orr_remote::navigation_yard3d::NavigationScenePin>();
    if format!("project/{}", pin.source()?) != manifest.payload.files[1].path {
        return Err("scene pin differs from manifest terrain path".into());
    }
    let view = player.view()?;
    if player.tick() != 0
        || player.checksum() != manifest.payload.initial_checksum
        || view.terrain.asset_id() != manifest.payload.terrain_asset_id
        || hex_revision(view.terrain.revision()) != manifest.payload.terrain_revision
        || hex_revision(view.graph.revision()) != manifest.payload.graph_revision
    {
        return Err("bundle admitted initial state differs from manifest".into());
    }
    for snapshot in &snapshots {
        snapshot.recheck()?;
    }
    manifest_snapshot.recheck()?;
    io::inventory(&root, &files)?;
    Ok(player)
}
