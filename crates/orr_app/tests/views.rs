//! Entity views and scene views against real bridges: a loopback pair of two prediction
//! sessions (real rollbacks) and an offline play session. No window, no renderer.
// View layer test: the view world takes seconds as f32.
#![allow(clippy::float_arithmetic)]
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::time::Duration;

use bytemuck::{Pod, Zeroable};
use orr_app::{
    EntityView, EventState, Extracted, Extracted3, Extractor, Extractor3, FrameView, InterpMode, Lifecycle, SceneView, Shape3, Style3, Transform2, Transform3, ViewContext, Views,
};
use orr_bridge::{
    Bridge, BridgeConfig, BridgeEvent, EventKey, EventStatus, InProc, LoopbackPair, PlayConfig, PlayHost, PlaySession, PlayerSlot, ViewUpdate,
};
use orr_session::SessionConfig;
use orr_sim::{ComponentRegistryBuilder, Entity, Frame, Game, SimContext, System};
use orr_view::{Quat, Shape, Style, Vec2, Vec3, ViewConfig, ViewWorld, ViewWorld3};

const ONE_TICK: Duration = Duration::from_nanos(16_666_667);

// ---- a small game: holding the trigger spawns a spark and emits a flash ----

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, Pod, Zeroable)]
struct Trigger {
    held: u32,
    _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
struct Spark {
    owner: u32,
    born: u32,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
struct Flash {
    owner: u32,
    tick: u32,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
struct Rules {
    players: u32,
    /// Ticks a spark lives.
    ttl: u32,
}

struct BurstConfig {
    players: u32,
    ttl: u32,
}

struct Burst;

impl Game for Burst {
    type Input = Trigger;
    type Command = ();
    type Event = Flash;
    type Config = BurstConfig;

    fn register(builder: &mut ComponentRegistryBuilder) {
        builder.register_component::<Spark>("Spark");
        builder.register_singleton::<Rules>("Rules");
    }

    fn setup(frame: &mut Frame, config: &BurstConfig) {
        frame.set_singleton(Rules { players: config.players, ttl: config.ttl });
    }

    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(Sparks)]
    }
}

struct Sparks;

impl System<Burst> for Sparks {
    fn name(&self) -> &'static str {
        "Sparks"
    }

    fn run(&mut self, ctx: &mut SimContext<Burst>) {
        let rules = *ctx.frame.singleton::<Rules>();
        let tick = ctx.tick as u32;
        let expired: Vec<Entity> = {
            let (entities, sparks) = ctx.frame.dense::<Spark>();
            entities.iter().zip(sparks).filter(|(_, s)| tick - s.born >= rules.ttl).map(|(e, _)| *e).collect()
        };
        for entity in expired {
            ctx.frame.despawn(entity);
        }
        for owner in 0..rules.players {
            if ctx.inputs.input(PlayerSlot(owner as u8)).held != 0 {
                let entity = ctx.frame.spawn();
                ctx.frame.add(entity, Spark { owner, born: tick });
                ctx.emit(Flash { owner, tick });
            }
        }
    }
}

/// Draws every spark at `x = owner`, so a pose can be checked against its entity.
struct SparkLook;

impl Extractor for SparkLook {
    fn extract(&self, frame: FrameView<'_>, out: &mut Vec<Extracted>) {
        for (entity, spark) in frame.iter::<Spark>() {
            out.push(Extracted {
                entity,
                transform: Transform2::new(Vec2::new(spark.owner as f32, 0.0), 0.0),
                mode: InterpMode::Prediction,
                style: Style { shape: Shape::Circle, size: 1.0, half_y: 0.0, color: [1.0; 4] },
            });
        }
    }
}

// ---- what the views saw ----

#[derive(Clone, Copy, Debug, PartialEq)]
enum Call {
    Create { entity: Entity, spark: Spark, tick: u64, frame: u32 },
    Update { entity: Entity, pose: Option<Transform2> },
    Destroy { entity: Entity, tick: u64, frame: u32 },
}

#[derive(Default)]
struct Log {
    /// The test loop's frame counter (one per `dispatch`).
    frame: Cell<u32>,
    entity_calls: RefCell<Vec<Call>>,
    flashes: RefCell<Vec<(Flash, EventState, u32)>>,
    notes: RefCell<Vec<(Lifecycle, u32)>>,
    /// The frames in which a scene view was told about a resync.
    resyncs: RefCell<Vec<u32>>,
    scene_calls: RefCell<Vec<String>>,
}

struct SparkView {
    log: Rc<Log>,
    spark: Option<Spark>,
}

impl EntityView<Burst> for SparkView {
    fn on_create(&mut self, ctx: &mut ViewContext<'_, Burst>, entity: Entity) {
        let spark = *ctx.predicted().get::<Spark>(entity).expect("on_create: the entity is in the predicted frame");
        self.spark = Some(spark);
        self.log.entity_calls.borrow_mut().push(Call::Create { entity, spark, tick: ctx.tick(), frame: self.log.frame.get() });
    }

    fn on_update(&mut self, ctx: &mut ViewContext<'_, Burst>, entity: Entity, pose: Option<Transform2>) {
        assert!(ctx.predicted().has::<Spark>(entity), "on_update: the entity is in the predicted frame");
        self.log.entity_calls.borrow_mut().push(Call::Update { entity, pose });
    }

    fn on_destroy(&mut self, ctx: &mut ViewContext<'_, Burst>, entity: Entity) {
        assert!(!ctx.predicted().exists(entity) || !ctx.predicted().has::<Spark>(entity), "on_destroy: the entity left the predicted frame");
        assert!(self.spark.is_some(), "on_destroy comes after on_create");
        self.log.entity_calls.borrow_mut().push(Call::Destroy { entity, tick: ctx.tick(), frame: self.log.frame.get() });
    }
}

struct Recorder {
    name: &'static str,
    log: Rc<Log>,
}

impl SceneView<Burst> for Recorder {
    fn on_start(&mut self, ctx: &mut ViewContext<'_, Burst>) {
        assert!(ctx.is_local(PlayerSlot(0)));
        self.log.scene_calls.borrow_mut().push(format!("{}:start", self.name));
    }

    fn on_update(&mut self, _ctx: &mut ViewContext<'_, Burst>) {
        self.log.scene_calls.borrow_mut().push(format!("{}:update", self.name));
    }

    fn on_event(&mut self, _ctx: &mut ViewContext<'_, Burst>, event: &Flash, state: EventState) {
        self.log.flashes.borrow_mut().push((*event, state, self.log.frame.get()));
        self.log.scene_calls.borrow_mut().push(format!("{}:event", self.name));
    }

    fn on_session(&mut self, _ctx: &mut ViewContext<'_, Burst>, note: Lifecycle) {
        self.log.notes.borrow_mut().push((note, self.log.frame.get()));
        self.log.scene_calls.borrow_mut().push(format!("{}:session", self.name));
    }

    fn on_resync(&mut self, _ctx: &mut ViewContext<'_, Burst>) {
        self.log.resyncs.borrow_mut().push(self.log.frame.get());
        self.log.scene_calls.borrow_mut().push(format!("{}:resync", self.name));
    }
}

fn views(log: &Rc<Log>) -> Views<Burst> {
    let mut views = Views::<Burst>::new();
    let for_entities = log.clone();
    views.entity_view::<Spark>(move || SparkView { log: for_entities.clone(), spark: None });
    views.scene_view(Recorder { name: "scene", log: log.clone() });
    views
}

/// One window frame without the window: update, poll, view world, dispatch.
fn frame<B: Bridge<Burst>>(bridge: &mut B, world: &mut ViewWorld<SparkLook>, views: &mut Views<Burst>, log: &Log) {
    bridge.update(ONE_TICK);
    let update = bridge.poll_view();
    world.update_from_bridge(1.0 / 60.0, &update);
    let mut poses = Vec::new();
    world.render_items(&mut poses);
    log.frame.set(log.frame.get() + 1);
    views.dispatch(bridge, &update, &poses);
}

fn world() -> ViewWorld<SparkLook> {
    ViewWorld::new(SparkLook, ViewConfig::default())
}

/// Two prediction sessions that talk over a delayed link. Player 1 (remote) holds the
/// trigger for ticks 20..30 and the local player never fires. The local session repeats
/// the last input it knows of player 1, so it guesses wrong at the start and at the end.
fn rolling_back_bridge() -> InProc<Burst, LoopbackPair<Burst>> {
    let pair = LoopbackPair::new(
        || BurstConfig { players: 2, ttl: 1000 },
        SessionConfig::new(2, PlayerSlot(0), 42, 60),
        SessionConfig::new(2, PlayerSlot(1), 42, 60),
        4,
        0,
        777,
        |tick| (Trigger { held: u32::from((20..30).contains(&tick)), _pad: 0 }, Vec::new()),
    );
    InProc::new(pair, BridgeConfig::default())
}

fn entity_calls(log: &Log) -> Vec<Call> {
    log.entity_calls.borrow().clone()
}

#[test]
fn a_rollback_destroys_the_entities_only_the_prediction_had_and_creates_the_ones_it_missed() {
    let log = Rc::new(Log::default());
    let (mut views, mut world) = (views(&log), world());
    let mut bridge = rolling_back_bridge();
    for _ in 0..120 {
        frame(&mut bridge, &mut world, &mut views, &log);
    }
    let snapshot = bridge.snapshot().unwrap();
    assert!(snapshot.stats().rollbacks >= 2, "the guess was wrong twice (start and end of the hold)");
    let truth: BTreeSet<(u32, u32)> = snapshot.verified().unwrap().iter::<Spark>().map(|(_, s)| (s.owner, s.born)).collect();
    assert!(snapshot.verified_tick() > 35, "the whole hold is verified");
    assert_eq!(truth, (20..30).map(|t| (1, t)).collect(), "the verified sim has the ten real sparks and nothing else");

    let calls = entity_calls(&log);
    let mut created: BTreeMap<Entity, (Spark, u32)> = BTreeMap::new();
    let mut destroyed: BTreeMap<Entity, u32> = BTreeMap::new();
    for call in &calls {
        match *call {
            Call::Create { entity, spark, frame, .. } => {
                assert!(created.insert(entity, (spark, frame)).is_none(), "on_create twice for {entity:?}");
            }
            Call::Destroy { entity, frame, tick } => {
                let (_, made) = created.get(&entity).unwrap_or_else(|| panic!("on_destroy without on_create for {entity:?}"));
                assert!(frame >= *made && tick > 0);
                assert!(destroyed.insert(entity, frame).is_none(), "on_destroy twice for {entity:?}");
            }
            Call::Update { entity, .. } => {
                assert!(created.contains_key(&entity) && !destroyed.contains_key(&entity), "on_update outside the life of {entity:?}");
            }
        }
    }

    // Entities of the guess: created, then destroyed in a frame that also had a Rollback note.
    let phantoms: Vec<Entity> = created.iter().filter(|(_, (s, _))| !truth.contains(&(s.owner, s.born))).map(|(e, _)| *e).collect();
    assert!(!phantoms.is_empty(), "the test must make the prediction create entities that are not real");
    let rollback_frames: BTreeSet<u32> =
        log.notes.borrow().iter().filter(|(n, _)| matches!(n, Lifecycle::Rollback(_))).map(|(_, f)| *f).collect();
    for entity in &phantoms {
        let gone = destroyed.get(entity).unwrap_or_else(|| panic!("the predicted-only {entity:?} was never destroyed"));
        assert!(rollback_frames.contains(gone), "{entity:?} was destroyed in frame {gone} without a rollback in it");
        assert!(created[entity].0.born >= 30, "only the guess after the release can be wrong: {:?}", created[entity].0);
    }

    // Entities that are real: created once (by the rollback that brought the input), never destroyed.
    let real: Vec<Entity> = created.iter().filter(|(_, (s, _))| truth.contains(&(s.owner, s.born))).map(|(e, _)| *e).collect();
    assert_eq!(real.len(), truth.len());
    assert!(real.iter().all(|e| !destroyed.contains_key(e)));

    // The views that are alive at the end are exactly the entities of the predicted frame.
    let alive: BTreeSet<Entity> = created.keys().filter(|e| !destroyed.contains_key(e)).copied().collect();
    let predicted: BTreeSet<Entity> = snapshot.predicted().dense::<Spark>().0.iter().copied().collect();
    assert_eq!(alive, predicted);

    // Every on_update had the pose of the view world for its entity.
    let updates: Vec<_> = calls.iter().filter_map(|c| if let Call::Update { entity, pose } = c { Some((*entity, *pose)) } else { None }).collect();
    assert!(!updates.is_empty());
    for (entity, pose) in updates {
        let pose = pose.unwrap_or_else(|| panic!("no pose for {entity:?}"));
        assert_eq!(pose.pos.x, created[&entity].0.owner as f32);
    }
}

#[test]
fn a_rollback_cancels_the_predicted_events_it_removes_and_verifies_the_rest_once() {
    let log = Rc::new(Log::default());
    let (mut views, mut world) = (views(&log), world());
    let mut bridge = rolling_back_bridge();
    for _ in 0..120 {
        frame(&mut bridge, &mut world, &mut views, &log);
    }
    let snapshot = bridge.snapshot().unwrap();
    let truth: BTreeSet<(u32, u32)> = snapshot.verified().unwrap().iter::<Spark>().map(|(_, s)| (s.owner, s.born)).collect();

    let mut states: BTreeMap<(u32, u32), Vec<EventState>> = BTreeMap::new();
    for (flash, state, _) in log.flashes.borrow().iter() {
        states.entry((flash.owner, flash.tick)).or_default().push(*state);
    }
    let (real, phantom): (Vec<_>, Vec<_>) = states.iter().partition(|(key, _)| truth.contains(key));
    assert_eq!(real.len(), truth.len(), "every real flash reached the scene view");
    assert!(!phantom.is_empty(), "the prediction flashed for a trigger nobody held");

    for (key, seen) in real {
        let verified = seen.iter().filter(|s| **s == EventState::Verified).count();
        assert_eq!(verified, 1, "flash {key:?} must be verified exactly once: {seen:?}");
        assert_eq!(seen.last(), Some(&EventState::Verified), "flash {key:?}: {seen:?}");
        assert!(!seen.contains(&EventState::Canceled), "flash {key:?}: {seen:?}");
    }
    for (key, seen) in phantom {
        assert_eq!(seen, &[EventState::Predicted, EventState::Canceled], "flash {key:?}");
    }
    // A cancel comes in a frame that also tells the scene view about the rollback.
    let rollback_frames: BTreeSet<u32> =
        log.notes.borrow().iter().filter(|(n, _)| matches!(n, Lifecycle::Rollback(_))).map(|(_, f)| *f).collect();
    for (_, state, frame) in log.flashes.borrow().iter() {
        if *state == EventState::Canceled {
            assert!(rollback_frames.contains(frame));
        }
    }
}

#[test]
fn an_index_that_comes_back_with_a_new_version_is_a_new_entity() {
    let log = Rc::new(Log::default());
    let (mut views, mut world) = (views(&log), world());
    let session = PlaySession::<Burst>::new(PlayConfig::new(1, 42, 60), BurstConfig { players: 1, ttl: 2 });
    let mut bridge = InProc::new(PlayHost::new(session, PlayerSlot(0)), BridgeConfig::default());
    for tick in 1..=12u32 {
        // One spark at tick 1, the next one at tick 7: the first one is long gone by then.
        let held = u32::from(tick == 1 || tick == 7);
        bridge.set_input(PlayerSlot(0), Trigger { held, _pad: 0 }).unwrap();
        frame(&mut bridge, &mut world, &mut views, &log);
    }
    let marks: Vec<(&str, Entity)> = entity_calls(&log)
        .into_iter()
        .filter_map(|c| match c {
            Call::Create { entity, .. } => Some(("create", entity)),
            Call::Destroy { entity, .. } => Some(("destroy", entity)),
            Call::Update { .. } => None,
        })
        .collect();
    assert_eq!(marks.len(), 4, "{marks:?}");
    let (first, second) = (marks[0].1, marks[2].1);
    assert_eq!([marks[0].0, marks[1].0, marks[2].0, marks[3].0], ["create", "destroy", "create", "destroy"]);
    assert_eq!(marks[1].1, first);
    assert_eq!(marks[3].1, second);
    assert_eq!(first.index, second.index, "the free index is reused");
    assert_ne!(first.version, second.version, "with a new version");
}

/// The runner drains the view world's lifecycle queue after each update. A game that spawns
/// and despawns an entity every tick keeps that queue at a constant size only if it is drained.
#[test]
fn the_view_world_lifecycle_queue_stays_bounded_when_drained_every_frame() {
    const FRAMES: usize = 300;
    let session = PlaySession::<Burst>::new(PlayConfig::new(1, 42, 60), BurstConfig { players: 1, ttl: 1 });
    let mut bridge = InProc::new(PlayHost::new(session, PlayerSlot(0)), BridgeConfig::default());
    let (mut drained, mut kept) = (world(), world());
    let mut largest = 0;
    for _ in 0..FRAMES {
        bridge.set_input(PlayerSlot(0), Trigger { held: 1, _pad: 0 }).unwrap();
        bridge.update(ONE_TICK);
        let update = bridge.poll_view();
        drained.update_from_bridge(1.0 / 60.0, &update);
        largest = largest.max(drained.take_lifecycle().len());
        kept.update_from_bridge(1.0 / 60.0, &update);
    }
    assert!(largest <= 2, "one spawn and one despawn per frame at most, got {largest}");
    assert!(drained.take_lifecycle().is_empty());
    assert!(kept.take_lifecycle().len() > FRAMES, "without a drain the queue grows with the run time");
}

#[test]
fn scene_views_start_once_and_are_called_in_the_order_they_were_added() {
    let log = Rc::new(Log::default());
    let mut views = Views::<Burst>::new();
    views.scene_view(Recorder { name: "a", log: log.clone() });
    views.scene_view(Recorder { name: "b", log: log.clone() });
    let session = PlaySession::<Burst>::new(PlayConfig::new(1, 42, 60), BurstConfig { players: 1, ttl: 100 });
    let mut bridge = InProc::new(PlayHost::new(session, PlayerSlot(0)), BridgeConfig::default());
    let mut world = world();
    for _ in 0..2 {
        bridge.set_input(PlayerSlot(0), Trigger { held: 1, _pad: 0 }).unwrap();
        frame(&mut bridge, &mut world, &mut views, &log);
    }
    let calls = log.scene_calls.borrow().clone();
    assert_eq!(calls.iter().filter(|c| c.ends_with(":start")).count(), 2, "one start per view, not per frame: {calls:?}");
    assert_eq!(&calls[..2], ["a:start", "b:start"]);
    // The session note comes before the events, and each call goes to a before b.
    assert_eq!(&calls[2..4], ["a:session", "b:session"]);
    assert!(matches!(log.notes.borrow()[0].0, Lifecycle::SessionStarted { .. }));
    let events: Vec<&String> = calls.iter().filter(|c| c.ends_with(":event")).collect();
    assert!(events.len() >= 4 && events.len() % 2 == 0);
    assert!(events.chunks(2).all(|pair| pair[0].starts_with("a:") && pair[1].starts_with("b:")));
    let updates: Vec<&String> = calls.iter().filter(|c| c.ends_with(":update")).collect();
    assert_eq!(updates.len(), 4);
}

/// Hand-made updates for the cases a real bridge does not produce on demand.
fn update_with(bridge: &mut impl Bridge<Burst>, events: Vec<BridgeEvent<Flash>>) -> ViewUpdate<Flash> {
    let mut update = bridge.poll_view();
    update.events = events;
    update
}

#[test]
fn a_cancel_carries_the_payload_of_its_predicted_event_and_a_resync_forgets_it() {
    let log = Rc::new(Log::default());
    let mut views = Views::<Burst>::new();
    views.scene_view(Recorder { name: "scene", log: log.clone() });
    let session = PlaySession::<Burst>::new(PlayConfig::new(1, 42, 60), BurstConfig { players: 1, ttl: 100 });
    let mut bridge = InProc::new(PlayHost::new(session, PlayerSlot(0)), BridgeConfig::default());
    bridge.step(1);
    let (first, second) = (EventKey::new(5, 0, 0), EventKey::new(6, 0, 0));
    let (a, b) = (Flash { owner: 7, tick: 5 }, Flash { owner: 8, tick: 6 });
    let sim = |key, status| BridgeEvent::Sim { key, status };

    let update = update_with(&mut bridge, vec![sim(first, EventStatus::Predicted(a)), sim(second, EventStatus::Predicted(b))]);
    views.dispatch(&mut bridge, &update, &[]);
    let update = update_with(&mut bridge, vec![sim(first, EventStatus::Canceled)]);
    views.dispatch(&mut bridge, &update, &[]);
    assert_eq!(*log.flashes.borrow().iter().map(|(f, s, _)| (*f, *s)).collect::<Vec<_>>(), [
        (a, EventState::Predicted),
        (b, EventState::Predicted),
        (a, EventState::Canceled)
    ]);

    // After a resync the view forgot the prediction, so a later cancel has nothing to cancel.
    let mut update = update_with(&mut bridge, vec![sim(second, EventStatus::Canceled)]);
    update.resync = Some(orr_bridge::ViewResync {
        generation: 1,
        discarded_events: 3,
        head_tick: 1,
        verified_tick: 1,
        disconnected: false,
        last_desync: None,
        lifecycle: Vec::new(),
    });
    views.dispatch(&mut bridge, &update, &[]);
    assert_eq!(log.flashes.borrow().len(), 3);
}

/// Sound or particles started on `Predicted` and stopped on `Canceled`, as the docs advise.
#[derive(Default)]
struct Effects {
    active: Rc<RefCell<BTreeSet<u32>>>,
    updates: Rc<Cell<u32>>,
}

impl SceneView<Burst> for Effects {
    fn on_update(&mut self, _ctx: &mut ViewContext<'_, Burst>) {
        self.updates.set(self.updates.get() + 1);
    }

    fn on_event(&mut self, _ctx: &mut ViewContext<'_, Burst>, event: &Flash, state: EventState) {
        match state {
            EventState::Predicted => self.active.borrow_mut().insert(event.tick),
            EventState::Verified | EventState::Canceled => self.active.borrow_mut().remove(&event.tick),
        };
    }

    fn on_resync(&mut self, _ctx: &mut ViewContext<'_, Burst>) {
        // No `Canceled` will come for what was started: stop it and start again from the frame.
        self.active.borrow_mut().clear();
    }
}

fn resync_note() -> orr_bridge::ViewResync {
    orr_bridge::ViewResync {
        generation: 1,
        discarded_events: 3,
        head_tick: 1,
        verified_tick: 1,
        disconnected: false,
        last_desync: None,
        lifecycle: Vec::new(),
    }
}

#[test]
fn a_resync_tells_the_scene_views_once_before_the_events_of_its_frame() {
    let log = Rc::new(Log::default());
    let effects = Effects::default();
    let (active, updates) = (effects.active.clone(), effects.updates.clone());
    let mut views = Views::<Burst>::new();
    views.scene_view(Recorder { name: "scene", log: log.clone() });
    views.scene_view(effects);
    let session = PlaySession::<Burst>::new(PlayConfig::new(1, 42, 60), BurstConfig { players: 1, ttl: 100 });
    let mut bridge = InProc::new(PlayHost::new(session, PlayerSlot(0)), BridgeConfig::default());
    bridge.step(1);
    let sim = |tick: u64, status| BridgeEvent::Sim { key: EventKey::new(tick, 0, 0), status };
    let flash = |tick: u32| Flash { owner: 0, tick };

    let update = update_with(&mut bridge, vec![sim(5, EventStatus::Predicted(flash(5)))]);
    views.dispatch(&mut bridge, &update, &[]);
    assert_eq!(*active.borrow(), BTreeSet::from([5]));
    assert!(log.resyncs.borrow().is_empty());

    // A resync without any session note, and an event after it that belongs to the new epoch.
    log.frame.set(1);
    let mut update = update_with(&mut bridge, vec![sim(6, EventStatus::Predicted(flash(6)))]);
    update.resync = Some(resync_note());
    views.dispatch(&mut bridge, &update, &[]);
    assert_eq!(*log.resyncs.borrow(), [1], "once");
    assert_eq!(*active.borrow(), BTreeSet::from([6]), "the old effect is gone, the new one runs");
    let calls = log.scene_calls.borrow().clone();
    let at = calls.iter().position(|c| c == "scene:resync").unwrap();
    assert_eq!(&calls[at..], ["scene:resync", "scene:event", "scene:update"]);

    // The Canceled of the forgotten event has nothing to cancel, and the frames go on.
    let update = update_with(&mut bridge, vec![sim(5, EventStatus::Canceled)]);
    views.dispatch(&mut bridge, &update, &[]);
    assert_eq!(*active.borrow(), BTreeSet::from([6]));
    assert_eq!(updates.get(), 3);
    assert_eq!(log.resyncs.borrow().len(), 1);
}

#[test]
fn the_legacy_resync_event_tells_the_scene_views_too() {
    let log = Rc::new(Log::default());
    let mut views = Views::<Burst>::new();
    views.scene_view(Recorder { name: "scene", log: log.clone() });
    let session = PlaySession::<Burst>::new(PlayConfig::new(1, 42, 60), BurstConfig { players: 1, ttl: 100 });
    let mut bridge = InProc::new(PlayHost::new(session, PlayerSlot(0)), BridgeConfig::default());
    bridge.step(1);
    let flash = BridgeEvent::Sim { key: EventKey::new(5, 0, 0), status: EventStatus::Predicted(Flash { owner: 0, tick: 5 }) };
    let update = update_with(&mut bridge, vec![BridgeEvent::ViewResynced(resync_note()), flash]);
    views.dispatch(&mut bridge, &update, &[]);
    assert_eq!(*log.resyncs.borrow(), [0]);
    let calls = log.scene_calls.borrow().clone();
    assert_eq!(calls, ["scene:start", "scene:resync", "scene:event", "scene:update"]);
}

/// A bridge whose queue holds one notification: a tick with two events overflows it, and the
/// recovery carries no session note, so `on_resync` is the only sign that something was lost.
#[test]
fn a_mailbox_overflow_of_sim_events_reaches_the_scene_views_as_a_resync() {
    let log = Rc::new(Log::default());
    let effects = Effects::default();
    let updates = effects.updates.clone();
    let mut views = Views::<Burst>::new();
    views.scene_view(Recorder { name: "scene", log: log.clone() });
    views.scene_view(effects);
    let session = PlaySession::<Burst>::new(PlayConfig::new(2, 42, 60), BurstConfig { players: 2, ttl: 100 });
    let host = PlayHost::new(session, PlayerSlot(0)).with_bot(|_, tick| Trigger { held: u32::from(tick >= 4), _pad: 0 });
    let mut bridge = InProc::new(host, BridgeConfig::default().with_view_event_capacity(1));
    let mut world = world();

    // Quiet frames first: the opening note is taken in and nothing overflows.
    for _ in 0..3 {
        frame(&mut bridge, &mut world, &mut views, &log);
    }
    assert!(log.resyncs.borrow().is_empty());

    // Both players hold the trigger: every tick has two flashes for a queue of one.
    let mut overflows = 0;
    for _ in 0..4 {
        bridge.set_input(PlayerSlot(0), Trigger { held: 1, _pad: 0 }).unwrap();
        bridge.update(ONE_TICK);
        let update = bridge.poll_view();
        if let Some(resync) = &update.resync {
            assert!(resync.lifecycle.is_empty(), "the overflow was made of sim events only");
            overflows += 1;
        }
        world.update_from_bridge(1.0 / 60.0, &update);
        log.frame.set(log.frame.get() + 1);
        views.dispatch(&mut bridge, &update, &[]);
    }
    assert!(overflows > 0, "the small mailbox must overflow");
    assert_eq!(log.resyncs.borrow().len(), overflows, "one call per resync");
    assert_eq!(updates.get(), 7, "every frame still updates the scene views");
}

#[test]
fn events_that_come_before_the_first_snapshot_wait_for_it() {
    let log = Rc::new(Log::default());
    let mut views = Views::<Burst>::new();
    views.scene_view(Recorder { name: "scene", log: log.clone() });
    let session = PlaySession::<Burst>::new(PlayConfig::new(1, 42, 60), BurstConfig { players: 1, ttl: 100 });
    let mut bridge = InProc::new(PlayHost::new(session, PlayerSlot(0)), BridgeConfig::default());
    let early = ViewUpdate { snapshot: None, events: vec![BridgeEvent::Lifecycle(Lifecycle::Paused { tick: 0 })], resync: None };
    views.dispatch(&mut bridge, &early, &[]);
    assert!(log.scene_calls.borrow().is_empty(), "no snapshot, no callbacks");

    bridge.step(1);
    let mut update = bridge.poll_view();
    update.events.clear();
    views.dispatch(&mut bridge, &update, &[]);
    let calls = log.scene_calls.borrow().clone();
    assert_eq!(calls, ["scene:start", "scene:session", "scene:update"]);
    assert_eq!(log.notes.borrow()[0].0, Lifecycle::Paused { tick: 0 });
}

#[test]
fn a_view_context_reads_both_frames_and_sends_commands() {
    struct Probe(Rc<RefCell<Vec<String>>>);
    impl SceneView<Burst> for Probe {
        fn on_update(&mut self, ctx: &mut ViewContext<'_, Burst>) {
            let verified = ctx.verified().map(|f| f.tick());
            let sent = ctx.send_command(()).is_ok();
            let line = format!(
                "tick {} verified {:?} (tick {}) local {:?} sent {:?}",
                ctx.predicted().tick(),
                verified,
                ctx.verified_tick(),
                ctx.local_players(),
                sent
            );
            self.0.borrow_mut().push(line);
        }
    }
    let lines = Rc::new(RefCell::new(Vec::new()));
    let mut views = Views::<Burst>::new();
    views.scene_view(Probe(lines.clone()));
    let session = PlaySession::<Burst>::new(PlayConfig::new(1, 42, 60), BurstConfig { players: 1, ttl: 100 });
    let mut bridge = InProc::new(PlayHost::new(session, PlayerSlot(0)), BridgeConfig::default());
    bridge.step(3);
    let update = bridge.poll_view();
    views.dispatch(&mut bridge, &update, &[]);
    let lines = lines.borrow();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].starts_with("tick 3 verified Some(3) (tick 3) local [PlayerSlot(0)] sent true"), "{}", lines[0]);
}

#[test]
fn a_view_context_reports_every_local_slot_of_the_bridge() {
    type Seen = Vec<(Vec<PlayerSlot>, [bool; 3])>;
    struct Probe(Rc<RefCell<Seen>>);
    impl SceneView<Burst> for Probe {
        fn on_update(&mut self, ctx: &mut ViewContext<'_, Burst>) {
            let slots = [PlayerSlot(0), PlayerSlot(1), PlayerSlot(2)];
            self.0.borrow_mut().push((ctx.local_players().to_vec(), slots.map(|slot| ctx.is_local(slot))));
        }
    }
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut views = Views::<Burst>::new();
    views.scene_view(Probe(seen.clone()));
    let session = PlaySession::<Burst>::new(PlayConfig::new(3, 42, 60), BurstConfig { players: 3, ttl: 100 });
    let host = PlayHost::new(session, PlayerSlot(0)).with_extra_local_slots([PlayerSlot(1)]);
    let mut bridge = InProc::new(host, BridgeConfig::default());
    bridge.step(1);
    let update = bridge.poll_view();
    views.dispatch(&mut bridge, &update, &[]);
    assert_eq!(*seen.borrow(), vec![(vec![PlayerSlot(0), PlayerSlot(1)], [true, true, false])]);
}

/// Draws every spark as a sphere at `(owner + 0.5, 1, 2)`.
struct SparkLook3;

impl Extractor3 for SparkLook3 {
    fn extract(&self, frame: FrameView<'_>, out: &mut Vec<Extracted3>) {
        for (entity, spark) in frame.iter::<Spark>() {
            out.push(Extracted3 {
                entity,
                transform: Transform3::new(Vec3::new(spark.owner as f32 + 0.5, 1.0, 2.0), Quat::IDENTITY),
                mode: InterpMode::Prediction,
                style: Style3::new(Shape3::Sphere { radius: 1.0 }, [1.0; 3]),
            });
        }
    }
}

#[test]
fn in_a_3d_run_the_entity_views_get_the_3d_pose_and_not_the_flat_one() {
    #[derive(Default)]
    struct Probe {
        flat: Rc<Cell<u32>>,
        poses: Rc<RefCell<Vec<(Entity, Option<Transform3>)>>>,
    }
    impl EntityView<Burst> for Probe {
        fn on_update(&mut self, _ctx: &mut ViewContext<'_, Burst>, _entity: Entity, _pose: Option<Transform2>) {
            self.flat.set(self.flat.get() + 1);
        }

        fn on_update3(&mut self, _ctx: &mut ViewContext<'_, Burst>, entity: Entity, pose: Option<Transform3>) {
            self.poses.borrow_mut().push((entity, pose));
        }
    }
    let (flat, poses) = (Rc::new(Cell::new(0)), Rc::new(RefCell::new(Vec::new())));
    let (for_view, for_poses) = (flat.clone(), poses.clone());
    let mut views = Views::<Burst>::new();
    views.entity_view::<Spark>(move || Probe { flat: for_view.clone(), poses: for_poses.clone() });
    let session = PlaySession::<Burst>::new(PlayConfig::new(1, 42, 60), BurstConfig { players: 1, ttl: 100 });
    let mut bridge = InProc::new(PlayHost::new(session, PlayerSlot(0)), BridgeConfig::default());
    bridge.set_input(PlayerSlot(0), Trigger { held: 1, _pad: 0 }).unwrap();
    bridge.step(1);
    let mut world = ViewWorld3::new(SparkLook3, ViewConfig::default());

    let update = bridge.poll_view();
    world.update_from_bridge(1.0 / 60.0, &update);
    let mut items = Vec::new();
    world.render_items(&mut items);
    views.dispatch3(&mut bridge, &update, &items);

    assert_eq!(flat.get(), 0);
    let poses = poses.borrow();
    assert_eq!(poses.len(), 1);
    let pose = poses[0].1.expect("the extractor lists the spark, so the view world has its pose");
    assert_eq!(pose.pos.to_array(), [0.5, 1.0, 2.0]);
}
