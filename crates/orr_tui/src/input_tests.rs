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

/// Exercise the UI's real send boundary, rather than passing the 3D ray layout
/// through the unrelated 2D keyboard encoder.
struct CountingSource {
    schema: String,
    inputs: Vec<(u8, Vec<u8>)>,
    reads: usize,
    controls: usize,
    statuses: usize,
}

impl CountingSource {
    fn new(schema: &str) -> Self {
        Self { schema: schema.into(), inputs: Vec::new(), reads: 0, controls: 0, statuses: 0 }
    }
}

impl crate::source::Source for CountingSource {
    fn schema_text(&self) -> &str { &self.schema }
    fn recv(&mut self, _: std::time::Duration) -> Result<Option<crate::source::Incoming>, String> {
        self.reads += 1;
        Ok(None)
    }
    fn set_input(&mut self, player: u8, bytes: &[u8]) -> Result<(), String> {
        self.inputs.push((player, bytes.to_vec()));
        Ok(())
    }
    fn control(&mut self, _: crate::source::Control) -> Result<(), String> {
        self.controls += 1;
        Ok(())
    }
    fn describe(&self) -> String { "counting source".into() }
    fn net_status(&mut self) -> Option<crate::source::NetStatus> {
        self.statuses += 1;
        None
    }
}

fn yard_schema() -> String {
    // The producer's version-2 ray layout. The src dependency guard also scans
    // test modules, so actual producer integration stays in tests/stream.rs.
    r#"{"format":"orrery.viewstream","version":2,"game":"Yard3D","tick_rate":60,"player_count":2,
      "frame3d":{"magic":"OVS1","message_type":3,"header_len":56,"record_len":88},"kinds":[],
      "input":{"size":32,"fields":[{"name":"buttons","offset":0,"size":4,"type":"flags"},
      {"name":"_pad","offset":4,"size":4,"type":"u32"},{"name":"origin","offset":8,"size":12,"type":"vec3"},
      {"name":"dir","offset":20,"size":12,"type":"vec3"}]},"events":[]}"#.into()
}

#[test]
fn three_d_ui_neutral_and_held_controls_never_send_game_input() {
    let text = yard_schema();
    let schema = ViewSchema::parse(&text).unwrap();
    let mut src = CountingSource::new(&text);
    for controls in [Controls::default(), Controls { x: 1, y: -1, spin: 1, fire: true }, Controls::default()] {
        assert!(!crate::ui::send_game_controls(&mut src, &schema, 0, &controls).unwrap());
    }
    assert!(src.inputs.is_empty(), "no fabricated ray, including neutral input");
    assert_eq!((src.reads, src.controls), (0, 0));
}

#[test]
fn two_d_ui_keeps_sending_the_original_schema_layout() {
    let schema = ViewSchema::parse(DEMO).unwrap();
    let mut src = CountingSource::new(DEMO);
    let controls = Controls { x: 1, y: -1, spin: -1, fire: true };
    assert!(crate::ui::send_game_controls(&mut src, &schema, 1, &controls).unwrap());
    assert_eq!(src.inputs, vec![(1, encode(&schema, &controls))]);
}

#[test]
fn three_d_headless_modes_refuse_before_reading_or_sending_scripted_controls() {
    let mut src = CountingSource::new(&yard_schema());
    let opts = crate::headless::HeadlessOpts { frames: 1, size: (80, 24), dump: None };
    let error = crate::headless::run(&mut src, &opts).unwrap_err();
    assert!(error.contains("unsupported for 3D"), "{error}");
    let error = crate::headless::run_client(&mut src, &crate::headless::ClientOpts::default()).unwrap_err();
    assert!(error.contains("unsupported for 3D"), "{error}");
    assert!(src.inputs.is_empty());
    assert_eq!((src.reads, src.controls, src.statuses), (0, 0, 0));
}
