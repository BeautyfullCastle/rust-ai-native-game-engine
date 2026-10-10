use orr_asset::{Domain, Manifest, SimTable};
use orr_asset_cook::{cook, sha256, CookOptions};
use std::path::PathBuf;

mod generated {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/fixture_v1/cooked/generated_sim.rs"
    ));
}

#[test]
fn checked_in_rust_compiles_and_reencodes_to_the_manifest() {
    let bytes = include_bytes!("../../../assets/fixture_v1/cooked/sim.manifest.bin");
    assert_eq!(sha256(bytes), generated::SIM_MANIFEST_SHA256);
    let manifest = Manifest::decode(bytes, Domain::Sim).unwrap();
    let table = SimTable::new(&generated::MOTION_PROFILES).unwrap();
    assert_eq!(manifest.entries().len(), table.entries().len());
    for ((id, value), entry) in table.entries().iter().zip(manifest.entries()) {
        assert_eq!(*id, entry.id);
        assert_eq!(sha256(&value.encode()), entry.payload_sha256);
        assert_eq!(value.encode().len() as u64, entry.payload_len);
        assert_eq!(
            table.resolve(manifest.typed_ref(*id).unwrap()).unwrap(),
            value
        );
    }
}

#[test]
fn checked_in_cooked_fixture_has_no_drift() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fixture_v1");
    cook(&CookOptions {
        index: root.join("index.json"),
        out: root.join("cooked"),
        cache: None,
        roots: generated::MOTION_PROFILES
            .iter()
            .map(|(id, _)| *id)
            .collect(),
        check: true,
    })
    .unwrap();
}
