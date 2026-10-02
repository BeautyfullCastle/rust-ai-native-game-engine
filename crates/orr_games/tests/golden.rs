//! Golden checksums of the sample games: fixed scripted inputs, plain `Simulation`
//! (no session, no rollback). The values were recorded on the code as it was in
//! `orr_sample` before the games moved to this crate (M6 step 4) and must be
//! bit-identical on every target (x86_64, aarch64, wasm32). Not to be changed unless
//! a simulation or checksum-format change is intended. ORRF v2 deliberately
//! rebaselines these Frame hashes to cover schema, collection boundaries and
//! list free order; scripted inputs and gameplay state are unchanged. See
//! `docs/frame-compatibility.md` for the migration boundary.

use orr_games::physics_game::{bot_input, NoCommand, PhysConfig, PhysGame, PhysInput, SceneMode};
use orr_games::yard3d_game::{NoCommand as YNo, Yard3D, YardConfig, YardInput, SHOOT, SPAWN_BALL, SPAWN_BOX};
use orr_sim::{PlayerSlot, Simulation, TickInputs};

fn phys(cfg: PhysConfig, ticks: u64) -> u64 {
    let mut sim = Simulation::<PhysGame>::new(cfg, 60, 777);
    for t in 1..=ticks {
        let mut i = TickInputs::<PhysInput, NoCommand>::new(t, 2);
        i.set_input(PlayerSlot(0), bot_input(1234, t, PlayerSlot(0)));
        i.set_input(PlayerSlot(1), bot_input(1234, t, PlayerSlot(1)));
        sim.step(&i);
    }
    sim.checksum()
}

fn yard(cfg: YardConfig, ticks: u64) -> u64 {
    let mut sim = Simulation::<Yard3D>::new(cfg, 60, 777);
    for t in 1..=ticks {
        let mut i = TickInputs::<YardInput, YNo>::new(t, 2);
        for s in 0..2u32 {
            let k = (t as i32 * 37 + s as i32 * 101) % 2000;
            let b = match (t + u64::from(s) * 7) % 40 {
                0 => SHOOT,
                10 => SPAWN_BOX,
                20 => SPAWN_BALL,
                _ => 0,
            };
            i.set_input(PlayerSlot(s as u8), YardInput { buttons: b, _pad: 0, origin: [k, 1000, 2000], dir: [-k / 4, -447, -894] });
        }
        sim.step(&i);
    }
    sim.checksum()
}

#[test]
fn golden_phys_rain120() {
    let mut c = PhysConfig::new(120, SceneMode::Rain);
    c.spawn_rate = 30;
    assert_eq!(phys(c, 300), 0xbe97_f50d_87d5_825a);
}

#[test]
fn golden_phys_pile200() {
    assert_eq!(phys(PhysConfig::new(200, SceneMode::Pile), 200), 0x2e84_7e92_dad1_9a84);
}

#[test]
fn golden_phys_mixer150() {
    assert_eq!(phys(PhysConfig::new(150, SceneMode::Mixer), 200), 0x1f2d_9339_56a9_cf6a);
}

#[test]
fn golden_yard60() {
    assert_eq!(yard(YardConfig::new(60), 300), 0x56d9_4c8b_d872_ea9d);
}

#[test]
fn golden_yard150() {
    assert_eq!(yard(YardConfig::new(150), 200), 0x2a95_9817_5036_0a21);
}
