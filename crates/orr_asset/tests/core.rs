use orr_asset::*;
use orr_fp::FP;

fn entry(id: u64) -> ManifestEntry {
    ManifestEntry {
        id: AssetRef::from_raw(id),
        type_id: 1,
        schema_version: 1,
        payload_len: 8,
        payload_sha256: [0x5a; 32],
    }
}
fn view_entry(id: u64, frames: u32) -> ManifestEntry {
    ManifestEntry {
        type_id: 2,
        payload_len: 8 + u64::from(frames) * 2,
        ..entry(id)
    }
}
fn encode(domain: Domain, entries: &[ManifestEntry]) -> Vec<u8> {
    let mut out = vec![0; manifest_encoded_len(domain, entries.len()).unwrap()];
    encode_manifest(domain, entries, &mut out).unwrap();
    out
}
fn err(bytes: &[u8], domain: Domain) -> AssetError {
    Manifest::decode(bytes, domain).unwrap_err()
}
fn write32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn write64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

#[test]
fn guid_pod_and_authoring_roundtrip_full_range() {
    fn pod<T: bytemuck::Pod + bytemuck::Zeroable>() {}
    pod::<AssetRef>();
    assert_eq!(core::mem::size_of::<AssetRef>(), 8);
    assert_eq!(AssetRef::default(), AssetRef::NULL);
    for n in [0, 1, 0x1001, (1 << 53) + 1, u64::MAX] {
        let id = AssetRef::from_raw(n);
        assert_eq!(id.to_string().parse::<AssetRef>(), Ok(id));
        assert_eq!(id.is_null(), n == 0);
    }
    for bad in [
        "",
        "a_1",
        "e_0000000000001001",
        "A_0000000000001001",
        "a_000000000000100A",
        "a_10000000000000000",
        "a_fffffffffffffffff",
        "a_0000000000001001\n",
        "a_000000000000100g",
        "a_é00000000000000",
    ] {
        assert_eq!(
            bad.parse::<AssetRef>(),
            Err(AssetError::InvalidId),
            "{bad:?}"
        );
    }
}

#[test]
fn sim_manifest_golden_bytes_and_unaligned_decode() {
    // Independent literal wire vector: ORAM v1/sim/count=1, GUID=0x1001,
    // type/schema=1, eight-byte payload, digest 00..1f.
    let expected: [u8; 72] = [
        0x4f, 0x52, 0x41, 0x4d, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0x10, 0, 0, 0, 0, 0, 0, 1,
        0, 0, 0, 1, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13,
        14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
    ];
    let mut e = entry(0x1001);
    e.payload_sha256 = core::array::from_fn(|n| n as u8);
    assert_eq!(encode(Domain::Sim, &[e]), expected);
    let mut unaligned = vec![0xff];
    unaligned.extend_from_slice(&expected);
    let m = Manifest::decode(&unaligned[1..], Domain::Sim).unwrap();
    assert_eq!(m.as_bytes(), expected);
    assert_eq!(m.domain(), Domain::Sim);
    assert_eq!(m.entries().collect::<Vec<_>>(), vec![e]);
    assert_eq!(m.find(e.id).unwrap(), e);
    assert_eq!(m.typed_ref::<MotionProfileV1>(e.id).unwrap().raw(), e.id);
}

#[test]
fn view_and_empty_golden_bytes() {
    let expected_empty = [b'O', b'R', b'A', b'M', 1, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(encode(Domain::View, &[]), expected_empty);
    assert_eq!(
        Manifest::decode(&expected_empty, Domain::View)
            .unwrap()
            .entries()
            .len(),
        0
    );
    let mut expected = vec![b'O', b'R', b'A', b'M', 1, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0];
    expected.extend_from_slice(&[
        1, 0x20, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 0x48, 0x38, 0, 0, 0, 0, 0, 0,
    ]);
    expected.extend_from_slice(&[0x5a; 32]);
    assert_eq!(encode(Domain::View, &[view_entry(0x2001, 7200)]), expected);
    assert_eq!(
        Manifest::decode(&expected, Domain::View)
            .unwrap()
            .typed_ref::<MotionProfileV1>(AssetRef::from_raw(0x2001)),
        Err(AssetError::WrongDomain)
    );
}

#[test]
fn every_truncation_and_trailing_bytes_rejected() {
    for domain in [Domain::Sim, Domain::View] {
        let e = if domain == Domain::Sim {
            entry(u64::MAX)
        } else {
            view_entry(u64::MAX, 48000)
        };
        let mut bytes = encode(domain, &[e]);
        for len in 0..bytes.len() {
            assert_eq!(err(&bytes[..len], domain), AssetError::Truncated, "{len}");
        }
        bytes.push(0);
        assert_eq!(err(&bytes, domain), AssetError::LengthMismatch);
    }
}

#[test]
fn header_rejections() {
    let good = encode(Domain::Sim, &[entry(1)]);
    let mut bytes = good.clone();
    bytes[0] ^= 1;
    assert_eq!(err(&bytes, Domain::Sim), AssetError::InvalidMagic);
    for version in [0, 2, u32::MAX] {
        bytes = good.clone();
        write32(&mut bytes, 4, version);
        assert_eq!(err(&bytes, Domain::Sim), AssetError::UnsupportedVersion);
    }
    for domain in [0, 2, u32::MAX] {
        bytes = good.clone();
        write32(&mut bytes, 8, domain);
        assert_eq!(err(&bytes, Domain::Sim), AssetError::WrongDomain);
    }
    bytes = good.clone();
    write32(&mut bytes, 12, 0);
    assert_eq!(err(&bytes, Domain::Sim), AssetError::LengthMismatch);
    bytes = good.clone();
    write32(&mut bytes, 12, 2);
    assert_eq!(err(&bytes, Domain::Sim), AssetError::Truncated);
    for count in [65, u32::MAX] {
        bytes = good.clone();
        write32(&mut bytes, 12, count);
        assert_eq!(err(&bytes, Domain::Sim), AssetError::BudgetExceeded);
    }
}

#[test]
fn entry_rejections_and_opaque_digests() {
    let good = encode(Domain::Sim, &[entry(1)]);
    let mut bytes = good.clone();
    write64(&mut bytes, 16, 0);
    assert_eq!(err(&bytes, Domain::Sim), AssetError::InvalidId);
    for type_id in [0, 3, u32::MAX] {
        bytes = good.clone();
        write32(&mut bytes, 24, type_id);
        assert_eq!(err(&bytes, Domain::Sim), AssetError::WrongType);
    }
    bytes = good.clone();
    write32(&mut bytes, 24, 2);
    assert_eq!(err(&bytes, Domain::Sim), AssetError::WrongDomain);
    for version in [0, 2, u32::MAX] {
        bytes = good.clone();
        write32(&mut bytes, 28, version);
        assert_eq!(err(&bytes, Domain::Sim), AssetError::UnsupportedVersion);
    }
    for len in [0, 7, 9, 65536] {
        bytes = good.clone();
        write64(&mut bytes, 32, len);
        assert_eq!(err(&bytes, Domain::Sim), AssetError::LengthMismatch);
    }
    for len in [65537, u64::MAX] {
        bytes = good.clone();
        write64(&mut bytes, 32, len);
        assert_eq!(err(&bytes, Domain::Sim), AssetError::BudgetExceeded);
    }
    for digest in [[0; 32], [255; 32]] {
        let e = ManifestEntry {
            payload_sha256: digest,
            ..entry(1)
        };
        let bytes = encode(Domain::Sim, &[e]);
        assert_eq!(
            Manifest::decode(&bytes, Domain::Sim)
                .unwrap()
                .find(e.id)
                .unwrap(),
            e
        );
    }
}

#[test]
fn ordering_duplicates_and_encoder_atomic_failure() {
    let original = encode(Domain::Sim, &[entry(1), entry(2)]);
    for (id, expected) in [(0, AssetError::InvalidId), (1, AssetError::DuplicateId)] {
        let mut bytes = original.clone();
        write64(&mut bytes, 72, id);
        assert_eq!(err(&bytes, Domain::Sim), expected);
    }
    let mut bytes = original;
    write64(&mut bytes, 16, 3);
    assert_eq!(err(&bytes, Domain::Sim), AssetError::UnsortedIds);
    for entries in [
        [entry(0), entry(2)],
        [entry(1), entry(1)],
        [entry(2), entry(1)],
    ] {
        let mut out = [0xa5; 128];
        assert!(encode_manifest(Domain::Sim, &entries, &mut out).is_err());
        assert_eq!(out, [0xa5; 128]);
    }
    for size in [0, 71, 73, 200] {
        let mut out = vec![0xa5; size];
        assert_eq!(
            encode_manifest(Domain::Sim, &[entry(1)], &mut out),
            Err(AssetError::LengthMismatch)
        );
        assert!(out.iter().all(|x| *x == 0xa5));
    }
}

#[test]
fn exact_limits_and_size_overflow() {
    assert_eq!(manifest_encoded_len(Domain::Sim, 64), Ok(3600));
    assert_eq!(manifest_encoded_len(Domain::View, 16), Ok(912));
    for domain in [Domain::Sim, Domain::View] {
        assert_eq!(
            manifest_encoded_len(domain, usize::MAX),
            Err(AssetError::BudgetExceeded)
        );
        let count = if domain == Domain::Sim { 64 } else { 16 };
        let mut entries: Vec<_> = (1..=count)
            .map(|n| {
                if domain == Domain::Sim {
                    entry(n)
                } else {
                    view_entry(n, 48000)
                }
            })
            .collect();
        let bytes = encode(domain, &entries);
        assert!(Manifest::decode(&bytes, domain).is_ok());
        entries.push(if domain == Domain::Sim {
            entry(count + 1)
        } else {
            view_entry(count + 1, 48000)
        });
        assert_eq!(
            encode_manifest(domain, &entries, &mut []),
            Err(AssetError::BudgetExceeded)
        );
    }
    assert_eq!(
        err(&vec![0; MAX_MANIFEST_BYTES + 1], Domain::Sim),
        AssetError::BudgetExceeded
    );
    // Actual input exactly at the general cap is still rejected for bad structure.
    assert_eq!(
        err(&vec![0; MAX_MANIFEST_BYTES], Domain::Sim),
        AssetError::InvalidMagic
    );
}

#[test]
fn view_lengths_require_whole_bounded_pcm_frames() {
    for frames in [1, 48000] {
        assert!(view_entry(1, frames).validate(Domain::View).is_ok());
    }
    for len in [0, 8, 9, 11, 96009, 96010] {
        let e = ManifestEntry {
            payload_len: len,
            ..view_entry(1, 1)
        };
        assert_eq!(e.validate(Domain::View), Err(AssetError::LengthMismatch));
        let mut bytes = encode(Domain::View, &[view_entry(1, 1)]);
        write64(&mut bytes, 32, len);
        assert_eq!(err(&bytes, Domain::View), AssetError::LengthMismatch);
    }
    assert_eq!(
        ManifestEntry {
            payload_len: u64::MAX,
            ..view_entry(1, 1)
        }
        .validate(Domain::View),
        Err(AssetError::BudgetExceeded)
    );
}

#[test]
fn motion_payload_golden_and_limits() {
    let profile = MotionProfileV1::new(FP::from_raw(4096)).unwrap();
    assert_eq!(profile.encode(), [0, 0x10, 0, 0, 0, 0, 0, 0]);
    assert_eq!(MotionProfileV1::decode(&profile.encode()), Ok(profile));
    for raw in [1i64, 16 * 65536] {
        assert_eq!(
            MotionProfileV1::decode(&raw.to_le_bytes())
                .unwrap()
                .speed_per_tick()
                .raw(),
            raw
        );
    }
    for raw in [i64::MIN, -1, 0, 16 * 65536 + 1, i64::MAX] {
        assert_eq!(
            MotionProfileV1::decode(&raw.to_le_bytes()),
            Err(AssetError::InvalidMotion)
        );
    }
    for len in [0, 1, 7, 9, 16] {
        assert_eq!(
            MotionProfileV1::decode(&vec![0; len]),
            Err(AssetError::LengthMismatch)
        );
    }
}

#[test]
fn typed_table_stable_lookup_without_stored_indices() {
    let a = MotionProfileV1::new(FP::from_raw(4096)).unwrap();
    let b = MotionProfileV1::new(FP::from_raw(8192)).unwrap();
    let first = [
        (AssetRef::from_raw(10), a),
        (AssetRef::from_raw(u64::MAX), b),
    ];
    let expanded = [(AssetRef::from_raw(1), b), first[0], first[1]];
    let one = SimTable::new(&first).unwrap();
    let two = SimTable::new(&expanded).unwrap();
    let id = TypedRef::<MotionProfileV1>::from_entry(&entry(10)).unwrap();
    assert_eq!(one.resolve(id), Ok(&a));
    assert_eq!(two.resolve(id), Ok(&a));
    assert_eq!(two.entries(), &expanded);
    assert_eq!(
        two.resolve(TypedRef::from_entry(&entry(u64::MAX)).unwrap()),
        Ok(&b)
    );
    assert_eq!(
        two.resolve(TypedRef::from_entry(&entry(2)).unwrap()),
        Err(AssetError::MissingAsset)
    );
    assert_eq!(
        TypedRef::<MotionProfileV1>::from_entry(&entry(0)),
        Err(AssetError::InvalidId)
    );
    assert_eq!(
        TypedRef::<MotionProfileV1>::from_entry(&ManifestEntry {
            schema_version: 2,
            ..entry(1)
        }),
        Err(AssetError::UnsupportedVersion)
    );
    assert_eq!(
        TypedRef::<MotionProfileV1>::from_entry(&view_entry(1, 1)),
        Err(AssetError::WrongDomain)
    );
    for (entries, error) in [
        (vec![(AssetRef::NULL, a)], AssetError::InvalidId),
        (vec![first[0], first[0]], AssetError::DuplicateId),
        (vec![first[1], first[0]], AssetError::UnsortedIds),
        (vec![first[0]; 65], AssetError::BudgetExceeded),
    ] {
        assert_eq!(SimTable::new(&entries).unwrap_err(), error);
    }
    let empty = SimTable::<MotionProfileV1>::new(&[]).unwrap();
    assert_eq!(empty.resolve(id), Err(AssetError::MissingAsset));
}

#[test]
fn manifest_lookup_every_position_and_missing() {
    let entries: Vec<_> = (1..=64).map(|n| entry(n * 2)).collect();
    let bytes = encode(Domain::Sim, &entries);
    let m = Manifest::decode(&bytes, Domain::Sim).unwrap();
    for n in 1..=129 {
        let result = m.find(AssetRef::from_raw(n));
        if n % 2 == 0 {
            assert_eq!(result, Ok(entry(n)));
        } else {
            assert_eq!(result, Err(AssetError::MissingAsset));
        }
    }
    assert_eq!(m.find(AssetRef::NULL), Err(AssetError::InvalidId));
    assert_eq!(
        m.find(AssetRef::from_raw(u64::MAX)),
        Err(AssetError::MissingAsset)
    );
}

#[test]
fn maximum_table_and_empty_sim_manifest() {
    let profile = MotionProfileV1::new(FP::from_raw(1)).unwrap();
    let records: Vec<_> = (1..=64)
        .map(|id| (AssetRef::from_raw(id), profile))
        .collect();
    let table = SimTable::new(&records).unwrap();
    for id in 1..=64 {
        assert_eq!(
            table.resolve(TypedRef::from_entry(&entry(id)).unwrap()),
            Ok(&profile)
        );
    }
    let bytes = encode(Domain::Sim, &[]);
    let manifest = Manifest::decode(&bytes, Domain::Sim).unwrap();
    assert_eq!(manifest.entries().len(), 0);
    assert_eq!(
        manifest.typed_ref::<MotionProfileV1>(AssetRef::from_raw(1)),
        Err(AssetError::MissingAsset)
    );
}
