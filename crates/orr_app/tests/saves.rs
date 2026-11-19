//! Save slots against a real offline bridge: save the verified frame, load it into a new session,
//! and refuse damaged, foreign and mismatched files without touching the run. No window.
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use bytemuck::{Pod, Zeroable};
use orr_app::{EventState, SaveError, SaveNotice, SaveStore, SceneView, ViewContext, Views, SAVE_VERSION};
use orr_bridge::{Bridge, BridgeConfig, InProc, PlayConfig, PlayHost, PlaySession, PlayerSlot};
use orr_sim::{ComponentRegistryBuilder, Frame, Game, SimContext, System};

const SEED: u64 = 7;
const RATE: u32 = 60;

// ---- a small game with state in a singleton, entities, the frame random numbers and an event ----

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, Pod, Zeroable)]
struct Step {
    add: u32,
    _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
struct Total {
    value: u64,
    noise: u64,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
struct Dot {
    born: u64,
}

/// Every 25 points of the total: a checkpoint.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Pod, Zeroable)]
struct Mark {
    level: u32,
    _pad: u32,
}

struct Counter;

impl Game for Counter {
    type Input = Step;
    type Command = ();
    type Event = Mark;
    type Config = ();

    fn register(builder: &mut ComponentRegistryBuilder) {
        builder.register_component::<Dot>("Dot");
        builder.register_singleton::<Total>("Total");
    }

    fn setup(frame: &mut Frame, _config: &()) {
        frame.set_singleton(Total { value: 0, noise: 0 });
    }

    fn systems() -> Vec<Box<dyn System<Self>>> {
        vec![Box::new(Tally)]
    }
}

struct Tally;

impl System<Counter> for Tally {
    fn name(&self) -> &'static str {
        "Tally"
    }

    fn run(&mut self, ctx: &mut SimContext<Counter>) {
        let add = ctx.inputs.input(PlayerSlot(0)).add;
        let noise = ctx.rng().next_u32();
        let before = ctx.frame.singleton::<Total>().value;
        let total = ctx.frame.singleton_mut::<Total>();
        total.value += u64::from(add);
        total.noise = total.noise.wrapping_add(u64::from(noise));
        let after = total.value;
        if ctx.tick % 7 == 0 {
            let dot = ctx.frame.spawn();
            ctx.frame.add(dot, Dot { born: ctx.tick });
        }
        if after / 25 != before / 25 {
            ctx.emit(Mark { level: (after / 25) as u32, _pad: 0 });
        }
    }
}

// ---- helpers ----

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("orr_app_saves_{name}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

type Bridged = InProc<Counter, PlayHost<Counter>>;

fn bridge_of(session: PlaySession<Counter>) -> Bridged {
    InProc::new(PlayHost::new(session, PlayerSlot(0)), BridgeConfig::default())
}

fn new_run() -> Bridged {
    bridge_of(PlaySession::<Counter>::new(PlayConfig::new(1, SEED, RATE), ()))
}

/// Runs `ticks` ticks of input 1 and gives the views each frame.
fn play(bridge: &mut Bridged, views: &mut Views<Counter>, ticks: u32) {
    for _ in 0..ticks {
        bridge.set_input(PlayerSlot(0), Step { add: 1, _pad: 0 }).unwrap();
        bridge.step(1);
        let update = bridge.poll_view();
        views.dispatch(bridge, &update, &[]);
    }
}

fn views_in(dir: &Path, game: &str, version: u32) -> Views<Counter> {
    let mut views = Views::<Counter>::new();
    views.enable_saves(SaveStore::at(dir, game, version).unwrap(), 1, RATE);
    views
}

fn checksum(bridge: &Bridged) -> u64 {
    bridge.snapshot().unwrap().verified().unwrap().checksum()
}

fn load_into_new_bridge(views: &mut Views<Counter>, slot: &str) -> Bridged {
    views.load_game(slot).expect("the save loads");
    let loaded = views.take_load().expect("an accepted load waits for the end of the frame");
    bridge_of(loaded.into_session::<Counter>(SEED).expect("the session starts"))
}

// ---- save and load ----

#[test]
fn a_loaded_session_has_the_saved_checksum_and_then_plays_exactly_like_the_original() {
    let dir = temp_dir("roundtrip");
    let (mut bridge, mut views) = (new_run(), views_in(&dir, "counter", 1));
    play(&mut bridge, &mut views, 120);
    let at_save = checksum(&bridge);
    let info = views.save_game("quick", "after 120 ticks").expect("saved");
    assert_eq!((info.tick, info.checksum, info.players, info.tick_rate), (120, at_save, 1, RATE));
    assert_eq!(views.save_game("quick", "x").map(|i| i.slot), Ok("quick".to_string()), "a slot can be written again");

    // The original goes on. The loaded session starts from the save, not from the end of the original.
    play(&mut bridge, &mut views, 60);
    assert_ne!(checksum(&bridge), at_save);
    let mut loaded = load_into_new_bridge(&mut views, "quick");
    let first = loaded.poll_view();
    assert_eq!(first.snapshot.expect("a first snapshot").tick(), 120);
    assert_eq!(checksum(&loaded), at_save, "the checksum after the load is the saved one");

    // Same state and same inputs make the same future (the random numbers are part of the frame).
    let mut reference = new_run();
    let mut scratch = Views::<Counter>::new();
    play(&mut reference, &mut scratch, 180);
    play(&mut loaded, &mut scratch, 60);
    assert_eq!(checksum(&loaded), checksum(&reference), "120 saved ticks and 60 more equal 180 straight ticks");
    assert_eq!(loaded.snapshot().unwrap().tick(), 180);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn slots_are_listed_in_name_order_and_deleted() {
    let dir = temp_dir("slots");
    let (mut bridge, mut views) = (new_run(), views_in(&dir, "counter", 1));
    assert!(views.save_game("a", "x").unwrap_err().to_string().contains("verified"), "nothing to save before the first frame");
    play(&mut bridge, &mut views, 10);
    views.save_game("beta", "second").unwrap();
    play(&mut bridge, &mut views, 5);
    views.save_game("alpha", "first").unwrap();
    let store = SaveStore::at(&dir, "counter", 1).unwrap();
    let list = store.list().unwrap();
    assert_eq!(list.iter().map(|e| e.slot.as_str()).collect::<Vec<_>>(), ["alpha", "beta"]);
    let (alpha, beta) = (list[0].info.clone().unwrap(), list[1].info.clone().unwrap());
    assert_eq!((alpha.label.as_str(), alpha.tick, beta.label.as_str(), beta.tick), ("first", 15, "second", 10));
    assert!(alpha.saved_at > 1_600_000_000, "the time of the save is set");

    store.delete("alpha").unwrap();
    assert_eq!(store.list().unwrap().iter().map(|e| e.slot.as_str()).collect::<Vec<_>>(), ["beta"]);
    assert_eq!(store.delete("alpha").unwrap_err(), SaveError::NotFound("alpha".to_string()));
    assert_eq!(store.delete("../x").unwrap_err(), SaveError::BadSlot("../x".to_string()));
    assert!(SaveStore::at(dir.join("nothing-here"), "counter", 1).unwrap().list().unwrap().is_empty(), "no folder is an empty list");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_damaged_save_is_refused_and_the_run_is_untouched() {
    let dir = temp_dir("damaged");
    let (mut bridge, mut views) = (new_run(), views_in(&dir, "counter", 1));
    play(&mut bridge, &mut views, 30);
    views.save_game("good", "").unwrap();
    let before = checksum(&bridge);
    let good = fs::read(dir.join("slot-good.orrsave")).unwrap();

    let mut flipped = good.clone();
    let middle = flipped.len() / 2;
    flipped[middle] ^= 0x40;
    fs::write(dir.join("slot-flipped.orrsave"), &flipped).unwrap();
    fs::write(dir.join("slot-cut.orrsave"), &good[..good.len() - 20]).unwrap();
    fs::write(dir.join("slot-garbage.orrsave"), b"not a save at all").unwrap();
    fs::write(dir.join("slot-empty.orrsave"), b"").unwrap();
    for slot in ["flipped", "cut", "garbage", "empty"] {
        let error = views.load_game(slot).unwrap_err();
        assert!(matches!(error, SaveError::Corrupt(_)), "{slot}: {error}");
        assert!(views.take_load().is_none(), "{slot}: a refused load asks for no new session");
        assert!(views.save_game("noop", "").is_ok(), "the run still saves");
    }
    assert_eq!(views.load_game("missing").unwrap_err(), SaveError::NotFound("missing".to_string()));
    play(&mut bridge, &mut views, 1);
    assert_ne!(checksum(&bridge), before, "the run goes on after the refusals");
    assert_eq!(bridge.snapshot().unwrap().tick(), 31);
    // The good file next to the bad ones still loads, and the list tells which files are bad.
    views.load_game("good").unwrap();
    assert!(views.take_load().is_some());
    let list = SaveStore::at(&dir, "counter", 1).unwrap().list().unwrap();
    assert_eq!(list.iter().filter(|e| e.info.is_ok()).count(), 2, "good and noop");
    assert_eq!(list.iter().filter(|e| e.info.is_err()).count(), 4);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn an_unknown_format_version_is_refused_with_both_versions_named() {
    let dir = temp_dir("version");
    let (mut bridge, mut views) = (new_run(), views_in(&dir, "counter", 1));
    play(&mut bridge, &mut views, 5);
    views.save_game("old", "").unwrap();
    let path = dir.join("slot-old.orrsave");
    let mut bytes = fs::read(&path).unwrap();
    bytes[4..8].copy_from_slice(&(SAVE_VERSION + 1).to_le_bytes());
    fs::write(&path, bytes).unwrap();
    let error = views.load_game("old").unwrap_err();
    assert_eq!(error, SaveError::UnsupportedVersion { found: SAVE_VERSION + 1, supported: SAVE_VERSION });
    assert!(error.to_string().contains(&format!("version {}", SAVE_VERSION + 1)) && error.to_string().contains("not migrated"), "{error}");
    assert!(views.take_load().is_none());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_save_of_another_game_or_game_version_or_player_count_is_refused() {
    let dir = temp_dir("foreign");
    let (mut bridge, mut views) = (new_run(), views_in(&dir, "counter", 1));
    play(&mut bridge, &mut views, 5);
    views.save_game("slot", "").unwrap();

    let mut other_game = views_in(&dir, "other-game", 1);
    assert_eq!(other_game.load_game("slot").unwrap_err(), SaveError::ForeignGame { found: "counter".into(), expected: "other-game".into() });
    let mut newer = views_in(&dir, "counter", 2);
    assert_eq!(newer.load_game("slot").unwrap_err(), SaveError::GameVersion { found: 1, expected: 2 });
    let mut two_players = Views::<Counter>::new();
    two_players.enable_saves(SaveStore::at(&dir, "counter", 1).unwrap(), 2, RATE);
    assert!(matches!(two_players.load_game("slot").unwrap_err(), SaveError::Mismatch(_)));
    let mut other_rate = Views::<Counter>::new();
    other_rate.enable_saves(SaveStore::at(&dir, "counter", 1).unwrap(), 1, 30);
    assert!(matches!(other_rate.load_game("slot").unwrap_err(), SaveError::Mismatch(_)));
    for views in [&mut other_game, &mut newer, &mut two_players, &mut other_rate] {
        assert!(views.take_load().is_none());
    }
    // The save itself is fine for the run it belongs to.
    assert!(views.load_game("slot").is_ok());
    let _ = fs::remove_dir_all(dir);
}

/// A frame of the right file shape but of another game: the registry does not match, so the load stops
/// before it can start a session.
#[test]
fn a_frame_of_a_game_with_other_types_is_refused_even_with_a_matching_header() {
    #[repr(C)]
    #[derive(Clone, Copy, Pod, Zeroable)]
    struct Other {
        x: u64,
    }
    struct Stranger;
    impl Game for Stranger {
        type Input = Step;
        type Command = ();
        type Event = Mark;
        type Config = ();
        fn register(builder: &mut ComponentRegistryBuilder) {
            builder.register_component::<Other>("Other");
        }
        fn setup(_frame: &mut Frame, _config: &()) {}
        fn systems() -> Vec<Box<dyn System<Self>>> {
            Vec::new()
        }
    }
    let dir = temp_dir("stranger");
    let session = PlaySession::<Stranger>::new(PlayConfig::new(1, SEED, RATE), ());
    SaveStore::at(&dir, "counter", 1).unwrap().write("theirs", "", session.frame(), 1, RATE).unwrap();
    let mut views = views_in(&dir, "counter", 1);
    assert!(matches!(views.load_game("theirs").unwrap_err(), SaveError::Corrupt(_)));
    assert!(views.take_load().is_none());
    let _ = fs::remove_dir_all(dir);
}

/// A slot file that is read-only (Windows) or in a folder without write permission (Unix) makes the next
/// write fail. The old save must still be there and load.
#[cfg(windows)]
#[test]
fn a_failed_write_keeps_the_old_save_loadable() {
    let dir = temp_dir("failed_write");
    let (mut bridge, mut views) = (new_run(), views_in(&dir, "counter", 1));
    play(&mut bridge, &mut views, 20);
    let saved = views.save_game("slot", "old").unwrap();
    let path = dir.join("slot-slot.orrsave");
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions.clone()).unwrap();
    play(&mut bridge, &mut views, 20);
    assert!(matches!(views.save_game("slot", "new"), Err(SaveError::Io(_))));
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    fs::set_permissions(&path, permissions).unwrap();
    let info = SaveStore::at(&dir, "counter", 1).unwrap().info("slot").unwrap();
    assert_eq!((info.label.as_str(), info.tick, info.checksum), ("old", saved.tick, saved.checksum));
    assert!(views.load_game("slot").is_ok());
    let leftovers = fs::read_dir(&dir).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().ends_with(".tmp")).count();
    assert_eq!(leftovers, 0);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_run_without_save_slots_refuses_with_a_reason_that_names_the_networked_case() {
    let mut views = Views::<Counter>::new();
    let mut bridge = new_run();
    play(&mut bridge, &mut views, 3);
    let error = views.save_game("a", "").unwrap_err();
    assert!(matches!(error, SaveError::Unavailable(_)), "{error}");
    assert!(error.to_string().contains("networked"), "the message names the networked case: {error}");
    assert!(matches!(views.load_game("a"), Err(SaveError::Unavailable(_))));
}

// ---- from a view: save and load calls, notices and checkpoints ----

#[derive(Default)]
struct Seen {
    notices: Vec<SaveNotice>,
    saved: Vec<Result<u64, String>>,
    loaded: Vec<Result<u64, String>>,
    listed: Vec<usize>,
}

/// A scene view that saves at tick 10, loads at tick 20, fails to load at tick 21 and reads the notice.
struct Slots {
    seen: Rc<RefCell<Seen>>,
}

impl SceneView<Counter> for Slots {
    fn on_update(&mut self, ctx: &mut ViewContext<'_, Counter>) {
        let mut seen = self.seen.borrow_mut();
        if let Some(notice) = ctx.save_notice() {
            if seen.notices.last() != Some(notice) {
                seen.notices.push(notice.clone());
            }
        }
        match ctx.tick() {
            10 => seen.saved.push(ctx.save_game("view", "from a view").map(|i| i.tick).map_err(|e| e.to_string())),
            20 => {
                seen.loaded.push(ctx.load_game("view").map(|i| i.tick).map_err(|e| e.to_string()));
                seen.listed.push(ctx.list_saves().unwrap().len());
            }
            21 => seen.loaded.push(ctx.load_game("nothing").map(|i| i.tick).map_err(|e| e.to_string())),
            _ => {}
        }
    }
}

#[test]
fn a_view_saves_loads_lists_and_reads_the_notice() {
    let dir = temp_dir("view");
    let seen = Rc::new(RefCell::new(Seen::default()));
    let (mut bridge, mut views) = (new_run(), views_in(&dir, "counter", 1));
    views.scene_view(Slots { seen: seen.clone() });
    play(&mut bridge, &mut views, 25);
    let seen = seen.borrow();
    assert_eq!(seen.saved, vec![Ok(10)]);
    assert_eq!(seen.loaded[0], Ok(10));
    assert!(seen.loaded[1].as_ref().unwrap_err().contains("no save in slot"));
    assert_eq!(seen.listed, vec![1]);
    assert!(matches!(seen.notices[0], SaveNotice::Saved { tick: 10, .. }));
    assert!(seen.notices.iter().any(|n| matches!(n, SaveNotice::Loaded { tick: 10, .. })));
    assert!(matches!(seen.notices.last(), Some(SaveNotice::Failed(_))), "the last call failed");
    drop(seen);
    assert!(views.take_load().is_some(), "the accepted load waits for the end of the frame");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_checkpoint_event_saves_into_its_slot_with_the_label_of_the_game() {
    let dir = temp_dir("checkpoint");
    let (mut bridge, mut views) = (new_run(), views_in(&dir, "counter", 1));
    views.checkpoint_on("checkpoint", |mark: &Mark| (mark.level >= 1).then(|| format!("Stage {}", mark.level)));
    play(&mut bridge, &mut views, 24);
    assert!(SaveStore::at(&dir, "counter", 1).unwrap().list().unwrap().is_empty(), "no checkpoint before the event");
    play(&mut bridge, &mut views, 1);
    let store = SaveStore::at(&dir, "counter", 1).unwrap();
    let info = store.info("checkpoint").unwrap();
    assert_eq!((info.label.as_str(), info.tick), ("Stage 1", 25));
    play(&mut bridge, &mut views, 25);
    let info = store.info("checkpoint").unwrap();
    assert_eq!((info.label.as_str(), info.tick), ("Stage 2", 50), "the next checkpoint replaces the slot");
    // The checkpoint loads like any save.
    let mut loaded = load_into_new_bridge(&mut views, "checkpoint");
    assert_eq!(loaded.poll_view().snapshot.expect("a first snapshot").tick(), 50);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn views_see_a_replaced_session_as_a_resync_and_forget_the_old_events() {
    struct Count {
        resyncs: Rc<RefCell<u32>>,
        events: Rc<RefCell<Vec<EventState>>>,
    }
    impl SceneView<Counter> for Count {
        fn on_resync(&mut self, _ctx: &mut ViewContext<'_, Counter>) {
            *self.resyncs.borrow_mut() += 1;
        }
        fn on_event(&mut self, _ctx: &mut ViewContext<'_, Counter>, _event: &Mark, state: EventState) {
            self.events.borrow_mut().push(state);
        }
    }
    let dir = temp_dir("replaced");
    let (resyncs, events) = (Rc::new(RefCell::new(0)), Rc::new(RefCell::new(Vec::new())));
    let (mut bridge, mut views) = (new_run(), views_in(&dir, "counter", 1));
    views.scene_view(Count { resyncs: resyncs.clone(), events: events.clone() });
    play(&mut bridge, &mut views, 30);
    views.save_game("s", "").unwrap();
    play(&mut bridge, &mut views, 30);
    assert_eq!((*resyncs.borrow(), events.borrow().len()), (0, 2), "two marks so far (ticks 25 and 50)");

    let mut bridge = load_into_new_bridge(&mut views, "s");
    views.session_replaced();
    play(&mut bridge, &mut views, 1);
    assert_eq!(*resyncs.borrow(), 1, "the scene views were told once, with the first frame of the new session");
    play(&mut bridge, &mut views, 30);
    assert_eq!(*resyncs.borrow(), 1);
    assert_eq!(events.borrow().len(), 3, "the mark at tick 50 comes again in the new timeline");
    let _ = fs::remove_dir_all(dir);
}
