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

## Explicit authored-UI template

`collect-dodge-ui-2d-v1` is a separate opt-in profile; the existing no-UI
`collect-dodge-2d-v1` and `arena-2d-v1` output contracts do not change.
Build the generator with `project-create,collect-ui`, then use:

```sh
orr_new_arena --output /absolute/new-game --template collect-dodge-ui-2d-v1 \
  --seed my-authored-namespace --game-id 12345678-1234-4234-8234-123456789abc
```

Supply your own canonical UUIDv4 for a distinct game's high-score namespace.
Reusing a UUID deliberately shares progress; a seed only namespaces authored
entity GUIDs. Opening, copying and exporting retain the existing identity.
The new profile namespaces entities by its own template ID and includes
`level.ui.json`, the strict default `collect-authored-v1` document. Its actual
Korean font is installed through `Project.install`, alongside the sprite package,
from build-bundled allowlisted bytes. There are no downloads, generated locks,
user font scanning, scripts, or compilation during creation.

The font remains separately OFL-licensed, with unchanged OFL, copyright, corpus
and source-manifest files in the installed/exported package closure. Only the
exact bundled 1,891,888-byte font (pinned SHA-256
`91c7e75ac1b54a3571a305d259a2f486b88289853baef90753506855f5c5dd08`)
receives a per-file limit exception in this profile. Other files keep the 1 MiB
limit, the total stays 2 MiB, and the closed profile permits at most 17 files;
old profiles keep their 16-file limit. Ordinary transaction cleanup and atomic
no-replace publication remain unchanged.

Build the editor with `collect-ui` and sprites support; open with
`orr_editor --collect-project /absolute/new-game`. Edit the UI text/layout through
the existing properties controls, Undo, Save, then reopen. Runtime/export require
`collect-ui,collect-sprites,collect-progress` (exporter also `project-export`).
A host without UI capability rejects the declared UI rather than discarding it.
Title Play, score/phase/best labels and Menu/Continue/Restart consume the saved
bounded document. Menus block controls while simulation keeps ticking.

This connects the existing optional UI authoring route to project generation;
it is not a general template/plugin system, native-device qualification or full
#94 game acceptance. Simulation remains UI/GPU/font-free. Required acceptance
includes generator CLI and old-profile negatives, exact package/identity/byte
closure, actual EditorApp edits/save/reopen, source-hidden read-only exported
execution, software GPU pixel evidence, and isolated high-score save/relaunch.
The three explicit `new_collect_ui_workflow` ignored test gates must be executed
with `--ignored --exact`; merely compiling or listing them is not a pass.
