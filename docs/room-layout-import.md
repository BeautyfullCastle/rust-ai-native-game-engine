# Import a Room puzzle into the editor

With `room-authoring`, open a saved local Room project and its matching saved
model bindings. In **Room level authoring → Import Room layout recipe**, paste
[the creator's schema-1 JSON](room-layout-creation.md). Choose **Preview layout
recipe**, inspect the summary, then **Apply imported layout**. Cancel retains
the text but discards the preview. Preview only parses the recipe: it does not
certify scene admission or a winning route.

Apply replaces every wall and moves PLAYER, KEY and EXIT in one existing host
transaction. Actor GUIDs and Y coordinates, floor and other scene data stay
intact. Walls receive new persistent GUIDs and canonical height. A wall with a
saved model binding blocks replacement; remove and save that binding first.
Dirty or mismatched model bindings block import. Camera, controls, model files,
packages and manifest are never written by import.

Full Room admission runs before commit. Rejected edits roll back the transaction
without a history entry. Undo/Redo applies to the complete import. Save uses the
normal scene save path; reopening loads the saved layout. Import does not
implicitly save, execute scripts or alter physical input mappings.

The pasted recipe is bounded to 64 KiB, schema 1, three actor position pairs and
at most 64 walls. Unknown/duplicate fields, non-string decimal coordinates and
invalid fixed-point numbers are refused. Scene bounds, collision constraints and
geometry are checked by admission on Apply. See the creator documentation for
exact X/Z coordinate and wall extent semantics.

Editing the text, changing scene path/load epoch/checksum, leaving Edit mode,
closing the panel or entering a blocked authoring state invalidates the preview.
Apply consumes it, including on failure; Preview again before another attempt.
The backend also checks local editable context, authoritative checksum and the
exact source document inside the transaction, including same-checksum renames. Uncertain host outcomes follow existing reconnect/inspect rules.

Verify the saved puzzle with `orr_playtest_room` or play it in the editor. A
recipe or admitted scene is not a solver result. CPU/headless widget and gameplay
checks do not establish native window/GPU or cross-platform acceptance.
