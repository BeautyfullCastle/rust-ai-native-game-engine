//! A long Eastern dragon that a player flies over a scene. The flight runs in
//! the simulation, in fixed point, so it is the same on every peer and in a
//! replay. The head leaves a trail of points; the view lays the serpentine
//! body along it.
//!
//! - [`Dragon`]: the dragon's head, heading, speeds and trail.
//! - [`DragonCamera`]: places its entity's `orr_gfx::Transform3` behind a
//!   dragon, so a `orr_gfx::Camera` on the same entity follows it.
//! - [`DragonGround`]: a coarse height grid of the scene's ground, so the
//!   dragon flies over hills instead of through them.
//!
//! Input is the player's [`crate::yard3d_game::YardInput::buttons`] with the
//! `DRAGON_*` bits.

use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistryBuilder, Entity, Frame};
use orr_fp::{FPVec3, FP};
use orr_reflect::{Reflect, TypeRegistry};

/// Input bit: turn left.
pub const DRAGON_LEFT: u32 = 16;
/// Input bit: turn right.
pub const DRAGON_RIGHT: u32 = 32;
/// Input bit: climb.
pub const DRAGON_CLIMB: u32 = 64;
/// Input bit: dive.
pub const DRAGON_DIVE: u32 = 128;
/// Input bit: fly faster.
pub const DRAGON_BOOST: u32 = 256;

/// Points of the head's trail.
pub const DRAGON_TRAIL: usize = 48;
/// A trail point is kept every this many ticks.
pub const TRAIL_EVERY: u64 = 3;
/// Points per side of [`DragonGround`].
pub const GROUND_SIDE: usize = 32;

/// Scene names of the types.
pub mod names {
    pub const DRAGON: &str = "orr_games::Dragon";
    pub const DRAGON_CAMERA: &str = "orr_games::DragonCamera";
    pub const DRAGON_GROUND: &str = "orr_games::DragonGround";
}

/// A dragon. `position` is the head; the body follows `trail`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Reflect)]
#[reflect(default)]
pub struct Dragon {
    /// World position of the head.
    #[reflect(range = "-30000..=30000")]
    pub position: FPVec3,
    /// Heading in degrees around the vertical axis; 0 flies along +z.
    #[reflect(range = "-100000..=100000")]
    pub yaw: FP,
    /// Degrees above the horizon.
    #[reflect(range = "-90..=90")]
    pub pitch: FP,
    /// Roll into a turn, degrees.
    #[reflect(range = "-90..=90")]
    pub bank: FP,
    /// Current speed, meters per second.
    #[reflect(range = "0..=500")]
    pub speed: FP,
    /// Speed without boost and with it.
    #[reflect(range = "0..=500")]
    pub cruise_speed: FP,
    #[reflect(range = "0..=500")]
    pub boost_speed: FP,
    /// Degrees per second of turning and of climbing or diving.
    #[reflect(range = "0..=360")]
    pub turn_rate: FP,
    #[reflect(range = "0..=360")]
    pub climb_rate: FP,
    /// Least height above the ground, and the highest the dragon flies.
    #[reflect(range = "0..=1000")]
    pub clearance: FP,
    #[reflect(range = "-10000..=30000")]
    pub ceiling: FP,
    /// Half size of the square the dragon stays in, around the origin.
    #[reflect(range = "10..=30000")]
    pub bounds: FP,
    /// The player who flies it.
    #[reflect(range = "0..=7")]
    pub player: u32,
    /// Slot of `trail` the next point goes to.
    #[reflect(range = "0..=47")]
    pub next: u32,
    /// Where the head was, a point every [`TRAIL_EVERY`] ticks; `next - 1` is the newest.
    pub trail: [FPVec3; DRAGON_TRAIL],
}

impl Default for Dragon {
    fn default() -> Self {
        Self {
            position: FPVec3::ZERO,
            yaw: FP::ZERO,
            pitch: FP::ZERO,
            bank: FP::ZERO,
            speed: FP::from_int(20),
            cruise_speed: FP::from_int(20),
            boost_speed: FP::from_int(45),
            turn_rate: FP::from_int(70),
            climb_rate: FP::from_int(45),
            clearance: FP::from_int(14),
            ceiling: FP::from_int(600),
            bounds: FP::from_int(480),
            player: 0,
            next: 0,
            trail: [FPVec3::ZERO; DRAGON_TRAIL],
        }
    }
}

/// Puts this entity's `orr_gfx::Transform3` behind and above `target`'s head, looking at it.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Reflect)]
#[reflect(default)]
pub struct DragonCamera {
    /// The dragon to follow.
    pub target: Entity,
    /// Meters behind and above the head.
    #[reflect(range = "0..=1000")]
    pub distance: FP,
    #[reflect(range = "-1000..=1000")]
    pub height: FP,
}

impl Default for DragonCamera {
    fn default() -> Self {
        Self { target: Entity::NONE, distance: FP::from_int(34), height: FP::from_int(9) }
    }
}

/// Ground heights on a square grid of [`GROUND_SIDE`] points per side,
/// centered on `center`, row by row from the -z edge. A scene writes it from
/// its terrain's heightmap; without it the ground is at height zero.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Reflect)]
#[reflect(default)]
pub struct DragonGround {
    /// World position of the grid's center, on the x and z axes.
    #[reflect(range = "-30000..=30000")]
    pub center: orr_fp::FPVec2,
    /// Side of the grid in meters.
    #[reflect(range = "1..=60000")]
    pub size: FP,
    #[reflect(range = "-10000..=10000")]
    pub heights: [FP; GROUND_SIDE * GROUND_SIDE],
}

impl Default for DragonGround {
    fn default() -> Self {
        Self { center: orr_fp::FPVec2::ZERO, size: FP::from_int(1024), heights: [FP::ZERO; GROUND_SIDE * GROUND_SIDE] }
    }
}

impl DragonGround {
    /// Bilinear ground height at `(x, z)`; the edge values outside the grid.
    pub fn height(&self, x: FP, z: FP) -> FP {
        let last = FP::from_int(GROUND_SIDE as i32 - 1);
        let cell = |v: FP, c: FP| ((v - c) / self.size + FP::HALF).clamp(FP::ZERO, FP::ONE) * last;
        let (gx, gz) = (cell(x, self.center.x), cell(z, self.center.y));
        let (i, j) = (gx.floor().to_int().clamp(0, GROUND_SIDE as i32 - 1) as usize, gz.floor().to_int().clamp(0, GROUND_SIDE as i32 - 1) as usize);
        let (i1, j1) = ((i + 1).min(GROUND_SIDE - 1), (j + 1).min(GROUND_SIDE - 1));
        let (tx, tz) = (gx - gx.floor(), gz - gz.floor());
        let at = |i: usize, j: usize| self.heights[j * GROUND_SIDE + i];
        let near = at(i, j) + (at(i1, j) - at(i, j)) * tx;
        let far = at(i, j1) + (at(i1, j1) - at(i, j1)) * tx;
        near + (far - near) * tz
    }
}

/// Registers the types with a game's simulation.
pub fn register(builder: &mut ComponentRegistryBuilder) {
    builder.register_component::<Dragon>(names::DRAGON);
    builder.register_component::<DragonCamera>(names::DRAGON_CAMERA);
    builder.register_singleton::<DragonGround>(names::DRAGON_GROUND);
}

fn init_ground(frame: &mut Frame, present: bool) {
    if !present {
        frame.set_singleton(DragonGround::default());
    }
}

/// Registers the types for scene files, the inspector and the `orr` command.
pub fn register_reflect(types: &mut TypeRegistry) {
    types.register_component::<Dragon>(names::DRAGON);
    types.register_component::<DragonCamera>(names::DRAGON_CAMERA);
    types.register_singleton_with_init::<DragonGround>(names::DRAGON_GROUND, init_ground);
}

/// Gives the ground its default in a new frame.
pub fn init(frame: &mut Frame) {
    frame.set_singleton(DragonGround::default());
}

fn radians(degrees: FP) -> FP {
    degrees * FP::PI / FP::from_int(180)
}

/// Unit direction of flight for a heading and a pitch in degrees.
pub fn forward(yaw: FP, pitch: FP) -> FPVec3 {
    let (sy, cy) = radians(yaw).sin_cos();
    let (sp, cp) = radians(pitch).sin_cos();
    FPVec3::new(cp * sy, sp, cp * cy)
}

/// `value` moved toward `target` by at most `step`.
fn approach(value: FP, target: FP, step: FP) -> FP {
    if value < target {
        (value + step).min(target)
    } else {
        (value - step).max(target)
    }
}

/// One tick of flight for `dragon` with the player's `buttons`, `dt` seconds long.
pub fn fly(dragon: &mut Dragon, buttons: u32, ground: &DragonGround, dt: FP, tick: u64) {
    let steer = FP::from_int(i32::from(buttons & DRAGON_RIGHT != 0) - i32::from(buttons & DRAGON_LEFT != 0));
    let climb = FP::from_int(i32::from(buttons & DRAGON_CLIMB != 0) - i32::from(buttons & DRAGON_DIVE != 0));
    dragon.yaw = (dragon.yaw + steer * dragon.turn_rate * dt).clamp(FP::from_int(-100_000), FP::from_int(100_000));
    // Bank into the turn and level out after it; pitch eases toward the wanted climb.
    dragon.bank = approach(dragon.bank, steer * FP::from_int(35), FP::from_int(90) * dt);
    dragon.pitch = approach(dragon.pitch, climb * FP::from_int(32), dragon.climb_rate * dt).clamp(FP::from_int(-60), FP::from_int(60));
    let wanted = if buttons & DRAGON_BOOST != 0 { dragon.boost_speed } else { dragon.cruise_speed };
    dragon.speed = approach(dragon.speed, wanted, FP::from_int(25) * dt).max(FP::ZERO);
    let mut p = dragon.position + forward(dragon.yaw, dragon.pitch) * (dragon.speed * dt);
    // Stay inside the square and between the ground and the ceiling.
    p.x = p.x.clamp(-dragon.bounds, dragon.bounds);
    p.z = p.z.clamp(-dragon.bounds, dragon.bounds);
    let floor = ground.height(p.x, p.z) + dragon.clearance;
    if p.y < floor {
        p.y = floor;
        dragon.pitch = dragon.pitch.max(FP::ZERO);
    }
    p.y = p.y.min(dragon.ceiling.max(floor));
    dragon.position = p;
    if tick % TRAIL_EVERY == 0 {
        let slot = (dragon.next as usize) % DRAGON_TRAIL;
        dragon.trail[slot] = p;
        dragon.next = ((slot + 1) % DRAGON_TRAIL) as u32;
    }
}

/// A trail that has never been written starts as a straight line behind the head.
pub fn settle_trail(dragon: &mut Dragon) {
    if dragon.trail.iter().all(|p| *p == FPVec3::ZERO) {
        let back = forward(dragon.yaw, FP::ZERO) * FP::from_ratio(-125, 100);
        for k in 0..DRAGON_TRAIL {
            // The oldest point is the farthest back.
            let age = FP::from_int((DRAGON_TRAIL - k) as i32);
            dragon.trail[k] = dragon.position + back * age;
        }
        dragon.next = 0;
    }
}

/// Where a [`DragonCamera`] sits for `dragon`: its position and the
/// `orr_gfx::Transform3` rotation (pitch, yaw, roll in degrees) that looks at the head.
pub fn camera_place(dragon: &Dragon, camera: &DragonCamera, ground: &DragonGround) -> (FPVec3, FPVec3) {
    let flat = forward(dragon.yaw, FP::ZERO);
    let mut eye = dragon.position - flat * camera.distance + FPVec3::new(FP::ZERO, camera.height, FP::ZERO);
    eye.y = eye.y.max(ground.height(eye.x, eye.z) + FP::from_int(3));
    let d = dragon.position - eye;
    let flat_length = (d.x * d.x + d.z * d.z).sqrt();
    let to_degrees = |r: FP| r * FP::from_int(180) / FP::PI;
    // Transform3: positive pitch looks down.
    let pitch = to_degrees((-d.y).atan2(flat_length));
    let yaw = to_degrees(d.x.atan2(d.z));
    (eye, FPVec3::new(pitch, yaw, FP::ZERO))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt() -> FP {
        FP::ONE / FP::from_int(60)
    }

    #[test]
    fn a_dragon_flies_ahead_turns_climbs_and_keeps_above_the_ground() {
        let ground = DragonGround { heights: [FP::from_int(50); GROUND_SIDE * GROUND_SIDE], ..DragonGround::default() };
        let mut dragon = Dragon { position: FPVec3::new(FP::ZERO, FP::from_int(100), FP::ZERO), ..Dragon::default() };
        settle_trail(&mut dragon);
        for tick in 0..60 {
            fly(&mut dragon, 0, &ground, dt(), tick);
        }
        // One second at cruise speed along +z.
        assert!((dragon.position.z - FP::from_int(20)).abs() < FP::from_ratio(1, 10), "{:?}", dragon.position);
        assert_eq!(dragon.trail.iter().filter(|p| p.z > FP::ZERO).count(), 20, "a trail point every three ticks");

        for tick in 60..120 {
            fly(&mut dragon, DRAGON_RIGHT | DRAGON_CLIMB | DRAGON_BOOST, &ground, dt(), tick);
        }
        assert!(dragon.yaw > FP::from_int(60) && dragon.bank > FP::ZERO && dragon.pitch > FP::from_int(20));
        assert!(dragon.speed > FP::from_int(40) && dragon.position.x > FP::ZERO);

        // Diving for long ends at the ground plus the clearance, never below.
        for tick in 120..900 {
            fly(&mut dragon, DRAGON_DIVE, &ground, dt(), tick);
        }
        assert_eq!(dragon.position.y, FP::from_int(64));
    }

    #[test]
    fn the_same_input_always_gives_the_same_flight() {
        let run = || {
            let mut dragon = Dragon { position: FPVec3::new(FP::ZERO, FP::from_int(80), FP::ZERO), ..Dragon::default() };
            settle_trail(&mut dragon);
            for tick in 0..600 {
                let buttons = if tick % 90 < 45 { DRAGON_LEFT | DRAGON_CLIMB } else { DRAGON_RIGHT | DRAGON_DIVE };
                fly(&mut dragon, buttons, &DragonGround::default(), FP::ONE / FP::from_int(60), tick);
            }
            dragon
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn the_ground_grid_is_sampled_between_its_points() {
        let mut ground = DragonGround { size: FP::from_int(310), ..DragonGround::default() };
        // A ramp rising 1 m per grid column.
        for j in 0..GROUND_SIDE {
            for i in 0..GROUND_SIDE {
                ground.heights[j * GROUND_SIDE + i] = FP::from_int(i as i32);
            }
        }
        // Columns are 10 m apart; x = -150 is column 0.5.
        assert!((ground.height(FP::from_int(-150), FP::ZERO) - FP::HALF).abs() < FP::from_ratio(1, 100));
        assert_eq!(ground.height(FP::from_int(9999), FP::ZERO), FP::from_int(31), "outside the grid the edge holds");
    }

    #[test]
    fn the_camera_sits_behind_and_above_the_head_and_looks_at_it() {
        let dragon = Dragon { position: FPVec3::new(FP::ZERO, FP::from_int(100), FP::ZERO), ..Dragon::default() };
        let (eye, rotation) = camera_place(&dragon, &DragonCamera::default(), &DragonGround::default());
        assert_eq!((eye.x, eye.y, eye.z), (FP::ZERO, FP::from_int(109), FP::from_int(-34)));
        assert!(rotation.x > FP::ZERO, "it looks down at the head");
        assert!(rotation.y.abs() < FP::from_ratio(1, 100), "it looks along +z");
    }
}
