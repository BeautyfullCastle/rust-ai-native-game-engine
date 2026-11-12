# Rust animated sequence package authoring (optional Linux CLI)

Build `cargo build --release -p orr_package --bin orr_pkg --features animation-authoring`.
Neither Python nor separate sequencer/package-source executables are needed:

```sh
orr_pkg create-animated-sequence --recipe sequence.json --asset-id demo.sequence.json \
  --license LICENSE.txt --name demo-animation --version 1.0.0 --engine '*' \
  --output new-package
```

The existing bounded Rust sequence authoring API is called in-process. The
existing recipe format, rig/clip/embedded-resource validation and deterministic
cooked bytes are unchanged. The package contains exactly `model.animated.json`,
`LICENSE.txt`, and `orr.package.json`; declared capabilities are exactly `models`
and `animation`. These declarations do not activate a runtime capability or
install anything. User-provided license/provenance is preserved, not a rights
certification. `create-assembled` retains its static-only profile and bytes.

Inputs: recipe at most 16 KiB, cooked model at most 64 MiB, UTF-8 nonempty license
at most 1 MiB, bounded metadata/path arguments. Existing decoder budgets remain.
Input/output ancestors must be trusted, stable and symlink-free. No shell or
external executable flags are accepted (`--sequencer`, `--package-source`).
The feature is default-off; unsupported-platform/feature commands fail before
opening inputs/outputs. This is Linux-tested authoring, not Windows support.

The shared publisher validates all three files against the requested closed
profile, syncs a private same-parent stage, and uses Linux no-replace rename.
Existing files, directories and dangling links at the destination are refused.
Before publication failure, only still-owned staged files are cleaned. After
rename, a parent-sync failure reports **published, durability uncertain**: do not
retry or remove the package. This deliberately improves on the legacy Python
publisher's visible partial directory, while preserving successful file bytes.
It does not claim hostile concurrent-ancestor safety or a wall-clock timeout.

Legacy Python and native example flags remain available for compatibility. The
new CLI directly shares the sequencer and package-source libraries rather than
spawning them. Tests compare original retained native/Python output against Rust,
then install and reopen an AnimatedBinding with the original sources hidden;
independent CPU pose/vertex oracles do not certify GPU rendering/live crossfade.
