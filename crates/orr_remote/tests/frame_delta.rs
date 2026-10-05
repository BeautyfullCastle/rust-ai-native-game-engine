//! Focused checks for the unconnected, in-memory ORRF v2 delta prototype.

#[path = "../src/frame_delta.rs"]
mod frame_delta;

use std::sync::Arc;

use frame_delta::{
    DecodeError, Decoder, DeltaError, EncodeError, Encoder, FrameRecord, FrameScope, FrameStamp,
    NeedFullReason,
};
use orr_ecs::{
    ComponentRegistry, ComponentRegistryBuilder, Entity, Frame, FrameDecodeError, FrameList,
};

const CAP: usize = 64 * 1024;
const SCOPE: FrameScope = FrameScope {
    stream_generation: 7,
    play_epoch: 11,
    timeline_epoch: 13,
};

struct Fixture {
    registry: Arc<ComponentRegistry>,
    frame: Frame,
    entity: Entity,
    list: FrameList<u8>,
}

fn fixture(tick: u64) -> Fixture {
    let mut builder = ComponentRegistryBuilder::new();
    builder.register_component::<u32>("Counter");
    builder.register_list::<u8>("Bytes");
    let registry = builder.build();
    let mut frame = Frame::new(Arc::clone(&registry));
    let entity = frame.spawn();
    assert_eq!(frame.add(entity, 0x1234_5678_u32), None);
    let list = frame.alloc_list::<u8>();
    for value in 0..512_u32 {
        frame.list_push(list, (value & 255) as u8);
    }
    frame.set_tick(tick);
    Fixture {
        registry,
        frame,
        entity,
        list,
    }
}

fn stamp(frame: &Frame, scope: FrameScope) -> FrameStamp {
    FrameStamp {
        scope,
        tick: frame.tick(),
        frame_checksum: frame.checksum(),
    }
}

fn full(frame: &Frame, scope: FrameScope) -> FrameRecord {
    FrameRecord::Full {
        target: stamp(frame, scope),
        bytes: Arc::from(frame.to_bytes()),
    }
}

fn assert_frame(expected: &Frame, actual: &Frame) {
    assert_eq!(actual.to_bytes(), expected.to_bytes());
    assert_eq!(actual.tick(), expected.tick());
    assert_eq!(actual.checksum(), expected.checksum());
}

fn assert_baseline(decoder: &Decoder, frame: &Frame, scope: FrameScope) {
    assert_eq!(decoder.baseline_stamp(), Some(stamp(frame, scope)));
    assert_eq!(decoder.retained_baseline_bytes(), frame.to_bytes().len());
}

// Frame deliberately has no Debug implementation, so Result::unwrap_err()
// cannot be used for decoder errors.
fn decode_error(decoder: &mut Decoder, record: &FrameRecord) -> DecodeError {
    match decoder.decode(record) {
        Ok(_) => panic!("malformed or incompatible record was accepted"),
        Err(error) => error,
    }
}

fn sparse_pair() -> (Fixture, Frame, FrameRecord, FrameRecord) {
    let fixture = fixture(10);
    let mut target = fixture.frame.clone();
    *target.get_mut::<u32>(fixture.entity).expect("Counter") ^= 1;
    target.set_tick(11);
    let mut encoder = Encoder::new(CAP, CAP);
    let first = encoder
        .encode(&fixture.frame, SCOPE)
        .expect("full baseline");
    let delta = encoder.encode(&target, SCOPE).expect("sparse record");
    assert!(matches!(&delta, FrameRecord::Delta { inserted, .. } if !inserted.is_empty()));
    (fixture, target, first, delta)
}

#[test]
fn forward_idle_frame_is_an_empty_insert_delta_and_restores_exact_bytes() {
    let fixture = fixture(10);
    let mut target = fixture.frame.clone();
    target.set_tick(11);
    let mut encoder = Encoder::new(CAP, CAP);
    let first = encoder.encode(&fixture.frame, SCOPE).expect("first frame");
    assert!(matches!(first, FrameRecord::Full { .. }));
    let next = encoder.encode(&target, SCOPE).expect("idle forward frame");
    match &next {
        FrameRecord::Delta {
            base,
            target: target_stamp,
            target_len,
            inserted,
            ..
        } => {
            assert_eq!(*base, stamp(&fixture.frame, SCOPE));
            assert_eq!(*target_stamp, stamp(&target, SCOPE));
            assert_eq!(*target_len, target.to_bytes().len() as u64);
            assert!(inserted.is_empty());
        }
        FrameRecord::Full { .. } => panic!("idle forward frame must use a delta"),
    }
    let mut decoder = Decoder::new(Arc::clone(&fixture.registry), CAP, CAP);
    assert_frame(
        &fixture.frame,
        &decoder.decode(&first).expect("decode first"),
    );
    assert_frame(&target, &decoder.decode(&next).expect("decode idle delta"));
    assert_baseline(&decoder, &target, SCOPE);
    assert_eq!(encoder.baseline_stamp(), Some(stamp(&target, SCOPE)));
    assert_eq!(encoder.retained_baseline_bytes(), target.to_bytes().len());
}

#[test]
fn sparse_component_edit_is_a_delta_and_restores_exact_bytes() {
    let (fixture, target, first, delta) = sparse_pair();
    if let FrameRecord::Delta { inserted, .. } = &delta {
        assert_eq!(inserted.len(), 1);
    }
    let mut decoder = Decoder::new(Arc::clone(&fixture.registry), CAP, CAP);
    decoder.decode(&first).expect("baseline");
    let decoded = decoder.decode(&delta).expect("component delta");
    assert_frame(&target, &decoded);
    assert_eq!(decoded.get::<u32>(fixture.entity), Some(&0x1234_5679_u32));
    assert_baseline(&decoder, &target, SCOPE);
}

#[test]
fn list_insertion_and_removal_deltas_restore_byte_exact_frames() {
    let fixture = fixture(10);
    let mut inserted = fixture.frame.clone();
    inserted.list_push(fixture.list, 0xa5);
    inserted.list_push(fixture.list, 0x5a);
    inserted.set_tick(11);
    let mut removed = inserted.clone();
    removed.list_clear(fixture.list);
    for value in 0..511_u32 {
        removed.list_push(fixture.list, (value & 255) as u8);
    }
    removed.set_tick(12);
    let mut encoder = Encoder::new(CAP, CAP);
    let first = encoder.encode(&fixture.frame, SCOPE).expect("baseline");
    let insertion = encoder.encode(&inserted, SCOPE).expect("insertion");
    let removal = encoder.encode(&removed, SCOPE).expect("removal");
    assert!(matches!(insertion, FrameRecord::Delta { .. }));
    assert!(matches!(removal, FrameRecord::Delta { .. }));
    let mut decoder = Decoder::new(Arc::clone(&fixture.registry), CAP, CAP);
    decoder.decode(&first).expect("decode baseline");
    assert_frame(
        &inserted,
        &decoder.decode(&insertion).expect("decode insertion"),
    );
    assert_frame(&removed, &decoder.decode(&removal).expect("decode removal"));
    assert_baseline(&decoder, &removed, SCOPE);
}

#[test]
fn high_change_small_baseline_uses_full_record_fallback() {
    let mut builder = ComponentRegistryBuilder::new();
    builder.register_component::<u8>("B");
    let registry = builder.build();
    let mut baseline = Frame::new(Arc::clone(&registry));
    baseline.set_tick(1);
    let mut target = baseline.clone();
    for value in 0..128_u8 {
        let entity = target.spawn();
        assert_eq!(target.add(entity, value), None);
    }
    target.set_tick(2);
    let mut encoder = Encoder::new(CAP, CAP);
    let first = encoder.encode(&baseline, SCOPE).expect("empty baseline");
    let changed = encoder.encode(&target, SCOPE).expect("large change");
    assert!(matches!(changed, FrameRecord::Full { .. }));
    let mut decoder = Decoder::new(registry, CAP, CAP);
    decoder.decode(&first).expect("decode baseline");
    assert_frame(&target, &decoder.decode(&changed).expect("decode fallback"));
}

#[test]
fn reconnect_play_timeline_and_nonforward_ticks_emit_full_resets() {
    let fixture = fixture(10);
    let mut encoder = Encoder::new(CAP, CAP);
    let mut decoder = Decoder::new(Arc::clone(&fixture.registry), CAP, CAP);
    let initial = encoder.encode(&fixture.frame, SCOPE).expect("initial");
    decoder.decode(&initial).expect("initial decode");
    let cases = [
        (10, SCOPE),
        (9, SCOPE),
        (
            10,
            FrameScope {
                stream_generation: 8,
                ..SCOPE
            },
        ),
        (
            11,
            FrameScope {
                stream_generation: 8,
                play_epoch: 12,
                ..SCOPE
            },
        ),
        (
            12,
            FrameScope {
                stream_generation: 8,
                play_epoch: 12,
                timeline_epoch: 14,
            },
        ),
    ];
    for (tick, scope) in cases {
        let mut target = fixture.frame.clone();
        target.set_tick(tick);
        let record = encoder.encode(&target, scope).expect("reset frame");
        assert!(matches!(record, FrameRecord::Full { .. }));
        assert_frame(&target, &decoder.decode(&record).expect("decode reset"));
        assert_eq!(encoder.baseline_stamp(), Some(stamp(&target, scope)));
        assert_baseline(&decoder, &target, scope);
    }
}

#[test]
fn missing_baseline_requires_full_before_delta_recovery() {
    let (fixture, target, first, delta) = sparse_pair();
    let mut decoder = Decoder::new(Arc::clone(&fixture.registry), CAP, CAP);
    assert_eq!(
        decode_error(&mut decoder, &delta),
        DecodeError::NeedFull(NeedFullReason::MissingBaseline)
    );
    assert_eq!(decoder.baseline_stamp(), None);
    assert_eq!(decoder.retained_baseline_bytes(), 0);
    decoder.decode(&first).expect("explicit full recovery");
    assert_frame(&target, &decoder.decode(&delta).expect("delta after full"));
}

#[test]
fn stale_out_of_order_and_replayed_deltas_preserve_baseline_until_full_recovery() {
    let fixture = fixture(10);
    let mut second = fixture.frame.clone();
    second.set_tick(11);
    let mut third = second.clone();
    third.set_tick(12);
    let mut encoder = Encoder::new(CAP, CAP);
    let first = encoder.encode(&fixture.frame, SCOPE).expect("first");
    let second_record = encoder.encode(&second, SCOPE).expect("second");
    let third_record = encoder.encode(&third, SCOPE).expect("third");
    assert!(matches!(second_record, FrameRecord::Delta { .. }));
    assert!(matches!(third_record, FrameRecord::Delta { .. }));
    let mut decoder = Decoder::new(Arc::clone(&fixture.registry), CAP, CAP);
    decoder.decode(&first).expect("first decode");
    assert_eq!(
        decode_error(&mut decoder, &third_record),
        DecodeError::NeedFull(NeedFullReason::StaleBase {
            expected: stamp(&second, SCOPE),
            got: stamp(&fixture.frame, SCOPE),
        })
    );
    assert_baseline(&decoder, &fixture.frame, SCOPE);
    assert_frame(
        &second,
        &decoder.decode(&second_record).expect("ordered second"),
    );
    assert_eq!(
        decode_error(&mut decoder, &second_record),
        DecodeError::NeedFull(NeedFullReason::StaleBase {
            expected: stamp(&fixture.frame, SCOPE),
            got: stamp(&second, SCOPE),
        })
    );
    assert_baseline(&decoder, &second, SCOPE);
    assert_frame(
        &third,
        &decoder.decode(&full(&third, SCOPE)).expect("full reset"),
    );
    assert_baseline(&decoder, &third, SCOPE);
}

#[test]
fn forged_scope_and_nonforward_delta_stamps_do_not_commit() {
    let (fixture, target_frame, first, delta) = sparse_pair();
    let mut decoder = Decoder::new(Arc::clone(&fixture.registry), CAP, CAP);
    decoder.decode(&first).expect("baseline");
    let mut mixed_scope = delta.clone();
    if let FrameRecord::Delta { target, .. } = &mut mixed_scope {
        target.scope.timeline_epoch += 1;
    }
    let mut other_scope = delta.clone();
    if let FrameRecord::Delta { base, target, .. } = &mut other_scope {
        base.scope.stream_generation += 1;
        target.scope = base.scope;
    }
    for record in [mixed_scope, other_scope] {
        assert_eq!(
            decode_error(&mut decoder, &record),
            DecodeError::NeedFull(NeedFullReason::ScopeChanged)
        );
        assert_baseline(&decoder, &fixture.frame, SCOPE);
    }
    for target_tick in [10, 9] {
        let mut nonforward = delta.clone();
        if let FrameRecord::Delta { target, .. } = &mut nonforward {
            target.tick = target_tick;
        }
        assert_eq!(
            decode_error(&mut decoder, &nonforward),
            DecodeError::NeedFull(NeedFullReason::NonForwardTick {
                base_tick: 10,
                target_tick,
            })
        );
        assert_baseline(&decoder, &fixture.frame, SCOPE);
    }
    assert_frame(
        &target_frame,
        &decoder.decode(&delta).expect("valid after rejection"),
    );
}

#[test]
fn malformed_ranges_checked_lengths_and_short_header_preserve_baseline() {
    let (fixture, target_frame, first, delta) = sparse_pair();
    let mut decoder = Decoder::new(Arc::clone(&fixture.registry), CAP, CAP);
    decoder.decode(&first).expect("baseline");
    let mut invalid_range = delta.clone();
    if let FrameRecord::Delta {
        prefix_len,
        suffix_len,
        ..
    } = &mut invalid_range
    {
        *prefix_len = fixture.frame.to_bytes().len() as u64;
        *suffix_len = 0;
    }
    let mut wrong_target_len = delta.clone();
    if let FrameRecord::Delta { target_len, .. } = &mut wrong_target_len {
        *target_len += 1;
    }
    let mut short_header = delta.clone();
    if let FrameRecord::Delta { target_len, .. } = &mut short_header {
        *target_len = 23;
    }
    for (record, reason) in [
        (invalid_range, DeltaError::BaseRange),
        (wrong_target_len, DeltaError::TargetLength),
        (short_header, DeltaError::FrameHeaderTooShort),
    ] {
        assert_eq!(
            decode_error(&mut decoder, &record),
            DecodeError::InvalidDelta(reason)
        );
        assert_baseline(&decoder, &fixture.frame, SCOPE);
    }
    let mut overflow = delta.clone();
    if let FrameRecord::Delta {
        prefix_len,
        suffix_len,
        ..
    } = &mut overflow
    {
        *prefix_len = u64::MAX;
        *suffix_len = 1;
    }
    let error = decode_error(&mut decoder, &overflow);
    if usize::BITS == 64 {
        assert_eq!(error, DecodeError::InvalidDelta(DeltaError::LengthOverflow));
    } else {
        assert_eq!(error, DecodeError::LengthOutOfRange);
    }
    assert_baseline(&decoder, &fixture.frame, SCOPE);
    assert_frame(
        &target_frame,
        &decoder.decode(&delta).expect("valid after malformed input"),
    );
}

#[test]
fn oversize_target_and_inconsistent_insertions_are_rejected_without_committing() {
    let (fixture, target_frame, first, delta) = sparse_pair();
    let cap = fixture.frame.to_bytes().len();
    let mut decoder = Decoder::new(Arc::clone(&fixture.registry), cap, cap);
    decoder.decode(&first).expect("exact frame cap");
    let mut oversize = delta.clone();
    if let FrameRecord::Delta { target_len, .. } = &mut oversize {
        *target_len = (cap + 1) as u64;
    }
    assert_eq!(
        decode_error(&mut decoder, &oversize),
        DecodeError::FrameTooLarge {
            actual: cap + 1,
            max: cap
        }
    );
    assert_baseline(&decoder, &fixture.frame, SCOPE);
    let mut address_space = delta.clone();
    if let FrameRecord::Delta { target_len, .. } = &mut address_space {
        *target_len = u64::MAX;
    }
    let error = decode_error(&mut decoder, &address_space);
    if usize::BITS == 64 {
        assert_eq!(
            error,
            DecodeError::FrameTooLarge {
                actual: usize::MAX,
                max: cap
            }
        );
    } else {
        assert_eq!(error, DecodeError::LengthOutOfRange);
    }
    let mut huge_insertion = delta.clone();
    if let FrameRecord::Delta { inserted, .. } = &mut huge_insertion {
        *inserted = vec![0; cap + 1];
    }
    assert_eq!(
        decode_error(&mut decoder, &huge_insertion),
        DecodeError::InvalidDelta(DeltaError::TargetLength)
    );
    assert_baseline(&decoder, &fixture.frame, SCOPE);
    assert_frame(
        &target_frame,
        &decoder.decode(&delta).expect("valid at exact cap"),
    );
}

#[test]
fn tampered_inserted_component_bytes_fail_checksum_without_committing() {
    let (fixture, target, first, delta) = sparse_pair();
    let mut corrupt = delta.clone();
    if let FrameRecord::Delta { inserted, .. } = &mut corrupt {
        assert_eq!(inserted.len(), 1);
        inserted[0] ^= 1;
    }
    let mut decoder = Decoder::new(Arc::clone(&fixture.registry), CAP, CAP);
    decoder.decode(&first).expect("baseline");
    assert!(matches!(
        decode_error(&mut decoder, &corrupt),
        DecodeError::FrameDecode(FrameDecodeError::ChecksumMismatch { .. })
    ));
    assert_baseline(&decoder, &fixture.frame, SCOPE);
    assert_frame(
        &target,
        &decoder.decode(&delta).expect("untampered recovery"),
    );
}

#[test]
fn full_checksum_tick_and_checksum_stamp_errors_preserve_baseline() {
    let fixture = fixture(10);
    let mut target_frame = fixture.frame.clone();
    target_frame.set_tick(11);
    let good = full(&target_frame, SCOPE);
    let mut corrupt = good.clone();
    if let FrameRecord::Full { bytes, .. } = &mut corrupt {
        let mut changed = bytes.to_vec();
        let last = changed.len() - 1;
        changed[last] ^= 1;
        *bytes = Arc::from(changed);
    }
    let mut decoder = Decoder::new(Arc::clone(&fixture.registry), CAP, CAP);
    decoder
        .decode(&full(&fixture.frame, SCOPE))
        .expect("baseline");
    assert!(matches!(
        decode_error(&mut decoder, &corrupt),
        DecodeError::FrameDecode(FrameDecodeError::ChecksumMismatch { .. })
    ));
    assert_baseline(&decoder, &fixture.frame, SCOPE);
    for change_tick in [true, false] {
        let mut bad_stamp = good.clone();
        let mut expected = stamp(&target_frame, SCOPE);
        if change_tick {
            expected.tick += 1;
        } else {
            expected.frame_checksum ^= 1;
        }
        if let FrameRecord::Full { target, .. } = &mut bad_stamp {
            *target = expected;
        }
        assert_eq!(
            decode_error(&mut decoder, &bad_stamp),
            DecodeError::StampMismatch {
                expected,
                actual_tick: target_frame.tick(),
                actual_checksum: target_frame.checksum(),
            }
        );
        assert_baseline(&decoder, &fixture.frame, SCOPE);
    }
    assert_frame(
        &target_frame,
        &decoder.decode(&good).expect("valid full reset"),
    );
}

#[test]
fn full_from_wrong_registry_does_not_replace_a_valid_local_baseline() {
    let fixture = fixture(10);
    let mut builder = ComponentRegistryBuilder::new();
    builder.register_component::<u32>("OtherCounter");
    builder.register_list::<u8>("Bytes");
    let registry = builder.build();
    let mut own_frame = Frame::new(Arc::clone(&registry));
    own_frame.set_tick(9);
    let mut decoder = Decoder::new(registry, CAP, CAP);
    decoder
        .decode(&full(&own_frame, SCOPE))
        .expect("own registry baseline");
    assert!(matches!(
        decode_error(&mut decoder, &full(&fixture.frame, SCOPE)),
        DecodeError::FrameDecode(FrameDecodeError::SchemaEntry {
            kind: "component",
            index: 0
        })
    ));
    assert_baseline(&decoder, &own_frame, SCOPE);
}

#[test]
fn exact_baseline_cap_retains_while_over_cap_and_zero_caps_clear_or_reject() {
    let fixture = fixture(10);
    let len = fixture.frame.to_bytes().len();
    for baseline_cap in [len, len - 1, 0] {
        let mut encoder = Encoder::new(len, baseline_cap);
        let record = encoder.encode(&fixture.frame, SCOPE).expect("frame at cap");
        let mut decoder = Decoder::new(Arc::clone(&fixture.registry), len, baseline_cap);
        assert_frame(
            &fixture.frame,
            &decoder.decode(&record).expect("frame at cap decode"),
        );
        if baseline_cap == len {
            assert_eq!(encoder.baseline_stamp(), Some(stamp(&fixture.frame, SCOPE)));
            assert_eq!(encoder.retained_baseline_bytes(), len);
            assert_baseline(&decoder, &fixture.frame, SCOPE);
        } else {
            assert_eq!(encoder.baseline_stamp(), None);
            assert_eq!(encoder.retained_baseline_bytes(), 0);
            assert_eq!(decoder.baseline_stamp(), None);
            assert_eq!(decoder.retained_baseline_bytes(), 0);
            let mut next = fixture.frame.clone();
            next.set_tick(11);
            assert!(matches!(
                encoder.encode(&next, SCOPE).expect("no cached baseline"),
                FrameRecord::Full { .. }
            ));
        }
    }
    let mut zero_encoder = Encoder::new(0, 0);
    assert_eq!(
        zero_encoder
            .encode(&fixture.frame, SCOPE)
            .expect_err("zero frame cap"),
        EncodeError::FrameTooLarge {
            actual: len,
            max: 0
        }
    );
    assert_eq!(zero_encoder.baseline_stamp(), None);
    let mut zero_decoder = Decoder::new(Arc::clone(&fixture.registry), 0, 0);
    assert_eq!(
        decode_error(&mut zero_decoder, &full(&fixture.frame, SCOPE)),
        DecodeError::FrameTooLarge {
            actual: len,
            max: 0
        }
    );
    assert_eq!(zero_decoder.baseline_stamp(), None);
    assert_eq!(zero_decoder.retained_baseline_bytes(), 0);
}

#[test]
fn valid_delta_growing_beyond_baseline_cap_is_delivered_then_clears_cache() {
    let fixture = fixture(10);
    let baseline_cap = fixture.frame.to_bytes().len();
    let mut target = fixture.frame.clone();
    target.list_push(fixture.list, 0xa5);
    target.set_tick(11);
    assert!(target.to_bytes().len() > baseline_cap);
    let mut encoder = Encoder::new(CAP, baseline_cap);
    let first = encoder.encode(&fixture.frame, SCOPE).expect("baseline");
    let delta = encoder.encode(&target, SCOPE).expect("growing delta");
    assert!(matches!(delta, FrameRecord::Delta { .. }));
    assert_eq!(encoder.baseline_stamp(), None);
    assert_eq!(encoder.retained_baseline_bytes(), 0);
    let mut decoder = Decoder::new(Arc::clone(&fixture.registry), CAP, baseline_cap);
    decoder.decode(&first).expect("decode baseline");
    assert_frame(
        &target,
        &decoder.decode(&delta).expect("deliver larger frame"),
    );
    assert_eq!(decoder.baseline_stamp(), None);
    assert_eq!(decoder.retained_baseline_bytes(), 0);
    let mut next = target.clone();
    next.set_tick(12);
    assert!(matches!(
        encoder.encode(&next, SCOPE).expect("after cache clear"),
        FrameRecord::Full { .. }
    ));
    assert_eq!(
        decode_error(&mut decoder, &delta),
        DecodeError::NeedFull(NeedFullReason::MissingBaseline)
    );
}

#[test]
fn encoder_oversize_error_preserves_baseline_and_explicit_resets_disable_delta() {
    let fixture = fixture(10);
    let cap = fixture.frame.to_bytes().len();
    let mut encoder = Encoder::new(cap, cap);
    let first = encoder.encode(&fixture.frame, SCOPE).expect("baseline");
    let mut oversize = fixture.frame.clone();
    oversize.list_push(fixture.list, 0xa5);
    oversize.set_tick(11);
    assert_eq!(
        encoder
            .encode(&oversize, SCOPE)
            .expect_err("oversize input"),
        EncodeError::FrameTooLarge {
            actual: oversize.to_bytes().len(),
            max: cap
        }
    );
    assert_eq!(encoder.baseline_stamp(), Some(stamp(&fixture.frame, SCOPE)));
    assert_eq!(encoder.retained_baseline_bytes(), cap);
    let mut target = fixture.frame.clone();
    target.set_tick(11);
    let delta = encoder
        .encode(&target, SCOPE)
        .expect("valid after rejected input");
    assert!(matches!(delta, FrameRecord::Delta { .. }));
    let mut decoder = Decoder::new(Arc::clone(&fixture.registry), cap, cap);
    decoder.decode(&first).expect("baseline decode");
    decoder.reset();
    assert_eq!(decoder.baseline_stamp(), None);
    assert_eq!(decoder.retained_baseline_bytes(), 0);
    assert_eq!(
        decode_error(&mut decoder, &delta),
        DecodeError::NeedFull(NeedFullReason::MissingBaseline)
    );
    encoder.reset();
    assert_eq!(encoder.baseline_stamp(), None);
    assert_eq!(encoder.retained_baseline_bytes(), 0);
    let reset = encoder.encode(&target, SCOPE).expect("encode after reset");
    assert!(matches!(reset, FrameRecord::Full { .. }));
    assert_frame(
        &target,
        &decoder.decode(&reset).expect("explicit reset recovery"),
    );
}
