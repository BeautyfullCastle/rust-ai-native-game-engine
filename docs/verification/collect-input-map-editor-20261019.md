# Collect input-map editor verification — 2026-10-19

The default-off `collect-input-map-editor` feature provides the dedicated `orr_collect_input_map` tool. It edits the existing six-action Collect keyboard format. Existing Collect runtime, Arena, project and export implementations are unchanged.

## Verified gates

- Canonical integration target: `cargo test -p orr_editor --features collect-input-map-editor --test collect_input_map_authoring_ui -- --test-threads=1`: **22 CPU cases passed** (18 document and four widget cases). The GPU case is executed separately; duplicate tests included by the binary are not counted again.
- **Two existing Arena input regression cases passed** against the unchanged runtime.
- Default library and minimal optional-feature checks, strict library/binary/integration Clippy, and the production tool build passed.
- **One explicit software-GPU UI case passed.** Three 720×650 captures show default, authored and conflicting bindings.

The production tool's widgets author all six actions, reject conflicts, preserve exact Undo/Redo, and save/reopen without overwriting. The saved file passes the public `CollectBindings` loader. A real Bridge trace reaches victory and then resets through the authored bindings. Accepted-map and pending-draft controls, error text and file actions were visually reviewed.

## Preserved failures and corrections

- The initial CPU run had 17 passes and five failures: four widget queries ambiguously matched summary labels and action buttons; a filesystem-socket fixture could not be established. Role-specific queries corrected the widget fixtures. Supported FIFO and device-file refusal tests replaced that newly authored fixture. Socket-specific execution remains unverified; the production file policy was not relaxed.
- A subsequent run passed all 18 document cases but failed four widgets because the production App emitted a title command every frame. Title updates now occur only on state transitions. The original bounded convergence assertion remains intact.
- A final wording correction aligned the pending-draft message with the actual Discard draft button. CPU, feature, strict, build and explicit GPU gates were rerun against the final source.

## Review and coverage limits

Independent implementation and workflow reviews completed. Final source remained unchanged throughout acceptance. Detailed local execution paths, binary identities and diagnostic metadata are intentionally excluded from this public summary; original failure evidence is retained separately.

The UI test uses synthetic widget events and software rendering. Its public adapter/Bridge trace does not replace the standalone player's private event-policy tests. No native OS-window interaction, physical keyboard input, hardware-GPU or Windows execution is claimed. The tested map is an external session file supplied through `--input-bindings`; it is not automatically embedded in a project or exported game.
