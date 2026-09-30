mod common;

use common::*;
use orr_edit::Target;
use orr_fp::{FPVec2, FP, FP32};
use orr_reflect::{decimal, TypeDesc, Value};
use orr_remote::json::{json_to_value, value_to_json};
use serde_json::{json, Value as J};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn edge_raws() -> Vec<i64> {
    vec![
        0,
        1,
        -1,
        2,
        65535,
        65536,
        65537,
        -65536,
        -65537,
        i64::MAX,
        i64::MIN + 1,
        i64::MAX - 1,
        1 << 47,
        -(1 << 47),
        (1 << 62) + 12345,
        999_999,
        6_553_600,
    ]
}

#[test]
fn fixed_point_round_trips_exactly_through_json() {
    let desc = TypeDesc::fixed();
    let mut raws = edge_raws();
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for _ in 0..200_000 {
        let r = rng.next() as i64;
        // Mix full-range values with small ones (the common case).
        raws.push(if rng.next() & 1 == 0 { r } else { r >> 40 });
    }
    for raw in raws {
        if raw == i64::MIN {
            continue; // no text parses to it
        }
        let v = Value::Fixed(FP::from_raw(raw));
        let j = value_to_json(&v);
        assert!(j.is_number(), "{j}");
        // The JSON text is exactly the decimal text of the raw value.
        assert_eq!(j.to_string(), decimal::fp_to_decimal(FP::from_raw(raw)));
        assert_eq!(json_to_value(&desc, &j, false).unwrap(), v, "raw {raw}");
        // As a string too, and through text (the wire) and back.
        assert_eq!(json_to_value(&desc, &J::String(j.to_string()), false).unwrap(), v);
        let wire: J = serde_json::from_str(&j.to_string()).unwrap();
        assert_eq!(json_to_value(&desc, &wire, false).unwrap(), v);
    }
    // The examples of the task.
    // Output is the shortest decimal that reads back to the same raw value (as in scene files);
    // input may have as many digits as you like: 1/65536 = 0.0000152587890625.
    assert_eq!(value_to_json(&Value::Fixed(FP::from_raw(1))).to_string(), "0.00002");
    assert_eq!(value_to_json(&Value::Fixed(FP::from_raw(-1))).to_string(), "-0.00002");
    for text in ["0.00001525878", "0.0000152587890625", "0.00002", "1.5259e-5"] {
        assert_eq!(json_to_value(&desc, &serde_json::from_str(text).unwrap(), false).unwrap(), Value::Fixed(FP::from_raw(1)), "{text}");
    }
    let big = FP::from_raw(i64::MAX);
    let text = decimal::fp_to_decimal(big);
    assert_eq!(json_to_value(&desc, &serde_json::from_str(&text).unwrap(), false).unwrap(), Value::Fixed(big));
    // An f64 could not hold this: 2^47 + 1 raw unit needs 63 bits.
    let precise = FP::from_raw((1 << 62) + 1);
    let text = decimal::fp_to_decimal(precise);
    assert_eq!(json_to_value(&desc, &serde_json::from_str(&text).unwrap(), false).unwrap(), Value::Fixed(precise));
    // Exponent spellings and extra digits (rounded to the nearest 1/65536).
    for (text, raw) in [("1.5259e-5", 1), ("1e0", 65536), ("-2.5e-1", -16384), ("6.5536E4", 65536 * 65536), ("0.5", 32768)] {
        let j: J = serde_json::from_str(text).unwrap();
        assert_eq!(json_to_value(&desc, &j, false).unwrap(), Value::Fixed(FP::from_raw(raw)), "{text}");
    }
    // Refusals.
    for bad in [json!("abc"), json!(true), json!(null), json!([1]), json!("1e999"), json!("--1"), json!("1.5.2")] {
        assert!(json_to_value(&desc, &bad, false).is_err(), "{bad}");
    }
    assert!(json_to_value(&desc, &serde_json::from_str("1e300").unwrap(), false).is_err(), "overflow");
    assert!(json_to_value(&desc, &serde_json::from_str("140737488355328").unwrap(), false).is_err(), "just over the Q48.16 range");
}

#[test]
fn fixed32_vectors_and_integers() {
    let d32 = TypeDesc::fixed32();
    for raw in [0, 1, -1, i32::MAX, i32::MIN + 1, 65536, -3 * 65536 - 7] {
        let v = Value::Fixed32(FP32::from_raw(raw));
        let j = value_to_json(&v);
        assert_eq!(json_to_value(&d32, &j, false).unwrap(), v, "raw {raw}");
    }
    let v2 = Value::Vec2(FPVec2::new(FP::from_raw(1), FP::from_raw(-70000)));
    let j = value_to_json(&v2);
    assert_eq!(j.to_string(), format!("[0.00002,{}]", decimal::fp_to_decimal(FP::from_raw(-70000))));
    assert_eq!(json_to_value(&TypeDesc::vec2(), &j, false).unwrap(), v2);
    assert!(json_to_value(&TypeDesc::vec2(), &json!([1, 2, 3]), false).is_err());
    assert!(json_to_value(&TypeDesc::vec3(), &json!([1, 2]), false).is_err());

    let du32 = TypeDesc::int(orr_reflect::IntKind::U32);
    let du64 = TypeDesc::int(orr_reflect::IntKind::U64);
    let di64 = TypeDesc::int(orr_reflect::IntKind::I64);
    assert_eq!(json_to_value(&du32, &json!(4294967295u64), false).unwrap(), Value::Int(4294967295));
    assert!(json_to_value(&du32, &json!(4294967296u64), false).is_err());
    assert!(json_to_value(&du32, &json!(-1), false).is_err());
    assert!(json_to_value(&du32, &json!(1.5), false).is_err());
    assert_eq!(json_to_value(&du32, &json!(3.0), false).unwrap(), Value::Int(3), "3.0 is the integer 3");
    let max = u64::MAX;
    let j = value_to_json(&Value::Int(i128::from(max)));
    assert_eq!(j.to_string(), max.to_string(), "u64::MAX keeps every digit");
    assert_eq!(json_to_value(&du64, &serde_json::from_str(&max.to_string()).unwrap(), false).unwrap(), Value::Int(i128::from(max)));
    assert_eq!(
        json_to_value(&di64, &serde_json::from_str(&i64::MIN.to_string()).unwrap(), false).unwrap(),
        Value::Int(i128::from(i64::MIN))
    );
    assert_eq!(json_to_value(&du64, &json!("18446744073709551615"), false).unwrap(), Value::Int(i128::from(max)));
}

#[test]
fn every_value_of_the_demo_scene_round_trips() {
    let doc = demo_doc();
    let view = doc.view();
    let types = types();
    let mut seen = 0;
    for info in view.entities() {
        let target = Target::Entity(info.entity);
        for (name, value) in view.components(&target).unwrap() {
            let desc = types.get(&name).unwrap().desc();
            let j = value_to_json(&value);
            let text = j.to_string();
            let wire: J = serde_json::from_str(&text).unwrap();
            let back = json_to_value(desc, &wire, false).unwrap_or_else(|e| panic!("{name}: {e} in {text}"));
            assert_eq!(back, value, "{name}");
            seen += 1;
        }
    }
    for (name, value) in view.singletons() {
        let desc = types.get(&name).unwrap().desc();
        let j = value_to_json(&value);
        assert_eq!(json_to_value(desc, &j, false).unwrap(), value, "{name}");
        seen += 1;
    }
    assert!(seen > 100, "{seen}");
}

#[test]
fn values_over_the_wire_keep_every_fixed_point_bit() {
    let host = TestHost::standard();
    let mut c = host.client("tok-all");
    let body = guid_of(&mut c, "body_04");
    let mut rng = Rng(42);
    // `pos` is limited to +-30000: values up to +-2^30 raw.
    let mut raws = vec![1i64, -1, 65537, -65537, 1 << 30, -(1 << 30), 1_234_567_891, 1_966_079_999, -1_966_079_999];
    for _ in 0..40 {
        raws.push((rng.next() as i64) >> 34);
    }
    for raw in raws {
        let text = decimal::fp_to_decimal(FP::from_raw(raw));
        // Send as a JSON number with exactly this text (built by hand, so no float is involved).
        let req = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"world.patch\",\"params\":{{\"entity\":\"{body}\",\"component\":\"orr_physics::Body\",\"path\":\"pos.x\",\"value\":{text}}}}}"
        );
        c.send_text(&req).unwrap();
        let resp: J = serde_json::from_str(&c.recv_text(std::time::Duration::from_secs(5)).unwrap().unwrap()).unwrap();
        assert!(resp.get("result").is_some(), "{raw}: {resp}");
        let got = c.call("world.get", json!({"entity": body, "component": "orr_physics::Body", "path": "pos.x"})).unwrap();
        assert_eq!(got["value"].to_string(), text, "raw {raw} came back as {}", got["value"]);
    }
    // And the same for a string spelling.
    c.call("world.patch", json!({"entity": body, "component": "orr_physics::Body", "path": "pos.y", "value": "0.00001525878"})).unwrap();
    let got = c.call("world.get", json!({"entity": body, "component": "orr_physics::Body", "path": "pos.y"})).unwrap();
    assert_eq!(got["value"].to_string(), "0.00002", "raw 1");
    // Out of the documented range: refused, and the message names the range.
    let e = c.call_err("world.patch", json!({"entity": body, "component": "orr_physics::Body", "path": "pos.x", "value": 40000}));
    assert!(e.message.contains("outside"), "{}", e.message);
}

// ---- schema ----

/// Just enough JSON Schema (the subset `registry.schema` writes) to check
/// that what the methods return is what the schema describes.
fn validate(root: &J, schema: &J, v: &J) -> Result<(), String> {
    if let Some(r) = schema.get("$ref").and_then(J::as_str) {
        let key = r.strip_prefix("#/$defs/").ok_or_else(|| format!("unsupported $ref {r}"))?;
        let target = root.get("$defs").and_then(|d| d.get(key)).ok_or_else(|| format!("missing $defs/{key}"))?;
        return validate(root, target, v);
    }
    if let Some(list) = schema.get("oneOf").and_then(J::as_array) {
        let ok = list.iter().filter(|s| validate(root, s, v).is_ok()).count();
        if ok != 1 {
            return Err(format!("{ok} of the oneOf alternatives match {v}"));
        }
        return Ok(());
    }
    if let Some(c) = schema.get("const") {
        return if c == v { Ok(()) } else { Err(format!("{v} is not {c}")) };
    }
    if let Some(list) = schema.get("enum").and_then(J::as_array) {
        return if list.contains(v) { Ok(()) } else { Err(format!("{v} is not one of {list:?}")) };
    }
    let ty = schema.get("type").and_then(J::as_str);
    match ty {
        Some("boolean") if !v.is_boolean() => return Err(format!("{v} is not a boolean")),
        Some("string") if !v.is_string() => return Err(format!("{v} is not a string")),
        Some("number" | "integer") => {
            let J::Number(n) = v else { return Err(format!("{v} is not a number")) };
            let text = n.to_string();
            if ty == Some("integer") {
                let i = decimal::parse_int(&text).map_err(|_| format!("{v} is not an integer"))?;
                if let Some(m) = schema.get("minimum").and_then(J::as_i64) {
                    if i < i128::from(m) {
                        return Err(format!("{v} < {m}"));
                    }
                }
                if let Some(m) = schema.get("maximum").and_then(J::as_u64) {
                    if i > i128::from(m) {
                        return Err(format!("{v} > {m}"));
                    }
                }
            } else {
                let x = decimal::parse_fp(&text).map_err(|_| format!("{v} is not a fixed-point number"))?;
                for (key, is_min) in [("minimum", true), ("maximum", false)] {
                    if let Some(b) = schema.get(key) {
                        let b = decimal::parse_fp(&b.to_string()).map_err(|_| format!("bad bound {b}"))?;
                        if (is_min && x < b) || (!is_min && x > b) {
                            return Err(format!("{v} violates {key} {b:?}"));
                        }
                    }
                }
            }
        }
        Some("array") => {
            let J::Array(items) = v else { return Err(format!("{v} is not an array")) };
            let n = |k: &str| schema.get(k).and_then(J::as_u64);
            if n("minItems").is_some_and(|m| (items.len() as u64) < m) || n("maxItems").is_some_and(|m| (items.len() as u64) > m) {
                return Err(format!("{} items", items.len()));
            }
            if let Some(J::Array(prefix)) = schema.get("prefixItems") {
                for (s, it) in prefix.iter().zip(items) {
                    validate(root, s, it)?;
                }
            }
            match schema.get("items") {
                Some(J::Bool(false)) => {
                    let allowed = schema.get("prefixItems").and_then(J::as_array).map_or(0, Vec::len);
                    if items.len() > allowed {
                        return Err("extra items".into());
                    }
                }
                Some(s @ J::Object(_)) => {
                    for it in items {
                        validate(root, s, it)?;
                    }
                }
                _ => {}
            }
        }
        Some("object") => {
            let J::Object(obj) = v else { return Err(format!("{v} is not an object")) };
            let props = schema.get("properties").and_then(J::as_object);
            for req in schema.get("required").and_then(J::as_array).into_iter().flatten() {
                if !obj.contains_key(req.as_str().unwrap()) {
                    return Err(format!("missing {req}"));
                }
            }
            for (k, x) in obj {
                match props.and_then(|p| p.get(k)) {
                    Some(s) => validate(root, s, x).map_err(|e| format!("{k}: {e}"))?,
                    None if schema.get("additionalProperties") == Some(&J::Bool(false)) => return Err(format!("unexpected key {k}")),
                    None => {}
                }
            }
        }
        _ => {}
    }
    Ok(())
}

#[test]
fn the_schema_describes_the_values_the_methods_return_and_accept() {
    let host = TestHost::standard();
    let mut c = host.client("tok-all");
    let whole = c.call("registry.schema", json!({})).unwrap();
    assert_eq!(whole["schema"]["$schema"], "https://json-schema.org/draft/2020-12/schema");
    assert!(whole["value_format"].as_str().unwrap().contains("exact decimal"));
    let types = c.call("registry.types", J::Null).unwrap();
    let names: Vec<(String, String)> =
        types["types"].as_array().unwrap().iter().map(|t| (t["name"].as_str().unwrap().into(), t["kind"].as_str().unwrap().into())).collect();
    assert!(names.len() >= 5);
    let mut per_type = std::collections::BTreeMap::new();
    for (name, _) in &names {
        let s = c.call("registry.schema", json!({"type": name})).unwrap();
        assert_eq!(s["schema"]["title"], name.as_str());
        per_type.insert(name.clone(), s["schema"].clone());
    }
    assert_eq!(c.call_err("registry.schema", json!({"type": "Nope"})).kind(), Some("unknown_type"));

    // Every value the server returns validates against the schema of its type.
    let q = c.call("world.query", json!({"values": true})).unwrap();
    let mut checked = 0;
    for e in q["entities"].as_array().unwrap() {
        for (name, value) in e["values"].as_object().unwrap() {
            let schema = &per_type[name.as_str()];
            validate(schema, schema, value).unwrap_or_else(|err| panic!("{name} of {}: {err}\n{value}", e["id"]));
            checked += 1;
        }
    }
    let singles = c.call("world.singleton.get", json!({})).unwrap();
    for (name, value) in singles["singletons"].as_object().unwrap() {
        let schema = &per_type[name.as_str()];
        validate(schema, schema, value).unwrap_or_else(|err| panic!("singleton {name}: {err}\n{value}"));
        checked += 1;
    }
    assert!(checked > 100);

    // The validator is not vacuous: bad values fail it, and the server refuses the same ones.
    let collider = &per_type["orr_physics::Collider"];
    let body = guid_of(&mut c, "body_05");
    let good = c.call("world.get", json!({"entity": body, "component": "orr_physics::Collider"})).unwrap()["value"].clone();
    validate(collider, collider, &good).unwrap();
    for (path, bad) in [("friction", json!(1000)), ("friction", json!("x")), ("shape", json!({"kind": "circle"})), ("shape", json!({"kind": "star"})), ("flags", json!(["nope"]))] {
        let mut v = good.clone();
        v[path] = bad.clone();
        assert!(validate(collider, collider, &v).is_err(), "{path}={bad} should violate the schema");
        let e = c.call_err("world.patch", json!({"entity": body, "component": "orr_physics::Collider", "path": path, "value": bad}));
        assert_eq!(e.code, orr_remote::INVALID_VALUE, "{path}={bad}: {e}");
    }
    // What the schema allows, the server takes: every alternative of `shape`.
    let shape_variants: Vec<J> = vec![
        json!({"kind": "circle", "radius": 0.5}),
        json!({"kind": "box", "half_extents": [0.5, 0.25]}),
        json!({"kind": "polygon", "verts": [[0, 0], [1, 0], [0, 1]]}),
        json!({"kind": "capsule", "half_length": 0.5, "radius": 0.25}),
        json!({"kind": "capsule_segment", "a": [0, 0], "b": [1, 0], "radius": 0.25}),
    ];
    for shape in shape_variants {
        let mut v = good.clone();
        v["shape"] = shape.clone();
        validate(collider, collider, &v).unwrap_or_else(|e| panic!("{shape}: {e}"));
        c.call("world.patch", json!({"entity": body, "component": "orr_physics::Collider", "path": "shape", "value": shape})).unwrap();
        let back = c.call("world.get", json!({"entity": body, "component": "orr_physics::Collider", "path": "shape"})).unwrap();
        validate(collider, &collider["properties"]["shape"], &back["value"]).unwrap();
    }
}
