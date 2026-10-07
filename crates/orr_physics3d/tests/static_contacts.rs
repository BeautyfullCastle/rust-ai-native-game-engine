//! The optional static geometry seam uses the ordinary solver and frame cache.
mod common;

use common::*;
use orr_ecs::{Entity, Frame};
use orr_fp::{fp, FPVec3, FP};
use orr_physics3d::{
    apply_impulse, is_asleep, step, step_with_static_contacts, Body, Collider, PhysicsConfig,
    PhysicsState, Scratch, StaticContact, StaticContactBody, StaticContactError,
    StaticContactObject,
};

fn object(frame: &mut Frame) -> StaticContactObject {
    StaticContactObject {
        entity: frame.spawn(),
        friction: fp!(0.6),
        restitution: FP::ZERO,
        layer: 1,
        mask: u32::MAX,
    }
}

fn planes(
    bodies: &[StaticContactBody],
    margin: FP,
    out: &mut Vec<StaticContact>,
    terrain: Entity,
    corner: bool,
    reverse: bool,
) {
    for current in bodies {
        let body = current.body;
        let radius = current.collider.shape.radius;
        for (normal, separation, feature_id) in [
            (FPVec3::Y, body.pos.y - radius, 7),
            (FPVec3::X, body.pos.x - radius, 2),
        ] {
            if (normal == FPVec3::X && !corner) || separation > margin {
                continue;
            }
            out.push(StaticContact {
                body: current.entity,
                static_body: terrain,
                point: body.pos - normal * (radius + separation * FP::HALF),
                normal,
                separation,
                feature_id,
            });
        }
    }
    if reverse {
        out.reverse();
    }
}

fn tick(
    frame: &mut Frame,
    scratch: &mut Scratch,
    terrain: StaticContactObject,
    corner: bool,
    reverse: bool,
) {
    step_with_static_contacts(frame, scratch, &[terrain], &mut |bodies, margin, out| {
        planes(bodies, margin, out, terrain.entity, corner, reverse);
        Ok::<_, ()>(())
    })
    .unwrap();
}

#[test]
fn no_static_objects_preserve_convex_frame_bytes() {
    let mut original = new_frame();
    ground(&mut original);
    spawn_sphere(&mut original, v3!(0, 2, 0), FP::HALF);
    spawn_box(&mut original, v3!(0.3, 3.1, 0), v3!(0.4, 0.4, 0.4));
    let mut extended = original.clone();
    let (mut plain_scratch, mut extra_scratch) = (Scratch::new(), Scratch::new());
    for _ in 0..150 {
        step(&mut original, &mut plain_scratch);
        step_with_static_contacts(&mut extended, &mut extra_scratch, &[], &mut |_, _, _| {
            panic!("an empty extension must not invoke the provider");
            #[allow(unreachable_code)]
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(original.to_bytes(), extended.to_bytes());
        assert_eq!(plain_scratch.stats(), extra_scratch.stats());
    }
}

#[test]
fn provider_sees_every_substep_and_final_integrated_pose() {
    let cfg = PhysicsConfig {
        gravity: FPVec3::ZERO,
        substeps: 4,
        sleep_ticks: 0,
        ..PhysicsConfig::default()
    };
    let mut frame = new_frame_with(cfg);
    let terrain = object(&mut frame);
    let sphere = spawn_sphere(&mut frame, v3!(0, 2, 0), FP::HALF);
    frame.get_mut::<Body>(sphere).unwrap().vel = v3!(2, 0, 0);
    let mut positions = Vec::new();
    step_with_static_contacts(
        &mut frame,
        &mut Scratch::new(),
        &[terrain],
        &mut |bodies, _, out| {
            assert!(out.is_empty());
            assert_eq!(bodies.len(), 1);
            positions.push(bodies[0].body.pos.x);
            Ok::<_, ()>(())
        },
    )
    .unwrap();
    assert_eq!(positions.len(), cfg.substeps as usize + 1);
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(positions[0], FP::ZERO);
    assert_eq!(*positions.last().unwrap(), body(&frame, sphere).pos.x);
    assert!(!frame.has::<Body>(terrain.entity));
    assert!(!frame.has::<Collider>(terrain.entity));
}

#[test]
fn independent_normals_and_shuffled_features_solve_identically() {
    let cfg = PhysicsConfig {
        gravity: v3!(-10, -10, 0),
        sleep_ticks: 0,
        ..PhysicsConfig::default()
    };
    let mut frame = new_frame_with(cfg);
    let sphere = spawn_sphere(&mut frame, v3!(0.5, 0.5, 0), FP::HALF);
    // Static entity follows the dynamic entity, exercising the normal flip.
    let terrain = object(&mut frame);
    let mut reversed = frame.clone();
    let (mut scratch, mut reversed_scratch) = (Scratch::new(), Scratch::new());
    for _ in 0..120 {
        tick(&mut frame, &mut scratch, terrain, true, false);
        tick(&mut reversed, &mut reversed_scratch, terrain, true, true);
        assert_eq!(frame.to_bytes(), reversed.to_bytes());
        let contacts = frame.list(frame.singleton::<PhysicsState>().contacts);
        assert_eq!(contacts.len(), 2);
        assert_eq!(
            contacts
                .iter()
                .map(|contact| contact.id)
                .collect::<Vec<_>>(),
            [2, 7]
        );
    }
    let current = body(&frame, sphere);
    assert!((current.pos.x - FP::HALF).abs() < fp!(0.02));
    assert!((current.pos.y - FP::HALF).abs() < fp!(0.02));
    assert!(current.vel.length() < fp!(0.05));
}

#[test]
fn errors_in_late_substeps_and_final_validation_are_atomic() {
    for fail_at in [3, 5] {
        let cfg = PhysicsConfig {
            substeps: 4,
            sleep_ticks: 0,
            ..PhysicsConfig::default()
        };
        let mut frame = new_frame_with(cfg);
        let terrain = object(&mut frame);
        spawn_sphere(&mut frame, v3!(0, 0.5, 0), FP::HALF);
        let before = frame.to_bytes();
        let mut scratch = Scratch::new();
        let mut calls = 0;
        let result = step_with_static_contacts(
            &mut frame,
            &mut scratch,
            &[terrain],
            &mut |bodies, margin, out| {
                calls += 1;
                if calls == fail_at {
                    return Err("rejected geometry");
                }
                planes(bodies, margin, out, terrain.entity, false, false);
                Ok(())
            },
        );
        assert_eq!(
            result,
            Err(StaticContactError::Provider("rejected geometry"))
        );
        assert_eq!(frame.to_bytes(), before);
        assert_eq!(calls, fail_at);
        // Failed temporaries cannot leak into a subsequent successful tick.
        let mut fresh = frame.clone();
        tick(&mut frame, &mut scratch, terrain, false, false);
        tick(&mut fresh, &mut Scratch::new(), terrain, false, false);
        assert_eq!(frame.to_bytes(), fresh.to_bytes());
    }
}

#[test]
fn duplicate_features_and_unallocated_objects_are_rejected_atomically() {
    let mut frame = new_frame();
    let terrain = object(&mut frame);
    spawn_sphere(&mut frame, v3!(0, 0.5, 0), FP::HALF);
    let before = frame.to_bytes();
    let result = step_with_static_contacts(
        &mut frame,
        &mut Scratch::new(),
        &[terrain],
        &mut |bodies, margin, out| {
            planes(bodies, margin, out, terrain.entity, false, false);
            out.push(out[0]);
            Ok::<_, ()>(())
        },
    );
    assert_eq!(result, Err(StaticContactError::DuplicateFeature));
    assert_eq!(frame.to_bytes(), before);
    let absent = StaticContactObject {
        entity: Entity::NONE,
        ..terrain
    };
    let result = step_with_static_contacts(
        &mut frame,
        &mut Scratch::new(),
        &[absent],
        &mut |_, _, _| Ok::<_, ()>(()),
    );
    assert_eq!(
        result,
        Err(StaticContactError::InvalidStaticObject(Entity::NONE))
    );
    assert_eq!(frame.to_bytes(), before);
}

#[test]
fn static_contacts_sleep_and_despawned_support_wakes_the_body() {
    let mut frame = new_frame();
    let terrain = object(&mut frame);
    let sphere = spawn_sphere(&mut frame, v3!(0, 1.5, 0), FP::HALF);
    let mut scratch = Scratch::new();
    for _ in 0..240 {
        tick(&mut frame, &mut scratch, terrain, false, false);
    }
    assert!(is_asleep(&frame, sphere));
    assert!(!frame
        .list(frame.singleton::<PhysicsState>().contacts)
        .is_empty());
    let height = body(&frame, sphere).pos.y;
    assert!(frame.despawn(terrain.entity));
    step(&mut frame, &mut scratch);
    assert!(!is_asleep(&frame, sphere));
    assert!(body(&frame, sphere).pos.y < height);
}

#[test]
fn cache_order_and_rollback_hold_with_interleaved_convex_pairs() {
    let cfg = PhysicsConfig {
        sleep_ticks: 0,
        ..PhysicsConfig::default()
    };
    let mut frame = new_frame_with(cfg);
    let terrain = object(&mut frame);
    ground(&mut frame);
    spawn_sphere(&mut frame, v3!(0, 0.5, 0), FP::HALF);
    spawn_box(&mut frame, v3!(0, 1.49, 0), v3!(0.5, 0.5, 0.5));
    let mut scratch = Scratch::new();
    for _ in 0..25 {
        tick(&mut frame, &mut scratch, terrain, false, false);
        let contacts = frame.list(frame.singleton::<PhysicsState>().contacts);
        assert!(contacts
            .windows(2)
            .all(|pair| (pair[0].a, pair[0].b, pair[0].id) < (pair[1].a, pair[1].b, pair[1].id)));
    }
    let snapshot = frame.clone();
    for _ in 0..40 {
        tick(&mut frame, &mut scratch, terrain, false, false);
    }
    let expected = frame.to_bytes();
    frame.copy_from(&snapshot);
    // Reuse scratch across restore, with deliberately reversed provider output.
    for _ in 0..40 {
        tick(&mut frame, &mut scratch, terrain, false, true);
    }
    assert_eq!(frame.to_bytes(), expected);
}

#[test]
fn sleeping_contact_cache_survives_idle_and_impulse_wakes() {
    let mut frame = new_frame();
    let terrain = object(&mut frame);
    let sphere = spawn_sphere(&mut frame, v3!(0, 0.5, 0), FP::HALF);
    let mut scratch = Scratch::new();
    for _ in 0..100 {
        tick(&mut frame, &mut scratch, terrain, false, false);
    }
    assert!(is_asleep(&frame, sphere));
    let sleeping = frame.to_bytes();
    for _ in 0..10 {
        tick(&mut frame, &mut scratch, terrain, false, false);
        assert_eq!(frame.to_bytes(), sleeping);
    }
    let point = body(&frame, sphere).pos;
    apply_impulse(&mut frame, sphere, v3!(0, 2, 0), point);
    assert!(!is_asleep(&frame, sphere));
    let initial_height = body(&frame, sphere).pos.y;
    tick(&mut frame, &mut scratch, terrain, false, false);
    assert!(body(&frame, sphere).pos.y > initial_height);
}

#[test]
fn static_filters_and_materials_use_existing_solver() {
    let cfg = PhysicsConfig {
        gravity: FPVec3::ZERO,
        sleep_ticks: 0,
        ..PhysicsConfig::default()
    };
    let mut frame = new_frame_with(cfg);
    let mut terrain = object(&mut frame);
    terrain.restitution = FP::ONE;
    let sphere = spawn_sphere(&mut frame, v3!(0, 0.5, 0), FP::HALF);
    frame.get_mut::<Body>(sphere).unwrap().vel = v3!(0, -2, 0);
    let mut filtered = frame.clone();
    tick(&mut frame, &mut Scratch::new(), terrain, false, false);
    assert!(body(&frame, sphere).vel.y > fp!(1.8));
    terrain.mask = 0;
    tick(&mut filtered, &mut Scratch::new(), terrain, false, false);
    assert!(body(&filtered, sphere).vel.y < fp!(-1.8));
    assert!(filtered
        .list(filtered.singleton::<PhysicsState>().contacts)
        .is_empty());
}

#[test]
fn zero_duration_substeps_are_explicit_atomic_errors() {
    let cfg = PhysicsConfig {
        dt: FP::from_raw(1),
        substeps: 2,
        ..PhysicsConfig::default()
    };
    let mut frame = new_frame_with(cfg);
    let terrain = object(&mut frame);
    spawn_sphere(&mut frame, v3!(0, 0.5, 0), FP::HALF);
    let before = frame.to_bytes();
    let result = step_with_static_contacts(
        &mut frame,
        &mut Scratch::new(),
        &[terrain],
        &mut |_, _, _| Ok::<_, ()>(()),
    );
    assert_eq!(result, Err(StaticContactError::InvalidConfig));
    assert_eq!(frame.to_bytes(), before);
}

#[test]
fn same_pair_independent_features_reuse_their_own_persistent_impulses() {
    let cfg = PhysicsConfig {
        gravity: FPVec3::ZERO,
        substeps: 1,
        velocity_iterations: 0,
        sleep_ticks: 0,
        ..PhysicsConfig::default()
    };
    let mut frame = new_frame_with(cfg);
    let terrain = object(&mut frame);
    let sphere = spawn_sphere(&mut frame, v3!(0.5, 0.5, 0), FP::HALF);
    let current = frame.get_mut::<Body>(sphere).unwrap();
    current.inv_mass = FP::ONE;
    current.inv_inertia = FPVec3::ZERO;
    let cache = frame.singleton::<PhysicsState>().contacts;
    for (id, impulse) in [(2, FP::ONE), (7, fp!(2))] {
        frame.list_push(
            cache,
            orr_physics3d::ContactCache {
                a: terrain.entity.index,
                b: sphere.index,
                id,
                _pad: 0,
                normal_impulse: impulse,
                tangent_impulse: FPVec3::ZERO,
            },
        );
    }
    // With no solve iterations, only the two persisted warm starts act.
    tick(&mut frame, &mut Scratch::new(), terrain, true, false);
    assert_eq!(body(&frame, sphere).vel, v3!(1, 2, 0));
}

#[test]
fn optional_path_clamps_before_each_integration() {
    let cfg = PhysicsConfig {
        gravity: v3!(0, -1000, 0),
        dt: fp!(0.1),
        substeps: 2,
        max_linear_speed: FP::ONE,
        sleep_ticks: 0,
        ..PhysicsConfig::default()
    };
    let mut frame = new_frame_with(cfg);
    let terrain = object(&mut frame);
    let sphere = spawn_sphere(&mut frame, v3!(0, 10, 0), FP::HALF);
    step_with_static_contacts(
        &mut frame,
        &mut Scratch::new(),
        &[terrain],
        &mut |bodies, _, _| {
            assert!(bodies[0].body.vel.y.abs() <= FP::ONE);
            Ok::<_, ()>(())
        },
    )
    .unwrap();
    assert_eq!(body(&frame, sphere).pos.y, fp!(10) - cfg.dt);
}

#[test]
fn unsafe_cached_impulse_returns_an_atomic_numeric_error() {
    let mut frame = new_frame();
    let terrain = object(&mut frame);
    let sphere = spawn_sphere(&mut frame, v3!(0, 0.5, 0), FP::HALF);
    let current = frame.get_mut::<Body>(sphere).unwrap();
    current.inv_mass = fp!(100000);
    current.inv_inertia = FPVec3::ZERO;
    let cache = frame.singleton::<PhysicsState>().contacts;
    frame.list_push(
        cache,
        orr_physics3d::ContactCache {
            a: terrain.entity.index,
            b: sphere.index,
            id: 7,
            _pad: 0,
            normal_impulse: fp!(1000000),
            tangent_impulse: FPVec3::ZERO,
        },
    );
    let before = frame.to_bytes();
    let result = step_with_static_contacts(
        &mut frame,
        &mut Scratch::new(),
        &[terrain],
        &mut |bodies, margin, out| {
            planes(bodies, margin, out, terrain.entity, false, false);
            Ok::<_, ()>(())
        },
    );
    assert_eq!(result, Err(StaticContactError::NumericOverflow));
    assert_eq!(frame.to_bytes(), before);
}

#[test]
fn expanded_query_margin_does_not_make_a_hovering_floor() {
    let mut frame = new_frame();
    let terrain = object(&mut frame);
    let sphere = spawn_sphere(&mut frame, v3!(0, 1, 0), FP::HALF);
    let mut scratch = Scratch::new();
    for _ in 0..240 {
        step_with_static_contacts(
            &mut frame,
            &mut scratch,
            &[terrain],
            &mut |bodies, _, out| {
                planes(bodies, fp!(0.15), out, terrain.entity, false, false);
                Ok::<_, ()>(())
            },
        )
        .unwrap();
    }
    assert!((body(&frame, sphere).pos.y - FP::HALF).abs() < fp!(0.02));
}

#[test]
fn losing_support_in_the_final_substep_clears_cache_and_prevents_sleep() {
    let cfg = PhysicsConfig {
        substeps: 1,
        sleep_ticks: 1,
        sleep_linear_speed: FP::ONE,
        ..PhysicsConfig::default()
    };
    let mut frame = new_frame_with(cfg);
    let terrain = object(&mut frame);
    let sphere = spawn_sphere(&mut frame, v3!(0, 0.5, 0), FP::HALF);
    frame.get_mut::<Body>(sphere).unwrap().vel = v3!(0.1, 0, 0);
    // Abstract finite support ends at x=0. The sphere is supported at the
    // initial sample, then crosses the endpoint in the sole integration.
    step_with_static_contacts(
        &mut frame,
        &mut Scratch::new(),
        &[terrain],
        &mut |bodies, margin, out| {
            if bodies[0].body.pos.x <= FP::ZERO {
                planes(bodies, margin, out, terrain.entity, false, false);
            }
            Ok::<_, ()>(())
        },
    )
    .unwrap();
    assert!(body(&frame, sphere).pos.x > FP::ZERO);
    assert!(!is_asleep(&frame, sphere));
    assert_eq!(body(&frame, sphere).sleep, 0);
    assert!(frame
        .list(frame.singleton::<PhysicsState>().contacts)
        .is_empty());
}

fn support_lost_after_first_substep(substeps: u32) {
    let cfg = PhysicsConfig {
        substeps,
        sleep_ticks: 2,
        sleep_linear_speed: FP::ONE,
        ..PhysicsConfig::default()
    };
    let mut frame = new_frame_with(cfg);
    let terrain = object(&mut frame);
    let sphere = spawn_sphere(&mut frame, v3!(0, 0.5, 0), FP::HALF);
    let current = frame.get_mut::<Body>(sphere).unwrap();
    current.vel = v3!(0.1, 0, 0);
    current.sleep = 1;
    let mut calls = 0;
    step_with_static_contacts(
        &mut frame,
        &mut Scratch::new(),
        &[terrain],
        &mut |bodies, margin, out| {
            calls += 1;
            if bodies[0].body.pos.x <= FP::ZERO {
                planes(bodies, margin, out, terrain.entity, false, false);
            }
            Ok::<_, ()>(())
        },
    )
    .unwrap();
    assert_eq!(calls, substeps + 1);
    assert!(body(&frame, sphere).pos.x > FP::ZERO);
    assert!(!is_asleep(&frame, sphere));
    assert_eq!(body(&frame, sphere).sleep, 0);
    assert!(frame
        .list(frame.singleton::<PhysicsState>().contacts)
        .is_empty());
    if substeps > 1 {
        assert!(body(&frame, sphere).vel.y < FP::ZERO);
    }
}

#[test]
fn support_lost_after_first_of_one_substep_resets_existing_sleep_timer() {
    support_lost_after_first_substep(1);
}

#[test]
fn support_lost_after_first_of_two_substeps_resets_existing_sleep_timer() {
    support_lost_after_first_substep(2);
}

#[test]
fn support_lost_after_first_of_eight_substeps_resets_existing_sleep_timer() {
    support_lost_after_first_substep(8);
}

#[test]
fn cached_support_absent_at_first_refresh_resets_existing_sleep_timer() {
    let cfg = PhysicsConfig {
        substeps: 8,
        sleep_ticks: 2,
        sleep_linear_speed: FP::ONE,
        ..PhysicsConfig::default()
    };
    let mut frame = new_frame_with(cfg);
    let terrain = object(&mut frame);
    let sphere = spawn_sphere(&mut frame, v3!(0, 0.5, 0), FP::HALF);
    frame.get_mut::<Body>(sphere).unwrap().sleep = 1;
    let cache = frame.singleton::<PhysicsState>().contacts;
    frame.list_push(
        cache,
        orr_physics3d::ContactCache {
            a: terrain.entity.index,
            b: sphere.index,
            id: 7,
            _pad: 0,
            normal_impulse: FP::ONE,
            tangent_impulse: FPVec3::ZERO,
        },
    );
    step_with_static_contacts(
        &mut frame,
        &mut Scratch::new(),
        &[terrain],
        &mut |_, _, _| Ok::<_, ()>(()),
    )
    .unwrap();
    assert!(!is_asleep(&frame, sphere));
    assert_eq!(body(&frame, sphere).sleep, 0);
    assert!(body(&frame, sphere).vel.y < FP::ZERO);
    assert!(frame.list(cache).is_empty());
}

#[test]
fn endpoint_reacquired_support_may_sleep_after_a_temporary_gap() {
    let cfg = PhysicsConfig {
        substeps: 8,
        sleep_ticks: 1,
        sleep_linear_speed: FP::ONE,
        ..PhysicsConfig::default()
    };
    let mut frame = new_frame_with(cfg);
    let terrain = object(&mut frame);
    let sphere = spawn_sphere(&mut frame, v3!(0, 0.5, 0), FP::HALF);
    let mut calls = 0;
    step_with_static_contacts(
        &mut frame,
        &mut Scratch::new(),
        &[terrain],
        &mut |bodies, margin, out| {
            calls += 1;
            // Support exists at the initial and final boundary, with a gap
            // between them. Only endpoint ownership controls the loss veto.
            if calls == 1 || calls >= cfg.substeps {
                planes(bodies, margin, out, terrain.entity, false, false);
            }
            Ok::<_, ()>(())
        },
    )
    .unwrap();
    assert!(is_asleep(&frame, sphere));
    assert_eq!(
        frame.list(frame.singleton::<PhysicsState>().contacts).len(),
        1
    );
}

fn tiny_gravity_sleep_config() -> PhysicsConfig {
    PhysicsConfig {
        gravity: v3!(0, -0.1, 0),
        max_linear_speed: fp!(16),
        sleep_ticks: 1,
        sleep_linear_speed: FP::ONE,
        ..PhysicsConfig::default()
    }
}

#[test]
fn tiny_gravity_free_flight_cannot_sleep_without_support_across_ticks() {
    let mut frame = new_frame_with(tiny_gravity_sleep_config());
    let terrain = object(&mut frame);
    let sphere = spawn_sphere(&mut frame, v3!(0, 2, 0), FP::HALF);
    let mut scratch = Scratch::new();
    for _ in 0..60 {
        step_with_static_contacts(&mut frame, &mut scratch, &[terrain], &mut |_, _, _| {
            Ok::<_, ()>(())
        })
        .unwrap();
        assert!(!is_asleep(&frame, sphere));
        assert_eq!(body(&frame, sphere).sleep, 0);
    }
    assert!(body(&frame, sphere).pos.y < fp!(2));
    assert!(body(&frame, sphere).vel.y < FP::ZERO);
    assert!(body(&frame, sphere).vel.length() < FP::ONE);
}

#[test]
fn touching_free_flight_cluster_cannot_ground_itself() {
    let mut frame = new_frame_with(tiny_gravity_sleep_config());
    let terrain = object(&mut frame);
    let lower = spawn_sphere(&mut frame, v3!(0, 4, 0), FP::HALF);
    let upper = spawn_sphere(&mut frame, v3!(0, 4.989, 0), FP::HALF);
    let initial_sum = body(&frame, lower).pos.y + body(&frame, upper).pos.y;
    let mut scratch = Scratch::new();
    let mut observed_contact = false;
    for _ in 0..30 {
        step_with_static_contacts(&mut frame, &mut scratch, &[terrain], &mut |_, _, _| {
            Ok::<_, ()>(())
        })
        .unwrap();
        assert!(!is_asleep(&frame, lower));
        assert!(!is_asleep(&frame, upper));
        observed_contact |= scratch.stats().manifolds > 0;
    }
    // The small initial overlap can resolve into physical separation later;
    // the contact-connected cluster must not sleep before or after that split.
    assert!(observed_contact);
    assert!(body(&frame, lower).pos.y + body(&frame, upper).pos.y < initial_sum);
}

#[test]
fn speculative_proximity_does_not_ground_a_slowly_falling_sphere() {
    let mut frame = new_frame_with(tiny_gravity_sleep_config());
    let terrain = object(&mut frame);
    let sphere = spawn_sphere(&mut frame, v3!(0, 0.6, 0), FP::HALF);
    let mut scratch = Scratch::new();
    for _ in 0..30 {
        step_with_static_contacts(
            &mut frame,
            &mut scratch,
            &[terrain],
            &mut |bodies, _, out| {
                planes(bodies, fp!(0.15), out, terrain.entity, false, false);
                Ok::<_, ()>(())
            },
        )
        .unwrap();
        assert!(!is_asleep(&frame, sphere));
    }
    assert!(body(&frame, sphere).pos.y < fp!(0.6));
    assert!(body(&frame, sphere).pos.y > fp!(0.51));
    for _ in 0..240 {
        tick(&mut frame, &mut scratch, terrain, false, false);
    }
    assert!(is_asleep(&frame, sphere));
    assert!((body(&frame, sphere).pos.y - FP::HALF).abs() < fp!(0.02));
}

#[test]
fn terrain_grounded_dynamic_stack_still_sleeps() {
    let mut frame = new_frame();
    let terrain = object(&mut frame);
    let lower = spawn_sphere(&mut frame, v3!(0, 0.5, 0), FP::HALF);
    let upper = spawn_sphere(&mut frame, v3!(0, 1.5, 0), FP::HALF);
    let mut scratch = Scratch::new();
    for _ in 0..240 {
        tick(&mut frame, &mut scratch, terrain, false, false);
    }
    assert!(is_asleep(&frame, lower));
    assert!(is_asleep(&frame, upper));
}

#[test]
fn convex_grounded_dynamic_stack_still_sleeps_with_a_static_provider() {
    let mut frame = new_frame();
    ground(&mut frame);
    let terrain = object(&mut frame);
    let lower = spawn_sphere(&mut frame, v3!(0, 0.5, 0), FP::HALF);
    let upper = spawn_sphere(&mut frame, v3!(0, 1.5, 0), FP::HALF);
    let mut scratch = Scratch::new();
    for _ in 0..240 {
        step_with_static_contacts(&mut frame, &mut scratch, &[terrain], &mut |_, _, _| {
            Ok::<_, ()>(())
        })
        .unwrap();
    }
    assert!(is_asleep(&frame, lower));
    assert!(is_asleep(&frame, upper));
}

#[test]
fn wall_and_ceiling_impulses_do_not_ground_a_body_against_downward_gravity() {
    for normal in [FPVec3::X, -FPVec3::Y] {
        let cfg = PhysicsConfig {
            substeps: 1,
            max_linear_speed: FP::ONE,
            ..tiny_gravity_sleep_config()
        };
        let mut frame = new_frame_with(cfg);
        let terrain = object(&mut frame);
        let sphere = spawn_sphere(&mut frame, v3!(0.5, 0.5, 0), FP::HALF);
        frame.get_mut::<Body>(sphere).unwrap().vel = -normal * fp!(0.1);
        let initial_projection = body(&frame, sphere).pos.dot(normal);
        step_with_static_contacts(
            &mut frame,
            &mut Scratch::new(),
            &[terrain],
            &mut |bodies, _, out| {
                let current = bodies[0];
                out.push(StaticContact {
                    body: current.entity,
                    static_body: terrain.entity,
                    point: current.body.pos - normal * FP::HALF,
                    normal,
                    separation: current.body.pos.dot(normal) - initial_projection,
                    feature_id: 3,
                });
                Ok::<_, ()>(())
            },
        )
        .unwrap();
        let cache = frame.list(frame.singleton::<PhysicsState>().contacts);
        assert!(cache[0].normal_impulse > FP::ZERO);
        assert!(!is_asleep(&frame, sphere));
        assert_eq!(body(&frame, sphere).sleep, 0);
    }
}

#[test]
fn zero_gravity_free_space_may_sleep_with_a_static_provider() {
    let cfg = PhysicsConfig {
        gravity: FPVec3::ZERO,
        ..tiny_gravity_sleep_config()
    };
    let mut frame = new_frame_with(cfg);
    let terrain = object(&mut frame);
    let sphere = spawn_sphere(&mut frame, v3!(0, 2, 0), FP::HALF);
    step_with_static_contacts(
        &mut frame,
        &mut Scratch::new(),
        &[terrain],
        &mut |_, _, _| Ok::<_, ()>(()),
    )
    .unwrap();
    assert!(is_asleep(&frame, sphere));
}

#[test]
fn falling_onto_a_terrain_grounded_sphere_stack_eventually_sleeps() {
    // Nearby release heights cover both penetrating arrival and arrival
    // inside the ordinary convex solver's positive speculative contact skin.
    for height in [fp!(3), fp!(3.03)] {
        let mut frame = new_frame();
        let terrain = object(&mut frame);
        let lower = spawn_sphere(&mut frame, v3!(0, 0.5, 0), FP::HALF);
        let upper = spawn_sphere(
            &mut frame,
            FPVec3::new(FP::ZERO, height, FP::ZERO),
            FP::HALF,
        );
        let mut scratch = Scratch::new();
        for _ in 0..360 {
            tick(&mut frame, &mut scratch, terrain, false, false);
        }
        let lower_body = body(&frame, lower);
        let upper_body = body(&frame, upper);
        let separation = (upper_body.pos - lower_body.pos).length() - FP::ONE;
        let cache = frame.list(frame.singleton::<PhysicsState>().contacts);
        assert!(is_asleep(&frame, lower),
            "lower from height {height}: lower={lower_body:?}; upper={upper_body:?}; pair separation={separation}; cache={cache:?}");
        assert!(is_asleep(&frame, upper),
            "upper from height {height}: lower={lower_body:?}; upper={upper_body:?}; pair separation={separation}; cache={cache:?}");
    }
}
