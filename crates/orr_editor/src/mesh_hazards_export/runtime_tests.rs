use super::*;

fn put16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn put64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

pub(super) fn fixture(platform: RuntimePlatform) -> Vec<u8> {
    match platform {
        RuntimePlatform::WindowsX86_64 => pe(),
        RuntimePlatform::LinuxX86_64 => elf(),
    }
}
pub(super) fn camera_fixture(platform: RuntimePlatform) -> Vec<u8> {
    match platform {
        RuntimePlatform::WindowsX86_64 => pe_with_camera(),
        RuntimePlatform::LinuxX86_64 => elf_with_camera(),
    }
}
fn pe() -> Vec<u8> {
    let mut bytes = vec![0; 1536];
    bytes[..2].copy_from_slice(b"MZ");
    put32(&mut bytes, 0x3c, 64);
    bytes[64..68].copy_from_slice(b"PE\0\0");
    put16(&mut bytes, 68, 0x8664);
    put16(&mut bytes, 70, 2);
    put16(&mut bytes, 84, 112);
    put16(&mut bytes, 86, 0x22);
    put16(&mut bytes, 88, 0x20b);
    put32(&mut bytes, 104, 4096);
    put32(&mut bytes, 120, 4096);
    put32(&mut bytes, 124, 512);
    put32(&mut bytes, 144, 12288);
    put32(&mut bytes, 148, 512);
    bytes[200..205].copy_from_slice(b".text");
    put32(&mut bytes, 208, 1);
    put32(&mut bytes, 212, 4096);
    put32(&mut bytes, 216, 512);
    put32(&mut bytes, 220, 512);
    put32(&mut bytes, 236, 0x6000_0020);
    bytes[240..248].copy_from_slice(b".orrhaz\0");
    put32(&mut bytes, 248, CAPABILITY_MARKER.len() as u32);
    put32(&mut bytes, 252, 8192);
    put32(&mut bytes, 256, 512);
    put32(&mut bytes, 260, 1024);
    put32(&mut bytes, 276, 0x4000_0040);
    bytes[512] = 0xc3;
    bytes[1024..1024 + CAPABILITY_MARKER.len()].copy_from_slice(CAPABILITY_MARKER);
    bytes
}
fn elf() -> Vec<u8> {
    let mut bytes = vec![0; 768];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    put16(&mut bytes, 16, 3);
    put16(&mut bytes, 18, 62);
    put32(&mut bytes, 20, 1);
    put64(&mut bytes, 24, 0x400100);
    put64(&mut bytes, 32, 64);
    put64(&mut bytes, 40, 512);
    put16(&mut bytes, 52, 64);
    put16(&mut bytes, 54, 56);
    put16(&mut bytes, 56, 1);
    put16(&mut bytes, 58, 64);
    put16(&mut bytes, 60, 4);
    put16(&mut bytes, 62, 3);
    put32(&mut bytes, 64, 1);
    put32(&mut bytes, 68, 5);
    put64(&mut bytes, 80, 0x400000);
    put64(&mut bytes, 96, 512);
    put64(&mut bytes, 104, 512);
    put64(&mut bytes, 112, 4096);
    bytes[256] = 0xc3;
    bytes[288..288 + CAPABILITY_MARKER.len()].copy_from_slice(CAPABILITY_MARKER);
    let names = b"\0.text\0.orrhaz\0.shstrtab\0";
    bytes[320..320 + names.len()].copy_from_slice(names);
    put32(&mut bytes, 576, 1);
    put32(&mut bytes, 580, 1);
    put64(&mut bytes, 584, 6);
    put64(&mut bytes, 592, 0x400100);
    put64(&mut bytes, 600, 256);
    put64(&mut bytes, 608, 1);
    put32(&mut bytes, 640, 7);
    put32(&mut bytes, 644, 1);
    put64(&mut bytes, 648, 2);
    put64(&mut bytes, 656, 0x400120);
    put64(&mut bytes, 664, 288);
    put64(&mut bytes, 672, CAPABILITY_MARKER.len() as u64);
    put32(&mut bytes, 704, 15);
    put32(&mut bytes, 708, 3);
    put64(&mut bytes, 728, 320);
    put64(&mut bytes, 736, names.len() as u64);
    bytes
}

fn pe_with_camera() -> Vec<u8> {
    let mut bytes = pe();
    bytes.resize(2048, 0);
    put16(&mut bytes, 70, 3);
    put32(&mut bytes, 144, 16384);
    let section = 280;
    bytes[section..section + 8].copy_from_slice(b".orrhcam");
    put32(
        &mut bytes,
        section + 8,
        CAMERA_CAPABILITY_MARKER.len() as u32,
    );
    put32(&mut bytes, section + 12, 12288);
    put32(&mut bytes, section + 16, 512);
    put32(&mut bytes, section + 20, 1536);
    put32(&mut bytes, section + 36, 0x4000_0040);
    bytes[1536..1536 + CAMERA_CAPABILITY_MARKER.len()].copy_from_slice(CAMERA_CAPABILITY_MARKER);
    bytes
}

fn elf_with_camera() -> Vec<u8> {
    let mut bytes = elf();
    bytes.resize(832, 0);
    put16(&mut bytes, 60, 5);
    let names = b"\0.text\0.orrhaz\0.shstrtab\0.orrhcam\0";
    bytes[320..320 + names.len()].copy_from_slice(names);
    bytes[384..384 + CAMERA_CAPABILITY_MARKER.len()].copy_from_slice(CAMERA_CAPABILITY_MARKER);
    let section = 768;
    put32(&mut bytes, section, 25);
    put32(&mut bytes, section + 4, 1);
    put64(&mut bytes, section + 8, 2);
    put64(&mut bytes, section + 16, 0x400180);
    put64(&mut bytes, section + 24, 384);
    put64(
        &mut bytes,
        section + 32,
        CAMERA_CAPABILITY_MARKER.len() as u64,
    );
    put64(&mut bytes, section + 48, 1);
    // The enlarged name string table remains the final section payload.
    put64(&mut bytes, 736, names.len() as u64);
    bytes
}

#[test]
fn valid_platform_fixtures() {
    assert!(matches!(
        validate_runtime(&pe()),
        Ok(RuntimePlatform::WindowsX86_64)
    ));
    assert!(matches!(
        validate_runtime(&elf()),
        Ok(RuntimePlatform::LinuxX86_64)
    ));
}
#[test]
fn camera_runtime_requires_both_exact_capabilities() {
    for platform in [RuntimePlatform::WindowsX86_64, RuntimePlatform::LinuxX86_64] {
        let player_only = fixture(platform);
        assert!(validate_runtime(&player_only).is_ok());
        assert!(
            validate_camera_runtime(&player_only)
                .unwrap_err()
                .contains("camera capability section is missing")
        );

        let both = camera_fixture(platform);
        assert!(validate_runtime(&both).is_ok());
        assert_eq!(validate_camera_runtime(&both), Ok(platform));
    }
}

#[test]
fn camera_capability_sections_are_strictly_validated_when_optional_or_required() {
    // Each mutation keeps the player capability intact and damages only camera metadata.
    let mut pe_bad_marker = pe_with_camera();
    pe_bad_marker[1536] ^= 1;
    let mut pe_bad_name = pe_with_camera();
    pe_bad_name[280] = b'x';
    let mut pe_bad_flags = pe_with_camera();
    put32(&mut pe_bad_flags, 316, 0xe000_0040);
    let mut pe_bad_size = pe_with_camera();
    put32(
        &mut pe_bad_size,
        288,
        (CAMERA_CAPABILITY_MARKER.len() - 1) as u32,
    );
    let mut pe_bad_padding = pe_with_camera();
    pe_bad_padding[1536 + CAMERA_CAPABILITY_MARKER.len()] = 1;
    for bytes in [pe_bad_marker, pe_bad_flags, pe_bad_size, pe_bad_padding] {
        assert!(validate_runtime(&bytes).is_err());
        assert!(validate_camera_runtime(&bytes).is_err());
    }
    // An unrecognized section name is not a capability; strict camera admission
    // still rejects it as missing, while the legacy player-only contract remains valid.
    assert!(validate_runtime(&pe_bad_name).is_ok());
    assert!(validate_camera_runtime(&pe_bad_name).is_err());

    let mut elf_bad_marker = elf_with_camera();
    elf_bad_marker[384] ^= 1;
    let mut elf_bad_name = elf_with_camera();
    elf_bad_name[768] = 7; // disguises the camera payload as a duplicate player capability
    let mut elf_bad_flags = elf_with_camera();
    put64(&mut elf_bad_flags, 776, 3);
    let mut elf_bad_size = elf_with_camera();
    put64(
        &mut elf_bad_size,
        800,
        (CAMERA_CAPABILITY_MARKER.len() - 1) as u64,
    );
    let mut elf_bad_mapping = elf_with_camera();
    put64(&mut elf_bad_mapping, 784, 0x500180);
    for bytes in [
        elf_bad_marker,
        elf_bad_name,
        elf_bad_flags,
        elf_bad_size,
        elf_bad_mapping,
    ] {
        assert!(validate_runtime(&bytes).is_err());
        assert!(validate_camera_runtime(&bytes).is_err());
    }
}

#[test]
fn duplicate_camera_sections_are_rejected_in_pe_and_elf() {
    let mut pe = pe_with_camera();
    put16(&mut pe, 70, 4);
    put32(&mut pe, 144, 20480);
    pe.resize(2560, 0);
    let section = 320;
    pe[section..section + 8].copy_from_slice(b".orrhcam");
    put32(&mut pe, section + 8, CAMERA_CAPABILITY_MARKER.len() as u32);
    put32(&mut pe, section + 12, 16384);
    put32(&mut pe, section + 16, 512);
    put32(&mut pe, section + 20, 2048);
    put32(&mut pe, section + 36, 0x4000_0040);
    pe[2048..2048 + CAMERA_CAPABILITY_MARKER.len()].copy_from_slice(CAMERA_CAPABILITY_MARKER);
    assert!(
        validate_camera_runtime(&pe)
            .unwrap_err()
            .contains("duplicate")
    );

    let mut elf = elf_with_camera();
    elf.resize(896, 0);
    put16(&mut elf, 60, 6);
    // Second .orrhcam section aliases the same valid, read-only marker payload.
    let section = 832;
    elf[416..416 + CAMERA_CAPABILITY_MARKER.len()].copy_from_slice(CAMERA_CAPABILITY_MARKER);
    put32(&mut elf, section, 25);
    put32(&mut elf, section + 4, 1);
    put64(&mut elf, section + 8, 2);
    put64(&mut elf, section + 16, 0x4001a0);
    put64(&mut elf, section + 24, 416);
    put64(
        &mut elf,
        section + 32,
        CAMERA_CAPABILITY_MARKER.len() as u64,
    );
    put64(&mut elf, section + 48, 1);
    assert!(
        validate_camera_runtime(&elf)
            .unwrap_err()
            .contains("duplicate")
    );
}
#[test]
fn every_truncation_rejected_without_panicking() {
    for bytes in [pe(), elf()] {
        for length in 0..bytes.len() {
            assert!(
                validate_runtime(&bytes[..length]).is_err(),
                "length {length}"
            );
        }
    }
    for bytes in [pe_with_camera(), elf_with_camera()] {
        for length in 0..bytes.len() {
            assert!(
                validate_camera_runtime(&bytes[..length]).is_err(),
                "camera runtime length {length}"
            );
        }
    }
}
#[test]
fn marker_is_not_an_arbitrary_substring() {
    assert!(validate_runtime(CAPABILITY_MARKER).is_err());
    for (mut bytes, offset) in [(pe(), 1024), (elf(), 288)] {
        bytes[offset] = 0;
        bytes.extend_from_slice(CAPABILITY_MARKER);
        assert!(validate_runtime(&bytes).is_err());
    }
}
#[test]
fn pe_rejects_wrong_architecture_and_header_variants() {
    for (offset, value) in [
        (68, 0x14c),
        (86, 0x2002),
        (86, 0),
        (88, 0x10b),
        (70, 0),
        (70, 4097),
        (84, 111),
    ] {
        let mut bytes = pe();
        put16(&mut bytes, offset, value);
        assert!(validate_runtime(&bytes).is_err(), "offset {offset}");
    }
    for (offset, value) in [
        (0x3c, u32::MAX),
        (148, 279),
        (196, 17),
        (260, u32::MAX),
        (256, u32::MAX),
        (248, 0),
        (104, 0),
        (144, 8193),
        (252, 4096),
        (260, 512),
        (276, 0xe000_0040),
    ] {
        let mut bytes = pe();
        put32(&mut bytes, offset, value);
        assert!(validate_runtime(&bytes).is_err(), "offset {offset}");
    }
}
#[test]
fn pe_rejects_missing_duplicate_and_mispositioned_markers() {
    let mut bytes = pe();
    bytes[240] = b'x';
    assert!(validate_runtime(&bytes).is_err());
    let mut bytes = pe();
    bytes[200..208].copy_from_slice(b".orrhaz\0");
    assert!(validate_runtime(&bytes).is_err());
    let mut bytes = pe();
    bytes[1048] = 1;
    assert!(validate_runtime(&bytes).is_err());
    let mut bytes = pe();
    bytes.copy_within(1024..1048, 1025);
    bytes[1024] = 0;
    assert!(validate_runtime(&bytes).is_err());
}
#[test]
fn elf_rejects_wrong_architecture_and_tables() {
    for (offset, value) in [(4, 1), (5, 2), (6, 0)] {
        let mut bytes = elf();
        bytes[offset] = value;
        assert!(validate_runtime(&bytes).is_err());
    }
    for (offset, value) in [
        (16, 1),
        (18, 183),
        (52, 63),
        (54, 55),
        (56, 0),
        (58, 63),
        (60, 0),
        (62, 0),
        (62, 4),
        (60, 4097),
    ] {
        let mut bytes = elf();
        put16(&mut bytes, offset, value);
        assert!(validate_runtime(&bytes).is_err(), "offset {offset}");
    }
    for (offset, value) in [
        (32, u64::MAX),
        (40, u64::MAX),
        (32, 0),
        (40, 64),
        (96, u64::MAX),
        (104, 0),
        (80, u64::MAX),
        (24, 0),
        (664, u64::MAX),
        (672, u64::MAX),
        (728, u64::MAX),
        (736, u64::MAX),
        (648, 3),
        (656, 0),
        (672, 23),
        (664, 256),
    ] {
        let mut bytes = elf();
        put64(&mut bytes, offset, value);
        assert!(validate_runtime(&bytes).is_err(), "offset {offset}");
    }
}
#[test]
fn elf_rejects_missing_duplicate_unmapped_and_invalid_section_names() {
    for (offset, value) in [
        (640, 1),
        (576, 7),
        (640, u32::MAX),
        (644, 8),
        (708, 1),
        (68, 7),
        (68, 4),
    ] {
        let mut bytes = elf();
        put32(&mut bytes, offset, value);
        assert!(validate_runtime(&bytes).is_err(), "offset {offset}");
    }
    let mut bytes = elf();
    bytes[512] = 1;
    assert!(validate_runtime(&bytes).is_err());
    let mut bytes = elf();
    bytes[344] = b'x';
    assert!(validate_runtime(&bytes).is_err());
}
#[test]
fn adversarial_single_byte_mutations_never_panic() {
    for original in [pe(), elf()] {
        for index in 0..original.len() {
            let mut bytes = original.clone();
            for value in [0, 0x7f, 0xff] {
                bytes[index] = value;
                let _ = validate_runtime(&bytes);
            }
        }
    }
}

#[test]
fn duplicates_are_rejected_even_when_both_sections_have_valid_markers() {
    let mut bytes = pe();
    bytes[200..208].copy_from_slice(b".orrhaz\0");
    put32(&mut bytes, 208, CAPABILITY_MARKER.len() as u32);
    put32(&mut bytes, 236, 0x4000_0040);
    bytes[512..512 + CAPABILITY_MARKER.len()].copy_from_slice(CAPABILITY_MARKER);
    assert!(validate_runtime(&bytes).unwrap_err().contains("duplicate"));
    let mut bytes = elf();
    put32(&mut bytes, 576, 7);
    put64(&mut bytes, 584, 2);
    put64(&mut bytes, 608, CAPABILITY_MARKER.len() as u64);
    bytes[256..256 + CAPABILITY_MARKER.len()].copy_from_slice(CAPABILITY_MARKER);
    assert!(validate_runtime(&bytes).unwrap_err().contains("duplicate"));
}

#[test]
fn elf_allows_gnu_retain_but_no_other_capability_flags() {
    let mut bytes = elf();
    put64(&mut bytes, 648, 0x20_0002);
    assert!(validate_runtime(&bytes).is_ok());
    for flag in [
        1,
        4,
        8,
        16,
        32,
        64,
        128,
        256,
        512,
        1024,
        0x8000_0000_0000_0000,
    ] {
        put64(&mut bytes, 648, 2 | flag);
        assert!(validate_runtime(&bytes).is_err());
    }
}

#[test]
fn capability_marker_must_be_complete_and_padding_zero() {
    let mut bytes = pe();
    bytes[1024 + CAPABILITY_MARKER.len()] = 1;
    assert!(validate_runtime(&bytes).is_err());
    let mut bytes = pe();
    let len = CAPABILITY_MARKER.len();
    bytes.copy_within(1024..1024 + len, 1025);
    bytes[1024] = 0;
    assert!(validate_runtime(&bytes).is_err());
    let mut bytes = elf();
    bytes[288 + len - 1] = 0;
    assert!(validate_runtime(&bytes).is_err());
}

#[test]
fn alignment_and_unbacked_entry_points_are_rejected() {
    for (offset, value) in [(120, 3), (124, 0), (124, 511), (220, 513), (212, 4097)] {
        let mut bytes = pe();
        put32(&mut bytes, offset, value);
        assert!(validate_runtime(&bytes).is_err());
    }
    let mut bytes = pe();
    put32(&mut bytes, 208, 1024);
    put32(&mut bytes, 104, 4096 + 512);
    assert!(validate_runtime(&bytes).is_err());
    for (offset, value) in [(112, 3), (80, 0x400001), (688, 3), (656, u64::MAX)] {
        let mut bytes = elf();
        put64(&mut bytes, offset, value);
        assert!(validate_runtime(&bytes).is_err());
    }
    let mut bytes = elf();
    put64(&mut bytes, 104, 4096);
    put64(&mut bytes, 24, 0x400200);
    assert!(validate_runtime(&bytes).is_err());
}

/// Opt-in CI/QA: validates a deliberately supplied self-built native executable,
/// never launches it. Hash pinning prevents validating a different build by accident.
#[test]
#[ignore = "requires explicit native runtime path and SHA-256 pin"]
fn actual_native_runtime_is_admitted() {
    use sha2::{Digest, Sha256};
    let path = std::path::PathBuf::from(
        std::env::var_os("ORR_MESH_HAZARDS_RUNTIME")
            .expect("set ORR_MESH_HAZARDS_RUNTIME to the self-built native player"),
    );
    let expected = std::env::var("ORR_MESH_HAZARDS_RUNTIME_SHA256")
        .expect("set ORR_MESH_HAZARDS_RUNTIME_SHA256 to the exact build digest");
    assert_eq!(
        expected.len(),
        64,
        "expected SHA-256 must have 64 lowercase hexadecimal digits"
    );
    assert!(
        expected
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
    let path = super::super::io::checked_existing_file(
        &path,
        super::super::MAX_RUNTIME_BYTES,
        "native runtime",
    )
    .unwrap();
    let bytes =
        super::super::io::read_bounded(&path, super::super::MAX_RUNTIME_BYTES, "native runtime")
            .unwrap();
    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(
        digest, expected,
        "native runtime differs from the exact build digest"
    );
    let platform = validate_camera_runtime(&bytes).expect(
        "native runtime container and exact .orrhaz/.orrhcam capabilities must be admitted",
    );
    assert!(
        platform.is_current(),
        "runtime must match the native test host"
    );
    eprintln!(
        "native hazard runtime admitted: path={} bytes={} sha256={} platform={platform:?} marker={} + {}",
        path.display(),
        bytes.len(),
        digest,
        std::str::from_utf8(CAPABILITY_MARKER).unwrap(),
        std::str::from_utf8(CAMERA_CAPABILITY_MARKER).unwrap()
    );
}
