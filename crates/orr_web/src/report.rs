//! What the page reads from the client: a JSON report (the proof of
//! determinism: verified checksums) and the arena's drawing data.

use orr_proto::Link;
use orr_session::{ClientState, RelayClient};
use orr_sim::Game;
use orr_testgame::{Arena, Bullet, PlayerTag, Position};

/// Frame-format-bound build id of the arena sample; the room's build hash is
/// `build_hash_of(ARENA_BUILD_ID, 0)` (`orr_server --game arena`).
pub const ARENA_BUILD_ID: u64 = orr_sim::frame_build_id(0x0A2E_4A00_0001);
/// Frame-format-bound build id of the physics sample (`orr_server --game physics`).
pub const PHYSICS_BUILD_ID: u64 = orr_sim::frame_build_id(0x0A2E_4A00_0002);

fn state_name(s: &ClientState) -> String {
    match s {
        ClientState::Rejected(r) => format!("rejected: {r:?}"),
        ClientState::Failed(m) => format!("failed: {m}"),
        other => format!("{other:?}"),
    }
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The client's numbers as one JSON object. `checksums` are the verified
/// `[tick, "hex"]` pairs (hex strings: 64-bit values do not survive JSON numbers).
pub fn client_report_json<G: Game, L: Link>(client: &RelayClient<G, L>, transport: &str) -> String {
    let (cs, ss) = (client.stats(), client.source_stats());
    let session = client.session();
    let sums: Vec<String> =
        session.map_or_else(Vec::new, |s| s.checksums().iter().map(|(t, c)| format!("[{t},\"{c:016x}\"]")).collect());
    format!(
        concat!(
            "{{\"state\":{},\"transport\":{},\"slot\":{},\"rtt_us\":{},\"delay\":{},\"rollbacks\":{},",
            "\"resim_ticks\":{},\"max_prediction_depth\":{},\"stall_episodes\":{},\"repeats\":{},",
            "\"desyncs\":{},\"decode_errors\":{},\"head_tick\":{},\"verified_tick\":{},\"checksums\":[{}]}}"
        ),
        json_str(&state_name(client.state())),
        json_str(transport),
        client.welcome().map_or("null".to_string(), |w| w.slot.to_string()),
        client.srtt_us(),
        client.delay(),
        session.map_or(0, |s| s.rollback_count()),
        cs.resim_ticks,
        cs.max_prediction_depth,
        cs.stall_episodes,
        ss.own_repeated,
        cs.desyncs,
        ss.decode_errors,
        session.map_or(0, |s| s.head_tick()),
        session.map_or(0, |s| s.verified_tick()),
        sums.join(",")
    )
}

/// Drawing data of the predicted frame, four integers per entity:
/// `kind` (0 player, 1 bullet), `slot` (the player's or the bullet's owner),
/// then `x` and `y` in world units (the fixed-point value, rounded down).
pub fn render_arena<L: Link>(client: &RelayClient<Arena, L>) -> Vec<i32> {
    let Some(session) = client.session() else { return Vec::new() };
    let frame = session.predicted_frame();
    let (entities, positions) = frame.dense::<Position>();
    let mut out = Vec::with_capacity(entities.len() * 4);
    for (&e, p) in entities.iter().zip(positions) {
        let (kind, slot) = if let Some(t) = frame.get::<PlayerTag>(e) {
            (0, t.slot)
        } else if let Some(b) = frame.get::<Bullet>(e) {
            (1, b.owner_slot)
        } else {
            continue;
        };
        out.extend_from_slice(&[kind, slot as i32, (p.pos.x.0 >> 16) as i32, (p.pos.y.0 >> 16) as i32]);
    }
    out
}
