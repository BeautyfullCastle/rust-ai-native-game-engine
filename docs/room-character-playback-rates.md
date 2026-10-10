# Room character playback rates

Bounded follow-on for #100, tracked under #94. Source starts from local cubic
commit `de88bba4416d0b6342e075e32a3a749c30413646`; its camera prerequisite
`d39f026` has separate acceptance. Passing the cubic slice does not imply that
the complete prerequisite stack or this slice has passed acceptance.

## Contract

- Schema 1 keeps the exact existing serialized fields and fixed 1x semantics.
  The generated character template remains schema 1.
- Schema 2 requires `speeds`, a strict object with searching, carrying and
  escaped strings. The closed choices are 1/4x, 1/2x, 1x, 2x and 4x.
- Object-only decoding preserves duplicate detection and rejects unknown or
  missing fields, nulls, arrays, numeric speeds and enum-shaped objects.
- State still comes from the current authoritative Frame. The existing shared
  `room_character::sample_pose` consumes the corresponding speed for both the
  editor's placements and the real runtime/export renderer.
- Clip time is absolute tick / tick rate multiplied by the selected exact
  power-of-two ratio. Integer period reduction happens before float conversion;
  fractional periods and adjacent huge ticks retain their phase. The modular
  path uses at most 112-bit products. Oversized integral periods safely exceed
  all representable ticks. Zero duration returns zero; invalid bounds/rate fail.
- Existing fixed-1x `sample_clip_time` remains the default wrapper. No ECS,
  replay, checkpoint, simulation rate, wall clock, accumulated delta, transition
  history, crossfade, graph, runtime caller or dependency changes are involved.

## Authoring and verification

The real panel exposes one speed menu per state. A changed speed upgrades its
candidate to schema 2; Apply validates and installs the document. Undo/Redo
restore clips, speeds and schema together. Save uses the existing paired
character/model persistence and external-change fences. A rate-only edit keeps
the model binding descriptor unchanged.

Focused tests cover strict legacy/schema-2 admission, all five rate choices,
distinct state rates, real changed poses, huge ticks, tiny/zero/extreme durations,
and unchanged Frame bytes. The editor harness drives actual widgets through
Apply/Undo/Redo/Save/reopen and compares an independent known-time model pose;
it repeats Seek, Stop and restart and compares simulation bytes with legacy 1x.

Explicit GPU tests require `ORR_REQUIRE_GPU=1`. The editor test compares the
production viewport's legacy and authored-rate pixels at an identical paused
tick. The source-hidden export test requires exact production runtime/exporter
and App-probe binaries plus `ORR_REQUIRE_PROJECT_ISOLATION=1`; it compares exact
source/export PNGs, checks a speed-only legacy negative control for searching
and carrying, and reuses the existing exported native App's actual-input
searching/carrying/escaped/restart probe. The latter is an App harness, not an
exported CLI second-press or native-window interaction claim.

At implementation time these new acceptance cases are source-reviewed and
await the coordinator's build/storage lease. No new execution result is implied
by this description, and no physical-GPU coverage is claimed.
