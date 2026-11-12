//! Actual Rust deployment export -> public native argument parser -> in-memory Hello.
//! No generated launch plan is executed; SimNet opens no operating-system sockets.
#![cfg(all(target_os = "linux", feature = "project"))]

use orr_proto::netsim::{PathParams, SimNet};
use orr_proto::{Channel, ClientMsg, Hello, Link, LinkEvent, RejectReason, ServerMsg, NO_SLOT};
use orr_sample::net_client::NetArgs;
use orr_server::presets::{self, Game, PhysicsScene};
use orr_server::RelayServer;
use serde_json::Value;
use std::path::Path;
use std::process::Command;

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

fn successful(command: &mut Command) -> Vec<u8> {
    let result = command
        .output()
        .expect("execute actual explicit deployment CLI");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    result.stdout
}

#[test]
#[ignore = "requires explicit ORR_RUST_PACKAGE_BIN with deployment-manifest feature"]
fn actual_exported_client_args_preserve_u64_and_match_in_memory_handshakes() {
    let binary = std::env::var_os("ORR_RUST_PACKAGE_BIN").expect("explicit native CLI required");
    assert!(Path::new(&binary).is_absolute() && Path::new(&binary).is_file());
    let temporary = tempfile::tempdir().unwrap();
    for (name, game, client_program, browser_game) in [
        ("arena", Game::Arena, "arena", "arena"),
        ("physics", Game::Physics, "physics", "phys"),
    ] {
        for raw in [1u64, 9_007_199_254_740_993, u64::MAX] {
            let manifest = temporary.path().join(format!("{name}-{raw}.json"));
            successful(
                Command::new(&binary)
                    .args([
                        "deployment",
                        "create",
                        "--game",
                        name,
                        "--game-code-identity",
                        "native-consumer-fixture",
                        "--game-code-id",
                        &raw.to_string(),
                        "--output",
                    ])
                    .arg(&manifest),
            );
            let before = std::fs::read(&manifest).unwrap();
            let exported = successful(
                Command::new(&binary)
                    .args(["deployment", "export"])
                    .arg(&manifest),
            );
            let plan: Value = serde_json::from_slice(&exported).unwrap();
            let expected = orr_sim::frame_build_id(raw);
            assert_ne!(expected, 0);
            assert_eq!(plan["game_code_id"].as_str().unwrap(), raw.to_string());
            assert_eq!(plan["build_id"].as_str().unwrap(), expected.to_string());
            assert_eq!(plan["native_client"]["program"], client_program);
            assert_eq!(plan["browser"]["opts"]["game"], browser_game);
            assert_eq!(
                plan["browser"]["opts"]["build_id"].as_str().unwrap(),
                expected.to_string()
            );
            assert_eq!(
                plan["browser"]["query"]["build"].as_str().unwrap(),
                expected.to_string()
            );
            let args: Vec<String> =
                serde_json::from_value(plan["native_client"]["args"].clone()).unwrap();
            assert_eq!(args, ["--build-id".to_string(), expected.to_string()]);
            let mut net_args = NetArgs::default();
            let mut args = args.into_iter();
            while let Some(option) = args.next() {
                assert!(net_args
                    .parse_option(&option, &mut |flag| args
                        .next()
                        .ok_or_else(|| format!("missing {flag}")))
                    .unwrap());
            }
            assert_eq!(net_args.build_id, Some(expected));
            assert!(
                net_args.connect.is_none(),
                "manifest export must not turn networking on"
            );
            for role in ["native_host", "relay"] {
                let launch: Vec<String> =
                    serde_json::from_value(plan[role]["args"].clone()).unwrap();
                assert_eq!(
                    launch,
                    [
                        "--game".to_owned(),
                        name.to_owned(),
                        "--build-id".to_owned(),
                        expected.to_string()
                    ]
                );
            }
            let matching = orr_sim::build_hash_of(net_args.build_id.unwrap(), 0);
            assert!(matches!(
                handshake(game, matching, matching),
                ServerMsg::Welcome(_)
            ));
            let mismatch = orr_sim::build_hash_of(expected ^ 1, 0);
            assert_ne!(mismatch, matching);
            assert_eq!(
                handshake(game, matching, mismatch),
                ServerMsg::Reject(RejectReason::BuildHashMismatch {
                    server: matching,
                    client: mismatch
                })
            );
            assert_eq!(
                std::fs::read(&manifest).unwrap(),
                before,
                "read-only export changed input"
            );
        }
    }
}
