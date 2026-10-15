//! Mandatory current-source CLI -> installed cooked variants -> persistent bindings.
//! Run with --ignored --exact and an absolute ORR_STATIC_MODEL_RETEXTURE_BIN
//! pinned by the acceptance gate. Missing producer evidence is a failure, never a skip.
#![allow(clippy::float_arithmetic)]

use orr_model::{Dependency, Image, StaticModel, Wrap};
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

const ORIGINAL: &[u8] = include_bytes!("../../orr_model/tests/fixtures/model.glb");
const SOURCE_ID: &str = "scene/source.glb";
const SOURCE_PATH: &str = "inputs/prop.model.json";
const PACKAGE: &str = "static-retexture-bindings";
const IDS: [&str; 2] = [
    "variants/magenta.retexture.json",
    "variants/cyan.retexture.json",
];
const ASSETS: [&str; 2] = ["variants/magenta.model.json", "variants/cyan.model.json"];
const TEXTURES: [&str; 2] = ["images/magenta.png", "images/cyan.png"];
const COLORS: [[u8; 4]; 2] = [[255, 0, 255, 0], [0, 255, 255, 64]];

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "orr-static-retexture-bindings-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("fresh owned fixture directory");
        // macOS /var may be a symlink: the producer requires canonical ancestry.
        Self(fs::canonicalize(path).unwrap())
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

// Original fixture geometry with two independently placed users of each material.
// Slot 2 is deliberately unused, so a table-compacting implementation must fail.
fn source_model() -> StaticModel {
    let imported = orr_model::import::import_with_resolver(SOURCE_ID, ORIGINAL, |_| {
        panic!("embedded GLB must not resolve external inputs")
    })
    .unwrap();
    let mut source = imported.source().clone();
    let original_primitives = source.primitives.clone();
    source.primitives.clear();
    for (node, sign, y) in [(1, -1.0, 1.25), (7, 1.0, -1.25)] {
        for (slot, primitive) in original_primitives.iter().enumerate() {
            let mut primitive = primitive.clone();
            primitive.id = format!("{SOURCE_ID}#node={node}/mesh=0/primitive={slot}");
            primitive.transform = [
                [0.75 * sign, 0.0, 0.0, 0.0],
                [0.0, 0.75, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, y, 0.1, 1.0],
            ];
            source.primitives.push(primitive);
        }
    }
    source.materials[0].base_color[3] = 0.25;
    source.materials[0].wrap_s = Wrap::Mirror;
    source.materials[0].wrap_t = Wrap::Repeat;
    source.materials[1].base_color[3] = 0.75;
    source.images.push(Image {
        width: 1,
        height: 1,
        rgba8: vec![17, 31, 47, 63],
    });
    let mut unused = source.materials[0].clone();
    unused.image = 2;
    unused.base_color = [0.25, 0.5, 0.75, 0.125];
    unused.linear_filter = true;
    source.materials.push(unused);
    source.dependencies.push(Dependency {
        uri: "provenance/original.txt".into(),
        sha256: digest(b"upstream metadata is bound by the cooked source hash"),
    });
    StaticModel::new(source).unwrap()
}

// Small test-owned RGBA PNG encoder, using a stored DEFLATE block. No codec,
// network service, socket, external tool, or producer implementation is reused.
fn png(color: [u8; 4]) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let mut crc = !0_u32;
        for &byte in kind.iter().chain(data) {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb88320 & 0_u32.wrapping_sub(crc & 1));
            }
        }
        out.extend_from_slice(&(!crc).to_be_bytes());
    }
    let mut raw = Vec::new();
    for _ in 0..2 {
        raw.push(0);
        raw.extend_from_slice(&color);
        raw.extend_from_slice(&color);
    }
    let length = raw.len() as u16;
    let mut compressed = vec![0x78, 0x01, 0x01];
    compressed.extend_from_slice(&length.to_le_bytes());
    compressed.extend_from_slice(&(!length).to_le_bytes());
    compressed.extend_from_slice(&raw);
    let (mut a, mut b) = (1_u32, 0_u32);
    for byte in raw {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    compressed.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(
        &mut bytes,
        b"IHDR",
        &[0, 0, 0, 2, 0, 0, 0, 2, 8, 6, 0, 0, 0],
    );
    chunk(&mut bytes, b"IDAT", &compressed);
    chunk(&mut bytes, b"IEND", &[]);
    bytes
}
fn produce(recipe: &Path, asset_id: &str, output: &Path) {
    let binary = PathBuf::from(
        std::env::var_os("ORR_STATIC_MODEL_RETEXTURE_BIN")
            .expect("mandatory consumer requires the current-source pinned CLI"),
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
fn assert_variant(
    actual: &StaticModel,
    original: &StaticModel,
    index: usize,
    recipe: &[u8],
    source_bytes: &[u8],
    texture: &[u8],
) {
    let before = original.source();
    let after = actual.source();
    assert_eq!(after.asset_id, IDS[index]);
    assert_eq!(after.primitives.len(), 4);
    assert_eq!(after.materials.len(), 3);
    assert_eq!(after.images.len(), 4);
    assert_eq!(&after.images[..3], before.images.as_slice());
    assert_eq!(
        after.images[3],
        Image {
            width: 2,
            height: 2,
            rgba8: COLORS[index].repeat(4)
        }
    );
    let mut expected_materials = before.materials.clone();
    expected_materials[0].image = 3;
    assert_eq!(
        after.materials, expected_materials,
        "all factors, alpha, sampler and unused slots survive"
    );
    let mut users = 0;
    for (old, new) in before.primitives.iter().zip(&after.primitives) {
        let mut expected = old.clone();
        expected.id = format!("{}{}", IDS[index], old.id.strip_prefix(SOURCE_ID).unwrap());
        assert_eq!(new, &expected, "only the recipe identity prefix may change");
        if new.material == 0 {
            users += 1;
            assert_eq!(after.materials[new.material as usize].image, 3);
        }
    }
    assert_eq!(users, 2, "both nodes use the selected material");
    assert_eq!(
        after.dependencies,
        vec![
            Dependency {
                uri: "$source".into(),
                sha256: digest(recipe)
            },
            Dependency {
                uri: SOURCE_PATH.into(),
                sha256: digest(source_bytes)
            },
            Dependency {
                uri: TEXTURES[index].into(),
                sha256: digest(texture)
            },
        ]
    );
}

#[test]
#[ignore = "mandatory hosted consumer requires the pinned current-source CLI"]
fn installed_static_texture_variants_keep_distinct_bindings_and_source_geometry() {
    let scratch = Scratch::new();
    let authoring = scratch.0.join("authoring");
    fs::create_dir_all(authoring.join("inputs")).unwrap();
    fs::create_dir(authoring.join("images")).unwrap();
    let original = source_model();
    let source_bytes = original.to_bytes().unwrap();
    fs::write(authoring.join(SOURCE_PATH), &source_bytes).unwrap();
    let package_source = scratch.0.join("authored-package");
    fs::create_dir_all(package_source.join("variants")).unwrap();
    let mut recipes = Vec::new();
    let mut textures = Vec::new();
    let mut cooked = Vec::new();
    for (index, &asset) in ASSETS.iter().enumerate() {
        let texture = png(COLORS[index]);
        fs::write(authoring.join(TEXTURES[index]), &texture).unwrap();
        let recipe = serde_json::to_vec(&json!({
            "format":"orr_static_material_texture","version":1,
            "source":SOURCE_PATH,"texture":TEXTURES[index],"material":0
        }))
        .unwrap();
        let recipe_path = authoring.join(Path::new(IDS[index]).file_name().unwrap());
        fs::write(&recipe_path, &recipe).unwrap();
        produce(&recipe_path, IDS[index], &package_source.join(asset));
        let bytes = fs::read(package_source.join(asset)).unwrap();
        let variant = StaticModel::from_bytes(&bytes).unwrap();
        assert_variant(&variant, &original, index, &recipe, &source_bytes, &texture);
        assert_eq!(variant.to_bytes().unwrap(), bytes);
        recipes.push(recipe);
        textures.push(texture);
        cooked.push(bytes);
    }
    assert_ne!(digest(&cooked[0]), digest(&cooked[1]));
    let all_ids: BTreeSet<_> = cooked
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
    assert_eq!(
        all_ids.len(),
        8,
        "variant primitive namespaces never collide"
    );
    fs::write(
        package_source.join("orr.package.json"),
        serde_json::to_vec(&json!({
            "schema":1,"name":PACKAGE,"version":"1.0.0","engine":"^0.0.1",
            "capabilities":["models"],"dependencies":{},"files":ASSETS
        }))
        .unwrap(),
    )
    .unwrap();
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
    assert!(!root.join(SOURCE_PATH).exists());
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
        assert_variant(
            loaded.static_model().unwrap(),
            &original,
            index,
            &recipes[index],
            &source_bytes,
            &textures[index],
        );
        let transform = LocalTransform {
            translation: [if index == 0 { -2.0 } else { 2.0 }, 0.5, -0.25],
            scale: [1.0, 2.0, 0.5],
            ..LocalTransform::default()
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
    // Load A, B, then A again to expose accidental name/cache aliasing.
    for index in [0, 1, 0] {
        let binding = &reopened.bindings[&format!("e_{:08x}", index + 1)];
        let loaded = model_bindings::load_binding(&root, binding).unwrap();
        assert_eq!(loaded.source_hash(), digest(&cooked[index]));
        assert_eq!(loaded.package_digest(), locked.digest);
        assert_eq!(
            loaded.static_model().unwrap().to_bytes().unwrap(),
            cooked[index]
        );
    }
    let mut wrong_variant = reopened.bindings["e_00000001"].clone();
    wrong_variant.asset = ASSETS[1].into();
    assert!(model_bindings::load_binding(&root, &wrong_variant).is_err());
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
    fs::write(installed, &cooked[0]).unwrap();
    let restored = model_bindings::load_binding(&root, &reopened.bindings["e_00000001"]).unwrap();
    assert_eq!(
        restored.static_model().unwrap().to_bytes().unwrap(),
        cooked[0]
    );
    assert_eq!(
        fs::read(scratch.0.join("hidden-authoring").join(SOURCE_PATH)).unwrap(),
        source_bytes
    );
}
