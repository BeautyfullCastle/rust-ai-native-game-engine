#![allow(dead_code)]
use orr_fp::FP;
use orr_terrain_import::{Endian, Recipe};
use std::{fs, path::Path};
pub const SAMPLES: [u16; 6] = [0, 1, 32767, 32768, 65534, 65535];
pub fn recipe() -> Recipe {
    Recipe {
        asset_id: "terrain/imported".into(),
        width: 3,
        depth: 2,
        endian: Endian::Little,
        height_offset: FP::from_int(-1),
        height_scale: FP::ONE,
        origin: [FP::from_int(-1); 2],
        spacing: FP::ONE,
        flip_z: false,
    }
}
pub fn raw(samples: &[u16], endian: Endian) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|v| match endian {
            Endian::Little => v.to_le_bytes(),
            Endian::Big => v.to_be_bytes(),
        })
        .collect()
}
pub fn temp() -> tempfile::TempDir {
    tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap()
}
pub fn terrain() -> orr_terrain::Terrain {
    orr_terrain_import::import_raw16(&raw(&SAMPLES, Endian::Little), &recipe()).unwrap()
}
pub fn source(root: &Path, name: &str, capabilities: &[&str], files: &[&str]) {
    fs::create_dir(root).unwrap();
    fs::write(root.join("lab.orrt"), terrain().cook()).unwrap();
    fs::write(
        root.join("orr.package.json"),
        serde_json::to_vec(&serde_json::json!({
            "schema":1,"name":name,"version":"1.0.0","engine":"=0.0.1",
            "capabilities":capabilities,"dependencies":{},"files":files
        }))
        .unwrap(),
    )
    .unwrap();
}
