#![cfg(feature = "import")]
#![allow(clippy::float_arithmetic)]

#[path = "../examples/static_model_retexture/authoring.rs"]
mod authoring;

use authoring::retexture_model;
use orr_model::{Dependency, Image, StaticModel, Wrap};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs,
    io::Cursor,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

const SOURCE_PATH: &str = "inputs/prop.model.json";
const TEXTURE_PATH: &str = "images/paint.png";
const RECIPE_ID: &str = "variants/blue.retexture.json";
const MAX_RECIPE_BYTES: usize = 16 * 1024;

fn recipe() -> Value {
    json!({
        "format": "orr_static_material_texture",
        "version": 1,
        "source": SOURCE_PATH,
        "texture": TEXTURE_PATH,
        "material": 0,
    })
}

fn recipe_bytes() -> Vec<u8> {
    serde_json::to_vec(&recipe()).unwrap()
}

fn fixture() -> StaticModel {
    orr_model::import::import_with_resolver(
        "props/original.glb",
        include_bytes!("fixtures/model.glb"),
        |_| panic!("embedded fixture must not resolve external resources"),
    )
    .unwrap()
}

fn rich_fixture() -> StaticModel {
    let mut source = fixture().source().clone();
    source.materials[0].base_color = [0.125, 0.5, 0.75, 0.25];
    source.materials[0].linear_filter = true;
    source.materials[0].wrap_s = Wrap::Mirror;
    source.materials[0].wrap_t = Wrap::Repeat;
    // The unselected slot shares the original selected image. It must retain it.
    source.materials[1].image = source.materials[0].image;
    source.materials.push(source.materials[1].clone());
    source.images.push(Image {
        width: 1,
        height: 1,
        rgba8: vec![19, 37, 71, 103],
    });
    let mut shared = source.primitives[0].clone();
    shared.id = format!("{}#node=17/mesh=8/primitive=3", source.asset_id);
    shared.transform = [
        [-2.0, 0.0, 0.0, 0.0],
        [0.0, 3.0, 0.0, 0.0],
        [0.0, 0.0, 0.5, 0.0],
        [4.0, -7.0, 11.0, 1.0],
    ];
    source.primitives.push(shared);
    source.dependencies.push(Dependency {
        uri: "original-materials/paint.png".into(),
        sha256: digest(b"original source metadata remains transitively bound"),
    });
    StaticModel::new(source).unwrap()
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn png_bytes(
    width: u32,
    height: u32,
    color: png::ColorType,
    depth: png::BitDepth,
    data: &[u8],
    animated: bool,
) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(Cursor::new(&mut bytes), width, height);
        encoder.set_color(color);
        encoder.set_depth(depth);
        if animated {
            encoder.set_animated(1, 0).unwrap();
        }
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(data).unwrap();
        writer.finish().unwrap();
    }
    bytes
}

fn paint() -> Vec<u8> {
    png_bytes(
        2,
        2,
        png::ColorType::Rgba,
        png::BitDepth::Eight,
        &[
            255, 0, 32, 0, 7, 129, 251, 64, 29, 41, 59, 128, 0, 255, 1, 255,
        ],
        false,
    )
}

fn indexed_png(alpha: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(Cursor::new(&mut bytes), 2, 1);
        encoder.set_color(png::ColorType::Indexed);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_palette(vec![11, 23, 47, 89, 131, 197]);
        if alpha {
            encoder.set_trns(vec![0, 93]);
        }
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[0, 1]).unwrap();
        writer.finish().unwrap();
    }
    bytes
}

fn apply(recipe: &[u8], id: &str, source: &[u8], png: &[u8]) -> Result<StaticModel, String> {
    retexture_model(recipe, id, |path| match path {
        SOURCE_PATH => Ok(source.to_vec()),
        TEXTURE_PATH => Ok(png.to_vec()),
        _ => panic!("unexpected resolver path: {path}"),
    })
}

fn reject_before_resolution(bytes: &[u8], id: &str) {
    assert!(
        retexture_model(bytes, id, |path| panic!("invalid recipe resolved {path}")).is_err(),
        "accepted recipe {:?} with identity {id:?}",
        String::from_utf8_lossy(bytes),
    );
}

fn imported_image(png: &[u8]) -> Result<Image, orr_model::Error> {
    let model = orr_model::import::import_with_resolver(
        "fixture.gltf",
        include_bytes!("fixtures/model.gltf"),
        |path| match path {
            "model.bin" => Ok(include_bytes!("fixtures/model.bin").to_vec()),
            "quadrants.png" => Ok(png.to_vec()),
            "marker.png" => Ok(include_bytes!("fixtures/marker.png").to_vec()),
            _ => panic!("unexpected importer fixture dependency {path}"),
        },
    )?;
    Ok(model.source().images[0].clone())
}

#[test]
fn recipe_accepts_exact_required_object_and_whitespace() {
    let source = fixture().to_bytes().unwrap();
    let png = paint();
    let compact = recipe_bytes();
    let mut spaced = b" \n\t".to_vec();
    spaced.extend(serde_json::to_vec_pretty(&recipe()).unwrap());
    spaced.extend(b"\n\t ");
    for bytes in [compact, spaced] {
        let result = apply(&bytes, RECIPE_ID, &source, &png).unwrap();
        assert_eq!(result.source().asset_id, RECIPE_ID);
        assert_eq!(result.source().dependencies[0].sha256, digest(&bytes));
    }
    let mut exact_limit = recipe_bytes();
    exact_limit.resize(MAX_RECIPE_BYTES, b' ');
    assert!(apply(&exact_limit, RECIPE_ID, &source, &png).is_ok());
}

#[test]
fn recipe_rejects_unknown_duplicate_missing_null_and_array_fields_before_resolution() {
    let original = recipe();
    for field in ["format", "version", "source", "texture", "material"] {
        let mut missing = original.clone();
        missing.as_object_mut().unwrap().remove(field);
        reject_before_resolution(&serde_json::to_vec(&missing).unwrap(), RECIPE_ID);
        let mut null = original.clone();
        null[field] = Value::Null;
        reject_before_resolution(&serde_json::to_vec(&null).unwrap(), RECIPE_ID);
        let mut duplicate = serde_json::to_string(&original).unwrap();
        duplicate.pop();
        duplicate.push_str(&format!(",{field:?}:{} }}", original[field]));
        reject_before_resolution(duplicate.as_bytes(), RECIPE_ID);
    }
    let mut unknown = original.clone();
    unknown["overwrite"] = json!(false);
    reject_before_resolution(&serde_json::to_vec(&unknown).unwrap(), RECIPE_ID);
    for alias in [
        json!([
            "orr_static_material_texture",
            1,
            SOURCE_PATH,
            TEXTURE_PATH,
            0
        ]),
        json!([["format", "orr_static_material_texture"], ["version", 1]]),
        json!([]),
        json!(null),
        json!(true),
        json!(1),
        json!("recipe"),
    ] {
        reject_before_resolution(&serde_json::to_vec(&alias).unwrap(), RECIPE_ID);
    }
    for bytes in [
        b"".as_slice(),
        b"{}{}",
        b"{",
        b"[]",
        b"\xff",
        b"{} trailing",
    ] {
        reject_before_resolution(bytes, RECIPE_ID);
    }
}

#[test]
fn recipe_rejects_wrong_format_version_types_and_byte_budget_before_resolution() {
    for (field, value) in [
        ("format", json!("orr_static_model")),
        ("format", json!(1)),
        ("version", json!(0)),
        ("version", json!(2)),
        ("version", json!("1")),
        ("version", json!(1.0)),
        ("version", json!(true)),
        ("version", json!(-1)),
        ("version", json!(4294967296u64)),
        ("source", json!([])),
        ("source", json!(3)),
        ("texture", json!({"path": TEXTURE_PATH})),
        ("texture", json!(false)),
        ("material", json!("0")),
        ("material", json!(0.0)),
        ("material", json!(-1)),
        ("material", json!(true)),
        ("material", json!([])),
        ("material", json!(4294967296u64)),
    ] {
        let mut bad = recipe();
        bad[field] = value;
        reject_before_resolution(&serde_json::to_vec(&bad).unwrap(), RECIPE_ID);
    }
    let mut too_large = recipe_bytes();
    too_large.resize(MAX_RECIPE_BYTES + 1, b' ');
    reject_before_resolution(&too_large, RECIPE_ID);
}

#[test]
fn paths_and_identity_are_normalized_portable_and_rejected_before_resolution() {
    for (field, suffix) in [
        ("source", ".model.json"),
        ("texture", ".png"),
        ("asset_id", ".retexture.json"),
    ] {
        let leaf = format!("prop{suffix}");
        let invalid = [
            String::new(),
            format!("/{leaf}"),
            format!("../{leaf}"),
            format!("a/../{leaf}"),
            format!("./{leaf}"),
            format!("a/./{leaf}"),
            format!("a//{leaf}"),
            format!("{leaf}/"),
            format!("a\\{leaf}"),
            format!("C:{leaf}"),
            format!("https://example.test/{leaf}"),
            format!("a%20/{leaf}"),
            format!("a?b/{leaf}"),
            format!("a#b/{leaf}"),
            format!(".hidden/{leaf}"),
            format!("a/.hidden/{leaf}"),
            format!(".{leaf}"),
            format!("a /{leaf}"),
            format!("a./{leaf}"),
            format!("a b/{leaf}"),
            format!("한글/{leaf}"),
            format!("a\0/{leaf}"),
            format!("a\n/{leaf}"),
            format!("CON{suffix}"),
            format!("prn{suffix}"),
            format!("AUX{suffix}"),
            format!("nul{suffix}"),
            format!("COM1{suffix}"),
            format!("lpt9{suffix}"),
            format!("NUL/{leaf}"),
            format!("a/CON.extra/{leaf}"),
            format!("{}{}", "a".repeat(241 - suffix.len()), suffix),
            "prop.wrong".into(),
            format!("prop{}", suffix.to_ascii_uppercase()),
        ];
        for path in invalid {
            let mut bad = recipe();
            let id = if field == "asset_id" {
                path.as_str()
            } else {
                bad[field] = json!(path);
                RECIPE_ID
            };
            reject_before_resolution(&serde_json::to_vec(&bad).unwrap(), id);
        }
    }
    let mut aliases = recipe();
    aliases["texture"] = aliases["source"].clone();
    reject_before_resolution(&serde_json::to_vec(&aliases).unwrap(), RECIPE_ID);
    reject_before_resolution(&recipe_bytes(), SOURCE_PATH);
    reject_before_resolution(&recipe_bytes(), TEXTURE_PATH);
}

#[test]
fn portable_logical_paths_accept_the_240_byte_boundary() {
    let source_path = format!("{}.model.json", "a".repeat(240 - ".model.json".len()));
    let texture_path = format!("{}.png", "b".repeat(240 - ".png".len()));
    let id = format!(
        "{}.retexture.json",
        "c".repeat(240 - ".retexture.json".len())
    );
    let mut input = recipe();
    input["source"] = json!(source_path);
    input["texture"] = json!(texture_path);
    let source = fixture().to_bytes().unwrap();
    let png = paint();
    let mut resolved = Vec::new();
    let result = retexture_model(&serde_json::to_vec(&input).unwrap(), &id, |path| {
        resolved.push(path.to_owned());
        if path == source_path {
            Ok(source.clone())
        } else if path == texture_path {
            Ok(png.clone())
        } else {
            panic!("unexpected path {path}");
        }
    })
    .unwrap();
    assert_eq!(result.source().asset_id, id);
    assert_eq!(resolved, [source_path, texture_path]);
}

#[test]
fn resolution_is_once_source_first_and_failures_are_preserved() {
    let source = fixture().to_bytes().unwrap();
    let png = paint();
    let mut resolved = Vec::new();
    retexture_model(&recipe_bytes(), RECIPE_ID, |path| {
        resolved.push(path.to_owned());
        match resolved.len() {
            1 => {
                assert_eq!(path, SOURCE_PATH);
                Ok(source.clone())
            }
            2 => {
                assert_eq!(path, TEXTURE_PATH);
                Ok(png.clone())
            }
            _ => panic!("dependency resolved more than once"),
        }
    })
    .unwrap();
    assert_eq!(resolved, [SOURCE_PATH, TEXTURE_PATH]);
    for fail_at in [1, 2] {
        let mut calls = 0;
        let error = retexture_model(&recipe_bytes(), RECIPE_ID, |path| {
            calls += 1;
            if calls == fail_at {
                return Err(format!("resolver-denied-{path}"));
            }
            assert_eq!(path, SOURCE_PATH);
            Ok(source.clone())
        })
        .unwrap_err();
        assert_eq!(calls, fail_at);
        assert!(error.contains("resolver-denied-"), "{error}");
    }
}

#[test]
fn source_is_fully_admitted_before_material_selection_or_texture_resolution() {
    let good = fixture();
    let mut invalid = Vec::new();
    invalid.push(b"not a cooked static model".to_vec());
    invalid.push(include_bytes!("fixtures/model.glb").to_vec());
    for (pointer, bad_value) in [
        ("/primitives/1/indices/0", json!(u32::MAX)),
        ("/primitives/1/material", json!(100)),
        ("/primitives/1/transform/0", json!([0, 0, 0, 0])),
        ("/images/1/width", json!(0)),
        ("/materials/1/base_color/0", json!(2)),
        ("/dependencies/0/sha256", json!("not-a-digest")),
        ("/version", json!(2)),
    ] {
        let mut document = serde_json::to_value(good.source()).unwrap();
        *document.pointer_mut(pointer).unwrap() = bad_value;
        invalid.push(serde_json::to_vec(&document).unwrap());
    }
    // A defect in an unused table entry still invalidates the complete source.
    let mut unused = good.source().clone();
    unused.materials.push(unused.materials[0].clone());
    unused.materials.last_mut().unwrap().image = u32::MAX;
    invalid.push(serde_json::to_vec(&unused).unwrap());
    for source in invalid {
        assert!(StaticModel::from_bytes(&source).is_err());
        let mut errors = Vec::new();
        for material in [0, u32::MAX] {
            let mut input = recipe();
            input["material"] = json!(material);
            let mut calls = 0;
            let error = retexture_model(&serde_json::to_vec(&input).unwrap(), RECIPE_ID, |path| {
                calls += 1;
                assert_eq!(
                    path, SOURCE_PATH,
                    "bad cooked source reached texture resolution"
                );
                Ok(source.clone())
            })
            .unwrap_err();
            assert_eq!(calls, 1);
            errors.push(error);
        }
        assert_eq!(
            errors[0], errors[1],
            "selection preceded full source admission"
        );
    }
}

#[test]
fn selection_requires_existing_used_material() {
    let original = rich_fixture();
    let source = original.to_bytes().unwrap();
    assert!(!original.source().primitives.iter().any(|p| p.material == 2));
    for material in [2, 3, u32::MAX] {
        let mut input = recipe();
        input["material"] = json!(material);
        let mut calls = 0;
        assert!(
            retexture_model(&serde_json::to_vec(&input).unwrap(), RECIPE_ID, |path| {
                calls += 1;
                assert_eq!(path, SOURCE_PATH);
                Ok(source.clone())
            })
            .is_err()
        );
        assert_eq!(calls, 1);
    }
    let mut input = recipe();
    input["material"] = json!(1);
    let result = apply(
        &serde_json::to_vec(&input).unwrap(),
        RECIPE_ID,
        &source,
        &paint(),
    )
    .unwrap();
    assert_eq!(result.source().materials[0], original.source().materials[0]);
    assert_eq!(
        result.source().materials[1].image as usize,
        original.source().images.len()
    );
}

#[test]
fn retexture_preserves_geometry_uv_transforms_slots_material_factors_and_shared_use() {
    let original = rich_fixture();
    let source_bytes = original.to_bytes().unwrap();
    let png = paint();
    let input = recipe_bytes();
    let result = apply(&input, RECIPE_ID, &source_bytes, &png).unwrap();
    let before = original.source();
    let after = result.source();
    assert_eq!(after.format, before.format);
    assert_eq!(after.version, before.version);
    assert_eq!(after.asset_id, RECIPE_ID);
    assert_eq!(after.primitives.len(), before.primitives.len());
    for (old, new) in before.primitives.iter().zip(&after.primitives) {
        let suffix = old.id.strip_prefix(&before.asset_id).unwrap();
        assert!(suffix.starts_with("#node="));
        let mut expected = old.clone();
        expected.id = format!("{RECIPE_ID}{suffix}");
        assert_eq!(new, &expected);
        assert_eq!(
            serde_json::to_vec(&new.vertices).unwrap(),
            serde_json::to_vec(&old.vertices).unwrap()
        );
        assert_eq!(
            serde_json::to_vec(&new.transform).unwrap(),
            serde_json::to_vec(&old.transform).unwrap()
        );
    }
    assert_eq!(
        after.primitives.iter().filter(|p| p.material == 0).count(),
        2
    );
    assert!(orr_model::determinant(after.primitives[2].transform) < 0.0);
    let mut expected_materials = before.materials.clone();
    expected_materials[0].image = before.images.len() as u32;
    assert_eq!(after.materials, expected_materials);
    assert_eq!(after.materials[0].base_color[3], 0.25);
    assert_eq!(
        &after.images[..before.images.len()],
        before.images.as_slice()
    );
    assert_eq!(after.images.last().unwrap(), &imported_image(&png).unwrap());
    assert_eq!(original.to_bytes().unwrap(), source_bytes);
}

#[test]
fn append_never_reclaims_unused_images_or_mutates_unselected_materials() {
    let original = rich_fixture();
    // Even an exact duplicate of an existing image is appended, never deduplicated.
    let old_image = &original.source().images[0];
    let png = png_bytes(
        old_image.width,
        old_image.height,
        png::ColorType::Rgba,
        png::BitDepth::Eight,
        &old_image.rgba8,
        false,
    );
    let result = apply(
        &recipe_bytes(),
        RECIPE_ID,
        &original.to_bytes().unwrap(),
        &png,
    )
    .unwrap();
    assert_eq!(
        result.source().images.len(),
        original.source().images.len() + 1
    );
    assert_eq!(
        &result.source().images[..original.source().images.len()],
        original.source().images.as_slice()
    );
    assert_eq!(result.source().images.last().unwrap(), old_image);
    assert_eq!(
        &result.source().materials[1..],
        &original.source().materials[1..]
    );
    assert_eq!(
        result.source().materials[1].image,
        original.source().materials[0].image
    );
}

#[test]
fn dependencies_bind_exact_recipe_cooked_source_and_png_bytes() {
    let original = rich_fixture();
    let mut source = original.to_bytes().unwrap();
    source.extend(b"\n  ");
    let mut input = recipe_bytes();
    input.extend(b"\n");
    let png = paint();
    let result = apply(&input, RECIPE_ID, &source, &png).unwrap();
    let expected = [
        Dependency {
            uri: "$source".into(),
            sha256: digest(&input),
        },
        Dependency {
            uri: SOURCE_PATH.into(),
            sha256: digest(&source),
        },
        Dependency {
            uri: TEXTURE_PATH.into(),
            sha256: digest(&png),
        },
    ];
    assert_eq!(result.source().dependencies, expected);
    assert!(!result
        .source()
        .dependencies
        .iter()
        .any(|d| d.uri == "original-materials/paint.png"));
    assert_ne!(expected[0].sha256, digest(&recipe_bytes()));
    assert_ne!(expected[1].sha256, digest(&original.to_bytes().unwrap()));
}

#[test]
fn cooked_roundtrip_is_byte_deterministic_and_recipe_ids_remain_distinct() {
    let source = rich_fixture().to_bytes().unwrap();
    let input = recipe_bytes();
    let png = paint();
    let first = apply(&input, RECIPE_ID, &source, &png).unwrap();
    let second = apply(&input, RECIPE_ID, &source, &png).unwrap();
    let cooked = first.to_bytes().unwrap();
    assert_eq!(cooked, second.to_bytes().unwrap());
    let reload = StaticModel::from_bytes(&cooked).unwrap();
    assert_eq!(reload.source(), first.source());
    assert_eq!(reload.to_bytes().unwrap(), cooked);
    let other_id = "variants/green.retexture.json";
    let other = apply(&input, other_id, &source, &png).unwrap();
    assert_ne!(first.source().asset_id, other.source().asset_id);
    assert_ne!(cooked, other.to_bytes().unwrap());
    let mut expected = first.source().clone();
    expected.asset_id = other_id.into();
    for primitive in &mut expected.primitives {
        primitive.id = format!(
            "{other_id}{}",
            primitive.id.strip_prefix(RECIPE_ID).unwrap()
        );
    }
    assert_eq!(other.source(), &expected);
}

#[test]
fn image_slot_limit_reserves_one_append_without_replacement() {
    let mut source = fixture().source().clone();
    let tiny = Image {
        width: 1,
        height: 1,
        rgba8: vec![0, 1, 2, 3],
    };
    source
        .images
        .resize(orr_model::MAX_IMAGES - 1, tiny.clone());
    let original = StaticModel::new(source.clone())
        .unwrap()
        .to_bytes()
        .unwrap();
    let result = apply(&recipe_bytes(), RECIPE_ID, &original, &paint()).unwrap();
    assert_eq!(result.source().images.len(), orr_model::MAX_IMAGES);
    assert_eq!(
        result.source().materials[0].image as usize,
        orr_model::MAX_IMAGES - 1
    );
    source.images.push(tiny);
    let full = StaticModel::new(source).unwrap().to_bytes().unwrap();
    assert!(apply(&recipe_bytes(), RECIPE_ID, &full, &paint()).is_err());
}

// Keep the large byte/decode/cook checks together, rather than allowing separate
// tests to allocate several 64-MiB fixtures concurrently on hosted runners.
#[test]
fn recipe_source_png_decoded_append_and_cooked_byte_budgets_are_enforced() {
    let input = recipe_bytes();
    let png = paint();
    {
        let oversized = vec![b' '; orr_model::MAX_FILE_BYTES + 1];
        let mut calls = 0;
        assert!(retexture_model(&input, RECIPE_ID, |path| {
            calls += 1;
            assert_eq!(path, SOURCE_PATH);
            Ok(oversized.clone())
        })
        .is_err());
        assert_eq!(calls, 1);
    }
    {
        let source = fixture().to_bytes().unwrap();
        let oversized = vec![0; orr_model::MAX_FILE_BYTES + 1];
        assert!(apply(&input, RECIPE_ID, &source, &oversized).is_err());
    }
    {
        let mut source = fixture().to_bytes().unwrap();
        source.resize(orr_model::MAX_FILE_BYTES - input.len() - png.len(), b' ');
        assert!(StaticModel::from_bytes(&source).is_ok());
        assert!(apply(&input, RECIPE_ID, &source, &png).is_ok());
        source.push(b' ');
        assert!(apply(&input, RECIPE_ID, &source, &png).is_err());
    }
    {
        let mut source = fixture().source().clone();
        source.images = vec![
            Image {
                width: 2048,
                height: 2048,
                rgba8: vec![0; 2048 * 2048 * 4],
            },
            Image {
                width: 2048,
                height: 2047,
                rgba8: vec![0; 2048 * 2047 * 4],
            },
        ];
        let decoded = source
            .images
            .iter()
            .map(|image| image.rgba8.len())
            .sum::<usize>();
        assert_eq!(decoded, orr_model::MAX_DECODED_BYTES - 8192);
        let cooked = StaticModel::new(source).unwrap().to_bytes().unwrap();
        let too_much = png_bytes(
            2048,
            2,
            png::ColorType::Rgba,
            png::BitDepth::Eight,
            &vec![0; 2048 * 2 * 4],
            false,
        );
        let error = apply(&input, RECIPE_ID, &cooked, &too_much).unwrap_err();
        assert!(
            error.contains("decoded") || error.contains("budget"),
            "{error}"
        );
    }
    {
        let source = fixture().to_bytes().unwrap();
        // Valid decoded size and tiny compressed input, but decimal cooked
        // representation exceeds MAX_FILE_BYTES after appending opaque white.
        let oversized_cook = png_bytes(
            2048,
            2048,
            png::ColorType::Rgba,
            png::BitDepth::Eight,
            &vec![255; 2048 * 2048 * 4],
            false,
        );
        assert!(oversized_cook.len() < orr_model::MAX_FILE_BYTES);
        let error = apply(&input, RECIPE_ID, &source, &oversized_cook).unwrap_err();
        assert!(error.contains("cook") || error.contains("byte"), "{error}");
    }
}

#[test]
fn png_rgba_indexed_alpha_and_16_bit_alpha_match_importer() {
    let source = fixture().to_bytes().unwrap();
    let rgba16 = png_bytes(
        2,
        1,
        png::ColorType::Rgba,
        png::BitDepth::Sixteen,
        &[
            0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0xff, 0xff, 0x80, 0x01, 0x00, 0xff,
            0x01, 0x23,
        ],
        false,
    );
    for png in [paint(), indexed_png(true), rgba16] {
        let expected = imported_image(&png).unwrap();
        let result = apply(&recipe_bytes(), RECIPE_ID, &source, &png).unwrap();
        assert_eq!(result.source().images.last().unwrap(), &expected);
    }
    let indexed = apply(&recipe_bytes(), RECIPE_ID, &source, &indexed_png(true)).unwrap();
    assert_eq!(
        indexed.source().images.last().unwrap().rgba8,
        [11, 23, 47, 0, 89, 131, 197, 93]
    );
}

#[test]
fn png_rgb_grayscale_and_indexed_without_alpha_are_rejected() {
    let source = fixture().to_bytes().unwrap();
    for png in [
        png_bytes(
            1,
            1,
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            &[3, 5, 7],
            false,
        ),
        png_bytes(
            1,
            1,
            png::ColorType::Grayscale,
            png::BitDepth::Eight,
            &[3],
            false,
        ),
        png_bytes(
            1,
            1,
            png::ColorType::GrayscaleAlpha,
            png::BitDepth::Eight,
            &[3, 7],
            false,
        ),
        indexed_png(false),
    ] {
        assert!(imported_image(&png).is_err());
        assert!(apply(&recipe_bytes(), RECIPE_ID, &source, &png).is_err());
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320u32 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

fn with_extent(mut png: Vec<u8>, width: u32, height: u32) -> Vec<u8> {
    assert_eq!(&png[12..16], b"IHDR");
    png[16..20].copy_from_slice(&width.to_be_bytes());
    png[20..24].copy_from_slice(&height.to_be_bytes());
    let crc = crc32(&png[12..29]);
    png[29..33].copy_from_slice(&crc.to_be_bytes());
    png
}

#[test]
fn png_apng_and_out_of_range_extents_are_rejected() {
    let source = fixture().to_bytes().unwrap();
    let animated = png_bytes(
        1,
        1,
        png::ColorType::Rgba,
        png::BitDepth::Eight,
        &[1, 2, 3, 4],
        true,
    );
    let mut cases = vec![animated];
    for (width, height) in [(0, 1), (1, 0), (2049, 1), (1, 2049), (u32::MAX, u32::MAX)] {
        cases.push(with_extent(paint(), width, height));
    }
    for png in cases {
        assert!(imported_image(&png).is_err());
        assert!(apply(&recipe_bytes(), RECIPE_ID, &source, &png).is_err());
    }
    let boundary = png_bytes(
        2048,
        1,
        png::ColorType::Rgba,
        png::BitDepth::Eight,
        &vec![1; 2048 * 4],
        false,
    );
    assert!(apply(&recipe_bytes(), RECIPE_ID, &source, &boundary).is_ok());
}

#[test]
fn png_critical_corruption_rejects_and_ancillary_crc_matches_importer() {
    let source = fixture().to_bytes().unwrap();
    let valid = paint();
    let mut cases = vec![vec![], b"not a PNG".to_vec(), vec![0; 100]];
    for n in [1, 8, 32, valid.len() / 2, valid.len() - 12, valid.len() - 1] {
        cases.push(valid[..n].to_vec());
    }
    let mut bad_iend = valid.clone();
    let last = bad_iend.len() - 1;
    bad_iend[last] ^= 0x80;
    cases.push(bad_iend);
    // The existing decoder discards bad ancillary CRCs. Keep this fixture
    // separate from critical corruption, and prove exact importer parity below.
    let mut bad_trailer = valid[..valid.len() - 12].to_vec();
    bad_trailer.extend(3u32.to_be_bytes());
    bad_trailer.extend(b"tEXt");
    bad_trailer.extend(b"k\0v");
    bad_trailer.extend(0u32.to_be_bytes());
    bad_trailer.extend(&valid[valid.len() - 12..]);
    let mut corrupt_data = valid.clone();
    let mut cursor = 8;
    while &corrupt_data[cursor + 4..cursor + 8] != b"IDAT" {
        let len = u32::from_be_bytes(corrupt_data[cursor..cursor + 4].try_into().unwrap()) as usize;
        cursor += 12 + len;
    }
    let len = u32::from_be_bytes(corrupt_data[cursor..cursor + 4].try_into().unwrap()) as usize;
    assert!(len > 0);
    corrupt_data[cursor + 8] ^= 0xff;
    let crc = crc32(&corrupt_data[cursor + 4..cursor + 8 + len]);
    corrupt_data[cursor + 8 + len..cursor + 12 + len].copy_from_slice(&crc.to_be_bytes());
    cases.push(corrupt_data);
    for (index, png) in cases.into_iter().enumerate() {
        assert!(
            imported_image(&png).is_err(),
            "importer accepted malformed case {index}"
        );
        assert!(
            apply(&recipe_bytes(), RECIPE_ID, &source, &png).is_err(),
            "accepted malformed case {index}"
        );
    }

    // This is an explicit inherited ancillary-metadata policy, not removal of a
    // malformed-image test. Critical IEND CRC, IDAT data and truncations above
    // still reject; both APIs must read through finish. The ignored metadata
    // changes provenance even though the original valid PNG's pixels survive.
    assert_ne!(bad_trailer, valid);
    assert_ne!(digest(&bad_trailer), digest(&valid));
    let original_pixels = imported_image(&valid).unwrap();
    assert_eq!(imported_image(&bad_trailer).unwrap(), original_pixels);
    let input = recipe_bytes();
    let result = apply(&input, RECIPE_ID, &source, &bad_trailer).unwrap();
    assert_eq!(result.source().images.last().unwrap(), &original_pixels);
    assert_eq!(
        result.source().dependencies[2],
        Dependency {
            uri: TEXTURE_PATH.into(),
            sha256: digest(&bad_trailer),
        }
    );
    let mut expected = apply(&input, RECIPE_ID, &source, &valid)
        .unwrap()
        .source()
        .clone();
    expected.dependencies[2].sha256 = digest(&bad_trailer);
    assert_eq!(result.source(), &expected);
}

static SERIAL: AtomicUsize = AtomicUsize::new(0);

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        // macOS may spell its temporary directory through /var. Positive tests
        // use physical ancestry; explicit symlink tests retain their aliases.
        let root = std::env::temp_dir().canonicalize().unwrap();
        assert!(root.is_absolute());
        for ancestor in root.ancestors() {
            let metadata = fs::symlink_metadata(ancestor).unwrap();
            assert!(metadata.is_dir() && !metadata.file_type().is_symlink());
        }
        let path = root.join(format!(
            "orr-static-retexture-test-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct CliFixture {
    temp: Temp,
    input: PathBuf,
    output: PathBuf,
    source: Vec<u8>,
    png: Vec<u8>,
    recipe: Vec<u8>,
}

impl CliFixture {
    fn new() -> Self {
        let temp = Temp::new();
        let input = temp.join("blue.retexture.json");
        let output = temp.join("blue.model.json");
        let source = rich_fixture().to_bytes().unwrap();
        let png = paint();
        let recipe = recipe_bytes();
        populate(&temp.0, &input, &source, &png, &recipe);
        Self {
            temp,
            input,
            output,
            source,
            png,
            recipe,
        }
    }

    fn args(&self) -> Vec<OsString> {
        cli_args(&self.input, &self.output)
    }

    fn assert_unchanged(&self) {
        assert_eq!(fs::read(&self.input).unwrap(), self.recipe);
        assert_eq!(fs::read(self.temp.join(SOURCE_PATH)).unwrap(), self.source);
        assert_eq!(fs::read(self.temp.join(TEXTURE_PATH)).unwrap(), self.png);
    }
}

fn populate(root: &Path, input: &Path, source: &[u8], png: &[u8], recipe: &[u8]) {
    fs::create_dir(root.join("inputs")).unwrap();
    fs::create_dir(root.join("images")).unwrap();
    fs::write(root.join(SOURCE_PATH), source).unwrap();
    fs::write(root.join(TEXTURE_PATH), png).unwrap();
    fs::write(input, recipe).unwrap();
}

fn cli_args(input: &Path, output: &Path) -> Vec<OsString> {
    vec![
        "--recipe".into(),
        input.into(),
        "--asset-id".into(),
        RECIPE_ID.into(),
        "--output".into(),
        output.into(),
    ]
}

fn tree(root: &Path) -> Vec<PathBuf> {
    fn visit(root: &Path, current: &Path, paths: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(current).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            paths.push(path.strip_prefix(root).unwrap().to_owned());
            let metadata = fs::symlink_metadata(&path).unwrap();
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                visit(root, &path, paths);
            }
        }
    }
    let mut paths = Vec::new();
    visit(root, root, &mut paths);
    paths.sort();
    paths
}

#[test]
fn cli_publishes_once_without_overwrite_and_preserves_all_inputs() {
    let fixture = CliFixture::new();
    let before = tree(&fixture.temp.0);
    authoring::run(fixture.args()).unwrap();
    let cooked = fs::read(&fixture.output).unwrap();
    let expected = apply(&fixture.recipe, RECIPE_ID, &fixture.source, &fixture.png).unwrap();
    assert_eq!(cooked, expected.to_bytes().unwrap());
    assert_eq!(
        StaticModel::from_bytes(&cooked).unwrap().source(),
        expected.source()
    );
    fixture.assert_unchanged();
    let after = tree(&fixture.temp.0);
    assert_eq!(after.len(), before.len() + 1);
    assert!(authoring::run(fixture.args()).is_err());
    assert_eq!(fs::read(&fixture.output).unwrap(), cooked);
    assert_eq!(tree(&fixture.temp.0), after);
    // An existing hardlink alias of either source is also never replaced.
    for (name, target) in [
        ("source-alias.model.json", fixture.temp.join(SOURCE_PATH)),
        ("recipe-alias.model.json", fixture.input.clone()),
        ("texture-alias.model.json", fixture.temp.join(TEXTURE_PATH)),
    ] {
        let alias = fixture.temp.join(name);
        fs::hard_link(target, &alias).unwrap();
        let snapshot = fs::read(&alias).unwrap();
        let before = tree(&fixture.temp.0);
        assert!(authoring::run(cli_args(&fixture.input, &alias)).is_err());
        assert_eq!(fs::read(&alias).unwrap(), snapshot);
        assert_eq!(tree(&fixture.temp.0), before);
    }
    fixture.assert_unchanged();
}

#[test]
fn cli_help_and_three_unique_pairs_are_bounded() {
    let fixture = CliFixture::new();
    let before = tree(&fixture.temp.0);
    for option in ["--help", "-h"] {
        authoring::run([OsString::from(option)]).unwrap();
    }
    assert_eq!(tree(&fixture.temp.0), before);
    fixture.assert_unchanged();
    let mut consumed = 0;
    let infinite = std::iter::from_fn(|| {
        consumed += 1;
        assert!(consumed <= 8, "CLI read an unbounded argument stream");
        Some(OsString::from("--unknown"))
    });
    assert!(authoring::run(infinite).is_err());
    assert!(consumed > 6);
    assert_eq!(tree(&fixture.temp.0), before);
    let shuffled = fixture
        .args()
        .chunks(2)
        .rev()
        .flat_map(|pair| pair.iter().cloned())
        .collect::<Vec<_>>();
    authoring::run(shuffled).unwrap();
    assert!(StaticModel::from_bytes(&fs::read(&fixture.output).unwrap()).is_ok());
    fixture.assert_unchanged();
}

#[test]
fn cli_bad_arguments_fail_without_output_or_staging() {
    let fixture = CliFixture::new();
    let good = fixture.args();
    let mut cases = vec![
        vec![],
        good[..5].to_vec(),
        vec!["--help".into(), "extra".into()],
    ];
    for start in [0, 2, 4] {
        let mut missing = good.clone();
        missing.drain(start..start + 2);
        cases.push(missing);
        let mut duplicate = good.clone();
        duplicate.extend_from_slice(&good[start..start + 2]);
        cases.push(duplicate);
        let mut empty = good.clone();
        empty[start + 1] = "".into();
        cases.push(empty);
        let mut long = good.clone();
        long[start + 1] = "x".repeat(4097).into();
        cases.push(long);
    }
    for (index, value) in [
        (0, "--unknown"),
        (0, "--input"),
        (0, "--force"),
        (0, "--stdin"),
        (0, "--install"),
        (0, "--network"),
        (0, "--help"),
        (2, "--recipe"),
        (4, "--asset-id"),
        (3, "../bad.retexture.json"),
        (1, "-"),
    ] {
        let mut bad = good.clone();
        bad[index] = value.into();
        cases.push(bad);
    }
    let before = tree(&fixture.temp.0);
    for bad in cases {
        assert!(authoring::run(bad).is_err());
        assert!(!fixture.output.exists());
        assert_eq!(tree(&fixture.temp.0), before);
        fixture.assert_unchanged();
    }
}

#[test]
fn cli_portable_output_names_and_package_store_rejection() {
    let fixture = CliFixture::new();
    let before = tree(&fixture.temp.0);
    for name in [
        "prop.png",
        "prop.json",
        "prop.MODEL.JSON",
        "prop name.model.json",
        "한글.model.json",
        "CON.model.json",
        "prn.model.json",
        "AUX.model.json",
        "nul.model.json",
        "COM1.model.json",
        "lpt9.model.json",
    ] {
        let output = fixture.temp.join(name);
        assert!(authoring::run(cli_args(&fixture.input, &output)).is_err());
        assert!(!output.exists());
        assert_eq!(tree(&fixture.temp.0), before);
    }
    let long = fixture.temp.join(&("x".repeat(100) + ".model.json"));
    assert!(authoring::run(cli_args(&fixture.input, &long)).is_err());
    assert!(!long.exists());
    for name in [".orr.", ".orr ", "ordinary.", "ordinary "] {
        let output = fixture.temp.join(name).join("prop.model.json");
        let error = authoring::run(cli_args(&fixture.input, &output)).unwrap_err();
        assert!(error.contains("ending in a dot or space"), "{error}");
        assert_eq!(tree(&fixture.temp.0), before);
    }
    // Different parents preserve both spellings on case-insensitive filesystems.
    for (index, name) in [".orr", ".ORR", ".OrR"].into_iter().enumerate() {
        let parent = fixture.temp.join(&format!("store-case-{index}"));
        fs::create_dir(&parent).unwrap();
        let store = parent.join(name);
        fs::create_dir(&store).unwrap();
        let output = store.join("prop.model.json");
        let snapshot = tree(&fixture.temp.0);
        assert!(authoring::run(cli_args(&fixture.input, &output)).is_err());
        assert!(!output.exists());
        assert_eq!(tree(&fixture.temp.0), snapshot);
    }
    let missing_parent = fixture.temp.join("missing").join("prop.model.json");
    assert!(authoring::run(cli_args(&fixture.input, &missing_parent)).is_err());
    let file_parent = fixture.temp.join("file-parent");
    fs::write(&file_parent, b"a file cannot be an ancestor").unwrap();
    assert!(authoring::run(cli_args(
        &fixture.input,
        &file_parent.join("prop.model.json")
    ))
    .is_err());
    assert_eq!(
        fs::read(file_parent).unwrap(),
        b"a file cannot be an ancestor"
    );
    fs::create_dir(&fixture.output).unwrap();
    let snapshot = tree(&fixture.temp.0);
    assert!(authoring::run(fixture.args()).is_err());
    assert_eq!(tree(&fixture.temp.0), snapshot);
    fixture.assert_unchanged();
}

#[test]
fn cli_missing_nonregular_and_oversized_inputs_leave_no_output() {
    let fixture = CliFixture::new();
    for (path, original, limit) in [
        (
            fixture.input.clone(),
            fixture.recipe.as_slice(),
            MAX_RECIPE_BYTES,
        ),
        (
            fixture.temp.join(SOURCE_PATH),
            fixture.source.as_slice(),
            orr_model::MAX_FILE_BYTES,
        ),
        (
            fixture.temp.join(TEXTURE_PATH),
            fixture.png.as_slice(),
            orr_model::MAX_FILE_BYTES,
        ),
    ] {
        fs::remove_file(&path).unwrap();
        let before = tree(&fixture.temp.0);
        assert!(authoring::run(fixture.args()).is_err());
        assert_eq!(tree(&fixture.temp.0), before);
        assert!(!fixture.output.exists());
        // Directory fixtures exercise nonregular rejection without socket or
        // FIFO setup and without opening a potentially blocking input.
        fs::create_dir(&path).unwrap();
        let before = tree(&fixture.temp.0);
        assert!(authoring::run(fixture.args()).is_err());
        assert_eq!(tree(&fixture.temp.0), before);
        assert!(!fixture.output.exists());
        fs::remove_dir(&path).unwrap();
        fs::write(&path, b"malformed").unwrap();
        let before = tree(&fixture.temp.0);
        assert!(authoring::run(fixture.args()).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"malformed");
        assert_eq!(tree(&fixture.temp.0), before);
        let file = fs::File::create(&path).unwrap();
        file.set_len(limit as u64 + 1).unwrap();
        drop(file);
        assert!(authoring::run(fixture.args()).is_err());
        assert_eq!(fs::metadata(&path).unwrap().len(), limit as u64 + 1);
        assert_eq!(tree(&fixture.temp.0), before);
        assert!(!fixture.output.exists());
        fs::write(&path, original).unwrap();
    }
    fixture.assert_unchanged();
}

#[test]
fn cli_resolver_is_anchored_to_recipe_parent() {
    let fixture = CliFixture::new();
    let nested = fixture.temp.join("recipe-parent");
    fs::create_dir(&nested).unwrap();
    let nested_input = nested.join("blue.retexture.json");
    let mut nested_source = rich_fixture().source().clone();
    nested_source.materials[0].base_color = [0.25, 0.5, 1.0, 0.75];
    let nested_source = StaticModel::new(nested_source).unwrap().to_bytes().unwrap();
    populate(
        &nested,
        &nested_input,
        &nested_source,
        &fixture.png,
        &fixture.recipe,
    );
    authoring::run(cli_args(&nested_input, &fixture.output)).unwrap();
    let cooked = fs::read(&fixture.output).unwrap();
    assert_eq!(
        cooked,
        apply(&fixture.recipe, RECIPE_ID, &nested_source, &fixture.png)
            .unwrap()
            .to_bytes()
            .unwrap()
    );
    assert_ne!(
        cooked,
        apply(&fixture.recipe, RECIPE_ID, &fixture.source, &fixture.png)
            .unwrap()
            .to_bytes()
            .unwrap()
    );
    fixture.assert_unchanged();
    fs::remove_file(&fixture.output).unwrap();
    fs::remove_file(nested.join(TEXTURE_PATH)).unwrap();
    let before = tree(&fixture.temp.0);
    assert!(authoring::run(cli_args(&nested_input, &fixture.output)).is_err());
    assert_eq!(tree(&fixture.temp.0), before);
    assert!(
        !fixture.output.exists(),
        "must not fall back to a different input parent"
    );
}

fn raw_parent_path(child: &Path, leaf: &str) -> OsString {
    // PathBuf::join normalizes .. for Windows verbatim roots. Construct the CLI
    // spelling directly, then assert the precondition that traversal is present.
    let mut raw = child.as_os_str().to_os_string();
    raw.push(std::path::MAIN_SEPARATOR_STR);
    raw.push("..");
    raw.push(std::path::MAIN_SEPARATOR_STR);
    raw.push(leaf);
    assert!(Path::new(&raw)
        .components()
        .any(|part| matches!(part, Component::ParentDir)));
    raw
}

#[test]
fn cli_parent_traversal_preserves_raw_spelling_and_never_publishes() {
    let fixture = CliFixture::new();
    let child = fixture.temp.join("traversal-child");
    fs::create_dir(&child).unwrap();
    // Both normalized counterfactuals stay inside this owned fixture, and the
    // normalized source exists, so incidental NotFound cannot make this pass.
    assert!(fixture.input.exists());
    let raw_input = raw_parent_path(&child, "blue.retexture.json");
    let raw_output = raw_parent_path(&child, "blue.model.json");
    for args in [
        cli_args(Path::new(&raw_input), &fixture.output),
        cli_args(&fixture.input, Path::new(&raw_output)),
    ] {
        assert!(Path::new(&args[1])
            .components()
            .chain(Path::new(&args[5]).components())
            .any(|part| matches!(part, Component::ParentDir)));
        let before = tree(&fixture.temp.0);
        let error = authoring::run(args).unwrap_err();
        assert_eq!(error, "empty paths and parent traversal are unsupported");
        assert_eq!(tree(&fixture.temp.0), before);
        assert!(!fixture.output.exists());
    }
    fixture.assert_unchanged();
    assert_eq!(fs::read_dir(child).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn cli_symlink_inputs_ancestors_and_destinations_reject() {
    use std::os::unix::fs::symlink;
    let fixture = CliFixture::new();
    let recipe_alias = fixture.temp.join("linked.retexture.json");
    symlink(&fixture.input, &recipe_alias).unwrap();
    let before = tree(&fixture.temp.0);
    assert!(authoring::run(cli_args(&recipe_alias, &fixture.output)).is_err());
    assert_eq!(tree(&fixture.temp.0), before);
    for relative in [SOURCE_PATH, TEXTURE_PATH] {
        let path = fixture.temp.join(relative);
        let saved = path.with_extension("saved");
        fs::rename(&path, &saved).unwrap();
        symlink(&saved, &path).unwrap();
        let before = tree(&fixture.temp.0);
        assert!(authoring::run(fixture.args()).is_err());
        assert_eq!(tree(&fixture.temp.0), before);
        fs::remove_file(&path).unwrap();
        fs::rename(&saved, &path).unwrap();
    }
    // Symlinked dependency parents cannot be followed even if the leaf is a file.
    for directory in ["inputs", "images"] {
        let path = fixture.temp.join(directory);
        let saved = fixture.temp.join(&format!("{directory}-saved"));
        fs::rename(&path, &saved).unwrap();
        symlink(&saved, &path).unwrap();
        let before = tree(&fixture.temp.0);
        assert!(authoring::run(fixture.args()).is_err());
        assert_eq!(tree(&fixture.temp.0), before);
        fs::remove_file(&path).unwrap();
        fs::rename(&saved, &path).unwrap();
    }
    for target in [fixture.input.clone(), fixture.temp.join("absent")] {
        symlink(target, &fixture.output).unwrap();
        let before = tree(&fixture.temp.0);
        assert!(authoring::run(fixture.args()).is_err());
        assert_eq!(tree(&fixture.temp.0), before);
        assert!(fs::symlink_metadata(&fixture.output)
            .unwrap()
            .file_type()
            .is_symlink());
        fs::remove_file(&fixture.output).unwrap();
    }
    let parent_alias = fixture.temp.join("linked-parent");
    symlink(&fixture.temp.0, &parent_alias).unwrap();
    let before = tree(&fixture.temp.0);
    assert!(authoring::run(cli_args(
        &parent_alias.join("blue.retexture.json"),
        &fixture.output
    ))
    .is_err());
    assert!(authoring::run(cli_args(
        &fixture.input,
        &parent_alias.join("blue.model.json")
    ))
    .is_err());
    assert_eq!(tree(&fixture.temp.0), before);
    assert!(!fixture.output.exists());
    fixture.assert_unchanged();
}

#[cfg(unix)]
#[test]
fn cli_non_utf8_option_names_and_logical_ids_reject() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let fixture = CliFixture::new();
    let before = tree(&fixture.temp.0);
    for (index, expected) in [
        (0, "option names must be UTF-8"),
        (3, "--asset-id must be UTF-8"),
    ] {
        let mut bad = fixture.args();
        bad[index] = OsString::from_vec(b"invalid-\xff.retexture.json".to_vec());
        assert_eq!(bad[index].as_bytes(), b"invalid-\xff.retexture.json");
        assert!(bad[index].to_str().is_none());
        assert_eq!(authoring::run(bad).unwrap_err(), expected);
        assert!(!fixture.output.exists());
        assert_eq!(tree(&fixture.temp.0), before);
    }
    fixture.assert_unchanged();
}

// Arbitrary invalid-UTF8 filenames are supported by the Linux CI filesystem.
// macOS filesystems can reject them before the CLI is invoked; that capability
// is not assumed or reported as passing by the portable logical-ID test above.
#[cfg(target_os = "linux")]
#[test]
fn cli_non_utf8_filesystem_inputs_work() {
    use std::os::unix::ffi::OsStringExt;
    let temp = Temp::new();
    let root = temp.0.join(OsString::from_vec(b"parent-\xff".to_vec()));
    fs::create_dir(&root).unwrap();
    let input = root.join(OsString::from_vec(b"recipe-\xfe.retexture.json".to_vec()));
    let output = root.join("blue.model.json");
    let source = rich_fixture().to_bytes().unwrap();
    let png = paint();
    let recipe = recipe_bytes();
    populate(&root, &input, &source, &png, &recipe);
    assert!(input.to_str().is_none());
    assert!(output.to_str().is_none());
    let before = tree(&temp.0);
    authoring::run(cli_args(&input, &output)).unwrap();
    assert_eq!(
        fs::read(&output).unwrap(),
        apply(&recipe, RECIPE_ID, &source, &png)
            .unwrap()
            .to_bytes()
            .unwrap()
    );
    assert_eq!(fs::read(&input).unwrap(), recipe);
    assert_eq!(fs::read(root.join(SOURCE_PATH)).unwrap(), source);
    assert_eq!(fs::read(root.join(TEXTURE_PATH)).unwrap(), png);
    assert_eq!(tree(&temp.0).len(), before.len() + 1);
}

#[cfg(windows)]
#[test]
fn cli_windows_nonlocal_prefixes_reject() {
    let fixture = CliFixture::new();
    let before = tree(&fixture.temp.0);
    for remote in [
        r"\\server\share\prop.model.json",
        r"\\?\UNC\server\share\prop.model.json",
        r"\\.\pipe\prop.model.json",
    ] {
        let error = authoring::run(cli_args(&fixture.input, Path::new(remote))).unwrap_err();
        assert!(error.contains("nonlocal Windows path prefixes"), "{error}");
        assert_eq!(tree(&fixture.temp.0), before);
    }
    for remote in [
        r"\\server\share\prop.retexture.json",
        r"\\?\UNC\server\share\prop.retexture.json",
        r"\\.\pipe\prop.retexture.json",
    ] {
        let error = authoring::run(cli_args(Path::new(remote), &fixture.output)).unwrap_err();
        assert!(error.contains("nonlocal Windows path prefixes"), "{error}");
        assert_eq!(tree(&fixture.temp.0), before);
    }
    assert!(!fixture.output.exists());
    fixture.assert_unchanged();
}
