# PNG sprite package verification

This acceptance slice imports an original generated PNG through the real
`png_sprite_package` example, checks immutable installation through `orr_pkg`,
and separately exercises Arena project admission with the actual compiled
capability inventory. Metadata-only package installation is not runtime admission.

The additive Linux and Windows workflow retains command logs and receipts on
failure. It enforces exact test counts with no ignored tests. Existing mandatory
workflows and golden checksums remain unchanged. Local Linux evidence does not
substitute for hosted CI or Windows execution.

## Real command-line checks

The Python harness generates a 2×2 RGBA PNG with four distinct pixels and explicit
fixture redistribution permission. It checks complete package determinism under
relocation, exact metadata and file inventory, byte-for-byte image and license
preservation, source/output collisions, malformed input, invalid arguments, and
static FIFO rejection on Unix. Windows records the FIFO case as inapplicable.

It invokes the actual `orr_pkg inspect`, `install`, `list`, `explain`, `verify`,
and `verify --package` commands. Source trees are relocated away after installation
and retained as evidence. Installed file hashes are independently recomputed.
Same-version conflicts must leave the project unchanged, and verification of a
tampered installed snapshot must fail without repairing the corrupted bytes.

The separate Rust admission suite uses the production Arena `PreparedProject`
route and real compiled inventory, asserting decoded pixels and binding data.
It does not manufacture capabilities or substitute direct asset reads for
consumer admission. These CPU checks do not establish GPU preview, physical
input, gameplay, or exported-application behavior.

## Reproduction

Use the commands in `.github/workflows/png-sprite-package.yml`. The harness accepts
`--evidence`, `--importer`, and `--package-cli`; its evidence directory must not
already exist. Supply paths to the built binaries, appending `.exe` on Windows
and adjusting paths if Cargo uses a custom target directory. A receipt is written
only after every assertion passes. Retain failed logs and do not report incomplete
checks as passed.

## Local Linux result

On 2026-10-19, all 16 example tests passed with zero failed, ignored, measured,
or filtered cases. The disabled example rejected both help and FIFO input
arguments before file access. The enabled example and package CLI built, strict
example Clippy passed with warnings denied, and formatting of the new example
tree passed. The real CLI harness completed every assertion and wrote its receipt.
The corresponding Windows inventory is 14 tests; the Unix symlink and Linux FIFO
tests are excluded by platform configuration, not ignored at runtime.

Two initial failures remain part of the evidence. Crate-wide formatting found
pre-existing differences in untouched sample files; the additive gate checks the
new example tree only and changes no existing required gate. The first CLI
harness assumed inspection preserved manifest file-list order. Inspection
canonicalizes that list; the harness now checks the precise canonical list while
still checking the original generated list separately, and its rerun passed.

These are local Linux results. Windows and hosted CI remain separate required
checks before a merge decision.
