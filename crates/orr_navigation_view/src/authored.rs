//! Read-only saved NavigationYard3D player. Source admission requires trusted,
//! quiescent ancestry: the existing local admission checks symlinks but is not
//! an atomic filesystem sandbox. After open, step/view/restart use Frame only.
use orr_bridge::FrameView;
use orr_ecs::Frame;
use orr_games::navigation_yard3d_game::validate_scene_frame;
use orr_remote::navigation_yard3d::{
    navigation_from_view, navigation_yard3d_doc_from_yaml, LocalNavigationAdmission,
    NavigationStepStatus, NavigationView, NavigationYard3D, NAVIGATION_BUILD_ID, YARD_PLAYERS,
    YARD_TICK_RATE,
};
use orr_sim::{Simulation, TickInputs};
use std::{
    fs::{self, File},
    io::Read,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::Path,
};
const MAX_SCENE: usize = 1024 * 1024;

pub struct AuthoredPlayer {
    initial: Frame,
    simulation: Simulation<NavigationYard3D>,
}
fn unchanged(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.len() == b.len()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
}
fn validate(frame: &Frame) -> Result<(), String> {
    let status = frame.singleton::<NavigationStepStatus>();
    if status.failed != 0 {
        return Err(format!(
            "navigation failed at tick {}: {}",
            status.failed_tick,
            status.message()
        ));
    }
    validate_scene_frame(frame)?;
    navigation_from_view(FrameView::of(frame))?;
    Ok(())
}
impl AuthoredPlayer {
    pub fn open(path: &Path) -> Result<Self, String> {
        if path.as_os_str().as_bytes().len() > 4096 {
            return Err("scene path exceeds4KiB".into());
        }
        // Preserve the existing scene-relative terrain authority and no-symlink
        // policy, rather than resolving the root through canonicalize.
        let admission = LocalNavigationAdmission::for_scene(path)?;
        let mut file = File::open(path).map_err(|e| e.to_string())?;
        let before = file.metadata().map_err(|e| e.to_string())?;
        if !before.is_file() || before.len() > MAX_SCENE as u64 {
            return Err("scene must be a regular file at most1MiB".into());
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_SCENE as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        let after = file.metadata().map_err(|e| e.to_string())?;
        LocalNavigationAdmission::for_scene(path)?;
        let named = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_SCENE
            || bytes.len() as u64 != before.len()
            || !unchanged(&before, &after)
            || !unchanged(&after, &named)
        {
            return Err("scene changed during bounded snapshot".into());
        }
        let yaml = std::str::from_utf8(&bytes).map_err(|_| "scene must be UTF-8")?;
        let doc = navigation_yard3d_doc_from_yaml(yaml, admission).map_err(|e| e.to_string())?;
        validate(doc.frame())?;
        if doc.frame().tick() != 0 {
            return Err("authored scene must start at tick0".into());
        }
        let view = navigation_from_view(FrameView::of(doc.frame()))?;
        if view.navigator.path().is_none() {
            return Err("authored scene has no admitted route".into());
        }
        let initial = doc.frame().clone();
        let simulation = Simulation::from_frame(&initial, YARD_TICK_RATE, NAVIGATION_BUILD_ID)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            initial,
            simulation,
        })
    }
    pub fn step(&mut self) -> Result<(), String> {
        validate(self.simulation.frame())?;
        let next = self
            .simulation
            .tick()
            .checked_add(1)
            .ok_or("navigation tick overflow")?;
        self.simulation.step(&TickInputs::new(next, YARD_PLAYERS));
        // An authoritative failed tick is retained, not rolled back or relabeled
        // success. Subsequent steps refuse until explicit restart.
        validate(self.simulation.frame())
    }
    pub fn restart(&mut self) -> Result<(), String> {
        validate(&self.initial)?;
        let candidate = Simulation::from_frame(&self.initial, YARD_TICK_RATE, NAVIGATION_BUILD_ID)
            .map_err(|e| e.to_string())?;
        validate(candidate.frame())?;
        self.simulation = candidate;
        Ok(())
    }
    pub fn tick(&self) -> u64 {
        self.simulation.tick()
    }
    pub fn checksum(&self) -> u64 {
        self.simulation.checksum()
    }
    pub fn frame(&self) -> &Frame {
        self.simulation.frame()
    }
    pub fn view(&self) -> Result<NavigationView, String> {
        validate(self.frame())?;
        navigation_from_view(FrameView::of(self.frame()))
    }
}
#[cfg(test)]
#[path = "authored_tests.rs"]
mod tests;
