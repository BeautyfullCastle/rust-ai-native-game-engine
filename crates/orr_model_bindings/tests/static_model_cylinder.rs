//! Real current-source cylinder CLI -> ordinary package install -> verified
//! bindings, persisted descriptors and immutable reload without authoring files.
//! Run --ignored --exact with the gate-pinned ORR_STATIC_MODEL_CYLINDER_BIN.
#![allow(clippy::float_arithmetic)]

use orr_model::{Dependency, StaticModel};
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform, ModelKind};
use orr_package::{Project, Runtime};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

const PACKAGE: &str = "static-cylinder-bindings";
const IDS: [&str; 2] = ["props/slender.cylinder.json", "props/wide.cylinder.json"];
const ASSETS: [&str; 2] = ["props/slender.model.json", "props/wide.model.json"];
const DIMENSIONS: [(f32, f32, u32); 2] = [(0.25, 1.5, 16), (0.75, 0.25, 64)];

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().canonicalize().unwrap();
        let path = root.join(format!(
            "orr-cylinder-bindings-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("fresh owned fixture directory");
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn produce(recipe: &Path, asset_id: &str, output: &Path) {
    let binary = PathBuf::from(
        std::env::var_os("ORR_STATIC_MODEL_CYLINDER_BIN")
            .expect("mandatory consumer requires current-source pinned cylinder CLI"),
    );
    assert!(binary.is_absolute(), "producer path must be absolute");
    assert!(fs::metadata(&binary).unwrap().is_file());
    let result = Command::new(binary)
        .arg("--recipe")
        .arg(recipe)
        .arg("--asset-id")
        .arg(asset_id)
        .arg("--output")
        .arg(output)
        .output()
        .expect("execute pinned current-source producer");
    assert!(
        result.status.success(),
        "producer failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

// Independently assert the observable contract without importing or invoking the
// recipe implementation. All consumer data comes from the actual CLI bytes.
fn assert_variant(model: &StaticModel, index: usize, recipe: &[u8]) {
    let source = model.source();
    let (radius, height, segments) = DIMENSIONS[index];
    let n = segments as usize;
    assert_eq!(source.asset_id, IDS[index]);
    assert_eq!(source.format, "orr_static_model");
    assert_eq!(source.version, 1);
    assert_eq!(
        source.dependencies,
        [Dependency {
            uri: "$source".into(),
            sha256: digest(recipe)
        }]
    );
    assert_eq!(source.primitives.len(), 3);
    assert_eq!(source.materials.len(), 3);
    assert_eq!(source.images.len(), 1);
    assert_eq!((source.images[0].width, source.images[0].height), (1, 1));
    assert_eq!(source.images[0].rgba8, [255; 4]);
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for (slot, primitive) in source.primitives.iter().enumerate() {
        assert_eq!(
            primitive.id,
            format!("{}#node=0/mesh=0/primitive={slot}", IDS[index])
        );
        assert_eq!(primitive.material, slot as u32);
        assert_eq!(primitive.transform, orr_model::IDENTITY);
        assert_eq!(
            primitive.vertices.len(),
            if slot == 0 { 2 * (n + 1) } else { n + 1 }
        );
        assert_eq!(
            primitive.indices.len(),
            if slot == 0 { 6 * n } else { 3 * n }
        );
        for vertex in &primitive.vertices {
            for axis in 0..3 {
                min[axis] = min[axis].min(vertex.position[axis]);
                max[axis] = max[axis].max(vertex.position[axis]);
            }
            assert!(vertex.uv.iter().all(|v| (0.0..=1.0).contains(v)));
            if slot != 0 {
                assert_eq!(
                    vertex.normal,
                    [0.0, if slot == 1 { 1.0 } else { -1.0 }, 0.0]
                );
            }
        }
    }
    assert_eq!(min, [-radius, -height / 2.0, -radius]);
    assert_eq!(max, [radius, height / 2.0, radius]);
    let side = &source.primitives[0];
    for i in 0..=n {
        let angle = std::f64::consts::TAU * i as f64 / n as f64;
        let vertex = &side.vertices[2 * i];
        assert!((f64::from(vertex.position[0]) - f64::from(radius) * angle.cos()).abs() < 1e-6);
        assert!((f64::from(vertex.position[2]) - f64::from(radius) * angle.sin()).abs() < 1e-6);
        assert_eq!(vertex.uv, [i as f32 / n as f32, 1.0]);
        assert_eq!(side.vertices[2 * i + 1].uv, [i as f32 / n as f32, 0.0]);
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
        assert_ne!(side.vertices[offset].uv, side.vertices[2 * n + offset].uv);
    }
}

#[test]
#[ignore = "mandatory hosted consumer requires the pinned current-source CLI"]
fn installed_static_cylinder_variants_keep_distinct_bindings_and_geometry() {
    let scratch = Scratch::new();
    let authoring = scratch.0.join("authoring");
    fs::create_dir(&authoring).unwrap();
    let package_source = scratch.0.join("authored-package");
    fs::create_dir_all(package_source.join("props")).unwrap();
    let mut recipes = Vec::new();
    let mut cooked = Vec::new();
    for (index, &asset) in ASSETS.iter().enumerate() {
        let (radius, height, segments) = DIMENSIONS[index];
        let recipe = serde_json::to_vec(&json!({"format":"orr_static_model_cylinder","version":1,"radius":radius,"height":height,"segments":segments})).unwrap();
        let recipe_path = authoring.join(Path::new(IDS[index]).file_name().unwrap());
        fs::write(&recipe_path, &recipe).unwrap();
        produce(&recipe_path, IDS[index], &package_source.join(asset));
        let bytes = fs::read(package_source.join(asset)).unwrap();
        let model = StaticModel::from_bytes(&bytes).unwrap();
        assert_variant(&model, index, &recipe);
        assert_eq!(model.to_bytes().unwrap(), bytes);
        assert_eq!(fs::read(&recipe_path).unwrap(), recipe);
        recipes.push(recipe);
        cooked.push(bytes);
    }
    assert_ne!(digest(&cooked[0]), digest(&cooked[1]));
    let ids: BTreeSet<_> = cooked
        .iter()
        .flat_map(|bytes| {
            StaticModel::from_bytes(bytes)
                .unwrap()
                .source()
                .primitives
                .iter()
                .map(|primitive| primitive.id.clone())
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(ids.len(), 6, "independent recipe namespaces never collide");
    fs::write(package_source.join("orr.package.json"), serde_json::to_vec(&json!({"schema":1,"name":PACKAGE,"version":"1.0.0","engine":"^0.0.1","capabilities":["models"],"dependencies":{},"files":ASSETS})).unwrap()).unwrap();
    let root = scratch.0.join("project");
    fs::create_dir(&root).unwrap();
    Project::open_for_install(&root, Runtime::content_only().engine_version)
        .unwrap()
        .install(std::slice::from_ref(&package_source))
        .unwrap();
    fs::rename(&authoring, scratch.0.join("hidden-authoring")).unwrap();
    fs::rename(&package_source, scratch.0.join("hidden-package-source")).unwrap();
    assert!(!authoring.exists());
    assert!(!package_source.exists());
    for id in IDS {
        assert!(!root.join(id).exists());
    }
    let project = model_bindings::open_project(&root).unwrap();
    let lock = project.verify().unwrap();
    let locked = &lock.packages[PACKAGE];
    assert_eq!(locked.files.len(), 2, "only cooked models are installed");
    let catalog = model_bindings::list_assets(&root).unwrap();
    assert_eq!(catalog.len(), 2);
    let mut bindings = BTreeMap::new();
    for (index, &asset) in ASSETS.iter().enumerate() {
        assert!(catalog.iter().any(|entry| entry.package == PACKAGE
            && entry.asset == asset
            && entry.kind == ModelKind::Static));
        assert_eq!(locked.files[asset], digest(&cooked[index]));
        let loaded =
            model_bindings::load_asset_for_kind(&root, PACKAGE, asset, ModelKind::Static).unwrap();
        assert_variant(loaded.static_model().unwrap(), index, &recipes[index]);
        let transform = LocalTransform {
            translation: [if index == 0 { -2.0 } else { 2.0 }, 0.5, -0.25],
            rotation: [0.0, 0.0, 1.0, 0.0],
            scale: [1.0, 2.0, 0.5],
        };
        let binding =
            Binding::from_asset(PACKAGE.into(), asset.into(), &loaded, transform).unwrap();
        binding.validate(&loaded).unwrap();
        bindings.insert(format!("e_{:08x}", index + 1), binding);
    }
    let document = Document {
        version: 2,
        scene: "scene.yaml".into(),
        project: ".".into(),
        bindings,
    };
    document.validate().unwrap();
    let sidecar = root.join("scene.models.json");
    let saved = serde_json::to_vec_pretty(&document).unwrap();
    fs::write(&sidecar, &saved).unwrap();
    let reopened: Document = serde_json::from_slice(&fs::read(&sidecar).unwrap()).unwrap();
    reopened.validate().unwrap();
    assert_eq!(reopened, document);
    // A/B/A loads catch accidental cache or recipe-identity aliasing.
    for index in [0, 1, 0] {
        let binding = &reopened.bindings[&format!("e_{:08x}", index + 1)];
        let loaded = model_bindings::load_binding(&root, binding).unwrap();
        assert_eq!(loaded.source_hash(), digest(&cooked[index]));
        assert_eq!(loaded.package_digest(), locked.digest);
        assert_variant(loaded.static_model().unwrap(), index, &recipes[index]);
        assert_eq!(
            loaded.static_model().unwrap().to_bytes().unwrap(),
            cooked[index]
        );
    }
    let mut wrong_variant = reopened.bindings["e_00000001"].clone();
    wrong_variant.asset = ASSETS[1].into();
    assert!(model_bindings::load_binding(&root, &wrong_variant).is_err());
    for field in ["package", "source"] {
        let mut stale = reopened.bindings["e_00000001"].clone();
        if field == "package" {
            stale.package_digest = "0".repeat(64);
        } else {
            stale.source_hash = "0".repeat(64);
        }
        assert!(model_bindings::load_binding(&root, &stale).is_err());
    }
    let lock_before = fs::read(root.join("orr.packages.lock.json")).unwrap();
    let installed = root
        .join(".orr/packages/objects")
        .join(&locked.digest)
        .join(ASSETS[0]);
    fs::write(&installed, &cooked[1]).unwrap();
    assert!(model_bindings::load_binding(&root, &reopened.bindings["e_00000001"]).is_err());
    assert!(model_bindings::list_assets(&root).is_err());
    assert_eq!(fs::read(&sidecar).unwrap(), saved);
    assert_eq!(
        fs::read(root.join("orr.packages.lock.json")).unwrap(),
        lock_before
    );
    fs::remove_file(&installed).unwrap();
    assert!(model_bindings::load_binding(&root, &reopened.bindings["e_00000001"]).is_err());
    assert!(model_bindings::list_assets(&root).is_err());
    assert_eq!(fs::read(&sidecar).unwrap(), saved);
    assert_eq!(
        fs::read(root.join("orr.packages.lock.json")).unwrap(),
        lock_before
    );
    fs::write(&installed, &cooked[0]).unwrap();
    let restored = model_bindings::load_binding(&root, &reopened.bindings["e_00000001"]).unwrap();
    assert_eq!(
        restored.static_model().unwrap().to_bytes().unwrap(),
        cooked[0]
    );
    for (index, id) in IDS.iter().enumerate() {
        assert_eq!(
            fs::read(
                scratch
                    .0
                    .join("hidden-authoring")
                    .join(Path::new(id).file_name().unwrap())
            )
            .unwrap(),
            recipes[index]
        );
    }
}
