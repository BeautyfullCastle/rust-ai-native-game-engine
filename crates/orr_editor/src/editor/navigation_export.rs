//! Authoritative saved-route receipt, not an editor lock or runtime trust token.
use super::*;
use crate::{navigation_panel::NavigationPanel, terrain_panel::TerrainPanel};
use orr_navigation_view::bundle::SavedSource;

impl Editor {
    fn navigation_export_idle(
        &self,
        route: &NavigationPanel,
        terrain: &TerrainPanel,
    ) -> Result<(), String> {
        if self.game() != EditorGame::NavigationYard3D
            || !self.spec().is_local()
            || self.erp_status().is_some()
            || !self.can_mutate()
            || !self.scene_edit_allowed
            || self.mode() != Mode::Edit
            || self.previewing().is_some()
            || self.in_gesture()
            || self.batch_uncertain
            || self.down.is_some()
            || self.history.in_tx
            || self.sim.in_tx
            || self.input_phase() != input::Phase::Off
            || self.input_cleanup_pending()
            || !self.screenshot_queries.is_empty()
            || !self.screenshot_validations.is_empty()
            || self.is_dirty()
            || self
                .pending
                .values()
                .any(|p| matches!(p.kind, Pend::Gesture(..) | Pend::Input(..)))
        {
            return Err("Navigation export requires a saved idle local Edit scene without listener, gesture, transaction, preview or pending mutation".into());
        }
        let snapshot = self
            .snapshot
            .as_ref()
            .ok_or("Wait for the admitted navigation Frame")?;
        if snapshot.seq() == 0
            || snapshot.tick() != 0
            || snapshot.timeline().is_some()
            || snapshot.predicted().tick() != 0
            || !self.yard_rows_coherent()
            || !self.admitted_navigation().admitted
        {
            return Err("Wait for a coherent admitted navigation initial Frame".into());
        }
        if terrain.document.dirty()
            || terrain.document.read_only()
            || (terrain.document.terrain().is_some() && !terrain.scene_matches(self))
            || route.route_stale(self, terrain)
        {
            return Err(
                "Save terrain and rebuild the current route explicitly before export".into(),
            );
        }
        if let Some(reason) = route.play_blocker(self, terrain) {
            return Err(reason);
        }
        Ok(())
    }
    pub(crate) fn navigation_export_source(
        &mut self,
        route: &NavigationPanel,
        terrain: &TerrainPanel,
    ) -> Result<SavedSource, String> {
        self.navigation_export_source_with(route, terrain, |_| {})
    }
    fn navigation_export_source_with(
        &mut self,
        route: &NavigationPanel,
        terrain: &TerrainPanel,
        checkpoint: impl FnOnce(&mut Self),
    ) -> Result<SavedSource, String> {
        self.navigation_export_idle(route, terrain)?;
        let generations = (
            self.change_gen,
            self.state_generation,
            self.scene_load_epoch(),
        );
        let before = self.call("sim.state", J::Null)?;
        let (path, checksum) = clean_state(&before)?;
        let history = self.call("history.list", J::Null)?;
        clean_history(&history)?;
        let proposals = self.call("proposal.list", J::Null)?;
        if proposals
            .get("proposals")
            .and_then(J::as_array)
            .is_none_or(|p| !p.is_empty())
        {
            return Err("Resolve open proposals before navigation export".into());
        }
        let doc = self.call("scene.save", json!({"write":false,"source":"doc"}))?;
        let text = doc
            .get("text")
            .and_then(J::as_str)
            .ok_or("Missing authoritative document")?;
        if text.len() > 1024 * 1024
            || doc.get("dirty").and_then(J::as_bool) != Some(false)
            || doc
                .get("checksum")
                .and_then(orr_remote::wire::parse_checksum)
                != Some(checksum)
        {
            return Err("Authoritative document is dirty, oversized or incoherent".into());
        }
        let source = SavedSource::capture(&path)?;
        if self.path().as_deref() != Some(source.scene_path())
            || source.initial_checksum() != checksum
        {
            return Err("Saved route and current scene identity differ".into());
        }
        let saved = orr_reflect::Scene::parse(
            std::str::from_utf8(source.scene_bytes()).map_err(|e| e.to_string())?,
            &self.types,
        )
        .map_err(|e| e.to_string())?;
        let current = orr_reflect::Scene::parse(text, &self.types).map_err(|e| e.to_string())?;
        if saved != current {
            return Err(
                "Saved scene differs from the authoritative document; save or reopen explicitly"
                    .into(),
            );
        }
        // FrameView intentionally has no raw-byte escape hatch. Query an actual local
        // authoritative frame, and bind it to the coherent visible initial checksum.
        // Owned ephemeral connection: every return drops its subscription; never cache
        // it in Editor.agent_clients or reuse a named script client.
        let mut client = self
            .backend
            .agent_client("navigation-export-admission")
            .map_err(|e| e.to_string())?;
        client.local_frames.clear();
        client
            .call(
                "watch.subscribe",
                json!({"topics":["frames"],"source":"view","max_fps":60}),
            )
            .map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let bytes = loop {
            client.poll().map_err(|e| e.to_string())?;
            let mut found = None;
            while let Some(frame) = client.local_frames.pop_front() {
                if frame.frame.tick() == 0 && frame.frame.checksum() == checksum {
                    found = Some(frame.frame.to_bytes());
                    break;
                }
            }
            if let Some(bytes) = found {
                break bytes;
            }
            if Instant::now() >= deadline {
                return Err("Timed out waiting for matching authoritative navigation Frame".into());
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        if bytes != source.initial_frame_bytes() {
            return Err("Saved route initial Frame differs from authoritative editor Frame".into());
        }
        drop(client);
        checkpoint(self);
        let after_history = self.call("history.list", J::Null)?;
        let after = self.call("sim.state", J::Null)?;
        self.navigation_export_idle(route, terrain)?;
        if before != after
            || history != after_history
            || generations
                != (
                    self.change_gen,
                    self.state_generation,
                    self.scene_load_epoch(),
                )
            || self.sim.checksum != checksum
            || self.checksum != checksum
            || self
                .snapshot
                .as_ref()
                .is_none_or(|s| s.predicted().checksum() != checksum)
        {
            return Err("Host/history/frame changed during navigation export capture; retry after it settles".into());
        }
        source.recheck()?;
        Ok(source)
    }
}

fn clean_state(state: &J) -> Result<(PathBuf, u64), String> {
    if state.get("mode").and_then(J::as_str) != Some("edit")
        || state.get("playing").and_then(J::as_bool) != Some(false)
        || state.get("dirty").and_then(J::as_bool) != Some(false)
        || state.get("in_tx").and_then(J::as_bool) != Some(false)
        || state.get("head_tick").and_then(J::as_u64) != Some(0)
        || state.get("epoch").and_then(J::as_u64) != Some(0)
        || state.get("session_mode").is_some()
    {
        return Err("Save the current scene and finish Play, control and transactions before exporting navigation".into());
    }
    let path = state
        .get("scene_path")
        .and_then(J::as_str)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .ok_or("Save the current scene before exporting navigation")?;
    let checksum = state
        .get("checksum")
        .and_then(orr_remote::wire::parse_checksum)
        .ok_or("Host did not return an authoritative scene checksum")?;
    if state
        .get("doc_checksum")
        .and_then(orr_remote::wire::parse_checksum)
        != Some(checksum)
    {
        return Err("Host document and visible scene checksums do not agree".into());
    }
    Ok((path, checksum))
}

fn clean_history(history: &J) -> Result<(), String> {
    if history.get("dirty").and_then(J::as_bool) != Some(false)
        || history.get("in_tx").and_then(J::as_bool) != Some(false)
        || history.get("can_undo").and_then(J::as_bool).is_none()
        || history.get("can_redo").and_then(J::as_bool).is_none()
        || history
            .get("entries")
            .and_then(J::as_array)
            .is_none_or(|entries| {
                entries.iter().any(|entry| {
                    entry.get("id").and_then(J::as_u64).is_none()
                        || entry.get("label").and_then(J::as_str).is_none()
                        || entry.get("origin").and_then(J::as_str).is_none()
                        || entry.get("op_count").and_then(J::as_u64).is_none()
                        || entry.get("undone").and_then(J::as_bool).is_none()
                })
            })
    {
        return Err("Current history is dirty, in a transaction or unavailable; resolve it before exporting navigation".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "navigation_export_tests.rs"]
mod tests;
