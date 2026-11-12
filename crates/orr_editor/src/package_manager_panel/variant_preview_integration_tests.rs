//! Actual import-form preparation worker; no package publication or GPU rendering.
use super::*;
use egui::accesskit::{Action, ActionData, ActionRequest, Role, Toggled};
use egui_kittest::{
    kittest::{NodeT, Queryable},
    Harness,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    time::{Duration, Instant},
};
const SOURCE: &[u8] =
    include_bytes!("../../../../assets/room_character_demo/courier.orrmodel.json");
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn visit(root: &Path, path: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(path).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                visit(root, &p, out);
            } else {
                out.push((p.strip_prefix(root).unwrap().into(), fs::read(p).unwrap()));
            }
        }
    }
    let mut result = Vec::new();
    visit(root, root, &mut result);
    result.sort();
    result
}
fn png() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[19, 37, 71, 103])
            .unwrap();
    }
    bytes
}
#[test]
fn actual_color_and_png_preview_workers_preserve_project_files_frame_and_consent() {
    for texture in [false, true] {
        let f = crate::package_manager::presentation_tests::fixture(true, true);
        let prepared = orr_sample::room_project::PreparedProject::open_with_capabilities(
            &f.root,
            orr_sample::room_project::RoomProjectCapabilities {
                character: true,
                lighting: true,
            },
        )
        .unwrap();
        let frame = prepared.scene().frame().to_bytes();
        let source = f.temp.path().join("preview.animated.json");
        fs::write(&source, SOURCE).unwrap();
        let texture_path = f.temp.path().join("skin.png");
        fs::write(&texture_path, png()).unwrap();
        let initial = files(f.temp.path());
        let mut h = Harness::builder()
            .with_size([1400.0, 1800.0])
            .build_ui_state(
                |ui, state: &mut (crate::model_package_import_panel::Panel, PathBuf, usize)| {
                    if state.0.poll(Some(&state.1), 4, 7).is_some() {
                        state.2 += 1;
                    }
                    state.0.show(ui, true, Some(&state.1), 4, 7);
                },
                (
                    crate::model_package_import_panel::Panel::default(),
                    f.root.clone(),
                    0,
                ),
            );
        h.run_steps(3);
        h.get_by_label("Import a model package").click_accesskit();
        h.run_steps(3);
        h.get_by_label("Cooked animated model").click_accesskit();
        h.run_steps(3);
        h.get_by_role_and_label(
            Role::TextInput,
            "Cooked animated model file (.animated.json)",
        )
        .focus();
        h.run_steps(2);
        h.get_by_role_and_label(
            Role::TextInput,
            "Cooked animated model file (.animated.json)",
        )
        .type_text(source.to_str().unwrap());
        h.run_steps(3);
        h.get_by_label(if texture {
            "Create animated PNG skin"
        } else {
            "Create animated color variant"
        })
        .click_accesskit();
        h.run_steps(3);
        if texture {
            h.get_by_role_and_label(Role::TextInput, "Variant PNG file")
                .focus();
            h.run_steps(2);
            h.get_by_role_and_label(Role::TextInput, "Variant PNG file")
                .type_text(texture_path.to_str().unwrap());
            h.run_steps(3);
        } else {
            for (label, value) in [
                ("Variant linear R (0..1)", 0.2),
                ("Variant linear G (0..1)", 0.7),
                ("Variant linear B (0..1)", 0.4),
            ] {
                let (target_node, target_tree) = h.get_by_label(label).accesskit_node().locate();
                h.event(egui::Event::AccessKitActionRequest(ActionRequest {
                    target_node,
                    target_tree,
                    action: Action::SetValue,
                    data: Some(ActionData::NumericValue(value)),
                }));
                h.run_steps(3);
            }
        }
        assert_eq!(
            h.get_by_label("I have reviewed rights to distribute this content")
                .accesskit_node()
                .toggled(),
            Some(Toggled::False)
        );
        assert!(h
            .get_by_label("Import cooked animated package")
            .accesskit_node()
            .is_disabled());
        h.get_by_label("Preview animated variant").click_accesskit();
        h.run_steps(3);
        let deadline = Instant::now() + Duration::from_secs(10);
        let preview = loop {
            h.run_steps(3);
            if let Some(frame) = h.state_mut().0.preview_frame() {
                break frame;
            }
            assert!(
                Instant::now() < deadline,
                "preview worker timeout: {}",
                h.state().0.status()
            );
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(preview.prepared.source_sha256, hash(SOURCE));
        assert_eq!(
            preview.prepared.asset_sha256,
            hash(&preview.prepared.model.to_bytes().unwrap())
        );
        let model = &preview.prepared.model;
        let old = orr_model::animation::AnimatedModel::from_bytes(SOURCE).unwrap();
        if texture {
            assert_eq!(preview.prepared.png_sha256, Some(hash(&png())));
            assert_eq!(model.source().images.len(), old.source().images.len() + 1);
            assert_eq!(
                model.source().images.last().unwrap().rgba8,
                [19, 37, 71, 103]
            );
            assert_eq!(
                model.source().materials[0].image as usize,
                old.source().images.len()
            );
            assert_eq!(
                model.source().materials[0].base_color,
                old.source().materials[0].base_color
            );
        } else {
            assert!(preview.prepared.png_sha256.is_none());
            assert_eq!(model.source().materials[0].base_color[..3], [0.2, 0.7, 0.4]);
            assert_eq!(
                model.source().materials[0].base_color[3].to_bits(),
                old.source().materials[0].base_color[3].to_bits()
            );
            assert_eq!(model.source().images, old.source().images);
        }
        for clip in 0..old.source().clips.len() as u32 {
            let a = old.sample_clip(clip, 0.25).unwrap();
            let b = model.sample_clip(clip, 0.25).unwrap();
            assert_eq!(a.skin_matrices(), b.skin_matrices());
            let bounds = crate::package_model_preview::PreviewBounds::from_animation(
                model.bounds(&b).unwrap(),
            )
            .unwrap();
            assert!(bounds.radius.is_finite() && bounds.radius > 0.0);
        }
        assert_eq!(
            h.state().2,
            0,
            "preview must never return an import/publication report"
        );
        assert_eq!(files(f.temp.path()), initial);
        assert_eq!(
            h.get_by_label("I have reviewed rights to distribute this content")
                .accesskit_node()
                .toggled(),
            Some(Toggled::False)
        );
        let reopened = orr_sample::room_project::PreparedProject::open_with_capabilities(
            &f.root,
            orr_sample::room_project::RoomProjectCapabilities {
                character: true,
                lighting: true,
            },
        )
        .unwrap();
        assert_eq!(reopened.scene().frame().to_bytes(), frame);
        if texture {
            h.state_mut().0.close_variant_preview();
            assert!(h.state_mut().0.preview_frame().is_none());
        } else {
            h.get_by_role_and_label(
                Role::TextInput,
                "Cooked animated model file (.animated.json)",
            )
            .focus();
            h.run_steps(2);
            h.get_by_role_and_label(
                Role::TextInput,
                "Cooked animated model file (.animated.json)",
            )
            .type_text("changed");
            h.run_steps(3);
            assert!(
                h.state_mut().0.preview_frame().is_none(),
                "changed form must revoke accepted snapshot"
            );
        }
        assert_eq!(files(f.temp.path()), initial);
    }
}
