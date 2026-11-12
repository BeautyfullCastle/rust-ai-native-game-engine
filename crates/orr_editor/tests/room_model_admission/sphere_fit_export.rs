use super::*;
use sha2::{Digest, Sha256};
use std::process::Command;

#[test]
fn edited_sphere_player_transform_is_used_by_source_hidden_exported_runtime() {
    let fixture = Fixture::new(false);
    let (mut editor, mut panel) = fixture.panel();
    editor.select(Some(Target::Guid(fixture.guids[&PLAYER].clone())));
    panel.use_selected_settings(&editor).unwrap();
    let document = panel.bindings.as_ref().unwrap().document().clone();
    let checksum = editor.checksum();
    panel.fit_selected_sphere(&editor).unwrap();
    let fit = panel.transform;
    assert_ne!(fit, LocalTransform::default());
    assert_eq!(panel.bindings.as_ref().unwrap().document(), &document);
    assert_eq!(editor.checksum(), checksum);
    panel.assign(&editor).unwrap();
    panel.save(&editor).unwrap();
    let saved = fs::read(fixture.root.join("room.models.json")).unwrap();
    let built_runtime = PathBuf::from(
        std::env::var_os("ORR_SPHERE_ROOM_RUNTIME")
            .expect("explicit focused export requires ORR_SPHERE_ROOM_RUNTIME"),
    )
    .canonicalize()
    .unwrap();
    let runtime = fixture._temp.path().join("trusted room runtime");
    fs::copy(&built_runtime, &runtime).unwrap();
    let digest: String = Sha256::digest(fs::read(&runtime).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let bundle = fixture._temp.path().join("installed sphere game");
    orr_sample::project_export::export_room(&orr_sample::project_export::ExportOptions {
        project: fixture.root.clone(),
        runtime: runtime.clone(),
        runtime_sha256: digest,
        output: bundle.clone(),
        trusted_runtime: true,
        source_revision: None,
    })
    .unwrap();
    assert_eq!(
        fs::read(bundle.join("project/room.models.json")).unwrap(),
        saved
    );
    fs::rename(&fixture.root, fixture.root.with_extension("hidden")).unwrap();
    fs::rename(
        fixture._temp.path().join("source"),
        fixture._temp.path().join("source.hidden"),
    )
    .unwrap();
    fs::rename(&runtime, runtime.with_extension("hidden")).unwrap();
    let deployed = PreparedProject::open(&bundle.join("project")).unwrap();
    assert_eq!(
        deployed.models().document.bindings[&fixture.guids[&PLAYER].to_string()].transform,
        fit
    );
    let output = Command::new(bundle.join("run-room-escape"))
        .current_dir(fixture._temp.path())
        .args(["--headless", "--ticks", "5", "--check-model-poses"])
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout)
        .contains("room model poses: 6 frames checked; final static: 1 animated: 0"));
}
