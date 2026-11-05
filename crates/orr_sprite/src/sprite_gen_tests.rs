use super::*;
use serde_json::{json, Value};
fn source() -> Value {
    json!({
        "engine":"component-row", "degraded_static_fallback":false,
        "game_input":"ignored/source.png", "sprite_sheet_alpha":"ignored/source.png",
        "frame_layout":{"sheetWidth":4,"sheetHeight":2,"rows":{
            "once":[{"x":0,"y":1,"w":2,"h":1},{"x":2,"y":1,"w":2,"h":1}],
            "walk":[{"x":2,"y":0,"w":2,"h":1},{"x":0,"y":0,"w":2,"h":1},{"x":2,"y":0,"w":2,"h":1}]}},
        "animation":{"columns":2,"rows":{
            "once":{"row":1,"frames":2,"durations_ms":[40,60],"fps":999,"loop":false},
            "walk":{"row":0,"frames":3,"durations_ms":[100,200,300],"fps":1,"loop":true}}}
    })
}
fn convert(v: &Value) -> Result<SpriteDocument, Error> {
    from_json(&v.to_string())
}
#[test]
fn preserves_rectangles_repetitions_timing_loop_once_and_inert_paths() {
    let d = convert(&source()).unwrap();
    assert_eq!(
        d.atlas(),
        &Atlas {
            image: "atlas.png".into(),
            width: 4,
            height: 2
        }
    );
    assert_eq!(
        d.clips().iter().map(|c| c.id()).collect::<Vec<_>>(),
        ["walk", "once"]
    );
    assert_eq!(
        d.regions(),
        &[
            Region {
                id: 0,
                x: 2,
                y: 0,
                width: 2,
                height: 1
            },
            Region {
                id: 1,
                x: 0,
                y: 0,
                width: 2,
                height: 1
            },
            Region {
                id: 2,
                x: 0,
                y: 1,
                width: 2,
                height: 1
            },
            Region {
                id: 3,
                x: 2,
                y: 1,
                width: 2,
                height: 1
            }
        ]
    );
    let walk = d.clip("walk").unwrap();
    assert_eq!(walk.duration_ms(), 600);
    for (t, id) in [
        (0, 0),
        (99, 0),
        (100, 1),
        (299, 1),
        (300, 0),
        (599, 0),
        (600, 0),
        (700, 1),
    ] {
        assert_eq!(walk.sample(t).region, id, "{t}");
        assert!(!walk.sample(t).finished);
    }
    let once = d.clip("once").unwrap();
    for (t, id) in [(0, 2), (39, 2), (40, 3), (99, 3), (100, 3), (u64::MAX, 3)] {
        assert_eq!(once.sample(t).region, id);
        assert_eq!(once.sample(t).finished, t >= 100);
    }
    let serialized: Value = serde_json::from_str(&d.to_json().unwrap()).unwrap();
    assert_eq!(
        serialized["clips"][0]["frames"],
        json!([
        {"region":0,"duration_ms":100},{"region":1,"duration_ms":200},{"region":0,"duration_ms":300}])
    );
    assert_eq!(
        SpriteDocument::from_json(&d.to_json().unwrap())
            .unwrap()
            .to_json()
            .unwrap(),
        d.to_json().unwrap()
    );
}
#[test]
fn rejects_inconsistent_or_unsupported_metadata_and_geometry() {
    let edits: &[fn(&mut Value)] = &[
        |v| v["engine"] = json!("other"),
        |v| v["degraded_static_fallback"] = json!(true),
        |v| v["game_input"] = json!("different.png"),
        |v| v["game_input"] = json!(""),
        |v| v["animation"]["rows"]["walk"]["row"] = json!(1),
        |v| v["animation"]["rows"]["walk"]["row"] = json!(4),
        |v| v["animation"]["rows"]["walk"]["frames"] = json!(2),
        |v| {
            v["animation"]["rows"]
                .as_object_mut()
                .unwrap()
                .remove("once")
                .map(|_| ())
                .unwrap()
        },
        |v| v["frame_layout"]["rows"] = json!({}),
        |v| v["frame_layout"]["sheetWidth"] = json!(0),
        |v| v["frame_layout"]["sheetHeight"] = json!(2049),
        |v| v["frame_layout"]["rows"]["walk"][0]["x"] = json!(u32::MAX),
        |v| v["frame_layout"]["rows"]["walk"][0]["w"] = json!(0),
        |v| v["frame_layout"]["rows"]["walk"][0]["y"] = json!(-1),
        |v| v["frame_layout"]["rows"]["walk"][0]["h"] = json!(3),
    ];
    for edit in edits {
        let mut v = source();
        edit(&mut v);
        assert!(convert(&v).is_err(), "{v}");
    }
}
#[test]
fn requires_explicit_positive_nonoverflowing_duration_arrays() {
    for durations in [
        Value::Null,
        json!([]),
        json!([1]),
        json!([0, 2, 3]),
        json!([-1, 2, 3]),
        json!([1.5, 2, 3]),
        json!([u64::MAX, 1, 1]),
    ] {
        let mut v = source();
        v["animation"]["rows"]["walk"]["durations_ms"] = durations;
        assert!(convert(&v).is_err());
    }
    let mut v = source();
    v["animation"]["rows"]["walk"]
        .as_object_mut()
        .unwrap()
        .remove("durations_ms");
    assert!(convert(&v).is_err(), "no implicit fps fallback");
    let mut v = source();
    v["animation"]["rows"]["once"]["loop"] = Value::Null;
    assert!(convert(&v).is_err());
}
#[test]
fn rejects_duplicate_structural_fields_and_state_names() {
    let original = source().to_string();
    for text in [
        original.replacen("\"engine\":", "\"engine\":\"component-row\",\"engine\":", 1),
        original.replacen(
            "\"walk\":",
            &format!(
                "\"walk\":{},\"walk\":",
                source()["animation"]["rows"]["walk"]
            ),
            1,
        ),
        original.replacen("\"sheetWidth\":4", "\"sheetWidth\":4,\"sheetWidth\":4", 1),
    ] {
        assert!(from_json(&text).is_err());
    }
}
#[test]
fn enforces_byte_state_frame_and_region_budgets() {
    assert!(from_json(&" ".repeat(MAX_JSON_BYTES + 1)).is_err());
    let mut v = source();
    v["frame_layout"]["rows"]["walk"] = json!(vec![
        json!({"x":0,"y":0,"w":1,"h":1});
        MAX_FRAMES_PER_CLIP + 1
    ]);
    v["animation"]["rows"]["walk"]["frames"] = json!(MAX_FRAMES_PER_CLIP + 1);
    v["animation"]["rows"]["walk"]["durations_ms"] = json!(vec![1; MAX_FRAMES_PER_CLIP + 1]);
    assert!(convert(&v).is_err());
    let mut v = source();
    for i in 0..MAX_CLIPS {
        v["animation"]["rows"][format!("extra{i}")] = v["animation"]["rows"]["once"].clone();
    }
    assert!(convert(&v).is_err());
    let mut v = source();
    v["frame_layout"]["rows"] = json!({});
    v["animation"]["rows"] = json!({});
    for row in 0..5 {
        v["frame_layout"]["rows"][format!("row{row}")] =
            json!(vec![json!({"x":0,"y":0,"w":1,"h":1}); MAX_FRAMES_PER_CLIP]);
        v["animation"]["rows"][format!("row{row}")] = json!({"row":row,"frames":MAX_FRAMES_PER_CLIP,"durations_ms":vec![1;MAX_FRAMES_PER_CLIP],"loop":true});
    }
    assert!(convert(&v).is_err());
}
#[test]
fn fps_is_not_a_second_timing_source_and_map_order_is_irrelevant() {
    let v = source();
    let expected = convert(&v).unwrap().to_json().unwrap();
    let mut changed = v.clone();
    changed["animation"]["rows"]["walk"]["fps"] = json!("ignored metadata");
    assert_eq!(convert(&changed).unwrap().to_json().unwrap(), expected);
    let pretty = serde_json::to_string_pretty(&v).unwrap();
    assert_eq!(from_json(&pretty).unwrap().to_json().unwrap(), expected);
}

#[test]
fn unique_region_limit_counts_rectangles_not_playback_slots() {
    let mut v = source();
    v["frame_layout"]["sheetWidth"] = json!(2048);
    v["frame_layout"]["sheetHeight"] = json!(3);
    v["frame_layout"]["rows"] = json!({});
    v["animation"]["rows"] = json!({});
    for row in 0..2 {
        let count = if row == 0 { MAX_REGIONS } else { 1 };
        let rects: Vec<_> = (0..count)
            .map(|i| json!({"x":i%2048,"y":if row==0{i/2048}else{2},"w":1,"h":1}))
            .collect();
        v["frame_layout"]["rows"][format!("row{row}")] = json!(rects);
        v["animation"]["rows"][format!("row{row}")] =
            json!({"row":row,"frames":count,"durations_ms":vec![1;count],"loop":true});
    }
    assert!(convert(&v).is_err(), "4097 unique rectangles must fail");
    v["frame_layout"]["rows"]["row1"][0] = json!({"x":0,"y":0,"w":1,"h":1});
    let d = convert(&v).unwrap();
    assert_eq!(d.regions().len(), MAX_REGIONS);
    assert_eq!(d.clip("row1").unwrap().frames()[0].region, 0);
}
