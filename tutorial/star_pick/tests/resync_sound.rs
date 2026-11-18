//! A `ViewResync` throws away the predicted events and no `Canceled` comes for them. The pick-up
//! sound that `Predicted` started must still be stopped, in steps 2 to 5.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use bytemuck::Zeroable;
use common::{SEED, TICK_RATE};
use orr_app::{AudioConfig, OfflineAudio, Views};
use orr_bridge::{Bridge, BridgeConfig, BridgeEvent, EventKey, EventStatus, InProc, PlayHost, ViewResync};
use orr_session::{PlayConfig, PlaySession};
use orr_sim::PlayerSlot;
use star_pick::sound;

const RATE: u32 = 44_100;

macro_rules! resync_stops_the_predicted_sound {
    ($name:ident, $step:ident, $players:expr, $setup:expr) => {
        #[test]
        fn $name() {
            use star_pick::$step::{Juice, Picked, Shared, StarPick};

            let fx = Shared::default();
            let mut views = Views::<StarPick>::new();
            let audio = Rc::new(RefCell::new(OfflineAudio::new_offline(AudioConfig::default(), RATE).unwrap()));
            views.set_sound_out(audio.clone());
            views.sound(sound::PICKUP, sound::pickup());
            views.scene_view(Juice { fx: fx.clone() });

            let session = PlaySession::<StarPick>::new(PlayConfig::new($players, SEED, TICK_RATE), $setup);
            let mut bridge = InProc::new(PlayHost::new(session, PlayerSlot(0)), BridgeConfig::default());
            bridge.step(1);

            // A predicted pick-up starts the ding.
            let mut update = bridge.poll_view();
            update.events = vec![BridgeEvent::Sim { key: EventKey::new(5, 0, 0), status: EventStatus::Predicted(Picked::zeroed()) }];
            views.dispatch(&mut bridge, &update, &[]);
            let handle = fx.borrow().rings[0].sound.expect("the ding started");
            assert!(audio.borrow().is_playing(handle));

            // A resync comes: the prediction is gone and its `Canceled` never arrives.
            let mut update = bridge.poll_view();
            update.resync = Some(ViewResync {
                generation: 1,
                discarded_events: 1,
                head_tick: 1,
                verified_tick: 1,
                disconnected: false,
                last_desync: None,
                lifecycle: Vec::new(),
            });
            views.dispatch(&mut bridge, &update, &[]);

            // The fade is 15 ms and the ding is 180 ms. After 2 x 735 frames (33 ms) it is quiet.
            let mut block = [0.0f32; 1470];
            audio.borrow_mut().render(&mut block).unwrap();
            audio.borrow_mut().render(&mut block).unwrap();
            assert!(!audio.borrow().is_playing(handle), "the ding was stopped");
            assert!(block.iter().all(|v| v.abs() < 1e-4), "nothing is heard after the fade");
            assert!(fx.borrow().rings.is_empty(), "the ring of the lost prediction is forgotten");
        }
    };
}

resync_stops_the_predicted_sound!(step2_resync_stops_the_predicted_sound, step2, 1, ());
resync_stops_the_predicted_sound!(step3_resync_stops_the_predicted_sound, step3, 2, ());
resync_stops_the_predicted_sound!(step4_resync_stops_the_predicted_sound, step4, 2, star_pick::step4::Setup::default());
resync_stops_the_predicted_sound!(step5_resync_stops_the_predicted_sound, step5, 2, star_pick::step5::Setup::default());
