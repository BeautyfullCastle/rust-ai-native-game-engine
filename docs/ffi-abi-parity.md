# Keeping the C header and Rust ABI in step

`crates/orr_ffi/include/orrery.h` remains hand-written. Update it together with
the public ABI in `crates/orr_ffi/src/lib.rs`, including the buffer, ownership,
threading, error, and compatibility documentation. An incompatible public
change still needs an ABI version decision; passing parity does not authorize
an API change.

The `header_parity` integration test reads the Rust declarations with `syn`
and generates a C translation unit against the current header. It checks
public `repr(C)` structures, field types/order/layout, `ORR_` constants, and
exported C function signatures. The C compiler checks the actual header under
the running target's data model, including `usize`/`size_t`, pointer widths,
alignment, and field offsets. No platform-independent hardcoded packing is
assumed. The opaque `OrrHost` handle has no public layout to compare.
The current inventory is three structures, 30 constants, and 16 functions.

The check compiles its generated assertions and type checks without linking
or running a simulation. Controlled header mutations must be rejected for
field order, field type, pointer constness, constant, and function signature
changes. Unsupported syntax within the modeled ABI declarations is reported as
a check failure rather than silently left unchecked. The generated probe is a
test artifact, not a replacement public header.

The supported declaration model is deliberately small. ABI declarations stay
at the top level of `src/lib.rs`; module source is inspected to reject exports
or public ABI types/constants outside that root. Types use known scalars,
canonical standard C aliases, and raw pointers; constants use integer literals. Arbitrary type
aliases, conditional ABI declarations, generics, extra `repr` modifiers,
arrays, and generated declarations need a reviewed extension to the checker.
The header inventory accepts flat named structure fields and rejects unsupported
aggregate syntax. It does not expand Rust macros or resolve arbitrary Rust/C
types. New structures also need an explicit binding to their compiled Rust
size/alignment before they can pass.
This is a source-declaration check, not an inventory of macro-expanded compiler
output or linked symbols. Source review must keep the public C ABI within these
supported declarations when introducing new macros or declaration constructs.

## Run the checks

Linux (GCC or Clang) or a Windows developer shell with MSVC installed:

```sh
ORR_REQUIRE_C_COMPILER=1 cargo test -p orr_ffi --release --test header_parity -- --nocapture
ORR_REQUIRE_C_COMPILER=1 cargo test -p orr_ffi --release --test c_client --test client_session
cargo clippy -p orr_ffi --all-targets --release -- -D warnings
```

PowerShell:

```powershell
$env:ORR_REQUIRE_C_COMPILER = '1'
cargo test -p orr_ffi --release --test header_parity -- --nocapture
cargo test -p orr_ffi --release --test c_client --test client_session
cargo clippy -p orr_ffi --all-targets --release -- -D warnings
```

The existing native workspace CI discovers the integration test automatically;
its workflow and required C-client/shared/static/import-library checks remain
unchanged. Linux CI requires a C compiler. On other runners a missing compiler
must print an explicit skip unless `ORR_REQUIRE_C_COMPILER=1` is set. A skipped
C check is not ABI validation. Use `--nocapture` to retain compiler/skip evidence.
MSVC needs C11 support for `_Generic` and `_Static_assert`; an installed compiler
that cannot compile these assertions fails the test instead of becoming a skip.

The parser and compiler adapter use the repository's locked `syn` and `cc`
versions; use `cargo test --locked` when checking reproducibility. The emitted
assertions come from the Rust source and the running target's sizes/alignment.
Their target-dependent numbers are intentional. Changes to the supported
declaration model need new drift fixtures and review before the check can cover
a new ABI construct. This does not claim validation on targets absent from the
native CI matrix, including 32-bit targets.

## What still needs behavior tests and review

A C type does not express whether an output buffer is consumed, whether a
null/zero-capacity probe is allowed, how long a borrowed pointer remains valid,
or whether close may race another call. Keep those contracts in both languages'
documentation and in the existing `c_client`/`client_session` runtime tests.
The parity test checks declarations and target layout, not equivalence of prose
or every ownership/lifecycle execution path. Existing error/buffer/reset/rejoin
assertions must not be weakened to make parity pass.
