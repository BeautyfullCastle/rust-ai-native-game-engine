use super::*;
#[test]
fn explicit_filename_dispatch_preserves_raw_and_recognizes_cooked_aliases_before_io() {
    for name in ["a.glb", "a.GLB"] {
        assert_eq!(source_asset(Path::new(name)).unwrap(), "model.glb");
    }
    for name in [
        "a.model.json",
        "a.MODEL.JSON",
        "a.orrmodel.json",
        "a.orrmodel",
        "a.ORRMODEL",
    ] {
        assert_eq!(source_asset(Path::new(name)).unwrap(), "model.model.json");
    }
    for name in ["a.json", "a.gltf", "a.glb.exe", "a.model.json.bak", ""] {
        assert!(source_asset(Path::new(name)).is_err());
    }
}
#[cfg(target_os = "linux")]
mod filesystem {
    use super::*;
    fn request(root: &Path, bytes: &[u8]) -> Request {
        let source = root.join("input.model.json");
        let license = root.join("LICENSE-input.txt");
        fs::write(&source, bytes).unwrap();
        fs::write(&license, b"Explicit fixture rights").unwrap();
        Request {
            source,
            license,
            destination: root.join("package"),
            name: "cooked-static".into(),
            version: "1.0.0".into(),
            license_acknowledged: true,
            graph_revision: [0x34; 32],
        }
    }
    #[test]
    fn authored_outputs_publish_verbatim_and_reopen_without_inputs() {
        for (name, bytes) in super::variants() {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let r = request(&root, &bytes);
            let source = r.source.clone();
            let license = r.license.clone();
            let report = import_package(r).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(report.asset, "model.model.json");
            assert_eq!(report.source_sha256, digest(&bytes));
            assert_eq!(fs::read(report.root.join(&report.asset)).unwrap(), bytes);
            assert_eq!(fs::read(&source).unwrap(), bytes);
            assert_eq!(fs::read(&license).unwrap(), b"Explicit fixture rights");
            let embedded = report.snapshot.encode_embedded("test.scene.yaml").unwrap();
            fs::rename(source, root.join("hidden-source")).unwrap();
            fs::rename(license, root.join("hidden-license")).unwrap();
            fs::rename(&report.root, root.join("hidden-package")).unwrap();
            let reopened =
                Snapshot::decode_embedded(&embedded, "test.scene.yaml", [0x34; 32]).unwrap();
            assert_eq!(reopened.source_model_arc().to_bytes().unwrap(), bytes);
            assert!(reopened.placement([orr_fp::FP::ZERO; 3]).is_ok());
        }
    }
    #[test]
    fn malformed_cooked_and_unacknowledged_inputs_do_not_publish_or_fallback() {
        for bytes in [
            b"{}".as_slice(),
            include_bytes!("../../../orr_model/tests/fixtures/model.glb").as_slice(),
            b"[]",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let r = request(&root, bytes);
            let dest = r.destination.clone();
            assert!(import_package(r).is_err());
            assert!(!dest.exists());
            assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
        }
        let temp = tempfile::tempdir().unwrap();
        let mut r = request(temp.path(), b"{}");
        r.license_acknowledged = false;
        assert!(import_package(r).is_err());
    }
    #[test]
    fn cooked_aliases_and_existing_destinations_keep_new_only_contract() {
        let bytes = super::variants().remove(0).1;
        for suffix in ["MODEL.JSON", "orrmodel.json", "orrmodel"] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let mut r = request(&root, &bytes);
            let alias = root.join(format!("input.{suffix}"));
            fs::rename(&r.source, &alias).unwrap();
            r.source = alias;
            let report = import_package(r).unwrap();
            assert_eq!(report.asset, "model.model.json");
            let sentinel = report.root.join("sentinel");
            fs::write(&sentinel, b"keep").unwrap();
            let r = request(&root, &bytes);
            assert!(import_package(r).is_err());
            assert_eq!(fs::read(sentinel).unwrap(), b"keep");
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn variants() -> Vec<(&'static str, Vec<u8>)> {
    crate::mesh_art::cooked_provenance::tests::variants()
}
