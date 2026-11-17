//! Player signals through the bridge of a local play session: the host
//! announces every player in tick 1, and the view reads each signal as a
//! `BridgeEvent::Player` that is already `Verified`.
use orr_bridge::{
    Bridge, BridgeConfig, BridgeEvent, ControlOp, EventStatus, InProc, PlayConfig, PlayHost, PlaySession, PlayerSlot, SimControl,
};
use orr_sim::{signal_key, RawPlayerSignal};
use orr_testgame::roster::{Avatar, Roster};

fn host(players: u8, paused: bool) -> PlayHost<Roster> {
    let mut cfg = PlayConfig::new(players, 3, 60);
    cfg.start_paused = paused;
    PlayHost::new(PlaySession::<Roster>::new(cfg, ()), PlayerSlot(0))
}

fn players_in(events: &[BridgeEvent<orr_testgame::roster::Moved>]) -> Vec<(orr_sim::EventKey, EventStatus<RawPlayerSignal>)> {
    events
        .iter()
        .filter_map(|e| match e {
            BridgeEvent::Player { key, status } => Some((*key, status.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn an_announced_host_tells_the_view_about_every_player_in_the_first_tick() {
    let mut bridge = InProc::new(host(3, false), BridgeConfig::default());
    bridge.step(1);
    let update = bridge.poll_view();

    let players = players_in(&update.events);
    assert_eq!(players.len(), 0, "a host that does not announce says nothing");

    let mut bridge = InProc::new(host(3, false).with_announced_players(), BridgeConfig::default());
    bridge.step(1);
    let update = bridge.poll_view();
    let players = players_in(&update.events);
    assert_eq!(players.len(), 3);
    for (slot, (key, status)) in players.iter().enumerate() {
        assert_eq!(*key, signal_key(1, PlayerSlot(slot as u8), 0));
        assert_eq!(*status, EventStatus::Verified(RawPlayerSignal::added(PlayerSlot(slot as u8))));
    }
    let snapshot = update.snapshot.expect("a snapshot came with the tick");
    assert_eq!(snapshot.predicted().dense::<Avatar>().1.len(), 3, "the avatars exist in the first published frame");

    bridge.step(1);
    assert!(players_in(&bridge.poll_view().events).is_empty(), "only tick 1 has signals");
}

#[test]
fn a_step_control_delivers_the_signals_of_the_tick_it_ran() {
    let mut bridge = InProc::new(host(2, true).with_announced_players(), BridgeConfig::default());
    bridge.control(ControlOp::Step(1)).unwrap();
    let players = players_in(&bridge.poll_view().events);
    assert_eq!(players.len(), 2);
}
