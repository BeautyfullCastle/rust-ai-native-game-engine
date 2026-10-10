#![cfg(feature = "sprites")]
#![allow(clippy::disallowed_types)]
use orr_editor::project::PreparedProject;
use std::{fs, path::Path};
#[allow(dead_code)]
#[path = "../../orr_sample/tests/common/project.rs"]
mod project_fixture;
use project_fixture::ProjectFixture;

#[test]
fn saved_ui_requires_the_editors_own_explicit_opt_in_and_retains_metadata() {
    // Do not use ProjectFixture::new(): its CARGO_MANIFEST_DIR is this crate.
    let root = tempfile::tempdir().unwrap();
    let path = root.path().canonicalize().unwrap();
    project_fixture::copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/saved_arena_project"),
        &path,
    );
    let _fixture = ProjectFixture { root: path.clone() };
    let project = orr_package::Project::open_for_install(
        &path,
        orr_package::Runtime::content_only().engine_version,
    )
    .unwrap();
    project
        .install(&[Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/game_ui_font")
            .canonicalize()
            .unwrap()])
        .unwrap();
    let manifest = path.join("orr.project.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["entry"]["ui"] = serde_json::json!({"profile":"arena-korean-v1","font":{
        "package":"korean-game-ui", "asset":"OrreryKoreanUI.otf"}});
    let bytes = serde_json::to_vec(&value).unwrap();
    fs::write(&manifest, &bytes).unwrap();
    let opened = PreparedProject::open(&path);
    #[cfg(not(feature = "project-ui"))]
    assert!(opened.err().unwrap().contains("UI"));
    #[cfg(feature = "project-ui")]
    {
        let opened = opened.unwrap();
        let prepared = opened.ui().unwrap();
        assert_eq!(
            prepared.font,
            include_bytes!("../../../assets/game_ui_font/OrreryKoreanUI.otf")
        );
        assert_eq!(
            prepared.descriptor.profile,
            orr_package::ProjectUiProfile::ArenaKoreanV1
        );
        let context = egui::Context::default();
        let preview = orr_editor::project_ui::Preview::install(prepared.clone(), &context);
        assert_eq!(preview.descriptor, prepared.descriptor);
        let output = context.run_ui(egui::RawInput::default(), |root| preview.show(root));
        output.drop_without_applying_deltas();
    }
    assert_eq!(
        fs::read(manifest).unwrap(),
        bytes,
        "opening/preview is read-only"
    );
}
