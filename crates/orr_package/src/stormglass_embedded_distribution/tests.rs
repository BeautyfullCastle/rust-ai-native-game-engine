use super::*;
pub(super) const SCENE: &[u8] = b"schema: orr.scene/1\nentities: {}\n";
fn put16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
// Synthetic header fixture only. This is deliberately not a runnable executable.
pub(super) fn fixture() -> Vec<u8> {
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

fn args() -> Vec<OsString> {
    [
        "--runtime",
        r"C:\trusted\runtime.exe",
        "--scene",
        r"C:\trusted\scene.yaml",
        "--runtime-sha256",
        &"a".repeat(64),
        "--output",
        r"C:\delivered\game.exe",
        "--trusted-runtime",
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}
#[test]
fn arguments_require_explicit_trust_and_reject_ambiguity_before_io() {
    let valid = Options::parse(args()).unwrap();
    assert!(valid.trusted_runtime && !valid.dry_run);
    let mut no_trust = args();
    no_trust.pop();
    assert!(Options::parse(no_trust.clone())
        .unwrap_err()
        .contains("trusted-runtime"));
    no_trust.push("--dry-run".into());
    assert!(Options::parse(no_trust).unwrap().dry_run);
    for extra in ["--force", "--trusted-runtime", "--runtime", "--json", "--"] {
        let mut a = args();
        a.push(extra.into());
        assert!(Options::parse(a).is_err());
    }
    let mut invalid = valid.clone();
    invalid.runtime_sha256 = "g".repeat(64);
    assert!(validate(&invalid).is_err());
    invalid = valid.clone();
    invalid.output = valid.runtime;
    assert!(validate(&invalid).unwrap_err().contains("distinct"));
}
#[test]
fn paths_reject_devices_streams_aliases_and_unsupported_spelling() {
    for path in [r"C:\x\a.exe", r"D:\folder-name\a_b.exe"] {
        validate_path(Path::new(path)).unwrap();
    }
    for path in [
        r"C:\a.exe",
        r"C:relative.exe",
        r"C:\x\\game.exe",
        r"C:\x\.\game.exe",
        r"C:\x\..\game.exe",
        r"C:/x/game.exe",
        r"\\server\share\game.exe",
        r"\\?\C:\x\game.exe",
        r"C:\x\game.exe:stream",
        r"C:\x\NUL.exe",
        r"C:\COM1\game.exe",
        r"C:\x\game.exe.",
        r"C:\x\game.exe ",
        r"C:\x\a b.exe",
        r"C:\SHORT~1\game.exe",
        r"C:\x\한글.exe",
        "C:\\x\\a\0.exe",
    ] {
        assert!(validate_path(Path::new(path)).is_err(), "{path:?}");
    }
}
#[test]
fn content_checks_bind_hash_pe_and_strict_scene() {
    assert_eq!(
        hash(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    content::validate_pe(&fixture()).unwrap();
    for n in 0..400 {
        assert!(content::validate_pe(&fixture()[..n]).is_err());
    }
    for (offset, byte) in [(0, 0), (68, 0), (86, 0), (89, 0), (367, 0)] {
        let mut bytes = fixture();
        bytes[offset] = byte;
        assert!(content::validate_pe(&bytes).is_err());
    }
    let comment = [SCENE, b"# comment\n"].concat();
    assert_eq!(expected(SCENE).unwrap(), expected(&comment).unwrap());
    assert_ne!(hash(SCENE), hash(&comment));
    assert!(expected(b"invalid: source").is_err());
    assert!(expected(&vec![b'x'; 65537]).is_err());
    assert_eq!(MAX_RUNTIME_BYTES, 128 * 1024 * 1024);
}
