# Arena agent-workflow benchmark

This is a **deterministic scripted integration baseline**, not an autonomous-agent
benchmark. It exercises discovery, guarded scene authoring, ordinary game input,
replay verification, undo/redo, persistence, and an actual host-process restart
through `orr`. A passing script establishes that this particular workflow and its
oracles work. It does not establish agent success rate, agent task-completion
latency, or superiority over another engine. A genuine native-agent trial is a
separate experiment described below.

## Run it

Python 3.9+ and built `orr_remote_host` / `orr` binaries are sufficient; the harness
uses only the Python standard library. From the repository root:

```sh
# Build once, outside the timed scripted tasks.
cargo build --locked --release -p orr_remote --bin orr_remote_host -p orr_cli --bin orr

# The default finds binaries in $CARGO_TARGET_DIR/release or target/release.
python3 tools/arena_agent_benchmark.py

# Explicit binaries and a new, non-existing artifact directory.
python3 tools/arena_agent_benchmark.py \
  --host-bin target/release/orr_remote_host --cli-bin target/release/orr \
  --output target/arena-agent-benchmark/my-attempt

# Harness-only regression checks: no Cargo, host, or external services needed.
python3 tools/arena_agent_benchmark.py --self-test
```

The script returns 0 only if all five tasks and the final provenance checks pass.
A failure returns 1 and retains `run.json`, `commands.jsonl`, the host logs, and
`failure.txt`. It never deletes or reuses an existing artifact directory, edits
repository source files, calls an external AI service, or contacts a paid model.
Each run starts its own loopback-only, development-auth host on an available port.
Run it against a stable source tree; concurrent source or binary changes make the
provenance check fail. It copies the blank fixture before authoring, so the
checked-in fixture is never overwritten.

Optional build measurements are explicit and separate:

```sh
python3 tools/arena_agent_benchmark.py --build-mode warm
python3 tools/arena_agent_benchmark.py --build-mode fresh-target
```

`warm` uses the current Cargo target directory. `fresh-target` creates an unused
build directory inside the run artifacts. This measures a fresh **target** build,
not a cache-free machine: registry downloads, toolchains, compiler wrappers, and
system caches may already exist. Any downloads and failures are included in the
build subprocess time. `prebuilt` is the default and records build status as
`prebuilt_unverified` with no invented build duration. Explicit `--host-bin` and
`--cli-bin` are allowed only with `prebuilt`. The harness records `rustc`/`cargo`
versions if available; running a prebuilt binary does not require a compiler.

Use `--human-interventions N --intervention-note '...'` to disclose intervention
before an attempt. Keep failed attempts and reruns as distinct artifact directories;
do not report only the successful rerun. There are no automatic task retries or
unexpected host restarts. Failed readiness probes are recorded separately from
task-command retries. Timing is informational; there is no latency pass threshold.

## Remote visual editor

The compiled editor supports `PhysGame` and `Arena` hosts. Keep the normal local
PhysGame launcher, and attach to an Arena authoring host with:

```sh
cargo build -p orr_editor --bin orr_editor
# Start the Arena host and author its players through the workflow below first.
target/debug/orr_editor --connect ws://127.0.0.1:7777 --select hero
```

The Arena viewport renders players/bullets with the sample's colors, supports
selection and `Position.pos` drags, and uses the same reflected inspector and
shared undo history as ERP agents. Proposal previews show changed outlines and
old-position ghosts. Their viewport and inspector widgets are read-only; the
inspector is explicitly labelled as the live document. Arena hides physics-only
Body creation. The initial camera fits the authored players, while ordinary
edits, seeks and presentation recovery preserve the person's camera.

Play/seek/stop operate on the host. Gameplay input stays with the CLI/MCP's
structured input adapter: the editor does not send raw input or overwrite an
agent's held movement/fire. Replay Viewer sessions allow inspection and timeline
playback, with mutations and Branch disabled; the editor never auto-branches a
Viewer. Local Arena launch, player creation widgets and GUI gameplay keyboard
control are outside this first slice.

Before decoding a frame, attachment requires an explicit supported game, ERP 1,
and the full compiled reflected schema. Main and proposal streams repeat the
game/build/schema check on their own connection. Arena requires fenced stream
delivery; existing PhysGame legacy delivery remains explicitly reported. These
checks establish descriptor compatibility for a closed compiled adapter set,
not an arbitrary dynamic binary ABI or a unique host instance: the default build ID
is version/game/frame-format based, not a hash of all source code.

The Linux sample CI job also requires a real-window smoke under Xvfb and Mesa:
`ORR_REQUIRE_NATIVE_EDITOR=1` runs the normal editor binary against the test's
real Arena ERP host, captures edit and scored-play windows, validates viewport
pixels and unchanged authoritative checksums, and uploads PNG/log/JSON evidence.
Its opt-in `--screenshot-settle` waits for the current paused snapshot, completed
panel refreshes and actual agent-pulse expiry, with a ten-second readiness
deadline. Ordinary `--screenshot --frames N` behavior stays frame-count based.
Default runs (including Windows) explicitly skip that optional window test.
Missing display support or screenshot pixels fails the required CI invocation.

## Host and input contract

The host is started with:

```sh
orr_remote_host --game arena --scene <copied-scene> --seed 42 \
  --players 2 --tick-rate 60 --bind 127.0.0.1:<port> --dev-no-auth
```

The default host game remains the existing physics game. Arena is selected
explicitly. It supplies reflection for `Position`, `PlayerTag`, `Bullet`, and
`Score`, and the metrics `players`, `bullets`, `score_0`, `score_1`, and
`out_of_bounds`. The initial fixture, `scenes/arena_blank.scene.yaml`, contains no
entities and a zeroed eight-slot `Score.kills` singleton. Arena has no configured
verification bot; use explicit `--idle`, `--last-play`, or `--replay` inputs.

Discover the current host rather than relying on the physics-pinned
`docs/AGENTS.md`:

```sh
orr --json status
orr --json schema --types
orr --json schema --input
orr agents-md
```

`schema --input` calls ERP `registry.input`, whose response is
`{ "schema": ..., "value_format": ..., "max_players": 8 }`. Arena `sim.start`
enforces that player limit. Ordinary Arena input is a complete object:

```json
{"axis_x":1,"axis_y":0,"buttons":[]}
```

Both axes are fixed-point values in `[-1, 1]`. `buttons` is a list of flag names;
`["fire"]` fires. An input value is held for that player until replaced. All
fields are supplied, including a neutral release:

```sh
orr sim start
orr sim input --player 0 '{"axis_x":1,"axis_y":0,"buttons":[]}'
orr sim step 10
orr sim input --player 0 '{"axis_x":0,"axis_y":0,"buttons":[]}'
orr sim step 5
orr sim stop
```

The structured path is ERP `sim.input_value { player, value }`, returning
`{ "ok": true }`; the existing raw `sim.input` ABI remains available and
unchanged. Raw `sim.input` does not derive game commands, because existing raw
bridges submit those commands explicitly. For each player slot, the most recently
admitted input path selects whether structured FIRE-to-command derivation is
enabled. This prevents a raw bridge from accidentally creating duplicate shots.
The benchmark uses only the structured path. Sessions start paused and use
manual steps, never wall-clock play. FIRE is pressed for exactly one tick and
then released; leaving it held would request a shot each subsequent tick.

The Arena implementation moves six units per tick at an axis of one. It maps
ordinary FIRE input to a game command, auto-aims at the other player, advances
bullets at thirty units per tick, and increments hit counters on collision.
`Score.kills` is a historical field name: the players do not die. This benchmark
does not claim to test health, a win state, or an in-game restart button.

## The five tasks and independent oracles

The exact expected values below are derived from Arena's source and become
measured results only when a run's saved oracle evidence passes. No result is
inferred from elapsed time, a process having started, or the presence of a file.

### 1. Author a two-player scene through guarded apply

Starting with the blank fixture, one `orr apply` creates:

- `hero`: `Position.pos = [-300, 0]`, `PlayerTag.slot = 0`
- `target`: `Position.pos = [300, 0]`, `PlayerTag.slot = 1`

The apply uses an explicit one-tick idle verification and checks two players,
zero bullets, zero scores, and no out-of-bounds entities before guarded
acceptance. The harness also reads the authored scene and history: exactly two
entities, unique slots 0 and 1, exact positions, all eight score slots zero, and
exactly one accepted history entry. It resolves the actual entity GUIDs from the
returned scene and stores the baseline document checksum and YAML.

Example (use the actual ERP URL from the run):

```sh
orr --json apply 'create two-player arena' \
  spawn --name hero 'Position={"pos":[-300,0]}' 'PlayerTag={"slot":0}' \
  spawn --name target 'Position={"pos":[300,0]}' 'PlayerTag={"slot":1}' \
  --idle 1 --check 'players.final == 2' --check 'bullets.final == 0' \
  --check 'score_0.final == 0' --check 'score_1.final == 0' \
  --check 'out_of_bounds.final == 0'
```

`apply` must report `accepted: true`, a passing verification, and a history ID.
The CLI uses `proposal.accept_verified` with the captured verification state;
there is no fallback to unguarded acceptance.

### 2. Move, release, and preserve the authoring document

Start baseline play, give slot 0 right input for ten ticks, and read position
`[-240, 0]`. Replace input with neutral and step five more ticks: the position must
still be `[-240, 0]`. Scores and bullets remain zero, slot 1 stays `[300, 0]`,
and stopping play must leave the authoring document checksum unchanged.

### 3. Fire once and verify the baseline recording

A fresh baseline session starts at tick 0 with zero score and no bullets. Press
FIRE for one tick, release, and step nineteen more ticks. At tick 20:

- `score_0 = 1`, `score_1 = 0`, all other score slots zero
- Two players, zero bullets, zero out-of-bounds entities
- Hero and target remain at `[-300, 0]` and `[300, 0]`

Export with `orr sim stop --replay-out baseline20.orrp`. Existing destinations are
refused before stopping play; choose a fresh filename to preserve the original
recording. Use `--force` only when deliberately replacing that destination.
Exports stage complete bytes locally before committing; they do not create
parent directories or guarantee crash durability. Independently run
`orr verify --replay baseline20.orrp --check recording_matches` plus `no_divergence` and final metric
checks. Require identical baseline/candidate reruns, 21 recorded checksum comparisons (tick 0 through 20), zero
mismatches, and zero replayed debug commands.

### 4. Prove a behavioral change, then undo and redo exactly

With play stopped, guarded-apply `target.Position.pos = [900, 0]` using the
**same saved twenty-tick recording**. Require baseline `score_0.final == 1` and
candidate `score_0.final == 0`, while retaining two players. The baseline must
still reproduce all 21 recording checksums. A checksum divergence alone does not
prove the desired behavior; metric checks do.

Start a fresh modified-scene session, FIRE for one tick, then neutral for 49 ticks.
The resulting tick-50 score must be 1 for slot 0, with two players, no bullets,
no out-of-bounds entities, and unchanged positions. Export the fifty-tick replay
and independently verify it. Collect every live checksum from tick 0 through 50.

After stopping, `undo` must restore the exact baseline document checksum and
scene facts. `redo` must restore the exact modified document checksum and facts.
This tests authoring undo/redo, not rollback of a running game.

### 5. Persist and reproduce in a new host process

Save the modified scene to the run's working YAML, terminate the host, wait for
it to exit, and spawn a new host process with the same binary, game identity,
seed, player count, and tick rate. A new play session must begin at tick 0 with
zero score, no bullets, and the same initial checksum. The restarted host's undo
history must be empty; the scene is reloaded from disk.

Repeat the same ordinary inputs and compare **all 51 live checksums** at ticks
0–50 against task 4, not just the final checksum. Independently verify the
**original** fifty-tick recording on the fresh host. Require
`recording.checked == 51`, `recording.mismatches == 0`,
`recording.first_mismatch == null`, and `debug_commands_replayed == 0`.

## Preventing shortcuts and overstated claims

Authoring and gameplay phases are separate. During gameplay the harness allows
ordinary player input, manual timeline control, and reads. It never assigns a
score, spawns a bullet directly, edits a scene component, or calls `sim.command`
to manufacture a hit. The game itself may create its normal game commands from
FIRE input. `debug_commands_replayed == 0` prevents a debug scene edit from being
silently replayed as the alleged gameplay result.

This is an oracle/harness policy, not a claim that every ERP endpoint globally
enforces the same authoring/gameplay restrictions. The benchmark has no generic
raw-RPC escape hatch. The command transcript makes its actions auditable.

The script reads the uncompressed `.orrp` header and checks game ID, seed, player
count, tick rate, 24-byte Arena input size, and replay build hash. A replay's
`build_hash` is `orr_sim::build_hash_of(host_build_id, 0)`, not the host build ID
itself. It rejects the untracked zero build identity. The engine verifier checks
the recording body and deterministic reproduction; the header identity oracle
is explicit in this script and is not described as universal engine enforcement.

The default host build ID derives from package/game/frame-format identity. It is
not a cryptographic identity of every source revision. Therefore each run also
records Git commit and HEAD tree, dirty status/diff, a SHA-256 of source worktree
paths/content (including untracked non-ignored source), and SHA-256 identities of
both executables, scene files, input manifest, and all evidence artifacts. OS,
architecture, and toolchain versions are also recorded. Source
and executable identity are checked again before declaring success. This records
provenance; a prebuilt binary alone does not prove which source compiled it.

Verification's metric sampling defaults can skip intermediate ticks in short
runs. These tasks use exact final metric checks and separately collect every-tick
checksums where claimed. They do not interpret `.max` over sparse samples as an
all-ticks safety proof. The movement task deliberately uses a multi-tick step;
the fifty-tick restart test deliberately uses single-tick steps.

Use `orr verify --last-play --sample-every 1 --check "bullets.max == 0"`
(or `orr apply ... --sample-every 1`) when asserting about transients at tick
boundaries. MCP accepts `sample_every: 1`. The default remains 60 ticks plus
start/end; `--sample-every 0` requests endpoints only. In the normal ±300 Arena
scene, a shot fired once and replayed for 20 ticks has sampled `bullets.max == 0`
at the default interval, but `bullets.max == 1` with interval 1. Both runs still
have final score 1 and reproduce the recording exactly. This is a sampling limit,
not a different simulation result. Even interval 1 cannot observe an event
created and removed entirely within a tick.

Reports include `metric_sampling.requested_interval`, actual `sample_count`,
`scope` (`sampled_tick_boundaries`), and `every_tick_boundary_observed`. The
coverage flag is based on actual samples, so a one-tick run covers every boundary
even with endpoints only. Sparse reports and min/max check reasons warn that
between samples is not checked; the warning does not make missed transients
observable. Checksum divergence remains every-tick, independent of sampling.

Compatibility: existing JSON fields, metric min/max meanings, and the default
interval of 60 are unchanged. The Rust `orr_edit::VerifyReport` has an additional
`sample_every` field so direct and activity reports retain the requested interval.
Downstream Rust code constructing that public struct with a literal must supply
the new field; this is not fully Rust source-compatible. Simulation and replay
formats and checksums are unchanged.



## Evidence and timing

Each new run directory contains:

- `run.json`: task verdicts and wall durations, build status/duration, actual host
  readiness records, failures, retry/restart/intervention counts, source and
  binary identity, artifact SHA-256 manifest, and scope limitations
- `commands.jsonl`: every command's argv, stage, working directory, UTC start,
  elapsed duration, return code, stdout/stderr or launch/timeout failure; host
  spawn and stop events are also recorded
- `host-1.log`, `host-2.log` and discovery/schema JSON: process and readiness
  evidence; readiness requires successful status and input-schema responses
- `AGENTS.arena.md` and type-schema output generated from the actual Arena host
- Blank, baseline, modified, and working scene YAML; original and fresh `.orrp`
  recordings with parsed header JSON; the exact input manifest
- Apply and verifier JSON, scene observations, undo/redo snapshots, and task 4/5
  arrays of all 51 checksums
- `source.diff` and, on failure, `failure.txt`

Task time is monotonic wall time from task entry through its oracle and artifact
writes, including CLI process launch and IPC. Host startup time is spawn through
successful status and input-schema readiness, recorded separately. Build time is
subprocess spawn through exit, also separate; no Cargo compilation is hidden
inside task timing. Total script wall time includes setup, build (if requested),
tasks, and cleanup. Build stdout/stderr are retained in `commands.jsonl`.

No LLM calls are made. Model, token, and cost fields are null, rather than
invented measurements. CLI invocation counts are not model tool-call counts.
Keep all attempts when calculating success rates or distributions; a single
script run is not a statistical agent evaluation.

## Later native-agent trial

Only after the scripted baseline is demonstrated should a separate native-agent
trial be run. Give the agent the five goals, the running host's generated guide,
and the same supported CLI/ERP interface. Keep the detailed validation/oracle
outside the agent's control. Freeze the source, fixtures, binary identities,
seed, allowed actions, intervention policy, timeout/budget, and attempt count
before running trials. Record the model/version and available token/cost usage;
mark unavailable usage as unknown. Do not send private source to an external
provider or start paid calls without authorization.

The agent trial should log its actual commands, rejected proposals, errors,
retries, resets, interventions, and per-task completion time through independent
oracle acceptance. Retain failed attempts and distinguish task failures from
infrastructure failures. Compare against another tool only with equivalent goals,
inputs, allowed actions, budgets, provenance, and independently checked outputs.
The scripted harness is the reproducibility control, not evidence of that later
agent's reasoning or success.

## Future full-stack coverage

Graphics, audio, networking, and asset workflows require separate runtime
oracles and artifact fields: for example rendered-frame evidence, audio-output
checks, multi-peer network traces, and asset-import/build results. Those can be
added as separately scoped experiments with their own environments and
reproducibility requirements. This script proves none of those capabilities or
full-stack superiority; its scope is the five headless Arena workflow tasks above.
