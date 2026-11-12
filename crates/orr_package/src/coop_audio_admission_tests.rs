use super::*;
use serde_json::{json, Value};
pub(super) fn fixture() -> (Value, BTreeMap<String, Vec<u8>>) {
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
fn admit_value(v: &Value, o: &BTreeMap<String, Vec<u8>>) -> Result<AdmittedBank, String> {
    admit(&serde_json::to_vec(v).unwrap(), o)
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
fn ordered_immutable_admission_preserves_samples_and_input_policy() {
    let (mut v, mut o) = fixture();
    v["unknown"] = json!({"label":"한글 </SCRIPT> 🌙", "nested":[true,null,1.25]});
    v["cues"][9]["unknown"] = json!(["kept"]);
    v["cues"].as_array_mut().unwrap().reverse();
    o.insert("unused.pcm".into(), vec![255]);
    let bank = admit_value(&v, &o).unwrap();
    for (record, cue) in bank.records().iter().zip(CUES.iter().rev()) {
        assert_eq!(record.cue(), *cue);
        assert_eq!(record.frames(), 2);
        assert_eq!(record.pcm_bytes(), o[&format!("{cue}.pcm")]);
        assert_eq!(&record.pcm_bytes()[8..], &[0, 224, 0, 32]);
    }
    // The admitted snapshot is owned independently of caller input mutations.
    o.clear();
    assert_eq!(bank.records().len(), 10);
    assert_eq!(bank.records()[0].pcm_bytes().len(), 12);
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
        assert!(admit(bad.as_bytes(), &o).is_err());
    }
    assert!(admit(&vec![b' '; 128 * 1024 + 1], &o).is_err());
    assert!(admit(&[255], &o).is_err());
}
#[test]
fn late_pcm_failure_hash_header_and_peak_cannot_publish() {
    let (v, o) = fixture();
    assert!(admit_value(&v, &o).is_ok());
    let mut bad = o.clone();
    bad.get_mut("death.pcm").unwrap()[8] ^= 1;
    assert!(admit_value(&v, &bad).is_err());
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
        assert!(admit_value(&m, &bad).is_err(), "offset {offset}");
    }
    let mut bad = o.clone();
    bad.remove("death.pcm");
    assert!(admit_value(&v, &bad).is_err());
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
        assert!(admit_value(&m, &o).is_err(), "{field}");
    }
    let mut m = v.clone();
    m["cues"][9] = m["cues"][0].clone();
    assert!(admit_value(&m, &o).is_err());
    let mut m = v.clone();
    m["version"] = json!(1.0);
    assert!(admit_value(&m, &o).is_err());
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
    assert!(admit_value(&m, &objects).is_ok());
}

#[test]
fn excessive_nesting_and_trailing_json_are_rejected() {
    let (v, o) = fixture();
    let text = serde_json::to_string(&v).unwrap();
    assert!(admit(format!("{text} {{}}").as_bytes(), &o).is_err());
    let deep = format!(
        "{},\"extra\":{}0{} }}",
        text.strip_suffix('}').unwrap(),
        "[".repeat(130),
        "]".repeat(130)
    );
    assert!(admit(deep.as_bytes(), &o).is_err());
}

#[cfg(all(feature = "coop-audio-load", target_os = "linux"))]
mod loader {
    use super::*;
    use std::{fs, os::unix::fs::symlink};
    fn write_bank(path: &std::path::Path) {
        let (manifest, objects) = fixture();
        fs::write(
            path.join("bank.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        for (name, bytes) in objects {
            fs::write(path.join(name), bytes).unwrap();
        }
    }
    #[test]
    fn loads_only_fixed_files_without_license_or_sources() {
        let dir = tempfile::tempdir().unwrap();
        write_bank(dir.path());
        // Unrelated entries are not opened, including symlinks.
        symlink("/not-a-bank", dir.path().join("unrelated")).unwrap();
        let bank = load_bank(dir.path()).unwrap();
        assert_eq!(bank.records().len(), CUES.len());
        assert_eq!(bank.records()[9].cue(), "death");
        assert_eq!(&bank.records()[0].pcm_bytes()[8..], &[0, 224, 0, 32]);
        let (manifest, objects) = fixture();
        let expected = admit_value(&manifest, &objects).unwrap();
        for (actual, expected) in bank.records().iter().zip(expected.records()) {
            assert_eq!((actual.cue(), actual.frames(), actual.pcm_bytes()),
                (expected.cue(), expected.frames(), expected.pcm_bytes()));
        }
        assert!(load_bank(std::path::Path::new("relative")).is_err());
        assert!(load_bank(&dir.path().join("..")).is_err());
    }
    #[test]
    fn rejects_linked_ancestry_manifest_and_pcm() {
        let root = tempfile::tempdir().unwrap();
        let bank = root.path().join("bank");
        fs::create_dir(&bank).unwrap();
        write_bank(&bank);
        let alias = root.path().join("alias");
        symlink(&bank, &alias).unwrap();
        assert!(load_bank(&alias).is_err());
        for name in ["bank.json", "death.pcm"] {
            let path = bank.join(name);
            let saved = root.path().join(name);
            fs::rename(&path, &saved).unwrap();
            symlink(&saved, &path).unwrap();
            assert!(load_bank(&bank).is_err());
            fs::remove_file(&path).unwrap();
            fs::rename(saved, path).unwrap();
        }
    }
    #[test]
    fn rejects_oversize_missing_and_nonregular_inputs() {
        let dir = tempfile::tempdir().unwrap();
        write_bank(dir.path());
        for (name, bound) in [("bank.json", 128 * 1024), ("death.pcm", 96008)] {
            let path = dir.path().join(name);
            let original = fs::read(&path).unwrap();
            fs::write(&path, vec![0; bound + 1]).unwrap();
            assert!(load_bank(dir.path()).is_err());
            fs::remove_file(&path).unwrap();
            assert!(load_bank(dir.path()).is_err());
            fs::create_dir(&path).unwrap();
            assert!(load_bank(dir.path()).is_err());
            fs::remove_dir(&path).unwrap();
            fs::write(path, original).unwrap();
        }
    }
}
