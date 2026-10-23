# Create and open Collect projects in the editor

On Linux x86_64, build `orr_editor` with both optional features `project-create` and
`collect-ui`. The default editor and either feature alone do not expose this
workflow. It reuses the existing closed `collect-dodge-ui-2d-v1` starter and its
`create_collect` transaction; it adds no template format, package downloader,
compiler, scripting hook, or alternative project loader.

## Create a project

1. In a clean, local Physics or Collect editor session, choose **File → New
   Collect project…**
2. Set **Project directory** to an absolute, nonexistent directory whose parent
   already exists and is not a symlink
3. Enter a **Seed** of 1–128 ASCII letters, digits, dots, underscores or hyphens,
   and an explicit canonical **Game UUIDv4**
4. Choose **Create and open**. The existing creator installs the bounded sprite
   and Korean font packages, validates the complete project, and atomically
   publishes it without replacing an existing destination. The editor admits
   the result through `PreparedProject` before switching hosts

A seed determines authored entity namespaces. The UUID identifies the game's
high-score namespace: deliberately reusing one shares progress. Use a distinct
UUID for a distinct game. This dialog does not generate or silently substitute
an identity. Opening, saving and reopening retain the project's original UUID.

The starter contains its scene, sprite sidecar, authored Korean UI, installed
packages and lock, licenses, provenance and progress declaration. See
[new Collect project](new-collect-project.md) for the exact closed profile and
bundled font licensing/limits. Creation performs no network downloads and does
not scan user fonts or run the exporter/runtime as an installation step.

## Open, edit and play

Choose **File → Open Collect project…**, enter the existing project's directory,
and choose **Open project**. Open requires a complete Collect project with sprites, authored UI and progress
metadata. The loader verifies the installed closure and
presentation metadata before adopting the new scene. A successful switch resets
old selection, histories and session input state rather than carrying them into
the new game. The installed sprites and Korean font/UI become the new project's
presentation. Use the existing UI authoring controls, Save and Play controls.

Opening the dialog is not permission to discard work. Scene edits, pending or
unsaved presentation edits, Play, an active viewport input lease, remote or
unsupported hosts, and a stale source session block replacement. Cancel leaves
the current project and filesystem unchanged. Validation, creation and admission
errors preserve the old host/document and show an error in the dialog.

If project creation succeeds but the editor cannot adopt it, the published
project remains available. The dialog offers **Open created project**, which
retries admission of that exact existing project rather than attempting another
creation or overwriting it. Existing destinations, invalid UUIDs/seeds and unsafe
paths are errors; they are never treated as requests to replace a project.

This editor route does not submit high scores. Normal standalone completed runs
remain the progress-writing authority. Existing creator, relocated export,
source-hiding and progress-relaunch acceptance requirements are unchanged.

## Mandatory acceptance

Run the checker self-tests with Python bytecode disabled:

```sh
PYTHONDONTWRITEBYTECODE=1 python -B tools/test_collect_project_creator_gate.py
```

For full acceptance use a clean, committed checkout and a fresh, absolute evidence
directory outside it for each lane. The gate deliberately rejects dirty sources,
stale evidence, source additions/mutations during execution and cached producer
artifacts. Each lane creates a new private Cargo target and retains the newly
compiled executable, exact feature metadata, SHA-256 hashes, command logs and
source inventory. Do not point it at another worker's build or evidence directory.

```sh
PYTHONDONTWRITEBYTECODE=1 \
ORR_COLLECT_CREATOR_GATE_EVIDENCE=/tmp/collect-creator-cpu-new \
python -B tools/collect_project_creator_gate.py cpu

PYTHONDONTWRITEBYTECODE=1 \
ORR_COLLECT_CREATOR_GATE_EVIDENCE=/tmp/collect-creator-gpu-new \
python -B tools/collect_project_creator_gate.py gpu
```

Linux x86_64 GPU acceptance requires an installed llvmpipe Vulkan ICD and working
bubblewrap user namespaces. The gate selects exactly one explicit software ICD,
uses a private XDG runtime directory, and fails if the software adapter or source
isolation is unavailable. It never silently substitutes a hardware adapter or
unisolated run. Preserve failed logs and evidence before attempting a fresh run.

The `collect_project_creator_ui` executable has a fixed, reviewed inventory.
The CPU lane executes these actual EditorApp widget workflows individually:

- `creator_file_widgets_create_save_reopen_preserve_identity`
- `creator_cancel_invalid_inputs_and_existing_dest_preserve_host`
- `creator_dirty_or_stale_presentation_blocks_switch`

The CPU lane also freshly compiles and executes 31 private guard cases across
`app::collect_project::tests`, `app::collect_project::panel::tests`,
`editor::collect_project::tests` and
`app::collect_ui_preview::project_switch_tests`. These cover failed and wrong-host
admission, explicit recovery, old-host epochs/checksums, stale clean caches,
read-only authoritative history/state, pending gestures, saved-scene and UI
staleness, and unsafe/malformed saved files. Each suite has an explicit fixed
inventory; ordinary unrelated library tests do not substitute for these cases.

The complete integration inventory contains three ordinary cases and two ignored
cases. The GPU lane explicitly executes the ignored
`creator_file_widgets_llvmpipe_korean_sprites_gameplay` case, which must successfully execute `creator_source_hidden_reopen` as its
source-hidden child. The child is never run directly without its parent-provided
project and evidence context. Merely compiling,
listing, or ignoring it is not coverage. Exact named cases must execute and
report one passing test with zero failures or ignored tests. Complete and ignored
inventories must match the gate; missing, extra, duplicate and zero-test results
fail. Feature-off/default and each single-feature configuration are separate
compile and strict-Clippy boundaries, never counted as executed acceptance tests.

GPU evidence retains complete 1800 × 1200 editor windows and a structured
`collect-creator-acceptance.json` receipt. The real app must exercise File New,
admission, File Open in a fresh app, installed Korean UI and sprites, authoritative
gameplay victory, and acknowledged restart. A source-hidden child reopens the
created project using installed content after the repository is masked. Review
the retained full windows for readable Korean text, HUD, controls and the actual
game outcome. Receipt validation and PNG decoding do not replace visual review.
The gate independently compares the Korean-title negative control and exact
Undo-restored canvas, validates each phase’s actors/ticks/checksum, and requires
production-default renderer options. The six main sprite frames are `created`,
`same-app-reopened`, `playing`, `won`, `restarted` and `stopped`. The child retains
`fresh-reopened` and its own four gameplay phases. Additional full-window Korean
title/HUD captures and both adapters remain part of the required artifact set.
Software Vulkan with injected egui events does not qualify a native OS window or
physical keyboard.

The dedicated `collect-project-creator.yml` workflow runs independent Linux CPU
and GPU lanes and an always-run aggregate that accepts only success. Its evidence
environment is step-scoped; the upload uses the identical direct path expression,
so failed logs and partial captures survive. Checker regression fixtures prevent
moving `runner.temp` into unsupported job-level environment context.

This new menu acceptance does not replace `new_collect_workflow`, the three
`new_collect_ui_workflow` creator/export/progress workflows, or mandatory syscall
isolation and existing Arena/Room/default-editor CI. They remain independently
required on the exact candidate. Local checker fixtures alone do not certify the
Rust implementation, GPU route, hosted CI, review or merge readiness. Preserve
original failures; a retry cannot rewrite their outcome. Main and PR1 merges are
outside this feature's scope.
