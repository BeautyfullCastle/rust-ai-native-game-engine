use super::*;
fn desc() -> EmitterDesc {
    EmitterDesc { origin: Vec3::ZERO, velocity: Vec3::new(1.0, 2.0, 0.0), velocity_jitter: Vec3::ZERO,
        burst: 4, rate: 0.0, duration: 0.0, lifetime: 1.0, size_start: 2.0, size_end: 0.0,
        color_start: [1.0; 4], color_end: [0.0; 4], seed: 7, blend: ParticleBlend::Alpha }
}
fn key(tick: u64) -> EventKey { EventKey::new(tick, 0, 0) }
fn output(p: &ParticlePool) -> Vec<ParticleDraw> { p.draws().collect() }
fn resync(tick: u64) -> ViewResync {
    ViewResync { generation: 1, discarded_events: 2, head_tick: tick, verified_tick: tick,
        disconnected: false, last_desync: None, lifecycle: Vec::new() }
}
#[test]
fn burst_lifetime_size_color_and_velocity_are_bounded() {
    let mut p = ParticlePool::new(); assert_eq!(p.start(key(1), Some(desc())), Ok(StartOutcome::Started));
    assert_eq!(p.particle_count(), 4); p.advance(0.25).unwrap();
    let draw = p.draws().next().unwrap(); assert_eq!(draw.position, Vec3::new(0.25, 0.5, 0.0));
    assert_eq!(draw.size, 1.5); assert_eq!(draw.color, [0.75; 4]);
    for _ in 0..3 { p.advance(0.25).unwrap(); } assert_eq!(p.particle_count(), 0);
}
#[test]
fn invalid_fields_are_atomic_for_every_component() {
    let mut p = ParticlePool::new(); p.start(key(1), Some(desc())).unwrap(); let before = output(&p);
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::MAX] {
        for field in 0..3 { for component in 0..3 {
            let mut d = desc(); let v = match field { 0 => &mut d.origin, 1 => &mut d.velocity, _ => &mut d.velocity_jitter };
            match component { 0 => v.x = bad, 1 => v.y = bad, _ => v.z = bad }
            assert_eq!(p.start(key(2), Some(d)), Err(ParticleError::InvalidEmitter));
        }}
        for field in 0..2 { for component in 0..4 {
            let mut d = desc(); if field == 0 { d.color_start[component] = bad; } else { d.color_end[component] = bad; }
            assert_eq!(p.start(key(2), Some(d)), Err(ParticleError::InvalidEmitter));
        }}
        for field in 0..5 {
            let mut d = desc(); match field { 0 => d.rate = bad, 1 => d.duration = bad, 2 => d.lifetime = bad, 3 => d.size_start = bad, _ => d.size_end = bad }
            assert_eq!(p.start(key(2), Some(d)), Err(ParticleError::InvalidEmitter));
        }
    }
    for d in [EmitterDesc { lifetime: 0.0, ..desc() }, EmitterDesc { rate: -1.0, ..desc() },
        EmitterDesc { duration: -1.0, ..desc() }, EmitterDesc { burst: 1025, ..desc() },
        EmitterDesc { velocity_jitter: Vec3::new(-1.0, 0.0, 0.0), ..desc() },
        EmitterDesc { color_end: [1.1; 4], ..desc() }] {
        assert_eq!(p.start(key(2), Some(d)), Err(ParticleError::InvalidEmitter));
    }
    assert_eq!(output(&p), before); assert_eq!(p.history, vec![History { key: key(1), canceled: false }]);
}
#[test]
fn delta_rejection_is_atomic_and_pause_freezes() {
    let mut p = ParticlePool::new(); p.start(key(1), Some(desc())).unwrap(); let before = output(&p);
    for dt in [-1.0, 0.251, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(p.advance(dt), Err(ParticleError::InvalidDelta)); assert_eq!(output(&p), before);
    }
    p.lifecycle(Lifecycle::Paused { tick: 1 }); p.advance(0.25).unwrap(); assert_eq!(output(&p), before);
    p.lifecycle(Lifecycle::Resumed { tick: 1 }); p.advance(0.25).unwrap(); assert_ne!(output(&p), before);
}
#[test]
fn same_seed_and_updates_reproduce_jitter_without_global_rng() {
    let mut a = ParticlePool::new(); let mut b = ParticlePool::new();
    let d = EmitterDesc { velocity_jitter: Vec3::new(3.0, 3.0, 3.0), rate: 16.0, duration: 2.0, ..desc() };
    for p in [&mut a, &mut b] { p.start(key(1), Some(d)).unwrap(); }
    for _ in 0..20 { a.advance(0.125).unwrap(); b.advance(0.125).unwrap(); assert_eq!(output(&a), output(&b)); }
}
#[test]
fn capacity_and_rate_drop_never_accumulate_catchup_debt() {
    let mut p = ParticlePool::new(); p.start(key(1), Some(EmitterDesc { burst: 1024, lifetime: 0.25, ..desc() })).unwrap();
    p.start(key(2), Some(EmitterDesc { burst: 0, rate: 8.0, duration: 1.0, ..desc() })).unwrap();
    p.advance(0.125).unwrap(); assert_eq!(p.particle_count(), 1024); assert_eq!(p.emitters[0].emitted, 1);
    p.advance(0.125).unwrap(); assert_eq!(p.particle_count(), 1); assert_eq!(p.emitters[0].emitted, 2);
}
#[test]
fn emitter_limit_consumes_identity_and_started_can_have_zero_particles() {
    let mut p = ParticlePool::new();
    for i in 0..16 { assert_eq!(p.start(key(i), Some(EmitterDesc { burst: 0, rate: 1.0, duration: 1.0, ..desc() })), Ok(StartOutcome::Started)); }
    assert_eq!(p.emitter_count(), 16); assert_eq!(p.start(key(20), Some(desc())), Ok(StartOutcome::AtCapacity));
    p.cancel(key(0)); assert_eq!(p.start(key(20), Some(desc())), Ok(StartOutcome::Duplicate));
    p.reset(); p.start(key(1), Some(EmitterDesc { burst: 1024, ..desc() })).unwrap();
    assert_eq!(p.start(key(2), Some(desc())), Ok(StartOutcome::Started)); assert_eq!(p.particle_count(), 1024);
}
#[test]
fn predicted_verified_canceled_repredicted_and_unmapped_are_ordered() {
    let mut p = ParticlePool::new();
    let event = |status| BridgeEvent::Sim { key: key(1), status };
    p.event(&event(EventStatus::Predicted(())), |_| Some(desc())).unwrap();
    p.event(&event(EventStatus::Verified(())), |_| Some(desc())).unwrap(); assert_eq!(p.particle_count(), 4);
    p.event(&event(EventStatus::Canceled), |_| None).unwrap(); assert_eq!(p.particle_count(), 0);
    p.event(&event(EventStatus::Predicted(())), |_| Some(desc())).unwrap(); assert_eq!(p.particle_count(), 4);
    p.reset(); p.event(&event(EventStatus::Predicted(())), |_| None).unwrap();
    p.event(&event(EventStatus::Verified(())), |_| Some(desc())).unwrap(); assert_eq!(p.particle_count(), 0);
}
#[test]
fn history_eviction_suppresses_ghosts_but_still_cancels_old_particles() {
    let mut p = ParticlePool::new(); p.start(key(1), Some(desc())).unwrap();
    for tick in 2..=4097 { p.start(key(tick), None).unwrap(); }
    assert_eq!(p.history.len(), 4096); assert_eq!(p.floor, Some(1));
    p.cancel(key(1)); assert_eq!(p.particle_count(), 0);
    assert_eq!(p.start(key(1), Some(desc())), Ok(StartOutcome::Suppressed));
    p.lifecycle(Lifecycle::Seeked { from: 4097, to: 0 }); assert_eq!(p.start(key(1), Some(desc())), Ok(StartOutcome::Started));
}
#[test]
fn recovery_seek_branch_disconnect_and_restart_boundaries() {
    let mut p = ParticlePool::new(); p.start(key(1), Some(desc())).unwrap();
    p.lifecycle(Lifecycle::Paused { tick: 1 }); p.resync(&resync(10));
    assert!(p.is_paused()); assert_eq!(p.particle_count(), 0);
    assert_eq!(p.start(key(9), Some(desc())), Ok(StartOutcome::Suppressed));
    p.lifecycle(Lifecycle::Branched { tick: 1, dropped: 9 }); assert!(p.is_paused());
    assert_eq!(p.start(key(1), Some(desc())), Ok(StartOutcome::Started));
    p.lifecycle(Lifecycle::Disconnected); assert_eq!(p.particle_count(), 0);
    assert_eq!(p.start(key(20), Some(desc())), Ok(StartOutcome::Suppressed));
    p.reset(); assert!(!p.is_paused()); assert_eq!(p.start(key(1), Some(desc())), Ok(StartOutcome::Started));
}
#[test]
fn invalid_mapping_does_not_hide_later_cancel_or_lifecycle() {
    let mut p = ParticlePool::new(); p.start(key(1), Some(desc())).unwrap();
    let update = ViewUpdate { snapshot: None, resync: None, events: vec![
        BridgeEvent::Sim { key: key(2), status: EventStatus::Predicted(()) },
        BridgeEvent::Sim { key: key(1), status: EventStatus::Canceled },
        BridgeEvent::Lifecycle(Lifecycle::Paused { tick: 2 })] };
    assert_eq!(p.reconcile(&update, |_| Some(EmitterDesc { lifetime: f32::NAN, ..desc() })), Err(ParticleError::InvalidEmitter));
    assert_eq!(p.particle_count(), 0); assert!(p.is_paused());
}
#[test]
fn maximal_valid_values_remain_finite_and_allocations_do_not_grow() {
    let mut p = ParticlePool::new(); let capacities = (p.particles.capacity(), p.emitters.capacity(), p.history.capacity());
    let d = EmitterDesc { origin: Vec3::new(1_000_000.0, -1_000_000.0, 1_000_000.0), velocity: Vec3::new(10_000.0, -10_000.0, 10_000.0),
        velocity_jitter: Vec3::new(10_000.0, 10_000.0, 10_000.0), burst: 1024, rate: MAX_RATE, duration: MAX_SECONDS,
        lifetime: MAX_SECONDS, size_start: 1000.0, size_end: 1000.0, seed: u64::MAX, ..desc() };
    for i in 0..16 { p.start(key(i), Some(d)).unwrap(); }
    for _ in 0..80 { p.advance(0.25).unwrap(); for draw in p.draws() {
        assert!(draw.position.to_array().into_iter().chain(draw.color).chain([draw.size]).all(f32::is_finite));
        assert!(draw.position.to_array().into_iter().all(|v| v.abs() <= 1_200_000.0));
    }}
    assert_eq!(p.particle_count(), 0); assert_eq!(p.emitter_count(), 0);
    assert_eq!((p.particles.capacity(), p.emitters.capacity(), p.history.capacity()), capacities);
}
#[test]
fn bounded_sequences_preserve_pool_limits_and_finite_draws() {
    let mut p = ParticlePool::new();
    for step in 0..10_000u64 {
        match step % 7 {
            0 => { p.start(key(step), Some(EmitterDesc { burst: 64, rate: 128.0, duration: 0.5, ..desc() })).unwrap(); }
            1 => p.cancel(key(step.saturating_sub(1))),
            2 => { p.advance(0.125).unwrap(); }
            3 => { p.start(key(step), None).unwrap(); }
            4 if step % 91 == 4 => p.resync(&resync(step)),
            5 if step % 119 == 5 => p.lifecycle(Lifecycle::Seeked { from: step, to: 0 }),
            _ => {}
        }
        assert!(p.particle_count() <= MAX_PARTICLES && p.emitter_count() <= MAX_EMITTERS && p.history.len() <= MAX_EVENT_HISTORY);
        assert!(p.draws().all(|d| d.position.to_array().into_iter().chain(d.color).chain([d.size]).all(f32::is_finite)));
    }
}
#[test]
fn cancellation_tombstone_blocks_verification_but_new_prediction_reopens() {
    let mut p = ParticlePool::new();
    let e = |status| BridgeEvent::Sim { key: key(1), status };
    p.event(&e(EventStatus::Predicted(())), |_| Some(desc())).unwrap();
    for _ in 0..2 { p.event(&e(EventStatus::Canceled), |_| None).unwrap(); }
    p.event(&e(EventStatus::Verified(())), |_| Some(desc())).unwrap(); assert_eq!(p.particle_count(), 0);
    p.event(&e(EventStatus::Predicted(())), |_| Some(desc())).unwrap(); assert_eq!(p.particle_count(), 4);
    p.event(&e(EventStatus::Verified(())), |_| Some(desc())).unwrap(); assert_eq!(p.particle_count(), 4);
    p.event(&BridgeEvent::Sim { key: key(2), status: EventStatus::Verified(()) }, |_| Some(desc())).unwrap();
    assert_eq!(p.particle_count(), 8);
    // Even a cancellation for an unseen key prevents a late verified-only replay.
    p.cancel(key(3)); p.event(&BridgeEvent::Sim { key: key(3), status: EventStatus::Verified(()) }, |_| Some(desc())).unwrap();
    assert_eq!(p.particle_count(), 8);
}
#[test]
fn ordered_recovery_summaries_and_sticky_disconnect_are_applied() {
    use orr_bridge::LifecycleRecovery;
    let mut p = ParticlePool::new(); let mut r = resync(9);
    r.lifecycle = vec![LifecycleRecovery { count: 3, last: Lifecycle::Paused { tick: 4 } },
        LifecycleRecovery { count: 1, last: Lifecycle::Resumed { tick: 5 } },
        LifecycleRecovery { count: 1, last: Lifecycle::Paused { tick: 9 } }];
    p.resync(&r); assert!(p.is_paused());
    r.lifecycle.reverse(); p.resync(&r); assert!(p.is_paused());
    r.lifecycle = vec![LifecycleRecovery { count: 1, last: Lifecycle::Paused { tick: 9 } },
        LifecycleRecovery { count: 1, last: Lifecycle::Resumed { tick: 9 } }];
    p.resync(&r); assert!(!p.is_paused());
    r.disconnected = true; p.resync(&r); assert_eq!(p.start(key(10), Some(desc())), Ok(StartOutcome::Suppressed));
    p.lifecycle(Lifecycle::Seeked { from: 9, to: 0 }); assert_eq!(p.start(key(1), Some(desc())), Ok(StartOutcome::Suppressed));
    p.reset(); assert_eq!(p.start(key(1), Some(desc())), Ok(StartOutcome::Started));
}
