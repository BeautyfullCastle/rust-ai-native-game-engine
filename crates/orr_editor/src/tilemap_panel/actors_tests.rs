use super::*;
use crate::{model::Target, sprite_bindings::Orientation};
use orr_sample::project::{PreparedProject, TilemapSupport, UiSupport};

fn fixture() -> (tempfile::TempDir, TilemapPanel, SpritePanel, Editor) {
    fixture_with_unbound(false)
}

fn fixture_with_unbound(unbound: bool) -> (tempfile::TempDir, TilemapPanel, SpritePanel, Editor) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("actor-source");
    orr_sample::project_create::create_tilemap(&orr_sample::project_create::CreateOptions {
        output: root.clone(),
        template: "arena-tilemap-2d-v1".into(),
        seed: "actor-source".into(),
    })
    .unwrap();
    // Admit different per-actor scales/orientations and locomotion so a source
    // replacement must preserve them and Undo must restore locomotion verbatim.
    let path = root.join("arena.sprites.json");
    let mut document = Document::from_bytes(&std::fs::read(&path).unwrap()).unwrap();
    if unbound {
        // Keep the real scene row while admitting only the other actor binding.
        document.bindings.pop_last().unwrap();
    }
    document.version = 3;
    document.camera_follow = document.bindings.keys().next().cloned();
    for (index, binding) in document.bindings.values_mut().enumerate() {
        binding.source = Source::Locomotion {
            idle: "idle".into(),
            walk: "walk".into(),
        };
        binding.units_per_pixel = 0.7 + index as f32 * 0.1;
        binding.orientation = Some(Orientation {
            quarter_turns: (index % 4) as u8,
            flip_x: index & 1 == 0,
            flip_y: true,
        });
    }
    std::fs::write(&path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
    let admitted = crate::project::PreparedProject::open(&root).unwrap();
    let mut editor = Editor::start(&admitted.host_spec()).unwrap();
    editor.sync();
    let parts = PreparedProject::open_with_capabilities(
        &root,
        crate::sprite_bindings::compiled_runtime(),
        UiSupport::Unsupported,
        TilemapSupport::Supported,
    )
    .unwrap()
    .into_presentation_parts();
    let mut panel =
        TilemapPanel::prepare(&parts.root, parts.scene.path(), parts.tilemap.unwrap()).unwrap();
    let mut sprites = SpritePanel::default();
    let ctx = egui::Context::default();
    let prepared = parts.sprites.unwrap();
    sprites.install_prepared(
        &ctx,
        Bindings::from_document(prepared.path, prepared.document),
        prepared.assets,
    );
    sprites.install_tilemap_asset(&ctx, parts.tilemap_asset);
    panel.install_renderer(None, &sprites).unwrap();
    panel.install_editor_identity(&editor).unwrap();
    let guids = sprites
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(guids.len(), if unbound { 1 } else { 2 });
    for guid in guids {
        assert!(editor.toggle_selection(Target::Guid(Guid::parse(&guid).unwrap())));
    }
    assert!(panel.editable(&editor));
    (temp, panel, sprites, editor)
}

fn snapshot(sprites: &SpritePanel) -> (Document, Document, Vec<Document>, Vec<Document>) {
    sprites
        .bindings
        .as_ref()
        .unwrap()
        .tilemap_history_snapshot()
}
fn cache(sprites: &SpritePanel) -> Vec<(AssetKey, String, Vec<u8>, egui::TextureId)> {
    sprites
        .tilemap_assets()
        .into_iter()
        .map(|(key, asset)| {
            let texture = sprites.tilemap_asset(&key).unwrap().1;
            (key, asset.document.to_json().unwrap(), asset.rgba, texture)
        })
        .collect()
}
fn find_png(root: &Path) -> PathBuf {
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() && path.extension().is_some_and(|e| e == "png") {
            return path;
        }
        if path.is_dir() {
            let found = find_png(&path);
            if !found.as_os_str().is_empty() {
                return found;
            }
        }
    }
    PathBuf::new()
}

#[test]
fn tilemap_actor_sources_group_history_preserves_all_other_fields_and_cache() {
    let (_temp, mut panel, mut sprites, editor) = fixture();
    let original = snapshot(&sprites);
    let tiles = panel.history_snapshot();
    let cached = cache(&sprites);
    let checksum = editor.checksum();
    let scene_history = editor.history().entries.len();
    assert!(panel
        .set_actor_source(&editor, &mut sprites, Source::Region(11))
        .unwrap());
    let region = snapshot(&sprites);
    assert_eq!(region.2, vec![original.0.clone()]);
    assert!(same_actor_contract(&original.0, &region.0));
    assert!(region
        .0
        .bindings
        .values()
        .all(|b| b.source == Source::Region(11)));
    assert!(panel
        .set_actor_source(&editor, &mut sprites, Source::Clip("walk".into()))
        .unwrap());
    let clip = snapshot(&sprites);
    assert_eq!(clip.2.len(), 2);
    assert!(same_actor_contract(&original.0, &clip.0));
    assert!(panel.undo_actor_source(&editor, &mut sprites).unwrap());
    let undone = snapshot(&sprites);
    assert_eq!(undone.0, region.0);
    assert!(!panel
        .set_actor_source(&editor, &mut sprites, Source::Region(11))
        .unwrap());
    assert_eq!(snapshot(&sprites), undone); // no-op preserves redo and saved marker
    assert!(panel.redo_actor_source(&editor, &mut sprites).unwrap());
    assert_eq!(snapshot(&sprites), clip);
    assert!(panel.undo_actor_source(&editor, &mut sprites).unwrap());
    assert!(panel.undo_actor_source(&editor, &mut sprites).unwrap());
    assert_eq!(snapshot(&sprites).0, original.0); // Locomotion restored
    assert!(!sprites.bindings.as_ref().unwrap().dirty());
    assert_eq!(cache(&sprites), cached);
    assert_eq!(panel.history_snapshot(), tiles);
    assert_eq!(editor.checksum(), checksum);
    assert_eq!(editor.history().entries.len(), scene_history);
    assert!(panel.compositor.is_none());
}

#[test]
fn tilemap_actor_sources_invalid_not_bound_and_empty_selection_are_atomic() {
    let (_temp, mut panel, mut sprites, mut editor) = fixture_with_unbound(true);
    let before = snapshot(&sprites);
    for source in [
        Source::Region(u32::MAX),
        Source::Clip("missing".into()),
        Source::Locomotion {
            idle: "idle".into(),
            walk: "walk".into(),
        },
    ] {
        assert!(panel
            .set_actor_source(&editor, &mut sprites, source)
            .is_err());
        assert_eq!(snapshot(&sprites), before);
    }
    editor.select(None);
    assert!(panel
        .set_actor_source(&editor, &mut sprites, Source::Region(10))
        .is_err());
    assert_eq!(snapshot(&sprites), before);
    let unbound = editor
        .rows()
        .iter()
        .find(|row| {
            row.guid
                .as_ref()
                .is_some_and(|guid| !before.0.bindings.contains_key(guid.as_str()))
        })
        .unwrap()
        .target();
    editor.select(Some(unbound));
    assert!(panel
        .set_actor_source(&editor, &mut sprites, Source::Region(10))
        .is_err());
    assert_eq!(snapshot(&sprites), before);
    editor.select(Some(Target::Guid(Guid::parse("e_ffffffff").unwrap())));
    assert!(panel
        .set_actor_source(&editor, &mut sprites, Source::Region(10))
        .is_err());
    assert_eq!(snapshot(&sprites), before);
}

#[test]
fn tilemap_actor_sources_reject_history_that_changes_non_source_fields() {
    let (_temp, mut panel, mut sprites, editor) = fixture();
    panel
        .set_actor_source(&editor, &mut sprites, Source::Region(11))
        .unwrap();
    let mut bad = sprites
        .bindings
        .as_ref()
        .unwrap()
        .document()
        .bindings
        .values()
        .next()
        .unwrap()
        .clone();
    bad.units_per_pixel = 2.;
    sprites
        .bindings
        .as_mut()
        .unwrap()
        .assign(editor.selected_guids(), Some(bad))
        .unwrap();
    sprites.bindings.as_mut().unwrap().undo();
    let before = snapshot(&sprites);
    assert!(panel
        .redo_actor_source(&editor, &mut sprites)
        .unwrap_err()
        .contains("only Source"));
    assert_eq!(snapshot(&sprites), before);
    // A contaminated old document must also never enter through Undo.
    let guid = editor.selected_guids()[0].clone();
    let mut old = before.0.clone();
    old.bindings.get_mut(guid.as_str()).unwrap().units_per_pixel = 2.;
    let path = sprites.bindings.as_ref().unwrap().path.clone();
    let mut history = Bindings::from_document(path, old);
    history
        .assign(
            std::slice::from_ref(&guid),
            Some(before.0.bindings[guid.as_str()].clone()),
        )
        .unwrap();
    sprites.bindings = Some(history);
    let before = snapshot(&sprites);
    assert!(panel
        .undo_actor_source(&editor, &mut sprites)
        .unwrap_err()
        .contains("only Source"));
    assert_eq!(snapshot(&sprites), before);
}

#[test]
fn tilemap_actor_history_is_bounded_and_noop_preserves_redo() {
    let (_temp, _panel, mut sprites, editor) = fixture();
    let bindings = sprites.bindings.as_mut().unwrap();
    for index in 0..260 {
        bindings
            .set_existing_sources(editor.selected_guids(), Source::Region(10 + index % 2))
            .unwrap();
    }
    assert_eq!(bindings.tilemap_history_snapshot().2.len(), 128);
    for _ in 0..128 {
        bindings.undo();
    }
    assert_eq!(bindings.tilemap_history_snapshot().3.len(), 128);
    let before = bindings.tilemap_history_snapshot();
    let source = before.0.bindings.values().next().unwrap().source.clone();
    bindings
        .set_existing_sources(editor.selected_guids(), source)
        .unwrap();
    assert_eq!(bindings.tilemap_history_snapshot(), before);
    for _ in 0..128 {
        bindings.redo();
    }
    assert_eq!(bindings.tilemap_history_snapshot().2.len(), 128);
}

#[test]
fn tilemap_actor_sources_and_save_reject_external_files_without_adopting_stamps() {
    for input in ["actor", "lock", "manifest", "tilemap", "png"] {
        let (_temp, mut panel, mut sprites, editor) = fixture();
        panel
            .set_actor_source(&editor, &mut sprites, Source::Region(11))
            .unwrap();
        let before = snapshot(&sprites);
        let stamps = (
            panel.stamp.manifest.clone(),
            panel.stamp.lock.clone(),
            panel.stamp.sidecar.clone(),
            panel.stamp.actors.clone(),
        );
        let path = match input {
            "actor" => panel.stamp.actors.as_ref().unwrap().0.clone(),
            "lock" => panel.stamp.root.join("orr.packages.lock.json"),
            "manifest" => panel.stamp.root.join("orr.project.json"),
            "tilemap" => panel.path.clone(),
            _ => find_png(&panel.stamp.root),
        };
        assert!(!path.as_os_str().is_empty());
        let mut changed = std::fs::read(&path).unwrap();
        changed.push(b' ');
        std::fs::write(&path, &changed).unwrap();
        let cached = cache(&sprites);
        assert!(
            panel
                .set_actor_source(&editor, &mut sprites, Source::Region(20))
                .is_err(),
            "{input}"
        );
        assert!(
            panel.undo_actor_source(&editor, &mut sprites).is_err(),
            "{input}"
        );
        assert!(
            panel.redo_actor_source(&editor, &mut sprites).is_err(),
            "{input}"
        );
        assert!(panel.save_actors(&editor, &mut sprites).is_err(), "{input}");
        assert_eq!(snapshot(&sprites), before, "{input}");
        assert_eq!(
            (
                panel.stamp.manifest.clone(),
                panel.stamp.lock.clone(),
                panel.stamp.sidecar.clone(),
                panel.stamp.actors.clone()
            ),
            stamps,
            "{input}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), changed, "{input}");
        assert_eq!(cache(&sprites), cached, "{input}");
    }
}

#[test]
fn tilemap_actor_save_updates_only_actor_stamp_and_allows_later_tile_save() {
    let (_temp, mut panel, mut sprites, editor) = fixture();
    let original = snapshot(&sprites);
    let stamps = (
        panel.stamp.manifest.clone(),
        panel.stamp.lock.clone(),
        panel.stamp.sidecar.clone(),
    );
    let cached = cache(&sprites);
    panel
        .set_actor_source(&editor, &mut sprites, Source::Clip("walk".into()))
        .unwrap();
    let dirty = snapshot(&sprites);
    panel.save_actors(&editor, &mut sprites).unwrap();
    let saved = snapshot(&sprites);
    assert_eq!(saved.0, dirty.0);
    assert_eq!(saved.1, dirty.0);
    assert_eq!((saved.2, saved.3), (dirty.2, dirty.3));
    assert_eq!(
        (
            panel.stamp.manifest.clone(),
            panel.stamp.lock.clone(),
            panel.stamp.sidecar.clone()
        ),
        stamps
    );
    let (path, bytes) = panel.stamp.actors.as_ref().unwrap();
    assert_eq!(&std::fs::read(path).unwrap(), bytes);
    assert_eq!(Document::from_bytes(bytes).unwrap(), saved.0);
    assert!(same_actor_contract(&original.0, &saved.0));
    assert!(!sprites.bindings.as_ref().unwrap().dirty());
    assert!(panel
        .change(CellCoord { column: 0, row: 0 }, Some(20), &sprites)
        .unwrap());
    panel.save(&editor, &sprites).unwrap();
    panel.check_stamp().unwrap();
    assert!(!panel.dirty());
    assert_eq!(cache(&sprites), cached);
    // Saved-marker semantics remain whole-document: Undo is dirty, Redo clean.
    assert!(panel.undo_actor_source(&editor, &mut sprites).unwrap());
    assert!(sprites.bindings.as_ref().unwrap().dirty());
    assert!(panel.redo_actor_source(&editor, &mut sprites).unwrap());
    assert!(!sprites.bindings.as_ref().unwrap().dirty());
    PreparedProject::open_with_capabilities(
        &panel.stamp.root,
        crate::sprite_bindings::compiled_runtime(),
        UiSupport::Unsupported,
        TilemapSupport::Supported,
    )
    .unwrap();
}

#[test]
fn tilemap_actor_sources_reject_stale_epoch_and_play_without_mutation() {
    let (_temp, mut panel, mut sprites, mut editor) = fixture();
    panel
        .set_actor_source(&editor, &mut sprites, Source::Region(11))
        .unwrap();
    let before = snapshot(&sprites);
    let epoch = panel.editor_epoch.take();
    assert!(panel
        .set_actor_source(&editor, &mut sprites, Source::Region(20))
        .is_err());
    assert!(panel.undo_actor_source(&editor, &mut sprites).is_err());
    assert!(panel.redo_actor_source(&editor, &mut sprites).is_err());
    assert!(panel.save_actors(&editor, &mut sprites).is_err());
    assert_eq!(snapshot(&sprites), before);
    panel.editor_epoch = epoch;
    editor.play();
    editor.sync();
    assert_eq!(editor.mode(), Mode::Play);
    assert!(panel
        .set_actor_source(&editor, &mut sprites, Source::Region(20))
        .is_err());
    assert!(panel.undo_actor_source(&editor, &mut sprites).is_err());
    assert!(panel.redo_actor_source(&editor, &mut sprites).is_err());
    assert!(panel.save_actors(&editor, &mut sprites).is_err());
    assert_eq!(snapshot(&sprites), before);
}

#[test]
fn tilemap_actor_sources_reject_symlink_sidecar_and_generic_sprite_mutations() {
    let (_temp, mut panel, mut sprites, editor) = fixture();
    let ctx = egui::Context::default();
    let before = snapshot(&sprites);
    assert!(sprites.assign(&ctx, &editor).is_err());
    assert!(sprites.undo(&ctx).is_err());
    assert!(sprites.redo(&ctx).is_err());
    assert!(sprites.reload(&ctx).is_err());
    assert!(sprites
        .open(&ctx, sprites.bindings.as_ref().unwrap().path.clone())
        .is_err());
    assert_eq!(snapshot(&sprites), before);
    let path = sprites.bindings.as_ref().unwrap().path.clone();
    let elsewhere = path.with_extension("original");
    std::fs::rename(&path, &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &path).unwrap();
    assert!(panel
        .set_actor_source(&editor, &mut sprites, Source::Region(11))
        .is_err());
    assert!(panel.save_actors(&editor, &mut sprites).is_err());
    assert_eq!(snapshot(&sprites), before);
    assert!(std::fs::symlink_metadata(path)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[cfg(all(target_arch = "x86_64", feature = "arena-project-export"))]
#[test]
fn tilemap_actor_saved_admission_and_arena_export_keep_the_updated_stamp() {
    let (_temp, mut panel, mut sprites, mut editor) = fixture();
    let root = panel.stamp.root.clone();
    panel
        .set_actor_source(&editor, &mut sprites, Source::Clip("walk".into()))
        .unwrap();
    assert!(editor
        .arena_export_source_with_tilemap(&root, &sprites, None, Some(&panel))
        .is_err());
    panel.save_actors(&editor, &mut sprites).unwrap();
    assert!(panel
        .change(CellCoord { column: 1, row: 1 }, Some(11), &sprites)
        .unwrap());
    panel.save(&editor, &sprites).unwrap();
    let fresh = PreparedProject::open_with_capabilities(
        &root,
        crate::sprite_bindings::compiled_runtime(),
        UiSupport::Unsupported,
        TilemapSupport::Supported,
    )
    .unwrap()
    .into_presentation_parts();
    let tilemap = fresh.tilemap.unwrap();
    assert_eq!(
        fresh.sprites.unwrap().document,
        *sprites.bindings.as_ref().unwrap().document()
    );
    panel.arena_export_ready(&editor, &root).unwrap();
    panel
        .arena_export_matches(&editor, &root, &tilemap)
        .unwrap();
    let before = snapshot(&sprites);
    let tiles = panel.history_snapshot();
    let source = editor
        .arena_export_source_with_tilemap(&root, &sprites, None, Some(&panel))
        .unwrap();
    assert_eq!(source.inventory().len(), 5);
    assert_eq!(snapshot(&sprites), before);
    assert_eq!(panel.history_snapshot(), tiles);
}

#[test]
fn tilemap_actor_save_staged_failure_leaves_file_history_saved_and_stamp_unchanged() {
    let (_temp, mut panel, mut sprites, editor) = fixture();
    panel
        .set_actor_source(&editor, &mut sprites, Source::Region(11))
        .unwrap();
    let before = snapshot(&sprites);
    let stamp = panel.stamp.actors.clone();
    let (path, old) = stamp.as_ref().unwrap();
    let files = std::fs::read_dir(path.parent().unwrap()).unwrap().count();
    let mut staged = false;
    let result = panel.save_actors_with(&editor, &mut sprites, |phase, target| {
        assert_eq!(phase, "staged");
        staged = true;
        assert_eq!(std::fs::read(target).unwrap(), *old);
        assert_eq!(
            std::fs::read_dir(target.parent().unwrap()).unwrap().count(),
            files + 1
        );
        Err("injected after synced stage".into())
    });
    assert!(staged);
    assert_eq!(result.unwrap_err(), "injected after synced stage");
    assert_eq!(snapshot(&sprites), before);
    assert_eq!(panel.stamp.actors, stamp);
    assert_eq!(std::fs::read(path).unwrap(), *old);
    assert_eq!(
        std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
        files
    );
}

#[test]
fn tilemap_actor_save_rechecks_external_changes_after_synced_stage() {
    for input in ["actor", "lock", "png"] {
        let (_temp, mut panel, mut sprites, editor) = fixture();
        panel
            .set_actor_source(&editor, &mut sprites, Source::Region(11))
            .unwrap();
        let before = snapshot(&sprites);
        let stamp = panel.stamp.actors.clone();
        let (actor, actor_bytes) = stamp.as_ref().unwrap();
        let path = match input {
            "actor" => actor.clone(),
            "lock" => panel.stamp.root.join("orr.packages.lock.json"),
            _ => find_png(&panel.stamp.root),
        };
        let mut external = std::fs::read(&path).unwrap();
        external.push(b' ');
        let mut staged = false;
        let result = panel.save_actors_with(&editor, &mut sprites, |phase, _| {
            assert_eq!(phase, "staged");
            staged = true;
            std::fs::write(&path, &external).unwrap();
            Ok(())
        });
        assert!(staged);
        assert!(result.is_err(), "{input}");
        assert_eq!(snapshot(&sprites), before, "{input}");
        assert_eq!(panel.stamp.actors, stamp, "{input}");
        assert_eq!(std::fs::read(&path).unwrap(), external, "{input}");
        if input != "actor" {
            assert_eq!(std::fs::read(actor).unwrap(), *actor_bytes);
        }
    }
}

#[test]
fn tilemap_actor_save_postpublication_failure_reports_uncertainty_without_rollback() {
    for alter_published_bytes in [false, true] {
        let (_temp, mut panel, mut sprites, editor) = fixture();
        panel
            .set_actor_source(&editor, &mut sprites, Source::Region(11))
            .unwrap();
        let before = snapshot(&sprites);
        let stamp = panel.stamp.actors.clone();
        let (path, old) = stamp.as_ref().unwrap();
        let intended = serde_json::to_vec_pretty(&before.0).unwrap();
        let mut observed_publication = false;
        let error = panel
            .save_actors_with(&editor, &mut sprites, |phase, target| {
                if phase == "published" {
                    observed_publication = true;
                    assert_eq!(std::fs::read(target).unwrap(), intended);
                    if alter_published_bytes {
                        std::fs::write(target, b"external postpublication bytes").unwrap();
                        return Ok(());
                    }
                    return Err("injected postpublication failure".into());
                }
                assert_eq!(phase, "staged");
                Ok(())
            })
            .unwrap_err();
        assert!(observed_publication);
        assert!(error.contains("published") && error.contains("outcome uncertain"));
        assert_eq!(snapshot(&sprites), before);
        assert_eq!(panel.stamp.actors, stamp);
        assert_ne!(&std::fs::read(path).unwrap(), old);
        assert!(sprites.bindings.as_ref().unwrap().dirty());
        assert!(panel.save_actors(&editor, &mut sprites).is_err());
        assert_eq!(snapshot(&sprites), before);
    }
}

#[test]
fn tilemap_actor_sources_and_save_refuse_active_tile_gestures_without_cancelling() {
    let (_temp, mut panel, mut sprites, editor) = fixture();
    panel
        .set_actor_source(&editor, &mut sprites, Source::Region(11))
        .unwrap();
    let before = snapshot(&sprites);
    let tile_history = panel.history_snapshot();
    let actor_stamp = panel.stamp.actors.clone();
    let original_bytes = std::fs::read(&actor_stamp.as_ref().unwrap().0).unwrap();
    for rectangle in [false, true] {
        panel.painting = !rectangle;
        panel.rectangle = rectangle.then(|| RectangleGesture {
            start: CellCoord { column: 0, row: 0 },
            end: CellCoord { column: 1, row: 1 },
            region: Some(10),
            source: panel.document().clone(),
            scene_bytes: std::fs::read(&panel.stamp.scene).unwrap(),
            frame: 0,
        });
        assert!(!panel.actors_editable(&editor));
        assert!(panel
            .set_actor_source(&editor, &mut sprites, Source::Region(20))
            .unwrap_err()
            .contains("active tile gesture"));
        assert!(panel.undo_actor_source(&editor, &mut sprites).is_err());
        assert!(panel.redo_actor_source(&editor, &mut sprites).is_err());
        assert!(panel.save_actors(&editor, &mut sprites).is_err());
        assert_eq!(snapshot(&sprites), before);
        assert_eq!(panel.history_snapshot(), tile_history);
        assert_eq!(panel.stamp.actors, actor_stamp);
        assert_eq!(
            std::fs::read(&actor_stamp.as_ref().unwrap().0).unwrap(),
            original_bytes
        );
        assert_eq!(panel.painting, !rectangle);
        assert_eq!(panel.rectangle.is_some(), rectangle);
    }
    panel.cancel_gesture();
    assert!(panel.actors_editable(&editor));
    assert!(panel
        .set_actor_source(&editor, &mut sprites, Source::Region(20))
        .unwrap());
}
