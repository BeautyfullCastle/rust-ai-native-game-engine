//! Thin browser ABI: all economy, combat, spawning and AI remain in DefenseGame.
use orr_games::coop_defense_game::*;
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
}

/// Reset the actual engine simulation. The browser supplies only a seed.
#[no_mangle]
pub extern "C" fn defense_init(seed: u32) {
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
            sim: Simulation::new(DefenseConfig { seed }, TICK_RATE, u64::from(seed)),
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
        write!(out, "{{\"phase\":{},\"tick\":{},\"wave\":{},\"hp\":{},\"gold\":{:?},\"investment\":{:?},\"upgrades\":{:?},\"kills\":{},\"leaks\":{},\"summons\":{},\"merges\":{},\"units\":[", r.phase,r.tick,r.wave,r.reactor_hp,r.gold,r.investment,r.upgrades,r.kills,r.leaks,r.summons,r.merges).unwrap();
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
