//! Thin browser ABI: all economy, combat, spawning and AI remain in DefenseGame.
use orr_games::coop_defense_game::*;
use orr_sim::{PlayerSlot, Simulation, TickInputs};
use std::cell::RefCell;
use std::fmt::Write;

struct Client {
    sim: Simulation<DefenseGame>,
    json: String,
}
thread_local! { static CLIENT: RefCell<Option<Client>> = const { RefCell::new(None) }; }

/// Reset the actual engine simulation. The browser supplies only a seed.
#[no_mangle]
pub extern "C" fn defense_init(seed: u32) {
    CLIENT.with(|slot| {
        *slot.borrow_mut() = Some(Client {
            sim: Simulation::new(DefenseConfig { seed }, TICK_RATE, u64::from(seed)),
            json: String::new(),
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
            client.sim.step(&inputs);
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
            write!(out,"{comma}[{},{},{},{},{},{},{}]",u.board,u.cell,u.role,u.tier,u.cooldown,x,y).unwrap(); comma=",";
        }
        out.push_str("],\"enemies\":["); comma="";
        for e in enemies(frame).into_iter().filter(|e| e.active != 0) {
            let (x,y)=enemy_position(e.lane,e.progress);
            write!(out,"{comma}[{},{},{},{},{},{},{},{}]",e.id,e.lane,e.kind,e.hp,e.max_hp,e.slow_ticks,x,y).unwrap(); comma=",";
        }
        out.push_str("],\"projectiles\":["); comma="";
        for p in projectiles(frame).into_iter().filter(|p| p.active != 0) {
            write!(out,"{comma}[{},{},{},{},{},{}]",p.board,p.role,p.target_id,p.x,p.y,p.damage).unwrap(); comma=",";
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
        out.as_ptr()
    })
}
#[no_mangle]
pub extern "C" fn defense_snapshot_len() -> usize {
    CLIENT.with(|slot| slot.borrow().as_ref().map_or(0, |c| c.json.len()))
}
