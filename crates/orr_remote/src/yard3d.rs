//! The first Yard3D ERP path: a typed, nonempty document and a local host
//! serving the existing 3D view stream over WebSocket or TCP.
//!
//! This factory does not create a relay/client session. Local seek replays a
//! document's play history; it is not late-input network rollback.

use orr_edit::{EditError, EditorDoc};
use orr_reflect::{Scene, TypeRegistry};
use orr_sample::yard3d_game::{register_reflect, Yard3D, YardConfig};
use orr_sample::yard3d_stream::Yard3dStreamProducer;
use orr_sim::Simulation;

use crate::{GameHooks, HostLimits, LocalHost, ServerConfig, ViewStreamHook};

pub use orr_sample::yard3d_game::TICK_RATE as YARD_TICK_RATE;

/// Stable preview/play seed for the local Yard3D authoring host.
pub const YARD_SEED: u64 = 42;
/// The first ERP host uses the same two player slots as the Yard bridge fixture.
pub const YARD_PLAYERS: u8 = 2;
/// The existing document play controller uses local build id zero. The stream
/// schema and replay identity must name that actual session.
pub const YARD_BUILD_ID: u64 = 0;

/// Types registered by Yard3D, including the physics singleton's bake hook.
pub fn yard3d_types() -> TypeRegistry {
    let mut types = TypeRegistry::new();
    register_reflect(&mut types);
    types
}

/// Builds a nonempty document from the game's deterministic setup.
///
/// `EditorDoc::from_scene` bakes the reflected scene and initializes hidden
/// physics storage. Merely starting an empty document would not run setup.
pub fn yard3d_doc(config: YardConfig) -> Result<EditorDoc, EditError> {
    let initial = Simulation::<Yard3D>::new(config, YARD_TICK_RATE, YARD_SEED);
    let types = yard3d_types();
    let scene = Scene::unbake(&types, initial.frame(), None)?;
    EditorDoc::from_scene(
        scene,
        types,
        Simulation::<Yard3D>::build_registry(),
        YARD_SEED,
    )
}

/// Configures the existing ERP host machinery for the Yard3D vocabulary.
pub fn configure_yard3d(limits: &mut HostLimits) {
    limits.player_count = YARD_PLAYERS;
    limits.tick_rate = YARD_TICK_RATE;
    limits.build_id = YARD_BUILD_ID;
    limits.game = GameHooks::new("Yard3D");
    limits.view_stream = Some(ViewStreamHook::new(Yard3dStreamProducer::new(
        limits.build_id,
        YARD_PLAYERS,
    )));
}

/// Starts an actual ERP host with the existing raw `sim.input` contract.
/// WebSocket binary and TCP hex view notifications share the same producer.
pub fn spawn_yard3d_host(config: YardConfig, mut cfg: ServerConfig) -> Result<LocalHost, String> {
    LocalHost::spawn::<Yard3D>(move || {
        let doc = yard3d_doc(config).map_err(|e| e.to_string())?;
        configure_yard3d(&mut cfg.limits);
        Ok((doc, cfg))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_sim::TickInputs;

    #[test]
    fn bootstrap_is_nonempty_deterministic_and_can_step_after_bake() {
        let mut cfg = YardConfig::new(4);
        cfg.rain_per_second = 0;
        cfg.max_entities = 128;
        let a = yard3d_doc(cfg).unwrap();
        let b = yard3d_doc(cfg).unwrap();
        assert!(
            a.frame().alive_count() > 4,
            "setup includes the yard's fixtures"
        );
        assert_eq!(a.frame().to_bytes(), b.frame().to_bytes());
        assert_eq!(a.checksum(), b.checksum());
        let mut sim =
            Simulation::<Yard3D>::from_frame(a.frame(), YARD_TICK_RATE, YARD_BUILD_ID).unwrap();
        for _ in 0..3 {
            sim.step(&TickInputs::new(sim.tick() + 1, YARD_PLAYERS));
        }
        assert_eq!(sim.tick(), 3);
        assert!(sim.frame().alive_count() > 0);
    }
}
