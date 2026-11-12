use super::*;

#[test]
fn source_and_license_changes_revoke_rights_and_all_changes_advance_generation() {
    let mut state = State::default();
    state.inputs.acknowledged = true;
    state.changed(false);
    assert!(state.inputs.acknowledged);
    assert_eq!(state.generation, 1);
    state.changed(true);
    assert!(!state.inputs.acknowledged);
    assert_eq!(state.generation, 2);
}

#[cfg(feature = "mesh-mission")]
mod editor_tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Editor, Panel) {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let editor = crate::mesh_workspace::mission::prepare(
            &root.join("import.scene.yaml"),
            &crate::mesh_workspace::mission::Settings::default(),
        )
        .unwrap();
        let mut panel = Panel::default();
        panel.sync(&editor);
        let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/animation_demo")
            .canonicalize()
            .unwrap();
        panel.import.inputs = Inputs {
            source: assets.join("animated.glb").display().to_string(),
            license: assets.join("LICENSE.txt").display().to_string(),
            destination: root.join("new-animation").display().to_string(),
            name: "raw-animation-test".into(),
            version: "1.0.0".into(),
            acknowledged: true,
        };
        assert!(panel.can_edit(&editor));
        (directory, editor, panel)
    }

    fn report(panel: &Panel) -> Report {
        let inputs = &panel.import.inputs;
        mesh_animation_import::import_package(Request {
            source: inputs.source.clone().into(),
            license: inputs.license.clone().into(),
            destination: inputs.destination.clone().into(),
            name: inputs.name.clone(),
            version: inputs.version.clone(),
            license_acknowledged: true,
            graph_revision: panel.graph_revision.unwrap(),
        })
        .unwrap()
    }

    fn finished_job(panel: &mut Panel, editor: &Editor, result: Result<Report, String>) {
        let context = context(panel, editor).unwrap();
        let (sender, receiver) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            sender.send(result).unwrap();
        });
        while !handle.is_finished() {
            std::thread::yield_now();
        }
        panel.import.job = Some(Job {
            context,
            receiver,
            handle: Some(handle),
            terminal: None,
        });
    }

    #[test]
    fn hidden_sync_stages_without_apply_save_or_history_mutation() {
        let (_directory, editor, mut panel) = fixture();
        let checksum = editor.checksum();
        let scene_bytes = std::fs::read(editor.path().unwrap()).unwrap();
        let sidecar = crate::mesh_animation_store::sidecar_path(&editor.path().unwrap()).unwrap();
        launch(&mut panel, &editor);
        assert!(panel.import_busy());
        assert!(!panel.dirty());
        assert!(!panel.can_edit(&editor));
        assert!(!panel.can_change_history(&editor));
        assert!(panel.play_blocker(&editor).unwrap().contains("worker"));
        assert!(panel
            .workspace_save_blocker(&editor)
            .unwrap()
            .contains("worker"));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while panel.import_busy() {
            panel.sync(&editor); // no show(), even with the header/panel hidden
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(panel.candidate.is_some());
        assert!(panel.current_snapshot().is_none());
        assert_eq!(panel.history.len(), 1);
        assert_eq!(panel.cursor, 0);
        assert_eq!(panel.saved_id, Some(0));
        assert_eq!(editor.checksum(), checksum);
        assert_eq!(std::fs::read(editor.path().unwrap()).unwrap(), scene_bytes);
        assert!(!sidecar.exists());
        assert!(!panel.import.inputs.acknowledged);
        assert!(panel
            .import
            .status
            .contains("staged for review, not applied"));
    }

    #[test]
    fn result_ready_does_not_release_worker_or_allow_duplicate_launch() {
        let (_directory, editor, mut panel) = fixture();
        let result = report(&panel);
        let context = context(&panel, &editor).unwrap();
        let (sender, receiver) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            sender.send(Ok(result)).unwrap();
            wait.recv().unwrap();
        });
        panel.import.job = Some(Job {
            context,
            receiver,
            handle: Some(handle),
            terminal: None,
        });
        while panel.import.job.as_ref().unwrap().terminal.is_none() {
            poll(&mut panel, &editor);
            std::thread::yield_now();
        }
        assert!(panel.import_busy());
        assert!(panel.candidate.is_none());
        launch(&mut panel, &editor);
        assert!(panel.import.job.as_ref().unwrap().terminal.is_some());
        release.send(()).unwrap();
        while !panel
            .import
            .job
            .as_ref()
            .unwrap()
            .handle
            .as_ref()
            .unwrap()
            .is_finished()
        {
            std::thread::yield_now();
        }
        poll(&mut panel, &editor);
        assert!(!panel.import_busy());
        assert!(panel.candidate.is_some());
    }

    #[test]
    fn every_launch_pin_rejects_stale_results_but_reports_published_path() {
        for changed in 0..10 {
            let (_directory, editor, mut panel) = fixture();
            let result = report(&panel);
            let published = result.root.clone();
            finished_job(&mut panel, &editor, Ok(result));
            match changed {
                0 => panel.import.inputs.source.push('x'),
                1 => panel.import.inputs.license.push('x'),
                2 => panel.import.inputs.destination.push('x'),
                3 => panel.import.inputs.name.push('x'),
                4 => panel.import.inputs.version.push('x'),
                5 => panel.import.inputs.acknowledged = false,
                6 => panel.import.changed(false),
                7 => panel.import.job.as_mut().unwrap().context.epoch = u64::MAX,
                8 => panel.import.job.as_mut().unwrap().context.revision = [0; 32],
                9 => {
                    panel.import.job.as_mut().unwrap().context.scene =
                        PathBuf::from("other.scene.yaml")
                }
                _ => unreachable!(),
            }
            poll(&mut panel, &editor);
            assert!(panel.candidate.is_none(), "changed field {changed}");
            assert!(panel.current_snapshot().is_none());
            assert!(panel.import.status.contains("not staged"));
            assert!(panel
                .import
                .status
                .contains(&published.display().to_string()));
            assert!(published.exists());
        }
    }

    #[test]
    fn changed_editor_scene_rejects_result_before_sync_rebinds_panel() {
        let (directory, editor, mut panel) = fixture();
        let result = report(&panel);
        finished_job(&mut panel, &editor, Ok(result));
        let other = crate::mesh_workspace::mission::prepare(
            &directory.path().join("other.scene.yaml"),
            &crate::mesh_workspace::mission::Settings::default(),
        )
        .unwrap();
        panel.sync(&other);
        assert!(!panel.import_busy());
        assert!(panel.candidate.is_none());
        assert!(panel.import.status.contains("not staged"));
    }

    #[test]
    fn disconnect_and_panic_are_terminal_and_do_not_stage_or_retry() {
        for panic in [false, true] {
            let (_directory, editor, mut panel) = fixture();
            let context = context(&panel, &editor).unwrap();
            let (sender, receiver) = mpsc::channel::<Result<Report, String>>();
            let handle = std::thread::spawn(move || {
                drop(sender);
                assert!(!panic, "test worker panic");
            });
            while !handle.is_finished() {
                std::thread::yield_now();
            }
            panel.import.job = Some(Job {
                context,
                receiver,
                handle: Some(handle),
                terminal: None,
            });
            panel.sync(&editor);
            assert!(!panel.import_busy());
            assert!(panel.candidate.is_none());
            assert!(panel.import.status.contains("disconnected"));
            if panic {
                assert!(panel.import.status.contains("panicked"));
            }
        }
    }

    #[test]
    fn publication_warning_never_stages_unverified_output() {
        let (_directory, editor, mut panel) = fixture();
        let mut result = report(&panel);
        result.publication_warning = Some("Published bytes changed externally".into());
        finished_job(&mut panel, &editor, Ok(result));
        panel.sync(&editor);
        assert!(panel.candidate.is_none());
        assert!(panel
            .import
            .status
            .contains("Published bytes changed externally"));
    }
}
