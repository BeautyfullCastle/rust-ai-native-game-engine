# Registered gameplay projects

The default-off `orr_editor/static-gameplay` feature adds
presentation and input contracts alongside `static-games`. Existing
`StaticGameDescriptor`, `StaticGameRegistry`, held/action profiles and spatial
picking are unchanged. One capability variant serves all registered projects;
games do not add their own engine/editor enum or branch.

## Admission and use

`GameplayContracts::new` validates 1–32 combined new/legacy entries, rejecting
duplicate or reserved names and invalid build identities before schema factories
run. Scene and input schemas are frozen and checked against the declared identity.
Spatial2d requires a reflected vec2 component field. Embedded presentation does
not require a fake position component.

A private `ValidatedGameplay` is distinct from legacy `RegisteredGame`.
`handle()` on an application-owned static registry produces a Copy
`RegisteredGameplay` bound to that registry and entry. Host, stream, view, codec
and consumption callbacks do not run during admission. Schema factories and the
18 legacy held-value conversions do run. These are trusted compiled native
callbacks, not sandboxed plugins. There is no downloaded code or hot reload.

The shared CPU router uses the frozen identity, types, expected input,
neutral and existing typed host/stream factories. Embedded views receive only
`Ui`, borrowed read-only `FrameView` and readonly mode/input-ready context, returning at most one bounded
`IntentRequest`. `create_view` is an explicit later operation, not validation.

`intent_catalog()` borrows frozen IDs/schemas and `command_max_sequence()` exposes
the admitted bound, without factories or allocation. Held returns empty/None.
This is library metadata; no network semantic discovery endpoint is provided.

Neutral is schema/range/size checked and frozen. Typed startup must compare it
against `G::Input::default()` before publishing a connector; reflection defaults
do not prove that equivalence.

Commands declare closed typed intents and reflected input. Every call checks
shape, range and 64KiB bounds. Opaque game receipts are limited to 4KiB and cannot
select ERP methods or grants. The codec receives router-owned player, sequence
and scene/play scope; the engine assumes no game-specific fields.

Intent/prepared tokens belong to the exact immutable contract instance, even
when another admission has identical metadata. Observation rejects mismatched
scope and pre-preparation ACK ticks, then waits for a newer frame before invoking
the game observer with the original player context.

## Shared execution and authority

The Editor keeps its existing single Backend, ERP, typed stream, Input and
Play/Pause/Step/Stop lifecycle. It shares a private command state machine with
the standalone CPU `GameplayRouter`; it does not launch a second session.
Human presenters and AI library callers use `Editor::submit_gameplay_intent`.
Existing managed-input permissions, leases, sequences and ACKs remain authoritative.
Opaque evidence pairs a validated snapshot and scope. Full fenced and codec
origins are distinct; decoder source changes invalidate pending work. This does
not prove screenshot coherence or arbitrary remote-host pairing.

The router separates submission, admission and game-observed consumption, with
one pending command. A neutral ACK also needs a newer same-scope frame before
another command can replace it. Lifecycle/focus loss, stale scope and timeout
invalidate pending work and release control; an admitted action may already have
run, so cancellation is not rollback. Unknown outcomes are not automatically
resubmitted. Status retention is bounded; this is not universal exactly-once.

Optional closed `sim.input_value.command` metadata correlates ID, intent and scope.
ACKs echo validated metadata; activity summaries record admission only, excluding
payloads/receipts/grants. Router consumption links that ID but is not a server-wide
activity assertion.

## Open and play

`PreparedRegisteredProject::open(path, registry)` reads a closed manifest:
`schema: "orr.registered-project/1"`, canonical `game` ID, canonical `build_id`,
portable relative `scene` and `input: "commands"`. It rejects unknown/duplicate
fields, unavailable IDs/builds/capabilities and linked or escaping scene paths.
Manifest and scene reads are bounded to 64KiB/256KiB. Startup uses owned admitted
scene bytes; normal Save/Restart retains the existing document lifecycle.
This is trusted/quiescent file admission, not an atomic filesystem snapshot.
The manifest selects compiled code; schema authority remains the compiled
contract and exact host/stream handshake, not an independent project schema pin.

The starter executable registers both examples once:

```sh
cargo build --locked --offline --release -j1 -p orr_static_starter --features gameplay,static-game-actions --bin editor
editor --project examples/static-game-starter/scenes/delivery.project.json
editor --project examples/static-game-starter/scenes/switchboard.project.json
```

Use Play, Take control and the project-owned presenter; Pause/Step/Stop are the
existing Editor controls. Embedded presenters get readonly frame/context and
return intents. Edit/Viewer snapshots are display-only; Play consumption requires
opaque evidence. Focus/lifecycle/source changes reset presentation. The new
generic startup supports Commands only; unsupported Held combinations are
rejected while legacy registered held/action/spatial games remain supported.

## Verification and completion

CPU tests cover legacy spatial/held compatibility, embedded admission,
identity/schema/name failures, bounded typed intents and encoded values, token
ownership and receipt scope/tick fencing. Project-side Delivery/Switchboard
examples reuse their existing game rules and typed host/stream factories.

AI observation reuses ERP queries, history/Undo, bounded activity subscriptions
and frame/screenshot identities. Atomic screenshot/state observations, network
semantic-command discovery/results and detailed log privacy remain separate work.

Acceptance requires two project manifests using the same executable, actual
Editor input/consumption, Pause/Step/Stop, restart and document/history/file
preservation. CPU and offscreen tests are distinct from native window evidence;
report actual Windows UI observations separately. These examples do not establish
Stormglass authoring, online cooperation or overall engine completion.
