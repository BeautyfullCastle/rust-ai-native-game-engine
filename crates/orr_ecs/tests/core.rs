mod common;
use common::*;
use orr_ecs::{Commands, Frame};

#[test]
fn spawn_despawn_version_reuse() {
    let reg = test_registry();
    let mut f = Frame::new(reg);
    let e0 = f.spawn();
    assert_eq!(e0.index, 0);
    assert_eq!(e0.version, 0);
    assert!(f.exists(e0));
    assert!(f.despawn(e0));
    assert!(!f.exists(e0));
    let e1 = f.spawn();
    assert_eq!(e1.index, 0);
    assert_eq!(e1.version, 1, "respawned slot must bump version");
    assert!(f.exists(e1));
    assert_ne!(e0, e1);
}

#[test]
fn stale_entity_detected() {
    let reg = test_registry();
    let mut f = Frame::new(reg);
    let e0 = f.spawn();
    f.add(e0, Pos { x: 1, y: 2 });
    f.despawn(e0);
    let e1 = f.spawn();
    assert_ne!(e0, e1);
    assert!(!f.exists(e0));
    assert!(f.get::<Pos>(e0).is_none(), "stale handle must not see new occupant's data");
    assert!(f.get::<Pos>(e1).is_none());
    // second despawn of stale handle is a no-op, not a panic
    assert!(!f.despawn(e0));
}

#[test]
fn add_remove_get_has_count() {
    let reg = test_registry();
    let mut f = Frame::new(reg);
    let e = f.spawn();
    assert!(!f.has::<Pos>(e));
    assert_eq!(f.count::<Pos>(), 0);
    let old = f.add(e, Pos { x: 3, y: 4 });
    assert!(old.is_none());
    assert!(f.has::<Pos>(e));
    assert_eq!(f.count::<Pos>(), 1);
    assert_eq!(f.get::<Pos>(e), Some(&Pos { x: 3, y: 4 }));
    f.get_mut::<Pos>(e).unwrap().x = 10;
    assert_eq!(f.get::<Pos>(e).unwrap().x, 10);
    let removed = f.remove::<Pos>(e);
    assert_eq!(removed, Some(Pos { x: 10, y: 4 }));
    assert!(!f.has::<Pos>(e));
    assert_eq!(f.count::<Pos>(), 0);
}

#[test]
fn despawn_clears_all_components() {
    let reg = test_registry();
    let mut f = Frame::new(reg);
    let e = f.spawn();
    f.add(e, Pos { x: 1, y: 1 });
    f.add(e, Vel { x: 2, y: 2 });
    f.despawn(e);
    assert_eq!(f.count::<Pos>(), 0);
    assert_eq!(f.count::<Vel>(), 0);
}

#[test]
fn singletons() {
    let reg = test_registry();
    let mut f = Frame::new(reg);
    assert_eq!(*f.singleton::<GameConfig>(), GameConfig { gravity: 0, max_players: 0 });
    f.set_singleton(GameConfig { gravity: -10, max_players: 4 });
    assert_eq!(f.singleton::<GameConfig>().gravity, -10);
    f.singleton_mut::<GameConfig>().max_players = 8;
    assert_eq!(f.singleton::<GameConfig>().max_players, 8);
}

#[test]
fn frame_lists() {
    let reg = test_registry();
    let mut f = Frame::new(reg);
    let h = f.alloc_list::<Score>();
    assert!(f.list(h).is_empty());
    f.list_push(h, Score { value: 1 });
    f.list_push(h, Score { value: 2 });
    assert_eq!(f.list(h).len(), 2);
    assert_eq!(f.list(h)[1].value, 2);
    f.list_clear(h);
    assert!(f.list(h).is_empty());
    f.list_free(h);
    // stale handle now returns empty rather than panicking
    assert!(f.list(h).is_empty());
    let h2 = f.alloc_list::<Score>();
    assert_eq!(h2.index, h.index, "freed slot index should be recycled");
    assert_ne!(h2.version, h.version, "recycled slot must bump version");
}

#[test]
fn query_1_to_4_components() {
    let reg = test_registry();
    let mut f = Frame::new(reg);
    let e1 = f.spawn();
    f.add(e1, Pos { x: 1, y: 0 });
    f.add(e1, Vel { x: 1, y: 0 });
    f.add(e1, Health { hp: 10, max: 10 });
    f.add(e1, Tag { value: 42 });

    let e2 = f.spawn();
    f.add(e2, Pos { x: 2, y: 0 });
    // e2 has no Vel

    // 1-component query
    let mut seen = Vec::new();
    f.query::<(&Pos,)>().for_each(|e, (p,)| seen.push((e, p.x)));
    seen.sort_by_key(|(e, _)| e.index);
    assert_eq!(seen, vec![(e1, 1), (e2, 2)]);

    // 2-component query, mutation
    f.query::<(&mut Pos, &Vel)>().for_each(|_, (p, v)| p.x += v.x);
    assert_eq!(f.get::<Pos>(e1).unwrap().x, 2);
    assert_eq!(f.get::<Pos>(e2).unwrap().x, 2, "e2 lacks Vel, must be untouched");

    // 3-component query
    let mut count = 0;
    f.query::<(&Pos, &Vel, &Health)>().for_each(|_, _| count += 1);
    assert_eq!(count, 1);

    // 4-component query
    let mut count4 = 0;
    f.query::<(&Pos, &Vel, &Health, &Tag)>().for_each(|_, _| count4 += 1);
    assert_eq!(count4, 1);
}

#[test]
fn query_without_filter() {
    let reg = test_registry();
    let mut f = Frame::new(reg);
    let e1 = f.spawn();
    f.add(e1, Pos { x: 1, y: 0 });
    let e2 = f.spawn();
    f.add(e2, Pos { x: 2, y: 0 });
    f.add(e2, Tag { value: 1 });

    let mut seen = Vec::new();
    f.query::<(&Pos,)>().without::<Tag>().for_each(|e, _| seen.push(e));
    assert_eq!(seen, vec![e1]);
}

#[test]
#[should_panic(expected = "aliases component")]
fn query_mutable_aliasing_panics() {
    let reg = test_registry();
    let mut f = Frame::new(reg);
    let e = f.spawn();
    f.add(e, Pos { x: 0, y: 0 });
    let _ = f.query::<(&mut Pos, &mut Pos)>();
}

#[test]
fn query_shared_aliasing_is_fine() {
    let reg = test_registry();
    let mut f = Frame::new(reg);
    let e = f.spawn();
    f.add(e, Pos { x: 5, y: 6 });
    let mut sum = 0i64;
    f.query::<(&Pos, &Pos)>().for_each(|_, (a, b)| sum = a.x + b.x);
    assert_eq!(sum, 10);
}

#[test]
fn commands_during_iteration() {
    let reg = test_registry();
    let mut f = Frame::new(reg);
    let e1 = f.spawn();
    f.add(e1, Health { hp: 0, max: 10 });
    let e2 = f.spawn();
    f.add(e2, Health { hp: 5, max: 10 });

    let mut cmds = Commands::new();
    // "iterate" (read-only) and defer structural changes
    f.query::<(&Health,)>().for_each(|e, (h,)| {
        if h.hp <= 0 {
            cmds.despawn(e);
        }
    });
    assert!(f.exists(e1), "despawn must not have applied yet");
    cmds.apply(&mut f);
    assert!(!f.exists(e1));
    assert!(f.exists(e2));
}

#[test]
fn commands_spawn_and_add_pending() {
    let reg = test_registry();
    let mut f = Frame::new(reg);
    let mut cmds = Commands::new();
    let pending = cmds.spawn();
    cmds.add(pending, Pos { x: 7, y: 8 });
    cmds.apply(&mut f);
    assert_eq!(f.alive_count(), 1);
    let mut found = None;
    f.query::<(&Pos,)>().for_each(|e, (p,)| found = Some((e, *p)));
    assert_eq!(found.unwrap().1, Pos { x: 7, y: 8 });
}

#[test]
fn snapshot_restore_checksum_roundtrip() {
    let reg = test_registry();
    let mut f = Frame::new(reg.clone());
    for i in 0..50 {
        let e = f.spawn();
        f.add(e, Pos { x: i, y: -i });
        if i % 2 == 0 {
            f.add(e, Vel { x: 1, y: 1 });
        }
    }
    f.set_tick(7);
    f.set_singleton(GameConfig { gravity: -1, max_players: 2 });
    let h = f.alloc_list::<Score>();
    f.list_push(h, Score { value: 99 });

    let snapshot = f.clone();
    let original_checksum = f.checksum();
    assert_eq!(snapshot.checksum(), original_checksum);

    // mutate
    for i in 0..50 {
        let e = orr_ecs::Entity { index: i as u32, version: 0 };
        if f.exists(e) {
            if let Some(p) = f.get_mut::<Pos>(e) {
                p.x += 1000;
            }
        }
    }
    f.set_tick(8);
    assert_ne!(f.checksum(), original_checksum);

    // restore
    let mut restored = Frame::new(reg);
    restored.copy_from(&snapshot);
    assert_eq!(restored.checksum(), original_checksum);
    assert_eq!(restored.tick(), 7);
}

#[test]
fn frame_ring_wraparound() {
    let reg = test_registry();
    let mut f = Frame::new(reg.clone());
    let mut ring = orr_ecs::FrameRing::new(4, reg.clone());

    let mut checksums = Vec::new();
    for tick in 0..10u64 {
        f.set_tick(tick);
        let e = f.spawn();
        f.add(e, Pos { x: tick as i64, y: 0 });
        ring.store(&f);
        checksums.push((tick, f.checksum()));
    }

    // Only the last 4 ticks (6,7,8,9) should still be retrievable.
    for (tick, cksum) in &checksums {
        if *tick >= 6 {
            assert_eq!(ring.get(*tick).unwrap().checksum(), *cksum);
        } else {
            assert!(ring.get(*tick).is_none(), "tick {tick} should have been overwritten");
        }
    }

    let mut target = Frame::new(reg);
    assert!(ring.restore_into(8, &mut target));
    assert_eq!(target.checksum(), checksums[8].1);
}

#[test]
fn checksum_determinism_across_independent_frames() {
    fn build(reg: std::sync::Arc<orr_ecs::ComponentRegistry>) -> Frame {
        let mut f = Frame::new(reg);
        for i in 0..20 {
            let e = f.spawn();
            f.add(e, Pos { x: i, y: i * 2 });
            if i % 3 == 0 {
                f.add(e, Vel { x: -i, y: i });
            }
        }
        let dead = f.spawn();
        f.despawn(dead);
        f.set_tick(123);
        f
    }
    let reg = test_registry();
    let f1 = build(reg.clone());
    let f2 = build(reg);
    assert_eq!(f1.checksum(), f2.checksum());
}

/// Golden checksum for a fixed, scripted op sequence, hardcoded so CI on
/// other platforms/architectures can compare against this exact value. If
/// this ever needs to change (e.g. a deliberate format change), regenerate
/// it by printing the new checksum and updating the constant deliberately.
const GOLDEN_CHECKSUM: u64 = 14364510420768418636;

#[test]
fn golden_checksum_scripted_sequence() {
    let reg = test_registry();
    let mut f = Frame::new(reg);
    for i in 0..10u32 {
        let e = f.spawn();
        f.add(e, Pos { x: i as i64, y: -(i as i64) });
        f.add(e, Vel { x: 1, y: -1 });
    }
    // despawn a couple, respawn, remove/re-add some components
    let e3 = orr_ecs::Entity { index: 3, version: 0 };
    f.despawn(e3);
    let e_new = f.spawn();
    f.add(e_new, Health { hp: 100, max: 100 });
    f.remove::<Vel>(orr_ecs::Entity { index: 5, version: 0 });
    f.set_singleton(GameConfig { gravity: -10, max_players: 4 });
    let h = f.alloc_list::<Score>();
    f.list_push(h, Score { value: 1 });
    f.list_push(h, Score { value: 2 });
    f.set_tick(42);

    assert_eq!(f.checksum(), GOLDEN_CHECKSUM);
}
