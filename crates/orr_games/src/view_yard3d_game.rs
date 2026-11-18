//! `ViewYard3D`: an empty 3D yard for placing entities that are drawn from view assets.
//!
//! It is the editor's scene for `orr_gfx::ViewRef`. The game registers what a Yard3D scene file
//! can hold (3D physics, joints, the `orr_gfx` components) and `ViewRef`, and runs the physics
//! system only: no player input, no rain, no dragon. `Yard3D` itself does not register `ViewRef`,
//! because that would change its frame layout and so its checksums.
//!
//! This is sim code: no floats, no wall clock, no hash maps.
#![deny(clippy::float_arithmetic)]
#![deny(clippy::disallowed_types)]

use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_physics3d::{init, register, PhysicsConfig, PhysicsSystem};
use orr_reflect::TypeRegistry;
use orr_sim::{Game, System};

pub use crate::yard3d_game::{NoCommand, NoEvent, YardInput, TICK_RATE};

/// The game of the editor's `ViewRef` scenes.
pub struct ViewYard3D;

/// Registers the types of a `ViewYard3D` scene file: physics, joints, the `orr_gfx` components
/// (lights, render layers, environment, post-processing) and `ViewRef`.
pub fn register_reflect(types: &mut TypeRegistry) {
    orr_physics3d::register_reflect(types);
    orr_physics3d::register_joints_reflect(types);
    orr_gfx::register_reflect(types);
    orr_gfx::register_view_ref_reflect(types);
}

impl Game for ViewYard3D {
    type Input = YardInput;
    type Command = NoCommand;
    type Event = NoEvent;
    type Config = ();

    fn register(builder: &mut ComponentRegistryBuilder) {
        register(builder);
        orr_physics3d::register_joints(builder);
        orr_gfx::register(builder);
        orr_gfx::register_view_ref(builder);
    }

    fn setup(frame: &mut Frame, _: &()) {
        init(frame, PhysicsConfig::default());
        orr_gfx::init(frame);
    }

    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(PhysicsSystem::<ViewYard3D>::new())]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yard3d_game::Yard3D;
    use orr_gfx::ViewRef;
    use orr_sim::{Simulation, TickInputs};

    #[test]
    fn the_game_registers_view_ref_and_yard3d_still_does_not() {
        assert!(Simulation::<ViewYard3D>::build_registry().component_id::<ViewRef>().is_some());
        assert!(Simulation::<Yard3D>::build_registry().component_id::<ViewRef>().is_none(), "Yard3D keeps its frame layout");
    }

    #[test]
    fn it_steps_with_two_idle_players() {
        let mut sim = Simulation::<ViewYard3D>::new((), TICK_RATE, 1);
        let before = sim.checksum();
        sim.step(&TickInputs::<YardInput, NoCommand>::new(1, 2));
        assert_ne!(before, sim.checksum(), "the tick advances");
    }
}
