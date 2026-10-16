use bytemuck::Zeroable;
use orr_ecs::{ComponentRegistryBuilder, Entity, Frame, FrameList};
use orr_fp::{fp, FPVec3, FP};
use orr_physics3d::{Body, Collider, Shape};
use orr_terrain::Terrain;
use orr_terrain_physics3d::asset::{
    admit_heightfield, reconstruct_terrain, register, terrain_view, AssetError,
    HeightfieldCollider, TerrainAsset, TerrainHeight, TerrainHole, TerrainIdentityByte,
};

fn fixture() -> Terrain {
    Terrain::new(
        "fixtures/physics/terrain.thf".into(),
        3,
        3,
        [fp!(-2), fp!(-2)],
        fp!(2),
        [0, 2, 0, 4, 8, -2, 0, 2, 0].map(FP::from_int).to_vec(),
        vec![false, true, false, false],
    )
    .unwrap()
}

fn frame() -> Frame {
    let mut registry = ComponentRegistryBuilder::new();
    orr_physics3d::register(&mut registry);
    register(&mut registry);
    Frame::new(registry.build())
}

fn material(revision: [u8; 32]) -> HeightfieldCollider {
    HeightfieldCollider {
        revision,
        friction: fp!(0.5),
        restitution: fp!(0.25),
        layer: 4,
        mask: 3,
    }
}

fn admitted() -> (Frame, Entity, Terrain) {
    let mut frame = frame();
    let terrain = fixture();
    let entity = admit_heightfield(
        &mut frame,
        &terrain.cook(),
        terrain.revision(),
        material(terrain.revision()),
    )
    .unwrap();
    (frame, entity, terrain)
}

#[test]
fn canonical_content_and_material_are_frame_owned() {
    let (frame, entity, expected) = admitted();
    assert!(frame.exists(entity));
    assert!(!frame.has::<Body>(entity));
    assert!(!frame.has::<Collider>(entity));
    let view = terrain_view(&frame).unwrap().unwrap();
    assert_eq!(view.asset.entity, entity);
    assert_eq!(view.asset.revision, expected.revision());
    assert_eq!(*view.collider, material(expected.revision()));
    assert_eq!(view.asset.width, expected.width());
    assert_eq!(view.asset.depth, expected.depth());
    assert_eq!(view.asset.origin, expected.origin());
    assert_eq!(view.asset.spacing, expected.spacing());
    assert_eq!(
        view.heights.iter().map(|h| h.0).collect::<Vec<_>>(),
        expected.heights()
    );
    assert_eq!(
        view.holes.iter().map(|h| h.0 != 0).collect::<Vec<_>>(),
        expected.holes()
    );
    assert_eq!(
        view.identity.iter().map(|b| b.0).collect::<Vec<_>>(),
        expected.asset_id().as_bytes()
    );
    for index in 0..expected.width() * expected.depth() {
        assert_eq!(view.vertex_position(index), expected.vertex_position(index));
    }
    assert_eq!(
        view.vertex_position(expected.width() * expected.depth()),
        None
    );
    assert_eq!(reconstruct_terrain(&frame).unwrap(), Some(expected));
}

#[test]
fn snapshots_restore_every_asset_byte_without_external_assets() {
    let (frame, entity, expected) = admitted();
    let baseline = frame.to_bytes();
    let checksum = frame.checksum();
    let mut copy = frame.clone();
    let asset = *copy.singleton::<TerrainAsset>();
    copy.list_mut(asset.heights)[0].0 = fp!(7);
    copy.list_mut(asset.holes)[0].0 = 1;
    copy.list_mut(asset.identity)[0].0 = b'x';
    copy.get_mut::<HeightfieldCollider>(entity)
        .unwrap()
        .friction = fp!(2);
    copy.singleton_mut::<TerrainAsset>().revision[31] ^= 1;
    assert_ne!(copy.checksum(), checksum);
    assert!(terrain_view(&copy).is_err());
    copy.copy_from(&frame);
    assert_eq!(copy.to_bytes(), baseline);
    assert_eq!(copy.checksum(), checksum);
    assert_eq!(reconstruct_terrain(&copy).unwrap(), Some(expected.clone()));
    let restored = Frame::from_bytes(frame.registry().clone(), &baseline).unwrap();
    drop(frame);
    assert_eq!(restored.checksum(), checksum);
    assert_eq!(restored.to_bytes(), baseline);
    assert_eq!(reconstruct_terrain(&restored).unwrap(), Some(expected));
}

#[test]
fn every_authoritative_field_changes_frame_checksum() {
    let (frame, entity, _) = admitted();
    let base = frame.checksum();
    let mutations: &[fn(&mut Frame, Entity)] = &[
        |f, _| f.singleton_mut::<TerrainAsset>().revision[31] ^= 1,
        |f, _| f.singleton_mut::<TerrainAsset>().origin[0] += FP::from_raw(1),
        |f, _| f.singleton_mut::<TerrainAsset>().spacing += FP::from_raw(1),
        |f, _| f.singleton_mut::<TerrainAsset>().width += 1,
        |f, _| f.singleton_mut::<TerrainAsset>().depth += 1,
        |f, _| f.singleton_mut::<TerrainAsset>().entity = Entity::NONE,
        |f, _| {
            let h = f.singleton::<TerrainAsset>().heights;
            f.list_mut(h)[0].0 += FP::from_raw(1);
        },
        |f, _| {
            let h = f.singleton::<TerrainAsset>().holes;
            f.list_mut(h)[0].0 ^= 1;
        },
        |f, _| {
            let h = f.singleton::<TerrainAsset>().identity;
            f.list_mut(h)[0].0 = b'z';
        },
        |f, e| f.get_mut::<HeightfieldCollider>(e).unwrap().revision[31] ^= 1,
        |f, e| f.get_mut::<HeightfieldCollider>(e).unwrap().friction += FP::from_raw(1),
        |f, e| f.get_mut::<HeightfieldCollider>(e).unwrap().restitution += FP::from_raw(1),
        |f, e| f.get_mut::<HeightfieldCollider>(e).unwrap().layer ^= 1,
        |f, e| f.get_mut::<HeightfieldCollider>(e).unwrap().mask ^= 1,
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut changed = frame.clone();
        mutate(&mut changed, entity);
        assert_ne!(changed.checksum(), base, "mutation {index}");
    }
}

#[test]
fn admission_is_atomic_for_every_revision_byte() {
    let terrain = fixture();
    let bytes = terrain.cook();
    for index in 0..32 {
        let mut frame = frame();
        let before = frame.to_bytes();
        let mut wrong = terrain.revision();
        wrong[index] ^= 1;
        assert_eq!(
            admit_heightfield(&mut frame, &bytes, wrong, material(wrong)),
            Err(AssetError::RevisionMismatch),
        );
        assert_eq!(frame.to_bytes(), before, "expected revision byte {index}");
        assert_eq!(
            admit_heightfield(&mut frame, &bytes, terrain.revision(), material(wrong)),
            Err(AssetError::RevisionMismatch),
        );
        assert_eq!(frame.to_bytes(), before, "collider revision byte {index}");
    }
}

#[test]
fn malformed_canonical_inputs_never_allocate_or_publish() {
    let terrain = fixture();
    let bytes = terrain.cook();
    let mut malformed: Vec<Vec<u8>> = (0..bytes.len()).map(|n| bytes[..n].to_vec()).collect();
    let mut trailing = bytes.clone();
    trailing.push(0);
    malformed.push(trailing);
    let mut noncanonical_hole = bytes.clone();
    *noncanonical_hole.last_mut().unwrap() = 2;
    malformed.push(noncanonical_hole);
    let mut bad_magic = bytes.clone();
    bad_magic[0] ^= 1;
    malformed.push(bad_magic);
    for bad in malformed {
        let mut frame = frame();
        let before = frame.to_bytes();
        assert_eq!(
            admit_heightfield(
                &mut frame,
                &bad,
                terrain.revision(),
                material(terrain.revision())
            ),
            Err(AssetError::InvalidCanonicalBytes),
        );
        assert_eq!(frame.to_bytes(), before);
    }
}

fn terrain_with(width: u32, depth: u32, origin: [FP; 2], spacing: FP, height: FP) -> Terrain {
    Terrain::new(
        "profile.thf".into(),
        width,
        depth,
        origin,
        spacing,
        vec![height; (width * depth) as usize],
        vec![false; ((width - 1) * (depth - 1)) as usize],
    )
    .unwrap()
}

#[test]
fn collision_profile_rejects_wider_asset_domain_atomically() {
    let outside = [
        terrain_with(66, 2, [FP::ZERO; 2], FP::ONE, FP::ZERO),
        terrain_with(2, 66, [FP::ZERO; 2], FP::ONE, FP::ZERO),
        terrain_with(
            2,
            2,
            [fp!(-1024) - FP::from_raw(1), FP::ZERO],
            FP::ONE,
            FP::ZERO,
        ),
        terrain_with(2, 2, [fp!(1024), FP::ZERO], FP::ONE, FP::ZERO),
        terrain_with(2, 2, [FP::ZERO, fp!(1024)], FP::ONE, FP::ZERO),
        terrain_with(2, 2, [FP::ZERO; 2], fp!(0.25) - FP::from_raw(1), FP::ZERO),
        terrain_with(2, 2, [FP::ZERO; 2], fp!(16) + FP::from_raw(1), FP::ZERO),
        terrain_with(2, 2, [FP::ZERO; 2], FP::ONE, fp!(1024) + FP::from_raw(1)),
        terrain_with(2, 2, [FP::ZERO; 2], FP::ONE, fp!(-1024) - FP::from_raw(1)),
        terrain_with(2, 2, [FP::ZERO; 2], FP::ONE, FP::from_raw(i64::MIN)),
        Terrain::new(
            "steep.thf".into(),
            2,
            2,
            [FP::ZERO; 2],
            FP::ONE,
            vec![FP::ZERO, fp!(16) + FP::from_raw(1), FP::ZERO, FP::ZERO],
            vec![false],
        )
        .unwrap(),
    ];
    for terrain in outside {
        let mut frame = frame();
        let before = frame.to_bytes();
        assert_eq!(
            admit_heightfield(
                &mut frame,
                &terrain.cook(),
                terrain.revision(),
                material(terrain.revision())
            ),
            Err(AssetError::OutsideCollisionProfile),
        );
        assert_eq!(frame.to_bytes(), before);
    }
}

#[test]
fn inclusive_collision_profile_edges_are_admitted() {
    let edges = [
        terrain_with(65, 65, [fp!(-1024); 2], fp!(16), fp!(-1024)),
        terrain_with(65, 65, [FP::ZERO; 2], fp!(16), fp!(1024)),
        terrain_with(2, 2, [FP::ZERO; 2], fp!(0.25), FP::ZERO),
        Terrain::new(
            "steep.thf".into(),
            2,
            2,
            [FP::ZERO; 2],
            fp!(0.25),
            vec![FP::ZERO, fp!(16), fp!(16), FP::ZERO],
            vec![false],
        )
        .unwrap(),
    ];
    for terrain in edges {
        let mut frame = frame();
        admit_heightfield(
            &mut frame,
            &terrain.cook(),
            terrain.revision(),
            material(terrain.revision()),
        )
        .unwrap();
        assert_eq!(reconstruct_terrain(&frame).unwrap(), Some(terrain));
    }
}

#[test]
fn material_rejection_and_second_admission_are_atomic() {
    let terrain = fixture();
    for (friction, restitution) in [
        (FP::from_raw(-1), FP::ZERO),
        (fp!(100) + FP::from_raw(1), FP::ZERO),
        (FP::ZERO, FP::from_raw(-1)),
        (FP::ZERO, FP::ONE + FP::from_raw(1)),
    ] {
        let mut frame = frame();
        let before = frame.to_bytes();
        let mut collider = material(terrain.revision());
        collider.friction = friction;
        collider.restitution = restitution;
        assert_eq!(
            admit_heightfield(&mut frame, &terrain.cook(), terrain.revision(), collider),
            Err(AssetError::InvalidMaterial)
        );
        assert_eq!(frame.to_bytes(), before);
    }
    let (mut frame, _, terrain) = admitted();
    let before = frame.to_bytes();
    assert_eq!(
        admit_heightfield(
            &mut frame,
            &terrain.cook(),
            terrain.revision(),
            material(terrain.revision())
        ),
        Err(AssetError::AlreadyAdmitted),
    );
    assert_eq!(frame.to_bytes(), before);
}

#[test]
fn runtime_rejects_corrupt_lists_revisions_and_metadata_without_mutation() {
    let (frame, entity, _) = admitted();
    let mutations: &[fn(&mut Frame, Entity)] = &[
        |f, _| f.singleton_mut::<TerrainAsset>().present = 2,
        |f, _| f.singleton_mut::<TerrainAsset>()._pad = 1,
        |f, _| f.singleton_mut::<TerrainAsset>().width = u32::MAX,
        |f, _| f.singleton_mut::<TerrainAsset>().depth = 1,
        |f, _| f.singleton_mut::<TerrainAsset>().spacing = FP::from_raw(i64::MAX),
        |f, _| f.singleton_mut::<TerrainAsset>().revision[31] ^= 1,
        |f, _| f.singleton_mut::<TerrainAsset>().heights = FrameList::<TerrainHeight>::NONE,
        |f, _| f.singleton_mut::<TerrainAsset>().holes = FrameList::<TerrainHole>::NONE,
        |f, _| f.singleton_mut::<TerrainAsset>().identity = FrameList::<TerrainIdentityByte>::NONE,
        |f, _| {
            let h = f.singleton::<TerrainAsset>().heights;
            f.list_mut(h)[0].0 += FP::from_raw(1);
        },
        |f, _| {
            let h = f.singleton::<TerrainAsset>().holes;
            f.list_mut(h)[0].0 = 2;
        },
        |f, _| {
            let h = f.singleton::<TerrainAsset>().identity;
            f.list_mut(h)[0].0 = b'/';
        },
        |f, e| f.get_mut::<HeightfieldCollider>(e).unwrap().revision[31] ^= 1,
        |f, e| f.get_mut::<HeightfieldCollider>(e).unwrap().friction = FP::from_raw(-1),
        |f, e| {
            f.remove::<HeightfieldCollider>(e);
        },
        |f, e| {
            f.despawn(e);
        },
        |f, e| {
            let collider = *f.get::<HeightfieldCollider>(e).unwrap();
            let extra = f.spawn();
            f.add(extra, collider);
        },
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut changed = frame.clone();
        mutate(&mut changed, entity);
        let before = changed.to_bytes();
        assert!(terrain_view(&changed).is_err(), "mutation {index}");
        assert!(reconstruct_terrain(&changed).is_err(), "mutation {index}");
        assert_eq!(changed.to_bytes(), before);
        // ORRF is structurally valid, but semantic terrain admission must still
        // reject the same bad Pod state after a byte-level snapshot restore.
        let restored = Frame::from_bytes(changed.registry().clone(), &before).unwrap();
        assert!(
            terrain_view(&restored).is_err(),
            "restored mutation {index}"
        );
    }
}

#[test]
fn transformed_or_convex_terrain_proxy_is_rejected() {
    let (frame, entity, _) = admitted();
    let mut body = frame.clone();
    body.add(entity, Body::new_static(FPVec3::ZERO));
    assert_eq!(
        terrain_view(&body).unwrap_err(),
        AssetError::AmbiguousTerrainBody
    );
    let mut collider = frame.clone();
    collider.add(
        entity,
        Collider {
            shape: Shape::sphere(FP::ONE),
            ..Collider::zeroed()
        },
    );
    assert_eq!(
        terrain_view(&collider).unwrap_err(),
        AssetError::AmbiguousTerrainBody
    );
}

#[test]
fn empty_and_unregistered_frames_are_distinguished() {
    let empty = frame();
    assert!(terrain_view(&empty).unwrap().is_none());
    assert!(reconstruct_terrain(&empty).unwrap().is_none());
    let mut unregistered = Frame::new(ComponentRegistryBuilder::new().build());
    let before = unregistered.to_bytes();
    assert_eq!(
        terrain_view(&unregistered).unwrap_err(),
        AssetError::Unregistered
    );
    let terrain = fixture();
    assert_eq!(
        admit_heightfield(
            &mut unregistered,
            &terrain.cook(),
            terrain.revision(),
            material(terrain.revision())
        ),
        Err(AssetError::Unregistered)
    );
    assert_eq!(unregistered.to_bytes(), before);
}

#[test]
fn identity_is_part_of_the_pin_even_when_geometry_is_identical() {
    let terrain = fixture();
    let renamed = Terrain::new(
        "renamed.thf".into(),
        terrain.width(),
        terrain.depth(),
        terrain.origin(),
        terrain.spacing(),
        terrain.heights().to_vec(),
        terrain.holes().to_vec(),
    )
    .unwrap();
    assert_ne!(terrain.revision(), renamed.revision());
    let mut frame = frame();
    let before = frame.to_bytes();
    assert_eq!(
        admit_heightfield(
            &mut frame,
            &renamed.cook(),
            terrain.revision(),
            material(terrain.revision())
        ),
        Err(AssetError::RevisionMismatch),
    );
    assert_eq!(frame.to_bytes(), before);
}
