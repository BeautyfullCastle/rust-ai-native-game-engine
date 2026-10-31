# Registered gameplay from the CLI

Open the registered project in the editor, then use its ERP endpoint:

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

The intent and payload must match that registered game's existing contract.
The examples are not universal actions. Authentication uses the existing
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
The two-game CLI fixture also performs describe, read-only denial, Play/Pause/
one-tick Step/resume/Stop. They reuse authored projects, fabricated credentials and outcome/
source/history checks. These tests are scripted Linux transport acceptance,
not real-model play, window/GPU input or cross-platform certification.
