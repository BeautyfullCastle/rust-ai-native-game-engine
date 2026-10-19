# Checked local package update preview

The optional `orr_editor/package-manager` feature can replace one installed,
unassigned **direct** package with an explicitly selected local version. It uses
the existing `Project::preview_install` and `Project::install_checked` APIs.
There is no registry lookup, automatic dependency upgrade, new package format,
network access, or change to the engine's default feature set.

## Editor workflow

1. Open a saved Room project and choose **Verify installed packages**
2. Set the existing local package directory field to a package with the same
   installed name and a different exact version
3. Choose **Preview update** and review the old/new versions and full package
   digests
4. Choose **Update previewed package** to apply that specific preview
5. Verify installed packages again before opening the updated asset preview or
   assigning a model

Preview does not alter the active lock, package cache, scene, model bindings,
camera, simulation history, or source files. The editor discards a pending
preview when the selected source or project changes. An asynchronous result for
an older source/project generation cannot authorize a later update. A failed preview
requires a new explicit preview before Apply.

After a successful update, existing model preview/Use/Save controls remain the
way to assign an installed model to a Room entity. Package update itself does
not create or migrate model assignments. The updated installed model can be
assigned to a KEY, saved, reopened without authoring/package sources, collected
during Play, and restored by Restart using the existing Room flow.

## Admission and preserved state

The replacement must have the same name as an installed direct package and a
different exact version. An indirect/transitive-only package is refused.
Both the old and new lock must contain exactly the same package names and direct
selection names. Only the selected package's version and immutable package
identity may differ. Its dependency map must remain identical; all other direct
selections and complete locked package identities must remain identical.

The selected package must be unused by saved static or animated model bindings.
Unsupported or malformed references are refused conservatively. This slice does
not rewrite references, migrate assets, change dependencies, or remove packages.
Existing source validation still applies: bounded local absolute paths, regular
non-symlink/non-reparse files, portable declared paths, a declared `LICENSE.txt`,
and supported package/runtime metadata. A declared license file is not a legal
review.

`plan_update` returns an immutable `UpdatePlan`. Its getters expose the selected
root, source, name and preview for inspection; callers cannot alter the plan's
checked snapshot. `apply_update` verifies the saved project and expected lock
again, then uses the package core's writer guard and checked publication path.
The writer callback rechecks the admitted root identity, source identity and
bytes (including whitespace-only manifest edits), saved scene, camera and static/animated bindings before lock publication.
Stale previews, source changes, path replacement, a competing writer, or saved
project changes fail closed instead of updating a different state.

The package store can retain unreachable staged objects after a failed apply.
The checked operation promises no unintended active-lock publication, not cache
garbage collection or rollback of staging. If publication succeeded but later
verification fails, the error reports that the published lock is retained; the
editor does not silently roll back an uncertain publication. Filesystem checks
are designed for a trusted local project, not as a sandbox against a hostile
process continuously swapping filesystem ancestry.

## Acceptance contract

`.github/workflows/package-update.yml` is additive. Its required aggregate job
needs every CPU/Linux, CPU/Windows and software-Vulkan UI/Linux row to succeed.
Existing mandatory workflows remain required separately. The evidence location
uses `runner.temp` in the execution step's environment and the matching artifact
upload path, never a job-level context expression.

Every row starts from a clean current checkout, captures the complete source
path/byte/mode inventory, and builds retained producers in a fresh empty target.
Cargo JSON proves the package, manifest, source target, enabled features, release
profile and newly compiled executable. Logs and retained binaries are hashed;
source and producer fences run before/after commands and at completion. Historical
Cargo or runtime hashes do not freeze future unrelated changes. Local source
review against the intended base remains necessary alongside executable checks.

New backend and panel inventories are named explicitly and counted separately
from existing removal/panel regressions. Ignored tests are not passing tests:
the production EditorApp acceptance must execute its exact single ignored case.
Missing GPU support, changed inventories, skipped cases, incomplete captures,
wrong producer/source identity, or an unchanged/nonfresh producer fail the gate.
Strict Clippy, default-off compilation, minimal `package-manager` compilation,
and animated support without package-manager are checked independently.

The UI acceptance creates distinct v1/v2 raw GLB inputs and imports them through
the production path. It retains evidence for immutable preview, checked Apply,
unchanged other packages, actual KEY Use and Save, hidden-source fresh reopen,
keyboard collection and Restart. Default production renderer options are used.
Saved/reopened/restarted pixels and installed hashes are compared, and a
presentation-free replay checks exact authoritative Frame/checksum parity.
Retained receipt claims alone do not replace file, image and lock comparisons.

The Linux UI gate accepts either one installed Lavapipe manifest or one explicit
absolute `VK_DRIVER_FILES`/`VK_ICD_FILENAMES` selection. If both are set, they must
name the same manifest. Path lists, relative/link paths, malformed or oversized
JSON, duplicate keys, and non-Lavapipe libraries are refused. The selected
manifest is copied and hashed; its bytes are fenced through completion. No
system Vulkan configuration or loader search path is rewritten.

Run the independent Python rejection fixtures without Cargo or a GPU:

```sh
PYTHONDONTWRITEBYTECODE=1 python -B tools/test_package_update_gate.py
```

On a clean committed checkout, run one acceptance row with a new evidence path
outside the repository:

```sh
PYTHONDONTWRITEBYTECODE=1 \
ORR_PACKAGE_UPDATE_EVIDENCE=/absolute/fresh/package-update-cpu \
python -B tools/package_update_gate.py cpu

PYTHONDONTWRITEBYTECODE=1 \
ORR_PACKAGE_UPDATE_EVIDENCE=/absolute/fresh/package-update-ui \
python -B tools/package_update_gate.py ui
```

A CPU-only local pass does not certify the UI row, Windows behavior, or hosted
CI. A blocked/failed check is recorded as such, never replaced with historical
results. This document defines required evidence; it does not claim a particular
local or hosted run has passed.
