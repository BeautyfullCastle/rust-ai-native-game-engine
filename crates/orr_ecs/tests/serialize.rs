mod common;
use common::*;
use orr_ecs::{ComponentRegistryBuilder, Frame, FrameDecodeError};

/// Builds a frame that exercises every serialized part: despawned slots with
/// bumped versions and a non-empty free list, sparse component sets, a set
/// singleton, freed and live `FrameList`s, and a non-zero tick.
fn build_frame() -> Frame {
    let mut f = Frame::new(test_registry());
    let mut es = Vec::new();
    for i in 0..200i64 {
        let e = f.spawn();
        f.add(e, Pos { x: i, y: -i });
        if i % 2 == 0 {
            f.add(e, Vel { x: i * 3, y: 1 });
        }
        if i % 5 == 0 {
            f.add(e, Health { hp: i as i32, max: 100 });
        }
        if i % 7 == 0 {
            f.add(e, Tag { value: i as u32 });
        }
        es.push(e);
    }
    for i in (0..200).step_by(3) {
        f.despawn(es[i]);
    }
    for i in 0..10 {
        let e = f.spawn();
        f.add(e, Pos { x: 1000 + i, y: 0 });
    }
    f.set_singleton(GameConfig { gravity: -981, max_players: 4 });
    let keep = f.alloc_list::<Score>();
    let gone = f.alloc_list::<Score>();
    let other = f.alloc_list::<Score>();
    for v in 0..5 {
        f.list_push(keep, Score { value: v });
        f.list_push(gone, Score { value: v * 10 });
    }
    f.list_push(other, Score { value: 77 });
    f.list_free(gone);
    f.set_tick(42);
    f
}

/// One deterministic mutation round touching every structure.
fn step(f: &mut Frame, i: i64) {
    let mut es = Vec::new();
    f.query::<(&Pos,)>().for_each(|e, _| es.push(e));
    f.query::<(&mut Pos, &Vel)>().for_each(|_, (p, v)| {
        p.x += v.x;
        p.y += v.y;
    });
    for (n, &e) in es.iter().enumerate() {
        match (n as i64 + i) % 6 {
            0 => {
                f.despawn(e);
            }
            1 => {
                f.add(e, Vel { x: i, y: n as i64 });
            }
            2 => {
                f.remove::<Vel>(e);
            }
            _ => {}
        }
    }
    let e = f.spawn();
    f.add(e, Pos { x: i, y: i });
    let l = f.alloc_list::<Score>();
    f.list_push(l, Score { value: i });
    f.singleton_mut::<GameConfig>().gravity += i;
    f.set_tick(f.tick() + 1);
}

#[test]
fn round_trip_keeps_checksum_and_bytes() {
    let f = build_frame();
    let bytes = f.to_bytes();
    let g = Frame::from_bytes(test_registry(), &bytes).expect("decode");
    assert_eq!(g.checksum(), f.checksum());
    assert_eq!(g.tick(), 42);
    assert_eq!(g.alive_count(), f.alive_count());
    assert_eq!(g.singleton::<GameConfig>(), f.singleton::<GameConfig>());
    assert_eq!(g.to_bytes(), bytes, "re-serializing the restored frame must reproduce the bytes");
}

#[test]
fn restored_frame_continues_identically() {
    let mut a = build_frame();
    let mut b = Frame::from_bytes(test_registry(), &a.to_bytes()).unwrap();
    for i in 0..60 {
        step(&mut a, i);
        step(&mut b, i);
        assert_eq!(a.checksum(), b.checksum(), "diverged at step {i}");
    }
}

#[test]
fn empty_frame_round_trips() {
    let f = Frame::new(test_registry());
    let g = Frame::from_bytes(test_registry(), &f.to_bytes()).unwrap();
    assert_eq!(g.checksum(), f.checksum());
}

#[test]
fn identical_frames_serialize_identically() {
    // Separate registries and separate build runs.
    assert_eq!(build_frame().to_bytes(), build_frame().to_bytes());
}

#[test]
fn every_truncation_is_an_error() {
    let bytes = build_frame().to_bytes();
    for len in 0..bytes.len() {
        assert!(Frame::from_bytes(test_registry(), &bytes[..len]).is_err(), "prefix of {len} bytes decoded");
    }
}

#[test]
fn every_single_byte_corruption_is_an_error() {
    let bytes = build_frame().to_bytes();
    for i in 0..bytes.len() {
        let mut bad = bytes.clone();
        bad[i] ^= 0x5a;
        assert!(Frame::from_bytes(test_registry(), &bad).is_err(), "flipping byte {i} decoded");
    }
}

#[test]
fn header_and_trailer_errors() {
    let bytes = build_frame().to_bytes();

    let mut bad = bytes.clone();
    bad[0] = b'X';
    assert_eq!(Frame::from_bytes(test_registry(), &bad).err(), Some(FrameDecodeError::BadMagic));

    let mut bad = bytes.clone();
    bad[4..8].copy_from_slice(&99u32.to_le_bytes());
    assert_eq!(Frame::from_bytes(test_registry(), &bad).err(), Some(FrameDecodeError::UnsupportedVersion(99)));

    let mut bad = bytes.clone();
    bad.push(0);
    assert_eq!(Frame::from_bytes(test_registry(), &bad).err(), Some(FrameDecodeError::TrailingBytes));

    assert!(matches!(Frame::from_bytes(test_registry(), &[]), Err(FrameDecodeError::Truncated)));
}

#[test]
fn mismatched_registry_is_rejected() {
    let bytes = build_frame().to_bytes();

    // Same types, different registration order.
    let mut b = ComponentRegistryBuilder::new();
    b.register_component::<Vel>("Vel");
    b.register_component::<Pos>("Pos");
    b.register_component::<Health>("Health");
    b.register_component::<Tag>("Tag");
    b.register_singleton::<GameConfig>("GameConfig");
    b.register_list::<Score>("Score");
    assert!(matches!(
        Frame::from_bytes(b.build(), &bytes),
        Err(FrameDecodeError::SchemaEntry { kind: "component", index: 0 })
    ));

    // Fewer types.
    let empty = ComponentRegistryBuilder::new().build();
    assert!(matches!(
        Frame::from_bytes(empty, &bytes),
        Err(FrameDecodeError::SchemaCount { kind: "component", expected: 0, found: 4 })
    ));
}

#[test]
fn forged_huge_count_does_not_allocate_or_panic() {
    // Empty registry layout: magic, version, tick, three zero counts, then
    // the allocator's `alive_count` and slot count (at byte 32).
    let empty = ComponentRegistryBuilder::new().build();
    let mut bytes = Frame::new(empty.clone()).to_bytes();
    bytes[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(Frame::from_bytes(empty, &bytes).err(), Some(FrameDecodeError::Truncated));
}

#[test]
fn deterministic_garbage_never_panics() {
    // xorshift; test-side only, not sim code.
    let mut s = 0x9e37_79b9_7f4a_7c15u64;
    for _ in 0..500 {
        let len = {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s % 200) as usize
        };
        let junk: Vec<u8> = (0..len)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                s as u8
            })
            .collect();
        assert!(Frame::from_bytes(test_registry(), &junk).is_err());
    }
}
