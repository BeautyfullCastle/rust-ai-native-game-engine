use crate::{Error, Result, BUILD_ID, MOTION_ID, PLAYER_COUNT, SEED, TICKS, TICK_RATE};
use bytemuck::{Pod, Zeroable};
use orr_asset::{AssetRef, Domain, Manifest, MotionProfileV1, SimTable};
use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_fp::{FrameRng, FP};
use orr_sim::{Game, PlayerSlot, SimCommand, SimContext, Simulation, System, TickInputs};

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
pub(crate) struct Input {
    pub axis: i32,
}

#[derive(Clone)]
pub(crate) struct NoCommand;
impl SimCommand for NoCommand {
    fn encode(&self, _out: &mut Vec<u8>) {}
    fn decode(bytes: &[u8]) -> Option<Self> {
        bytes.is_empty().then_some(Self)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Pod, Zeroable)]
pub struct Impact {
    pub cue: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Pod, Zeroable)]
pub(crate) struct Motion {
    pub profile: AssetRef,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Pod, Zeroable)]
pub(crate) struct Position {
    pub x: FP,
}

pub(crate) struct AssetFixtureGame;
impl Game for AssetFixtureGame {
    type Input = Input;
    type Command = NoCommand;
    type Event = Impact;
    type Config = ();

    fn register(builder: &mut ComponentRegistryBuilder) {
        builder.register_component::<Motion>("asset_fixture_v1::Motion");
        builder.register_component::<Position>("asset_fixture_v1::Position");
    }
    fn setup(frame: &mut Frame, _config: &()) {
        let entity = frame.spawn();
        frame.add(entity, Motion { profile: MOTION_ID });
        frame.add(entity, Position { x: FP::ZERO });
    }
    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(MoveSystem {
            table: SimTable::new(&crate::generated::MOTION_PROFILES)
                .expect("reviewed static table"),
            manifest: Manifest::decode(crate::SIM_BYTES, Domain::Sim)
                .expect("reviewed static manifest"),
        })]
    }
}

struct MoveSystem {
    table: SimTable<'static, MotionProfileV1>,
    manifest: Manifest<'static>,
}
impl System<AssetFixtureGame> for MoveSystem {
    fn name(&self) -> &'static str {
        "StaticMotion"
    }
    fn run(&mut self, ctx: &mut SimContext<AssetFixtureGame>) {
        #[cfg(test)]
        TICK_CALLS.with(|count| count.set(count.get() + 1));
        let axis = ctx.inputs.input(PlayerSlot(0)).axis;
        for (_, (motion, position)) in ctx.frame.query::<(&Motion, &mut Position)>() {
            // Only the admitted immutable refs/schedule can reach this system.
            // System::run has no Result: any residual failure is a fatal invariant,
            // never a fallback asset or permission to continue a damaged session.
            let reference = self
                .manifest
                .typed_ref(motion.profile)
                .expect("admitted Motion reference");
            let profile = self
                .table
                .resolve(reference)
                .expect("admitted static profile");
            position.x += FP::from_raw(profile.speed_per_tick().raw() * i64::from(axis));
        }
        if ctx.tick == 1 || ctx.tick == 181 {
            ctx.emit(Impact { cue: 1 });
        }
    }
}

pub(crate) fn new_simulation() -> Simulation<AssetFixtureGame> {
    Simulation::with_build_id((), TICK_RATE, SEED, BUILD_ID)
}
pub(crate) fn axis(tick: u64) -> i32 {
    match tick {
        1..=120 => 1,
        121..=180 => 0,
        181..=300 => -1,
        _ => 0,
    }
}
pub(crate) fn inputs(tick: u64) -> TickInputs<Input, NoCommand> {
    let mut input = TickInputs::new(tick, PLAYER_COUNT);
    input.set_input(PlayerSlot(0), Input { axis: axis(tick) });
    input
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixtureState {
    pub tick: u64,
    pub x: FP,
    pub checksum: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CueEvent {
    pub key: orr_sim::EventKey,
    pub impact: Impact,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Step {
    pub state: FixtureState,
    pub events: Vec<CueEvent>,
}

/// Fixed-schedule live runner. No mutable Frame, arbitrary restore, game input,
/// branch, hotpatch or debug surface is exposed.
pub struct FixtureRun {
    pub(crate) sim: Simulation<AssetFixtureGame>,
}
impl FixtureRun {
    pub(crate) fn new() -> Self {
        Self {
            sim: new_simulation(),
        }
    }
    pub fn state(&self) -> FixtureState {
        state(self.sim.frame()).expect("runner preserves admitted frame invariants")
    }
    pub fn advance(&mut self) -> Result<Step> {
        let tick = self.sim.tick() + 1;
        if tick > TICKS {
            return Err(Error::TickOutOfRange(tick));
        }
        let events = self
            .sim
            .step(&inputs(tick))
            .into_iter()
            .map(|event| CueEvent {
                key: event.key,
                impact: event.payload,
            })
            .collect();
        validate_frame(self.sim.frame(), tick)?;
        Ok(Step {
            state: state(self.sim.frame())?,
            events,
        })
    }
}

pub(crate) fn validate_frame(frame: &Frame, expected_tick: u64) -> Result<()> {
    let registry = frame.registry();
    if registry.component_id::<Motion>() != Some(orr_ecs::ComponentId(0))
        || registry.component_id::<Position>() != Some(orr_ecs::ComponentId(1))
        || registry.singleton_id::<FrameRng>() != Some(orr_ecs::SingletonId(0))
        || registry.component_count() != 2
        || registry.singleton_count() != 1
        || registry.list_count() != 0
    {
        return Err(Error::Frame("registry"));
    }
    if frame.tick() != expected_tick || expected_tick > TICKS {
        return Err(Error::Frame("tick"));
    }
    if frame.alive_count() != 1 {
        return Err(Error::Frame("entity count"));
    }
    let entity = frame
        .entities()
        .next()
        .ok_or(Error::Frame("missing entity"))?;
    let motion = frame
        .get::<Motion>(entity)
        .ok_or(Error::Frame("missing Motion"))?;
    let position = frame
        .get::<Position>(entity)
        .ok_or(Error::Frame("missing Position"))?;
    if frame.dense::<Motion>().1.len() != 1 || frame.dense::<Position>().1.len() != 1 {
        return Err(Error::Frame("component count"));
    }
    let manifest = Manifest::decode(crate::SIM_BYTES, Domain::Sim)?;
    let table = SimTable::new(&crate::generated::MOTION_PROFILES)?;
    table.resolve(manifest.typed_ref(motion.profile)?)?;
    if position.x < FP::ZERO
        || position.x > FP::from_raw(120 * 4096)
        || (expected_tick == 0 && (position.x != FP::ZERO || motion.profile != MOTION_ID))
    {
        return Err(Error::Frame("coordinate or initial reference"));
    }
    Ok(())
}

pub(crate) fn state(frame: &Frame) -> Result<FixtureState> {
    let entity = frame
        .entities()
        .next()
        .ok_or(Error::Frame("missing entity"))?;
    let position = frame
        .get::<Position>(entity)
        .ok_or(Error::Frame("missing Position"))?;
    Ok(FixtureState {
        tick: frame.tick(),
        x: position.x,
        checksum: frame.checksum(),
    })
}

#[cfg(test)]
std::thread_local! { pub(crate) static TICK_CALLS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) }; }
