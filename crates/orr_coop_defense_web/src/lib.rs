//! Thin browser ABI: all economy, combat, spawning and AI remain in DefenseGame.
use orr_games::coop_defense_game::*;
use orr_games::coop_defense_scene::{parse_scenario, MAX_SOURCE_BYTES};
use orr_sim::{PlayerSlot, Simulation, TickInputs};
use std::cell::{Cell, RefCell};
mod presentation;
use presentation::LatestTickEvents;
pub use presentation::MAX_PRESENTATION_EVENTS;
use std::fmt::Write;

struct Client {
    sim: Simulation<DefenseGame>,
    json: String,
    source: u64,
    events: LatestTickEvents<DefenseEvent>,
}
thread_local! {
    static CLIENT: RefCell<Option<Client>> = const { RefCell::new(None) };
    static SOURCE: Cell<u64> = const { Cell::new(0) };
    static SCENE_BYTES: RefCell<[u8; MAX_SOURCE_BYTES]> = const { RefCell::new([0; MAX_SOURCE_BYTES]) };
}

/// Reset the actual engine simulation. The browser supplies only a seed.
#[no_mangle]
pub extern "C" fn defense_init(seed: u32) {
    replace_client(DefenseConfig { seed, ..Default::default() });
}

/// Adapter-owned staging memory. Copy bytes immediately; never retain a memory
/// view across another export. No caller-selected pointer is accepted.
#[no_mangle]
pub extern "C" fn defense_scene_buffer() -> *mut u8 {
    SCENE_BYTES.with(|bytes| bytes.borrow_mut().as_mut_ptr())
}
#[no_mangle]
pub extern "C" fn defense_scene_capacity() -> usize { MAX_SOURCE_BYTES }

/// 0 = loaded, 1 = oversized, 2 = invalid UTF-8, 3 = rejected Scene.
/// All admission happens before replacing the current client/source identity.
#[no_mangle]
pub extern "C" fn defense_init_scene(length: usize) -> u32 {
    if length > MAX_SOURCE_BYTES { return 1; }
    let candidate = SCENE_BYTES.with(|bytes| {
        let bytes = bytes.borrow();
        let text = std::str::from_utf8(&bytes[..length]).map_err(|_| 2u32)?;
        parse_scenario(text).map_err(|_| 3)
    });
    match candidate {
        Ok(scenario) => {
            replace_client(DefenseConfig { scenario, ..Default::default() });
            0
        }
        Err(status) => status,
    }
}

fn replace_client(config: DefenseConfig) {
    let seed = config.seed;
    let source = SOURCE.with(|serial| {
        let next = serial
            .get()
            .checked_add(1)
            .expect("defense source identity exhausted");
        serial.set(next);
        next
    });
    CLIENT.with(|slot| {
        *slot.borrow_mut() = Some(Client {
            sim: Simulation::with_build_id(
                config,
                TICK_RATE,
                u64::from(seed),
                SCENARIO_BUILD_ID,
            ),
            json: String::new(),
            source,
            events: LatestTickEvents::new(),
        })
    });
}

/// One fixed tick and one input sample; AI partner is owned by the Rust game.
#[no_mangle]
pub extern "C" fn defense_step(action: u32, source_cell: u32, target_cell: u32, sequence: u32) {
    CLIENT.with(|slot| {
        if let Some(client) = slot.borrow_mut().as_mut() {
            let mut inputs = TickInputs::new(client.sim.tick(), 2);
            inputs.set_input(
                PlayerSlot(0),
                DefenseInput {
                    action,
                    source_cell,
                    target_cell,
                    sequence,
                },
            );
            client.events.replace(client.sim.step(&inputs));
        }
    });
}

/// Returns a UTF-8 snapshot pointer valid until the next snapshot/reset.
/// The JS caller copies these bytes immediately before calling another export.
#[no_mangle]
pub extern "C" fn defense_snapshot() -> *const u8 {
    CLIENT.with(|slot| {
        let mut borrow = slot.borrow_mut();
        let Some(client) = borrow.as_mut() else { return std::ptr::null() };
        let frame = client.sim.frame();
        let r = run(frame);
        let out = &mut client.json;
        out.clear();
        write!(out, "{{\"phase\":{},\"tick\":{},\"wave\":{},\"hp\":{},\"maxHp\":{},\"gold\":{:?},\"investment\":{:?},\"upgrades\":{:?},\"kills\":{},\"leaks\":{},\"summons\":{},\"merges\":{},\"units\":[", r.phase,r.tick,r.wave,r.reactor_hp,frame.singleton::<Scenario>().reactor_hp,r.gold,r.investment,r.upgrades,r.kills,r.leaks,r.summons,r.merges).unwrap();
        let mut comma = "";
        for u in units(frame).into_iter().filter(|u| u.active != 0) {
            let (x,y) = board_cell_position(u.board,u.cell);
            write!(out,"{comma}[{},{},{},{},{},{},{},{}]",u.board,u.cell,u.role,u.tier,u.cooldown,x,y,u.id).unwrap(); comma=",";
        }
        out.push_str("],\"enemies\":["); comma="";
        for e in enemies(frame).into_iter().filter(|e| e.active != 0) {
            let (x,y)=enemy_position(e.lane,e.progress);
            write!(out,"{comma}[{},{},{},{},{},{},{},{}]",e.id,e.lane,e.kind,e.hp,e.max_hp,e.slow_ticks,x,y).unwrap(); comma=",";
        }
        out.push_str("],\"projectiles\":["); comma="";
        for p in projectiles(frame).into_iter().filter(|p| p.active != 0) {
            write!(out,"{comma}[{},{},{},{},{},{},{},{}]",p.board,p.role,p.target_id,p.x,p.y,p.damage,p.id,p.source_unit_id).unwrap(); comma=",";
        }
        out.push_str("],\"cells\":["); comma="";
        for board in 0..2 { for cell in 0..CELLS_PER_BOARD {
            let (x,y)=board_cell_position(board,cell);
            write!(out,"{comma}[{board},{cell},{x},{y}]").unwrap(); comma=",";
        }}
        out.push_str("],\"paths\":[");
        for lane in 0..2 {
            if lane > 0 { out.push(','); }
            out.push('[');
            for (i,progress) in [0,500,900,1400,1600,2000].iter().enumerate() {
                if i>0 { out.push(','); }
                let (x,y)=enemy_position(lane,*progress);
                write!(out,"[{x},{y}]").unwrap();
            }
            out.push(']');
        }
        write!(out,"],\"costs\":[{},{},{}],\"checksum\":\"{:016x}\"}}",SUMMON_COST,upgrade_cost(r.upgrades[0]),investment_cost(r.investment[0]),client.sim.checksum()).unwrap();
        // Append an additive, versioned object without changing legacy fields.
        out.pop();
        let overflow = client.events.dropped != 0;
        write!(out, ",\"presentation\":{{\"version\":1,\"source\":\"{}\",\"generation\":{},\"tick\":\"{}\",\"delivery\":\"latest_tick\",\"status\":\"{}\",\"dropped\":{},\"fullResync\":{},\"windup\":false,\"events\":[", client.source, r.generation, client.sim.tick(), if overflow { "overflow" } else { "complete" }, client.events.dropped, overflow).unwrap();
        comma = "";
        for event in &client.events.events {
            let e = event.payload;
            let kind = e.event_kind().expect("known defense event kind").name();
            write!(out, "{comma}{{\"tick\":\"{}\",\"system\":{},\"seq\":{},\"kind\":\"{kind}\",\"generation\":{},\"unitId\":{},\"otherUnitId\":{},\"enemyId\":{},\"projectileId\":{},\"board\":{},\"role\":{},\"value\":{},\"hpAfter\":{},\"x\":{},\"y\":{}}}", event.key.tick, event.key.system_index, event.key.seq, e.generation, e.unit_id, e.other_unit_id, e.enemy_id, e.projectile_id, e.board, e.role, e.value, e.hp_after, e.x, e.y).unwrap();
            comma = ",";
        }
        out.push_str("]}}");
        out.as_ptr()
    })
}
#[no_mangle]
pub extern "C" fn defense_snapshot_len() -> usize {
    CLIENT.with(|slot| slot.borrow().as_ref().map_or(0, |c| c.json.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn json() -> String {
        defense_snapshot();
        CLIENT.with(|slot| slot.borrow().as_ref().unwrap().json.clone())
    }
    fn load(bytes: &[u8]) -> u32 {
        SCENE_BYTES.with(|buffer| buffer.borrow_mut()[..bytes.len()].copy_from_slice(bytes));
        defense_init_scene(bytes.len())
    }
    const AUTHORED: &str = "schema: orr.scene/1\nentities: {}\nsingletons: {CoopDefense::Scenario: {starting_gold: 250, reactor_hp: 75}}\n";
    #[test]
    fn saved_scene_matches_canonical_simulation_and_restart() {
        assert_eq!(load(AUTHORED.as_bytes()), 0);
        let config = DefenseConfig { scenario: parse_scenario(AUTHORED).unwrap(), ..Default::default() };
        let mut expected = Simulation::<DefenseGame>::with_build_id(config, TICK_RATE, u64::from(config.seed), SCENARIO_BUILD_ID);
        CLIENT.with(|slot| assert_eq!(slot.borrow().as_ref().unwrap().sim.checksum(), expected.checksum()));
        assert!(json().contains("\"hp\":75,\"maxHp\":75,\"gold\":[250, 250]"));
        for (action, sequence) in [(SUMMON, 1), (UPGRADE, 2), (RESTART, 3), (SUMMON, 4)] {
            let mut inputs = TickInputs::new(expected.tick(), 2);
            inputs.set_input(PlayerSlot(0), DefenseInput { action, source_cell: 0, target_cell: 0, sequence });
            expected.step(&inputs);
            defense_step(action, 0, 0, sequence);
            CLIENT.with(|slot| assert_eq!(slot.borrow().as_ref().unwrap().sim.checksum(), expected.checksum()));
        }
        assert!(json().contains("\"hp\":75,\"maxHp\":75"));
        println!("authored trace checksum: {:016x}", expected.checksum());
    }
    #[test]
    fn rejected_scene_preserves_entire_client() {
        defense_init(71825); defense_step(SUMMON, 0, 0, 1);
        let before = json();
        assert_eq!(defense_init_scene(MAX_SOURCE_BYTES + 1), 1);
        assert_eq!(json(), before);
        assert_eq!(load(&[0xff]), 2);
        assert_eq!(json(), before);
        for bad in ["", "schema: wrong", "schema: orr.scene/1\nentities: {e_00000001: {name: injected}}\n", "schema: orr.scene/1\nentities: {}\nsingletons: {CoopDefense::Scenario: {starting_gold: 1, reactor_hp: 0}}\n"] {
            assert_eq!(load(bad.as_bytes()), 3);
            assert_eq!(json(), before);
        }
    }
    #[test]
    fn imported_empty_scene_uses_default_seed_not_legacy_seed() {
        defense_init(DefenseConfig::default().seed);
        let expected = CLIENT.with(|slot| slot.borrow().as_ref().unwrap().sim.checksum());
        assert_eq!(load(b"schema: orr.scene/1\nentities: {}\n"), 0);
        CLIENT.with(|slot| assert_eq!(slot.borrow().as_ref().unwrap().sim.checksum(), expected));
        assert_eq!(defense_scene_capacity(), 65536);
        assert!(!defense_scene_buffer().is_null());
    }
    #[test]
    fn legacy_seed_trace_matches_native_simulation() {
        let seed = 71825;
        defense_init(seed);
        let config = DefenseConfig { seed, ..Default::default() };
        let mut expected = Simulation::<DefenseGame>::with_build_id(config, TICK_RATE, u64::from(seed), SCENARIO_BUILD_ID);
        let mut sequence = 0;
        for tick in 0..1200 {
            let (action, sequence) = if tick % 30 == 0 { sequence += 1; (SUMMON, sequence) } else { (0, 0) };
            let mut inputs = TickInputs::new(expected.tick(), 2);
            inputs.set_input(PlayerSlot(0), DefenseInput { action, source_cell: 0, target_cell: 0, sequence });
            expected.step(&inputs);
            defense_step(action, 0, 0, sequence);
        }
        CLIENT.with(|slot| assert_eq!(slot.borrow().as_ref().unwrap().sim.checksum(), expected.checksum()));
        println!("legacy native trace checksum: {:016x}", expected.checksum());
    }
    #[test]
    fn browser_tracks_scenario_build_identity() {
        defense_init(7);
        CLIENT.with(|slot| {
            assert_eq!(
                slot.borrow().as_ref().unwrap().sim.build_id(),
                SCENARIO_BUILD_ID
            );
        });
        assert_ne!(SCENARIO_BUILD_ID, 0);
    }
    #[test]
    fn repeated_snapshots_preserve_keys_and_reset_has_a_new_source() {
        defense_init(7);
        defense_step(SUMMON, 0, 0, 1);
        let first = json();
        assert_eq!(first, json());
        assert!(first.contains("\"kind\":\"summon\""));
        assert!(first.contains("\"tick\":\"1\",\"system\":0,\"seq\":0"));
        defense_step(SUMMON, 0, 0, 1);
        assert!(json().contains("\"events\":[]"));
        defense_step(RESTART, 0, 0, 2);
        assert!(json().contains("\"generation\":1,\"tick\":\"3\""));
        let old_source = CLIENT.with(|slot| slot.borrow().as_ref().unwrap().source);
        defense_init(7);
        CLIENT.with(|slot| {
            let borrow = slot.borrow();
            let c = borrow.as_ref().unwrap();
            assert_ne!(c.source, old_source);
            assert_eq!(run(c.sim.frame()).generation, 0);
            assert!(c.events.events.is_empty());
        });
    }
    #[test]
    fn overflow_is_explicit_and_never_serializes_partial_events() {
        defense_init(7);
        CLIENT.with(|slot| {
            let mut borrow = slot.borrow_mut();
            let c = borrow.as_mut().unwrap();
            c.events.replace(
                (0..=MAX_PRESENTATION_EVENTS)
                    .map(|seq| orr_sim::SimEvent {
                        key: orr_sim::EventKey::new(0, 0, seq as u32),
                        payload: DefenseEvent::default(),
                    })
                    .collect(),
            );
        });
        let snapshot = json();
        assert!(snapshot.contains("\"status\":\"overflow\",\"dropped\":1025,\"fullResync\":true"));
        assert!(snapshot.contains("\"events\":[]"));
    }
}
