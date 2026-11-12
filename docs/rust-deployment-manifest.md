# Rust deployment manifest records

Build `cargo build --release -p orr_package --bin orr_pkg --features deployment-authoring`.
This optional Linux CLI replaces the user-facing Python manifest workflow:

```sh
orr_pkg deployment create --game arena --game-code-identity release-1 \
  --game-code-id 0x2a --source-revision owner-record --binary player=./arena \
  --output deployment.json
orr_pkg deployment validate deployment.json --expect-game arena
orr_pkg deployment export deployment.json
orr_pkg deployment id deployment.json
```

The existing `orr.deployment/1` schema is preserved. Create accepts nonzero decimal
or hex u64 and stores canonical decimal strings; load requires canonical decimal
strings. The frame-bound ID comes directly from `orr_sim::frame_build_id`, not a
second mix implementation. The v1 tool requires integer frame version2 and fails
closed if the engine's `orr_ecs::FRAME_FORMAT_VERSION` changes. It never binds an
already derived ID a second time. Values above JavaScript's2^53 boundary remain
strings in browser plans; physics maps to browser `phys` and native `physics`.

All seven root fields and exact provenance fields are mandatory, including
explicit nullable source_revision. Unknown, duplicate and missing fields are
rejected. `--previous` rejects reuse of the same raw game-code ID across a changed
game or game-code identity, not changes in provenance alone. Binary array order
is retained. Sorted keys, two-space formatting, ASCII Unicode escapes/surrogate
pairs and final newline match legacy Python output.

Source revision and binary hashes are **release-owner records**, not signatures,
authenticity, source-to-binary proof, compiler attestation or deployment success.
Export only prints launch plans. It does not execute programs, Cargo, Git, a
shell, networking, servers or a browser. Bootstrap source-provenance is a separate
contract and is not inferred from this manifest.

New safety bounds deliberately narrow the legacy unbounded filesystem interface:
manifest<=1MiB, at most32 binary records, each binary<=512MiB streaming in1MiB
chunks, text<=4096 bytes, argv<=64KiB/80 arguments, paths<=4096 bytes/128 components.
Paths reject traversal/NUL and all symlink components, including provenance inputs;
regular files are checked by held descriptor and identity before/after reading.
Trusted stable ancestry is still required, not hostile-concurrent-user isolation.

New output uses exclusive creation and never replaces files/directories/dangling
links. Failure after creation may leave a partial file, with no automatic deletion.
After bytes/identity verification, sync failure reports an existing complete file
with uncertain durability; do not blindly retry, overwrite or remove it. This
is not atomic file publication. Validation/id/export never write the source.

Original Python tools remain for compatibility and differential testing. Actual
native-client option consumption and in-memory relay handshakes are tested without
sockets; browser plan parity is not browser execution. No Windows/GPU certification
is claimed. Final HTML packaging now has a separate optional Linux
[Rust workflow](rust-coop-workflow.md); authored execution gates still require Node.

The native CLI also rejects repeated singleton flags and unpaired Unicode surrogate
escapes, instead of Python last-value argument selection or non-scalar strings.
These are explicit stricter admission deltas, not universal negative-input parity.
