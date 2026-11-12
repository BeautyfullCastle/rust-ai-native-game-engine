#![allow(clippy::float_arithmetic)]
use super::*;
use orr_model::{StaticModel, Wrap, IDENTITY};
fn spec() -> CylinderSpec {
    CylinderSpec {
        radius: 0.5,
        height: 2.0,
        segments: 8,
        asset_id: "props/column.cylinder.json".into(),
    }
}
fn unopened() -> CylinderRequest {
    CylinderRequest {
        license: PathBuf::new(),
        destination: PathBuf::new(),
        name: "new-column".into(),
        version: "1.0.0".into(),
        license_acknowledged: true,
        spec: spec(),
    }
}
fn request(root: &Path) -> CylinderRequest {
    let license = root.join("LICENSE.txt");
    fs::write(&license, b"Rights reviewed\n").unwrap();
    CylinderRequest {
        license,
        destination: root.join("new-package"),
        ..unopened()
    }
}
fn close(a: f64, b: f64, tolerance: f64) {
    assert!((a - b).abs() <= tolerance, "{a} != {b}");
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
// Independent analytic ring, cap and winding oracle; no producer trig table,
// model importer or fixture geometry participates in expected positions.
fn geometry(model: &StaticModel, radius: f32, height: f32, segments: u32) {
    let source = model.source();
    let n = segments as usize;
    let r = f64::from(radius);
    assert_eq!(source.primitives.len(), 3);
    assert_eq!(source.materials.len(), 3);
    assert_eq!(source.images.len(), 1);
    assert_eq!(source.images[0].rgba8, vec![255; 4]);
    for material in &source.materials {
        assert_eq!(material.base_color, [1.0; 4]);
        assert_eq!(material.image, 0);
        assert!(!material.linear_filter);
        assert_eq!(
            (material.wrap_s, material.wrap_t),
            (Wrap::Clamp, Wrap::Clamp)
        );
    }
    let side = &source.primitives[0];
    assert_eq!(side.vertices.len(), 2 * (n + 1));
    assert_eq!(side.indices.len(), 6 * n);
    for index in 0..=n {
        let angle = std::f64::consts::TAU * index as f64 / n as f64;
        for (offset, y, v) in [(0, -height / 2.0, 1.0), (1, height / 2.0, 0.0)] {
            let vertex = &side.vertices[2 * index + offset];
            close(f64::from(vertex.position[0]), r * angle.cos(), r * 2e-7);
            close(f64::from(vertex.position[2]), r * angle.sin(), r * 2e-7);
            assert_eq!(vertex.position[1], y);
            close(f64::from(vertex.normal[0]), angle.cos(), 1e-7);
            close(f64::from(vertex.normal[2]), angle.sin(), 1e-7);
            assert_eq!(vertex.normal[1], 0.0);
            assert_eq!(vertex.uv, [index as f32 / n as f32, v]);
        }
    }
    for offset in [0, 1] {
        assert_eq!(
            side.vertices[offset].position.map(f32::to_bits),
            side.vertices[2 * n + offset].position.map(f32::to_bits)
        );
        assert_eq!(side.vertices[offset].uv[0], 0.0);
        assert_eq!(side.vertices[2 * n + offset].uv[0], 1.0);
    }
    for (slot, y, normal) in [
        (1, height / 2.0, [0.0, 1.0, 0.0]),
        (2, -height / 2.0, [0.0, -1.0, 0.0]),
    ] {
        let cap = &source.primitives[slot];
        assert_eq!(cap.vertices.len(), n + 1);
        assert_eq!(cap.indices.len(), 3 * n);
        assert_eq!(cap.vertices[0].position, [0.0, y, 0.0]);
        assert_eq!(cap.vertices[0].uv, [0.5, 0.5]);
        for (index, vertex) in cap.vertices.iter().enumerate() {
            if index > 0 {
                let angle = std::f64::consts::TAU * (index - 1) as f64 / n as f64;
                close(f64::from(vertex.position[0]), r * angle.cos(), r * 2e-7);
                close(f64::from(vertex.position[2]), r * angle.sin(), r * 2e-7);
            }
            assert_eq!(vertex.position[1], y);
            assert_eq!(vertex.normal, normal);
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
    }
    for (slot, p) in source.primitives.iter().enumerate() {
        assert_eq!(p.transform, IDENTITY);
        assert_eq!(p.material, slot as u32);
        assert_eq!(
            p.id,
            format!("{}#node=0/mesh=0/primitive={slot}", source.asset_id)
        );
        for face in p.indices.chunks_exact(3) {
            let points =
                [face[0], face[1], face[2]].map(|i| p.vertices[i as usize].position.map(f64::from));
            let normal = cross(sub(points[1], points[0]), sub(points[2], points[0]));
            assert!(dot(normal, normal) > 0.0);
            let outward = match slot {
                0 => [
                    points[0][0] + points[1][0] + points[2][0],
                    0.0,
                    points[0][2] + points[1][2] + points[2][2],
                ],
                1 => [0.0, 1.0, 0.0],
                _ => [0.0, -1.0, 0.0],
            };
            assert!(dot(normal, outward) > 0.0);
            for index in face {
                assert!(dot(normal, p.vertices[*index as usize].normal.map(f64::from)) > 0.0);
            }
        }
    }
}
#[test]
fn cylinder_source_less_creation_matches_independent_geometry_for_all_segments_and_bounds() {
    for segments in [8, 16, 32, 64, 128] {
        for (radius, height) in [(0.5, 2.0), (0.001, 1_000_000.0), (1_000_000.0, 0.001)] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let mut req = request(&root);
            req.spec.segments = segments;
            req.spec.radius = radius;
            req.spec.height = height;
            // Only a license exists. Creation cannot read or inspect a model.
            assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
            let report = create_cylinder_package(req).unwrap();
            let bytes = fs::read(report.root.join(&report.asset)).unwrap();
            geometry(
                &StaticModel::from_bytes(&bytes).unwrap(),
                radius as f32,
                height as f32,
                segments,
            );
        }
    }
}
#[test]
fn cylinder_exact_recipe_only_provenance_and_models_only_publication_preserve_license() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let req = request(&root);
    let license = req.license.clone();
    let report = create_cylinder_package(req).unwrap();
    assert_eq!(report.kind, ModelKind::Static);
    assert_eq!(report.asset, "model.orrmodel");
    let bytes = fs::read(report.root.join(&report.asset)).unwrap();
    let model = StaticModel::from_bytes(&bytes).unwrap();
    let recipe=br#"{"format":"orr_static_model_cylinder","version":1,"radius":0.5,"height":2.0,"segments":8}"#;
    assert_eq!(model.source().dependencies.len(), 1);
    assert_eq!(model.source().dependencies[0].uri, "$source");
    assert_eq!(model.source().dependencies[0].sha256, digest(recipe));
    assert_eq!(report.source_sha256, digest(&bytes));
    assert_eq!(fs::read(&license).unwrap(), b"Rights reviewed\n");
    assert_eq!(
        fs::read(report.root.join("LICENSE.txt")).unwrap(),
        fs::read(&license).unwrap()
    );
    assert_eq!(report.license_sha256, digest(&fs::read(license).unwrap()));
    let package =
        orr_package::inspect(&report.root, &orr_sample::room_project::compiled_runtime()).unwrap();
    assert_eq!(
        package.manifest.capabilities,
        BTreeSet::from(["models".into()])
    );
    assert_eq!(
        package.manifest.files,
        BTreeSet::from(["LICENSE.txt".into(), "model.orrmodel".into()])
    );
    assert_eq!(fs::read_dir(root).unwrap().count(), 2);
}
#[test]
fn cylinder_original_f64_recipe_values_preserve_recipe_provenance() {
    let mut a = spec();
    a.radius = 0.1;
    a.height = 0.3;
    let recipe = cylinder_recipe_bytes(&a).unwrap();
    assert_eq!(recipe,br#"{"format":"orr_static_model_cylinder","version":1,"radius":0.1,"height":0.3,"segments":8}"#);
    let first =
        StaticModel::from_bytes(&cylinder_model_bytes(&recipe, &a.asset_id).unwrap()).unwrap();
    let mut b = a.clone();
    b.radius = f64::from(a.radius as f32);
    b.height = f64::from(a.height as f32);
    let second = StaticModel::from_bytes(
        &cylinder_model_bytes(&cylinder_recipe_bytes(&b).unwrap(), &b.asset_id).unwrap(),
    )
    .unwrap();
    assert_eq!(first.source().primitives, second.source().primitives);
    assert_ne!(first.source().dependencies, second.source().dependencies);
}
#[test]
fn cylinder_invalid_spec_and_rights_reject_before_paths() {
    let mut r = unopened();
    r.license_acknowledged = false;
    assert!(create_cylinder_package(r)
        .err()
        .unwrap()
        .contains("acknowledgement"));
    for id in [
        "",
        "../bad.cylinder.json",
        "CON.cylinder.json",
        "bad.uv.json",
        "x//bad.cylinder.json",
    ] {
        let mut r = unopened();
        r.spec.asset_id = id.into();
        assert!(create_cylinder_package(r)
            .err()
            .unwrap()
            .contains("variant identity"));
    }
    for bad in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        -0.0,
        0.0,
        -1.0,
        0.001 - f64::EPSILON,
        1_000_000.0 + f64::EPSILON * 1_000_000.0,
    ] {
        for radius in [true, false] {
            let mut r = unopened();
            if radius {
                r.spec.radius = bad;
            } else {
                r.spec.height = bad;
            }
            assert!(create_cylinder_package(r)
                .err()
                .unwrap()
                .contains("finite f64"));
        }
    }
    for segments in [0, 1, 7, 9, 127, 129, u32::MAX] {
        let mut r = unopened();
        r.spec.segments = segments;
        assert!(create_cylinder_package(r)
            .err()
            .unwrap()
            .contains("segments"));
    }
}
#[test]
fn cylinder_private_pipeline_refuses_source_hash_source_path_and_wrong_mode() {
    for case in 0..4 {
        let mut req = Request {
            source: PathBuf::new(),
            license: PathBuf::new(),
            destination: PathBuf::new(),
            name: "column".into(),
            version: "1.0.0".into(),
            kind: ModelKind::Static,
            source_format: SourceFormat::CookedStatic,
            license_acknowledged: true,
        };
        let mut hash = None;
        match case {
            0 => req.source = "not-read.model.json".into(),
            1 => hash = Some("a".repeat(64)),
            2 => req.kind = ModelKind::Animated,
            _ => req.source_format = SourceFormat::RawGlb,
        }
        assert!(
            import_package_inner(req, Some(AuthoringVariant::Cylinder(spec())), hash)
                .err()
                .unwrap()
                .contains("source-less Static mode")
        );
    }
}
#[test]
fn cylinder_destination_policy_and_path_shapes_reject_without_partial_output() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut req = request(&root);
    req.destination = root.join(".orr").join("no-output");
    assert!(create_cylinder_package(req)
        .err()
        .unwrap()
        .contains(".orr store"));
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    let mut req = unopened();
    req.license = "relative-license".into();
    assert!(create_cylinder_package(req)
        .err()
        .unwrap()
        .contains("path must"));
}
#[test]
fn cylinder_invalid_license_metadata_and_existing_destination_preserve_original_files() {
    for case in 0..6 {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let mut req = request(&root);
        let destination = req.destination.clone();
        let license = req.license.clone();
        match case {
            0 => fs::write(&license, [0xff]).unwrap(),
            1 => fs::write(&license, b"  ").unwrap(),
            2 => req.name = "../bad".into(),
            3 => req.version = "not-semver".into(),
            4 => {
                fs::create_dir(&destination).unwrap();
                fs::write(destination.join("keep"), b"untouched").unwrap();
            }
            _ => fs::write(&license, vec![b'x'; MAX_LICENSE_BYTES as usize + 1]).unwrap(),
        }
        let before = fs::read(&license).unwrap();
        assert!(create_cylinder_package(req).is_err());
        assert_eq!(fs::read(license).unwrap(), before);
        if case == 4 {
            assert_eq!(fs::read(destination.join("keep")).unwrap(), b"untouched");
        } else {
            assert!(!destination.exists());
            assert_eq!(fs::read_dir(root).unwrap().count(), 1);
        }
    }
}
#[cfg(unix)]
#[test]
fn cylinder_linked_license_and_destination_parent_fail_closed() {
    for license_link in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let mut req = request(&root);
        let link = root.join("linked");
        if license_link {
            std::os::unix::fs::symlink(&req.license, &link).unwrap();
            req.license = link;
        } else {
            let parent = root.join("real-parent");
            fs::create_dir(&parent).unwrap();
            std::os::unix::fs::symlink(&parent, &link).unwrap();
            req.destination = link.join("output");
        }
        let destination = req.destination.clone();
        assert!(create_cylinder_package(req).is_err());
        assert!(!destination.exists());
        assert_eq!(
            fs::read(root.join("LICENSE.txt")).unwrap(),
            b"Rights reviewed\n"
        );
    }
}
