# Verification resource budget v1

**Status: Proposed design only.** This ADR records the current resource owners,
the measurements needed to choose budgets, and compatibility constraints. It
does not implement a limit, change defaults, or claim any measurements. Budget
values remain **TBD** until the normal-fixture measurement work below is reviewed.

Source basis: shared `su/engine-correctness-hardening` commit
`68ff783a0fa71698c0e75fa26997a458352a8643`. The separate #19 screenshot
candidate is discussed only as a resource-ownership comparison; it is not
part of that shared tree or evidence of verification-budget enforcement.

## Context and current contract

`proposal.verify` and `verify.self` keep their current delayed ERP responses and
report format. An `ErpServer` admits one verification at a time; another call
gets `verify_busy` before frame/input copies or replay decoding. `verify.self`
and proposal verification run from an immutable captured document snapshot;
acceptance of a proposal remains guarded by its existing verified-state stamp.
See `docs/nonblocking-verification.md` and `ErpServer::request_verification`
in `crates/orr_remote/src/server.rs`.

This admission contract covers WebSocket and `LocalConnector` calls serviced
by `ErpServer`. Direct `call_local` remains synchronous and does not acquire
the asynchronous server worker slot. Its shared dispatcher runs
`PreparedVerify::run` on the embedding caller's thread with a local cancellation
flag; asynchronous response, disconnect cancellation and slot reap semantics do
not apply. Measure and label that path separately. A future budget decision
must state whether it also changes embeddings, rather than treating the server
slot as their admission or process-wide concurrency guarantee.

The configured `HostLimits.max_verify_ticks` defaults to 6000 and is surfaced
through `rpc.discover` (`crates/orr_remote/src/dispatch.rs`,
`crates/orr_remote/src/proposals.rs`). It bounds executed ticks. It does not
bound admission-copy cost, full replay decode, time spent in a hook or one
simulation tick, result serialization, or retained report bytes. Scripted
`bot`/`idle` inputs reject a requested count above the configured limit. For
recorded input, `ReplayReader::parse` materializes the complete recording;
`VerifyInputs::tick_list` then applies `max_ticks` to the execution tick list.
It first collects that tick list and only then truncates it, so the execution
cap is not an allocation cap for the list itself.
Thus a long but valid replay can be fully decoded while only its first capped
ticks are simulated. A top-level requested `ticks` value above the limit is
rejected, but `build_inputs` parses recorded input before checking that explicit
value, so even this refusal can incur full replay preparation cost. Omitting
the value does not make the replay parse itself bounded. See
`build_inputs` in `crates/orr_remote/src/proposals.rs`,
`ReplayReader::parse` in `crates/orr_session/src/replay.rs`, and
`VerifyInputs::tick_list` / `verify_frames_cancellable` in
`crates/orr_edit/src/verify.rs`.

The replay decompressor rejects impossible LZ4 size prefixes and bounds the
output relative to compressed block length (255×); it does not impose an
absolute output-byte ceiling (`crates/orr_session/src/wire.rs`). Network
messages have a separate default 4 MiB per-message limit, documented in
`docs/command-admission.md`; this is not an aggregate or local-connector replay
limit. This ADR treats those as current behavior, not a total memory guarantee.

Verification keeps one coordinator thread per admitted asynchronous job. With
`VerifyOptions.parallel` enabled (the default) and at least 16 executed ticks,
the core uses one scoped side thread in parallel with the coordinator,
so at most two engine-owned execution threads serve that job
(`verify_frames_cancellable` in `crates/orr_edit/src/verify.rs`). This is a
per-`ErpServer` limit, not a process-wide or hook-thread limit. The game may
create its own threads. Disconnect sets the cooperative cancel flag; a client
call timeout alone does not. Cancellation is checked around preparation, ticks
and report construction, but cannot preempt a blocked replay decoder, game
hook, or individual simulation tick. The slot stays occupied until actual
worker exit. On server drop, cancellation is requested and the `JoinHandle` is
detached; shutdown does not wait, and a detached worker may still consume
resources while a replacement server exists. No hard preemption or absolute
elapsed-time guarantee exists (`docs/nonblocking-verification.md`; server
`disconnected`, `complete_verification`, and `Drop for ErpServer`).

Each side stores one checksum per executed tick. Sampling also retains metric
series and checksum samples; `series: false` only omits metric-series arrays
from the JSON projection and does not remove the typed report's internal
series. A successful job builds both JSON response data and a typed report
retained in an `Arc<VerifyDetail>` on its activity entry. The default activity
ring holds 2000 entries and evicts by entry count, not bytes; Rust owners of
cloned entries can retain shared details beyond ring eviction. JSON clients
do not own that typed `Arc`. Therefore neither the worker slot nor the
activity capacity is a byte or RSS cap (`orr_edit/src/verify.rs`,
`orr_remote/src/proposals.rs`, `orr_remote/src/activity.rs`,
`orr_remote/src/server.rs`).

## Proposed budget ownership

No numerical resource thresholds are selected in this proposal. The named
owner is the component that must account for a resource if a later change adds
an enforceable bound. A boundary without an existing hard limit is explicitly
marked as unbounded today.

| Resource and owner / measurement boundary | Current accounting / limit | Current excess behavior | Future budget / compatibility decision |
| --- | --- | --- | --- |
| Admission snapshot — host `request_verification` around `prepare_verify` | One asynchronous job per server. Base/candidate frames, options/checks and input data are copied on the host; the registry is shared by `Arc`. No copy-byte or duration cap. | A second job returns `verify_busy` before copy; copy cost has no threshold. | Measure host elapsed and copied/retained byte estimates. A new admission ceiling would reject previously accepted scenes; define discoverability, default/opt-in policy and an explicit refusal before implementation. Estimating size can itself cost host time; this is not hard preemption. |
| Replay preparation — worker `build_inputs`, `ReplayReader::parse`, `decompress_bounded`, then `VerifyInputs::tick_list` | Full base64 decode and parse; decompression has a ratio bound but no absolute verification byte cap. The recorded tick list is allocated in full before truncation. | Existing parse errors remain; ordinary large recordings have no verification-specific absolute size refusal. Explicit excessive `ticks` can be refused after parsing. | Measure encoded, decoded, parsed-owned and pre-truncation tick-list bytes at those boundaries. A new ceiling affects previously accepted valid recordings, including `last_play`; select checks before the corresponding allocation and a documented error. Do not change ORRP or silently shorten decoding. |
| Tick execution and metrics — worker `verify_frames_cancellable` / `run_side` | Default ERP execution cap6000, one or two engine-owned execution threads; checksum comparison every tick, metrics every60 boundaries by default. | Scripted excess returns `verify_limit`; recorded execution is capped. Cooperative cancellation gives no partial success. | Measure each side's execution, checksum and metric calls separately. A proposed time/work ceiling is cooperative only. Preserve deterministic state, golden outputs, default tick semantics and sampling meaning; any early refusal needs a named API error and reviewed default policy. |
| Report and response — worker `assemble` / `report_json`, then host `finish_verification` / `response_ok` | Checksums per executed tick; samples and metric series scale by cadence/metric count. Typed report, JSON tree and serialized response coexist at different stages; no report-byte ceiling. | Existing complete response or error; no size-based truncation. | Measure each representation and host serialization elapsed separately. A future cap can turn previous success into refusal; preserve successful report shape, document error/default negotiation, and never return truncated successful checks. |
| Retained history — host `push_activity` / ring entry lifetime | Default2000 entries by count; shared typed details and Rust clones can outlive eviction. No retained-byte ceiling. | Count capacity evicts oldest entry; no byte-based eviction. | Measure uniquely owned/shared allocations and retention by entry lifetime without double-counting `Arc` references. Earlier byte eviction would change activity availability; define that compatibility policy and its observability before implementation. Outbound JSON copies require their own owner. |
| Physical worker occupancy — spawn, actual exit, host `complete_verification`, server `Drop` | One job slot per server; dropped-server detached workers are outside a replacement server's slot. | Busy until finished worker is reaped; drop requests cancellation but does not join/hard-stop. | Measure cancellation request, observation, actual exit and reap separately. Any process-wide cap changes admission across hosts; define a process-lifetime owner that survives server drop before reusing its permit. A hard deadline needs a separate isolation decision; this ADR adds no cancel/job API or process-wide permit. |
| Direct synchronous embedding — `call_local` dispatcher and embedding caller | The same prepare/run/report stages execute inline; no asynchronous server slot, delayed reply or disconnect owner. Embeddings decide their own concurrency. | Existing dispatcher errors and tick limit remain; server busy/disconnect/reap behavior is inapplicable. | Measure the same stage boundaries on the caller thread and attribute retained results to the embedding. Any new admission/lifetime ceiling needs an explicit embedding owner and compatibility decision; do not promise the asynchronous server's cancellation or capacity contract here. |

The caller timeout, disconnect, cooperative cancellation, host drop and actual
worker exit are different events. There is no public job handle or separate
cancel RPC in this contract. A future budget must not report capacity freed at
timeout/cancel request time: capacity is reusable after actual worker exit is
observed and reaped by that host. Do not introduce force-abort behavior under
this ADR.

| Event | Current action / observable guarantee | Resource and response consequence |
| --- | --- | --- |
| Client call timeout | `Client::call` stops waiting at its own deadline; it does not close transport or issue a cancel RPC. | Verification can continue and retain its slot; this is not a server execution deadline. |
| Requester disconnect | When the host processes the disconnect, `disconnected` sets that job's cooperative flag. | No reply to the removed connection; no replacement worker until the old worker exits and is reaped. |
| Cooperative cancellation flag | Checked around setup, input preparation, tick boundaries and report construction. | Work already inside a decoder/hook/tick can continue until it returns. Cancellation wins over a completed result when reaped; no partial successful report. |
| Server / host drop | Server sets the flag and drops the thread handle without joining. | Shutdown does not wait for an uncooperative worker. The detached worker can outlive this host; a new host has a separate slot. |
| Actual worker exit and host reap | `complete_verification` joins only after `is_finished`. | Original slot is freed, terminal activity is recorded and a connected requester can receive its one response. Cancel intent alone is insufficient. |

## Measurement plan before selecting values

All measurements are future work; this document contains none. Use ordinary,
valid generated scenes and valid recordings only. Retain every attempt, including
cancelled or failed runs, and do not omit slow samples. For each run record:

- exact source commit and tree, fixture source and SHA-256, and all changed
  files;
- OS, architecture, CPU model/core count, available memory, Rust/toolchain and
  dependency versions, build profile/flags, and whether build artifacts were
  warm or cold (cache state is context, not verifier work);
- a serialized measurement lane with no unrelated compile/test/host/GPU load;
- per-stage elapsed time and peak process RSS with the host baseline identified.
  RSS is a process-level observation, not attribution to a particular Rust
  allocation or proof of a whole-machine ceiling; label estimates separately;
- admitted tick count, actual simulated tick count, metric interval, `series`
  setting, `parallel` and whether the 16-tick threshold activates a side thread,
  base/candidate frame sizes, replay encoded/compressed/decoded/parsed-owned
  sizes, pre-truncation tick-list length and bytes, metric count, report size,
  and activity retention observation.

Before execution, record the finite fixture dimensions, call paths and run
order. For each matrix cell, plan one labelled warm-up and exactly three
measured repetitions with the same fixture and options; any different fixed
count must be declared and reviewed before baseline data collection. Retain
warm-up and all attempts separately. Cold-build timing belongs to a separate build record.
Publish every sample alongside the summary, and state the repetition count
instead of inferring a tail bound from it. Agree a finite harness watchdog and
stop-on-first-failure rule before running. A harness stop is not a production
deadline: request cooperative cancellation, wait for actual exit, and report
any worker that does not exit as unresolved; do not admit a replacement sample
or claim that cancellation reclaimed its resources.

Measure stages independently where possible: host admission/snapshot copy;
worker base64/decompression/parse; base and candidate simulation/metrics;
report assembly/JSON encoding; and typed activity retention. If a stage cannot
be isolated without instrumentation, state the method and its attribution
limits instead of presenting total wall time as a component budget. Wait for
actual worker exit after every cancellation trial before starting another
sample or cleaning up a fixture.

The minimum fixture matrix should include:

The `parallel: false/true` comparison below is a core-level `VerifyOptions`
harness comparison. Current ERP verification and direct `call_local` use the
default `parallel: true` and expose no toggle parameter. Their fixture cells
use those supported default options; this plan does not add a public toggle.

| Case | Inputs / options | What it separates |
| --- | --- | --- |
| Short scripted baseline | Small normal scene; bot and idle; short valid tick count; default sample interval; `series` false/true; include `parallel` false/true around 15 and 16 ticks | Admission and fixed costs; serial versus conditional side-thread execution; report option cost. |
| Representative and upper configured tick counts | Normal scenes at representative entity counts and at the configured maximum (default 6000); both default sampling and `sample_every: 1` | Tick scaling, two-side parallel threshold, checksum storage and sampling cost. |
| Valid recorded replay | Engine-generated recordings at representative sizes and one valid recording longer than the execution cap; with omitted `ticks`, an in-limit explicit count, and an over-limit explicit count | Base64/decode/parse and pre-truncation tick-list memory versus capped simulation work. The explicit refusal separately measures preparation before `verify_limit`; do not use malformed or adversarial payloads. |
| Proposal snapshot | Ordinary proposal over small and representative scenes, with a normal check list | Candidate-frame construction/copy and host-thread responsiveness while the worker runs. |
| Report retention | One successful report at default and every-boundary sampling, inspect the detail retained by one activity entry and its lifetime after response/eviction | Typed report retention vs JSON response; characterize count-only ring policy without extrapolating a hard RSS cap. |
| Cooperative cancellation lifecycle | Ordinary run with a controllable test hook that pauses at a documented cooperative boundary; request disconnect/cancel, then release it and wait for actual worker exit | Time-to-observe cancellation, slot occupancy and cleanup semantics. This is not a blocked-hook preemption test. |
| Direct synchronous embedding | The same short scripted and representative valid replay fixtures through `call_local`, with one caller at a time and identical options | Caller-thread preparation/execution/report cost and embedding-held result lifetime; no asynchronous busy, disconnect cancellation or worker reap claim. |

Use deterministic fixtures and verify identical inputs produce identical
checksums/reports before comparing budgets. Keep all existing golden checksums,
default tick limits, mandatory tests and successful report shape intact. Do not
derive a threshold from a single machine or use measurements to relax required
CI assertions.

## Follow-up subtasks and acceptance

1. **Fixture and measurement harness (depends on design review).** Define
   deterministic normal scenes/replays and stage timers/accounting hooks. It
   must emit the complete provenance record and preserve every attempt; do not
   add public API fields merely for the harness.
2. **Baseline measurement report (depends on the harness).** Run the minimum
   matrix in the serialized lane, state peak-RSS and attribution limitations,
   and propose values only with headroom rationale. No runtime guard changes in
   this subtask.
3. **Budget/API decision (depends on reviewed measurements).** Decide which
   caps are enforceable and their owners, defaults, discoverability and error
   behavior. Review compatibility explicitly before implementation. If the
   required deadline exceeds cooperative cancellation, compare a separately
   supervised process with the current worker: startup and IPC cost, serialized
   snapshot/input/report copies, duplicated runtime memory, supervisor lifetime,
   actual process exit/reap, and failure reporting. Select or reject isolation
   in that decision; this design neither implements it nor promises forced
   preemption of arbitrary game code.
4. **Implementation and regression coverage (depends on accepted decision).**
   Add guards at the owning stage, preserve the current API by default where
   required, and test exact boundary/refusal behavior using ordinary valid
   inputs. Cancellation must never yield a partial successful report.

This ADR is not completion of a resource-budget implementation or evidence that
the selected runtime has any additional memory, CPU-time, or process-wide
concurrency guarantee.

## Separate screenshot service

The screenshot service in the separate #19 change is not part of verification
accounting and is not evidence for verification limits. Its one caller slot,
framebuffer/readback bounds and encoder permit have separate owners and
lifecycle; in particular, caller timeout does not release the physical permit
while a GPU event or encoder worker remains outstanding. See
`docs/view-screenshot.md` in that change for its contract. Keep it separate
from any verification worker or replay budget decision.
