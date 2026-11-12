# Saved shaded Mesh props: Linux headless delivery

`mesh-prop-delivery` is a separate, default-off Linux x86-64 delivery profile.
It consumes saved MeshNavigation authoring and runs deterministic **paused,
headless** simulation with optional offscreen PNG capture. It is not an
interactive native game window, a new shader, or full glTF/PBR conformance.
The existing scalar metallic/roughness implementation is unchanged.

## Create and run

Save a base MeshNavigation scene and its Mesh props in the Editor. At least one
prop is required; diffuse and scalar-shaded props may coexist. Save camera and
lighting settings if desired. Existing limits remain: 128 props, six distinct
appearances, 3,584 prop draws, and a 16 MiB props sidecar.

```sh
cargo build --locked --release -p orr_editor --features mesh-prop-delivery \
  --bin orr_mesh_prop_export --bin orr_mesh_prop_player
sha256sum target/release/orr_mesh_prop_player

target/release/orr_mesh_prop_export \
  --scene /absolute/project/example.scene.yaml \
  --runtime /absolute/build/orr_mesh_prop_player \
  --runtime-sha256 LOWERCASE_SHA256 --trusted-runtime \
  --output /absolute/delivery/new-bundle

/absolute/delivery/new-bundle/orr_mesh_prop_player \
  --bundle /absolute/delivery/new-bundle --ticks 60 \
  --size 640x480 --capture /absolute/captures/new.png --restart-check
```

Use the player built in the same feature configuration as the exporter.
`--trusted-runtime` explicitly authorizes execution of the selected local binary
for capability and restart/trajectory checks. A SHA-256 digest, ELF header or
successful smoke test does **not** authenticate code or sandbox its execution.
Use only a runtime you trust.

The output directory must be new and outside the authoring directory. Keep the
scene's exact basename: scene and sidecar bindings are not rewritten. The bundle
can be relocated and launched without the original authoring files or selected
runtime. A software GPU adapter is sufficient for capture; Linux build success
alone does not establish GPU availability.

## Contract and receipts

The independent `mesh.prop.bundle.json` manifest identifies
`MeshNavigationShadedPropsHeadlessV1`, Linux x86-64, scene checksum, graph revision,
file modes, byte lengths and SHA-256 digests. Inventory is closed: exact scene,
required props, optional camera/lighting, bundled executable and manifest only.
The running player must be the executable at that bundle's declared path.

Mission/hazard/nonbase authored profiles and agent-art, animation and HUD
sidecars are rejected rather than silently omitted. Camera follow/projection and
saved lighting/postprocessing are consumed by existing read-only rendering APIs.
Legacy Windows mesh schema-1 export and its shaded-prop refusal are unchanged.

Ticks are exact integers from 0 to 6,000; no wall-clock simulation loop runs.
Capture dimensions are 64–2,048 per axis and at most 1,048,576 pixels. PNG output
must be new and outside the bundle. Without `--capture`, no GPU is required.
JSON stdout reports manifest identity, verified tick, authoritative checksum,
fixed-point agent position, status and graph revision. Capture receipts add
applied camera, lighting/settings identities, adapter and PNG/RGBA hashes.
`--restart-check` compares stopped/restarted and fresh-host readbacks and, when
capturing, fresh-host pixels. This is checksum and complete agent-readback
comparison, not a claim of serialized full-Frame byte comparison or pixel
portability across adapters.

Reads are bounded and reject symlink ancestry. Publication is staged and
non-clobbering with source/identity rechecks. These checks require trusted,
quiescent filesystem ancestry and are not a filesystem sandbox. Prepublication
failure retains a private stage for inspection. An error after publication says
that publication occurred and durability/identity is uncertain; inspect before
retrying.

## Acceptance

Focused tests cover malformed manifests, hashes/inventory, source changes,
unsupported attachments, exact basenames, output safety and paused replay.
Explicit actual-binary tests additionally export, relocate and hide originals,
exercise shaded/diffuse props with camera/light/postprocessing, capture multiple
ticks and verify restart. Ignored GPU tests are not passes unless explicitly run.
