# Static custom Inspectors (experimental)

Issue #562, opt-in CPU-tested consumer slice. Enable `orr_editor/custom-inspector`
and install a `Registry` with `EditorApp::with_custom_inspectors` before the first
frame. The feature is off by default; enabling it alone installs no panels.
Each app owns its registry. There is no game-specific editor dispatch or global
registry. This slice covers reflected entity components, not singletons or the
specialized 3D panels that bypass the generic component Inspector.

## Existing Arena example

From the repository root:

```sh
cargo run --locked -p orr_editor --features custom-inspector --example arena_custom_inspector -- scenes/arena_blank.scene.yaml
```

Add/select an Arena player. Its custom PlayerTag panel assigns slots 0 through 7.
The same executable registers a second panel for an existing Bullet's lifetime.
`examples/arena_custom_inspector/panels.rs` owns both registrations and draw
functions. Adding the second component required no editor/core/game-enum changes.
Arena has no health or inventory, so these examples do not complete #562's
health/inventory consumer requirement. No new game runtime was created.

Choose **Use generic fields** to edit that component using the existing widgets.
This presentation-only choice is kept in egui memory, never in the saved scene.
Custom callbacks and the registered component's edits are disabled outside Edit,
during proposal previews, under disabled parent UI, and when the editor cannot
mutate. Unregistered types retain the existing generic Inspector policy.

## Compatibility and edit authority

Use `SchemaKey::for_type` on the compiled game's reflected registry. It captures
the exact deterministic type JSON Schema and reflected type name. The app derives
the current key from its verified registry; no invented numeric fingerprint or
hash can make a changed schema match. Different text fails closed to generic
fallback, even for semantically equivalent schema text. Type or field renames,
ranges or schema documentation changes may require a matching registration.

An extension declares a nonempty stable ID, API version 1 and signed priority.
Higher priority wins independently of registration order. Duplicate IDs and equal
priorities for the same key reject without changing the registry. Unsupported
versions reject. This is experimental Rust source API, not a stable binary ABI;
the earlier unpublished numeric fingerprint field is replaced by exact schema.

Callbacks receive egui UI, a scoped ID and an owned read-only `Snapshot`. The
snapshot rejects malformed values and duplicate fields. Return at most one
`FieldEdit` per frame. Exact top-level bool, integer, fixed-point, enum and flag
fields are accepted after reflection kind/range checks. Nested/structural/entity
edits remain generic-Inspector work. UI state stays outside gameplay data.

Valid intents become ordinary `InspEvent::Set` and use the existing
`Editor::set_field`/host validation, Undo/Redo and dirty-state path. The app does
not retain intents across frames. Invalid intents only report errors. For users
of the lower-level registry API, binding the same target/schema/generation and
rechecking current host authorization remain caller obligations.

Callbacks are trusted native Rust code, not sandboxed plugins. This API supplies
no host, document, disk or network handles, but cannot prevent native code using
ambient authority, blocking, recursing or panicking. No dynamic loader, registry
service, listener or credential path is added.

## Validation and remaining acceptance

Run `custom_inspector_contract` and `custom_inspector_app` with the feature
enabled, plus `inspector_number_focus` with or without it. CPU egui tests cover
custom/generic edits, invalid
intent preservation, Undo/Redo, dirty/Save/reopen, Play suppression, selection,
preview, exact-schema fallback, duplicate fields and disabled parent callbacks.
The example executable is build-checked; tests use actual EditorApp and local
hosts with injected egui input, not physical input or native window/GPU proof.

Health/inventory consumption, native Windows visual/physical-input acceptance,
disconnect recovery, full required checks and development integration remain
pending. Generic numeric edits now commit once on Enter, Tab, pointer or
programmatic focus transfer. Escape, disabled UI and a hidden Inspector discard
drafts without document mutation. The original focus-transfer failure and initial
compile failure are retained alongside the regression results.
Publication is separately blocked by the Windows GitHub network policy;
local validation does not waive that restriction or complete #562.
