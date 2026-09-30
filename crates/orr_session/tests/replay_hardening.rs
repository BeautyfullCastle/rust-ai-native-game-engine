//! The `.orrp` parser reads untrusted files: forged counts must not drive
//! allocation, and malformed input must return an error, never panic.
use orr_fp::{FrameRng, FP};
use orr_session::{replay_verify, ReplayError, ReplayHeader, ReplayReader, ReplayWriter};
use orr_sim::{PlayerSlot, Simulation, TickInputs};
use orr_testgame::{Arena, ArenaConfig, ArenaInput, SpawnBulletCmd, FIRE};

const HEADER_LEN: usize = 4 + 4 + (4 + "arena".len()) + 8 + 8 + 1 + 4 + 4;

fn cfg() -> ArenaConfig {
    ArenaConfig { player_count: 2 }
}

/// A valid replay with commands, checksums and keyframes.
fn valid_replay() -> Vec<u8> {
    let mut sim = Simulation::<Arena>::new(cfg(), 60, 9);
    let mut writer = ReplayWriter::<Arena>::new(ReplayHeader {
        format_version: 2,
        game_id: "arena".to_string(),
        build_hash: 0,
        seed: 9,
        player_count: 2,
        tick_rate: 60,
        input_size: std::mem::size_of::<ArenaInput>() as u32,
    })
    .with_keyframe_interval(20);
    let mut rng = FrameRng::new(3);
    for tick in 1..=60u64 {
        let mut ti = TickInputs::<ArenaInput, SpawnBulletCmd>::new(tick, 2);
        let mut inputs = Vec::new();
        let mut cmds = Vec::new();
        for slot in 0..2u8 {
            let fire = rng.next_u32() % 3 == 0;
            let input = ArenaInput::new(FP::from_int(rng.range_i32(-1, 2)), FP::from_int(0), fire);
            ti.set_input(PlayerSlot(slot), input);
            inputs.push(input);
            if input.buttons & FIRE != 0 {
                cmds.push((PlayerSlot(slot), SpawnBulletCmd { owner: slot as u32 }));
            }
        }
        ti.set_commands(cmds.clone());
        sim.step(&ti);
        writer.record_tick(tick, &inputs, &cmds);
        writer.maybe_record_keyframe(sim.frame());
        if tick % 10 == 0 {
            writer.record_checksum(tick, sim.checksum());
        }
    }
    writer.finish()
}

/// Wraps an uncompressed body in a valid header and lz4 block.
fn file_with_body(body: &[u8]) -> Vec<u8> {
    let valid = valid_replay();
    let compressed = lz4_flex::block::compress_prepend_size(body);
    let mut out = valid[..HEADER_LEN].to_vec();
    out.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
    out.extend_from_slice(&compressed);
    out
}

fn body_of(file: &[u8]) -> Vec<u8> {
    let len = u32::from_le_bytes(file[HEADER_LEN..HEADER_LEN + 4].try_into().unwrap()) as usize;
    lz4_flex::block::decompress_size_prepended(&file[HEADER_LEN + 4..HEADER_LEN + 4 + len]).unwrap()
}

fn parse(bytes: &[u8]) -> Result<ReplayReader<Arena>, ReplayError> {
    ReplayReader::<Arena>::parse(bytes)
}

#[test]
fn forged_counts_are_errors_not_allocations() {
    let u32max = u32::MAX.to_le_bytes();

    // One tick whose command count claims 4 billion commands. Sized by that
    // count this would try to reserve tens of GB.
    let mut body = 1u32.to_le_bytes().to_vec();
    body.extend_from_slice(&1u64.to_le_bytes()); // tick
    body.extend_from_slice(&0u32.to_le_bytes()); // change mask: no inputs
    body.extend_from_slice(&u32max); // command count
    assert!(matches!(parse(&file_with_body(&body)), Err(ReplayError::Truncated)));

    // Tick count.
    assert!(matches!(parse(&file_with_body(&u32max)), Err(ReplayError::Truncated)));

    // Checksum count.
    let mut body = 0u32.to_le_bytes().to_vec(); // no ticks
    body.extend_from_slice(&u32max);
    assert!(matches!(parse(&file_with_body(&body)), Err(ReplayError::Truncated)));

    // Keyframe count, and a keyframe length.
    let mut body = 0u32.to_le_bytes().to_vec();
    body.extend_from_slice(&0u32.to_le_bytes()); // no checksums
    let mut forged_count = body.clone();
    forged_count.extend_from_slice(&u32max);
    assert!(matches!(parse(&file_with_body(&forged_count)), Err(ReplayError::Truncated)));
    let mut forged_len = body;
    forged_len.extend_from_slice(&1u32.to_le_bytes());
    forged_len.extend_from_slice(&5u64.to_le_bytes());
    forged_len.extend_from_slice(&u32max);
    assert!(matches!(parse(&file_with_body(&forged_len)), Err(ReplayError::Truncated)));

    // Header lengths: the compressed block and the game id string.
    let valid = valid_replay();
    let mut bad = valid.clone();
    bad[HEADER_LEN..HEADER_LEN + 4].copy_from_slice(&u32max);
    assert!(matches!(parse(&bad), Err(ReplayError::Truncated)));
    let mut bad = valid.clone();
    bad[8..12].copy_from_slice(&u32max);
    assert!(matches!(parse(&bad), Err(ReplayError::Truncated)));

    // The lz4 size prefix: claims 4 GB from a few bytes.
    let mut bad = valid.clone();
    bad[HEADER_LEN + 4..HEADER_LEN + 8].copy_from_slice(&u32max);
    assert!(matches!(parse(&bad), Err(ReplayError::Decompress(_))));
}

#[test]
fn oversized_player_count_rejected() {
    let mut bad = valid_replay();
    let player_count_at = 4 + 4 + (4 + "arena".len()) + 8 + 8;
    bad[player_count_at] = 255;
    assert!(matches!(parse(&bad), Err(ReplayError::BadPlayerCount(255))));
}

#[test]
fn forged_tick_numbers_do_not_hang() {
    // A recorded tick number of u64::MAX must not turn seek or verify into
    // a counting loop over the whole range.
    let mut body = 1u32.to_le_bytes().to_vec();
    body.extend_from_slice(&u64::MAX.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes()); // mask
    body.extend_from_slice(&0u32.to_le_bytes()); // commands
    body.extend_from_slice(&0u32.to_le_bytes()); // checksums
    body.extend_from_slice(&0u32.to_le_bytes()); // keyframes
    body.extend_from_slice(&0u32.to_le_bytes()); // debug commands
    let file = file_with_body(&body);
    let reader = parse(&file).expect("well-formed");
    assert_eq!(reader.seek(cfg(), u64::MAX).expect("seek").tick(), 1);
    assert!(replay_verify::<Arena>(&file, cfg()).expect("verify").ok());
}

#[test]
fn every_truncation_is_an_error() {
    let valid = valid_replay();
    assert!(parse(&valid).is_ok());
    for len in 0..valid.len() {
        assert!(parse(&valid[..len]).is_err(), "prefix of {len} bytes parsed");
    }
}

#[test]
fn random_bytes_never_panic() {
    let mut rng = FrameRng::new(0xBAD_F00D);
    for _ in 0..3000 {
        let len = (rng.next_u32() % 300) as usize;
        let junk: Vec<u8> = (0..len).map(|_| rng.next_u32() as u8).collect();
        let _ = parse(&junk);
        // Past the magic, so the fields behind it get exercised too.
        let mut with_magic = b"ORRP".to_vec();
        with_magic.extend_from_slice(&(1 + rng.next_u32() % 2).to_le_bytes());
        with_magic.extend_from_slice(&junk);
        let _ = parse(&with_magic);
    }
}

#[test]
fn mutated_files_never_panic() {
    let valid = valid_replay();
    let body = body_of(&valid);
    let mut rng = FrameRng::new(0xC0DE);
    let mut parsed = 0u32;
    for round in 0..4000u32 {
        // Damage the compressed file itself, or the body under a fresh
        // (valid) lz4 wrapper so the record parsing sees the damage.
        let bad = if round % 2 == 0 {
            let mut f = valid.clone();
            for _ in 0..1 + rng.next_u32() % 4 {
                let at = (rng.next_u32() as usize) % f.len();
                f[at] = rng.next_u32() as u8;
            }
            f
        } else {
            let mut b = body.clone();
            for _ in 0..1 + rng.next_u32() % 4 {
                let at = (rng.next_u32() as usize) % b.len();
                b[at] = rng.next_u32() as u8;
            }
            file_with_body(&b)
        };
        if parse(&bad).is_ok() {
            parsed += 1;
        }
    }
    // Some mutants must survive, or the record parsing was never reached.
    assert!(parsed > 0, "mutations were too destructive to test the record parser");
}

#[test]
fn seek_reaches_every_tick_including_keyframes() {
    // Keyframe ticks are seek targets that need no resimulation.
    let reader = parse(&valid_replay()).unwrap();
    for tick in 0..=reader.last_tick() {
        assert_eq!(reader.seek(cfg(), tick).unwrap().tick(), tick);
    }
}

#[test]
fn forged_debug_command_tables_are_errors() {
    let u32max = u32::MAX.to_le_bytes();
    // An empty recording (no ticks, checksums or keyframes) plus a debug table.
    let empty = || {
        let mut body = 0u32.to_le_bytes().to_vec();
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body
    };
    let mut forged_count = empty();
    forged_count.extend_from_slice(&u32max);
    assert!(matches!(parse(&file_with_body(&forged_count)), Err(ReplayError::Truncated)));

    let mut forged_len = empty();
    forged_len.extend_from_slice(&1u32.to_le_bytes());
    forged_len.extend_from_slice(&3u64.to_le_bytes());
    forged_len.extend_from_slice(&u32max);
    assert!(matches!(parse(&file_with_body(&forged_len)), Err(ReplayError::Truncated)));

    // A command that does not decode.
    let mut bad_command = empty();
    bad_command.extend_from_slice(&1u32.to_le_bytes());
    bad_command.extend_from_slice(&3u64.to_le_bytes());
    bad_command.extend_from_slice(&2u32.to_le_bytes());
    bad_command.extend_from_slice(&[99, 99]);
    assert!(matches!(parse(&file_with_body(&bad_command)), Err(ReplayError::BadCommand)));
}
