#![cfg(feature = "navigation")]
use orr_bridge::FrameView;
use orr_remote::navigation_yard3d::*;
use orr_sim::Simulation;

#[test]
fn terrain_patrol_game_host_mismatch_rejects_before_readback() {
    let simulation = Simulation::<NavigationYard3D>::new((), YARD_TICK_RATE, NAVIGATION_BUILD_ID);
    let error = navigation_from_view(FrameView::of(simulation.frame()))
        .err()
        .unwrap();
    assert_eq!(
        error.contains("patrol feature profile differs"),
        orr_games::navigation_yard3d_game::TERRAIN_PATROL_ENABLED
            != cfg!(feature = "terrain-patrol")
    );
}

#[test]
fn terrain_patrol_host_build_profile_is_distinct_and_legacy_stays_exact() {
    let mut limits = orr_remote::HostLimits::default();
    configure_navigation_yard3d(&mut limits);
    let expected = if cfg!(feature = "terrain-patrol") {
        0x544e_4156_0000_0001
    } else {
        YARD_BUILD_ID
    };
    assert_eq!(limits.build_id, expected);
    assert_eq!(NAVIGATION_BUILD_ID, expected);
    assert_ne!(0x544e_4156_0000_0001_u64, YARD_BUILD_ID);
}
