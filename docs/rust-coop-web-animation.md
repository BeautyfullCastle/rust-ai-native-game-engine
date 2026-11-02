# Rust web animation authoring and composition

Optional Linux tools, built with `cargo build --release -p orr_package --bin orr_pkg --features coop-web-authoring`:

```
orr_pkg create-coop-animation --assets-dir assets --output new.json --role unit:0 --image unit.png --frame idle 0 0 32 32 --clip idle 10 loop
orr_pkg compose-coop-animations --assets-dir assets --output combined.json --manifest unit.json --manifest enemy.json
```

The existing role/FPS schema is not converted to sprite duration_ms. Default FPS
and loop fields stay absent unless authored; the actual JS consumer supplies them.
This slice removes Python/Pillow from these two production authoring commands.
Trusted Node on PATH **remains required** for the actual JS sampler gate. Final
HTML packaging and scene/WASM validation are separate, still Python-backed tools.
The original Python commands remain for compatibility and differential tests.

Manifest admission rejects duplicate keys before image lookup, nonfinite values,
boolean numbers, unknown fields/roles/clips, invalid image paths and all existing
frame/FPS/pivot/anchor/logical-size/attack timing bounds. Manifest bytes are at most
64KiB, each PNG8MiB, at most11 images/32MiB total. PNG chunks/CRCs/APNG/critical
chunks/trailing bytes are checked before full existing png17 codec pixel decode.
Web dimensions remain4096; the separate sprite schema stays2048 unchanged. Pixels
are decoded sequentially with a64MiB output ceiling and discarded, not retained
across images. Original PNG bytes/digests are preserved; no reencoding occurs.

A structural/pixel Prepared snapshot is **not** a sampler receipt. Only a successful
Node run of the exact embedded verifier and sampler can create the private verified
state accepted by publication. Both JS files are compiler include inputs, so source
changes invalidate the Rust artifact; the real script bytes and consumer are checked
in focused tests. This proves the embedded sampler revision, not arbitrary future
build output. The later packer must still validate its own exact build sampler.
No user JS/executable override or shell command field exists. NODE_OPTIONS and
NODE_PATH are removed, stdout/stderr are concurrently bounded at64KiB each, the
15-second timeout kills and waits, and receipt fields/hash/length must exactly match
immutable manifest bytes. Missing Node, bad output or timeout has no fallback.
This uses a trusted local Node, not a sandbox against hostile executables.

Before publication, source images and composed input manifests are rechecked against
snapshots. Composition preserves input receipt order, rejects duplicate roles and
never rewrites image paths. Float JSON spelling may differ from Python (e.g. exponent
formatting); semantic numeric values/defaults are preserved, and every receipt hashes
its actual emitted bytes. Do not assume universal old/new text byte identity.

Explicit new output is mandatory, no historical overwrite mode. A held-parent
exclusive sibling is written/synced/identity-checked, then hard-linked without
replacement. Existing file/directory/link outputs remain untouched. Failure before
publication conservatively retains the owned sibling for inspection rather than
recursively deleting anything. A failure after publication reports existing output
with uncertain durability/cleanup; no blind retry or rollback. Inputs/ancestry must
be trusted and quiescent; no hostile-concurrent-filesystem guarantee is claimed.
Additional argv64KiB/8192arguments and path4096bytes/128components bounds apply.

Tests exercise actual unchanged JS sampler and unchanged Python packer/HTML checker
with a real WASM build. These are CPU/integrity/JS syntax checks, not browser/GPU,
physical sound playback, or new WASM scene checksum verification.

Observed decoder difference: Pillow13.0.0 accepts a valid-CRC4x2 PNG whose complete
zlib stream contains only one scanline, zero-filling the missing row. Native png17
rejects insufficient pixel data. This deliberate conservative refusal is recorded
as an old-accept/new-reject differential, not claimed identical malformed-PNG
acceptance. The original failed parity expectation is retained in evidence.
