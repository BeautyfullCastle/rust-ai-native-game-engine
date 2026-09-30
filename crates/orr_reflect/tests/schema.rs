//! JSON Schema output: valid JSON, stable text, references that resolve.

mod common;

use common::*;
use serde_json::Value as Json;

const GOLDEN_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/scene.schema.json");

#[test]
fn schema_is_valid_json_with_draft_2020_12_marker() {
    let text = types().json_schema();
    let json: Json = serde_json::from_str(&text).expect("schema must be valid JSON");
    assert_eq!(json["$schema"], "https://json-schema.org/draft/2020-12/schema");
    assert_eq!(json["properties"]["schema"]["const"], "orr.scene/1");
    assert_eq!(json["required"], serde_json::json!(["schema", "entities"]));
    assert_eq!(json["additionalProperties"], false);
    assert!(text.ends_with("}\n"));
}

#[test]
fn every_reference_resolves() {
    let json: Json = serde_json::from_str(&types().json_schema()).unwrap();
    let mut refs = Vec::new();
    collect_refs(&json, &mut refs);
    assert!(refs.len() > 10);
    for r in refs {
        let pointer = r.strip_prefix('#').expect("local reference");
        assert!(json.pointer(pointer).is_some(), "dangling $ref {r}");
    }
}

fn collect_refs(v: &Json, out: &mut Vec<String>) {
    match v {
        Json::Object(m) => {
            for (k, x) in m {
                if k == "$ref" {
                    out.push(x.as_str().unwrap().to_string());
                } else {
                    collect_refs(x, out);
                }
            }
        }
        Json::Array(a) => a.iter().for_each(|x| collect_refs(x, out)),
        _ => {}
    }
}

#[test]
fn schema_describes_ranges_docs_enums_and_tagged_values() {
    let json: Json = serde_json::from_str(&types().json_schema()).unwrap();
    let t = &json["$defs"]["Transform"];
    assert_eq!(t["description"], "Position and heading.");
    let pos = &t["properties"]["pos"];
    assert_eq!(pos["description"], "World position.");
    assert_eq!(pos["prefixItems"][0]["minimum"], -1000);
    assert_eq!(pos["prefixItems"][0]["maximum"], 1000);
    assert_eq!(t["required"], serde_json::json!(["pos", "rot"]));
    assert_eq!(t["additionalProperties"], false);
    // Fixed point without a range points at the shared definition with the precision note.
    assert_eq!(t["properties"]["rot"]["$ref"].as_str(), Some("#/$defs/orr.Fixed"));
    assert!(json["$defs"]["orr.Fixed"]["description"].as_str().unwrap().contains("1/65536"));

    let h = &json["$defs"]["Health"];
    assert_eq!(h["properties"]["max"]["minimum"], 1);
    assert_eq!(h["properties"]["max"]["maximum"], 1000);
    assert_eq!(h["properties"]["current"]["minimum"], i32::MIN);
    assert_eq!(h["properties"]["alive"]["type"], "boolean");
    assert!(h["properties"].get("cache").is_none(), "hidden field must not appear");

    let f = &json["$defs"]["Follow"];
    assert_eq!(f["properties"]["mode"]["enum"], serde_json::json!(["idle", "chase", "flee"]));
    assert_eq!(f["properties"]["traits"]["uniqueItems"], true);
    assert_eq!(f["properties"]["target"]["$ref"].as_str(), Some("#/$defs/orr.EntityRef"));

    let b = &json["$defs"]["Blob"]["oneOf"];
    assert_eq!(b.as_array().unwrap().len(), 3);
    assert_eq!(b[0]["properties"]["kind"]["const"], "dot");
    assert_eq!(b[0]["properties"]["radius"]["minimum"], 0.05);
    assert_eq!(b[2]["properties"]["pts"]["minItems"], 3);
    assert_eq!(b[2]["properties"]["pts"]["maxItems"], 4);

    let s = &json["$defs"]["Stats"];
    assert_eq!(s["properties"]["scores"]["minItems"], 4);
    assert_eq!(s["properties"]["scores"]["items"]["maximum"], u32::MAX);

    let entity = &json["$defs"]["orr.SceneEntity"];
    assert!(entity["properties"].get("Transform").is_some());
    assert!(entity["properties"].get("Stats").is_none(), "singletons are not entity components");
    assert_eq!(json["properties"]["singletons"]["properties"]["Stats"]["$ref"].as_str(), Some("#/$defs/Stats"));
    assert_eq!(json["properties"]["entities"]["propertyNames"]["pattern"], "^e_[0-9a-f]{8,32}$");
}

#[test]
fn per_type_schema_is_standalone_json() {
    let reg = types();
    for name in ["Transform", "Health", "Follow", "Blob", "Stats"] {
        let text = reg.type_json_schema(name).unwrap();
        let json: Json = serde_json::from_str(&text).unwrap();
        assert_eq!(json["title"], name);
        assert!(json["$defs"].get("orr.Fixed").is_some());
        let mut refs = Vec::new();
        collect_refs(&json, &mut refs);
        for r in refs {
            assert!(json.pointer(r.strip_prefix('#').unwrap()).is_some(), "{name}: dangling {r}");
        }
    }
    assert!(reg.type_json_schema("Nope").is_none());
}

#[test]
fn schema_text_is_stable_and_matches_the_golden_file() {
    let text = types().json_schema();
    assert_eq!(text, types().json_schema(), "same registry, same text");
    if std::env::var_os("ORR_UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(std::path::Path::new(GOLDEN_PATH).parent().unwrap()).unwrap();
        std::fs::write(GOLDEN_PATH, &text).unwrap();
    }
    let golden = std::fs::read_to_string(GOLDEN_PATH).expect("golden file (run once with ORR_UPDATE_GOLDEN=1)");
    // The repository may check files out with CRLF on Windows.
    let golden = golden.replace("\r\n", "\n");
    assert_eq!(text, golden, "schema text changed; if that is intended run the test with ORR_UPDATE_GOLDEN=1 and commit the file");
}

#[test]
fn schema_can_be_written_to_a_file() {
    let path = std::env::temp_dir().join(format!("orr_reflect_schema_{}.json", std::process::id()));
    types().write_json_schema(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(text, types().json_schema());
}
