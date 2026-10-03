# Arena agent trial contract

The trial harness is separate from [the scripted integration control](arena-agent-workflow.md).
The scripted benchmark remains unchanged and is never counted as a native-agent
success rate. A mock solver can validate protocol, accounting and host integration;
it cannot establish model quality. External-provider uploads and paid calls need
separate authorization before any genuine model trial.

## Five goals and independent evidence

Keep the same five headless goals: guarded two-player authoring, ordinary movement
and input release, a shot and verified recording, a guarded target edit with exact
undo/redo, and a fresh host restart with deterministic reproduction. The detailed
scene, recording and metric checks remain those of the existing baseline.

The runner owns the host, CLI execution, original recordings, observations and
verdicts. A solver submits proposed operations through a constrained request/reply
protocol. Completion is a request for oracle evaluation; it does not supply the
verdict. The runner must independently preserve and compare all 51 checksums for
ticks 0 through 50 and re-verify the original recording after restart. A fresh
recording alone is insufficient.

Solver requests do not carry output paths, oracle implementations or verdict
files. Authoring is forbidden during gameplay, and direct score/bullet edits,
raw ERP escape hatches and direct simulation debug commands are forbidden.
Accepted protocol requests still need the independent goal checks.

## Freeze and retain attempts

Before the first attempt, fix the source/fixture/binary identities, game/build and
schema identity, seed, player count, tick rate, solver/model/version, budget,
attempt count, timeouts, intervention policy and resume policy in a manifest.
Do not change them in response to a failed attempt. Unavailable token/cost usage
is unknown; command counts are not token usage or model tool-call counts.

Retain every raw attempt and transcript. Distinguish ordinary goal failure from
infrastructure interruption, explicit intervention, aborted and unstarted work.
A resume appends a segment to the original attempt instead of erasing it or
resetting its duration, budget or attempt index. Record downtime separately from
solver-active time; elapsed wall time includes interruptions.

Report the denominator explicitly. All scheduled attempts and unresolved cells
remain visible; a rate over completed tasks must name its passed-plus-failed
denominator and disclose excluded interrupted/unstarted cells. A resumed success
does not retroactively remove a prior interruption or intervention.

`unstarted_task_cells` counts scheduled cells that have never begun;
`unresolved_task_cells` also includes started cells without a terminal verdict.
Historical interrupted cells may later be completed by a resume, so interruption
and completion counts overlap and must not be summed as disjoint categories.
The request limit applies to the whole experiment and survives every resume.
Wall time includes each running segment and the downtime between the end of its
cleanup and the next resume, without counting cleanup twice.

## Trust boundary and cleanup

The protocol gives the solver no operation for modifying the oracle or writing
runner artifacts. Source and executable fingerprints and runner-owned evidence
must remain stable and independently checked. Transcript/artifact hashes provide
tamper evidence, not cryptographic attestation of the trial environment.

An arbitrary subprocess under the same OS account can access files or loopback
services outside the protocol. This interface is not an OS sandbox or a guarantee
against such a hostile process. A deployment requiring that guarantee needs a
separate restricted account/container/VM. Keep solver working directories apart
from evidence and scrub inherited ERP settings to avoid accidental cross-talk.

Own and clean up host, solver and CLI processes on success, timeout and exception,
using bounded termination. Record an expected task-five restart separately from
unexpected host death. Preserve interruption records even when cleanup fails.
Linux and Windows validation must retain the raw artifacts and actual cleanup
evidence; mock accounting checks alone do not prove process lifecycle behavior.

## Run a local mock trial

Build the real headless host and CLI once from the trial's source worktree:

```sh
cargo build --locked --release -p orr_remote --bin orr_remote_host -p orr_cli --bin orr
python tools/arena_agent_benchmark.py --self-test
python tools/arena_agent_trial.py --self-test
python tools/arena_agent_trial.py --write-mock-manifest target/arena-trial-manifest.json
python tools/arena_agent_trial.py --manifest target/arena-trial-manifest.json --output target/arena-agent-trial/run-1
```

The same commands work in PowerShell with Python on PATH. The runner resolves
`target/release/orr_remote_host.exe` and `orr.exe` on Windows and extensionless
binaries on Linux. Use a fresh output directory for each new experiment. Keep
the manifest and output inside ignored `target/` so generated evidence does not
change the measured source fingerprint. A manifest is finalized before any solver
attempt; resume uses the saved manifest and verifies the same identities.

For an interrupted experiment, append a resume segment to the same output:

```sh
python tools/arena_agent_trial.py --resume target/arena-agent-trial/run-1
```

Resume requires a persisted `interrupted` status. This version does not
automatically recover a runner killed abruptly or a machine power loss while
its saved status is `running`: unfinished time and budgets cannot be inferred
as a clean continuation. Preserve those artifacts for an unresolved/aborted
handoff instead of changing the status or starting a replacement successful
attempt. The bundled `infra_once` scenario is an explicitly injected local
interruption fixture, not evidence of recovery from a machine failure.

The protocol uses one JSON object per line over solver stdin/stdout. The runner
sends a `start` object naming the attempt, task and fixed goal. A solver sends,
for example:

```json
{"type":"request","id":"discovery-1","op":"read","args":{"what":"status"}}
```

The runner replies with the same id and an `ok` flag, result or error. Available
operations are `read`, guarded `apply`, `sim_start`, `sim_input`, `sim_step`,
`sim_stop`, `verify_replay`, `undo`, `redo`, `save`, `restart_host` and `task_done`. Each has
typed arguments and task/state restrictions. No operation accepts a shell
command, raw RPC method, arbitrary destination path, score edit or verdict.
`task_done` asks the private oracle to inspect the actual host and artifacts.

This version executes only the bundled mock driver, identified as such in the
manifest. Its ordinary failure and interruption scenarios exercise accounting
without a model provider. A genuine solver adapter is a follow-up implementation
for an approved environment; it must use this same protocol and freeze its
identity before execution. Provider permissions and usage reporting remain
separate from harness validation. A recorded token/cost budget with unavailable
usage does not establish that the budget was enforced.

## Validation and artifact handoff

Run the actual-host integration scenarios after the build:

```sh
python tools/arena_agent_trial.py --integration-test --output target/arena-agent-trial-integration
```

The dedicated `arena-agent-trials.yml` workflow runs the scripted control's
existing self-tests, trial self-tests and actual-host mock scenarios on Linux
and Windows. It preserves raw evidence on failure as well as success. These are
additional harness checks; the existing determinism workflow and its mandatory
gate stay in place.

Each experiment retains `manifest.json`, aggregate `trial.json`, append-only
`events.jsonl`, and per-attempt evidence. Per-attempt control directories hold
CLI transcripts, host/solver logs, checkpoints, original recordings and oracle
reports. Do not edit a failed experiment into a success or remove interrupted
segments before sharing results. Hand off the exact source SHA and dirty status,
manifest identity, binary hashes, full artifact directory and CI links together.

Keep local focused checks, the scripted control, actual mock-driven host
integration, and exact-head full CI as separate evidence. Do not report a build
or mock unit test as a real model trial, physical GPU/audio result, or a comparison
with another engine.
