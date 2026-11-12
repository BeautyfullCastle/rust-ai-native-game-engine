//! The generated valley and its meshes, checked without a GPU.
#![allow(clippy::float_arithmetic)]

use orr_scenery::mesh::{self, Mesh, TREE_LEVELS};
use orr_scenery::world::{GRID, HALF};
use orr_scenery::{viewpoints, World};

const SEED: u32 = 7;

#[test]
fn the_same_seed_gives_the_same_world() {
    let (a, b) = (World::generate(SEED), World::generate(SEED));
    assert_eq!(a.heights, b.heights);
    assert_eq!(a.conifers, b.conifers);
    assert_eq!(a.broadleaves, b.broadleaves);
    assert_eq!(a.rocks, b.rocks);
    assert_eq!(a.cover, b.cover);
    assert_eq!(a.cover_chunks, b.cover_chunks);
    assert_ne!(a.heights, World::generate(SEED + 1).heights);
}

#[test]
fn the_stream_only_runs_downhill_and_has_no_jumps() {
    let world = World::generate(SEED);
    let mut z = -HALF;
    let mut cascades = 0;
    let mut in_cascade = false;
    while z < HALF {
        let (here, ahead) = (world.stream(z), world.stream(z + 0.5));
        let drop = here.level - ahead.level;
        assert!((0.0..0.6).contains(&drop), "level changes by {drop} at z = {z}");
        assert!(here.half_width > 2.0 && here.depth > 0.0);
        if here.cascade > 0.99 && !in_cascade {
            cascades += 1;
        }
        in_cascade = here.cascade > 0.99;
        z += 0.5;
    }
    assert!(cascades >= 10, "only {cascades} cascades");
    assert!(world.stream(-HALF).level - world.stream(HALF).level > 20.0);
}

#[test]
fn the_stream_bed_is_under_water_and_the_banks_are_dry() {
    let world = World::generate(SEED);
    for step in 0..200 {
        let z = -HALF + 5.0 + step as f32 * 5.0;
        let s = world.stream(z);
        let bed = world.ground(s.center, z);
        assert!(s.level - bed > 0.2, "no water at z = {z}");
        for side in [-1.0, 1.0] {
            let x = s.center + side * (s.half_width + 8.0);
            assert!(world.ground(x, z) > s.level, "bank under water at z = {z}");
        }
    }
}

#[test]
fn trees_stand_on_dry_ground_inside_the_detailed_square() {
    let world = World::generate(SEED);
    assert!(world.conifers.len() > 3000, "{} conifers", world.conifers.len());
    assert!(world.broadleaves.len() > 1500, "{} broadleaves", world.broadleaves.len());
    for tree in world.conifers.iter().chain(&world.broadleaves) {
        let [x, y, z] = tree.position;
        assert!(x.abs() < HALF && z.abs() < HALF);
        assert!(world.bank_distance(x, z) >= 3.0, "tree in the stream at {x}, {z}");
        assert!((y - world.height(x, z)).abs() < 0.5);
        // Roots are sunk a little into the ground; the ground itself must be above the water.
        assert!(world.height(x, z) > world.stream(z).level, "ground under a tree is below the water at {x}, {z}");
        assert!(tree.scale > 0.5 && tree.scale < 2.0 && tree.yaw.is_finite());
    }
}

#[test]
fn ground_cover_chunks_index_the_instance_list_without_gaps() {
    let world = World::generate(SEED);
    assert!(world.cover.len() > 20_000, "{} cover instances", world.cover.len());
    let mut next = 0;
    for chunk in &world.cover_chunks {
        assert_eq!(chunk.first, next);
        let mut first = chunk.first as usize;
        for (variant, &count) in chunk.counts.iter().enumerate() {
            let instances = &world.cover[first..first + count as usize];
            assert!(instances.iter().all(|p| usize::from(p.variant) == variant));
            first += count as usize;
        }
        next = first as u32;
    }
    assert_eq!(next as usize, world.cover.len());
    for p in &world.cover {
        let [x, y, z] = p.position;
        assert!(world.water_depth(x, z, y) < 0.0, "ground cover under water at {x}, {z}");
    }
}

fn assert_well_formed(name: &str, mesh: &Mesh) {
    assert!(!mesh.indices.is_empty() && mesh.indices.len() % 3 == 0, "{name}: index count");
    assert!(mesh.indices.iter().all(|&i| (i as usize) < mesh.vertices.len()), "{name}: index out of range");
    for v in &mesh.vertices {
        assert!(v.position.iter().chain(&v.color).chain(&v.params).all(|c| c.is_finite()), "{name}: nonfinite vertex");
        let length = v.normal.iter().map(|c| c * c).sum::<f32>().sqrt();
        assert!((length - 1.0).abs() < 1e-3, "{name}: normal length {length}");
        assert!((0.0..=1.0).contains(&v.params[0]), "{name}: ambient visibility {}", v.params[0]);
    }
    assert!(mesh.radius() > 0.2 && mesh.radius() < 40.0, "{name}: radius {}", mesh.radius());
}

#[test]
fn prop_meshes_are_well_formed_and_get_simpler_with_distance() {
    // The seeds the renderer builds its meshes from, and a few others.
    for seed in [0, 1, 2, 100, 101, 102, 200, 201, 202, 300, 301, 302, 400, 401, 402] {
        let conifers: Vec<Mesh> = (0..TREE_LEVELS).map(|level| mesh::conifer(level, seed)).collect();
        let broadleaves: Vec<Mesh> = (0..TREE_LEVELS).map(|level| mesh::broadleaf(level, seed)).collect();
        for family in [&conifers, &broadleaves] {
            for (level, tree) in family.iter().enumerate() {
                assert_well_formed(&format!("tree level {level}"), tree);
            }
            assert!(family.windows(2).all(|pair| pair[0].indices.len() > 2 * pair[1].indices.len()));
            // All levels of one tree have the same height, so a level change does not resize it.
            let top = |m: &Mesh| m.vertices.iter().map(|v| v.position[1]).fold(f32::MIN, f32::max);
            assert!(family.iter().all(|m| (top(m) - top(&family[0])).abs() < 1.5));
        }
        assert_well_formed("rock", &mesh::rock(seed));
        assert_well_formed("grass", &mesh::grass_tuft(false, seed));
        assert_well_formed("flowers", &mesh::grass_tuft(true, seed));
        assert_well_formed("fern", &mesh::fern(seed));
    }
}

#[test]
fn terrain_and_water_meshes_match_the_world() {
    let world = World::generate(SEED);
    let (vertices, indices) = mesh::terrain(&world);
    assert_eq!(vertices.len(), GRID * GRID);
    assert_eq!(indices.len(), (GRID - 1) * (GRID - 1) * 6);
    for v in &vertices {
        assert!(v.position.iter().chain(&v.normal).chain(&v.data).all(|c| c.is_finite()));
        assert!(v.normal[1] > 0.0 && (0.0..=1.0).contains(&v.data[0]) && (0.0..=1.0).contains(&v.data[2]));
    }
    let (far, far_indices) = mesh::far_terrain(&world);
    assert!(far_indices.iter().all(|&i| (i as usize) < far.len()));
    // The backdrop leaves the middle of the detailed square open.
    assert!(far_indices.chunks(3).all(|t| t.iter().any(|&i| {
        let p = far[i as usize].position;
        p[0].abs() > HALF - 40.0 || p[2].abs() > HALF - 40.0
    })));
    let (water, water_indices) = mesh::water(&world);
    assert!(water_indices.iter().all(|&i| (i as usize) < water.len()));
    let wet = water.iter().filter(|v| v.data[0] > 0.0).count();
    assert!(wet * 2 > water.len(), "{wet} of {} water vertices are over water", water.len());
    for v in &water {
        let flow = (v.flow[0] * v.flow[0] + v.flow[1] * v.flow[1]).sqrt();
        assert!((flow - 1.0).abs() < 1e-3 && v.flow[1] > 0.5, "flow {:?}", v.flow);
    }
}

#[test]
fn viewpoints_are_above_the_ground_and_look_at_the_valley() {
    let world = World::generate(SEED);
    let views = viewpoints(&world);
    assert_eq!(views.len(), 6);
    for (index, camera) in views.iter().enumerate() {
        let [x, y, z] = camera.position;
        let floor = world.height(x, z).max(world.stream(z).level);
        assert!(y - floor > 0.5, "viewpoint {} is {} m above the ground", index + 1, y - floor);
        assert!(camera.pitch.abs() < 1.2 && camera.yaw.is_finite());
        let nearest = world.conifers.iter().chain(&world.broadleaves).map(|t| ((t.position[0] - x).powi(2) + (t.position[2] - z).powi(2)).sqrt()).fold(f32::MAX, f32::min);
        // A walking viewpoint must not start inside a trunk.
        assert!(y - floor > 3.0 || nearest > 0.8, "viewpoint {} is {nearest} m from a trunk", index + 1);
    }
}
