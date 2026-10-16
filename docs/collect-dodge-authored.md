# CollectDodgeV1 authored game slice

This optional adapter follows the [game core](collect-dodge-v1.md). It remains
partial #94/#104. It includes strict admission, an ERP/editor adapter, primitive square rendering,
a standalone window/headless runtime and a Linux trusted-runtime folder exporter.
The full roadmap still needs sprite/UI authoring. The separate default-off
[progress slice](collect-progress-v1.md) adds explicit identity and local high scores.

## Closed, initial-only schema

A schema-2 project selects `collect-dodge-v1`. This versioned game profile is not
a per-project/save identity. Current project admission deliberately rejects UI
and sprite declarations and verifies the entire package lock with route-local
content-only capabilities. Cargo feature unification never enables unsupported
content. Existing Arena project consumers reject this different entry kind.

The existing `orr.scene/1` envelope contains exactly the singleton
`CollectDodgeV1::Run` with `time_limit_ticks`, and 2–49 entities, each with exactly
`CollectDodgeV1::Actor`: `position`, `velocity`, `kind`, `ordinal`. The versioned
game identity determines this closed field contract; arbitrary runtime state is
not an authoring format. Runtime-only initial backups, active flags, score, phase,
elapsed time, goal and restart edge fields are omitted from reflection. Unknown
fields, including those hidden fields, are rejected rather than ignored.

Shared admission requires exactly one kind-0 player (ordinal 0, no velocity),
1–32 kind-1 collectibles (no velocity), and at most 16 kind-2 hazards. Collectible
and hazard ordinals must each be unique and contiguous from zero. Bounds and
motion/time limits are checked through `CollectLevel::new`, then the candidate
Frame receives its derived initial backups, active flags and goal. No mutation
is published if validation fails. Runtime component layout and core golden
checksums do not change merely because reflection is enabled.

Both raw source and canonical serialized scene are bounded to 64 KiB. Names are
at most 128 UTF-8 bytes. The backward-compatible `BakeAdmission::admit_source`
hook defaults to no-op for existing hosts and runs before initial parsing and
`EditorDoc::load_yaml`, including ERP scene.load. Existing atomic bake admission
covers edits, undo/redo, proposals and loads; unsafe play-time reflected debug
edits are prohibited. The size checks prevent accepted edits from saving a
scene that cannot be reopened by the same route.

## Immutable launch and identity

`PreparedScene` owns admitted text/Frame/index. Its simulation and PlaySession
constructors copy that Frame; neither rereads files. In-game restart resets
admitted initial state through deterministic input, while editor Stop/Play
retains the editor document lifecycle. The code/schema build profile is
`orr_remote_host/<version>/CollectDodgeV1` with the existing frame-format binding.
It differs from Arena but does not provide a high-score storage namespace.

The ERP host uses one managed input slot, 60 Hz, seed 42, with x/y and restart input.
No gameplay command, bot, external script, live asset dependency or extra network
transport is introduced. Filesystem opening uses existing bounded regular-file
and project-root helpers; this is not an adversarial concurrent path sandbox.

## Verification and remaining work

Focused acceptance covers initial/edited/saved/reopened Frame parity, actual
PlaySession recording and serialized replay verification/seek across restarts,
malformed/hidden fields and size/name limits, and atomic rejection preserving
source/frame/history. Existing-game no-policy input behavior must remain intact.
Graphical EditorApp controls and exported runtime have separate mandatory
acceptance commands below. Software-GPU captures do not prove physical native
window input or Windows-local acceptance. This base adapter has no persistence;
explicit identity and local high scores require the separate
[progress feature](collect-progress-v1.md).

## Run and edit

Build `orr_editor --features collect-dodge` and open the checked-in
`assets/collect_dodge_project` with `orr_editor --collect-project DIR`.
`--game collect-dodge-v1 --scene FILE` opens a standalone initial scene.
The ordinary hierarchy/inspector, edit transactions, Undo/Redo, Save, Play,
Take control, Stop and reopen use the same host document and shared admission.
During Play, WASD/arrows move, a fresh Space press restarts, and Escape releases
control. The editor displays current score and outcome from the displayed Frame.

Build `orr_sample --features collect-dodge --bin collect_dodge`, then run
`collect_dodge --project DIR`. This dedicated runtime displays the same authored
square actors and actual score/outcome in the window title. It has no bot and
no dependency on the editor. Camera bounds include authored/current positions
and full moving-hazard patrol axes, so collections do not discard their authored
camera extent. The floor depicts the full closed arena.

Window and editor restart inputs retain up to eight fresh taps until a simulation
tick acknowledges them, with an observed neutral sample between taps. Excess taps
coalesce. Focus/control loss cancels pending taps; a new control epoch cannot
resurrect old requests. Movement remains held-state sampling.

`--headless --ticks N` runs 0–6000 ticks. Optional `--hold right,up,restart` supplies
held inputs; omitting it means neutral. `--capture NEW.png` requires an actual
software GPU and never overwrites an existing file. Capture failure is an error;
a failed new-file write may leave that new partial file. The capture contains
game geometry; outcome/score are also printed, not fabricated as HUD pixels.

## Export

Build `orr_sample --features collect-dodge,project-export --bin collect_dodge
--bin orr_export_collect`. Supply the already-built known trusted runtime and its
measured SHA256 to `orr_export_collect --project DIR --runtime BINARY
--runtime-sha256 HASH --trusted-runtime --output NEW_DIR`.
The copied runtime hash and zero-tick smoke establish compatibility with that
project, not code authenticity or source attestation. No download/build/install
occurs. The new `run-collect-dodge` launcher finds its sibling project from any
cwd. The profile shares existing bounded closure, byte rechecks, bounded owned
process-group smoke and Linux atomic no-replace publication with Arena; closed
profile selection preserves existing Arena output/domain/launcher semantics.
Only manifest/scene/active-lock package closure is copied. Settings, old scenes,
source repository, caches and arbitrary root files are excluded. Bundle code
and project may be read-only. Installers, signing, system-library compliance and
other native target exports remain separate.

## Mandatory acceptance

Focused feature tests and strict Clippy are necessary but do not replace these:

- `cargo test -p orr_editor --features collect-dodge --test collect_dodge`
- `cargo test -p orr_editor --features collect-dodge --test collect_dodge collect_actual_editor_gpu_edit_save_play_reopen -- --ignored --exact`
- `cargo test -p orr_sample --features collect-dodge,project-export --test collect_project_export -- --ignored`

Explicit external-tool tests require absolute `ORR_COLLECT_RUNTIME` and
`ORR_COLLECT_EXPORTER` pointing to the actual built production tools, Linux
bubblewrap and software GPU support. Missing prerequisites fail, never skip.
`ORR_COLLECT_CAPTURES` optionally retains evidence. Required CI must positively
check that the exact ignored tests exist and then execute them; default-feature
green checks cannot stand in for these feature gates. Physical native-window
interaction, Windows-local validation and complete #94 acceptance remain open.
