# Mesh scene point light

Build the editor with `mesh-navigation` (or `mesh-hazards-export` for a Hazard game).
Open the scene and expand **Mesh game lighting**. Enable **point light**, edit its
world-space position, linear RGB, intensity and range, then **Apply game lighting**.
Staging does not affect the applied light. Lighting Undo/Redo, Save and explicit
reload use the existing lighting sidecar workflow; simulation scene/history and
checksums do not change.

The scene supports zero or one fixed, unshadowed diffuse point light. Positions
must be finite and within ±1e9 world units, RGB and intensity within 0..10000, and
range within 0.000001..1e9. This is the existing stylized renderer, not a glTF
punctual-light or photometric implementation. Directional lighting, post effects,
static props and animated models retain their existing rendering paths.

Saved scenes reopen with the applied light. Both native Mesh players use the same
admitted settings as the editor, including after restart. Switching to a scene
without an admitted light clears the previous light. Invalid, stale or pending
documents cannot supply point-light settings and retain the existing Play/save
guards. Lighting presets preserve the target scene's point light; presets do not
capture or transfer it.

An enabled saved light uses lighting schema 3. Files without a point light keep
their existing schema 1/2 bytes. Explicit null, unknown fields, wrong schema or
scene/graph bindings and invalid numbers reject. Export additionally checks that
the chosen runtime advertises point-light support; old runtimes remain usable for
scenes without a point light.

Acceptance covers strict codec and bounds, actual UI staging/Apply/Undo/Redo/Save,
reopening, scene-switch clearing, runtime capability checks, copied sidecars and
native headless settings/restart parity. CPU receipts verify the saved settings,
not pixels. The connected desktop is locked; actual GPU appearance, window input
and playtest video remain unverified.
