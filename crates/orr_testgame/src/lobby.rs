//! `Lobby`: a tiny game whose avatars come from prefabs.
//!
//! No avatar exists at the start. `Added` makes one with [`orr_ecs::spawn_prefab`]: the prefab
//! is a recipe of `Transform3`, `ViewRef` and `PlayerLink` that the game was given in its
//! [`LobbyConfig`] before the simulation started, so no file is read while it runs. The player
//! number chooses the prefab (or the start data does, see [`Profile`]). `Removed` deletes the
//! avatar again. Each avatar walks on the ground plane with the input of its player.
//!
//! The game is the test game of prefab spawning and of `ViewRef`: the avatar of each player is
//! drawn by the view layer from the view asset its prefab names.

use bytemuck::{Pod, Zeroable};
use orr_ecs::{prefab, ComponentRegistryBuilder, Entity, Frame, Prefab, PrefabId};
use orr_fp::{fp, FP};
use orr_fp::FPVec3;
use orr_gfx::{Material, Transform3, ViewRef};
use orr_sim::{fn_systems, player_signals, register_player_link, Game, PlayerLink, PlayerSignal, PlayerSlot, Players, SimContext, Simulation, System};

/// How far an avatar walks in one tick at full input: a quarter unit.
pub const STEP: FP = FP(1 << 14);

/// The per-tick input of a player: -1, 0 or 1 along each ground axis.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Pod, Zeroable)]
pub struct LobbyInput {
    /// Sideways: +1 is +x.
    pub dx: i32,
    /// Forward: +1 is +z.
    pub dz: i32,
}

/// The start data of a player (`Players::Player`).
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Pod, Zeroable)]
pub struct Profile {
    /// The avatar the player picked: the number of a prefab plus one. 0 lets the slot choose.
    pub choice: u32,
    /// Keeps the layout without padding.
    pub reserved: u32,
}

/// Event: an avatar walked.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
pub struct Walked {
    /// The player.
    pub slot: u32,
    /// Keeps the layout without padding.
    pub reserved: u32,
}

/// What `Lobby::setup` needs: the prefabs of the avatars, baked before the run
/// (`orr_reflect::Scene::bake_prefabs`, or `Prefab::with`). Prefab `i` has `PrefabId(i)`.
#[derive(Clone, Default, Debug)]
pub struct LobbyConfig {
    /// The avatar prefabs.
    pub avatars: Vec<Prefab>,
}

impl LobbyConfig {
    /// One avatar prefab for each view asset path (like `"avatars/blue.orrview"`): a `Transform3`
    /// at the origin, the `ViewRef` of the path and a `PlayerLink` that names no player yet.
    pub fn from_view_assets(paths: &[&str]) -> Self {
        let registry = Simulation::<Lobby>::build_registry();
        let avatars = paths
            .iter()
            .map(|path| {
                Prefab::new()
                    .with(&registry, Transform3::default())
                    .and_then(|prefab| prefab.with(&registry, ViewRef::at(path)))
                    .and_then(|prefab| prefab.with(&registry, PlayerLink { player: PlayerSlot::NONE }))
                    .expect("the lobby registers these components")
            })
            .collect();
        Self { avatars }
    }
}

/// The lobby game.
pub struct Lobby;

impl Game for Lobby {
    type Input = LobbyInput;
    type Command = ();
    type Event = Walked;
    type Config = LobbyConfig;

    fn register(builder: &mut ComponentRegistryBuilder) {
        builder.register_component::<Transform3>(orr_gfx::names::TRANSFORM);
        builder.register_component::<Material>(orr_gfx::names::MATERIAL);
        orr_gfx::register_view_ref(builder);
        register_player_link(builder);
        prefab::register(builder);
    }

    fn setup(frame: &mut Frame, config: &LobbyConfig) {
        prefab::install(frame, &config.avatars).expect("the lobby registers prefabs in Game::register");
        // A floor to stand on: a slab 30 units wide, its top at height 0.
        let floor = frame.spawn();
        frame.add(floor, Transform3 { position: FPVec3::new(FP::ZERO, fp!(-0.25), FP::ZERO), scale: FPVec3::new(fp!(30), fp!(0.5), fp!(30)), ..Default::default() });
        frame.add(floor, Material::new("floor"));
    }

    fn systems() -> Vec<Box<dyn System<Self>>> {
        let mut systems = vec![player_signals::<Self>()];
        systems.extend(fn_systems([walk]));
        systems
    }
}

impl Players for Lobby {
    type Player = Profile;

    fn on_player(frame: &mut Frame, signal: PlayerSignal<Profile>) {
        match signal {
            PlayerSignal::Added { slot, data } => {
                // A player that comes twice keeps the one avatar.
                if avatar_of(frame, slot).is_some() {
                    return;
                }
                let count = prefab::prefab_count(frame);
                if count == 0 {
                    return;
                }
                let pick = if data.choice != 0 { data.choice - 1 } else { u32::from(slot.0) } % count;
                let Some(entity) = orr_ecs::spawn_prefab(frame, PrefabId(pick)) else { return };
                if let Some(link) = frame.get_mut::<PlayerLink>(entity) {
                    link.player = slot;
                }
                if let Some(placed) = frame.get_mut::<Transform3>(entity) {
                    placed.position.x = fp!(3) * FP::from_int(i32::from(slot.0)) - fp!(1.5);
                }
            }
            PlayerSignal::Removed { slot } => {
                if let Some(entity) = avatar_of(frame, slot) {
                    frame.despawn(entity);
                }
            }
            PlayerSignal::Disconnected { .. } | PlayerSignal::Reconnected { .. } => {}
        }
    }
}

/// The avatar of a player: the entity whose `PlayerLink` names the slot.
pub fn avatar_of(frame: &mut Frame, slot: PlayerSlot) -> Option<Entity> {
    frame.query::<(&PlayerLink,)>().find(|(_, (link,))| link.player == slot).map(|(entity, _)| entity)
}

fn walk(ctx: &mut SimContext<Lobby>) {
    for number in 0..ctx.inputs.player_count() {
        let slot = PlayerSlot(number);
        let input = ctx.inputs.input(slot);
        let (dx, dz) = (input.dx, input.dz);
        if dx == 0 && dz == 0 {
            continue;
        }
        let Some(entity) = avatar_of(ctx.frame, slot) else { continue };
        let Some(placed) = ctx.frame.get_mut::<Transform3>(entity) else { continue };
        placed.position.x += STEP * FP::from_int(dx.clamp(-1, 1));
        placed.position.z += STEP * FP::from_int(dz.clamp(-1, 1));
        ctx.emit(Walked { slot: u32::from(number), reserved: 0 });
    }
}
