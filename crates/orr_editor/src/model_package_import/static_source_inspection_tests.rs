use super::*;
use source_inspection::{inspect_static_source, static_catalog_from_bytes};

fn source() -> orr_model::StaticModel {
    let original = orr_model::import::import_with_resolver(
        "props/catalog.glb",
        include_bytes!("../../../orr_model/tests/fixtures/model.glb"),
        |_| panic!("embedded fixture must not resolve dependencies"),
    )
    .unwrap();
    let mut source = original.source().clone();
    source.materials[0].base_color = [-0.0, 0.25, 0.75, 1.0];
    source.materials[0].linear_filter = true;
    source.materials[0].wrap_s = orr_model::Wrap::Mirror;
    source.materials[0].wrap_t = orr_model::Wrap::Repeat;
    source.materials.push(source.materials[0].clone());
    orr_model::StaticModel::new(source).unwrap()
}

#[test]
fn static_catalog_matches_complete_inventory_and_exact_raw_hash_without_clips() {
    let original = source();
    let raw = serde_json::to_vec_pretty(original.source()).unwrap();
    assert_ne!(raw, original.to_bytes().unwrap());
    let catalog = static_catalog_from_bytes(&raw).unwrap();
    assert_eq!(catalog.source_sha256, digest(&raw));
    assert!(catalog.clips.is_empty());
    assert_eq!(catalog.materials.len(), original.source().materials.len());
    for (index, entry) in catalog.materials.iter().enumerate() {
        let material = &original.source().materials[index];
        let image = &original.source().images[material.image as usize];
        assert_eq!(entry.index, index as u32);
        assert_eq!(
            entry.used_primitive_count,
            original
                .source()
                .primitives
                .iter()
                .filter(|primitive| primitive.material == index as u32)
                .count() as u32
        );
        assert_eq!(
            entry.base_color.map(f32::to_bits),
            material.base_color.map(f32::to_bits)
        );
        assert_eq!(entry.image, material.image);
        assert_eq!((entry.width, entry.height), (image.width, image.height));
        assert_eq!(
            (entry.linear_filter, entry.wrap_s, entry.wrap_t),
            (material.linear_filter, material.wrap_s, material.wrap_t)
        );
    }
    assert_eq!(catalog.materials.last().unwrap().used_primitive_count, 0);
}

#[test]
fn static_catalog_rejects_wrong_suffix_before_paths_and_malformed_payload() {
    for path in [
        "missing.glb",
        "missing.animated.json",
        "missing.orrmodel",
        "missing.json",
    ] {
        assert!(inspect_static_source(Path::new(path))
            .err()
            .unwrap()
            .contains(".model.json"));
    }
    assert!(static_catalog_from_bytes(b"bad JSON").is_err());
    let mut value = serde_json::to_value(source().source()).unwrap();
    value["materials"][0]["image"] = serde_json::json!(u32::MAX);
    assert!(static_catalog_from_bytes(&serde_json::to_vec(&value).unwrap()).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn static_catalog_is_readonly_and_rejects_symlink_directory_and_oversized_input() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let raw = serde_json::to_vec_pretty(source().source()).unwrap();
    let path = root.join("source.MODEL.JSON");
    fs::write(&path, &raw).unwrap();
    let before = fs::read_dir(&root).unwrap().count();
    assert_eq!(
        inspect_static_source(&path).unwrap().source_sha256,
        digest(&raw)
    );
    assert_eq!(fs::read(&path).unwrap(), raw);
    assert_eq!(fs::read_dir(&root).unwrap().count(), before);
    let link = root.join("link.model.json");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert!(inspect_static_source(&link).is_err());
    let directory = root.join("directory.model.json");
    fs::create_dir(&directory).unwrap();
    assert!(inspect_static_source(&directory).is_err());
    let large = root.join("large.model.json");
    File::create(&large)
        .unwrap()
        .set_len(MAX_SOURCE_BYTES + 1)
        .unwrap();
    assert!(inspect_static_source(&large)
        .err()
        .unwrap()
        .contains("byte limit"));
}
