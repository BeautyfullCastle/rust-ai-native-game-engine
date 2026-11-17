//! Entity views, scene views and the window-free code that calls them.
//!
//! [`Views::dispatch`] is the whole stage after `poll_view` in the frame order of
//! `docs/view-framework.md` 4.2: events (`on_event`, `on_session`), then the entity
//! diff (`on_create`, `on_destroy`), then `on_update`. It has no window and no
//! renderer, so the window loop only calls it and tests can drive it with any bridge.

use std::collections::{BTreeMap, BTreeSet};
use std::marker::PhantomData;

use orr_bridge::{Bridge, BridgeError, BridgeEvent, EventKey, EventStatus, FrameView, Lifecycle, PlayerSlot, Snapshot, ViewUpdate};
use orr_ecs::{Component, Entity};
use orr_sim::Game;
use orr_view::{RenderItem, Transform2};

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
    local: [PlayerSlot; 1],
    bridge: &'a mut dyn Bridge<G>,
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

    /// The players this client controls. Offline: the one local slot.
    pub fn local_players(&self) -> &[PlayerSlot] {
        &self.local
    }

    /// Whether `slot` is controlled by this client.
    pub fn is_local(&self, slot: PlayerSlot) -> bool {
        self.local.contains(&slot)
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

    /// A session note: rollback, stall, disconnect, seek and so on.
    fn on_session(&mut self, ctx: &mut ViewContext<'_, G>, note: Lifecycle) {
        let _ = (ctx, note);
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

type Poses = BTreeMap<Entity, Transform2>;

trait EntityLayer<G: Game> {
    /// Calls `on_destroy` for entities that left the predicted frame, then `on_create` for new ones.
    fn sync(&mut self, ctx: &mut ViewContext<'_, G>);
    fn update(&mut self, ctx: &mut ViewContext<'_, G>, poses: &Poses);
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

    fn update(&mut self, ctx: &mut ViewContext<'_, G>, poses: &Poses) {
        for (entity, view) in &mut self.live {
            view.on_update(ctx, *entity, poses.get(entity).copied());
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
}

impl<G: Game> Default for Views<G> {
    fn default() -> Self {
        Self::new()
    }
}

impl<G: Game> Views<G> {
    pub fn new() -> Self {
        Self { layers: Vec::new(), scenes: Vec::new(), started: false, snapshot: None, waiting: Vec::new(), announced: BTreeMap::new() }
    }

    /// One view (made by `factory`) for every entity that has component `C`.
    pub fn entity_view<C: Component>(&mut self, factory: impl EntityFactory<G> + 'static) {
        self.layers.push(Box::new(Layer::<G, C, _> { factory, live: BTreeMap::new(), component: PhantomData }));
    }

    /// A scene view. Called after the ones added before it.
    pub fn scene_view(&mut self, view: impl SceneView<G> + 'static) {
        self.scenes.push(Box::new(view));
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
        if let Some(snapshot) = &update.snapshot {
            self.snapshot = Some(snapshot.clone());
        }
        let mut events = std::mem::take(&mut self.waiting);
        events.extend(update.events.iter().cloned());
        let Some(snapshot) = self.snapshot.clone() else {
            self.waiting = events;
            return;
        };
        let mut ctx = ViewContext { snapshot: &snapshot, local: [bridge.local_slot()], bridge };

        if !self.started {
            self.started = true;
            for scene in &mut self.scenes {
                scene.on_start(&mut ctx);
            }
        }
        if let Some(resync) = &update.resync {
            // Events before the resync are gone. Speculative effects must not wait for a `Canceled` that cannot come.
            self.announced.clear();
            for recovery in &resync.lifecycle {
                self.session_note(&mut ctx, recovery.last);
            }
        }
        for event in events {
            match event {
                BridgeEvent::ViewResynced(_) => self.announced.clear(),
                BridgeEvent::Lifecycle(note) => self.session_note(&mut ctx, note),
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
            let by_entity: Poses = poses.iter().filter(|item| item.entity != Entity::NONE).map(|item| (item.entity, item.transform)).collect();
            for layer in &mut self.layers {
                layer.update(&mut ctx, &by_entity);
            }
        }
        for scene in &mut self.scenes {
            scene.on_update(&mut ctx);
        }
    }

    fn event(&mut self, ctx: &mut ViewContext<'_, G>, payload: &G::Event, state: EventState) {
        for scene in &mut self.scenes {
            scene.on_event(ctx, payload, state);
        }
    }

    fn session_note(&mut self, ctx: &mut ViewContext<'_, G>, note: Lifecycle) {
        for scene in &mut self.scenes {
            scene.on_session(ctx, note);
        }
    }
}
