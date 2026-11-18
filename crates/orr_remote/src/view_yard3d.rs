//! The ERP host of `ViewYard3D`: the editor's scene for entities drawn from view assets
//! (`orr_gfx::ViewRef`). It serves the same view stream and input contract as Yard3D, from a scene
//! file; the game only differs in registering `ViewRef` and running no gameplay.

use std::path::PathBuf;

use orr_edit::{EditError, EditorDoc};
use orr_reflect::TypeRegistry;
use orr_sample::view_yard3d_game::{register_reflect, ViewYard3D, TICK_RATE};
use orr_sample::yard3d_stream::Yard3dStreamProducer;
use orr_sim::Simulation;

use crate::yard3d::{YARD_BUILD_ID, YARD_PLAYERS, YARD_SEED};
use crate::{GameHooks, HostLimits, LocalHost, ServerConfig, ViewStreamHook};

/// Types registered by `ViewYard3D`, including the physics singleton's bake hook.
pub fn view_yard3d_types() -> TypeRegistry {
    let mut types = TypeRegistry::new();
    register_reflect(&mut types);
    types
}

/// Loads an authored scene with the types, bake hooks and seed of `ViewYard3D`. The supplied
/// scene is authoritative; game setup is not run.
pub fn view_yard3d_doc_from_yaml(yaml: &str) -> Result<EditorDoc, EditError> {
    EditorDoc::from_yaml(yaml, view_yard3d_types(), Simulation::<ViewYard3D>::build_registry(), YARD_SEED)
}

/// Configures the ERP host machinery for the `ViewYard3D` vocabulary.
pub fn configure_view_yard3d(limits: &mut HostLimits) {
    limits.player_count = YARD_PLAYERS;
    limits.tick_rate = TICK_RATE;
    limits.build_id = YARD_BUILD_ID;
    limits.game = GameHooks::new("ViewYard3D");
    limits.view_stream = Some(ViewStreamHook::new(Yard3dStreamProducer::new(limits.build_id, YARD_PLAYERS)));
}

/// Starts a `ViewYard3D` host from scene text. A configured `cfg.limits.scene_path` is the
/// `scene.save {write: true}` target.
pub fn spawn_view_yard3d_yaml_host(yaml: String, mut cfg: ServerConfig) -> Result<LocalHost, String> {
    LocalHost::spawn_configured::<ViewYard3D>(
        move || {
            let doc = view_yard3d_doc_from_yaml(&yaml).map_err(|e| match &cfg.limits.scene_path {
                Some(path) => format!("{}: {e}", path.display()),
                None => e.to_string(),
            })?;
            configure_view_yard3d(&mut cfg.limits);
            Ok((doc, cfg))
        },
        |server| {
            server.set_structured_input::<ViewYard3D>("YardInput", YARD_PLAYERS, |_, _| Vec::new());
            server.enable_managed_input();
        },
    )
}

/// Reads a scene file on this host and starts its local `ViewYard3D` authority. The same path
/// becomes the default save destination.
pub fn spawn_view_yard3d_scene_host(path: PathBuf, mut cfg: ServerConfig) -> Result<LocalHost, String> {
    let yaml = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    cfg.limits.scene_path = Some(path);
    spawn_view_yard3d_yaml_host(yaml, cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_scene_loads_and_the_view_ref_type_is_known() {
        let yaml = "schema: orr.scene/1\nentities:\n  e_00000001:\n    name: pawn\n    orr_gfx::Transform3: { position: [0, 0, 0], rotation: [0, 0, 0], scale: [1, 1, 1] }\n    orr_gfx::ViewRef: { asset: 5 }\n";
        let doc = view_yard3d_doc_from_yaml(yaml).unwrap();
        assert_eq!(doc.frame().alive_count(), 1);
    }
}
