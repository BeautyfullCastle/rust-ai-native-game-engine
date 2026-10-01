//! Input bytes follow the layout of the schema, whatever it is.

use crate::input::{encode, scenario, Controls};
use crate::schema::ViewSchema;

/// The physics demo's layout (docs/view-stream.md).
const DEMO: &str = r#"{"format":"orrery.viewstream","version":1,"game":"PhysGame","tick_rate":60,"player_count":2,
 "kinds":[],"input":{"size":24,"fields":[{"name":"axis_x","offset":0,"size":8,"type":"fixed"},
 {"name":"axis_y","offset":8,"size":8,"type":"fixed"},{"name":"spin","offset":16,"size":4,"type":"i32"},
 {"name":"buttons","offset":20,"size":4,"bits":{"shoot":1},"type":"flags"}]},"command":{"size":4},"events":[]}"#;

/// Another game: different order, sizes and a 32-bit fixed axis.
const OTHER: &str = r#"{"format":"orrery.viewstream","version":1,"game":"Other","tick_rate":30,"player_count":1,
 "kinds":[],"input":{"size":16,"fields":[{"name":"buttons","offset":0,"size":2,"bits":{"jump":4,"other":1},"type":"flags"},
 {"name":"move_y","offset":4,"size":4,"type":"fixed32"},{"name":"move_x","offset":8,"size":4,"type":"fixed32"}]},"events":[]}"#;

#[test]
fn the_demo_layout_gets_the_bytes_of_its_fields() {
    let s = ViewSchema::parse(DEMO).unwrap();
    let bytes = encode(&s, &Controls { x: 1, y: -1, spin: -1, fire: true });
    assert_eq!(bytes.len(), 24);
    assert_eq!(i64::from_le_bytes(bytes[0..8].try_into().unwrap()), 65536);
    assert_eq!(i64::from_le_bytes(bytes[8..16].try_into().unwrap()), -65536);
    assert_eq!(i32::from_le_bytes(bytes[16..20].try_into().unwrap()), -1);
    assert_eq!(u32::from_le_bytes(bytes[20..24].try_into().unwrap()), 1);
    assert_eq!(encode(&s, &Controls::default()), vec![0u8; 24]);
}

#[test]
fn another_layout_moves_the_offsets() {
    let s = ViewSchema::parse(OTHER).unwrap();
    let bytes = encode(&s, &Controls { x: -1, y: 1, spin: 0, fire: true });
    assert_eq!(bytes.len(), 16);
    assert_eq!(u16::from_le_bytes(bytes[0..2].try_into().unwrap()), 4, "the fire-like bit (jump), not the first one");
    assert_eq!(i32::from_le_bytes(bytes[4..8].try_into().unwrap()), 65536, "move_y");
    assert_eq!(i32::from_le_bytes(bytes[8..12].try_into().unwrap()), -65536, "move_x");
}

#[test]
fn the_scenario_is_the_one_of_the_c_client() {
    // Same formulas as scenario_input in orr_ffi/tests/c/view_client.c.
    assert_eq!(scenario(0, 1), Controls { x: -1, y: -1, spin: -1, fire: true });
    assert_eq!(scenario(1, 45), Controls { x: -1, y: -1, spin: 1, fire: false });
    assert!(!scenario(1, 1).fire && scenario(0, 40).fire && !scenario(0, 45).fire);
}

#[test]
fn a_schema_of_another_format_or_version_is_refused() {
    assert!(ViewSchema::parse("{}").is_err());
    assert!(ViewSchema::parse(&DEMO.replace("\"version\":1", "\"version\":2")).unwrap_err().contains("version"));
    assert!(ViewSchema::parse("nope").is_err());
}
