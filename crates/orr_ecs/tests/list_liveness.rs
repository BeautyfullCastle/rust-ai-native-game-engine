use orr_ecs::{ComponentRegistryBuilder, Frame, FrameList};

#[test]
fn live_empty_lists_are_distinct_from_stale_or_unregistered_handles() {
    let mut registry = ComponentRegistryBuilder::new();
    registry.register_list::<u32>("u32");
    let mut frame = Frame::new(registry.build());
    assert!(!frame.list_is_alive(FrameList::<u64>::NONE));
    assert!(!frame.list_is_alive(FrameList::<u32>::NONE));
    let old = frame.alloc_list::<u32>();
    assert!(frame.list_is_alive(old));
    assert!(frame.list(old).is_empty());
    frame.list_free(old);
    assert!(!frame.list_is_alive(old));
    let new = frame.alloc_list::<u32>();
    assert_eq!(old.index, new.index);
    assert_ne!(old.version, new.version);
    assert!(frame.list_is_alive(new));
    assert!(!frame.list_is_alive(old));
    let restored = Frame::from_bytes(frame.registry().clone(), &frame.to_bytes()).unwrap();
    assert!(restored.list_is_alive(new));
    assert!(!restored.list_is_alive(old));
}
