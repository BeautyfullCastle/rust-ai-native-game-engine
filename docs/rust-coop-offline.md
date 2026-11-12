# Rust offline web delivery

Build `orr_pkg` with optional `coop-web-authoring` on Linux, then:

```sh
orr_pkg pack-coop-offline --build-dir trusted/dist --output new-game.html \
  --scene authored.scene.yaml --animations animation.json --audio bank.json
```

All three authored inputs are optional. This delivery command requires neither Python nor Pillow. Trusted Node on PATH remains required when a Scene or animation is supplied. It does not build/fetch code, install packages, or accept executable/script overrides. The legacy Python tool remains a compatibility oracle; copy and recipe entrypoints share this hub through the [Rust workflow](rust-coop-workflow.md).

## Same immutable build, real validation

The twelve fixed assets are snapshotted without following symlinks. The closed, reviewed JS graph is bundled without a general module resolver; unexpected import/export keywords and script/style terminators are refused. The ordinary no-authoring path preserves opaque WASM and default atlas bytes, rather than claiming they have been executed or decoded.

Scenes execute against the exact snapshotted build WASM using the existing fixed verifier. The private receipt binds Scene/WASM hashes, length and actual initial checksum. Animations reuse strict JSON/rectangle/PNG admission and execute against the **supplied build's exact sampler**, not merely the engine source-tree sampler used by the authoring CLI. Fixed atlases reused by animation come from the original build snapshot. Other image paths are bounded; every PNG is fully decoded sequentially, without reencoding original pixels. The known stricter native refusal of missing scanlines remains intentional.

Audio banks retain all unknown manifest/record fields and cue order while requiring the fixed ten cue identities, file paths, SHA256, mono 48 kHz PCM16 header/frame count and inclusive peak8192. Existing native PCM validation is reused. All inputs are rechecked before publication.

## HTML and JSON compatibility

A pinned optional html5gum tokenizer checks decoded, case-sensitive ID values before attribute deduplication. Duplicate/reserved IDs, reserved authored prefixes and the served animation hook are refused. Only script/style switch to raw text, matching the legacy conservative ID fence rather than claiming a browser DOM. Original HTML is not reserialized; exactly one literal stylesheet and module anchor must also be actual tags outside comments/raw text.

Characters metadata preserves duplicate-last-wins semantics, large numeric text and unknown fields by embedding validated original JSON bytes with literal `<` escaped. Audio similarly preserves original JSON. Thus payload/JSON meaning and closed JS bytes are compared with Python, **not universal whole-HTML byte equality**. Native deliberately refuses NaN/Infinity, excessive JSON depth, malformed tokenizer events, NUL, empty/valueless IDs, self-closing/unterminated raw-text tags and excessive tag count. These are documented stronger boundaries than Python's tolerant parser; no weak fallback is used.

## Output and bounds

Every Rust invocation requires an explicit new output. It does not reproduce the old implicit default-file overwrite exception. Existing file/directory/link outputs are preserved. A held-directory, synced private sibling and no-replace hard link publish only after all applicable proofs. Failures may retain owned sibling files; postpublication sync/cleanup uncertainty is reported without rollback or blind retry.

Existing per-role limits remain: fixed assets64MiB each, Scene/animation64KiB, audio manifest128KiB, animation image8MiB/total32MiB and dimensions4096. Additional limits are128MiB aggregate snapshotted inputs,256MiB output,100000 HTML tags and bounded argv/paths. Base64/output arithmetic is checked before allocation. Decoded PNG working memory is separately capped64MiB per image. This assumes trusted stable directory ancestry and trusted supplied build code, not a hostile-code sandbox or a complete process memory limit.

Focused tests use the actual engine WASM, all authored combinations, legacy Python semantic differential, unchanged offline checker/Node syntax, altered-build-sampler rejection and malformed/overwrite cases. This is CPU/Node delivery evidence, not browser/GPU rendering, physical sound or a played-game result.
