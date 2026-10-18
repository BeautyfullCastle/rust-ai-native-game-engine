# Explicit MeshHazardsV1 editor authoring

Base source: the PR207 tree `cc87fe1501e2584854e3c195aa66b6a9ed46cc58`.
Validation environment: Linux, Rust 1.99.1, software Vulkan llvmpipe
(LLVM 19.1.7, Mesa 25.0.7), 2026-10-19 KST.

## Delivered scope

An explicit `mesh-hazards-v1` editor profile has its own typed stream/identity.
The hazard panel edits bounded fixed-point spheres through admitted host patches,
with exact readback, Undo/Redo, normal scene Save/Open and stale-draft guards.
The viewport renders authored-radius warning bands and loss status. Managed goal
input refuses terminal loss/win and releases existing grants. Legacy scene,
workspace, geometry-reprojection and standalone-export identities stay separate.
Hazard workspace creation and standalone export remain explicitly unsupported.

## Focused results

Observed focused modules contain 38 distinct CPU cases: panel11, game identity1,
backend profile1, managed controls4, shared context1, hazard rendering6 and mesh
play controls14. These were executed across focused development runs, not claimed
as one immutable clean 38-test run. Later fixes were rechecked in affected modules.
Default and mesh-export-only production library checks passed. Strict hazard
library, mesh-export-only library and actual UI-test Clippy passed with warnings denied. A fresh standalone editor binary build and its help command also passed; no separate native-window acceptance is claimed. Whole-workspace and
native Windows UI validation were not run locally.

Actual production EditorApp UI acceptance passed explicitly (1 passed, 0 failed,
0 ignored) using synthetic egui input and a verified CPU Vulkan adapter:

- Invalid radius preserves scene checksum, history and saved bytes; Play blocked.
- Widget Apply, Undo/Redo, Save, fresh local host reopen and Open dialog succeed.
- Actual path and swept polyline equal the fixture's straight segment; both
  endpoints are outside the sphere. Loss occurs at tick1 and suppresses mission
  victory, retaining the pre-tick position.
- Input is disabled after loss; the simulation clock can continue while gameplay
  stays frozen. Stop restores authored checksum/configuration/zero loss and bytes.
- Fresh warning, authored, reopen, Play diagnostic, loss and Stop captures were
  generated. Independent visual review found readable status and correct disabled
  controls; the earlier overlapping status was fixed before this passing capture.

Run04 receipt tracked diff SHA256:
`78f0529068f9762402211972f21c06256834f5ed396f75a6d7502e2af959f2a4`.
The tracked editor production diff still matches that receipt. New-file hashes
are included in the local receipt. Afterwards only a test comment clarified that
reopening creates a host instance, not a separate OS process; workflow/docs and
an unrelated feature-gated animated-path repair were added. No new hardware GPU,
physical OS input, performance, or whole-engine quality claim is made.

## Failures retained

- Initial managed-input profile gates excluded hazards; source review identified
  and corrected them, including terminal-state release and input tests.
- Three of six first hazard-view tests rejected unsupported primitive IDs. The
  producer now uses canonical `#node` IDs; the schema was not relaxed.
- One acquisition fixture timed out in a parallel16-test run. It passed isolated;
  the default mission could finish while acquiring control. The fixture now
  explicitly disables that unrelated mission; no timeout was expanded. The
  timing explanation is an inference, not a captured original terminal state.
- Three of14 existing keyboard tests dropped unapplied egui texture deltas. The
  input-only fixture now uses egui's documented disposal API; assertions remain.
- Export-only compilation initially exposed a shared-context feature gate error;
  common context is now gated by mesh-navigation and its identity data reused.
- First actual UI fixture put its sphere on a start-goal chord rather than the
  actual bent route. It correctly did not lose. The fixed fixture explicitly
  asserts its real path/segment before gameplay; collision tolerances unchanged.
- Earlier actual capture had overlapping hint/status text. Status now has a
  separate wrapped, padded dark panel. Original capture/failed attempts retained.

Focused compiler results were recorded in tool outputs and a written ledger,
not fabricated raw log artifacts. Actual GPU attempts have separate logs,
receipts and PNGs. Local evidence is not represented as public downloadable files.

## Animated relative-path repair

A separate reviewed one-file fix rejects forward-slash rooted hints on all OSes,
while preserving intended `.` / `..` relative forms. Windows considers a rooted
path without a drive non-absolute, so `Path::is_absolute` alone was insufficient.
Four model-binding library tests and strict narrow Clippy passed; both existing
editor integration path-rejection tests passed individually on Linux. The original
PR205 Windows7-pass/2-fail report is preserved. Matching test names were inferred
from unchanged source assertions, not recovered original logs; Windows recovery
requires the new exact-head Windows CI cases and is not claimed here. The exact new workflow library filter `animated_bindings::` selected three cases and passed locally; the separate complete four-case library result includes another module.

## Publication boundary

New Linux/Windows focused CPU and animated-path coverage plus Linux software-GPU
UI acceptance are additive. Existing required CI remains mandatory. Publication,
CI satisfaction, candidate-stack merge and shared-development integration are
separate. Main and PR1 are excluded.
