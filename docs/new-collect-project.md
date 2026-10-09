# New CollectDodge project (bounded Linux template)

The optional `project-create,collect-dodge` combination extends `orr_new_arena`
with a closed `collect-dodge-2d-v1` template. The existing Arena template/API is
unchanged. This is not a general template engine or game-code compiler.

```sh
cargo build --release -p orr_sample --bin orr_new_arena --features project-create,collect-dodge
orr_new_arena --output /absolute/new-game --template collect-dodge-2d-v1 \
  --seed authored-entities-v1 --game-id YOUR-NEW-CANONICAL-UUIDV4
```

Supply a fresh, lowercase, hyphenated UUIDv4 for a **new game**. The generator
validates the UUID using `ProjectProgress`; it cannot determine whether an
otherwise valid UUID has already been used elsewhere. Reusing a UUID shares
progress identity. The seed namespaces entity GUIDs only, never save identity.
No identity is minted on open, restart, export, relocation or ordinary copying.
Copying an existing game intentionally retains its identity; an explicit fork
must provide a different validated UUID. No global UUID registry is implied.

Creation writes schema 3 `orr.project.json`, `level.scene.yaml`,
`level.sprites.json`, a README and the genuine installed `sample-sprites` package
closure. The existing `Project.install` computes the lock; the generator does not
copy a prepared lock or scan arbitrary source files. All content comes from the
closed build-time allowlist and includes its MIT notice. The four authored actors
are a player, two collectibles and one hazard. Move right to collect both items,
avoid the hazard, and use Space to restart. Initial level data stays editable
through the existing strict CollectDodge admission. No arbitrary scripts, hooks,
network downloads, UI layout authoring or new game simulation is introduced.

The parent directory must exist and contain no symlink components. A bounded,
private staging directory is verified before Linux `RENAME_NOREPLACE` publication;
existing destinations, including empty directories and symlinks, are never
replaced. Failed creation removes only its owned stage. Hostile filesystem races
and power-loss durability are outside this initial generator contract.

Open with `orr_editor --collect-project /absolute/project` built with
`collect-dodge,sprites`; launch with `collect_dodge --project /absolute/project`
built with `collect-progress,collect-sprites`. The schema explicitly declares the
high-score profile, so hosts without supported progress metadata must reject it
rather than silently discard it. Export through `orr_export_collect` with a trusted
matching runtime, retaining the same project UUID and assets. High scores belong
in user data and are only submitted by normal standalone completed runs. Editor,
headless, capture and export-smoke paths do not submit scores. General player
settings/gameplay UI and audio remain separate work.

Validation is staged: generator/unit/CLI negatives and unchanged Arena checks;
actual generated-project EditorApp edit/save/play/win/restart; source-hidden,
read-only relocated export and progress relaunch; independent review; then the
exact development candidate's mandatory CI. Local tests do not replace mandatory
syscall isolation CI, and a local ptrace denial remains a failure, not a pass.

## Focused verification

```sh
cargo test --release -p orr_sample --features project-create,collect-dodge --lib project_create
cargo test --release -p orr_sample --features project-create,collect-dodge --test new_arena_cli
cargo test --release -p orr_sample --features project-create --test new_arena_cli
cargo test --release -p orr_sample --features project-create,project-export,collect-progress,collect-sprites --lib
```

The first three invocations currently execute 17, 5 and 5 tests respectively.
The joint sample library executes 105 tests, with two explicit opt-in tests not
counted as passed. The generated progress workflow explicitly executes the
isolated App child; it does not establish the separate syscall-isolation gate.
The editor acceptance target `new_collect_workflow` requires each of its three
named ignored tests to be selected individually with `--ignored --exact` and
must report `1 passed; 0 failed; 0 ignored`. Its source header lists the required
production binary paths, feature combinations and capture environment. A
missing binary, forbidden namespace operation or unavailable GPU is a failure.
The GPU oracle checks installed opaque texels for every visible actor and exact
source/export image equality, beyond checksum-only validation.

On 2026-10-07, these local configurations and all three generated workflows
passed; editor framebuffer identified llvmpipe LLVM 19.1.7. Strict all-targets
Clippy passed for the joint editor/sample feature configuration and Arena-only
project creation. Initial workflow-test compilation and lint failures were
repaired without suppressions; an Arena-only build disk-guard interruption was
retained and its retry passed after verified released-cache cleanup. Required
remote feature CI and development integration are separate gates.
