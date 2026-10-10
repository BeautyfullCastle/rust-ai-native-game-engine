//! Built-in identities agree across native/browser clients and room presets,
//! while the old checksum generation is refused during the Hello handshake.

use std::cell::RefCell;
use std::rc::Rc;

use orr_games::physics_game::PhysGame;
use orr_proto::netsim::{PathParams, SimNet};
use orr_proto::{Channel, ClientMsg, Hello, Link, LinkEvent, RejectReason, ServerMsg, NO_SLOT};
use orr_sample::net_client;
use orr_server::presets::{self, Game, PhysicsScene};
use orr_server::RelayServer;
use orr_session::{DumpCollector, RelayClient, RelayClientConfig};
use orr_sim::build_hash_of;
use orr_testgame::Arena;

// Exact pre-ORRFv2 IDs and wire hashes. Keep these historical values fixed:
// computing "old" hashes through a future implementation could hide a regression.
const OLD_ARENA_ID: u64 = 0x0A2E_4A00_0001;
const OLD_PHYSICS_ID: u64 = 0x0A2E_4A00_0002;
const OLD_ARENA_HASH: u64 = 0x2fa4_82c8_48c6_58f8;
const OLD_PHYSICS_HASH: u64 = 0x855b_3a44_dff5_a753;

#[test]
fn native_browser_and_server_use_the_same_versioned_build_ids() {
    for (server, native, browser, old_id, old_hash) in [
        (
            presets::ARENA_BUILD_ID,
            net_client::ARENA_BUILD_ID,
            orr_web::ARENA_BUILD_ID,
            OLD_ARENA_ID,
            OLD_ARENA_HASH,
        ),
        (
            presets::PHYSICS_BUILD_ID,
            net_client::PHYSICS_BUILD_ID,
            orr_web::PHYSICS_BUILD_ID,
            OLD_PHYSICS_ID,
            OLD_PHYSICS_HASH,
        ),
    ] {
        assert_eq!(server, native);
        assert_eq!(server, browser);
        assert_eq!(server, orr_sim::frame_build_id(old_id));
        assert_ne!(server, 0);
        assert_ne!(server, old_id);
        assert_ne!(build_hash_of(server, 0), old_hash);
    }
}

fn handshake(game: Game, server_hash: u64, client_hash: u64) -> ServerMsg {
    let net = SimNet::new(17);
    let mut server = RelayServer::new(net.endpoint(), 23);
    let mut room = presets::room_config(game, 1, 60, presets::SAMPLE_SEED, PhysicsScene::default());
    room.build_hash = server_hash;
    server.create_room(1, room);
    let mut link = net.connect(PathParams::default());
    assert_eq!(link.poll(), Some(LinkEvent::Connected));
    let hello = Hello {
        build_hash: client_hash,
        room: 1,
        input_size: presets::SAMPLE_INPUT_SIZE,
        want_slot: NO_SLOT,
        token: 0,
    };
    link.send(Channel::Reliable, &ClientMsg::Hello(hello).encode());
    server.update(0);
    let Some(LinkEvent::Message {
        channel: Channel::Reliable,
        data,
    }) = link.poll()
    else {
        panic!("server did not answer Hello");
    };
    ServerMsg::decode(&data).expect("valid server response")
}

#[test]
fn both_sample_rooms_reject_old_new_mixes_before_simulation() {
    for (game, current_id, old_hash) in [
        (Game::Arena, presets::ARENA_BUILD_ID, OLD_ARENA_HASH),
        (Game::Physics, presets::PHYSICS_BUILD_ID, OLD_PHYSICS_HASH),
    ] {
        let current_hash = build_hash_of(current_id, 0);
        let room = presets::room_config(game, 1, 60, presets::SAMPLE_SEED, PhysicsScene::default());
        assert_eq!(room.build_hash, current_hash);
        // Same wire handshake for QUIC, WebSocket and WebTransport.
        assert!(matches!(
            handshake(game, current_hash, current_hash),
            ServerMsg::Welcome(_)
        ));
        for (server, client) in [(current_hash, old_hash), (old_hash, current_hash)] {
            assert_eq!(
                handshake(game, server, client),
                ServerMsg::Reject(RejectReason::BuildHashMismatch { server, client }),
            );
        }
    }
}

fn browser_hello<G: orr_sim::Game>(build_id: u64, max_datagram: usize) -> Hello {
    let sent = Rc::new(RefCell::new(Vec::new()));
    let out = sent.clone();
    let (link, port) = orr_web::WebLink::new(
        move |channel, bytes| out.borrow_mut().push((channel, bytes.to_vec())),
        || {},
    );
    let mut client = RelayClient::<G, _>::new(
        RelayClientConfig::new(1, build_id),
        link,
        |_| panic!("no Welcome yet"),
        DumpCollector::new(),
    );
    port.connected(max_datagram);
    client.update(0, &mut |_| panic!("no simulation before Welcome"));
    let messages = sent.borrow();
    let (channel, bytes) = messages.first().expect("client sent Hello");
    assert_eq!(*channel, Channel::Reliable);
    let ClientMsg::Hello(hello) = ClientMsg::decode(bytes).unwrap() else {
        panic!("first message was not Hello")
    };
    hello
}

#[test]
fn browser_websocket_and_webtransport_send_the_preset_identity() {
    for max_datagram in [0, orr_proto::UNRELIABLE_MTU] {
        for (game, hello) in [
            (
                Game::Arena,
                browser_hello::<Arena>(orr_web::ARENA_BUILD_ID, max_datagram),
            ),
            (
                Game::Physics,
                browser_hello::<PhysGame>(orr_web::PHYSICS_BUILD_ID, max_datagram),
            ),
        ] {
            let room =
                presets::room_config(game, 1, 60, presets::SAMPLE_SEED, PhysicsScene::default());
            assert_eq!(hello.build_hash, room.build_hash);
            assert_eq!(hello.input_size, room.input_size);
            assert!(matches!(
                handshake(game, room.build_hash, hello.build_hash),
                ServerMsg::Welcome(_)
            ));
        }
    }
}
