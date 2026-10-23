use super::*;
use serde_json::json;

fn wall(name: &str, x: i64) -> (String, Value) {
    (
        name.into(),
        json!({
            "RoomEscapeV1::Actor":{"kind":4,"ordinal":0},
            "orr_physics3d::Body":{
                "pos":[x,0.5,0],"rot":{"kind":"unit","x":0,"y":0,"z":0,"w":1},
                "vel":[0,0,0],"omega":[0,0,0],"inv_mass":0,"inv_inertia":[0,0,0],
                "linear_damping":0,"angular_damping":0,"kind":"static"
            },
            "orr_physics3d::Collider":{
                "shape":{"kind":"box","half_extents":[0.25,1,4]},
                "restitution":0,"friction":0.5,"layer":1,"mask":0
            }
        }),
    )
}

#[test]
fn capture_encode_decode_preserves_order_and_full_components() {
    let source = Template::capture(vec![wall("north", 2), wall("south", -2)]).unwrap();
    let bytes = source.encode().unwrap();
    let decoded = Template::decode(&bytes).unwrap();
    assert_eq!(decoded, source);
    assert_eq!(decoded.walls()[0].name, "north");
    assert_eq!(decoded.walls()[1].name, "south");
    assert_eq!(decoded.walls()[0].components, wall("north", 2).1);
}

#[test]
fn rejects_schema_duplicates_unknowns_and_non_object_root() {
    assert!(Template::decode(br#"{"schema":1,"schema":1,"walls":[]}"#).is_err());
    let valid = Template::capture(vec![wall("north", 2)])
        .unwrap()
        .encode()
        .unwrap();
    let duplicate_nested = String::from_utf8(valid.clone())
        .unwrap()
        .replace("\"kind\":4", "\"kind\":4,\"kind\":4");
    assert!(Template::decode(duplicate_nested.as_bytes()).is_err());
    let unknown = String::from_utf8(valid)
        .unwrap()
        .replace("\"schema\":1", "\"schema\":1,\"extra\":true");
    assert!(Template::decode(unknown.as_bytes()).is_err());
    assert!(Template::decode(b"[]").is_err());
    assert!(Template::decode(br#"{"schema":2,"walls":[]}"#).is_err());
}

#[test]
fn preserves_decimal_tokens_without_float_rounding() {
    let valid = Template::capture(vec![wall("north", 2)])
        .unwrap()
        .encode()
        .unwrap();
    let exact = String::from_utf8(valid)
        .unwrap()
        .replace("\"pos\":[2,0.5,0]", "\"pos\":[2.00000000000000001,0.5,0]");
    let decoded = Template::decode(exact.as_bytes()).unwrap();
    let encoded = String::from_utf8(decoded.encode().unwrap()).unwrap();
    assert!(encoded.contains("2.00000000000000001"));
}

#[test]
fn allows_repeated_names_and_rejects_entity_metadata() {
    let repeated = Template::capture(vec![wall("north", 2), wall("north", -2)]).unwrap();
    let decoded = Template::decode(&repeated.encode().unwrap()).unwrap();
    assert_eq!(decoded.walls()[0].name, decoded.walls()[1].name);
    assert_eq!(decoded.walls()[0].components, wall("north", 2).1);
    assert_eq!(decoded.walls()[1].components, wall("north", -2).1);
    let mut extra = wall("north", 2).1;
    extra
        .as_object_mut()
        .unwrap()
        .insert("guid".into(), json!("entity-1"));
    assert!(Template::capture(vec![("north".into(), extra)]).is_err());
    let mut extra_body_field = wall("north", 2).1;
    extra_body_field[BODY]["custom"] = json!(1);
    assert!(Template::capture(vec![("north".into(), extra_body_field)]).is_err());
}

#[test]
fn round_trips_empty_and_whitespace_display_names() {
    let template = Template::capture(vec![wall("", 2), wall("  ", -2)]).unwrap();
    let decoded = Template::decode(&template.encode().unwrap()).unwrap();
    assert_eq!(decoded.walls()[0].name, "");
    assert_eq!(decoded.walls()[1].name, "  ");
}

#[test]
fn rejects_nonwall_or_noncanonical_physics_components() {
    let mut actor = wall("north", 2).1;
    actor[ACTOR]["kind"] = json!(1);
    assert!(Template::capture(vec![("north".into(), actor)]).is_err());
    let mut body = wall("north", 2).1;
    body[BODY]["kind"] = json!("dynamic");
    assert!(Template::capture(vec![("north".into(), body)]).is_err());
    let mut collider = wall("north", 2).1;
    collider[COLLIDER]["shape"]["kind"] = json!("sphere");
    assert!(Template::capture(vec![("north".into(), collider)]).is_err());
}

#[test]
fn rejects_out_of_bounds_or_invalid_geometry() {
    for (position, half_extents) in [
        (json!([17, 0.5, 0]), json!([0.25, 1, 4])),
        (json!([2, 0.5, 0]), json!([0.25, 0.2, 4])),
        (json!([2, 0.5, 0]), json!([0.01, 1, 4])),
        (json!([2, 0.5, 0]), json!([0.25, 9, 4])),
    ] {
        let mut components = wall("north", 2).1;
        components[BODY]["pos"] = json!(position);
        components[COLLIDER]["shape"]["half_extents"] = json!(half_extents);
        assert!(Template::capture(vec![("north".into(), components)]).is_err());
    }
    assert!(Template::capture(vec![("line\nbreak".into(), wall("north", 2).1)]).is_err());
}

#[test]
fn enforces_wall_count_and_serialized_size_bounds() {
    assert!(Template::capture(Vec::new()).is_err());
    assert!(Template::capture(
        (0..=MAX_WALLS)
            .map(|index| wall(&format!("wall-{index}"), 2))
            .collect()
    )
    .is_err());
    let mut oversized = wall("north", 2).1;
    oversized[BODY]["padding"] = json!("x".repeat(MAX_BYTES));
    assert!(Template::capture(vec![("north".into(), oversized)]).is_err());
    assert!(Template::decode(&vec![b' '; MAX_BYTES + 1]).is_err());
}
