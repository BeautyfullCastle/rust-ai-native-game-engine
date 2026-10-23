# MeshHazardsV1 export and standalone Player verification

Final local gates passed: 75 CPU cases; each of the three opt-in gates ran
separately and passed 1/1 (native runtime admission, export UI, relocated Player
UI). Standalone CLI expectations passed 11/11. Native-window gameplay remains
unverified for the display limitation recorded below.

Base checkout: `76744dab5b6a226ac80aea5436dbbb7d220c94be` plus the
reviewed export/Player working tree. Linux x86-64; Rust/Cargo 1.99.1;
2026-10-19 KST. This record does not claim Windows execution or hardware-GPU acceptance.

## Scope and reviewed safety boundaries

The separate `mesh-hazards-export` feature admits an explicit three-file hazard
bundle and dedicated `orr_mesh_hazards_player`. The Player verifies its own
executable digest and native platform before opening the admitted MeshHazardsV1
host. It does not execute a runtime supplied by a manifest. Gameplay rendering
and input are reused; legacy Replay, seek and speed controls are not exposed.
Restart uses confirmed Stop/sync/Start/sync and validates paused tick zero,
zero loss/mission progress and input off; Cancel/Escape preserve paused progress.

Independent source reviews closed these findings before final acceptance:

- Runtime reads use no-follow/nonblocking admission; a regular-file-to-FIFO race
  is rejected without blocking. Published staging ownership is released before
  postvalidation, so a recreated old staging path is not deleted.
- A later panel review found the separate pre-export digest reader still used
  plain file opening. It now delegates to hardened bounded bundle IO. Completion
  compares the original entered path form consistently, avoiding a Windows
  canonical-path mismatch. Three new regression cases cover FIFO/symlink swaps
  and completion-input changes. Cancellation coverage here is a predicate/unit
  check, not a claimed end-to-end in-flight worker cancellation test.
- PE32+/ELF64 x64 capability validation requires the exact `.orrhaz` section and
  `ORR_MESH_HAZARDS_PLAYER_V1` marker. ELF alloc + GNU-retain is supported; write,
  execution and unrelated flags remain rejected.
- Hazard-only screenshot output uses atomic `create_new`; physical pixel bounds
  are checked before capture and image bounds/storage before encoding. The
  existing legacy PNG writer was not changed.
- Restart confirmation binds scene identity, simulation epoch and paused snapshot
  sequence. The host resets its epoch to zero on Start, so epoch alone cannot
  identify a replaced session. Ordinary paused pump/sync frames remain valid.
- Five modulo checks and three fixed-size chunk loops received equivalent
  `is_multiple_of`/`as_chunks` lint repairs. Review confirmed nonzero divisors and
  exact table lengths; no parser rules or lint severity were relaxed.

## Final CPU/build results

- `cargo test -p orr_editor --lib --features mesh-hazards-export,mesh-animated-models mesh_hazards -- --test-threads=1`:
  **75 passed, 0 failed, 3 explicitly ignored**. The opt-in cases are native
  executable admission, actual EditorApp export UI and relocated Player UI.
- The native executable admission opt-in was then run separately by exact name:
  **1 passed, 0 failed, 0 ignored**. It checked the supplied SHA-256, bounded file,
  production container/capability parser and native platform, without executing
  the binary. It is not included again in the 75 executed-case count.
- `cargo build -p orr_editor --bin orr_mesh_hazards_player --features mesh-hazards-export`:
  passed. The raw debug artifact exceeds the unchanged 256 MiB admission limit;
  a separate `objcopy --strip-debug` copy is the acceptance runtime.
- `cargo clippy -p orr_editor --bin orr_mesh_hazards_player --features mesh-hazards-export -- -D warnings`:
  passed. The test build retains an existing unused import warning in the
  protected legacy `mesh_workspace/tests.rs`; production strict Clippy is clean.

Core Player coverage comprises 10 launch/parser/hash/profile cases and 10 Linux
session/capture cases, including real loss reset, preserved authored checksum and
stopped recording, raced output file/symlink refusal, high-DPI physical bounds,
and same-scene session replacement rejection. Runtime fixtures: 12; bundle: 13;
export-panel unit cases: 8. These are subsets of the 75, not additional totals.

## Exact artifacts and source freeze

Acceptance runtime:
`/workspace/shared/hazard-player-acceptance-20261019/orr_mesh_hazards_player`

- Size: **116731408 bytes**
- SHA-256: `a0f477820c3aaacfc3c934bdb33fa49485992b92f0c6902f846d290008a841ee`
- Admitted platform: `LinuxX86_64`; exact capability: `ORR_MESH_HAZARDS_PLAYER_V1`
- Raw debug artifact SHA-256:
  `babfa5b8595ad8cc3315806d879912dfb6f378a0101deee27eca9cff02ed7d4c`

The final source inventory `/tmp/hazard-final-v6-frozen.sha256` has SHA-256
`d1a85c45522dbbcf2cacfe0139e3ea69a8a8157a5ee2235dbf8ace6ea348dfa6`.
It covers the new source/tests, relevant hazard sources, Cargo/lib/app hooks and
usage documentation. Every entry remained unchanged across the final CPU cohort
and native-admission run. This verification document and workflow additions are
outside that source inventory. Representative production SHA-256 values:

- Player app: `7494a3d46fc1082f275166cd4d7c92833cfeeeaaa371f115908408cb6edd30b2`
- Player launcher: `2b924efe347b646809fb5c7799a1231cbffc1b527ec5f1eb12f5cd6c609c5027`
- Bundle: `022829d779741b3d2b23a87835fae0c6a56d5d0e523de4ec2fde668a4d8a2ff5`
- Runtime parser: `a1b3352479eaf73faf0237c62bc332fa19a6ac9b5a9ef662da6db472f7ed2f06`

Local logs: `/tmp/hazard-cohort-v6.log`, `/tmp/hazard-native-v6.log`,
`/tmp/hazard-clippy-v6.log`, `/tmp/hazard-build-v6.log`,
and `/tmp/hazard-stability-native-v6.log`. Local paths are evidence locations,
not public download links.

## Actual UI and relocation gates

Final production EditorApp export UI passed separately: **1 passed, 0 failed,
0 ignored**, 48.88 seconds, with exact runtime `a0f47782…` and unchanged v6 source
inventory. It verifies trusted export, exact three-file inventory, overwrite
refusal, dirty-scene blocking, and unchanged source/runtime bytes. Four PNGs,
receipt, run log and source-stability log are preserved under
`/workspace/shared/hazard-export-ui-20261019-v4`.

- Source scene SHA-256 before/after:
  `decedf1d14513e4522e0fea6acd8ece96808810a293fc572f0a1fb2eefb45082`
- Runtime SHA-256 before/after: the exact `a0f47782…` digest recorded above
- Bundle manifest SHA-256:
  `b5ad195361af16851bbf939185c4482b78db8e4ee05486f1b3f23135da6657ca`

Final relocated production PlayerApp UI passed separately: **1 passed, 0 failed,
0 ignored**, 25.07 seconds, with six reviewed captures and llvmpipe software
Vulkan. It used the exported bundle relocated into a path containing spaces;
the source scene, source runtime and checkout were masked in the test process,
and bundle files were read-only and byte-identical before/after.

Synthetic AccessKit/pointer input verified Pause/Resume, a live goal causing
loss, Cancel/Escape preserving paused loss, confirmed reset and repeated restart.
Observed paused tick was 58; loss tick was 46. The test deliberately used the
existing 250-permille clock setting after a fresh paused restart to accommodate
software-rendering wall time, asserting unchanged tick-zero checksum. This is
recorded test pacing, not a Player speed-control feature. UI ran production
`PlayerApp` in libtest, not the standalone executable's native window.

Separately, the actual relocated standalone executable matched **11/11** CLI
exit/text expectations, including help/capabilities, malformed arguments/bundle
rejections and admitted startup reaching the known display error. The latter is
not a successful native screenshot or native gameplay run. Evidence:
`/workspace/shared/hazard-relocated-acceptance-20261019-v4/player-ui/receipt.json`,
`cli-receipt.json`, `player-ui-run.log`, `cli-run.log`, `source-stability.log`
and `harness-stability.log`. Source and artifact hashes remained stable.

Final libtest executable SHA-256:
`3cff68b0eb2743b8c66bd694ece818b09fec096f92f6bd3a9ada14f09908d5f9`.
The three opt-in passes are reported separately from the 75 CPU passes; none was
silently skipped in acceptance. Earlier export-UI v1 (`88541ea7…`), v2
(`0122d2fa…`) and v3 (`771f32af…`) remain intermediate evidence.

Native display startup is blocked in this environment: attempted Xorg `:88`
could not establish listening sockets. The diagnostic log is
`/workspace/shared/hazard-native-acceptance-20261019-v1/xorg-launch.log`.
Software Vulkan/egui harness acceptance must not be described as native OS-window,
physical mouse/keyboard, physical GPU or Windows acceptance.

## Original failures retained

- Minimal-feature **test** compilation exposed a pre-existing protected
  `mesh_export/tests.rs:207` reference to a helper gated by `mesh-animated-models`.
  Tests use that additive feature; minimal production build remains verified.
  Log: `/tmp/hazard-player-tests.log`.
- Initial Player suite: 13 pass/1 fail because its test incorrectly expected a
  new session epoch to increase. The host resets it to zero; the expectation was
  corrected and sequence-bound stale-session checks added, without weakening
  reset/checksum/recording assertions. Log: `/tmp/hazard-player-tests-full.log`.
- Initial panel compilation failed two SHA-256 `LowerHex` formatting calls;
  bytewise hexadecimal formatting fixed them. `/tmp/hazard-combined-tests.log`.
- Parallel cohort: 65 pass/1 existing managed-input acquisition timeout. The exact
  case passed isolated, then all 66 passed serially without production changes.
  Scheduling/timing is an inference, not a proven captured root cause. Logs:
  `/tmp/hazard-combined-tests-v2.log`, `/tmp/hazard-input-retry.log`,
  `/tmp/hazard-combined-tests-serial.log`.
- The first screenshot-fix compile rejected a mutable match guard; restructuring
  that branch fixed it. `/tmp/hazard-player-revision-tests.log`.
- Strict production Clippy initially rejected eight new runtime-parser style
  warnings. The reviewed equivalent repairs passed without lint allowances.
  `/tmp/hazard-player-clippy.log` preserves the failed result.

- Source review reopened acceptance after the earlier 71-pass/3-ignored cohort,
  native 1/1 and export UI v2 pass, finding the panel digest race and Windows path
  comparison defect. Both were fixed/reviewed; final CPU/native/export UI gates
  were rerun against `771f32af…`. The former results are preserved.
- First relocated Player UI attempt reached mission victory before enabling
  goal input, correctly refusing it. The fixture uses a fresh UI restart and
  test-only 250-permille clock pacing without changing authored checksum/tick
  zero; this is not an exposed Player speed control. The next attempt reproduced
  stale AccessKit button text after a single-frame action. Only the test fixture
  was corrected to refresh accessibility frames. Original logs:
  `/workspace/shared/hazard-relocated-acceptance-20261019-v2/player-ui-run.log`
  and `/workspace/shared/hazard-relocated-acceptance-20261019-v3/player-ui-run.log`.
- Actual Player UI then exposed a production defect: after confirmed Stop, Start
  refused with `Wait for the current local hazard scene and admitted graph` and
  left the app in Edit. Newly created authoring panels had not adopted an Edit
  baseline; Play-derived cached blockers were reused before panel refresh. The
  app now refreshes all presentation panels between Stop/sync and guarded Start,
  preserving stopped-record/scene checks and genuine authoring blockers. A
  normal-UI-click CPU integration regression reproduced the exact error before
  the fix and passed afterwards. Logs: `/tmp/hazard-restart-regression-before.log`,
  `/tmp/hazard-restart-regression-after.log`; diagnostic capture/log under
  `/workspace/shared/hazard-relocated-acceptance-20261019-v3/`.
- A test-only acceptance refinement arrived during the v4 CPU run. That run
  passed but its source-stability check detected the change. The frozen v5 CPU
  and native runs were repeated and passed with all inventory hashes unchanged.

No whole-workspace result, Windows recovery, required-CI satisfaction, publication
or merge is implied by these local checks. Main and PR1 remain outside scope.
