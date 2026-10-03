use orr_ecs::{ComponentRegistryBuilder, Frame, FrameDecodeError, FRAME_FORMAT_VERSION};
use xxhash_rust::xxh3::xxh3_64;

fn assert_body_checksum(frame: &Frame) {
    let bytes = frame.to_bytes();
    let (body, trailer) = bytes.split_at(bytes.len() - 8);
    assert_eq!(frame.checksum(), xxh3_64(body));
    assert_eq!(trailer, frame.checksum().to_le_bytes());
}

#[test]
fn checksum_is_the_hash_of_every_encoded_body_byte() {
    let mut registry = ComponentRegistryBuilder::new();
    registry.register_component::<u32>("A");
    registry.register_component::<()>("Marker");
    registry.register_singleton::<u64>("Clock");
    registry.register_list::<u8>("Bytes");
    registry.register_list::<()>("Units");
    let registry = registry.build();
    let mut frame = Frame::new(registry.clone());
    assert_body_checksum(&frame);
    let dead = frame.spawn();
    let live = frame.spawn();
    frame.add(live, 17u32);
    frame.add(dead, ());
    frame.despawn(dead);
    frame.set_singleton(99u64);
    let freed = frame.alloc_list::<u8>();
    let kept = frame.alloc_list::<u8>();
    frame.list_push(kept, 255);
    frame.list_free(freed);
    let units = frame.alloc_list::<()>();
    for _ in 0..100 {
        frame.list_push(units, ());
    }
    frame.set_tick(41);
    assert_body_checksum(&frame);
    assert_body_checksum(&frame.clone());
    let restored = Frame::from_bytes(registry, &frame.to_bytes()).unwrap();
    assert_body_checksum(&restored);
    assert_eq!(restored.checksum(), frame.checksum());
}

#[test]
fn identical_payload_in_different_component_stores_has_different_checksum() {
    let mut registry = ComponentRegistryBuilder::new();
    registry.register_component::<u32>("A");
    registry.register_component::<i32>("B");
    let mut a = Frame::new(registry.build());
    let entity = a.spawn();
    let mut b = a.clone();
    a.add(entity, 7u32);
    b.add(entity, 7i32);
    assert_eq!(a.count::<u32>(), 1);
    assert_eq!(b.count::<u32>(), 0);
    assert_ne!(a.checksum(), b.checksum());
}

#[test]
fn schema_is_part_of_checksum_even_when_payloads_match() {
    let mut a = ComponentRegistryBuilder::new();
    a.register_singleton::<u32>("A");
    let mut b = ComponentRegistryBuilder::new();
    b.register_singleton::<u32>("B");
    assert_ne!(Frame::new(a.build()).checksum(), Frame::new(b.build()).checksum());
    let mut c = ComponentRegistryBuilder::new();
    c.register_component::<u32>("A");
    let mut d = ComponentRegistryBuilder::new();
    d.register_list::<u32>("A");
    assert_ne!(Frame::new(c.build()).checksum(), Frame::new(d.build()).checksum());
}

#[test]
fn list_free_order_changes_checksum_and_survives_restore() {
    let mut registry = ComponentRegistryBuilder::new();
    registry.register_list::<u32>("Numbers");
    let registry = registry.build();
    let mut a = Frame::new(registry.clone());
    let first = a.alloc_list::<u32>();
    let second = a.alloc_list::<u32>();
    let mut b = a.clone();
    a.list_free(first);
    a.list_free(second);
    b.list_free(second);
    b.list_free(first);
    assert_ne!(a.checksum(), b.checksum());
    let mut tampered = a.to_bytes();
    let free_offset = tampered.len() - 16;
    tampered[free_offset..free_offset + 4].copy_from_slice(&second.index.to_le_bytes());
    tampered[free_offset + 4..free_offset + 8].copy_from_slice(&first.index.to_le_bytes());
    assert!(matches!(
        Frame::from_bytes(registry.clone(), &tampered),
        Err(FrameDecodeError::ChecksumMismatch { .. })
    ));
    let mut snapshot = a.clone();
    let mut restored = Frame::from_bytes(registry, &a.to_bytes()).unwrap();
    let next = a.alloc_list::<u32>();
    assert_eq!(next, snapshot.alloc_list::<u32>());
    assert_eq!(next, restored.alloc_list::<u32>());
    assert_eq!(next.index, second.index);
    assert_eq!(b.alloc_list::<u32>().index, first.index);
    assert_eq!(a.checksum(), snapshot.checksum());
    assert_eq!(a.checksum(), restored.checksum());
}

#[test]
fn list_item_boundaries_are_part_of_checksum() {
    let mut registry = ComponentRegistryBuilder::new();
    registry.register_list::<u8>("Bytes");
    let mut a = Frame::new(registry.build());
    let first = a.alloc_list::<u8>();
    let second = a.alloc_list::<u8>();
    let mut b = a.clone();
    // This payload is exactly the old unframed encoding of a live slot's
    // version and alive flag. Moving it between lists used to hash equally.
    for byte in [1, 0, 0, 0, 1] {
        a.list_push(first, byte);
        b.list_push(second, byte);
    }
    assert_ne!(a.checksum(), b.checksum());
}

#[test]
fn list_pool_boundaries_are_part_of_checksum() {
    let mut registry = ComponentRegistryBuilder::new();
    registry.register_list::<u32>("A");
    registry.register_list::<i32>("B");
    let mut a = Frame::new(registry.build());
    let mut b = a.clone();
    a.alloc_list::<u32>();
    b.alloc_list::<i32>();
    assert_ne!(a.checksum(), b.checksum());
}

#[test]
fn zero_sized_list_lengths_round_trip_and_change_checksum() {
    let mut registry = ComponentRegistryBuilder::new();
    registry.register_list::<()>("Units");
    let registry = registry.build();
    let mut frame = Frame::new(registry.clone());
    let list = frame.alloc_list::<()>();
    let empty_checksum = frame.checksum();
    for _ in 0..100 {
        frame.list_push(list, ());
    }
    assert_ne!(frame.checksum(), empty_checksum);
    let bytes = frame.to_bytes();
    // The count is greater than the bytes remaining at its position.
    let mut restored = Frame::from_bytes(registry, &bytes).unwrap();
    assert_eq!(restored.list(list).len(), 100);
    assert_eq!(restored.to_bytes(), bytes);
    assert_body_checksum(&restored);
    restored.list_clear(list);
    assert_eq!(restored.checksum(), empty_checksum);
}

#[test]
fn maximum_zero_sized_count_needs_no_payload_allocation_or_count_sized_loop() {
    let mut registry = ComponentRegistryBuilder::new();
    registry.register_list::<()>("Units");
    let registry = registry.build();
    let mut frame = Frame::new(registry.clone());
    let list = frame.alloc_list::<()>();
    let mut bytes = frame.to_bytes();
    // The last slot's count precedes the zero-length free list and trailer.
    let count_offset = bytes.len() - 16;
    bytes[count_offset..count_offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(
        Frame::from_bytes(registry.clone(), &bytes),
        Err(FrameDecodeError::ChecksumMismatch { .. })
    ));
    // A correctly checksummed large ZST list is valid: it occupies no bytes.
    let body_len = bytes.len() - 8;
    let checksum = xxh3_64(&bytes[..body_len]);
    bytes[body_len..].copy_from_slice(&checksum.to_le_bytes());
    let restored = Frame::from_bytes(registry, &bytes).unwrap();
    assert_eq!(restored.list(list).len(), u32::MAX as usize);
    assert_eq!(restored.to_bytes(), bytes);
}

#[test]
fn legacy_frame_version_is_rejected_explicitly() {
    let registry = ComponentRegistryBuilder::new().build();
    let mut bytes = Frame::new(registry.clone()).to_bytes();
    assert_eq!(&bytes[4..8], &FRAME_FORMAT_VERSION.to_le_bytes());
    assert_eq!(&bytes[4..8], &2u32.to_le_bytes());
    bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
    assert_eq!(Frame::from_bytes(registry, &bytes).err(), Some(FrameDecodeError::UnsupportedVersion(1)));
}
