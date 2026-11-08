use super::*;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const ID: &str = "props/public.cylinder.json";
const SEGMENTS: [u32; 5] = [8, 16, 32, 64, 128];

fn recipe(radius: f64, height: f64, segments: u32) -> Value {
    json!({"format":"orr_static_model_cylinder","version":1,"radius":radius,"height":height,"segments":segments})
}

// Reuse the existing independent geometric oracle, never the generator's fixed
// trigonometry table or its construction helpers.
fn close(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "actual {actual:?}, expected {expected:?}, tolerance {tolerance:?}"
    );
}
fn point_key(position: [f32; 3]) -> [u32; 3] {
    position.map(|value| if value == 0.0 { 0 } else { value.to_bits() })
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

// This oracle does not reuse a producer table, triangulation helper or cooked
// fixture. f64 libm is used only to check approximate geometric expectations;
// exact cross-platform producer bits are checked separately below.
fn assert_geometry(model: &StaticModel, radius: f32, height: f32, segments: u32) {
    let source = model.source();
    let n = segments as usize;
    assert_eq!(source.format, "orr_static_model");
    assert_eq!(source.version, 1);
    assert_eq!(source.primitives.len(), 3);
    assert_eq!(source.materials.len(), 3);
    assert_eq!(
        source.images,
        [Image {
            width: 1,
            height: 1,
            rgba8: vec![255; 4]
        }]
    );
    for material in &source.materials {
        assert_eq!(material.base_color, [1.0; 4]);
        assert_eq!(material.image, 0);
        assert!(!material.linear_filter);
        assert_eq!(
            (material.wrap_s, material.wrap_t),
            (Wrap::Clamp, Wrap::Clamp)
        );
    }
    let r = f64::from(radius);
    let h = f64::from(height);
    let mut positions = BTreeSet::new();
    let mut edges: BTreeMap<([u32; 3], [u32; 3]), (usize, i32)> = BTreeMap::new();
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    let mut volume_six = 0.0;
    let mut triangles = 0;
    for (slot, primitive) in source.primitives.iter().enumerate() {
        assert_eq!(
            primitive.id,
            format!("{}#node=0/mesh=0/primitive={slot}", source.asset_id)
        );
        assert_eq!(primitive.transform, crate::IDENTITY);
        assert_eq!(primitive.material, slot as u32);
        assert_eq!(
            primitive.vertices.len(),
            if slot == 0 { 2 * (n + 1) } else { n + 1 }
        );
        assert_eq!(
            primitive.indices.len(),
            if slot == 0 { 6 * n } else { 3 * n }
        );
        let mut used = BTreeSet::new();
        for vertex in &primitive.vertices {
            assert!(vertex
                .position
                .iter()
                .chain(&vertex.normal)
                .chain(&vertex.uv)
                .all(|v| v.is_finite()));
            assert!(vertex.uv.iter().all(|v| (0.0..=1.0).contains(v)));
            for axis in 0..3 {
                min[axis] = min[axis].min(vertex.position[axis]);
                max[axis] = max[axis].max(vertex.position[axis]);
            }
            positions.insert(point_key(vertex.position));
            close(
                dot(vertex.normal.map(f64::from), vertex.normal.map(f64::from)),
                1.0,
                2e-7,
            );
        }
        for face in primitive.indices.chunks_exact(3) {
            assert!(face
                .iter()
                .all(|&index| (index as usize) < primitive.vertices.len()));
            used.extend(face.iter().copied());
        }
        // Index-manifold checks would falsely reject intentional UV seam and
        // hard-normal duplicates. Weld positions, then require oriented closure.
        for face in primitive.indices.chunks_exact(3) {
            let p = [face[0], face[1], face[2]]
                .map(|index| primitive.vertices[index as usize].position.map(f64::from));
            let normal = cross(sub(p[1], p[0]), sub(p[2], p[0]));
            assert!(
                dot(normal, normal) > 0.0,
                "degenerate triangle in slot {slot}"
            );
            let center = [
                (p[0][0] + p[1][0] + p[2][0]) / 3.0,
                (p[0][1] + p[1][1] + p[2][1]) / 3.0,
                (p[0][2] + p[1][2] + p[2][2]) / 3.0,
            ];
            let outward = match slot {
                0 => [center[0], 0.0, center[2]],
                1 => [0.0, 1.0, 0.0],
                _ => [0.0, -1.0, 0.0],
            };
            assert!(dot(normal, outward) > 0.0, "inward winding in slot {slot}");
            for &index in face {
                assert!(
                    dot(
                        normal,
                        primitive.vertices[index as usize].normal.map(f64::from)
                    ) > 0.0
                );
            }
            volume_six += dot(p[0], cross(p[1], p[2]));
            for (a, b) in [(face[0], face[1]), (face[1], face[2]), (face[2], face[0])] {
                let a = point_key(primitive.vertices[a as usize].position);
                let b = point_key(primitive.vertices[b as usize].position);
                assert_ne!(a, b);
                let (key, direction) = if a < b { ((a, b), 1) } else { ((b, a), -1) };
                let entry = edges.entry(key).or_default();
                entry.0 += 1;
                entry.1 += direction;
            }
            triangles += 1;
        }
        assert_eq!(
            used.len(),
            primitive.vertices.len(),
            "no unused or hidden geometry"
        );
    }
    assert_eq!(min, [-radius, -height / 2.0, -radius]);
    assert_eq!(max, [radius, height / 2.0, radius]);
    assert_eq!(positions.len(), 2 * n + 2);
    assert_eq!(triangles, 4 * n);
    assert_eq!(edges.len(), 6 * n);
    assert!(
        edges
            .values()
            .all(|&(count, balance)| count == 2 && balance == 0),
        "closed oriented positional surface required"
    );
    assert_eq!(
        positions.len() + triangles,
        edges.len() + 2,
        "closed genus-zero Euler characteristic"
    );
    let expected_volume = n as f64 * 0.5 * r * r * (std::f64::consts::TAU / n as f64).sin() * h;
    close(volume_six / 6.0, expected_volume, expected_volume * 3e-7);

    let side = &source.primitives[0];
    for i in 0..=n {
        let angle = std::f64::consts::TAU * i as f64 / n as f64;
        for (offset, y, v) in [(0, -height / 2.0, 1.0), (1, height / 2.0, 0.0)] {
            let vertex = &side.vertices[2 * i + offset];
            close(f64::from(vertex.position[0]), r * angle.cos(), r * 2e-7);
            close(f64::from(vertex.position[2]), r * angle.sin(), r * 2e-7);
            assert_eq!(vertex.position[1], y);
            close(f64::from(vertex.normal[0]), angle.cos(), 1e-7);
            close(f64::from(vertex.normal[2]), angle.sin(), 1e-7);
            assert_eq!(vertex.normal[1], 0.0);
            assert_eq!(vertex.uv, [i as f32 / n as f32, v]);
        }
    }
    for offset in [0, 1] {
        assert_eq!(
            side.vertices[offset].position.map(f32::to_bits),
            side.vertices[2 * n + offset].position.map(f32::to_bits)
        );
        assert_eq!(
            side.vertices[offset].normal.map(f32::to_bits),
            side.vertices[2 * n + offset].normal.map(f32::to_bits)
        );
        assert_eq!(
            (
                side.vertices[offset].uv[0],
                side.vertices[2 * n + offset].uv[0]
            ),
            (0.0, 1.0)
        );
    }
    for (slot, y, normal) in [
        (1, height / 2.0, [0.0, 1.0, 0.0]),
        (2, -height / 2.0, [0.0, -1.0, 0.0]),
    ] {
        let cap = &source.primitives[slot];
        assert_eq!(cap.vertices[0].position, [0.0, y, 0.0]);
        assert_eq!(cap.vertices[0].uv, [0.5, 0.5]);
        for vertex in &cap.vertices {
            assert_eq!(vertex.normal, normal, "cap normals are hard, never radial");
            assert_eq!(vertex.position[1], y);
            close(
                f64::from(vertex.uv[0]),
                0.5 + 0.5 * f64::from(vertex.position[0]) / r,
                1e-7,
            );
            close(
                f64::from(vertex.uv[1]),
                0.5 + 0.5 * f64::from(vertex.position[2]) / r,
                1e-7,
            );
        }
        for i in 0..n {
            let side_vertex = &side.vertices[2 * i + usize::from(slot == 1)];
            assert_eq!(
                cap.vertices[i + 1].position.map(f32::to_bits),
                side_vertex.position.map(f32::to_bits)
            );
            assert_ne!(cap.vertices[i + 1].normal, side_vertex.normal);
        }
    }
}

#[test]
fn public_cylinder_api_matches_independent_axial_radial_topology_winding_normals_and_uv_oracles() {
    for segments in SEGMENTS {
        for (radius, height) in [(0.5, 1.0), (0.001, 1_000_000.0), (1_000_000.0, 0.001)] {
            let input = serde_json::to_vec(&recipe(radius, height, segments)).unwrap();
            let model = author_cylinder(&input, ID).unwrap();
            assert_geometry(&model, radius as f32, height as f32, segments);
        }
    }
}

#[test]
fn public_cylinder_api_enforces_strict_schema_original_f64_bounds_and_portable_identity() {
    let valid = recipe(0.5, 1.0, 32);
    for key in ["format", "version", "radius", "height", "segments"] {
        let mut missing = valid.clone();
        missing.as_object_mut().unwrap().remove(key);
        assert!(author_cylinder(&serde_json::to_vec(&missing).unwrap(), ID).is_err());
        let text = serde_json::to_string(&valid).unwrap();
        let duplicate = format!("{{\"{key}\":{},{}", valid[key], &text[1..]);
        assert!(author_cylinder(duplicate.as_bytes(), ID).is_err());
    }
    for key in ["radius", "height"] {
        // Both declared values round onto allowed f32 endpoints, but must fail as f64.
        for value in [0.000_999_999_999, 1_000_000.000_01] {
            assert!((0.001f32..=1_000_000.0).contains(&(value as f32)));
            let mut invalid = valid.clone();
            invalid[key] = json!(value);
            assert!(author_cylinder(&serde_json::to_vec(&invalid).unwrap(), ID).is_err());
        }
        for value in [
            json!(null),
            json!(true),
            json!("0.5"),
            json!(-0.0),
            json!(-1),
            json!(1e100),
        ] {
            let mut invalid = valid.clone();
            invalid[key] = value;
            assert!(author_cylinder(&serde_json::to_vec(&invalid).unwrap(), ID).is_err());
        }
    }
    for segments in [
        json!(0),
        json!(7),
        json!(9),
        json!(129),
        json!(8.0),
        json!("8"),
        json!(null),
    ] {
        let mut invalid = valid.clone();
        invalid["segments"] = segments;
        assert!(author_cylinder(&serde_json::to_vec(&invalid).unwrap(), ID).is_err());
    }
    let mut extra = valid.clone();
    extra["source"] = json!("unexpected.model.json");
    assert!(author_cylinder(&serde_json::to_vec(&extra).unwrap(), ID).is_err());
    let positional = br#"["orr_static_model_cylinder",1,0.5,1.0,32]"#;
    assert!(author_cylinder(positional, ID).is_err());
    let input = serde_json::to_vec(&valid).unwrap();
    for id in [
        "",
        "../bad.cylinder.json",
        "/bad.cylinder.json",
        "bad.model.json",
        "CON.cylinder.json",
        ".hidden/shape.cylinder.json",
        "x\\shape.cylinder.json",
    ] {
        assert!(author_cylinder(&input, id).is_err());
    }
    let boundary = format!("{}.cylinder.json", "a".repeat(240 - ".cylinder.json".len()));
    assert_eq!(
        author_cylinder(&input, &boundary)
            .unwrap()
            .source()
            .asset_id,
        boundary
    );
    assert!(author_cylinder(&input, &format!("a{boundary}")).is_err());
}

#[test]
fn public_cylinder_api_binds_original_f64_recipe_bytes_and_keeps_fixed_grid_golden_bits() {
    let input = serde_json::to_vec(&recipe(0.5, 1.0, 32)).unwrap();
    let model = author_cylinder(&input, ID).unwrap();
    assert_eq!(
        model.source().dependencies,
        vec![Dependency {
            uri: "$source".into(),
            sha256: digest(&input)
        }]
    );
    // Different original f64 dimensions narrow to the same generated f32 geometry.
    let original_numbers =
        serde_json::to_vec(&recipe(0.500_000_000_01, 0.999_999_999_99, 32)).unwrap();
    let rounded = author_cylinder(&original_numbers, ID).unwrap();
    assert_eq!(model.source().primitives, rounded.source().primitives);
    assert_eq!(model.source().materials, rounded.source().materials);
    assert_eq!(model.source().images, rounded.source().images);
    assert_ne!(model.source().dependencies, rounded.source().dependencies);
    assert_eq!(
        rounded.source().dependencies[0].sha256,
        digest(&original_numbers)
    );
    let mut padded = input.clone();
    padded.resize(MAX_RECIPE_BYTES, b' ');
    let boundary = author_cylinder(&padded, ID).unwrap();
    assert_eq!(boundary.source().primitives, model.source().primitives);
    assert_eq!(boundary.source().dependencies[0].sha256, digest(&padded));
    padded.push(b' ');
    assert!(author_cylinder(&padded, ID).is_err());
    let bytes = model.to_bytes().unwrap();
    assert_eq!(
        author_cylinder(&input, ID).unwrap().to_bytes().unwrap(),
        bytes
    );
    assert_eq!(
        StaticModel::from_bytes(&bytes).unwrap().to_bytes().unwrap(),
        bytes
    );
    // Independent binary32 golden for sqrt(1/2), plus exact cardinal positive zeros.
    let d = f32::from_bits(0x3f35_04f3);
    let octants = [
        [1., 0.],
        [d, d],
        [0., 1.],
        [-d, d],
        [-1., 0.],
        [-d, -d],
        [0., -1.],
        [d, -d],
    ];
    for segments in SEGMENTS {
        let model =
            author_cylinder(&serde_json::to_vec(&recipe(1., 2., segments)).unwrap(), ID).unwrap();
        for (octant, [x, z]) in octants.into_iter().enumerate() {
            let vertex = &model.source().primitives[0].vertices[2 * octant * segments as usize / 8];
            assert_eq!(
                vertex.position.map(f32::to_bits),
                [x, -1., z].map(f32::to_bits)
            );
            assert_eq!(
                vertex.normal.map(f32::to_bits),
                [x, 0., z].map(f32::to_bits)
            );
        }
    }
}
