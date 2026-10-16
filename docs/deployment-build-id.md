# Deployment build identity

The development defaults and explicit `0 = untracked` behavior remain unchanged.
For a release, use one deployment manifest for the native host, relay, native
sample clients and browser. Deployment validation rejects zero. It does not
change the ORRF format, the default sample IDs or the golden checksums.

## Release ownership and identity

The release owner assigns a nonzero `game_code_id` whenever the game code or
game identity changes. Keep IDs distinct across games and incompatible releases.
`game_code_identity` names that declared release, for example a game-code Git
revision or a release label. Reusing an old ID with changed code can make the
network accept incompatible peers: a label alone cannot prevent that.

The manifest records the game, declared code identity and ID, frame format, and
derived `build_id`. The tool implements the existing
`orr_sim::frame_build_id(game_code_id)` rule for ORRF version 2. The derived ID
is the exact value passed to `--build-id` and the browser's `build_id` option.
Neither endpoint should apply `frame_build_id` a second time: the relay already
applies `build_hash_of(build_id, 0)` when checking the handshake.

The v1 tool is pinned to frame format 2. A deliberate format change requires
updating the deployment tool/schema and its Rust reference checks. A manifest
for another format fails validation. A code change requires a new declared
game-code ID even when the engine's format is unchanged. `create --previous`
checks that a changed game/code label does not reuse the previous ID; the
release owner must retain earlier manifests and manage the full ID history.

`provenance.source_revision` and `provenance.binaries` describe the source revision and
artifact hashes separately. They do not affect the compatibility ID and do not
prove that an artifact was built from that revision. The 64-bit ID and network
comparison provide compatibility detection, not authentication or cryptographic
attestation.

## Create and validate

Run from the repository root with Python 3.10 or later. Creation requires a new
output file and never silently replaces an existing manifest.

```sh
python tools/deployment_manifest.py create --game arena --game-code-identity arena-release-2026-10-03 --game-code-id 9007199254740993 --source-revision YOUR_RELEASE_SOURCE_SHA --output target/arena-release.json
python tools/deployment_manifest.py validate target/arena-release.json --expect-game arena --expect-game-code-identity arena-release-2026-10-03
python tools/deployment_manifest.py export target/arena-release.json
python tools/deployment_manifest.py id target/arena-release.json
```

The numeric example is a test identity; assign an actual release ID under your
release policy. Add `--binary LABEL=PATH` for each built artifact whose hash
should be recorded. The tool accepts full u64 decimal/hex code-ID input and
writes canonical decimal strings. Keep both IDs as strings in JSON: converting
them through a JavaScript Number can lose bits above `2^53 - 1`.

For a replacement release, use the previous manifest as an additional check:

```sh
python tools/deployment_manifest.py create --game arena --game-code-identity arena-release-next --game-code-id 9007199254740995 --previous target/arena-release.json --output target/arena-next.json
```

The schema is [deployment-manifest.schema.json](../deploy/deployment-manifest.schema.json).
It describes the JSON structure; the Python validator is authoritative for
integer representation, the derived ID relationship and release comparisons.
Always run `validate` before consuming a manifest.
The example [arena.example.json](../deploy/arena.example.json) is a fixture, not
evidence of a deployed binary. Duplicate JSON keys, unsupported fields, numeric
JSON IDs, inconsistent derived IDs, unsupported formats and untracked zero IDs
are rejected.

## Apply the exported plan

`export` returns argv arrays for `native_host`, `relay` and `native_client`,
and a `browser` object with worker options and page query parameters. Pass arrays
directly to a process API, without constructing a shell command.

Build the programs you plan to use:

```sh
cargo build --locked --release -p orr_server --bin orr_server -p orr_remote --bin orr_remote_host -p orr_sample --bin arena --bin physics
```

On Windows, append `.exe` to the program names. For example, PowerShell can pass
the generated relay arguments without converting the ID into a number:

```powershell
$plan = python tools/deployment_manifest.py export target/arena-release.json | ConvertFrom-Json
$relayArgs = @($plan.relay.args)
& ./target/release/orr_server.exe @relayArgs --bind 127.0.0.1:7778 --ws-bind 127.0.0.1:7779
```

Add the existing connection, certificate, room and scene options for your
environment. Native sample `arena` and `physics` clients accept the exported
`--build-id` with their usual `--connect` options. The physics ERP host also
passes its explicit `--build-id` to the client when used with `--join`; its view
schema retains that selected identity. Arena ERP authoring remains local.

For the existing browser page, append the exported `browser.query` fields
`game` and `build` to its URL alongside the connection options. The page passes
the exact `build` text through the worker to the WASM constructor. An embedding
application can instead use `browser.opts` as the worker start options and add
its connection settings. Preserve `build_id` as a string.

The exported relay plan covers relay rooms. Authoritative room setup, custom
games, hot-patch generations and deployment orchestration require their own
integration; this tool does not publish a service or change those contracts.
The C ABI/TUI configuration has no new build-ID field in this change.

## Validation and evidence

```sh
python -m unittest tools.test_deployment_manifest
npm test --prefix tools/webtransport
cargo test --locked -p orr_sample --release --test deployment_identity -- --nocapture
cargo test --locked -p orr_sample --release --test build_identity
cargo test --locked -p orr_sim --release
cargo test --locked -p orr_web --lib
```

The deployment integration checks the tool's derived values against the actual
Rust frame-format identity rule, then exercises the native client path against
a real loopback relay. Matching manifest IDs must play; a different declared
game-code ID must reach the existing `BuildHashMismatch` rejection.

The browser proof extends the existing mandatory Chromium/WASM runtime tests.
It supplies a manifest to the actual page/worker/client path, checks successful
native/browser participation, and separately checks the mismatch rejection:

```sh
ORR_REQUIRE_BROWSER=1 cargo test --locked -p orr_server --release --test browser_e2e -- --nocapture --test-threads=1
```

Build the existing WASM packages and install the pinned Playwright tools first;
see [webtransport-trial.md](webtransport-trial.md). The existing required CI
browser job provides that environment. A Node helper test or a locally skipped
browser test does not establish browser runtime success. Preserve the exact
commit, commands, prerequisite/skip status and CI logs when reporting results.
