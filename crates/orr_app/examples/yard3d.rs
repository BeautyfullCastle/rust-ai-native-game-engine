//! The 3D physics yard on `orr_app`: `cargo run -p orr_app --release --example yard3d`.
//! Space shoots a ball at the middle of the picture, B, N and C drop a box, a ball or a capsule, Escape quits.
use orr_app::{default_orbit, App, KeyCode as K, Look3D};
use orr_games::yard3d_game::{Yard3D, YardConfig, TICK_RATE};
use orr_sample::yard3d_view::{input_for, YardKeys};

fn main() -> Result<(), orr_app::Error> {
    let aim = default_orbit().camera();
    App::<Yard3D>::with_config(YardConfig::new(400))
        .tick_rate(TICK_RATE)
        .input(move |k, _slot| {
            let keys = YardKeys { shoot: k.held(K::Space), spawn_box: k.held(K::KeyB), spawn_ball: k.held(K::KeyN), spawn_capsule: k.held(K::KeyC) };
            input_for(keys, &aim, [450.0, 450.0], (900, 900))
        })
        .view3d(Look3D::default())
        .run()
}
