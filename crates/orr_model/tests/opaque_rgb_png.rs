#![cfg(feature = "import")]
#[path = "common/opaque_rgb_fixture.rs"]
mod fixture;
use orr_model::{import::import_with_resolver, Error, StaticModel};
use serde_json::json;

fn model(bytes: &[u8]) -> Result<StaticModel, Error> {
    import_with_resolver("opaque.glb", bytes, |uri| {
        panic!("unexpected dependency {uri}")
    })
}
#[cfg(feature = "animation")]
fn animated(bytes: &[u8]) -> Result<orr_model::animation::AnimatedModel, Error> {
    orr_model::animation_import::import_with_resolver("opaque.glb", bytes, |uri| {
        panic!("unexpected dependency {uri}")
    })
}
#[test]
fn static_rgb_literal_pixels_geometry_and_cooked_payload_match_rgba_twin() {
    let rgb = model(&fixture::glb(false, true)).unwrap();
    let rgba = model(&fixture::glb(false, false)).unwrap();
    for image in &rgb.source().images {
        assert_eq!((image.width, image.height), (2, 2));
        assert_eq!(image.rgba8, fixture::RGBA);
    }
    assert_eq!(rgb.source().primitives, rgba.source().primitives);
    assert_eq!(rgb.source().materials, rgba.source().materials);
    assert_eq!(rgb.source().images, rgba.source().images);
    assert_ne!(
        rgb.source().dependencies,
        rgba.source().dependencies,
        "different encoded source bytes must have different provenance"
    );
    let cooked = rgb.to_bytes().unwrap();
    assert_eq!(
        StaticModel::from_bytes(&cooked).unwrap().source(),
        rgb.source()
    );
    let mut comparable = rgba.source().clone();
    comparable.dependencies = rgb.source().dependencies.clone();
    assert_eq!(
        StaticModel::new(comparable).unwrap().to_bytes().unwrap(),
        cooked,
        "cooked payload equality after explicitly aligning only provenance"
    );
}
#[cfg(feature = "animation")]
#[test]
fn animated_rgb_preserves_skin_clips_moving_poses_and_cooked_payload() {
    use orr_model::animation::AnimatedModel;
    let rgb = animated(&fixture::glb(true, true)).unwrap();
    let rgba = animated(&fixture::glb(true, false)).unwrap();
    assert_eq!(rgb.source().primitives, rgba.source().primitives);
    assert_eq!(rgb.source().nodes, rgba.source().nodes);
    assert_eq!(rgb.source().skins, rgba.source().skins);
    assert_eq!(rgb.source().clips, rgba.source().clips);
    assert_eq!(rgb.source().materials, rgba.source().materials);
    assert_eq!(rgb.source().images, rgba.source().images);
    for image in &rgb.source().images {
        assert_eq!(image.rgba8, fixture::RGBA);
    }
    assert_ne!(rgb.source().dependencies, rgba.source().dependencies);
    for clip in 0..rgb.source().clips.len() as u32 {
        for time in [0.0, 0.25, 0.75, 1.5] {
            let a = rgb.sample_clip(clip, time).unwrap();
            let b = rgba.sample_clip(clip, time).unwrap();
            assert_eq!(a.local(), b.local());
            assert_eq!(a.global(), b.global());
            assert_eq!(a.skin_matrices(), b.skin_matrices());
            assert_eq!(rgb.deform(&a).unwrap(), rgba.deform(&b).unwrap());
        }
    }
    assert_ne!(
        rgb.deform(&rgb.sample_clip(0, 0.0).unwrap()).unwrap(),
        rgb.deform(&rgb.sample_clip(0, 0.75).unwrap()).unwrap(),
        "fixture must actually move"
    );
    let cooked = rgb.to_bytes().unwrap();
    assert_eq!(
        AnimatedModel::from_bytes(&cooked).unwrap().source(),
        rgb.source()
    );
    let mut comparable = rgba.source().clone();
    comparable.dependencies = rgb.source().dependencies.clone();
    assert_eq!(
        AnimatedModel::new(comparable).unwrap().to_bytes().unwrap(),
        cooked,
        "cooked payload equality after explicitly aligning only provenance"
    );
}

fn png(
    color: png::ColorType,
    depth: png::BitDepth,
    data: &[u8],
    trns: Option<&[u8]>,
    apng: bool,
) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 2, 1);
        encoder.set_color(color);
        encoder.set_depth(depth);
        if color == png::ColorType::Indexed {
            encoder.set_palette(vec![11, 23, 47, 89, 131, 197]);
        }
        if let Some(alpha) = trns {
            encoder.set_trns(alpha.to_vec());
        }
        if apng {
            encoder.set_animated(1, 0).unwrap();
        }
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(data).unwrap();
        writer.finish().unwrap();
    }
    out
}
fn with_image(is_animated: bool, data: &[u8], unused: bool) -> Vec<u8> {
    let (mut doc, mut bin) = fixture::parts(is_animated, true);
    if unused {
        // Keep every original image valid; append an unreferenced image alone.
        let count = doc["images"].as_array().unwrap().len();
        let mut holder = json!({"images":[{}], "bufferViews":doc["bufferViews"].clone()});
        fixture::replace_images(&mut holder, &mut bin, data);
        doc["bufferViews"] = holder["bufferViews"].clone();
        doc["images"]
            .as_array_mut()
            .unwrap()
            .push(holder["images"][0].clone());
        assert_eq!(doc["images"].as_array().unwrap().len(), count + 1);
    } else {
        fixture::replace_images(&mut doc, &mut bin, data);
    }
    fixture::pack(&doc, &bin)
}
fn reject_both(data: &[u8]) {
    for unused in [false, true] {
        assert!(
            model(&with_image(false, data, unused)).is_err(),
            "static accepted invalid PNG, unused={unused}"
        );
        #[cfg(feature = "animation")]
        assert!(
            animated(&with_image(true, data, unused)).is_err(),
            "animated accepted invalid PNG, unused={unused}"
        );
    }
}
#[test]
fn unsupported_original_formats_remain_rejected_even_in_unused_images() {
    use png::{BitDepth as B, ColorType as C};
    for data in [
        png(C::Rgb, B::Sixteen, &[0; 12], None, false),
        png(C::Grayscale, B::Eight, &[7, 9], None, false),
        png(C::GrayscaleAlpha, B::Eight, &[7, 255, 9, 255], None, false),
        png(C::Indexed, B::Eight, &[0, 1], None, false),
        png(C::Rgb, B::Eight, &[0; 6], None, true),
    ] {
        reject_both(&data);
    }
}
#[test]
fn existing_rgba16_rgb_transparency_and_indexed_alpha_semantics_are_unchanged() {
    use png::{BitDepth as B, ColorType as C};
    let cases = [
        (
            png(
                C::Rgba,
                B::Sixteen,
                &[11, 1, 23, 2, 47, 3, 89, 4, 131, 5, 197, 6, 251, 7, 0, 8],
                None,
                false,
            ),
            vec![11, 23, 47, 89, 131, 197, 251, 0],
        ),
        (
            png(
                C::Rgb,
                B::Eight,
                &[11, 23, 47, 89, 131, 197],
                Some(&[0, 11, 0, 23, 0, 47]),
                false,
            ),
            vec![11, 23, 47, 0, 89, 131, 197, 255],
        ),
        (
            png(
                C::Rgb,
                B::Sixteen,
                &[11, 1, 23, 2, 47, 3, 89, 4, 131, 5, 197, 6],
                Some(&[11, 1, 23, 2, 47, 3]),
                false,
            ),
            vec![11, 23, 47, 0, 89, 131, 197, 255],
        ),
        (
            png(C::Indexed, B::Eight, &[0, 1], Some(&[0, 93]), false),
            vec![11, 23, 47, 0, 89, 131, 197, 93],
        ),
    ];
    for (data, expected) in cases {
        let m = model(&with_image(false, &data, false)).unwrap();
        for image in &m.source().images {
            assert_eq!(image.rgba8, expected);
        }
        #[cfg(feature = "animation")]
        {
            let m = animated(&with_image(true, &data, false)).unwrap();
            for image in &m.source().images {
                assert_eq!(image.rgba8, expected);
            }
        }
    }
}
#[test]
fn truncation_bad_crc_and_malformed_png_reject_in_used_and_unused_slots() {
    let valid = fixture::png(true);
    reject_both(b"not a PNG");
    for len in [8, 29, valid.len() / 2, valid.len() - 1, valid.len() - 12] {
        reject_both(&valid[..len]);
    }
    let mut bad_header = valid.clone();
    bad_header[29] ^= 1; // IHDR CRC.
    reject_both(&bad_header);
    let mut bad_end = valid.clone();
    *bad_end.last_mut().unwrap() ^= 1; // IEND CRC, checked by finish().
    reject_both(&bad_end);
    let mut bad_idat = valid;
    let start = bad_idat
        .windows(4)
        .position(|bytes| bytes == b"IDAT")
        .unwrap();
    bad_idat[start + 4] ^= 1;
    reject_both(&bad_idat);
}
#[test]
fn unreferenced_rgb_image_is_decoded_with_literal_opaque_alpha() {
    let m = model(&with_image(false, &fixture::png(true), true)).unwrap();
    assert!(m.source().images.len() > 1);
    assert_eq!(m.source().images.last().unwrap().rgba8, fixture::RGBA);
    #[cfg(feature = "animation")]
    {
        let m = animated(&with_image(true, &fixture::png(true), true)).unwrap();
        assert!(m.source().images.len() > 1);
        assert_eq!(m.source().images.last().unwrap().rgba8, fixture::RGBA);
    }
}

#[test]
fn aggregate_budget_charges_four_bytes_for_each_rgb_pixel_including_unused_images() {
    // Two images fit; three have <32 MiB RGB samples but >32 MiB final RGBA.
    let mut data = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut data, 2048, 1400);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&vec![17; 2048 * 1400 * 3]).unwrap();
        writer.finish().unwrap();
    }
    for is_animated in [false, true] {
        if is_animated && !cfg!(feature = "animation") {
            continue;
        }
        let (mut doc, mut bin) = fixture::parts(is_animated, true);
        fixture::replace_images(&mut doc, &mut bin, &data);
        let image = doc["images"][0].clone();
        doc["images"] = json!([image.clone(), image.clone()]);
        let bytes = fixture::pack(&doc, &bin);
        if !is_animated {
            assert!(model(&bytes).is_ok());
        }
        #[cfg(feature = "animation")]
        if is_animated {
            assert!(animated(&bytes).is_ok());
        }
        doc["images"].as_array_mut().unwrap().push(image);
        let bytes = fixture::pack(&doc, &bin);
        let error = if !is_animated {
            model(&bytes).unwrap_err()
        } else {
            #[cfg(feature = "animation")]
            {
                animated(&bytes).unwrap_err()
            }
            #[cfg(not(feature = "animation"))]
            {
                unreachable!()
            }
        };
        assert!(
            error.to_string().contains("decoded image budget exceeded"),
            "{error}"
        );
    }
}
