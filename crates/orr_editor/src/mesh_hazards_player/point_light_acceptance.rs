//! Real relocated native CPU process; no window, GPU, or physical-input claim.
#![cfg(target_os = "linux")]
#![allow(clippy::disallowed_types)]
use super::*;
use std::{fs, process::Command};
#[test]
#[ignore = "requires trusted ORR_MESH_POINT_LIGHT_RUNTIME native executable"]
fn point_light_survives_source_hidden_native_export_and_restart() {
    let runtime = PathBuf::from(
        std::env::var_os("ORR_MESH_POINT_LIGHT_RUNTIME").expect("trusted runtime required"),
    );
    assert!(runtime.is_absolute());
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let scene = root.join("source.scene.yaml");
    let (_, pin, mission) =
        crate::mesh_workspace::mission::configured_scene(&Default::default()).unwrap();
    let types = orr_remote::mesh_navigation::hazards::hazards_types();
    let mut singletons = serde_json::Map::new();
    for (name, bytes) in [
        (
            orr_remote::mesh_navigation::PIN_NAME,
            bytemuck::bytes_of(&pin),
        ),
        (
            orr_remote::mesh_navigation::MISSION_NAME,
            bytemuck::bytes_of(&mission),
        ),
    ] {
        singletons.insert(
            name.into(),
            orr_remote::json::value_to_json(&types.get(name).unwrap().read(bytes)),
        );
    }
    fs::write(
        &scene,
        serde_json::json!({"schema":"orr.scene/1","singletons":singletons,"entities":{}})
            .to_string(),
    )
    .unwrap();
    let doc = orr_remote::mesh_navigation::hazards::hazards_doc_from_path(&scene).unwrap();
    let checksum = doc.frame().checksum();
    let light = orr_render::PointLight {
        position: [2., 3., 4.],
        color: [0.2, 0.7, 0.4],
        intensity: 3.,
        range: 8.,
    };
    let mut lighting =
        crate::mesh_lighting::Document::from_lighting(orr_render::Lighting::default()).unwrap();
    lighting.point_light = Some(light);
    let sidecar = crate::mesh_lighting_store::sidecar_path(&scene).unwrap();
    fs::write(
        &sidecar,
        lighting
            .encode("source.scene.yaml", pin.graph_revision)
            .unwrap(),
    )
    .unwrap();
    let source_runtime = root.join("trusted-player");
    fs::copy(runtime, &source_runtime).unwrap();
    let dest = root.join("bundle");
    crate::mesh_hazards_export::export_interactive(crate::mesh_hazards_export::ExportRequest {
        scene: &scene,
        expected_checksum: checksum,
        destination: &dest,
        runtime: &source_runtime,
        trusted_runtime: true,
    })
    .unwrap();
    fs::remove_file(scene).unwrap();
    fs::remove_file(sidecar).unwrap();
    fs::remove_file(source_runtime).unwrap();
    let bundle = crate::mesh_hazards_export::open_bundle(&dest).unwrap();
    let before = super::acceptance::inventory(&dest);
    let run = || {
        let out = Command::new(dest.join(bundle.platform.runtime_filename()))
            .args([
                "--bundle",
                dest.to_str().unwrap(),
                "--verify-headless",
                "--ticks",
                "8",
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()
    };
    let first = run();
    assert_eq!(first, run());
    assert_eq!(first["tick"], 8);
    assert_eq!(first["point_light"], serde_json::to_value(light).unwrap());
    assert_eq!(first["point_light_restart_parity"], true);
    assert_eq!(first["lighting_restart_parity"], true);
    assert_eq!(first["restarted_checksum"], format!("{checksum:016x}"));
    assert_eq!(super::acceptance::inventory(&dest), before);
}
