# Independent generated-project native acceptance

This is an acceptance orchestrator, not a second editor/runtime implementation.
`tools/user_game_native_gate.py` bootstraps the pinned starter, creates a real
independent Cargo workspace in a path containing spaces, and calls the same
`static_game_native_gate.py` and `static_game_standalone_gate.py` drivers used by
the original engine harness. The original engine CI gates remain mandatory.
Missing display, software Vulkan, XTEST or bubblewrap capabilities are failures,
never skipped acceptance.

## Three identities, one generated workspace

- Engine input remains commit `27e53b356b25aa505995f3e8b322093948ef9db0`, tree
  `59158bede83603d7cb272ce7a101b4178956c598`, independently verified against the
  bootstrap's reviewed full Git inventory. It is not relabelled as the user code.
- Hosted gate identity is the actual reviewed `GITHUB_SHA`, a successor of
  bootstrap PR #288's `c6139f889607a6126fdd59cf19885e70a4a09987`.
- Generated source has its own local Git commit/tree, independently generated
  offline `Cargo.lock`, exact package ID and no Git remotes. No user repository,
  credentials, Git hooks or global signing configuration are used.

Before generating the lock and committing, an explicit acceptance-only overlay
restores the standalone test feature/target and installs the three existing
scenario adapters, their support modules and four canonical test scenes. Every
copied file records its previous hash (null if absent), final hash and gate-source
hash. Production `src/` files are unchanged. The library name remains
`orr_static_starter`; the independent package is `orr_independent_acceptance`.
Editor fixtures are independently checked against production scene constants plus
the original authored-scene transformations. Standalone fixtures preserve the
original exact bytes. This does not claim arbitrary user-authored games passed.

`ORR_STATIC_NATIVE_BUILD_SHA` and `ORR_STATIC_STANDALONE_BUILD_SHA` bind compilation
to the user commit; `GITHUB_SHA` remains the hosted gate commit. A strict
`orr.native-execution-context/1` document binds user/engine/gate roots, those three
commits, hidden roots and four committed fixture paths. Its SHA-256 is embedded in
all three integration-test binaries via `ORR_ACCEPTANCE_CONTEXT_SHA256`; runtime
context substitution, omission or injection into an unbound engine harness fails.
All native and replay receipts carry that same context digest.

Cargo metadata must name the generated package's own manifest/workspace and exact
library/test/bin paths. Each compiler artifact must have the same package ID,
exact target name/kind/source and a canonical regular path below the dedicated
fresh target directory. The gate records library artifacts as well as executable
hashes. No cached engine executable is accepted as a user-project producer.

## Source fence and delivered execution

User and gate Git inventories compare every tracked blob and executable mode,
independently of stat caches, assume-unchanged or core.fileMode. Complete scans
reject unknown and ignored inputs, including `src/`, `scenes/`, `tests/`, root
build files and symlinked directories. The pinned engine uses its full immutable
inventory and must not contain an unrecorded target directory. Parent Cargo and
rustup configuration, ambient compiler wrappers/flags and Cargo-home configuration
are rejected. User, engine, gate, dedicated target and evidence roots cannot
contain one another. Inventories, the own lock and context bytes are rechecked
after execution.

The editor retains queued egui input, real App framebuffer capture and independent
PNG/pixel validation. Both delivered players retain XTEST input and XGetImage
framebuffer observations under owned Xvfb and a source-hidden, network-disabled
bubblewrap process. The sandbox must hide user and engine sources, gate checkout,
targets, toolchain, Cargo home, the execution context and known producer paths.
No physical keyboard or physical GPU claim is made. Software Vulkan/Mesa remains
required. The editor-only CPU verifier consumes the exact raw ORRP recordings
produced by those delivered processes, not a regenerated trace. The standalone
player feature graph remains separate from the editor-only verifier.

## CI and bounded execution

The new `generated-user-game-native` job is one Ubuntu 22.04 lane, not a matrix.
It adds one job to the workflow. The existing required checksum aggregate depends
on it and explicitly requires success, including when a dependency fails or skips.
The pinned engine is materialized with `git archive` from the verified local
checkout object/tree, not an unverified source download. Registry preparation may
fetch dependencies; user lock generation and every acceptance build are offline.

The job has a 90-minute operational cap: setup step caps sum to 15 minutes,
acceptance is capped at 70 minutes, and always-attempted evidence upload at five
minutes. Original child command caps total 64 minutes 5 seconds before source
checks, fixture tests and bounded process-group termination tails. These caps are
not measured hosted performance guarantees. Setup state, command output, timings,
partial native receipts and failure details are retained. Forced runner shutdown
can prevent final receipt completion or upload; an unfinished receipt is never a
pass. Ubuntu 22.04 compatibility must be migrated before its documented removal.

Example invocation on a prepared supported hosted runner (all paths must be
canonical, fresh where appropriate, and mutually nonnested):

```sh
python3 -B tools/user_game_native_gate.py \
  --gate "$GITHUB_WORKSPACE" \
  --engine "$RUNNER_TEMP/generated-user-engine" \
  --project "$RUNNER_TEMP/generated user game" \
  --target "$RUNNER_TEMP/generated-user-target" \
  --artifacts "$RUNNER_TEMP/generated-user-native-job/acceptance"
```

Python synthetic tests and CPU compile/check results establish only their stated
boundaries. Actual generated-project native acceptance is established only by a
successful hosted run and validated receipts from this gate; implementing the job
or passing negative fixtures is not equivalent evidence.
