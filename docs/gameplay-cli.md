# Registered gameplay from the CLI

Open a registered project in the editor:

```sh
orr gameplay describe --json
orr gameplay describe act --json
orr gameplay control play
orr gameplay control pause
orr gameplay control step  # one tick; requires paused Play
orr gameplay control play
orr --erp ws://127.0.0.1:7777 gameplay act act --payload '{"press":1}'
orr gameplay act act --payload '{"action":1,"source_cell":0,"target_cell":0}' --player 0 --timeout-ms 10000 --json
```

Intent and payload must match the registered contract; examples are not universal. Authentication uses the existing
`ORR_ERP_TOKEN` or `--token`; prefer the environment to keep tokens out of
process arguments. Payload arguments can still be visible in process listings.
Describe requires `read`; control and act require `read` and `sim_control`.

## Lifecycle and discovery

`describe [intent]` reads the registered contract: intent IDs, optionally the
requested payload schema, identity and current state. It does not mutate or
claim control. Contract/schema output is intentional; unrelated host fields,
connection fences and lease data are not emitted.

`control play|pause|step|stop` uses the **paired editor** lifecycle, not the
separate `sim.*` commands. It submits once using a fresh description, then only
reads descriptions until the requested state is observed: running Play, paused
Play, paused Play at least one tick after the starting tick, or Edit after Stop.
Step requires already paused Play. All confirmations require no error or pending
cleanup. `queued` alone is never success. Output has `status: observed`: concurrent
human/client actions mean this is not exclusive causal proof. Stop keeps the
editor's original source document/history, as covered by the registered fixture.
Use `orr gameplay control stop` when finished.

## Semantic action contract

One connection performs readiness discovery, a single claim, fresh discovery,
one semantic submission, read-only status polling, and one release. Exit 0
requires matching command and connection fences, a completed command with
consumed and later neutral frame receipts in the same stream/play/timeline
scope, and confirmed release cleanup. JSON output retains only the command ID
and validated tick/epoch/checksum receipt fields. It does not expose the
payload, token, lease, scope or raw host error.

`act` does not start, pause, stop, capture or edit the game. Consumption
does not imply winning, a successful purchase, or any other game-specific
outcome. Existing source documents and undo history remain the editor's own.

The whole operation, including connection/authentication and release, has a
100–60,000 ms deadline (default 10,000 ms). The last 50 ms are reserved by the
ERP minimum request timeout; the global `--timeout` does not override it.
A rejected, malformed, stale, uncertain, disconnected or timed-out operation
returns nonzero. No mutation is retried and no connection is reopened.
After an error the client disconnects; normal host disconnect cleanup applies,
but the CLI does not claim completion or successful cleanup.

## Verification

CLI mock tests cover lifecycle/discovery, queued-versus-observed results,
step preconditions, single mutations, stale/error/deadline failures, and
semantic request counts, fresh fences, bounded polling, transient
claim/release states, malformed receipts, permission rejection and disconnect
without retry. The existing localhost WebSocket acceptance fixture has explicit
ignored CLI variants: `gameplay_cli_two_registered_games_consume_and_release`
and, with `stormglass`, `gameplay_cli_stormglass_consumes_summon_and_preserves_source`.
Run them explicitly with `ORR_GAMEPLAY_CLI_BIN` set to the absolute path of the
same candidate's freshly built `orr`; absent paths fail rather than skip.
The two-game fixture also checks describe, read-only denial, Play/Pause/Step/resume/Stop,
using authored projects, fabricated credentials and outcome/source/history checks. These tests are scripted Linux transport acceptance,
not real-model play, window/GPU input or cross-platform certification.

## Local framebuffer capture

`orr gameplay capture --out frame.png --json` authenticates once, reads game/build
identity, and requests `view.screenshot` exactly once on that connection. Requires
`read` and the paired editor framebuffer. `--max-width`/`--max-height` accept
1–2048 (default 2048); the image also has a 1,048,576-pixel and 4 MiB PNG cap.
Bounded base64 and PNG decoding check checksums, dimensions and allocation limits. The incoming transport payload queue is capped at 6 MiB.
`--timeout-ms` (100–60000, default 10000) bounds connection/auth/requests; the host
capture request is at most 5 seconds. Local filesystem operations can take longer.

The destination and all existing parent components must not be symlinks; parents
must already exist. There is no `--force`: existing files, directories and broken
links are refused before connection. Same-directory staging and atomic no-clobber
publication also reject a later destination collision and clean ordinary failures.
This is not crash durability or protection against hostile parent-directory swaps.

Output contains only capture metadata and the local path, never base64 or raw
host errors. A PNG may already be saved if stdout subsequently fails. Capture and
a separate state query are **not atomic**. `epoch` is the PlaySession timeline
epoch, not the gameplay binding epoch. Busy/stale/timeout responses fail without
automatic retry/reconnect; choose a fresh path for an intentional later capture.

The explicit ignored `gameplay_cli_two_projects_capture_actual_wgpu_png` test
uses `ORR_GAMEPLAY_CLI_BIN` for the same candidate and `api-acceptance`: a real
localhost read-authorized CLI captures Delivery/Switchboard's actual offscreen
EditorApp framebuffer, checks project colors and unchanged scene/history/source.
It never retries capture and labels the adapter; software WGPU is not hardware
GPU, OS-window, other-platform or real-model play certification.
