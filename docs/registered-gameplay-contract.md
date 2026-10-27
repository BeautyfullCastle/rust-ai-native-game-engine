# Registered gameplay capabilities (contract stage)

The default-off `orr_editor/static-gameplay` feature adds application-owned
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

The future shared consumer can use the frozen identity, types, expected input,
neutral and existing typed host/stream factories. Embedded views receive only
`Ui`, borrowed read-only `FrameView` and readonly mode/input-ready context, returning at most one bounded
`IntentRequest`. `create_view` is an explicit later operation, not validation.

`intent_catalog()` borrows each admitted intent ID and exact payload schema;
`command_max_sequence()` exposes the admitted command bound. Held capabilities
return an empty catalog and no command bound. These getters neither regenerate
schemas nor call application code. This is metadata discovery only: no ERP
discovery endpoint or AI tool connection is provided by this stage.

The supplied neutral is schema/range/size checked and frozen exactly. Reflection
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

## Authority and remaining work

The future command router still must authenticate and validate the actual managed
input ACK, lease/player ownership, current scene/play lifecycle, monotonic
correlation sequence, retries, one-pending-command bound, neutral and release.
The helpers validate sequence bounds, not monotonicity. Calling `observe` with
caller-supplied ACK metadata is not proof that an RPC ACK happened. A callback's
Consumed result alone is not cross-session authority.
It must supply frames from this descriptor's verified typed stream and reject
view intents in Edit/Paused/Viewer or while input is not ready, even if a trusted
view callback ignores that context. Arbitrary FrameView/game pairing is not
validated here and could panic in game-specific reads.

The next vertical slice is shared registered routing/viewport use, followed by
manifest resolution of already-compiled registered game IDs. Unknown IDs must
fail closed. No manifest should download or execute arbitrary code.

## Verification and completion

CPU contract tests cover legacy spatial/held compatibility, embedded admission,
identity/schema/name failures, bounded typed intents and encoded values, token
ownership and receipt scope/tick fencing without starting factories. Existing
Delivery/Switchboard remain separate regression consumers.

Contract test success does not establish native UI play or project opening.
Final acceptance remains two distinct games authored only in project files,
opened and played/stopped through the shared editor without changing engine or
editor sources. This stage does not yet provide that workflow.
