# Rust assembled-model package authoring

Use the optional Linux `orr_pkg create-assembled` command to turn a cooked static
assembly recipe into an installable content package **without Python or separate
assembler/package-source executables**. It calls the same strict Rust assembler and
package-source libraries in process; no decoder, shell, runtime or installer is
embedded in a subprocess wrapper.

Build once:

```sh
cargo build --release -p orr_package --features model-authoring --bin orr_pkg
```

Author a package using the existing assembly recipe format:

```sh
orr_pkg create-assembled \
  --recipe assets/chair.assemble.json --asset-id furniture/chair.assemble.json \
  --license LICENSE.txt --name furniture --version 1.0.0 --engine '^0.0.1' \
  --output new-furniture-package
```

The seven authoring arguments retain their meanings. The new command explicitly
rejects `--assembler`, `--package-source`, commands and unknown/duplicate flags;
it does not execute or silently ignore legacy executable paths. Successful stdout
is the absolute output directory. Exit 2 means usage error, 1 means creation error,
and 0 means publication succeeded (read any durability warning).

## Shared APIs and compatibility

`orr_model::static_assemble::{assemble_model, assemble_recipe}` retains bounded
recipe/source admission and canonical model serialization. Existing assembler
examples/tests retain their façade and import feature. Package metadata uses
`orr_package::source_authoring::create` unchanged; the old `orr_pkg_source` and
example interfaces remain. The resulting files are exactly `model.model.json`,
`LICENSE.txt`, and `orr.package.json`, declaring only the `models` capability.
Installation and actual consuming-host validation remain separate.

Recipes are at most 16 KiB, cooked model files 64 MiB, licenses 1 MiB and metadata
arguments 4096 UTF-8 bytes. Licenses must contain non-whitespace UTF-8 text and are
copied byte-for-byte: this is not rights verification. Existing model/dependency,
embedded image, transform, aggregate and path checks still apply. Input/output
ancestry must be trusted and stable; traversal, `.orr` and symlink paths are
rejected. The in-process path has bounded inputs, not the legacy child process's
60-second wall-time timeout.

## Publication guarantees

A held no-follow output-parent descriptor anchors an exclusively owned private
sibling staging directory. All three regular files and source metadata are verified
and synchronized before a descriptor-relative no-replace directory rename. Existing
files, directories and dangling links are never overwritten; unsupported no-replace
rename fails rather than falling back to overwrite. Parent/stage identity and bytes
are rechecked before publication. Failures clean only still-owned staging contents.

This intentionally improves the Python wrapper's per-file writes into a visible
new directory: the final directory now appears complete in one rename. If parent
synchronization fails after rename, the command reports **published, durability
uncertain** and does not retry, roll back or remove the package. This is not a
hostile-concurrent-user sandbox or a claim of guaranteed storage across every crash.

The Python frontend remains available for compatibility and differential tests;
its removal awaits consumer migration. New user-facing workflows should use Rust.
Animated sequence packaging is the next same-priority slice, not implemented by
this static command. Tests distinguish exact package-byte/inspection equivalence,
real install/source-hidden Binding loading, rejection/cleanup and publication
failure injection from GPU rendering or browser execution.
