//! The view stream format: round trips, the pinned bytes of a tiny frame, and
//! what a reader does with damaged messages.

use orr_viewstream::*;

fn sample_frame() -> ViewFrame {
    ViewFrame {
        flags: FLAG_PAUSED,
        tick: 7,
        verified_tick: 5,
        seq: 3,
        rollback: Some((4, 6)),
        entities: vec![
            EntityRecord {
                id: 0x0000_0002_0000_0001,
                kind: 3,
                shape: SHAPE_QUAD,
                mode: MODE_NONE,
                size: 5.0,
                half_y: 0.5,
                rgba: [255, 128, 0, 255],
                prev: [1.0, 2.0, 0.25],
                cur: [1.5, 2.5, -0.25],
            },
            EntityRecord {
                id: 9,
                kind: 1,
                shape: SHAPE_CIRCLE,
                mode: MODE_PREDICTION,
                size: 0.5,
                half_y: 0.0,
                rgba: [1, 2, 3, 4],
                prev: [0.0; 3],
                cur: [0.0, -1.0, 3.0],
            },
        ],
        props: [7u32.to_le_bytes(), 1.5f32.to_bits().to_le_bytes()].concat(),
    }
}

#[test]
fn frame_round_trip() {
    let f = sample_frame();
    let bytes = f.encode();
    assert_eq!(bytes.len(), HEADER_LEN + 2 * RECORD_LEN + 8);
    let back = ViewFrame::decode(&bytes).unwrap();
    // The rollback flag is derived from the range.
    assert_eq!(back, ViewFrame { flags: FLAG_PAUSED | FLAG_ROLLED_BACK, ..f.clone() });
    assert!(back.has(FLAG_ROLLED_BACK) && back.has(FLAG_PAUSED) && !back.has(FLAG_DISCONTINUITY));
    let props = back.props_by_entity(|k| usize::from(k == 3 || k == 1)).unwrap();
    assert_eq!(props[0], 7u32.to_le_bytes());
    assert_eq!(props[1], 1.5f32.to_bits().to_le_bytes());
    assert!(back.props_by_entity(|_| 0).is_none(), "property bytes that do not add up are refused");
    assert_eq!(message_type(&bytes), Ok(MSG_FRAME));
}

#[test]
fn event_round_trip_pads_records_to_eight_bytes() {
    let batch = EventBatch {
        events: vec![
            EventRecord { tick: 10, system: 2, seq: 1, state: STATE_PREDICTED, event_type: 4, payload: vec![1, 2, 3] },
            EventRecord { tick: 10, system: 2, seq: 1, state: STATE_CANCELED, event_type: 0, payload: vec![] },
            EventRecord { tick: 11, system: 0, seq: 0, state: STATE_VERIFIED, event_type: 4, payload: vec![9; 8] },
        ],
    };
    let bytes = batch.encode();
    assert_eq!(bytes.len(), EVENT_BATCH_HEADER_LEN + (EVENT_HEAD_LEN + 8) + EVENT_HEAD_LEN + (EVENT_HEAD_LEN + 8));
    assert_eq!(bytes.len() % 8, 0);
    assert_eq!(EventBatch::decode(&bytes).unwrap(), batch);
    assert_eq!(message_type(&bytes), Ok(MSG_EVENTS));
    assert_eq!(ViewFrame::decode(&bytes), Err(DecodeError::WrongType(MSG_EVENTS)));
    assert_eq!(EventBatch::decode(&sample_frame().encode()), Err(DecodeError::WrongType(MSG_FRAME)));
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Pins the format: if this changes, the version must change too.
#[test]
fn golden_bytes_of_a_tiny_frame() {
    let f = ViewFrame {
        flags: FLAG_DISCONTINUITY,
        tick: 1,
        verified_tick: 1,
        seq: 2,
        rollback: None,
        entities: vec![EntityRecord {
            id: 0x0000_0001_0000_0002,
            kind: 1,
            shape: SHAPE_CAPSULE,
            mode: MODE_PREDICTION,
            size: 2.0,
            half_y: 0.5,
            rgba: [0x11, 0x22, 0x33, 0xff],
            prev: [1.0, 0.0, 0.0],
            cur: [2.0, -1.0, 0.5],
        }],
        props: vec![],
    };
    let expected = concat!(
        // header: magic "OVS1", version 1, type 1, flags 2
        "4f565331", "0100", "01", "02",
        // tick 1, verified 1, seq 2, rollback from 0 to 0
        "0100000000000000", "0100000000000000", "0200000000000000", "0000000000000000", "0000000000000000",
        // entity count 1, props bytes 0
        "01000000", "00000000",
        // record: id (index 2, version 1), kind 1, shape 2, mode 0
        "0200000001000000", "0100", "02", "00",
        // size 2.0, half_y 0.5, rgba
        "00000040", "0000003f", "112233ff",
        // prev (1, 0, 0), cur (2, -1, 0.5)
        "0000803f", "00000000", "00000000", "00000040", "000080bf", "0000003f",
    );
    assert_eq!(hex(&f.encode()), expected);
    assert_eq!(ViewFrame::decode(&f.encode()).unwrap(), f);
}

#[test]
fn damaged_messages_are_refused_not_trusted() {
    let bytes = sample_frame().encode();
    for cut in [0, 3, 10, HEADER_LEN - 1, HEADER_LEN + 10, bytes.len() - 1] {
        assert_eq!(ViewFrame::decode(&bytes[..cut]), Err(DecodeError::Truncated), "cut at {cut}");
    }
    let mut bad = bytes.clone();
    bad[0] = b'X';
    assert_eq!(ViewFrame::decode(&bad), Err(DecodeError::BadMagic));
    let mut newer = bytes.clone();
    newer[4..6].copy_from_slice(&(VERSION + 1).to_le_bytes());
    assert_eq!(ViewFrame::decode(&newer), Err(DecodeError::UnsupportedVersion(VERSION + 1)));
    // An entity count that promises more than the message holds.
    let mut huge = bytes;
    huge[48..52].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(ViewFrame::decode(&huge), Err(DecodeError::Truncated));
}

#[test]
fn entity_ids_keep_index_and_version() {
    let e = orr_ecs::Entity { index: 5, version: 3 };
    assert_eq!(entity_id(e), (3u64 << 32) | 5);
}

#[test]
fn color_quantization_rounds_and_clamps() {
    assert_eq!([color_to_u8(0.0), color_to_u8(1.0), color_to_u8(0.5), color_to_u8(-1.0), color_to_u8(9.0)], [0, 255, 128, 0, 255]);
}
