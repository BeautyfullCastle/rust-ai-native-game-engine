//! Separate terrain checksum baseline; existing convex golden constants are unchanged.
use orr_ecs::{ComponentRegistryBuilder, Frame};
use orr_fp::{fp, FPVec3, FP};
use orr_physics3d::{Body, Collider, Scratch, Shape, SLEEP_FLAG};
use orr_terrain::Terrain;
use orr_terrain_physics3d::{
    admit_heightfield, register, terrain_config, terrain_step, HeightfieldCollider,
};

#[test]
fn golden_frame_owned_solid_and_hole_spheres() {
    let bytes = include_bytes!("../../../scenes/terrain/sphere_demo.orrt");
    let terrain = Terrain::load(bytes).unwrap();
    let mut registry = ComponentRegistryBuilder::new();
    orr_physics3d::register(&mut registry);
    register(&mut registry);
    let mut frame = Frame::new(registry.build());
    orr_physics3d::init(&mut frame, terrain_config());
    admit_heightfield(
        &mut frame,
        bytes,
        terrain.revision(),
        HeightfieldCollider {
            revision: terrain.revision(),
            friction: fp!(0.6),
            restitution: FP::ZERO,
            layer: 1,
            mask: u32::MAX,
        },
    )
    .unwrap();
    let shape = Shape::sphere(FP::HALF);
    let solid = orr_physics3d::spawn_body(
        &mut frame,
        Body::new_dynamic(FPVec3::new(fp!(-2), fp!(3), FP::ZERO), &shape, FP::ONE),
        Collider::new(shape),
    );
    let hole = orr_physics3d::spawn_body(
        &mut frame,
        Body::new_dynamic(FPVec3::new(fp!(2), fp!(3), fp!(1)), &shape, FP::ONE),
        Collider::new(shape),
    );
    let mut scratch = Scratch::new();
    let mut samples = Vec::new();
    for tick in 1..=240 {
        terrain_step(&mut frame, &mut scratch).unwrap();
        if [60, 120, 240].contains(&tick) {
            samples.push(frame.checksum());
        }
    }
    let grounded = frame.get::<Body>(solid).unwrap();
    assert!(grounded.pos.y >= fp!(0.48) && grounded.pos.y <= fp!(0.51));
    assert_ne!(grounded.sleep & SLEEP_FLAG, 0);
    assert!(frame.get::<Body>(hole).unwrap().pos.y < fp!(-20));
    println!("terrain solid/hole checksums at 60,120,240: {samples:016x?}");
    // New terrain-only baseline. Change only for an intentional behavior or
    // frame schema change; never rewrite the separate convex golden constants.
    assert_eq!(samples, [0x8d551ecfcd815005, 0x9c8701f6d712c0e7, 0xa7fc4ddbfbedeefe],
        "terrain golden changed; explain any intended update in the commit");
}
