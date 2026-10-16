//! Explicit candidate generator. It never changes the source-controlled SHA allowlist.
use orr_asset_fixture::PreparedFixture;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: regenerate <new-output-directory>")?,
    );
    fs::create_dir(&out)?; // Never overwrite a retained release.
    let prepared = PreparedFixture::embedded()?;
    let recording = prepared.record()?;
    fs::write(out.join("baseline.orrp"), &recording.replay)?;
    let checksums: String = recording
        .states
        .iter()
        .map(|state| format!("{} {:016x}\n", state.tick, state.checksum))
        .collect();
    fs::write(out.join("checksums.txt"), checksums)?;
    fs::write(out.join("release.json"), prepared.release().to_json()?)?;
    let digest: [u8; 32] = Sha256::digest(&recording.replay).into();
    println!("Candidate replay SHA-256: {digest:?}");
    println!("Review the candidate bytes/checksums, then explicitly pin the SHA in src/replay.rs.");
    Ok(())
}
