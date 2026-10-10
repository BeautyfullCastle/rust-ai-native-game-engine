# Matched Full+LZ4 / negotiated delta cost probe

## Scope and status

Issue [#14](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/14)
needs a measured trade-off, not an assumption that delta is faster. This probe
originally used the codec and wire contracts from candidate
`c252ae9ae741449ca01976f6b2ce3c54fe5a5600`. The 2026-10-05 elapsed-time evidence
below is preserved. The separately dated Linux calling-thread CPU addendum uses
the later shared-development source identified there.

Instrumentation lives in
[`frame_delta_cost.rs`](../crates/orr_remote/tests/frame_delta_cost.rs). The CPU
probe adds a Linux-only **dev-dependency** edge to already-locked `rustix 1.1.5`
with its `time` feature; no package version/checksum or production dependency
changes. Production codec, client, server, goldens and CI are unchanged.
Existing codec/transport CI results are not counted as this probe's results.
The recommendation remains opt-in, with no default-policy change.

## Fixture and correctness oracle

Build one normal `Simulation::<PhysGame>::new` simulation using `PhysConfig::new(1000, Rain)`,
60 Hz, simulation seed 7, layout seed `0x0DDB_A110`, two players, neutral inputs,
and no spawning. There are exactly 1,000 initial dynamic bodies **plus** the
normal walls, obstacles and paddles. Both modes receive the exact same Frames,
registry, ordering, common delivery metadata and lifecycle cuts. Fixture construction,
physics stepping, restoration and validation are outside the timers.

Each pass contains 82 matched frame pairs:

| Case | Frames | Construction |
| --- | ---: | --- |
| `idle_tick_only` | 33 | Initial snapshot, tick 0 through 32; unchanged payload, not a settled-physics claim |
| `changing_physics` | 33 | Actual neutral-input physics steps 0 through 32 |
| `rollback_resimulation` | 10 | Publish tick 16, restore tick 8 in a new timeline, re-simulate 9 through 16 and compare with saved originals |
| `backward_seek` | 3 | Tick 32, restored tick 8 in a new timeline, tick 9 |
| `reconnect` | 3 | Tick 31, discard endpoint baselines and replace subscription, tick 31 Full, tick 32 |

Every decoded output must match original `Frame::to_bytes()`, tick and checksum.
The regular tests also reject missing and stale delta bases without changing
retained state and recover only through an explicit Full. Every scope change
requires a Full. Baseline caps are checked after every frame; coverage includes
8 MiB, zero, exact serialized size, and one byte below serialized size. Caps
bound each endpoint's one retained serialized baseline, not RSS or transient
allocations. The legacy endpoint retains no delta baseline; both modes still
have their ordinary application Frame and temporary buffers.

These are in-memory data-plane lifecycle tests. They do not replace transport
handshake, queue admission, reset announcement, client mailbox, timeout or
legacy fallback tests. Seek/rollback use authentic restored simulation Frames
with explicit codec scope changes, not a live ERP seek RPC.

## What is measured

Legacy encoding is actual `Frame::to_bytes` plus
`wire::encode_frame_message` (ORRS v1, existing LZ4). Legacy decoding includes
`decode_frame_message` plus `Frame::from_bytes`.

Negotiated encoding uses `Encoder::prepare`, encoding **both** eligible
Full/Delta compressed ORRS v2 envelopes, choosing the smaller successful
message (Full wins ties), and `Encoder::commit`, matching the server's selection
policy. It does not use the codec's uncompressed-size heuristic. Decoding includes
`decode_frame_record_message`, reconstruction/validation, and decoder commit.

Endpoint construction, destruction, explicit reconnect baseline resets, and
common JSON metadata construction are outside both timers. Construction of the
negotiated-only `play_epoch` JSON field is also outside its timer. This measures
per-frame codec operations, not endpoint setup/teardown or total reconnect time. Both modes have the
same deterministic fixed-width send timestamp and empty delivery cut metadata.
Negotiated encoding additionally carries its real `play_epoch` and generated
`frame_codec` metadata. JSON serialization is inside both timers. The fixture's
`timeline: null` and empty lifecycle records are deliberately controlled; a live
host's timeline history and event notes can add bytes and CPU work.

Reported byte lengths include complete ERP binary messages and their JSON/LZ4
headers. They exclude WebSocket/TCP/TLS framing, capability discovery,
subscription requests/acknowledgements, reset-announcement text messages, and
network retransmissions. Thus lifecycle rows report reconstruction/data-frame
cost, **not** total reconnect/seek network cost. Full and Delta counts show when
compression policy actually falls back. There is no byte-savings assertion.

The original wall-clock mode uses `Instant` and reports monotonic elapsed
nanoseconds: encode/decode wall-clock cost proxies, **not exclusive CPU time**. Per-pass mean/p50/p95 include initial Full
and reset Full records. The report gives initial-frame bytes separately; small
three-frame lifecycle percentiles are descriptive, not reliable tail estimates.
No simulation/network/startup latency is included. Retained baseline byte maxima
come from both production codec accessors. That original mode does not measure
calling-thread CPU; the separate Linux CPU mode is documented in the addendum.
Peak heap/RSS remains unmeasured; do not reinterpret retained bytes as a
process-memory ceiling.

## Bounded run and reproduction

Use an existing warmed release target and the coordinator's established profile
and toolchain environment; do not create a cold target just for this probe.
First reserve the build lane and run correctness:

```sh
cargo test -p orr_remote --test frame_delta_cost --release --locked --offline -- --test-threads=1
```

Then independently review the source, ensure no Cargo/rustc/other benchmark is
active, and reserve a quiet measurement lane. The ignored test runs exactly one
unreported warm-up pass per case followed by five measured passes (410 matched
frame pairs). Mode order alternates by pass to reduce first-mode/cache bias.
This is a small diagnostic probe, not a confidence-interval campaign.

```sh
cargo test -p orr_remote --test frame_delta_cost --release --locked --offline \
  measure_full_lz4_vs_negotiated -- --ignored --exact --nocapture --test-threads=1
```

Capture command/environment, source SHA/diff, Rust version, CPU/OS, target path,
profile overrides, start/end time, stdout/stderr and exit status. Preserve failed
attempts separately. JSON lines contain one manifest and 50 case/mode/pass rows,
including first/last fixture checksums. Byte counts and kinds should repeat;
timing naturally varies. Record raw rows before computing summaries. If the
source changes, rerun the affected correctness checks before new measurement.

Follow-up measurements should use the same named fixture and lifecycle cuts,
including the same metadata, caps, warm-up and mode ordering. A live transport
or allocator/RSS study is separate scope; do not claim those costs from this
probe. Report a bandwidth loss or encode/decode regression as observed, without
changing assertions or silently enabling delta by default.

## Measured result: 2026-10-05

Measured source: `87fe8aa98901a33ad41bb46f9664b7f20efba679` (this results section
was added afterward, without changing the Rust source). The single invocation
ran 15:17:59–15:18:00 UTC, reported 1.10 s test duration, and exited 0. It emitted
50 measured rows, covering 410 matched frame pairs. Bytes and Full/Delta counts
were identical across all five passes. All reconstruction assertions passed.

The timing summary is the **median of five per-pass arithmetic means**, in
microseconds per frame. It is not a pooled median, confidence interval, isolated
CPU measurement or latency-through-the-network measurement.

| Case | Legacy bytes / pass | Negotiated bytes / pass | Data-frame saving | Legacy encode / decode µs | Negotiated encode / decode µs | Negotiated Full / Delta |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `idle_tick_only` | 2,451,186 | 95,425 | 96.11% | 394.4 / 328.6 | 641.7 / 136.4 | 1 / 32 |
| `changing_physics` | 2,496,779 | 1,934,411 | 22.52% | 381.9 / 308.1 | 832.6 / 352.1 | 1 / 32 |
| `rollback_resimulation` | 748,769 | 571,401 | 23.69% | 399.7 / 329.4 | 775.2 / 308.7 | 2 / 8 |
| `backward_seek` | 227,974 | 187,567 | 17.72% | 350.9 / 300.4 | 569.8 / 303.4 | 2 / 1 |
| `reconnect` | 234,591 | 231,511 | 1.31% | 389.6 / 305.4 | 602.8 / 346.2 | 2 / 1 |

Each negotiated endpoint retained at most 428,807 serialized baseline bytes
(857,614 bytes summed across encoder and decoder); legacy retains zero delta
baseline bytes. Actual retained bytes varied by snapshot: idle maximum 425,703,
rollback 425,831, other cases 428,807. The configured allowance was still
8,388,608 bytes per endpoint. Neither actual retention nor the configured limit
includes temporary normalized/reconstructed buffers, caller-owned Frames,
records, queueing, server admission reservation, or allocator overhead.

The controlled idle payload saves substantial data-frame bytes and decode time,
but costs more encode time. Changing physics still saves 22.52% of data-frame
bytes here, while encode cost more than doubles and decode cost rises about 14%.
Reconnect's 1.31% data-frame saving is especially small and excludes negotiation
messages, so it is not evidence of an overall reconnect bandwidth gain.

**Recommendation: retain explicit opt-in.** This small fixture demonstrates a
bandwidth/encode/decode/memory trade-off, not a universal speed improvement.
No default enablement, packet-level savings, allocator/RSS bound, exclusive CPU
result, production-readiness claim, or complete issue #14 acceptance follows.
Future scope would need representative live delivery metadata/control traffic,
a wider workload mix, and separately authorized peak-memory/CPU measurements.

### Runner and evidence

- Linux x86_64, kernel 6.18.44; `/proc/cpuinfo` reported AMD EPYC 9V74 80-Core
  Processor. This is the reported CPU model, not an assertion of dedicated
  access to 80 physical cores. `lscpu` could not read the sandbox's CPU topology.
- Rust 1.97.1 (`8bab26f4f`, LLVM 22.1.6); Cargo 1.97.1 (`c980f4866`)
- Existing `su-tui-wss-target` release cache; `CARGO_BUILD_JOBS=2`; no
  `RUSTFLAGS` or `CARGO_PROFILE_*` overrides. Manifest profile: optimized release,
  debug info 1, thin LTO, one codegen unit; default features
- Exactly the two commands documented above, plus targeted strict linting:
  `cargo clippy -p orr_remote --test frame_delta_cost --release --locked --offline -- -D warnings`
- Final focused correctness: 2 passed, 1 ignored, 0 failed; strict Clippy exit 0.
  These are narrow checks, not a workspace or all-target CI result
- The first Clippy attempt exited 101 on `assertions_on_constants` in the
  release-only probe guard. The guard now uses `black_box`; final correctness
  and Clippy were rerun successfully before measurement. The original failure
  log is retained; no measured attempt failed or was discarded
- Known engine workers had returned their compute lanes; only this worker was
  authorized to compute. Owned Cargo sessions completed before the probe.
  Process inspection was sandbox-local, **not** host-wide isolation. No claim
  is made about other tenants, physical CPU isolation, fixed frequency or
  pinned affinity. About 3.0 GiB disk space remained
- Raw stdout/stderr, exit files, environment, initial/final correctness and
  failed/final Clippy logs are retained in the task's `su-delta-cost-logs`
  receipt directory. The measured JSON rows are reproduced verbatim below

SHA-256 receipts:

| Evidence | SHA-256 |
| --- | --- |
| Measured Rust source | `051dcd390eb58609b00b50192087e7869505f33005f92964b8974e20f798a22d` |
| Probe stdout | `2fb632453c612d55e99e32e9a44c271801f6b194069515837fb960a2c063d297` |
| Probe stderr | `76603bec383eef072b2d003659041f4842c03e83a65588f420169034ec3f0df3` |
| First Clippy failure stderr | `98823f83a29a4f094d9e25a923b9f9687ad088004c84f6aca0b12a48b84de192` |
| Final correctness stdout | `b77635b95e38a9090f9eb9f32813932ab235d19bf93b6d7ad0666e9cfa057408` |
| Final Clippy stderr | `5ade5cea87715dd7c4047e618a2b355a709efa38c7fabd7f88daf71e800f38ca` |
| Measured executable | `0da4ff4100431b49c31c3fae8edda8644959228e051b19f36d058f4b22ea127d` |

### Raw measured manifest and rows

The following preserves the JSON output in emitted order. Libtest's surrounding
status lines are excluded; the stdout hash above covers the complete original.
No rounding or aggregation has been applied to these raw rows.

```jsonl
{"probe":"frame_delta_cost_v1","dynamic_bodies":1000,"fixture":"PhysGame Rain / layout_seed 0x0DDB_A110 / sim_seed 7 / 60 Hz / 2 players / neutral input","warmup_passes":1,"measured_passes":5,"timing":"monotonic elapsed wall time; CPU cost proxy, not exclusive CPU time","byte_scope":"ERP binary messages only; excludes transport and reset/negotiation control messages","baseline_cap":8388608,"frame_cap":8388608}
{"pass":0,"case":"idle_tick_only","mode":"legacy_full_lz4","frames":33,"erp_binary_bytes_total":2451186,"erp_binary_bytes_first":74270,"erp_binary_bytes_min":74270,"erp_binary_bytes_max":74283,"full_records":33,"delta_records":0,"encode_wall_ns":{"mean":400214,"p50":357048,"p95":610373},"decode_wall_ns":{"mean":312338,"p50":291321,"p95":404669},"encoder_retained_baseline_bytes_max":0,"decoder_retained_baseline_bytes_max":0,"first_frame_checksum":"0x4df741b9961616ee","last_frame_checksum":"0xba10013b08293fac"}
{"pass":0,"case":"idle_tick_only","mode":"negotiated","frames":33,"erp_binary_bytes_total":95425,"erp_binary_bytes_first":74534,"erp_binary_bytes_min":649,"erp_binary_bytes_max":74534,"full_records":1,"delta_records":32,"encode_wall_ns":{"mean":682856,"p50":613317,"p95":1202318},"decode_wall_ns":{"mean":134016,"p50":110233,"p95":294916},"encoder_retained_baseline_bytes_max":425703,"decoder_retained_baseline_bytes_max":425703,"first_frame_checksum":"0x4df741b9961616ee","last_frame_checksum":"0xba10013b08293fac"}
{"pass":0,"case":"changing_physics","mode":"legacy_full_lz4","frames":33,"erp_binary_bytes_total":2496779,"erp_binary_bytes_first":74270,"erp_binary_bytes_min":74270,"erp_binary_bytes_max":78269,"full_records":33,"delta_records":0,"encode_wall_ns":{"mean":374552,"p50":355085,"p95":503945},"decode_wall_ns":{"mean":301324,"p50":282758,"p95":384018},"encoder_retained_baseline_bytes_max":0,"decoder_retained_baseline_bytes_max":0,"first_frame_checksum":"0x4df741b9961616ee","last_frame_checksum":"0x302d4035feb3319a"}
{"pass":0,"case":"changing_physics","mode":"negotiated","frames":33,"erp_binary_bytes_total":1934411,"erp_binary_bytes_first":74534,"erp_binary_bytes_min":33630,"erp_binary_bytes_max":74658,"full_records":1,"delta_records":32,"encode_wall_ns":{"mean":843050,"p50":805322,"p95":1013579},"decode_wall_ns":{"mean":366127,"p50":389607,"p95":767275},"encoder_retained_baseline_bytes_max":428807,"decoder_retained_baseline_bytes_max":428807,"first_frame_checksum":"0x4df741b9961616ee","last_frame_checksum":"0x302d4035feb3319a"}
{"pass":0,"case":"rollback_resimulation","mode":"legacy_full_lz4","frames":10,"erp_binary_bytes_total":748769,"erp_binary_bytes_first":74876,"erp_binary_bytes_min":74770,"erp_binary_bytes_max":74982,"full_records":10,"delta_records":0,"encode_wall_ns":{"mean":372536,"p50":370357,"p95":406441},"decode_wall_ns":{"mean":320304,"p50":292833,"p95":505908},"encoder_retained_baseline_bytes_max":0,"decoder_retained_baseline_bytes_max":0,"first_frame_checksum":"0xd96d96189db16448","last_frame_checksum":"0xd96d96189db16448"}
{"pass":0,"case":"rollback_resimulation","mode":"negotiated","frames":10,"erp_binary_bytes_total":571401,"erp_binary_bytes_first":75142,"erp_binary_bytes_min":33940,"erp_binary_bytes_max":75142,"full_records":2,"delta_records":8,"encode_wall_ns":{"mean":925891,"p50":847543,"p95":1461652},"decode_wall_ns":{"mean":345516,"p50":345391,"p95":782017},"encoder_retained_baseline_bytes_max":425831,"decoder_retained_baseline_bytes_max":425831,"first_frame_checksum":"0xd96d96189db16448","last_frame_checksum":"0xd96d96189db16448"}
{"pass":0,"case":"backward_seek","mode":"legacy_full_lz4","frames":3,"erp_binary_bytes_total":227974,"erp_binary_bytes_first":78269,"erp_binary_bytes_min":74770,"erp_binary_bytes_max":78269,"full_records":3,"delta_records":0,"encode_wall_ns":{"mean":405987,"p50":419751,"p95":430256},"decode_wall_ns":{"mean":539374,"p50":502043,"p95":805231},"encoder_retained_baseline_bytes_max":0,"decoder_retained_baseline_bytes_max":0,"first_frame_checksum":"0x302d4035feb3319a","last_frame_checksum":"0x304edc190fc4132e"}
{"pass":0,"case":"backward_seek","mode":"negotiated","frames":3,"erp_binary_bytes_total":187567,"erp_binary_bytes_first":78534,"erp_binary_bytes_min":33999,"erp_binary_bytes_max":78534,"full_records":2,"delta_records":1,"encode_wall_ns":{"mean":569769,"p50":610062,"p95":692734},"decode_wall_ns":{"mean":303352,"p50":320965,"p95":359852},"encoder_retained_baseline_bytes_max":428807,"decoder_retained_baseline_bytes_max":428807,"first_frame_checksum":"0x302d4035feb3319a","last_frame_checksum":"0x304edc190fc4132e"}
{"pass":0,"case":"reconnect","mode":"legacy_full_lz4","frames":3,"erp_binary_bytes_total":234591,"erp_binary_bytes_first":78161,"erp_binary_bytes_min":78161,"erp_binary_bytes_max":78269,"full_records":3,"delta_records":0,"encode_wall_ns":{"mean":370017,"p50":361274,"p95":391069},"decode_wall_ns":{"mean":315894,"p50":283089,"p95":382445},"encoder_retained_baseline_bytes_max":0,"decoder_retained_baseline_bytes_max":0,"first_frame_checksum":"0xf3f5797dd53de329","last_frame_checksum":"0x302d4035feb3319a"}
{"pass":0,"case":"reconnect","mode":"negotiated","frames":3,"erp_binary_bytes_total":231511,"erp_binary_bytes_first":78427,"erp_binary_bytes_min":74657,"erp_binary_bytes_max":78427,"full_records":2,"delta_records":1,"encode_wall_ns":{"mean":668576,"p50":560509,"p95":1027721},"decode_wall_ns":{"mean":355589,"p50":367174,"p95":385129},"encoder_retained_baseline_bytes_max":428807,"decoder_retained_baseline_bytes_max":428807,"first_frame_checksum":"0xf3f5797dd53de329","last_frame_checksum":"0x302d4035feb3319a"}
{"pass":1,"case":"idle_tick_only","mode":"legacy_full_lz4","frames":33,"erp_binary_bytes_total":2451186,"erp_binary_bytes_first":74270,"erp_binary_bytes_min":74270,"erp_binary_bytes_max":74283,"full_records":33,"delta_records":0,"encode_wall_ns":{"mean":363426,"p50":340334,"p95":431749},"decode_wall_ns":{"mean":316981,"p50":304170,"p95":418709},"encoder_retained_baseline_bytes_max":0,"decoder_retained_baseline_bytes_max":0,"first_frame_checksum":"0x4df741b9961616ee","last_frame_checksum":"0xba10013b08293fac"}
{"pass":1,"case":"idle_tick_only","mode":"negotiated","frames":33,"erp_binary_bytes_total":95425,"erp_binary_bytes_first":74534,"erp_binary_bytes_min":649,"erp_binary_bytes_max":74534,"full_records":1,"delta_records":32,"encode_wall_ns":{"mean":617136,"p50":591035,"p95":864999},"decode_wall_ns":{"mean":198953,"p50":108631,"p95":378250},"encoder_retained_baseline_bytes_max":425703,"decoder_retained_baseline_bytes_max":425703,"first_frame_checksum":"0x4df741b9961616ee","last_frame_checksum":"0xba10013b08293fac"}
{"pass":1,"case":"changing_physics","mode":"legacy_full_lz4","frames":33,"erp_binary_bytes_total":2496779,"erp_binary_bytes_first":74270,"erp_binary_bytes_min":74270,"erp_binary_bytes_max":78269,"full_records":33,"delta_records":0,"encode_wall_ns":{"mean":441059,"p50":364680,"p95":648349},"decode_wall_ns":{"mean":308058,"p50":284560,"p95":466841},"encoder_retained_baseline_bytes_max":0,"decoder_retained_baseline_bytes_max":0,"first_frame_checksum":"0x4df741b9961616ee","last_frame_checksum":"0x302d4035feb3319a"}
{"pass":1,"case":"changing_physics","mode":"negotiated","frames":33,"erp_binary_bytes_total":1934411,"erp_binary_bytes_first":74534,"erp_binary_bytes_min":33630,"erp_binary_bytes_max":74658,"full_records":1,"delta_records":32,"encode_wall_ns":{"mean":850058,"p50":789077,"p95":1081660},"decode_wall_ns":{"mean":368039,"p50":392050,"p95":524456},"encoder_retained_baseline_bytes_max":428807,"decoder_retained_baseline_bytes_max":428807,"first_frame_checksum":"0x4df741b9961616ee","last_frame_checksum":"0x302d4035feb3319a"}
{"pass":1,"case":"rollback_resimulation","mode":"legacy_full_lz4","frames":10,"erp_binary_bytes_total":748769,"erp_binary_bytes_first":74876,"erp_binary_bytes_min":74770,"erp_binary_bytes_max":74982,"full_records":10,"delta_records":0,"encode_wall_ns":{"mean":529172,"p50":379952,"p95":1962944},"decode_wall_ns":{"mean":309343,"p50":296919,"p95":416756},"encoder_retained_baseline_bytes_max":0,"decoder_retained_baseline_bytes_max":0,"first_frame_checksum":"0xd96d96189db16448","last_frame_checksum":"0xd96d96189db16448"}
{"pass":1,"case":"rollback_resimulation","mode":"negotiated","frames":10,"erp_binary_bytes_total":571401,"erp_binary_bytes_first":75142,"erp_binary_bytes_min":33940,"erp_binary_bytes_max":75142,"full_records":2,"delta_records":8,"encode_wall_ns":{"mean":681784,"p50":722368,"p95":832151},"decode_wall_ns":{"mean":291539,"p50":325051,"p95":447852},"encoder_retained_baseline_bytes_max":425831,"decoder_retained_baseline_bytes_max":425831,"first_frame_checksum":"0xd96d96189db16448","last_frame_checksum":"0xd96d96189db16448"}
{"pass":1,"case":"backward_seek","mode":"legacy_full_lz4","frames":3,"erp_binary_bytes_total":227974,"erp_binary_bytes_first":78269,"erp_binary_bytes_min":74770,"erp_binary_bytes_max":78269,"full_records":3,"delta_records":0,"encode_wall_ns":{"mean":330269,"p50":336959,"p95":337860},"decode_wall_ns":{"mean":300447,"p50":287044,"p95":333613},"encoder_retained_baseline_bytes_max":0,"decoder_retained_baseline_bytes_max":0,"first_frame_checksum":"0x302d4035feb3319a","last_frame_checksum":"0x304edc190fc4132e"}
{"pass":1,"case":"backward_seek","mode":"negotiated","frames":3,"erp_binary_bytes_total":187567,"erp_binary_bytes_first":78534,"erp_binary_bytes_min":33999,"erp_binary_bytes_max":78534,"full_records":2,"delta_records":1,"encode_wall_ns":{"mean":573227,"p50":513960,"p95":782747},"decode_wall_ns":{"mean":307615,"p50":315657,"p95":396407},"encoder_retained_baseline_bytes_max":428807,"decoder_retained_baseline_bytes_max":428807,"first_frame_checksum":"0x302d4035feb3319a","last_frame_checksum":"0x304edc190fc4132e"}
{"pass":1,"case":"reconnect","mode":"legacy_full_lz4","frames":3,"erp_binary_bytes_total":234591,"erp_binary_bytes_first":78161,"erp_binary_bytes_min":78161,"erp_binary_bytes_max":78269,"full_records":3,"delta_records":0,"encode_wall_ns":{"mean":389586,"p50":377458,"p95":414473},"decode_wall_ns":{"mean":292268,"p50":296398,"p95":298471},"encoder_retained_baseline_bytes_max":0,"decoder_retained_baseline_bytes_max":0,"first_frame_checksum":"0xf3f5797dd53de329","last_frame_checksum":"0x302d4035feb3319a"}
{"pass":1,"case":"reconnect","mode":"negotiated","frames":3,"erp_binary_bytes_total":231511,"erp_binary_bytes_first":78427,"erp_binary_bytes_min":74657,"erp_binary_bytes_max":78427,"full_records":2,"delta_records":1,"encode_wall_ns":{"mean":556680,"p50":455344,"p95":788687},"decode_wall_ns":{"mean":340323,"p50":335937,"p95":373943},"encoder_retained_baseline_bytes_max":428807,"decoder_retained_baseline_bytes_max":428807,"first_frame_checksum":"0xf3f5797dd53de329","last_frame_checksum":"0x302d4035feb3319a"}
{"pass":2,"case":"idle_tick_only","mode":"legacy_full_lz4","frames":33,"erp_binary_bytes_total":2451186,"erp_binary_bytes_first":74270,"erp_binary_bytes_min":74270,"erp_binary_bytes_max":74283,"full_records":33,"delta_records":0,"encode_wall_ns":{"mean":418313,"p50":376136,"p95":631114},"decode_wall_ns":{"mean":362491,"p50":300104,"p95":615100},"encoder_retained_baseline_bytes_max":0,"decoder_retained_baseline_bytes_max":0,"first_frame_checksum":"0x4df741b9961616ee","last_frame_checksum":"0xba10013b08293fac"}
{"pass":2,"case":"idle_tick_only","mode":"negotiated","frames":33,"erp_binary_bytes_total":95425,"erp_binary_bytes_first":74534,"erp_binary_bytes_min":649,"erp_binary_bytes_max":74534,"full_records":1,"delta_records":32,"encode_wall_ns":{"mean":659426,"p50":633978,"p95":857868},"decode_wall_ns":{"mean":146804,"p50":108350,"p95":340794},"encoder_retained_baseline_bytes_max":425703,"decoder_retained_baseline_bytes_max":425703,"first_frame_checksum":"0x4df741b9961616ee","last_frame_checksum":"0xba10013b08293fac"}
{"pass":2,"case":"changing_physics","mode":"legacy_full_lz4","frames":33,"erp_binary_bytes_total":2496779,"erp_binary_bytes_first":74270,"erp_binary_bytes_min":74270,"erp_binary_bytes_max":78269,"full_records":33,"delta_records":0,"encode_wall_ns":{"mean":381883,"p50":359902,"p95":535292},"decode_wall_ns":{"mean":302340,"p50":290049,"p95":386491},"encoder_retained_baseline_bytes_max":0,"decoder_retained_baseline_bytes_max":0,"first_frame_checksum":"0x4df741b9961616ee","last_frame_checksum":"0x302d4035feb3319a"}
{"pass":2,"case":"changing_physics","mode":"negotiated","frames":33,"erp_binary_bytes_total":1934411,"erp_binary_bytes_first":74534,"erp_binary_bytes_min":33630,"erp_binary_bytes_max":74658,"full_records":1,"delta_records":32,"encode_wall_ns":{"mean":832551,"p50":824039,"p95":1049232},"decode_wall_ns":{"mean":352139,"p50":369547,"p95":757540},"encoder_retained_baseline_bytes_max":428807,"decoder_retained_baseline_bytes_max":428807,"first_frame_checksum":"0x4df741b9961616ee","last_frame_checksum":"0x302d4035feb3319a"}
{"pass":2,"case":"rollback_resimulation","mode":"legacy_full_lz4","frames":10,"erp_binary_bytes_total":748769,"erp_binary_bytes_first":74876,"erp_binary_bytes_min":74770,"erp_binary_bytes_max":74982,"full_records":10,"delta_records":0,"encode_wall_ns":{"mean":362234,"p50":355856,"p95":436866},"decode_wall_ns":{"mean":329405,"p50":301156,"p95":502883},"encoder_retained_baseline_bytes_max":0,"decoder_retained_baseline_bytes_max":0,"first_frame_checksum":"0xd96d96189db16448","last_frame_checksum":"0xd96d96189db16448"}
{"pass":2,"case":"rollback_resimulation","mode":"negotiated","frames":10,"erp_binary_bytes_total":571401,"erp_binary_bytes_first":75142,"erp_binary_bytes_min":33940,"erp_binary_bytes_max":75142,"full_records":2,"delta_records":8,"encode_wall_ns":{"mean":785479,"p50":779723,"p95":1127468},"decode_wall_ns":{"mean":308682,"p50":318842,"p95":479259},"encoder_retained_baseline_bytes_max":425831,"decoder_retained_baseline_bytes_max":425831,"first_frame_checksum":"0xd96d96189db16448","last_frame_checksum":"0xd96d96189db16448"}
{"pass":2,"case":"backward_seek","mode":"legacy_full_lz4","frames":3,"erp_binary_bytes_total":227974,"erp_binary_bytes_first":78269,"erp_binary_bytes_min":74770,"erp_binary_bytes_max":78269,"full_records":3,"delta_records":0,"encode_wall_ns":{"mean":410073,"p50":414724,"p95":473160},"decode_wall_ns":{"mean":333480,"p50":350148,"p95":361735},"encoder_retained_baseline_bytes_max":0,"decoder_retained_baseline_bytes_max":0,"first_frame_checksum":"0x302d4035feb3319a","last_frame_checksum":"0x304edc190fc4132e"}
{"pass":2,"case":"backward_seek","mode":"negotiated","frames":3,"erp_binary_bytes_total":187567,"erp_binary_bytes_first":78534,"erp_binary_bytes_min":33999,"erp_binary_bytes_max":78534,"full_records":2,"delta_records":1,"encode_wall_ns":{"mean":479098,"p50":390467,"p95":677902},"decode_wall_ns":{"mean":271061,"p50":314536,"p95":350419},"encoder_retained_baseline_bytes_max":428807,"decoder_retained_baseline_bytes_max":428807,"first_frame_checksum":"0x302d4035feb3319a","last_frame_checksum":"0x304edc190fc4132e"}
{"pass":2,"case":"reconnect","mode":"legacy_full_lz4","frames":3,"erp_binary_bytes_total":234591,"erp_binary_bytes_first":78161,"erp_binary_bytes_min":78161,"erp_binary_bytes_max":78269,"full_records":3,"delta_records":0,"encode_wall_ns":{"mean":375197,"p50":358570,"p95":412860},"decode_wall_ns":{"mean":285866,"p50":285893,"p95":287546},"encoder_retained_baseline_bytes_max":0,"decoder_retained_baseline_bytes_max":0,"first_frame_checksum":"0xf3f5797dd53de329","last_frame_checksum":"0x302d4035feb3319a"}
{"pass":2,"case":"reconnect","mode":"negotiated","frames":3,"erp_binary_bytes_total":231511,"erp_binary_bytes_first":78427,"erp_binary_bytes_min":74657,"erp_binary_bytes_max":78427,"full_records":2,"delta_records":1,"encode_wall_ns":{"mean":541971,"p50":416256,"p95":801355},"decode_wall_ns":{"mean":346195,"p50":355306,"p95":368195},"encoder_retained_baseline_bytes_max":428807,"decoder_retained_baseline_bytes_max":428807,"first_frame_checksum":"0xf3f5797dd53de329","last_frame_checksum":"0x302d4035feb3319a"}
{"pass":3,"case":"idle_tick_only","mode":"legacy_full_lz4","frames":33,"erp_binary_bytes_total":2451186,"erp_binary_bytes_first":74270,"erp_binary_bytes_min":74270,"erp_binary_bytes_max":74283,"full_records":33,"delta_records":0,"encode_wall_ns":{"mean":371105,"p50":362647,"p95":446440},"decode_wall_ns":{"mean":338539,"p50":309758,"p95":566247},"encoder_retained_baseline_bytes_max":0,"decoder_retained_baseline_bytes_max":0,"first_frame_checksum":"0x4df741b9961616ee","last_frame_checksum":"0xba10013b08293fac"}
{"pass":3,"case":"idle_tick_only","mode":"negotiated","frames":33,"erp_binary_bytes_total":95425,"erp_binary_bytes_first":74534,"erp_binary_bytes_min":649,"erp_binary_bytes_max":74534,"full_records":1,"delta_records":32,"encode_wall_ns":{"mean":634547,"p50":609521,"p95":823067},"decode_wall_ns":{"mean":133653,"p50":105957,"p95":324460},"encoder_retained_baseline_bytes_max":425703,"decoder_retained_baseline_bytes_max":425703,"first_frame_checksum":"0x4df741b9961616ee","last_frame_checksum":"0xba10013b08293fac"}
{"pass":3,"case":"changing_physics","mode":"legacy_full_lz4","frames":33,"erp_binary_bytes_total":2496779,"erp_binary_bytes_first":74270,"erp_binary_bytes_min":74270,"erp_binary_bytes_max":78269,"full_records":33,"delta_records":0,"encode_wall_ns":{"mean":460041,"p50":375445,"p95":950506},"decode_wall_ns":{"mean":399076,"p50":285572,"p95":359873},"encoder_retained_baseline_bytes_max":0,"decoder_retained_baseline_bytes_max":0,"first_frame_checksum":"0x4df741b9961616ee","last_frame_checksum":"0x302d4035feb3319a"}
{"pass":3,"case":"changing_physics","mode":"negotiated","frames":33,"erp_binary_bytes_total":1934411,"erp_binary_bytes_first":74534,"erp_binary_bytes_min":33630,"erp_binary_bytes_max":74658,"full_records":1,"delta_records":32,"encode_wall_ns":{"mean":815828,"p50":820513,"p95":1053558},"decode_wall_ns":{"mean":349768,"p50":373672,"p95":667417},"encoder_retained_baseline_bytes_max":428807,"decoder_retained_baseline_bytes_max":428807,"first_frame_checksum":"0x4df741b9961616ee","last_frame_checksum":"0x302d4035feb3319a"}
{"pass":3,"case":"rollback_resimulation","mode":"legacy_full_lz4","frames":10,"erp_binary_bytes_total":748769,"erp_binary_bytes_first":74876,"erp_binary_bytes_min":74770,"erp_binary_bytes_max":74982,"full_records":10,"delta_records":0,"encode_wall_ns":{"mean":492189,"p50":464717,"p95":731232},"decode_wall_ns":{"mean":342668,"p50":338290,"p95":434953},"encoder_retained_baseline_bytes_max":0,"decoder_retained_baseline_bytes_max":0,"first_frame_checksum":"0xd96d96189db16448","last_frame_checksum":"0xd96d96189db16448"}
{"pass":3,"case":"rollback_resimulation","mode":"negotiated","frames":10,"erp_binary_bytes_total":571401,"erp_binary_bytes_first":75142,"erp_binary_bytes_min":33940,"erp_binary_bytes_max":75142,"full_records":2,"delta_records":8,"encode_wall_ns":{"mean":775206,"p50":792492,"p95":1218052},"decode_wall_ns":{"mean":358386,"p50":382646,"p95":745373},"encoder_retained_baseline_bytes_max":425831,"decoder_retained_baseline_bytes_max":425831,"first_frame_checksum":"0xd96d96189db16448","last_frame_checksum":"0xd96d96189db16448"}
{"pass":3,"case":"backward_seek","mode":"legacy_full_lz4","frames":3,"erp_binary_bytes_total":227974,"erp_binary_bytes_first":78269,"erp_binary_bytes_min":74770,"erp_binary_bytes_max":78269,"full_records":3,"delta_records":0,"encode_wall_ns":{"mean":347961,"p50":344209,"p95":383397},"decode_wall_ns":{"mean":291975,"p50":288527,"p95":303099},"encoder_retained_baseline_bytes_max":0,"decoder_retained_baseline_bytes_max":0,"first_frame_checksum":"0x302d4035feb3319a","last_frame_checksum":"0x304edc190fc4132e"}
{"pass":3,"case":"backward_seek","mode":"negotiated","frames":3,"erp_binary_bytes_total":187567,"erp_binary_bytes_first":78534,"erp_binary_bytes_min":33999,"erp_binary_bytes_max":78534,"full_records":2,"delta_records":1,"encode_wall_ns":{"mean":561507,"p50":446380,"p95":827133},"decode_wall_ns":{"mean":273605,"p50":319352,"p95":346393},"encoder_retained_baseline_bytes_max":428807,"decoder_retained_baseline_bytes_max":428807,"first_frame_checksum":"0x302d4035feb3319a","last_frame_checksum":"0x304edc190fc4132e"}
{"pass":3,"case":"reconnect","mode":"legacy_full_lz4","frames":3,"erp_binary_bytes_total":234591,"erp_binary_bytes_first":78161,"erp_binary_bytes_min":78161,"erp_binary_bytes_max":78269,"full_records":3,"delta_records":0,"encode_wall_ns":{"mean":455176,"p50":382976,"p95":618785},"decode_wall_ns":{"mean":305411,"p50":291661,"p95":339312},"encoder_retained_baseline_bytes_max":0,"decoder_retained_baseline_bytes_max":0,"first_frame_checksum":"0xf3f5797dd53de329","last_frame_checksum":"0x302d4035feb3319a"}
{"pass":3,"case":"reconnect","mode":"negotiated","frames":3,"erp_binary_bytes_total":231511,"erp_binary_bytes_first":78427,"erp_binary_bytes_min":74657,"erp_binary_bytes_max":78427,"full_records":2,"delta_records":1,"encode_wall_ns":{"mean":643835,"p50":476244,"p95":1039397},"decode_wall_ns":{"mean":336698,"p50":331941,"p95":364219},"encoder_retained_baseline_bytes_max":428807,"decoder_retained_baseline_bytes_max":428807,"first_frame_checksum":"0xf3f5797dd53de329","last_frame_checksum":"0x302d4035feb3319a"}
{"pass":4,"case":"idle_tick_only","mode":"legacy_full_lz4","frames":33,"erp_binary_bytes_total":2451186,"erp_binary_bytes_first":74270,"erp_binary_bytes_min":74270,"erp_binary_bytes_max":74283,"full_records":33,"delta_records":0,"encode_wall_ns":{"mean":394369,"p50":368285,"p95":492138},"decode_wall_ns":{"mean":328569,"p50":301526,"p95":556894},"encoder_retained_baseline_bytes_max":0,"decoder_retained_baseline_bytes_max":0,"first_frame_checksum":"0x4df741b9961616ee","last_frame_checksum":"0xba10013b08293fac"}
{"pass":4,"case":"idle_tick_only","mode":"negotiated","frames":33,"erp_binary_bytes_total":95425,"erp_binary_bytes_first":74534,"erp_binary_bytes_min":649,"erp_binary_bytes_max":74534,"full_records":1,"delta_records":32,"encode_wall_ns":{"mean":641739,"p50":622691,"p95":926460},"decode_wall_ns":{"mean":136358,"p50":107710,"p95":313304},"encoder_retained_baseline_bytes_max":425703,"decoder_retained_baseline_bytes_max":425703,"first_frame_checksum":"0x4df741b9961616ee","last_frame_checksum":"0xba10013b08293fac"}
{"pass":4,"case":"changing_physics","mode":"legacy_full_lz4","frames":33,"erp_binary_bytes_total":2496779,"erp_binary_bytes_first":74270,"erp_binary_bytes_min":74270,"erp_binary_bytes_max":78269,"full_records":33,"delta_records":0,"encode_wall_ns":{"mean":377741,"p50":357008,"p95":497065},"decode_wall_ns":{"mean":344090,"p50":312392,"p95":539929},"encoder_retained_baseline_bytes_max":0,"decoder_retained_baseline_bytes_max":0,"first_frame_checksum":"0x4df741b9961616ee","last_frame_checksum":"0x302d4035feb3319a"}
{"pass":4,"case":"changing_physics","mode":"negotiated","frames":33,"erp_binary_bytes_total":1934411,"erp_binary_bytes_first":74534,"erp_binary_bytes_min":33630,"erp_binary_bytes_max":74658,"full_records":1,"delta_records":32,"encode_wall_ns":{"mean":818150,"p50":816277,"p95":1019749},"decode_wall_ns":{"mean":351698,"p50":369897,"p95":634158},"encoder_retained_baseline_bytes_max":428807,"decoder_retained_baseline_bytes_max":428807,"first_frame_checksum":"0x4df741b9961616ee","last_frame_checksum":"0x302d4035feb3319a"}
{"pass":4,"case":"rollback_resimulation","mode":"legacy_full_lz4","frames":10,"erp_binary_bytes_total":748769,"erp_binary_bytes_first":74876,"erp_binary_bytes_min":74770,"erp_binary_bytes_max":74982,"full_records":10,"delta_records":0,"encode_wall_ns":{"mean":399667,"p50":407422,"p95":541341},"decode_wall_ns":{"mean":353919,"p50":297690,"p95":852811},"encoder_retained_baseline_bytes_max":0,"decoder_retained_baseline_bytes_max":0,"first_frame_checksum":"0xd96d96189db16448","last_frame_checksum":"0xd96d96189db16448"}
{"pass":4,"case":"rollback_resimulation","mode":"negotiated","frames":10,"erp_binary_bytes_total":571401,"erp_binary_bytes_first":75142,"erp_binary_bytes_min":33940,"erp_binary_bytes_max":75142,"full_records":2,"delta_records":8,"encode_wall_ns":{"mean":752687,"p50":779203,"p95":941352},"decode_wall_ns":{"mean":298535,"p50":364489,"p95":465299},"encoder_retained_baseline_bytes_max":425831,"decoder_retained_baseline_bytes_max":425831,"first_frame_checksum":"0xd96d96189db16448","last_frame_checksum":"0xd96d96189db16448"}
{"pass":4,"case":"backward_seek","mode":"legacy_full_lz4","frames":3,"erp_binary_bytes_total":227974,"erp_binary_bytes_first":78269,"erp_binary_bytes_min":74770,"erp_binary_bytes_max":78269,"full_records":3,"delta_records":0,"encode_wall_ns":{"mean":350915,"p50":347474,"p95":369907},"decode_wall_ns":{"mean":290950,"p50":284000,"p95":308156},"encoder_retained_baseline_bytes_max":0,"decoder_retained_baseline_bytes_max":0,"first_frame_checksum":"0x302d4035feb3319a","last_frame_checksum":"0x304edc190fc4132e"}
{"pass":4,"case":"backward_seek","mode":"negotiated","frames":3,"erp_binary_bytes_total":187567,"erp_binary_bytes_first":78534,"erp_binary_bytes_min":33999,"erp_binary_bytes_max":78534,"full_records":2,"delta_records":1,"encode_wall_ns":{"mean":664733,"p50":584374,"p95":1005197},"decode_wall_ns":{"mean":356490,"p50":320564,"p95":531977},"encoder_retained_baseline_bytes_max":428807,"decoder_retained_baseline_bytes_max":428807,"first_frame_checksum":"0x302d4035feb3319a","last_frame_checksum":"0x304edc190fc4132e"}
{"pass":4,"case":"reconnect","mode":"legacy_full_lz4","frames":3,"erp_binary_bytes_total":234591,"erp_binary_bytes_first":78161,"erp_binary_bytes_min":78161,"erp_binary_bytes_max":78269,"full_records":3,"delta_records":0,"encode_wall_ns":{"mean":454859,"p50":471688,"p95":472388},"decode_wall_ns":{"mean":328409,"p50":325731,"p95":352461},"encoder_retained_baseline_bytes_max":0,"decoder_retained_baseline_bytes_max":0,"first_frame_checksum":"0xf3f5797dd53de329","last_frame_checksum":"0x302d4035feb3319a"}
{"pass":4,"case":"reconnect","mode":"negotiated","frames":3,"erp_binary_bytes_total":231511,"erp_binary_bytes_first":78427,"erp_binary_bytes_min":74657,"erp_binary_bytes_max":78427,"full_records":2,"delta_records":1,"encode_wall_ns":{"mean":602754,"p50":391258,"p95":1031015},"decode_wall_ns":{"mean":382162,"p50":376086,"p95":437868},"encoder_retained_baseline_bytes_max":428807,"decoder_retained_baseline_bytes_max":428807,"first_frame_checksum":"0xf3f5797dd53de329","last_frame_checksum":"0x302d4035feb3319a"}
```


## Calling-thread CPU addendum: 2026-10-06

### Timer and unchanged workload

Linux-only `measure_thread_cpu_full_lz4_vs_negotiated` uses the safe,
already-locked [`rustix 1.1.5` time API](https://docs.rs/rustix/1.1.5/rustix/time/index.html):
`clock_gettime(ClockId::ThreadCPUTime)` and `clock_getres` for nominal resolution.
This is **calling-thread CPU time, user + system combined**, including
synchronously called functions and some timer overhead. It excludes other
threads, sleep and descheduled time. It is not exclusive-function CPU, process
CPU, physical-CPU isolation or end-to-end latency. There is no CPU fallback on
non-Linux targets; their existing wall and correctness tests remain available.
The CPU test is absent there, so a filtered invocation reporting zero tests is
not CPU evidence.

A monomorphized clock parameter selects one clock per run. The original wall
mode still uses `Instant` with its original wall labels and manifest. CPU mode
retains exactly the same operation boundaries and object lifetimes described
above. Fixture construction/physics, metadata, explicit resets, endpoint
construction/destruction, assertions, sample-vector updates and JSON reporting
remain outside the timers. Timestamp fields, nanosecond conversion and elapsed
subtraction are checked; invalid or backward readings fail the test.

Before warm-up, CPU mode records 64 back-to-back clock intervals, their
min/upper-median/max and zero count, plus the positive nominal `clock_getres`
resolution. These are timer diagnostics, **not an overhead correction**: nothing
is subtracted from samples. Nanosecond storage and nominal resolution do not
establish nanosecond accuracy. A zero observation means unresolved at this
clock/run, not zero CPU cost.

All five original cases, seeds, 1,000 bodies, metadata, caps and protocol versions
are unchanged: 82 matched pairs per pass, one unreported warm-up per mode/case,
then five measured passes, alternating execution order by pass. The 50 rows
cover 410 matched pairs / 820 frame-mode observations. CPU distributions include
raw samples, count, checked total, integer arithmetic mean, p50, p95 and zero
count under `encode_thread_cpu_ns` and `decode_thread_cpu_ns`. The summary uses
the median of five per-pass means; three-frame lifecycle percentiles remain
descriptive. Byte, record-count, checksum and retained-baseline fields are
control diagnostics rather than a new bandwidth or memory campaign.

### Reproduction and evidence requirements

Use the reserved warmed release lane and established environment described
above. First run the focused ordinary tests and strict Clippy:

```sh
cargo test -p orr_remote --test frame_delta_cost --release --locked --offline -- --test-threads=1
cargo clippy -p orr_remote --test frame_delta_cost --release --locked --offline -- -D warnings
```

After source review, select the exact already-compiled test executable from the
focused test output. Run the following **once**, with an owned-process 30-second
ceiling after compilation:

```sh
timeout --signal=KILL 30s "$TEST_EXE" \
  measure_thread_cpu_full_lz4_vs_negotiated --ignored --exact --nocapture --test-threads=1
```

A timeout/error is failed or incomplete evidence, not permission to rerun for
better numbers. Record the base and instrumented source commit/tree, source,
lockfile, executable and timer-manifest hashes, command/environment, clock
resolution/diagnostics, start/end/status and raw output. Verify 50 rows, 410
matched pairs, exact reconstruction, consistent byte/record/baseline/checksum
controls across passes and all arithmetic summaries before drawing conclusions.


### Measured result and interpretation

The **single** CPU invocation ran on 2026-10-06, 02:23:21.127083–02:23:22.077078
UTC and exited 0. Libtest reported 0.95 s (the external wrapper measured
949,804,565 elapsed ns; this is runner duration, not a CPU sample). It emitted
one manifest and all 50 expected rows. All 410 matched frame pairs reconstructed
exact original bytes, ticks and checksums. Byte totals, Full/Delta counts,
retained-baseline maxima and endpoint fixture checksums were identical across
all five passes; their controls also match the historical report. The raw
1,640 encode/decode CPU samples and all 64 clock-diagnostic intervals are
preserved, with no failed/discarded CPU measurement or number-selection rerun.

The table reports **median of five per-pass arithmetic means**, in µs/frame,
rounded to one decimal only here. These are calling-thread CPU values from
this run, not the historical wall-time values or a same-run wall/CPU ratio.

| Case | Legacy encode / decode CPU µs | Negotiated encode / decode CPU µs | Negotiated Full / Delta |
| --- | ---: | ---: | ---: |
| `idle_tick_only` | 347.0 / 301.0 | 558.2 / 114.5 | 1 / 32 |
| `changing_physics` | 374.5 / 297.4 | 748.9 / 312.8 | 1 / 32 |
| `rollback_resimulation` | 370.2 / 299.8 | 698.6 / 293.6 | 2 / 8 |
| `backward_seek` | 358.7 / 322.3 | 517.2 / 265.0 | 2 / 1 |
| `reconnect` | 356.3 / 295.0 | 512.7 / 326.2 | 2 / 1 |

The trade-off remains clear: controlled idle data-frame bytes fall 96.11%,
while encode CPU rises about 61% and decode CPU falls about 62%. Changing
physics bytes fall 22.52%, while encode CPU is about twice legacy and decode
CPU rises about 5%. The three-frame reconnect case saves just 1.31% of
data-frame bytes while both measured CPU operations cost more; control/transport
messages remain excluded. This is one small workload/runner, not statistical
proof of a universal ratio.

**Retain explicit opt-in.** This addendum supplies the previously missing Linux
calling-thread encode/decode CPU evidence. It does not establish Windows CPU,
exclusive-function CPU, live-network costs, peak heap/RSS, default enablement,
production readiness or whole-issue #14 acceptance. Retained-baseline controls
remain at most 428,807 bytes per negotiated endpoint, not a process-memory
bound. Historical elapsed-time results above remain a separate measurement.

### Runner, timer diagnostics and receipts

- Base: `f0162b9222e42d1b74247ab6aeebd9ff65b86ac3`, tree
  `8136c32e455c8825991426b34f40c245f48a9506`
- Measured instrumented source: `02bfe84387bbb4cb5abf57fb12e2739baaf52e90`, tree
  `e50e59c833801e863dd904c1813362a4475db95f`; results and evidence were added
  afterward without changing the Rust source, manifest or lockfile
- Linux x86_64, kernel 6.18.44, glibc 2.41; reported model AMD EPYC 9V74 80-Core
  Processor. The process could use logical CPU IDs 0–8 according to
  `sched_getaffinity`; that is not nine dedicated physical cores. Cgroup quota
  and effective cpuset files were unavailable, so no quota capacity is claimed
- Rust/Cargo 1.97.1, LLVM 22.1.6; existing `su-tui-wss-target`, default features,
  release optimization/debug=1/thin LTO/one codegen unit, jobs=2; no RUSTFLAGS or
  CARGO_PROFILE overrides. Compilation finished before the bounded invocation
- The coordinator reserved the sole known engine compute lane; focused Cargo
  sessions had completed. Process visibility was sandbox-local, not host-wide.
  No pinned affinity, fixed frequency, physical isolation or absence of other
  tenants is asserted. Available disk space was 1,218,170,880 bytes before and
  1,218,121,728 bytes after the CPU invocation, above the 1 GiB floor
- Nominal `clock_getres`: **1 ns**. The 64 back-to-back intervals had min / upper
  median / max **110 / 120 / 360 ns**, zero count **0**. No overhead subtraction;
  these do not establish 1 ns accuracy. None of the 1,640 operation samples was
  zero; the raw arrays remain authoritative
- Premeasurement focused release correctness: **4 passed, 2 ignored, 0 failed**,
  including invalid timestamp/backward-clock arithmetic tests, exact lifecycle
  reconstruction/bounds and missing/stale-base Full recovery. Strict targeted
  release Clippy: **exit 0**, with no warning or source correction needed
- `cargo metadata --offline --format-version 1` first updated only the intended
  lock edge, then exited 101 while trying to materialize unrelated uncached
  `accesskit_consumer 0.38.0` workspace metadata. The failure is preserved; no
  network download or version update followed. Both focused locked/offline
  checks above then passed. This was not a measured attempt
- A post-run parser independently verified the manifest/counts, all sample
  totals/means/order-statistic percentiles/zero counts and repeatable controls.
  Source review and these focused checks are not workspace/all-platform CI

[Raw manifest and 50 rows](evidence/frame-delta-thread-cpu-2026-10-06/raw.jsonl)
preserve emitted JSON text and order (only libtest's surrounding status text is
removed). [Complete stdout](evidence/frame-delta-thread-cpu-2026-10-06/probe.stdout),
[stderr](evidence/frame-delta-thread-cpu-2026-10-06/probe.stderr),
[path-redacted run identity/command/status](evidence/frame-delta-thread-cpu-2026-10-06/probe-run.json),
[timer manifest](evidence/frame-delta-thread-cpu-2026-10-06/timer-manifest.json),
[redacted build environment](evidence/frame-delta-thread-cpu-2026-10-06/environment.txt),
[CPU availability](evidence/frame-delta-thread-cpu-2026-10-06/cpu-availability.txt),
[correctness stdout](evidence/frame-delta-thread-cpu-2026-10-06/correctness.stdout),
[path-redacted correctness build log](evidence/frame-delta-thread-cpu-2026-10-06/correctness.stderr),
[path-redacted Clippy log](evidence/frame-delta-thread-cpu-2026-10-06/clippy.stderr),
[metadata failure](evidence/frame-delta-thread-cpu-2026-10-06/metadata-attempt.txt),
[validated summary](evidence/frame-delta-thread-cpu-2026-10-06/summary.json) and
[validation output](evidence/frame-delta-thread-cpu-2026-10-06/validation.json)
are retained beside it. JSONL/manifest hashes include their final newline.

Four published runner receipts (environment, run identity, correctness build
stderr and Clippy stderr) replace absolute session-local paths with `$WORKSPACE`,
`$REPO` and `$TARGET_DIR`, and the container hostname with
`[redacted-hostname]`. These placeholders mean the session workspace root,
measurement checkout and existing warmed Cargo target; choose their local
values for reproduction. They are not literal shell paths to execute.
[The transformation manifest](evidence/frame-delta-thread-cpu-2026-10-06/receipt-redactions.json)
records the ordered substitutions, occurrence counts and SHA-256 of both each
original and its published version. Unredacted originals remain in the local
audit archive outside the publication checkout. All other bytes in those four
receipts, plus the complete CPU stdout/stderr, raw samples, timer manifest,
timestamps, source/lockfile and executable hash, are unchanged. No build or
measurement was rerun for this artifact-only privacy cleanup. Hashes in the
table below identify the **published versions**.

| Evidence | SHA-256 |
| --- | --- |
| Measured Rust source | `d01073091c75bf3dfd59ee5cfc346e5ed3366d97280a90eec124fac8b2db7e8e` |
| Cargo manifest | `7abcc69c686f5624c426d2219da9d7a7314f8fe14c0fc8c82e58f6d5b1725a2b` |
| Cargo.lock | `1f5685276b9f9526afca5716a89295978f21fad75a737e67af07b5903002a060` |
| Measured executable | `c4a0061b7ebc1cad91a9f50b4ea98ac579b5da78abe6cd9043252d50d50648a0` |
| Timer manifest | `1da4a27d92ba7a20635b20853e90d95c31f83e17e248b3bb18cd1d8ed42c08b1` |
| Raw JSONL | `af70f44b95b6143e2e65c893158615f0cf4b6ca7ff067ebb0767273d0e5860b4` |
| Complete stdout | `f73974ceea6f19ee4f38cce11a337e3d17aec59cafd289e24d377cf5edfab295` |
| Complete stderr (empty) | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| Run identity and command (path-redacted) | `d94a9ba0f5df30da6ac75c2e6fde9e90e4eff941f7d67c441b568895bd676556` |
| Environment (redacted) | `b6be060e40dc7f86a2c54e6a9dfe25c9626bd2c79a87e89eb703bf682e6a504b` |
| Correctness build stderr (path-redacted) | `5f0b1086e8fc418cc1a668f637d8b3c756379556dc179489029c4b7d39bcd0eb` |
| Clippy stderr (path-redacted) | `38cf1d5fd398c1551f2d561af58b88c624e67cd7c4b5c22f5b6020834992e2a8` |
| Receipt transformation manifest | `12791890e4570e4a851fedfe73f3bb588e7e311060424b87c4c9e5182bba16e0` |
