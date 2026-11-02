# Installable assembled static models

`tools/create_assembled_model_package.py` bridges the existing cooked static-model
assembler and package-source authoring tools. It is an optional offline authoring
CLI, not a new decoder or automatic engine capability switch. The editor's GLB
importer remains unchanged. This command creates a package, **not an installation**.

Build trusted local executables:

```sh
cargo build --locked --release -p orr_model --features import --example static_model_assemble
cargo build --locked --release -p orr_package --bin orr_pkg_source --bin orr_pkg
```

Use the existing `orr_static_model_assemble` version-1 recipe with 1–32 cooked
`.model.json` parts and placement matrices. Part paths remain recipe-relative.

```sh
python3 tools/create_assembled_model_package.py \
  --recipe props/chair.assemble.json --asset-id props/chair.assemble.json \
  --license LICENSE.txt --name assembled-chair --version 1.0.0 --engine '^0.0.1' \
  --assembler target/release/examples/static_model_assemble \
  --package-source target/release/orr_pkg_source --output authored-chair
```

The output parent must already exist. A successful new directory contains only
`model.model.json`, `LICENSE.txt`, and `orr.package.json`. The package declares
`models`, not animation. Use the normal package manager or explicitly install:

```sh
target/release/orr_pkg install project --path authored-chair
target/release/orr_pkg verify project
```

Package verification establishes declared bytes and digests, not model validity
by itself. Model validity comes from the existing assembler's bounded cooked
admission, and the consumer subsequently validates through `model_bindings`.

## Preserved contracts

The native assembler alone admits recipe and cooked input, preserves geometry,
UVs, winding, embedded images and material/sampler values, offsets material
references, prefixes primitive identities and applies each placement. Historical
source digests remain provenance, not runtime external-file dependencies. This is
**static-only**: animation/skinning are not carried by this workflow. The existing
package-source tool alone authors and validates manifest metadata and files.

The supplied license is copied exactly as nonempty UTF-8 text; neither this CLI
nor its tests establish ownership or distribution rights. Its byte limit is
1 MiB. Recipe bytes are bounded to 16 KiB and produced model bytes to 64 MiB;
all stricter native part/resource/decoded-image limits remain in effect. Each
explicit native executable has a 60-second timeout and is invoked without a
shell or PATH lookup. These trusted local programs are **not sandboxed**.

Inputs/output ancestors and executable paths must be stable, link-free, trusted
directories. Traversal and internal `.orr` store paths are rejected. All native
work runs into private scratch before final output creation. Existing output
paths, even empty directories or dangling symlinks, are refused. Publication
uses exclusive directory/files and removes only its known newly written files
on a handled error, retaining existing data. It is not a crash-atomic directory
transaction or protection against a hostile concurrent directory owner; cleanup
failure is reported explicitly.

## Focused verification

The Python tests exercise admission and publication boundaries. The dedicated
`orr_model_bindings` integration test runs the real wrapper/native tools, installs
into a disposable project, hides source inputs/package, loads the cooked asset,
and verifies persistent Binding identity and embedded content. It is explicitly
ignored in generic runs because native tools must first be built; execute it with:

```sh
ORR_ASSEMBLE_BIN="$PWD/target/release/examples/static_model_assemble" \
ORR_PACKAGE_SOURCE_BIN="$PWD/target/release/orr_pkg_source" \
cargo test --locked --release -p orr_model_bindings --test assembled_package_cli -- --ignored
```

No renderer, GPU, editor-UI or visual equivalence certification is implied by
successful installation and CPU model/binding loading.
