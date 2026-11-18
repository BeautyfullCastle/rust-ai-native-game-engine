//! Entity views, scene views and the window-free code that calls them.
//!
//! [`Views::dispatch`] is the whole stage after `poll_view` in the frame order of
//! `docs/view-framework.md` 4.2: events (`on_event`, `on_session`), then the entity
//! diff (`on_create`, `on_destroy`), then `on_update`. It has no window and no
//! renderer, so the window loop only calls it and tests can drive it with any bridge.

use std::collections::{BTreeMap, BTreeSet};
use std::marker::PhantomData;

use orr_bridge::{
    Bridge, BridgeError, BridgeEvent, EventKey, EventStatus, FrameView, Lifecycle, PlayerSlot, RawPlayerSignal, Snapshot, ViewUpdate,
};
use orr_ecs::{Component, Entity};
use orr_sim::Game;
use orr_audio::Clip;
use orr_view::{RenderItem, RenderItem3, Transform2, Transform3};

use crate::audio::{Audio, SoundOut};
use crate::overlay::Overlay;

/// What happened to an event, as the scene view sees it. Same meaning as
/// `orr_session::EventStatus`, without the payload (the payload is a separate argument).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventState {
    /// The event came from a predicted tick. A rollback can still remove it.
    /// Start sounds and particles here, and stop them on `Canceled`.
    Predicted,
    /// The tick of the event is verified. Announced once per event. Use it for
    /// achievements, statistics and anything else that must not be undone.
    Verified,
    /// A rollback removed (or changed) an event that was `Predicted` before. The
    /// payload is the one that came with the `Predicted` call.
    Canceled,
}

/// What a view can read and do during one call. Valid for this call only.
pub struct ViewContext<'a, G: Game> {
    snapshot: &'a Snapshot,
    local: Vec<PlayerSlot>,
    bridge: &'a mut dyn Bridge<G>,
    overlay: &'a mut Overlay,
    audio: &'a mut Audio,
}

impl<'a, G: Game> ViewContext<'a, G> {
    /// The newest predicted frame. Use it for drawing and for instant reactions.
    pub fn predicted(&self) -> FrameView<'a> {
        self.snapshot.predicted()
    }

    /// The newest verified frame, if the bridge has one. Use it for things that must not jump back.
    pub fn verified(&self) -> Option<FrameView<'a>> {
        self.snapshot.verified()
    }

    /// The tick of [`ViewContext::predicted`].
    pub fn tick(&self) -> u64 {
        self.snapshot.tick()
    }

    /// The tick of [`ViewContext::verified`].
    pub fn verified_tick(&self) -> u64 {
        self.snapshot.verified_tick()
    }

    /// The players this client controls: every local slot of the bridge
    /// (offline: slots `0..n` of [`crate::App::local_players`], default one).
    pub fn local_players(&self) -> &[PlayerSlot] {
        &self.local
    }

    /// Whether `slot` is controlled by this client.
    pub fn is_local(&self, slot: PlayerSlot) -> bool {
        self.local.contains(&slot)
    }

    /// The overlay of this frame: text on the screen and lines and rectangles in the world, drawn
    /// after the default renderer, on top of the picture. It is empty at the start of every frame,
    /// so call it in every frame in which the things should be visible (for example in a scene
    /// view's `on_update`). The text font is ASCII only (see [`crate::Overlay`]).
    pub fn draw(&mut self) -> &mut Overlay {
        self.overlay
    }

    /// Plays and stops sounds (see [`crate::Audio`]). Without an audio device the calls do nothing.
    pub fn audio(&mut self) -> &mut Audio {
        self.audio
    }

    /// Sends a one-off command to the simulation (same as [`Bridge::send_command`]).
    pub fn send_command(&mut self, command: G::Command) -> Result<(), BridgeError> {
        self.bridge.send_command(command)
    }
}

/// The screen object of one simulation entity.
///
/// One instance exists for every entity that has the component given to
/// [`crate::App::entity_view`] in the newest predicted frame. Every method does nothing by default.
pub trait EntityView<G: Game> {
    /// The entity appeared in the predicted frame.
    fn on_create(&mut self, ctx: &mut ViewContext<'_, G>, entity: Entity) {
        let _ = (ctx, entity);
    }

    /// Once per frame while the entity exists. `pose` is the position that `ViewWorld`
    /// shows (interpolated and smoothed after a rollback, by the interpolation mode
    /// the extractor chose). It is `None` when the extractor does not list this
    /// entity, because then the view world has no pose for it.
    fn on_update(&mut self, ctx: &mut ViewContext<'_, G>, entity: Entity, pose: Option<Transform2>) {
        let _ = (ctx, entity, pose);
    }

    /// The same as [`EntityView::on_update`] for a run with a 3D view ([`crate::App::view3d`]):
    /// the app calls this method there, and `on_update` in a 2D run. `pose` is the position and
    /// rotation that `ViewWorld3` shows.
    fn on_update3(&mut self, ctx: &mut ViewContext<'_, G>, entity: Entity, pose: Option<Transform3>) {
        let _ = (ctx, entity, pose);
    }

    /// The entity is gone from the predicted frame: it was despawned, or a rollback
    /// removed an entity that only the prediction had. The frame in `ctx` no longer has it.
    fn on_destroy(&mut self, ctx: &mut ViewContext<'_, G>, entity: Entity) {
        let _ = (ctx, entity);
    }
}

/// A screen object that exists once for the whole scene (score board, sound
/// effects, camera shake). Several can be registered. They are called in registration order.
pub trait SceneView<G: Game> {
    /// Once, before the first event and the first update, after the first snapshot arrived.
    fn on_start(&mut self, ctx: &mut ViewContext<'_, G>) {
        let _ = ctx;
    }

    /// Once per frame, after every entity view was updated.
    fn on_update(&mut self, ctx: &mut ViewContext<'_, G>) {
        let _ = ctx;
    }

    /// A game event. One event can arrive several times: `Predicted`, then `Verified` or `Canceled`.
    fn on_event(&mut self, ctx: &mut ViewContext<'_, G>, event: &G::Event, state: EventState) {
        let _ = (ctx, event, state);
    }

    /// A player signal: a player entered, dropped, came back or left. It arrives like an event:
    /// `Predicted`, then `Verified` or `Canceled`, and a signal of a tick comes before the events
    /// of that tick. Create the avatar view or a name tag on `Predicted` (and remove it on
    /// `Canceled`). Do what cannot be undone, such as a "player joined" message log entry, on `Verified`.
    ///
    /// `signal.typed::<P>()` gives the start data as the game's `Players::Player`. Without a
    /// server, the app announces every local player with `Added` at the first tick, and that
    /// signal is `Verified` at once.
    fn on_player(&mut self, ctx: &mut ViewContext<'_, G>, signal: &RawPlayerSignal, state: EventState) {
        let _ = (ctx, signal, state);
    }

    /// A session note: rollback, stall, disconnect, seek and so on.
    fn on_session(&mut self, ctx: &mut ViewContext<'_, G>, note: Lifecycle) {
        let _ = (ctx, note);
    }

    /// The bridge dropped events it could not deliver (its queue overflowed), so the view lost
    /// track. All earlier `Predicted` events are forgotten: no `Verified` or `Canceled` will come
    /// for them. Stop the speculative effects you started on `Predicted` and rebuild what you
    /// show from `ctx.predicted()` and `ctx.verified()`. Called once per resync, before the
    /// events and session notes of that frame (those notes still follow in `on_session`).
    fn on_resync(&mut self, ctx: &mut ViewContext<'_, G>) {
        let _ = ctx;
    }
}

/// Makes one [`EntityView`] for each new entity. Any `FnMut() -> V` is a factory,
/// so `PaddleView::default` and `|| PaddleView::new(..)` both work.
pub trait EntityFactory<G: Game> {
    type View: EntityView<G> + 'static;

    fn make(&mut self) -> Self::View;
}

impl<G: Game, V: EntityView<G> + 'static, F: FnMut() -> V> EntityFactory<G> for F {
    type View = V;

    fn make(&mut self) -> V {
        self()
    }
}

/// The poses the view world shows, by entity: flat (2D run) or in space (3D run).
enum PoseMap {
    D2(BTreeMap<Entity, Transform2>),
    D3(BTreeMap<Entity, Transform3>),
}

/// The items of the view world, as the caller of [`Views::dispatch`] or [`Views::dispatch3`] has them.
enum Poses<'a> {
    D2(&'a [RenderItem]),
    D3(&'a [RenderItem3]),
}

impl Poses<'_> {
    fn by_entity(&self) -> PoseMap {
        match self {
            Poses::D2(items) => PoseMap::D2(items.iter().filter(|item| item.entity != Entity::NONE).map(|item| (item.entity, item.transform)).collect()),
            Poses::D3(items) => PoseMap::D3(items.iter().filter(|item| item.entity != Entity::NONE).map(|item| (item.entity, item.transform)).collect()),
        }
    }
}

trait EntityLayer<G: Game> {
    /// Calls `on_destroy` for entities that left the predicted frame, then `on_create` for new ones.
    fn sync(&mut self, ctx: &mut ViewContext<'_, G>);
    fn update(&mut self, ctx: &mut ViewContext<'_, G>, poses: &PoseMap);
}

/// The live views of one component type, keyed by entity `(index, version)`.
struct Layer<G: Game, C, F: EntityFactory<G>> {
    factory: F,
    live: BTreeMap<Entity, F::View>,
    component: PhantomData<fn() -> C>,
}

impl<G: Game, C: Component, F: EntityFactory<G>> EntityLayer<G> for Layer<G, C, F> {
    fn sync(&mut self, ctx: &mut ViewContext<'_, G>) {
        let frame = ctx.predicted();
        let now: BTreeSet<Entity> = if frame.registers::<C>() { frame.dense::<C>().0.iter().copied().collect() } else { BTreeSet::new() };
        let gone: Vec<Entity> = self.live.keys().filter(|e| !now.contains(e)).copied().collect();
        for entity in gone {
            if let Some(mut view) = self.live.remove(&entity) {
                view.on_destroy(ctx, entity);
            }
        }
        for entity in now {
            if !self.live.contains_key(&entity) {
                let mut view = self.factory.make();
                view.on_create(ctx, entity);
                self.live.insert(entity, view);
            }
        }
    }

    fn update(&mut self, ctx: &mut ViewContext<'_, G>, poses: &PoseMap) {
        for (entity, view) in &mut self.live {
            match poses {
                PoseMap::D2(poses) => view.on_update(ctx, *entity, poses.get(entity).copied()),
                PoseMap::D3(poses) => view.on_update3(ctx, *entity, poses.get(entity).copied()),
            }
        }
    }
}

/// All entity views and scene views of one run, and the state they need to be called right.
///
/// [`crate::App`] owns one and calls [`Views::dispatch`] once per frame. It can also be
/// used without a window, with any [`Bridge`].
pub struct Views<G: Game> {
    layers: Vec<Box<dyn EntityLayer<G>>>,
    scenes: Vec<Box<dyn SceneView<G>>>,
    started: bool,
    snapshot: Option<Snapshot>,
    /// Events that came before the first snapshot. Delivered with it.
    waiting: Vec<BridgeEvent<G::Event>>,
    /// The payload of every `Predicted` event that is neither verified nor canceled yet,
    /// because `Canceled` itself carries no payload.
    announced: BTreeMap<EventKey, G::Event>,
    /// The same for player signals.
    announced_signals: BTreeMap<EventKey, RawPlayerSignal>,
    /// What the views drew in the last frame of view calls.
    overlay: Overlay,
    audio: Audio,
}

impl<G: Game> Default for Views<G> {
    fn default() -> Self {
        Self::new()
    }
}

impl<G: Game> Views<G> {
    pub fn new() -> Self {
        Self { layers: Vec::new(), scenes: Vec::new(), started: false, snapshot: None, waiting: Vec::new(), announced: BTreeMap::new(), announced_signals: BTreeMap::new(), overlay: Overlay::default(), audio: Audio::default() }
    }

    /// One view (made by `factory`) for every entity that has component `C`.
    pub fn entity_view<C: Component>(&mut self, factory: impl EntityFactory<G> + 'static) {
        self.layers.push(Box::new(Layer::<G, C, _> { factory, live: BTreeMap::new(), component: PhantomData }));
    }

    /// A scene view. Called after the ones added before it.
    pub fn scene_view(&mut self, view: impl SceneView<G> + 'static) {
        self.scenes.push(Box::new(view));
    }

    /// What the views drew in the newest call of [`Views::dispatch`]: it is cleared at the start of every call.
    pub fn overlay(&self) -> &Overlay {
        &self.overlay
    }

    /// Registers a clip under `name` for `ctx.audio().play_named(name)`. Replaces a clip of the same name.
    pub fn sound(&mut self, name: &str, clip: Clip) {
        self.audio.add(name, clip);
    }

    /// The output that the sounds of the views play on. Without one (the default for a window-free
    /// `Views`) the views play nothing. [`crate::App`] makes the run open the system device
    /// when the first sound plays (and none for a screenshot).
    pub fn set_sound_out(&mut self, out: impl SoundOut + 'static) {
        self.audio.set_out(Box::new(out));
    }

    pub(crate) fn use_sound_device(&mut self) {
        self.audio.use_device();
    }

    pub(crate) fn mute_sound(&mut self) {
        self.audio.mute();
    }

    /// Once per frame: drops an audio output that failed.
    pub(crate) fn poll_audio(&mut self) {
        self.audio.poll();
    }

    /// `false` when no view is registered, so the caller can skip the work of building poses.
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty() && self.scenes.is_empty()
    }

    /// One frame of view callbacks, in this order:
    /// `on_start` (first frame only), session notes and events, `on_destroy`, `on_create`,
    /// entity `on_update`, scene `on_update`.
    ///
    /// `update` is what `Bridge::poll_view` returned. `poses` are the items of the view
    /// world after it took the same update (`ViewWorld::render_items`); entities are matched by key.
    /// Nothing is dropped: events that arrive before the first snapshot are kept until it comes.
    pub fn dispatch(&mut self, bridge: &mut dyn Bridge<G>, update: &ViewUpdate<G::Event>, poses: &[RenderItem]) {
        self.dispatch_poses(bridge, update, Poses::D2(poses));
    }

    /// [`Views::dispatch`] for a 3D run: `poses` are the items of `ViewWorld3::render_items`, and
    /// the entity views get them through [`EntityView::on_update3`].
    pub fn dispatch3(&mut self, bridge: &mut dyn Bridge<G>, update: &ViewUpdate<G::Event>, poses: &[RenderItem3]) {
        self.dispatch_poses(bridge, update, Poses::D3(poses));
    }

    fn dispatch_poses(&mut self, bridge: &mut dyn Bridge<G>, update: &ViewUpdate<G::Event>, poses: Poses<'_>) {
        self.overlay.clear();
        if let Some(snapshot) = &update.snapshot {
            self.snapshot = Some(snapshot.clone());
        }
        let mut events = std::mem::take(&mut self.waiting);
        events.extend(update.events.iter().cloned());
        let Some(snapshot) = self.snapshot.clone() else {
            self.waiting = events;
            return;
        };
        // The context borrows the overlay and the audio while the view calls borrow `self`, so they are moved out for the frame.
        let mut overlay = std::mem::take(&mut self.overlay);
        let mut audio = std::mem::take(&mut self.audio);
        let mut ctx = ViewContext { snapshot: &snapshot, local: bridge.local_slots(), bridge, overlay: &mut overlay, audio: &mut audio };

        if !self.started {
            self.started = true;
            for scene in &mut self.scenes {
                scene.on_start(&mut ctx);
            }
        }
        if let Some(resync) = &update.resync {
            // Events before the resync are gone. Speculative effects must not wait for a `Canceled` that cannot come.
            self.resynced(&mut ctx);
            for recovery in &resync.lifecycle {
                self.session_note(&mut ctx, recovery.last);
            }
        }
        for event in events {
            match event {
                BridgeEvent::ViewResynced(_) => self.resynced(&mut ctx),
                BridgeEvent::Lifecycle(note) => self.session_note(&mut ctx, note),
                BridgeEvent::Player { key, status } => match status {
                    EventStatus::Predicted(signal) => {
                        self.announced_signals.insert(key, signal.clone());
                        self.player(&mut ctx, &signal, EventState::Predicted);
                    }
                    EventStatus::Verified(signal) => {
                        self.announced_signals.remove(&key);
                        self.player(&mut ctx, &signal, EventState::Verified);
                    }
                    EventStatus::Canceled => {
                        if let Some(signal) = self.announced_signals.remove(&key) {
                            self.player(&mut ctx, &signal, EventState::Canceled);
                        }
                    }
                },
                BridgeEvent::Sim { key, status } => match status {
                    EventStatus::Predicted(payload) => {
                        self.announced.insert(key, payload);
                        self.event(&mut ctx, &payload, EventState::Predicted);
                    }
                    EventStatus::Verified(payload) => {
                        self.announced.remove(&key);
                        self.event(&mut ctx, &payload, EventState::Verified);
                    }
                    EventStatus::Canceled => {
                        // Nothing to cancel when the view never saw the event (it was forgotten at a resync).
                        if let Some(payload) = self.announced.remove(&key) {
                            self.event(&mut ctx, &payload, EventState::Canceled);
                        }
                    }
                },
            }
        }

        for layer in &mut self.layers {
            layer.sync(&mut ctx);
        }
        if !self.layers.is_empty() {
            let by_entity = poses.by_entity();
            for layer in &mut self.layers {
                layer.update(&mut ctx, &by_entity);
            }
        }
        for scene in &mut self.scenes {
            scene.on_update(&mut ctx);
        }
        drop(ctx);
        self.overlay = overlay;
        self.audio = audio;
    }

    fn event(&mut self, ctx: &mut ViewContext<'_, G>, payload: &G::Event, state: EventState) {
        for scene in &mut self.scenes {
            scene.on_event(ctx, payload, state);
        }
    }

    fn player(&mut self, ctx: &mut ViewContext<'_, G>, signal: &RawPlayerSignal, state: EventState) {
        for scene in &mut self.scenes {
            scene.on_player(ctx, signal, state);
        }
    }

    /// Forgets the payloads of the `Predicted` events and tells the scene views.
    fn resynced(&mut self, ctx: &mut ViewContext<'_, G>) {
        self.announced.clear();
        self.announced_signals.clear();
        for scene in &mut self.scenes {
            scene.on_resync(ctx);
        }
    }

    fn session_note(&mut self, ctx: &mut ViewContext<'_, G>, note: Lifecycle) {
        for scene in &mut self.scenes {
            scene.on_session(ctx, note);
        }
    }
}
