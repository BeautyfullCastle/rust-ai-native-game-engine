//! The fallback ERP host identity changes with the frame/checksum format,
//! even when the package version and game name have not changed.

fn legacy_default_build_id(game: &str) -> u64 {
    let text = format!("orr_remote_host/{}/{game}", env!("CARGO_PKG_VERSION"));
    text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[test]
fn default_host_ids_bind_the_game_and_frame_checksum_format() {
    for game in ["Arena", "PhysGame"] {
        let legacy = legacy_default_build_id(game);
        let current = orr_remote::default_build_id(game);
        assert_eq!(current, orr_sim::frame_build_id(legacy));
        assert_ne!(current, 0);
        assert_ne!(current, legacy);
        assert_ne!(
            orr_sim::build_hash_of(current, 0),
            orr_sim::build_hash_of(legacy, 0)
        );
    }
    assert_ne!(
        orr_remote::default_build_id("Arena"),
        orr_remote::default_build_id("PhysGame")
    );
}
