use super::*;
use serde_json::{json, Value};
fn fixture() -> (Value, BTreeMap<String, Vec<u8>>) {
    let mut records = Vec::new();
    let mut objects = BTreeMap::new();
    for (index, cue) in CUES.iter().enumerate() {
        let mut bytes = 48000u32.to_le_bytes().to_vec();
        bytes.extend(2u32.to_le_bytes());
        bytes.extend((-8192i16).to_le_bytes());
        bytes.extend(8192i16.to_le_bytes());
        let hash: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        records.push(json!({"cue":cue,"frames":2,"assetId":format!("a_000000005444{:04x}",index+1),"file":format!("{cue}.pcm"),"bytes":bytes.len(),"sha256":hash}));
        objects.insert(format!("{cue}.pcm"), bytes);
    }
    (
        json!({"version":1,"sampleRate":48000,"cues":records}),
        objects,
    )
}
fn admit(v: &Value, o: &BTreeMap<String, Vec<u8>>) -> Result<PreparedAudio, String> {
    prepare(&serde_json::to_vec(v).unwrap(), o)
}
fn rehash(v: &mut Value, o: &BTreeMap<String, Vec<u8>>, index: usize) {
    let file = v["cues"][index]["file"].as_str().unwrap();
    let hash: String = Sha256::digest(&o[file])
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    v["cues"][index]["sha256"] = hash.into();
}
#[test]
fn preserved_unknown_fields_order_and_script_safe_embedding() {
    let (mut v, o) = fixture();
    v["unknown"] = json!({"label":"한글 </SCRIPT> 🌙", "nested":[true,null,1.25]});
    v["cues"][9]["unknown"] = json!(["kept"]);
    v["cues"].as_array_mut().unwrap().reverse();
    let raw = serde_json::to_string_pretty(&v).unwrap();
    let p = prepare(raw.as_bytes(), &o).unwrap();
    assert!(p.embedded().contains(&raw.replace('<', "\\u003c")));
    assert!(!p.embedded().contains("</SCRIPT>"));
    let body = p
        .embedded()
        .strip_prefix("<script type=\"application/json\" id=\"authored-audio-bank\">")
        .unwrap()
        .strip_suffix("</script>")
        .unwrap();
    let parsed: Value = serde_json::from_str(body).unwrap();
    assert_eq!(parsed["manifest"], v);
    for (name, bytes) in &o {
        assert_eq!(
            STANDARD
                .decode(parsed["objects"][name].as_str().unwrap())
                .unwrap(),
            *bytes
        );
    }
    let object_text = body.split("\"objects\":").nth(1).unwrap();
    assert!(object_text.find("death.pcm").unwrap() < object_text.find("attack-light.pcm").unwrap());
}
#[test]
fn strict_json_unknown_duplicates_nonfinite_and_size_refused() {
    let (v, o) = fixture();
    let text = serde_json::to_string(&v).unwrap();
    for suffix in [
        ",\"extra\":{\"x\":1,\"x\":2}",
        ",\"extra\":NaN",
        ",\"extra\":1e999",
        ",\"version\":1",
    ] {
        let bad = format!("{}{} }}", text.strip_suffix('}').unwrap(), suffix);
        assert!(prepare(bad.as_bytes(), &o).is_err());
    }
    assert!(prepare(&vec![b' '; 128 * 1024 + 1], &o).is_err());
    assert!(prepare(&[255], &o).is_err());
}
#[test]
fn late_pcm_failure_hash_header_and_peak_cannot_publish() {
    let (v, o) = fixture();
    assert!(admit(&v, &o).is_ok());
    let mut bad = o.clone();
    bad.get_mut("death.pcm").unwrap()[8] ^= 1;
    assert!(admit(&v, &bad).is_err());
    for (offset, bytes) in [
        (0, 44100u32.to_le_bytes().to_vec()),
        (4, 3u32.to_le_bytes().to_vec()),
        (8, 8193i16.to_le_bytes().to_vec()),
        (8, i16::MIN.to_le_bytes().to_vec()),
    ] {
        let mut bad = o.clone();
        bad.get_mut("death.pcm").unwrap()[offset..offset + bytes.len()].copy_from_slice(&bytes);
        let mut m = v.clone();
        rehash(&mut m, &bad, 9);
        assert!(admit(&m, &bad).is_err(), "offset {offset}");
    }
    let mut bad = o.clone();
    bad.remove("death.pcm");
    assert!(admit(&v, &bad).is_err());
}
#[test]
fn closed_record_identity_integer_fields_and_budgets() {
    let (v, o) = fixture();
    for (field, value) in [
        ("frames", json!(true)),
        ("frames", json!(2.0)),
        ("frames", json!(0)),
        ("frames", json!(48001)),
        ("bytes", json!(12.0)),
        ("assetId", json!("wrong")),
        ("file", json!("../death.pcm")),
        ("sha256", json!("A".repeat(64))),
    ] {
        let mut m = v.clone();
        m["cues"][9][field] = value;
        assert!(admit(&m, &o).is_err(), "{field}");
    }
    let mut m = v.clone();
    m["cues"][9] = m["cues"][0].clone();
    assert!(admit(&m, &o).is_err());
    let mut m = v.clone();
    m["version"] = json!(1.0);
    assert!(admit(&m, &o).is_err());
    // Ten maximum-length legal clips remain within both existing aggregate budgets.
    let mut m = v;
    let mut objects = BTreeMap::new();
    for (i, cue) in CUES.iter().enumerate() {
        let mut bytes = 48000u32.to_le_bytes().to_vec();
        bytes.extend(48000u32.to_le_bytes());
        bytes.resize(96008, 0);
        objects.insert(format!("{cue}.pcm"), bytes);
        m["cues"][i]["frames"] = json!(48000);
        m["cues"][i]["bytes"] = json!(96008);
        rehash(&mut m, &objects, i);
    }
    assert!(admit(&m, &objects).is_ok());
}
