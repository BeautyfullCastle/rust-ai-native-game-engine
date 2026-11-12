#![allow(clippy::disallowed_types)]
use crate::{
    editor::Editor,
    game::EditorGame,
    model::{History, Mode},
    terrain_document::TerrainSource,
    terrain_panel::TerrainPanel,
};
use egui_kittest::{kittest::Queryable, Harness};
use orr_fp::FP;
use orr_model::StaticModel;
use orr_terrain::{Edit, Terrain};
use orr_terrain_view::raw16::Endian;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

// Non-square, asymmetric and sensitive to both bytes. Literal Q48.16 results
// for offset -1 and scale 1 catch stripping, endian guessing and transposition.
const SAMPLES: [u16; 6] = [0, 1, 256, 257, 32769, 65535];
const HEIGHTS: [i64; 6] = [-65536, -65535, -65280, -65279, -32767, 0];
const FLIPPED: [i64; 6] = [-65279, -32767, 0, -65536, -65535, -65280];

fn encoded_png(width: u32, depth: u32, samples: &[u16], gamma: u32) -> Vec<u8> {
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, width, depth);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Sixteen);
        encoder.set_source_gamma(png::ScaledFloat::from_scaled(gamma));
        let mut writer = encoder.write_header().unwrap();
        let raw: Vec<_> = samples
            .iter()
            .flat_map(|sample| sample.to_be_bytes())
            .collect();
        writer.write_image_data(&raw).unwrap();
        writer.finish().unwrap();
    }
    encoded
}
fn unsupported_png(color: png::ColorType, depth: png::BitDepth) -> Vec<u8> {
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, 3, 2);
        encoder.set_color(color);
        encoder.set_depth(depth);
        let mut writer = encoder.write_header().unwrap();
        let channels = match color {
            png::ColorType::Grayscale => 1,
            png::ColorType::GrayscaleAlpha => 2,
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
            png::ColorType::Indexed => unreachable!(),
        };
        let bytes_per_channel = if depth == png::BitDepth::Sixteen {
            2
        } else {
            1
        };
        writer
            .write_image_data(&vec![0; 6 * channels * bytes_per_channel])
            .unwrap();
        writer.finish().unwrap();
    }
    encoded
}
fn settle(editor: &mut Editor) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        editor.sync();
        if editor.yard_rows_coherent()
            && editor
                .snapshot()
                .is_some_and(|s| s.timeline().is_some() == (editor.mode() == Mode::Play))
        {
            return;
        }
        assert!(Instant::now() < deadline, "Yard snapshot not coherent");
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn fixture() -> (tempfile::TempDir, PathBuf, Editor, TerrainPanel) {
    let temp = tempfile::tempdir().unwrap();
    let scene = temp.path().join("yard.yaml");
    fs::write(
        &scene,
        include_str!("../../../../../scenes/yard3d_authoring.scene.yaml"),
    )
    .unwrap();
    let mut editor = Editor::open_game(&scene, EditorGame::Yard3D).unwrap();
    settle(&mut editor);
    let mut panel = TerrainPanel {
        asset_id: "terrain/png16-height.orrt".into(),
        dimensions: [3, 2],
        origin: ["-1.5".into(), "2.25".into()],
        spacing: "0.5".into(),
        ..TerrainPanel::default()
    };
    panel.create_for_editor(&editor).unwrap();
    panel
        .document
        .apply(&[Edit::SetHole {
            x: 1,
            z: 0,
            hole: true,
        }])
        .unwrap();
    panel.save_for_editor(&editor).unwrap();
    panel.open_for_editor(&editor).unwrap();
    let source = temp.path().join("heights.png");
    fs::write(&source, encoded_png(3, 2, &SAMPLES, 45455)).unwrap();
    panel.png16.fields.path = source.to_str().unwrap().into();
    panel.png16.fields.offset = "-1".into();
    panel.png16.fields.scale = "1".into();
    (temp, scene, editor, panel)
}
fn heights(panel: &TerrainPanel) -> Vec<i64> {
    panel
        .document
        .terrain()
        .unwrap()
        .heights()
        .iter()
        .map(|h| h.raw())
        .collect()
}
fn with_heights(original: &Terrain, expected: &[i64]) -> Terrain {
    Terrain::new(
        original.asset_id().to_owned(),
        original.width(),
        original.depth(),
        original.origin(),
        original.spacing(),
        expected.iter().map(|raw| FP::from_raw(*raw)).collect(),
        original.holes().to_vec(),
    )
    .unwrap()
}

struct Snapshot {
    bytes: Vec<u8>,
    revision: Option<[u8; 32]>,
    dirty: bool,
    undo: bool,
    redo: bool,
    model: Option<Arc<StaticModel>>,
    source: Option<TerrainSource>,
    terrain_file: Vec<u8>,
    png_file: Option<Vec<u8>>,
    scene_file: Vec<u8>,
    history: History,
    checksum: u64,
}
impl Snapshot {
    fn take(temp: &Path, scene: &Path, panel: &TerrainPanel, editor: &Editor) -> Self {
        Self {
            bytes: panel.document.bytes().unwrap().to_vec(),
            revision: panel.document.revision(),
            dirty: panel.document.dirty(),
            undo: panel.document.can_undo(),
            redo: panel.document.can_redo(),
            model: panel.document.model().cloned(),
            source: panel.document.source().cloned(),
            terrain_file: fs::read(temp.join("terrain.orrt")).unwrap(),
            png_file: fs::read(temp.join("heights.png")).ok(),
            scene_file: fs::read(scene).unwrap(),
            history: editor.history().clone(),
            checksum: editor.checksum(),
        }
    }
    fn assert_unchanged(&self, temp: &Path, scene: &Path, panel: &TerrainPanel, editor: &Editor) {
        assert_eq!(panel.document.bytes(), Some(self.bytes.as_slice()));
        assert_eq!(panel.document.revision(), self.revision);
        assert_eq!(panel.document.dirty(), self.dirty);
        assert_eq!(panel.document.can_undo(), self.undo);
        assert_eq!(panel.document.can_redo(), self.redo);
        assert_eq!(panel.document.source(), self.source.as_ref());
        match (&self.model, panel.document.model()) {
            (Some(before), Some(after)) => assert!(Arc::ptr_eq(before, after)),
            (None, None) => {}
            _ => panic!("model presence changed"),
        }
        assert_eq!(
            fs::read(temp.join("terrain.orrt")).unwrap(),
            self.terrain_file
        );
        assert_eq!(fs::read(temp.join("heights.png")).ok(), self.png_file);
        assert_eq!(fs::read(scene).unwrap(), self.scene_file);
        assert_eq!(editor.history(), &self.history);
        assert_eq!(editor.checksum(), self.checksum);
    }
}

#[test]
fn actual_preview_apply_single_undo_redo_save_reopen_preserves_metadata_holes_and_source() {
    let (temp, scene, editor, panel) = fixture();
    let original = panel.document.bytes().unwrap().to_vec();
    let original_terrain = panel.document.terrain().unwrap().clone();
    let before = Snapshot::take(temp.path(), &scene, &panel, &editor);
    let expected = with_heights(&original_terrain, &HEIGHTS);
    let mut h = Harness::builder().with_size([1400., 3400.]).build_ui_state(
        |ui, state: &mut (TerrainPanel, Editor)| state.0.show_in_parent_scroll(ui, &state.1),
        (panel, editor),
    );
    h.run_steps(2);
    h.get_by_label("Terrain authoring").click();
    h.run_steps(3);
    h.get_by_label("PNG16 height replacement").click();
    h.run_steps(3);
    h.get_by_label("Preview PNG16").click();
    h.run_steps(3);
    assert!(h.state().0.png16.has_preview(), "{:?}", h.state().0.error());
    before.assert_unchanged(temp.path(), &scene, &h.state().0, &h.state().1);
    h.get_by_label("Apply PNG16 heights").click();
    h.run_steps(3);
    let (panel, editor) = h.state();
    assert!(panel.error().is_none(), "{:?}", panel.error());
    assert!(!panel.png16.has_preview());
    assert_eq!(heights(panel), HEIGHTS);
    assert_eq!(panel.document.terrain().unwrap().cook(), expected.cook());
    assert_eq!(panel.document.source(), before.source.as_ref());
    assert!(panel.document.dirty());
    let changed = panel.document.bytes().unwrap().to_vec();
    assert_eq!(
        fs::read(temp.path().join("terrain.orrt")).unwrap(),
        original
    );
    assert_eq!(
        fs::read(temp.path().join("heights.png")).ok(),
        before.png_file
    );
    assert_eq!(editor.history(), &before.history);
    assert_eq!(editor.checksum(), before.checksum);
    let model = panel.viewport_model(editor).unwrap();
    let ys: Vec<_> = model
        .source()
        .primitives
        .iter()
        .flat_map(|p| &p.vertices)
        .map(|v| v.position[1])
        .collect();
    assert!(!ys.is_empty());
    assert!(ys.contains(&-1.0));
    assert!(ys.contains(&(-65535.0 / 65536.0)));
    assert!(ys
        .iter()
        .all(|y| HEIGHTS.iter().any(|raw| *y == *raw as f32 / 65536.0)));
    h.get_by_label("Terrain Undo").click();
    h.run_steps(3);
    assert_eq!(h.state().0.document.bytes(), Some(original.as_slice()));
    assert!(!h.state().0.document.can_undo());
    h.get_by_label("Terrain Redo").click();
    h.run_steps(3);
    assert_eq!(h.state().0.document.bytes(), Some(changed.as_slice()));
    assert_eq!(
        fs::read(temp.path().join("terrain.orrt")).unwrap(),
        original
    );
    h.get_by_label("Save terrain").click();
    h.run_steps(3);
    h.get_by_label("Open terrain").click();
    h.run_steps(3);
    let (panel, editor) = h.state();
    assert!(panel.error().is_none(), "{:?}", panel.error());
    assert_eq!(heights(panel), HEIGHTS);
    assert!(!panel.document.dirty());
    assert!(!panel.document.can_undo());
    let decoded = Terrain::load(&fs::read(temp.path().join("terrain.orrt")).unwrap()).unwrap();
    assert_eq!(decoded.cook(), expected.cook());
    assert_eq!(
        fs::read(temp.path().join("heights.png")).ok(),
        before.png_file
    );
    assert_eq!(fs::read(scene).unwrap(), before.scene_file);
    assert_eq!(editor.history(), &before.history);
    assert_eq!(editor.checksum(), before.checksum);
}

#[test]
fn literal_big_endian_samples_flip_and_gamma_match_raw_twin_with_exact_metadata() {
    for flip in [false, true] {
        for gamma in [45455, 100000] {
            let (temp, scene, editor, mut panel) = fixture();
            fs::write(
                temp.path().join("heights.png"),
                encoded_png(3, 2, &SAMPLES, gamma),
            )
            .unwrap();
            let raw: Vec<_> = SAMPLES.iter().flat_map(|n| n.to_be_bytes()).collect();
            fs::write(temp.path().join("heights.raw"), raw).unwrap();
            panel.png16.fields.flip_z = flip;
            let expected = if flip { FLIPPED } else { HEIGHTS };
            let original = panel.document.terrain().unwrap().clone();
            let before = Snapshot::take(temp.path(), &scene, &panel, &editor);
            panel.preview_png16(&editor).unwrap();
            before.assert_unchanged(temp.path(), &scene, &panel, &editor);
            panel.apply_png16(&editor).unwrap();
            assert_eq!(heights(&panel), expected);
            assert_eq!(
                panel.document.bytes(),
                Some(with_heights(&original, &expected).cook().as_slice())
            );
            let png_result = panel.document.bytes().unwrap().to_vec();
            assert!(panel.undo(&editor).unwrap());
            panel.raw16.fields.path = temp.path().join("heights.raw").to_str().unwrap().into();
            panel.raw16.fields.offset = "-1".into();
            panel.raw16.fields.scale = "1".into();
            panel.raw16.fields.endian = Endian::Big;
            panel.raw16.fields.flip_z = flip;
            panel.preview_raw16(&editor).unwrap();
            panel.apply_raw16(&editor).unwrap();
            assert_eq!(panel.document.bytes(), Some(png_result.as_slice()));
            assert_eq!(
                fs::read(temp.path().join("heights.png")).ok(),
                before.png_file
            );
        }
    }
}

#[test]
fn failed_preview_revokes_prior_candidate_and_preserves_all_applied_state() {
    for case in [
        "negative-scale",
        "invalid-offset",
        "overflow",
        "long-decimal",
        "relative",
        "traversal",
        "alias",
        "hardlink-alias",
        "missing",
        "directory",
        "truncated",
        "trailing",
        "gray8",
        "rgb16",
        "alpha16",
        "dimensions",
        "symlink",
        "long-path",
    ] {
        let (temp, scene, editor, mut panel) = fixture();
        panel.preview_png16(&editor).unwrap();
        assert!(panel.png16.has_preview());
        let input = temp.path().join("heights.png");
        match case {
            "negative-scale" => panel.png16.fields.scale = "-1".into(),
            "invalid-offset" => panel.png16.fields.offset = "NaN".into(),
            "overflow" => panel.png16.fields.offset = "99999999999999999999999999".into(),
            "long-decimal" => panel.png16.fields.scale = "0".repeat(129),
            "relative" => panel.png16.fields.path = "heights.png".into(),
            "traversal" => {
                panel.png16.fields.path =
                    temp.path().join("../heights.png").to_str().unwrap().into()
            }
            "alias" => {
                panel.png16.fields.path = temp.path().join("terrain.orrt").to_str().unwrap().into()
            }
            "hardlink-alias" => {
                let link = temp.path().join("alias.png");
                fs::hard_link(temp.path().join("terrain.orrt"), &link).unwrap();
                panel.png16.fields.path = link.to_str().unwrap().into();
            }
            "missing" => {
                panel.png16.fields.path = temp.path().join("missing.png").to_str().unwrap().into()
            }
            "directory" => panel.png16.fields.path = temp.path().to_str().unwrap().into(),
            "truncated" => {
                let mut bytes = fs::read(&input).unwrap();
                bytes.truncate(bytes.len() - 7);
                fs::write(&input, bytes).unwrap();
            }
            "trailing" => {
                let mut bytes = fs::read(&input).unwrap();
                bytes.push(0);
                fs::write(&input, bytes).unwrap();
            }
            "gray8" => fs::write(
                &input,
                unsupported_png(png::ColorType::Grayscale, png::BitDepth::Eight),
            )
            .unwrap(),
            "rgb16" => fs::write(
                &input,
                unsupported_png(png::ColorType::Rgb, png::BitDepth::Sixteen),
            )
            .unwrap(),
            "alpha16" => fs::write(
                &input,
                unsupported_png(png::ColorType::GrayscaleAlpha, png::BitDepth::Sixteen),
            )
            .unwrap(),
            "dimensions" => fs::write(&input, encoded_png(2, 3, &SAMPLES, 45455)).unwrap(),
            "symlink" => {
                #[cfg(unix)]
                {
                    let link = temp.path().join("link.png");
                    std::os::unix::fs::symlink(&input, &link).unwrap();
                    panel.png16.fields.path = link.to_str().unwrap().into();
                }
                #[cfg(not(unix))]
                {
                    continue;
                }
            }
            "long-path" => panel.png16.fields.path = format!("/{}", "a".repeat(4096)),
            _ => unreachable!(),
        }
        let before = Snapshot::take(temp.path(), &scene, &panel, &editor);
        assert!(panel.preview_png16(&editor).is_err(), "{case}");
        assert!(!panel.png16.has_preview(), "{case}");
        assert!(panel.apply_png16(&editor).is_err(), "{case}");
        before.assert_unchanged(temp.path(), &scene, &panel, &editor);
    }
}

#[test]
fn changed_encoded_metadata_settings_source_terrain_and_renderer_reject_atomically() {
    for case in [
        "samples", "metadata", "offset", "scale", "flip", "path", "source", "terrain", "renderer",
        "missing",
    ] {
        let (temp, scene, editor, mut panel) = fixture();
        panel.preview_png16(&editor).unwrap();
        let input = temp.path().join("heights.png");
        match case {
            "samples" => fs::write(
                &input,
                encoded_png(3, 2, &[1, 1, 256, 257, 32769, 65535], 45455),
            )
            .unwrap(),
            "metadata" => {
                let different = encoded_png(3, 2, &SAMPLES, 100000);
                assert_ne!(fs::read(&input).unwrap(), different);
                // Pixels remain identical; Apply still pins the encoded snapshot.
                let decoded = orr_terrain_view::png16::import_png16(
                    &different,
                    &panel.png16_recipe().unwrap(),
                )
                .unwrap();
                assert_eq!(
                    decoded
                        .heights()
                        .iter()
                        .map(|h| h.raw())
                        .collect::<Vec<_>>(),
                    HEIGHTS
                );
                fs::write(&input, different).unwrap();
            }
            "offset" => panel.png16.fields.offset = "0".into(),
            "scale" => panel.png16.fields.scale = "0".into(),
            "flip" => panel.png16.fields.flip_z = true,
            "path" => {
                let second = temp.path().join("second.png");
                fs::copy(&input, &second).unwrap();
                panel.png16.fields.path = second.to_str().unwrap().into();
            }
            "source" => {
                fs::copy(
                    temp.path().join("terrain.orrt"),
                    temp.path().join("other.orrt"),
                )
                .unwrap();
                panel.local_path = "other.orrt".into();
                panel.open_for_editor(&editor).unwrap();
            }
            "terrain" => panel
                .document
                .apply(&[Edit::SetHeight {
                    x: 2,
                    z: 1,
                    height: FP::ONE,
                }])
                .unwrap(),
            "renderer" => panel
                .document
                .set_render_admission_error(Some("injected PNG16 render failure".into())),
            "missing" => fs::remove_file(input).unwrap(),
            _ => unreachable!(),
        }
        let before = Snapshot::take(temp.path(), &scene, &panel, &editor);
        assert!(panel.apply_png16(&editor).is_err(), "{case}");
        assert!(!panel.png16.has_preview());
        assert!(panel.apply_png16(&editor).is_err(), "consumed {case}");
        before.assert_unchanged(temp.path(), &scene, &panel, &editor);
    }
}

#[test]
fn undo_to_identical_terrain_allows_preview_and_noop_keeps_redo_and_model() {
    let (temp, scene, editor, mut panel) = fixture();
    panel.preview_png16(&editor).unwrap();
    panel
        .document
        .apply(&[Edit::SetHeight {
            x: 2,
            z: 1,
            height: FP::ONE,
        }])
        .unwrap();
    assert!(panel.undo(&editor).unwrap());
    panel.apply_png16(&editor).unwrap();
    assert_eq!(heights(&panel), HEIGHTS);
    panel
        .document
        .apply(&[Edit::SetHeight {
            x: 2,
            z: 1,
            height: FP::ONE,
        }])
        .unwrap();
    assert!(panel.undo(&editor).unwrap());
    assert!(panel.document.can_redo());
    let before = Snapshot::take(temp.path(), &scene, &panel, &editor);
    panel.preview_png16(&editor).unwrap();
    panel.apply_png16(&editor).unwrap();
    assert!(!panel.png16.has_preview());
    before.assert_unchanged(temp.path(), &scene, &panel, &editor);
    assert!(panel.redo(&editor).unwrap());
    assert_eq!(heights(&panel)[5], FP::ONE.raw());
}

#[test]
fn play_scene_epoch_readonly_and_active_gesture_reject_without_document_changes() {
    for case in [
        "play",
        "same-scene-epoch",
        "different-scene",
        "readonly",
        "gesture",
    ] {
        let (temp, scene, mut editor, mut panel) = fixture();
        panel.preview_png16(&editor).unwrap();
        match case {
            "play" => {
                assert!(editor.start_play());
                settle(&mut editor);
            }
            "same-scene-epoch" => {
                let old = editor.scene_load_epoch();
                assert!(editor.open_path(&scene));
                settle(&mut editor);
                assert_ne!(editor.scene_load_epoch(), old);
                assert!(panel.scene_matches(&editor));
            }
            "different-scene" => {
                let next = temp.path().join("next.yaml");
                fs::copy(&scene, &next).unwrap();
                assert!(editor.open_path(&next));
                settle(&mut editor);
            }
            "readonly" => {
                use orr_terrain_view::package::{install_and_reload, ASSET_PATH, PACKAGE_NAME};
                let project = temp.path().join("project");
                install_and_reload(
                    panel.document.terrain().unwrap(),
                    &project,
                    &temp.path().join("package-source"),
                    "1.0.0",
                )
                .unwrap();
                panel
                    .document
                    .open_package(&project, PACKAGE_NAME, ASSET_PATH)
                    .unwrap();
                assert!(panel.document.read_only());
            }
            "gesture" => {
                editor.begin_edit("PNG16 input guard");
                assert!(editor.in_gesture());
            }
            _ => unreachable!(),
        }
        let before = Snapshot::take(temp.path(), &scene, &panel, &editor);
        assert!(panel.apply_png16(&editor).is_err(), "{case}");
        assert!(!panel.png16.has_preview());
        if case != "same-scene-epoch" {
            assert!(panel.preview_png16(&editor).is_err(), "{case}");
            assert!(!panel.png16.has_preview());
        } else {
            // A fresh preview in the same reloaded scene is explicitly allowed.
            panel.preview_png16(&editor).unwrap();
        }
        before.assert_unchanged(temp.path(), &scene, &panel, &editor);
    }
}

#[test]
fn full_footprint_budget_accepts_4096_and_rejects_larger_even_noops_before_decode() {
    let (temp, _scene, editor, mut panel) = fixture();
    for side in [64, 65] {
        panel.dimensions = [side, side];
        panel.local_path = format!("terrain-{side}.orrt");
        panel.create_for_editor(&editor).unwrap();
        panel.save_for_editor(&editor).unwrap();
        panel.open_for_editor(&editor).unwrap();
        fs::write(
            temp.path().join("heights.png"),
            encoded_png(side, side, &vec![0; (side * side) as usize], 45455),
        )
        .unwrap();
        panel.png16.fields.offset = "0".into();
        panel.png16.fields.scale = "0".into();
        let before = panel.document.bytes().unwrap().to_vec();
        let rev = panel.document.revision();
        let model = panel.document.model().cloned().unwrap();
        if side == 64 {
            panel.preview_png16(&editor).unwrap();
            panel.apply_png16(&editor).unwrap();
            assert_eq!(panel.document.bytes(), Some(before.as_slice()));
            assert_eq!(panel.document.revision(), rev);
            assert!(Arc::ptr_eq(&model, panel.document.model().unwrap()));
            assert!(!panel.document.can_undo());
            assert!(!panel.document.dirty());
            panel.png16.fields.offset = "1".into();
            panel.preview_png16(&editor).unwrap();
            panel.apply_png16(&editor).unwrap();
            assert!(panel
                .document
                .terrain()
                .unwrap()
                .heights()
                .iter()
                .all(|h| *h == FP::ONE));
            assert!(panel.undo(&editor).unwrap());
        } else {
            let err = panel.preview_png16(&editor).unwrap_err();
            assert!(err.contains("4096"), "{err}");
            // Budget admission must precede reading or decoding an invalid file.
            fs::write(temp.path().join("heights.png"), b"not PNG").unwrap();
            let err = panel.preview_png16(&editor).unwrap_err();
            assert!(err.contains("4096"), "{err}");
            assert!(!panel.png16.has_preview());
        }
        assert_eq!(panel.document.bytes(), Some(before.as_slice()));
        assert_eq!(panel.document.revision(), rev);
        assert!(!panel.document.can_undo());
        assert!(!panel.document.dirty());
        assert_eq!(
            fs::read(temp.path().join(format!("terrain-{side}.orrt"))).unwrap(),
            before
        );
    }
}

#[test]
fn dirty_terrain_is_allowed_and_one_undo_restores_the_entire_dirty_state() {
    let (temp, scene, editor, mut panel) = fixture();
    panel
        .document
        .apply(&[Edit::SetHeight {
            x: 0,
            z: 0,
            height: FP::from_int(3),
        }])
        .unwrap();
    assert!(panel.document.dirty());
    let before = Snapshot::take(temp.path(), &scene, &panel, &editor);
    let original = panel.document.terrain().unwrap().clone();
    panel.preview_png16(&editor).unwrap();
    before.assert_unchanged(temp.path(), &scene, &panel, &editor);
    panel.apply_png16(&editor).unwrap();
    assert_eq!(
        panel.document.bytes(),
        Some(with_heights(&original, &HEIGHTS).cook().as_slice())
    );
    assert_eq!(
        fs::read(temp.path().join("terrain.orrt")).unwrap(),
        before.terrain_file
    );
    assert!(panel.undo(&editor).unwrap());
    assert_eq!(panel.document.bytes(), Some(before.bytes.as_slice()));
    assert!(panel.document.dirty());
    assert!(panel.document.can_undo());
    assert!(panel.undo(&editor).unwrap());
    assert!(!panel.document.can_undo());
    assert!(!panel.document.dirty());
    assert_eq!(
        fs::read(temp.path().join("heights.png")).ok(),
        before.png_file
    );
}

#[test]
fn png_settings_and_candidate_are_separate_from_raw16_and_ui_edits_revoke_png() {
    let (temp, _scene, editor, mut panel) = fixture();
    let raw: Vec<_> = SAMPLES.iter().flat_map(|n| n.to_le_bytes()).collect();
    fs::write(temp.path().join("heights.raw"), raw).unwrap();
    panel.raw16.fields.path = temp.path().join("heights.raw").to_str().unwrap().into();
    panel.raw16.fields.offset = "-1".into();
    panel.raw16.fields.scale = "1".into();
    panel.raw16.fields.endian = Endian::Little;
    panel.raw16.output_path = temp.path().join("export.raw").to_str().unwrap().into();
    panel.preview_raw16(&editor).unwrap();
    panel.preview_png16(&editor).unwrap();
    let raw_fields = panel.raw16.fields.clone();
    let raw_output = panel.raw16.output_path.clone();
    let mut h = Harness::builder().with_size([1400., 3400.]).build_ui_state(
        |ui, state: &mut (TerrainPanel, Editor)| state.0.show_in_parent_scroll(ui, &state.1),
        (panel, editor),
    );
    h.run_steps(2);
    h.get_by_label("Terrain authoring").click();
    h.run_steps(3);
    h.get_by_label("PNG16 height replacement").click();
    h.run_steps(3);
    assert!(h.state().0.png16.has_preview());
    h.get_by_label("PNG16 flip Z rows").click();
    h.run_steps(3);
    assert!(!h.state().0.png16.has_preview());
    assert!(h.state().0.raw16.has_preview());
    assert_eq!(h.state().0.raw16.fields, raw_fields);
    assert_eq!(h.state().0.raw16.output_path, raw_output);
    h.get_by_label("Preview PNG16").click();
    h.run_steps(3);
    h.get_by_label("Apply PNG16 heights").click();
    h.run_steps(3);
    assert_eq!(heights(&h.state().0), FLIPPED);
    assert_eq!(h.state().0.raw16.fields, raw_fields);
    assert_eq!(h.state().0.raw16.output_path, raw_output);
    let (panel, editor) = h.state_mut();
    assert!(panel.apply_raw16(editor).is_err()); // terrain content has changed
    assert!(!panel.raw16.has_preview());
}
