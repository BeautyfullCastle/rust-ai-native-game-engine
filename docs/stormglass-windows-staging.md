# Stormglass Windows reserved staging primitive

The opt-in `orr_package` feature `stormglass-windows-stage` exposes
`UnvalidatedStage::prepare(&reserved_inputs, trusted_sha256, existing_parent)`
on Windows. It reuses the reserved-input module's fixed local NTFS,
volume-GUID traversal and no-reparse ancestor reservations. Existing snapshot
behavior is unchanged; only internal directory-helper visibility is expanded.

The primitive creates a unique `.stormglass-unvalidated-*` directory with only
the admitted `coop_defense.exe`, `game.scene.yaml`, and repository-owned
`run-game.ps1`. Files use create-new writes, flush, bounded exact readback and
single-link checks. `path()` reports the retained location; `recheck()` checks
held sources, payload bytes and the current three-name inventory. It never
executes a runtime, emits a readiness receipt, modifies permissions, overwrites
a preexisting file or recursively removes a directory.

Use only trusted directories with cooperating writers. Stage creation-to-open
and writer-to-reader transitions are not hostile-directory transactions. Readback
checks contents, not provenance of a substituted byte-identical object.
Live stage, parent, ancestor and payload reservations deny ordinary replacement
and leaf writes until the value is dropped. The source reservation owner is
borrowed for that lifetime. Directory sharing does not prevent child creation:
inventory checks are point-in-time, not an atomic closure guarantee. No defense
against privileged/kernel changes or preexisting writable mappings is claimed.

Drop and every post-creation failure retain the stage without cleanup. Preparation
errors include its location so callers can inspect partial output. Ordinary
permissions are inherited unchanged. After handles are dropped, stage paths and
source paths reopened later have no continuing reservation guarantee.

## Deliberately pending

This is **prepare-only**, not a publisher or validated runnable export. There is
no rename/publication API, CLI wiring, receipt or execution readiness claim.
Native no-replace experiments did not establish a valid handle-preserving rename:
absolute volume-GUID targets failed with sharing violation, including a sibling
output parent; parent-handle/relative-name testing failed with invalid parameter.
A bare relative target resolved against the process directory rather than the
intended parent and was rejected by location assertions. Experiments and original
failures are retained outside git. No check-then-rename fallback was added.

Safe no-replace publication, competing-publisher verification and bounded real
copied-binary smoke remain separate work. Synthetic PE fixtures validate only
staging/content checks and are never executed.

## Acceptance

Run on Windows:

```
cargo test --offline --locked -p orr_package --features stormglass-windows-stage --lib stormglass_windows
cargo clippy --offline --locked -p orr_package --features stormglass-windows-stage --all-targets -- -D warnings
```

Tests cover exact preparation, distinct stages, bad-pin nonmutation, leaf/source
reservations, injected-child refusal, partial collision preservation with retained
path diagnostics, and release-without-cleanup behavior. Existing content and
snapshot suites remain enabled. Linux aggregate and native runtime smoke are
not established by these Windows-focused checks.
