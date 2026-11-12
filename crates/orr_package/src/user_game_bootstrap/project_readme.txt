# {{PROJECT_NAME}}: independent authoring workspace

This is the reviewed two-game starter, not a new runtime. You own Cargo.toml,
src/, scenes/, tests/, and the Cargo.lock you generate. The library name remains
orr_static_starter so copied Rust sources are unchanged. Engine dependencies use
absolute paths to the separately pinned local engine checkout in provenance.json.
Keep that checkout immutable and at the same location. This folder alone is NOT
an engine-source-free authoring SDK or a delivered player bundle.

From THIS folder, with a supported installed Rust toolchain and cached dependencies:

    cargo generate-lockfile --offline
    cargo test --offline --locked --no-default-features --features standalone,bundle --lib --test standalone_contract --test bundle
    cargo test --offline --locked --test acceptance
    cargo run --offline --locked --bin editor -- delivery scenes/delivery.scene.yaml
    cargo run --offline --locked --bin player -- delivery scenes/delivery.scene.yaml

The bootstrap never invokes Cargo. Lock generation can fail if dependencies are
not cached; there is no claim of offline build readiness until that separate gate
succeeds. Review and commit your generated Cargo.lock with your project sources.
The template's license is MIT OR Apache-2.0; review your project's licensing.

For native builds, first commit your user project (including lock and provenance)
to your own Git repository, then use that clean user commit's lowercase 40-hex
SHA as ORR_STATIC_STANDALONE_BUILD_SHA during compilation. Do NOT use the engine
SHA as a substitute for user-code provenance. Use the same user SHA for bundle
--source-sha, and explicitly review the runtime binary/hash before using the
existing bundle tool's --trusted-runtime flag. Nothing here trusts or executes a
runtime. After reviewing and committing the user project, the explicit Linux
native build steps (run from this folder, not the engine) are:

    test -z "$(git status --porcelain)" || exit 1
    USER_SOURCE_SHA="$(git rev-parse --verify HEAD)"
    export ORR_STATIC_STANDALONE_BUILD_SHA="$USER_SOURCE_SHA"
    cargo build --offline --locked --release --no-default-features --features standalone,bundle --bin delivery-player --bin switchboard-player --bin bundle
    ./target/release/delivery-player --scene "$PWD/scenes/delivery.scene.yaml"
    ./target/release/switchboard-player --scene "$PWD/scenes/switchboard.scene.yaml"

For a Delivery bundle, first review the built runtime before executing the next
commands, which explicitly trust it. The staging copy avoids Cargo hard links;
replace the output path with your own NEW absolute bundle destination:

    RUNTIME_STAGE="$(mktemp -d)"
    cp -- ./target/release/delivery-player "$RUNTIME_STAGE/delivery-player"
    RUNTIME_SHA256="$(sha256sum "$RUNTIME_STAGE/delivery-player" | cut -d ' ' -f 1)"
    ./target/release/bundle delivery --runtime "$RUNTIME_STAGE/delivery-player" --runtime-sha256 "$RUNTIME_SHA256" --scene "$PWD/scenes/delivery.scene.yaml" --output /absolute/new-delivery-bundle --source-sha "$USER_SOURCE_SHA" --trusted-runtime
    /absolute/new-delivery-bundle/run-game

These commands assume the default target directory, an installed supported
Linux x86-64 toolchain, and the documented display/graphics dependencies. If you
set CARGO_TARGET_DIR, adjust binary paths. Both native players start paused;
Enter starts/resumes, Escape or focus loss pauses. Actual window/source-hidden
bundle acceptance is still required. UPSTREAM_README.md is historical context:
its -p orr_static_starter selector is WRONG for this renamed package; omit -p
inside this workspace or use -p {{PROJECT_NAME}}, and change repo-root scene paths to scenes/.

The engine's native standalone_x11 test and its support module are deliberately
omitted: that source-fence harness assumes the engine repository layout. Native
window/trace tests are retained but require their documented environment and
separate renderer validation. Portable tests and successful bootstrap creation do
NOT prove native GUI behavior, source-hidden delivery, or bundle reproducibility.
Build and validate actual delivered bundles separately using the existing player
and bundle implementation. No engine runtime or registry source was changed.

provenance.json records the engine pin and initial generated-file SHA256 values;
it is a creation receipt, not ongoing attestation after you edit files. The old
engine README is preserved verbatim as UPSTREAM_README.md, not as runnable
instructions for this relocated workspace.
