use super::*;
use crate::game::{self, Motion, Position};
use orr_fp::FP;

fn release() -> ReleaseBinding {
    ReleaseBinding::parse(include_bytes!("../fixtures/release.json")).unwrap()
}
fn with_bundle(f: impl FnOnce(BundleBytes<'_>)) {
    let objects = [
        ArtifactBytes {
            sha256: sha256(MOTION_BYTES),
            bytes: MOTION_BYTES,
        },
        ArtifactBytes {
            sha256: sha256(IMPACT_BYTES),
            bytes: IMPACT_BYTES,
        },
    ];
    f(BundleBytes {
        sim_manifest: SIM_BYTES,
        view_manifest: VIEW_BYTES,
        objects: &objects,
    });
}
fn reset_ticks() {
    game::TICK_CALLS.with(|n| n.set(0));
}
fn ticks() -> u64 {
    game::TICK_CALLS.with(|n| n.get())
}

#[test]
fn strict_release_roundtrip_uses_fields_not_json_identity() {
    let original = release();
    assert_eq!(original.game(), GAME_ID);
    assert_eq!(original.game_code_id(), GAME_CODE_ID);
    assert_eq!(original.build_id(), BUILD_ID);
    assert_eq!(BUILD_ID, 10434068928185386027);
    assert_ne!(orr_sim::build_hash_of(BUILD_ID, 0), 0);
    assert_eq!(
        ReleaseBinding::parse(&original.to_json().unwrap()).unwrap(),
        original
    );
    let value: serde_json::Value =
        serde_json::from_slice(include_bytes!("../fixtures/release.json")).unwrap();
    let reordered = serde_json::to_vec(&value).unwrap();
    assert_ne!(reordered, original.to_json().unwrap());
    assert_eq!(ReleaseBinding::parse(&reordered).unwrap(), original);
}

#[test]
fn strict_release_rejects_invalid_fields_and_noncanonical_spellings() {
    let base: serde_json::Value =
        serde_json::from_slice(include_bytes!("../fixtures/release.json")).unwrap();
    for (field, value) in [
        ("format", serde_json::json!("orr.deployment/1")),
        ("game", serde_json::json!("")),
        ("frame_format_version", serde_json::json!(3)),
        ("game_code_id", serde_json::json!(11193926291457u64)),
        ("game_code_id", serde_json::json!("0")),
        ("game_code_id", serde_json::json!("01")),
        ("game_code_id", serde_json::json!("+1")),
        ("game_code_id", serde_json::json!("1e2")),
        ("game_code_id", serde_json::json!("18446744073709551616")),
        ("build_id", serde_json::json!("0")),
        ("build_id", serde_json::json!("01")),
        ("build_id", serde_json::json!("1")),
        ("sim_manifest_sha256", serde_json::json!("A".repeat(64))),
        ("sim_manifest_sha256", serde_json::json!("0".repeat(63))),
        ("view_manifest_sha256", serde_json::json!("g".repeat(64))),
        ("unknown", serde_json::json!(true)),
    ] {
        let mut modified = base.clone();
        modified[field] = value;
        assert!(
            ReleaseBinding::parse(&serde_json::to_vec(&modified).unwrap()).is_err(),
            "{field}"
        );
    }
    for field in base.as_object().unwrap().keys() {
        let mut missing = base.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(
            ReleaseBinding::parse(&serde_json::to_vec(&missing).unwrap()).is_err(),
            "missing {field}"
        );
    }
    let text = String::from_utf8(release().to_json().unwrap()).unwrap();
    let duplicate = text.replacen('{', "{\"game\":\"asset_fixture_v1\",", 1);
    assert!(ReleaseBinding::parse(duplicate.as_bytes()).is_err());
    assert!(matches!(
        ReleaseBinding::parse(&vec![b' '; MAX_INPUT_BYTES + 1]),
        Err(Error::BudgetExceeded)
    ));
}

#[test]
fn release_index_rejects_reuse_but_allows_view_only_updates() {
    let mut index = ReleaseIndex::default();
    let original = release();
    index.admit(&original).unwrap();
    index.admit(&original).unwrap();
    let mut view_only = original.clone();
    view_only.view_digest = [9; 32];
    index.admit(&view_only).unwrap();
    let mut changed = original.clone();
    changed.sim_digest = [1; 32];
    assert!(matches!(index.admit(&changed), Err(Error::ReleaseIdReuse)));
    changed.game_code_id += 1;
    changed.build_id = orr_sim::frame_build_id(changed.game_code_id);
    index.admit(&changed).unwrap();
}

#[test]
fn preparation_binds_full_digest_identity_objects_and_static_reencoding_before_ticks() {
    reset_ticks();
    with_bundle(|bundle| {
        PreparedFixture::prepare(bundle, &release()).unwrap();
        for mutate in [
            |binding: &mut ReleaseBinding| binding.game = "another_game".into(),
            |binding: &mut ReleaseBinding| binding.game_code_id += 1,
            |binding: &mut ReleaseBinding| binding.build_id ^= 1,
            |binding: &mut ReleaseBinding| binding.sim_digest = [0; 32],
        ] {
            let mut bad = release();
            mutate(&mut bad);
            assert!(PreparedFixture::prepare(bundle, &bad).is_err());
        }
        let mut bad = release();
        bad.build_id = 0;
        assert!(PreparedFixture::prepare(bundle, &bad).is_err());
        assert!(validate_identity(&release(), 0, EXPECTED_SIM_SHA256).is_err());
        assert!(validate_identity(&release(), BUILD_ID, [0; 32]).is_err());
        let mut wrong_view = release();
        wrong_view.view_digest = [0; 32];
        assert!(matches!(
            PreparedFixture::prepare(bundle, &wrong_view),
            Err(Error::DigestMismatch)
        ));
        let missing = BundleBytes {
            objects: &bundle.objects[..1],
            ..bundle
        };
        assert!(matches!(
            PreparedFixture::prepare(missing, &release()),
            Err(Error::MissingArtifact)
        ));
        let mut corrupt = MOTION_BYTES.to_vec();
        corrupt[1] ^= 1;
        let mut objects = bundle.objects.to_vec();
        objects[0].bytes = &corrupt;
        assert!(matches!(
            PreparedFixture::prepare(
                BundleBytes {
                    objects: &objects,
                    ..bundle
                },
                &release()
            ),
            Err(Error::DigestMismatch)
        ));
        objects[0].bytes = &MOTION_BYTES[..7];
        assert!(matches!(
            PreparedFixture::prepare(
                BundleBytes {
                    objects: &objects,
                    ..bundle
                },
                &release()
            ),
            Err(Error::Asset(orr_asset::AssetError::LengthMismatch))
        ));
        let too_many = vec![bundle.objects[0]; 81];
        assert!(matches!(
            PreparedFixture::prepare(
                BundleBytes {
                    objects: &too_many,
                    ..bundle
                },
                &release()
            ),
            Err(Error::BudgetExceeded)
        ));
    });
    let manifest = Manifest::decode(SIM_BYTES, Domain::Sim).unwrap();
    validate_table(manifest, &generated::MOTION_PROFILES).unwrap();
    let changed = [(MOTION_ID, MotionProfileV1::new(FP::from_raw(8192)).unwrap())];
    assert!(matches!(
        validate_table(manifest, &changed),
        Err(Error::SimBindingMismatch)
    ));
    assert!(validate_table(manifest, &[]).is_err());
    assert_eq!(ticks(), 0);
}

#[test]
fn same_guid_new_sim_payload_is_refused_by_old_binary_before_tick_zero() {
    reset_ticks();
    let mut entry = Manifest::decode(SIM_BYTES, Domain::Sim)
        .unwrap()
        .find(MOTION_ID)
        .unwrap();
    let changed = MotionProfileV1::new(FP::from_raw(8192)).unwrap().encode();
    entry.payload_sha256 = sha256(&changed);
    let mut manifest = vec![0; SIM_BYTES.len()];
    orr_asset::encode_manifest(Domain::Sim, &[entry], &mut manifest).unwrap();
    assert_ne!(sha256(&manifest), EXPECTED_SIM_SHA256);
    let mut binding = release();
    binding.sim_digest = sha256(&manifest);
    let objects = [
        ArtifactBytes {
            sha256: sha256(&changed),
            bytes: &changed,
        },
        ArtifactBytes {
            sha256: sha256(IMPACT_BYTES),
            bytes: IMPACT_BYTES,
        },
    ];
    let bundle = BundleBytes {
        sim_manifest: &manifest,
        view_manifest: VIEW_BYTES,
        objects: &objects,
    };
    assert!(matches!(
        PreparedFixture::prepare(bundle, &binding),
        Err(Error::SimBindingMismatch)
    ));
    assert!(matches!(
        PreparedFixture::prepare(bundle, &release()),
        Err(Error::DigestMismatch)
    ));
    assert_eq!(ticks(), 0);
}

#[test]
fn frame_validation_rejects_missing_types_references_components_and_out_of_bounds_state() {
    reset_ticks();
    let empty = orr_ecs::Frame::new(orr_ecs::ComponentRegistryBuilder::new().build());
    assert!(matches!(
        game::validate_frame(&empty, 0),
        Err(Error::Frame("registry"))
    ));
    let initial = game::new_simulation();
    game::validate_frame(initial.frame(), 0).unwrap();
    let entity = initial.frame().entities().next().unwrap();
    for id in [
        orr_asset::AssetRef::NULL,
        orr_asset::AssetRef::from_raw(0xdead),
    ] {
        let mut frame = initial.frame().clone();
        frame.get_mut::<Motion>(entity).unwrap().profile = id;
        assert!(game::validate_frame(&frame, 0).is_err());
    }
    for raw in [-1, 120 * 4096 + 1, 1] {
        let mut frame = initial.frame().clone();
        frame.get_mut::<Position>(entity).unwrap().x = FP::from_raw(raw);
        assert!(game::validate_frame(&frame, 0).is_err());
    }
    let mut frame = initial.frame().clone();
    frame.remove::<Motion>(entity);
    assert!(game::validate_frame(&frame, 0).is_err());
    let mut frame = initial.frame().clone();
    frame.remove::<Position>(entity);
    assert!(game::validate_frame(&frame, 0).is_err());
    let mut frame = initial.frame().clone();
    frame.spawn();
    assert!(game::validate_frame(&frame, 0).is_err());
    assert!(game::validate_frame(initial.frame(), 1).is_err());
    assert_eq!(ticks(), 0);
}

#[test]
fn new_fixture_exact_baseline_schedule_cues_and_closed_replay_roundtrip() {
    let prepared = PreparedFixture::embedded().unwrap();
    let recording = prepared.record().unwrap();
    let golden: Vec<(u64, u64)> = include_str!("../fixtures/checksums.txt")
        .lines()
        .map(|line| {
            let (tick, checksum) = line.split_once(' ').unwrap();
            (
                tick.parse().unwrap(),
                u64::from_str_radix(checksum, 16).unwrap(),
            )
        })
        .collect();
    assert_eq!(golden.len(), 301);
    assert_eq!(
        recording
            .states
            .iter()
            .map(|s| (s.tick, s.checksum))
            .collect::<Vec<_>>(),
        golden
    );
    assert_eq!(recording.states[0].checksum, INITIAL_CHECKSUM);
    assert_eq!(recording.states[120].x, FP::from_raw(491520));
    assert_eq!(recording.states[180].x, FP::from_raw(491520));
    assert_eq!(recording.states[300].x, FP::ZERO);
    assert_eq!(
        recording
            .events
            .iter()
            .map(|e| (e.key.tick, e.key.system_index, e.key.seq, e.impact.cue))
            .collect::<Vec<_>>(),
        [(1, 0, 0, 1), (181, 0, 0, 1)]
    );
    assert_eq!(
        recording.replay,
        include_bytes!("../fixtures/baseline.orrp")
    );
    assert_eq!(recording.replay, prepared.record().unwrap().replay);
    assert_eq!(sha256(&recording.replay), EXPECTED_REPLAY_SHA256);
    assert_eq!(&recording.replay[..8], b"ORRP\x03\x00\x00\x00");
    let report = prepared.verify(&recording.replay).unwrap();
    assert!(report.ok());
    assert_eq!(
        (report.ticks_simulated, report.checksums_checked),
        (300, 300)
    );
    for tick in [
        0, 1, 59, 60, 61, 119, 120, 121, 179, 180, 181, 239, 240, 241, 299, 300,
    ] {
        assert_eq!(
            prepared.seek(&recording.replay, tick).unwrap(),
            recording.states[tick as usize]
        );
    }
    assert!(matches!(
        prepared.seek(&recording.replay, 301),
        Err(Error::TickOutOfRange(301))
    ));
    let mut run = prepared.start();
    for _ in 1..=TICKS {
        run.advance().unwrap();
    }
    let before = run.state();
    assert!(run.advance().is_err());
    assert_eq!(run.state(), before);
}

#[test]
fn rollback_resimulation_preserves_every_checksum_and_event() {
    let prepared = PreparedFixture::embedded().unwrap();
    let baseline = prepared.record().unwrap();
    let mut run = prepared.start();
    let mut snapshots = Vec::new();
    snapshots.push(run.sim.frame().clone());
    for tick in 1..=300 {
        run.advance().unwrap();
        if tick == 60 || tick == 180 {
            snapshots.push(run.sim.frame().clone());
        }
    }
    for snapshot in snapshots {
        game::validate_frame(&snapshot, snapshot.tick()).unwrap();
        run.sim.restore(&snapshot);
        let mut events = Vec::new();
        for tick in snapshot.tick() + 1..=300 {
            let step = run.advance().unwrap();
            assert_eq!(step.state, baseline.states[tick as usize]);
            events.extend(step.events);
        }
        assert_eq!(
            events,
            baseline
                .events
                .iter()
                .filter(|e| e.key.tick > snapshot.tick())
                .copied()
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn view_package_validates_root_guid_uniqueness_pcm_headers_and_objects() {
    reset_ticks();
    with_bundle(|bundle| {
        let original = Manifest::decode(VIEW_BYTES, Domain::View)
            .unwrap()
            .find(IMPACT_ID)
            .unwrap();
        for entries in [
            vec![],
            vec![ManifestEntry {
                id: MOTION_ID,
                ..original
            }],
            vec![
                ManifestEntry {
                    id: MOTION_ID,
                    ..original
                },
                original,
            ],
        ] {
            let mut view_bytes =
                vec![0; orr_asset::manifest_encoded_len(Domain::View, entries.len()).unwrap()];
            orr_asset::encode_manifest(Domain::View, &entries, &mut view_bytes).unwrap();
            let mut binding = release();
            binding.view_digest = sha256(&view_bytes);
            assert!(PreparedFixture::prepare(
                BundleBytes {
                    view_manifest: &view_bytes,
                    ..bundle
                },
                &binding
            )
            .is_err());
        }
        for (offset, value) in [(0, 44100u32), (4, 0), (4, 48001), (4, 7199), (4, 7201)] {
            let mut payload = IMPACT_BYTES.to_vec();
            payload[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            let entry = ManifestEntry {
                payload_sha256: sha256(&payload),
                ..original
            };
            let mut manifest = vec![0; VIEW_BYTES.len()];
            orr_asset::encode_manifest(Domain::View, &[entry], &mut manifest).unwrap();
            let objects = [
                bundle.objects[0],
                ArtifactBytes {
                    sha256: sha256(&payload),
                    bytes: &payload,
                },
            ];
            let mut binding = release();
            binding.view_digest = sha256(&manifest);
            assert!(matches!(
                PreparedFixture::prepare(
                    BundleBytes {
                        view_manifest: &manifest,
                        objects: &objects,
                        ..bundle
                    },
                    &binding
                ),
                Err(Error::Binding(
                    "PCM sample rate, frame count or exact length"
                ))
            ));
        }
        let mut duplicate = bundle.objects.to_vec();
        duplicate.push(bundle.objects[0]);
        assert!(matches!(
            PreparedFixture::prepare(
                BundleBytes {
                    objects: &duplicate,
                    ..bundle
                },
                &release()
            ),
            Err(Error::Binding("ambiguous object digest"))
        ));
        let mut unknown = bundle.objects.to_vec();
        unknown.push(ArtifactBytes {
            sha256: [1; 32],
            bytes: b"unused",
        });
        assert!(matches!(
            PreparedFixture::prepare(
                BundleBytes {
                    objects: &unknown,
                    ..bundle
                },
                &release()
            ),
            Err(Error::Binding("unreferenced object"))
        ));
    });
    assert_eq!(ticks(), 0);
}

#[test]
fn valid_view_only_change_keeps_sim_release_and_replay_identity() {
    with_bundle(|bundle| {
        let mut payload = IMPACT_BYTES.to_vec();
        payload[8] ^= 1;
        let mut entry = Manifest::decode(VIEW_BYTES, Domain::View)
            .unwrap()
            .find(IMPACT_ID)
            .unwrap();
        entry.payload_sha256 = sha256(&payload);
        let mut view = vec![0; VIEW_BYTES.len()];
        orr_asset::encode_manifest(Domain::View, &[entry], &mut view).unwrap();
        let mut binding = release();
        binding.view_digest = sha256(&view);
        let objects = [
            bundle.objects[0],
            ArtifactBytes {
                sha256: sha256(&payload),
                bytes: &payload,
            },
        ];
        let prepared = PreparedFixture::prepare(
            BundleBytes {
                view_manifest: &view,
                objects: &objects,
                ..bundle
            },
            &binding,
        )
        .unwrap();
        assert_eq!(prepared.release().build_id(), BUILD_ID);
        assert_eq!(
            prepared.release().sim_manifest_sha256(),
            EXPECTED_SIM_SHA256
        );
        assert_ne!(
            prepared.release().view_manifest_sha256(),
            release().view_manifest_sha256()
        );
        assert_eq!(
            prepared.record().unwrap().replay,
            include_bytes!("../fixtures/baseline.orrp")
        );
        prepared
            .verify(include_bytes!("../fixtures/baseline.orrp"))
            .unwrap();
    });
}
