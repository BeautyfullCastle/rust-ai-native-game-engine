use super::*;

const SCENE: &[u8] = b"schema: orr.scene/1\nentities: {}\n";

fn put16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
// Synthetic header fixture only. This is deliberately not a runnable executable.
fn fixture() -> Vec<u8> {
    let mut bytes = vec![0u8; 1024];
    bytes[..2].copy_from_slice(b"MZ");
    put32(&mut bytes, 0x3c, 64);
    bytes[64..68].copy_from_slice(b"PE\0\0");
    put16(&mut bytes, 68, 0x8664);
    put16(&mut bytes, 70, 1);
    put16(&mut bytes, 84, 240);
    put16(&mut bytes, 86, 2);
    put16(&mut bytes, 88, 0x20b);
    put32(&mut bytes, 104, 4096);
    put32(&mut bytes, 148, 512);
    put16(&mut bytes, 156, 3);
    put32(&mut bytes, 340, 4096);
    put32(&mut bytes, 344, 512);
    put32(&mut bytes, 348, 512);
    put32(&mut bytes, 364, 0x2000_0000);
    bytes
}
fn hash(bytes: &[u8]) -> String {
    digest(RUNTIME_NAME, bytes).sha256
}
#[test]
fn sha256_matches_known_vector() {
    assert_eq!(
        hash(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
fn reject(bytes: &[u8]) {
    assert!(admit(bytes, &hash(bytes), SCENE).is_err());
}

#[test]
fn fixed_inputs_preserve_exact_bytes_config_and_digests() {
    let runtime = fixture();
    let content = admit(&runtime, &hash(&runtime).to_uppercase(), SCENE).unwrap();
    assert_eq!(content.runtime().as_ptr(), runtime.as_ptr());
    assert_eq!(content.scene(), SCENE);
    assert_eq!(content.config(), &DefenseConfig::default());
    assert_eq!(
        content.files(),
        &[
            digest(RUNTIME_NAME, &runtime),
            digest(SCENE_NAME, SCENE),
            digest(LAUNCHER_NAME, LAUNCHER)
        ]
    );
    assert_eq!(content.files().len(), 3);
}

#[test]
fn pin_is_required_exact_hex_and_must_match_snapshot() {
    let runtime = fixture();
    for pin in [
        "".to_owned(),
        "0".repeat(63),
        "0".repeat(65),
        "g".repeat(64),
        "0".repeat(64),
        format!(" {}", hash(&runtime)),
    ] {
        assert!(admit(&runtime, &pin, SCENE).is_err());
    }
    let pin = hash(&runtime);
    let mut changed = runtime;
    changed[800] ^= 1;
    assert_eq!(
        admit(&changed, &pin, SCENE).unwrap_err(),
        "runtime SHA256 mismatch"
    );
}

#[test]
fn scene_uses_shared_strict_parser_and_preserves_authored_config() {
    let runtime = fixture();
    let pin = hash(&runtime);
    let scene = b"schema: orr.scene/1\nentities: {}\nsingletons: {CoopDefense::Scenario: {starting_gold: 250, reactor_hp: 75}}\n";
    let content = admit(&runtime, &pin, scene).unwrap();
    assert_eq!(content.config().scenario.starting_gold, 250);
    assert_eq!(content.config().scenario.reactor_hp, 75);
    assert_eq!(content.files()[1], digest(SCENE_NAME, scene));
    for invalid in [b"\xff".as_slice(), b"invalid: true", b"schema: orr.scene/1\nentities: {}\nsingletons: {CoopDefense::Scenario: {reactor_hp: 75}}\n"] {
        assert!(admit(&runtime, &pin, invalid).is_err());
    }
    assert_eq!(
        admit(&runtime, &pin, &vec![b' '; MAX_SOURCE_BYTES + 1]).unwrap_err(),
        "scene exceeds 64 KiB"
    );
}

#[test]
fn every_truncated_header_is_rejected_without_panicking() {
    let runtime = fixture();
    for end in 0..runtime.len() {
        reject(&runtime[..end]);
    }
}

#[test]
fn rejects_wrong_platform_dll_signature_and_subsystem() {
    for (offset, value) in [
        (0, 0),
        (64, 0),
        (68, 0x14c),
        (86, 0),
        (86, 0x2002),
        (88, 0x10b),
        (156, 1),
    ] {
        let mut runtime = fixture();
        put16(&mut runtime, offset, value);
        reject(&runtime);
    }
}

#[test]
fn rejects_unbounded_offsets_sections_and_entry_points() {
    for (offset, value) in [
        (60, u32::MAX),
        (60, 0),
        (148, 0),
        (148, 1025),
        (104, 0),
        (104, 4608),
        (344, u32::MAX),
        (348, 0),
        (348, u32::MAX),
        (364, 0),
    ] {
        let mut runtime = fixture();
        put32(&mut runtime, offset, value);
        reject(&runtime);
    }
    for (offset, value) in [(70, 0), (70, 97), (84, 111), (84, u16::MAX)] {
        let mut runtime = fixture();
        put16(&mut runtime, offset, value);
        reject(&runtime);
    }
}

#[test]
fn scene_and_launcher_hashes_are_content_not_paths() {
    let runtime = fixture();
    let changed = [SCENE, b"\n"].concat();
    let first = admit(&runtime, &hash(&runtime), SCENE).unwrap();
    let second = admit(&runtime, &hash(&runtime), &changed).unwrap();
    assert_ne!(first.files()[1], second.files()[1]);
    assert_eq!(first.files()[0], second.files()[0]);
    assert_eq!(first.files()[2], second.files()[2]);
}

#[test]
#[ignore = "requires explicit trusted local runtime path and SHA256; only reads bytes, never executes"]
fn real_local_runtime_content_admission() {
    let path = std::env::var_os("STORMGLASS_ADMISSION_RUNTIME").expect("set runtime path");
    let pin = std::env::var("STORMGLASS_ADMISSION_SHA256").expect("set trusted runtime SHA256");
    let bytes = std::fs::read(path).unwrap();
    let scene_path = std::env::var_os("STORMGLASS_ADMISSION_SCENE").expect("set scene path");
    let scene = std::fs::read(scene_path).unwrap();
    let content = admit(&bytes, &pin, &scene).unwrap();
    assert_eq!(content.files()[0].sha256, pin.to_ascii_lowercase());
}
