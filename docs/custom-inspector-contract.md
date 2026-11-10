# Static custom Inspector contract (experimental)

Issue #562, first contract-only slice. Enable `orr_editor/custom-inspector` to
use `custom_inspector::{Registry, Extension, Snapshot, FieldEdit}`. The feature
is off by default. No EditorApp call site, sample game panel or runtime dispatch
is changed in this slice. Enabling it alone does not install a panel.

## Registration and compatibility

Create a registry and explicitly register statically linked function pointers.
Each extension declares a nonempty stable ID, the exact reflected type name,
host schema fingerprint, API version 1 and a signed display priority. Higher
priority wins independently of registration order. Duplicate IDs and equal
priorities for the same type/schema are rejected without replacing entries.
Unknown type/schema resolves to the existing generic Inspector. Unsupported API
versions are rejected during registration. With the feature off, existing
Inspector behavior is unchanged. This is experimental source API, not a stable
binary ABI. Type/field renames or layout changes require a new compatible schema
key; never fabricate a fingerprint to make an old extension match.

## Snapshot and edit boundary

Construct `Snapshot` from the current host's type/schema, reflected descriptor
and owned value. Construction validates the value. Extensions can read the
snapshot but cannot mutate it through this API. Keep presentation settings
outside gameplay data. `Draw` receives egui UI, a stable scoped ID and the
snapshot; it may return at most one discrete `FieldEdit` per frame.

The contract accepts exact top-level scalar fields only: bool, integer, fixed
point, enum or flags. Reflection checks value kind, range and integer width.
Nested paths, entity references and structural changes remain generic-Inspector
work. Valid edits become ordinary `InspEvent::Set`; invalid edits become errors.
No event is applied here. The eventual consumer must bind it to the same target
and host generation and send it through the existing authorization, validation,
Undo/Redo and dirty-state path. A checked event is not a capability and must not
be replayed against a newer snapshot without host validation.

Calling `Registry::show` with `editable = false` skips all custom callbacks and
renders only a disabled generic view, discarding its events. The host must still
enforce its current play-mode permission independently. Extensions are trusted
native Rust code, not sandboxed plugins: the API offers no host/document/disk or
network handle, but cannot prevent native code using ambient process authority,
recursing, blocking or panicking. Do not load untrusted code. There is no dynamic
loader, remote registry, new listener or credential path.

## Verification and remaining acceptance

Run `cargo test --locked -p orr_editor --features custom-inspector --test custom_inspector_contract`.
The nine focused tests cover exact identity/fallback, deterministic priority,
registration rejection, read-only snapshots, invalid intent rejection,
structural/duplicate-field rejection, disabled callbacks and invalid callback output. They use
CPU egui frames, not native windows. The Health fixture is synthetic contract
coverage, not an existing sample-game consumer.

Before #562 can be accepted: coordinate one EditorApp dispatch owner, consume an
existing game's component, add a second consumer without game-specific editor
branches, and demonstrate selection/edit/rejection/Undo/Redo/Save/reopen/Play on
Windows. Preserve cancellation/focus and play-mode checksums. Full required
checks and independent integration review remain separate from these focused
tests; this partial API must not be reported as completed product acceptance.
