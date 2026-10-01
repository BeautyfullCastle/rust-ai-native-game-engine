# Authoritative server mode

Design reference: `design-v1.md` section 6.1 (modes) and decision 7 ("relay by default, server-authoritative optional"). This file describes what `orr_server` implements (M6 step 1).

A room opts in; every other room stays a plain relay and behaves exactly as before.

```text
RelayServer::create_room(id, cfg)                                  // relay (default)
RelayServer::create_authoritative_room(id, cfg, auth, Box<dyn ServerSim>)   // authoritative
orr_server --authoritative [--ai-slot N]...                        // the binary, arena only
```

## Who simulates what

| | relay | authoritative |
|---|---|---|
| clients | predict, roll back, verify on the confirmed bundle | same (the same `RelayClient`) |
| server | collects inputs, finalizes each tick at its deadline, relays the bundle | same, **and** steps its own `Simulation<G>` on every finalized bundle |

The server sim never predicts and never rolls back: it only sees confirmed bundles. It steps inside the finalize of tick `T`, so when the inputs of tick `T + 1` are decided its state is the state after `T` (one tick behind the confirm point). `GameSim<G>` builds the sim exactly like a client's `Session::new` (config, tick rate, seed, build id) and turns a bundle into `TickInputs` the way a session does (absent slot -> `disconnected` flag, commands that do not decode are skipped).

## Flow

```text
tick T finalizes:
  1. server slots: ServerSim::drive(T)      -> input + commands of AI / scripted slots (from the frame after T-1)
  2. bundle(T) built from received inputs + the server slots' inputs
  3. ServerSim::step(bundle)                -> sim at T; the state audit runs (violations -> log / kick)
  4. T % checksum_interval == 0: server checksum + frame anchor kept (for judging and dumps)
  5. bundle sent to clients as usual

client verifies tick C (a checkpoint) and sends Checksum{C, cs}:
  server cs == client cs        -> nothing
  server cs != client cs        -> the client is the one that diverged (no vote)
       - ServerNote::Desync, room desync counter, server .orrd dump (DumpWriter)
       - Nth wrong checksum within kick_window_secs -> Bye(BYE_KICKED_DESYNC), slot freed
       - otherwise Desync{C, [(slot, cs), (SERVER_SLOT, server cs)]}  to that client only
                   Correction{from_tick C, tick F, checksum, lz4 frame}   (F = newest finalized tick)
       - reports for ticks <= F from that client are ignored (same wrong timeline)
```

The client (`RelayClient`): the `Desync` writes its own `.orrd` as before; the `Correction` is decompressed, validated (tick + checksum) and applied with `Session::restore_confirmed`: the sim, frame ring and anchors are replaced by the server frame at `F`, the session continues from `F` as its verified tick (like a late joiner) and keeps the confirmed inputs it already holds above `F` and its own pending inputs; predicted events are canceled in the returned batch; its recorded checksums from the bad checkpoint on are dropped. If the reliable `Correction` arrives after unreliable bundles above `F` were already verified, the client replays those bundles from its bundle log; if the log does not reach back that far the correction is ignored (`ClientStats::corrections_ignored`) and the next checkpoint is judged again.

"The confirmed inputs since the snapshot" are not sent again: the snapshot is of the newest tick, and every bundle after it flows through the normal confirmed stream (the client's ack is not touched).

## Late join / rejoin

In an authoritative room a joiner gets `JoinSnapshot` from the server's own sim at the last finalized tick (lz4 `Frame::to_bytes`, the late-join format), immediately after `Welcome`. No donor is asked; there is no backlog (the snapshot is at the newest tick). Several joiners at once do not queue. Rejoin by token works as in relay mode.

## Server-only commands (AI, scripted events)

`AuthoritativeConfig::server_slots` lists slots played by the server. Nobody can take them (`SlotTaken`), the room starts without them (`min_players_to_start` is clamped to the player slots). For each such slot `GameSim::with_brain(|frame, tick, slot| (input, commands))` decides the input and commands from the server frame after `T - 1`. They are written into the bundle of `T` with flags 0 (neither repeated nor absent), so every client receives and simulates them identically; clients cannot tell them from a human's. Server-driven slots are mispredicted by clients (they predict "repeat last input") exactly as remote humans are; the cost is rollbacks, never divergence.

Demo: `presets::arena_ai` (moves toward the nearest player, fires every 12 ticks), `orr_server --authoritative --ai-slot 3`; for the physics sample the bot is `bot_input` (test `orr_sample/tests/authoritative_physics.rs`).

## Cheat checks

- `InputValidator` (the relay + validate hook) is unchanged and works in both modes. New: `Verdict::Kick` rejects the input and removes the player in an authoritative room (in a relay room it acts as `Reject`).
- State audit: `GameSim::with_audit(|a: &Audit<G>| -> Vec<Violation>)` sees the frame before and after each tick and the tick's inputs. Example `presets::arena_speed_audit`: a player cannot move more than 9 units in a tick (honest input moves at most 8.5; a forged axis value of 3 moves 18). Violations become `ServerNote::Violation`; at `violation_limit` (default 5, 0 = log only) the player is kicked with `BYE_KICKED_CHEAT`. An audit costs one frame copy per tick.

## Messages and versioning

`PROTOCOL_VERSION` is now 3; decoders accept 2 and 3. A message is encoded with the lowest version that can express it, so **relay-mode traffic is byte-identical to version 2** (a test pins this). Version 3 adds:

| message | change |
|---|---|
| `ServerMsg::Welcome` | trailing `flags` byte, written (header version 3) only when a flag is set; `WELCOME_AUTHORITATIVE = 1`. Version 2 `Welcome` decodes with `flags = 0`. |
| `ServerMsg::Correction` (kind 12) | `from_tick, tick, checksum, data` (lz4 frame); version 3 only |
| `ServerMsg::Bye` | no format change; codes `BYE_BEHIND = 1`, `BYE_KICKED_DESYNC = 2`, `BYE_KICKED_CHEAT = 3` |
| `Desync` reports | slot `SERVER_SLOT` (0xFF) carries the server's checksum |

Client messages are unchanged. A version-2 client cannot join an authoritative room (it fails to decode the version-3 `Welcome`); relay rooms accept old and new clients.

Client side: `ClientEvent::Corrected { from_tick, tick }`, `ClientEvent::Kicked { code }`, `RelayClient::kicked()`, `ClientStats::{corrections, corrections_ignored}`. Server side: `ServerNote::{Correction, ServerSnapshot, Violation, Kicked}`, `RoomStats::{corrections, kicks, violations, server_snapshots, server_ticks}`, `RelayServer::{create_authoritative_room, set_dump_sink, server_checksums}`.

## Costs (release, this 4-core box)

Measured with `cargo test --release` (`orr_sample/tests/authoritative_physics.rs`, `orr_server/tests/authoritative.rs::arena_server_sim_cost_per_tick`), wall clock around the `ServerSim` calls, no network:

| game | `step` per tick (avg / p99 / max) | checksum (per checkpoint) | snapshot (`to_bytes` + lz4) | frame |
|---|---|---|---|---|
| PhysGame, 1000 bodies, 4 paddles, bot slot | 0.98 ms / 2.5 ms / 3.7 ms (budget 16.7 ms at 60 Hz, about 6%) | 0.043 ms | 0.64 + 0.86 ms | 510 KB -> 113 KB |
| arena, 4 slots (1 AI, speed audit on), 6000 ticks | 2.4 us / 3.8 us / 74 us | 3.5 us | 5.7 + 12.5 us | 1.6 KB -> 1.0 KB |

Per tick the server also keeps the checkpoint frame every `checksum_interval` ticks (the `to_bytes` column, about once per 30 ticks, for dumps). The audit adds one frame copy per tick. A correction or a server snapshot costs one snapshot; a diverged client costs one correction per divergence until the kick limit.

## Limits and known gaps

- The server must simulate bit-exactly what the clients do: same game build (`build_id`), config, seed, player count. The room's `config_blob` is what clients feed their `Game::Config`; the server's `GameSim` is built by the host with the same values (no handshake check beyond the room's build hash).
- One sim per room, on the thread that calls `update`; a 1000-body physics room costs about one extra client's simulation on the server.
- A correction is the whole frame (physics 1000 bodies: see costs); corrections are rate limited only by the kick rule and by ignoring reports for the corrected timeline.
- Dumps: the server writes one `.orrd` per judged mismatch (`local_slot = 0xFF`); with no sink they are dropped. The `orr_server` binary writes them to `--dump-dir`.
- The binary hosts only the arena sample authoritatively (it does not link the physics sample); a game of your own, or the physics sample, creates its room through `create_authoritative_room`.
- No ban list: a kicked client may reconnect (its slot is not reserved for it). No hot-patch generation in the server sim.
- Spectators, server-side rollback of cheater inputs (retroactive judgment) and server-side lag compensation are out of scope.
