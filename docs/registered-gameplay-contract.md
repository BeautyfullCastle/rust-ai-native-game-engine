# Registered gameplay capabilities and CPU routing

The default-off `orr_editor/static-gameplay` feature adds
presentation and input contracts alongside `static-games`. Existing
`StaticGameDescriptor`, `StaticGameRegistry`, held/action profiles and spatial
picking are unchanged. No built-in game enum or per-game editor branch is added.

## Admission and use

`GameplayContracts::new` validates 1–32 combined new/legacy entries, rejecting
duplicate or reserved names and invalid build identities before schema factories
run. Scene and input schemas are frozen and checked against the declared identity.
Spatial2d requires a reflected vec2 component field. Embedded presentation does
not require a fake position component.

A private `ValidatedGameplay` is an admission result, **not** `RegisteredGame` and
not an editor launch handle. The current editor does not consume it yet. Host,
stream, view, command codec and consumption callbacks do not run during admission.
Schema factories and the 18 legacy held-value conversions do run. These are
trusted compiled native callbacks, not sandboxed
plugins. This adds no runtime discovery, downloaded code or hot reload.

The shared CPU router uses the frozen identity, types, expected input,
neutral and existing typed host/stream factories. Embedded views receive only
`Ui`, borrowed read-only `FrameView` and readonly mode/input-ready context, returning at most one bounded
`IntentRequest`. `create_view` is an explicit later operation, not validation.

`intent_catalog()` borrows each admitted intent ID and exact payload schema;
`command_max_sequence()` exposes the admitted command bound. Held capabilities
return an empty catalog and no command bound. These getters neither regenerate
schemas nor call application code. This is metadata discovery only: no ERP
discovery endpoint or AI tool connection is provided by this stage.

Neutral is schema/range/size checked and frozen. Reflection
defaults are not proof of the game's semantic default. The existing typed host
startup must check this same frozen value against `G::Input::default()` before
publishing a connector. Metadata admission does not establish that equivalence.

Commands declare a closed typed intent set and a reflected input schema. Every
intent and encoded input is checked for shape, range and a 64KiB bound on every
call. Receipts are opaque game-owned JSON bounded to 4KiB; they cannot select ERP
methods, grants or transport sequence. The codec receives a router-owned player,
correlation sequence and scene/play scope. It may encode game-specific receipt
evidence without assuming any game's field names.

`ValidatedIntent` and `PreparedGameplayCommand` are bound to the exact immutable
contract instance, not its name or callback pointers. A clone retains that
identity; another admission with identical metadata cannot exchange tokens.
Observation rejects a different scope or ACK tick earlier than preparation and
waits for a frame strictly newer than the admitted ACK tick before calling the
game's consumption observer, including the unchanged router-owned player context.

## Shared routing and authority

`editor::input::GameplayRouter::connect_local` pairs connections from one LocalHost.
It owns its ERP client and typed stream. Human and
AI library callers use the same bounded intent submission path. It reuses the
existing managed-input lease/sequence/ACK machinery rather than introducing a
second input transport. Startup checks the control and stream identity/input
contracts. Readonly opaque frame evidence pairs a validated snapshot and scope;
legacy streams without evidence fail closed. Unpaired remote hosts are unsupported.
Full fenced frames and frame-codec
records have explicitly distinct scope origins and cannot be exchanged.

The router separates submission, admission and game-observed consumption, with
one pending command. A neutral ACK also needs a newer same-scope frame before
another command can replace it. Lifecycle/focus loss, stale scope and timeout
invalidate pending work and release control; an admitted action may already have
run, so cancellation is not rollback. Unknown outcomes are not automatically
resubmitted. Status retention is bounded; this is not universal exactly-once.

Optional closed `sim.input_value.command` metadata correlates command ID, intent
and scene/play scope. Existing ERP permissions, lease ownership and typed input
validation remain authoritative. The ACK echoes validated metadata and existing
activity summaries record admission only. Payloads, receipts and grants are not
added to those summaries. A router consumption result links the same command ID;
it is not a server-wide activity assertion of consumption.

The next vertical slice is shared registered viewport use, followed by
manifest resolution of already-compiled registered game IDs. Unknown IDs must
fail closed. No manifest should download or execute arbitrary code.

## Verification and completion

CPU tests cover legacy spatial/held compatibility, embedded admission,
identity/schema/name failures, bounded typed intents and encoded values, token
ownership and receipt scope/tick fencing. Project-side Delivery/Switchboard
examples reuse their existing game rules and typed host/stream factories.

AI observation reuses ERP queries, history/Undo, bounded activity queries and
subscriptions, and existing frame/screenshot identities. Screenshot and structured
state are not yet one atomic observation. Unified network semantic-command
discovery/results, detailed log filters/privacy and viewport/manifest wiring remain
separate acceptance work; this stage adds no coordinate-only automation layer.

Contract test success does not establish native UI play or project opening.
Final acceptance remains two distinct games authored only in project files,
opened and played/stopped through the shared editor without changing engine or
editor sources. This stage does not yet provide that workflow.
