# Collect HUD subtree duplication

Optional editor-only `collect-ui` convenience, based on development
`b0a494037a9166d013cae7085dfd700b8bff3b79`. This is a bounded #106/#94 increment,
not completion of those issues or the original 22-item roadmap.

## Using it

Open a Collect authored-UI project, expand **Collect UI document**, select a node,
and click **Duplicate node and descendants**. The selected node and all its
transitive descendants are appended in their original draw order, even when
unrelated nodes were interleaved. The copied root becomes selected; use the
existing anchor/size/offset controls to move it. Copies initially overlap their
sources, so later draw-order occlusion still follows the existing UI rules.

The copied root keeps its external parent. Internal parents point to copied
containers. Screen, text, binding/action, geometry and source nodes are unchanged.
Every new ID is the first unused `ui_copy_N` in document order, avoiding both
existing IDs and earlier copies. These IDs stay valid even when a source ID is
already 32 characters long. Copies own their text and can be edited independently.

One successful click is one UI Undo entry. There is no clipboard, autosave, new
schema, simulation state or runtime behavior. **Save UI document** remains an
explicit, single-file operation with the existing file identity/external-edit
checks. Play is read-only; no selection disables duplication.

## Bounds and atomicity

The source and whole candidate are validated before applying. The existing
`apply_document` performs complete font/corpus and serialization validation
before it changes the document/history. Selection changes only after success.
A failed duplicate preserves document, selection, Undo and on-disk bytes.

The existing 32-node, depth-4, 64-KiB, geometry, text and glyph rules are unchanged.
A valid duplicate preserves depth rather than adding another hierarchy level.
The oversized text defensive fixture is rejected by the 128-character text
validator; it does **not** prove the serialization byte-cap branch. Valid bounded
nodes cannot approach 64 KiB with the current schema. The dedicated gate also
runs the existing parser's actual 64-KiB boundary test, including an exactly
64-KiB accepted document and a 64-KiB-plus-one rejected input.

## Verification contract

`tools/check-ui-subtree-duplicate.sh contracts` inventories all eight dedicated
integration tests, runs seven CPU widget tests (one explicit GPU case remains
ignored here), reruns the unchanged six panel unit tests, the byte boundary,
default check and strict default/Collect all-target Clippy. No Cargo or shared
CI inventory change is needed. The standalone workflow requires both lanes.

`tools/check-ui-subtree-duplicate.sh gpu` builds and fingerprints the exact
runtime/exporter/generator producers, then explicitly executes the ignored GPU
case. Missing binaries, software GPU, namespaces or retained evidence path fail.
The case generates a real Collect UI project, uses Duplicate and position/Save
widgets, reopens exact content, verifies new pixels only in the copied HUD bounds,
and compares source versus relocated read-only export pixels at four gameplay
states. Namespace masks hide the workspace, original tools and source project
from exported execution. Masks are checked inside the subprocess. Original and
copied UI runs preserve four complete Frame byte/checksum probes; scene bytes and
source/bundle trees remain unchanged. Captures and producer hashes are retained.

This proves headless software-GPU behavior only. Native windows, physical GPUs,
Windows/macOS, IME, accessibility interaction beyond the widget harness, and a
Room duplication workflow are not covered. PR150 generalizes the panel and
source-retirement protections: preserve those during subsequent union review,
and rerun the Room boundaries there. This branch changes only the existing
Collect panel plus new isolated acceptance/doc/workflow/script files.

Local implementation, local execution, independent review, remote publication,
exact-head hosted CI and development integration are distinct gates. None of
those execution/completion outcomes follow merely from this document existing.
