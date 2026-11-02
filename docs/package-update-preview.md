# Checked local package update preview

The optional `orr_editor/package-manager` feature replaces one installed,
unassigned **direct** package with an explicitly selected local exact version.
There is no registry lookup, automatic dependency upgrade, network access, new
package format or default-feature change.

## Editor workflow

1. Open a saved Room project and **Verify installed packages**.
2. Enter a local package directory with the same installed name and a different
   exact version.
3. For unchanged dependencies, choose **Preview update**, inspect versions and
   digests, then **Update previewed package**.
4. For additional dependencies, enter 1–8 **Dependency candidate directories
   (one per line)**. Choose **Preview package update with dependencies**, inspect
   the full lock and unused candidates, then **Update package with dependencies**.
5. Verify again before previewing or assigning an installed model. Existing
   Use/Save controls remain the separate assignment path.

Preview writes no lock, cache, scene, binding, camera, history or source files.
Source/project changes invalidate previews; failed previews require a new one.
Candidate Add and Update have separate opaque plans: starting either preview
invalidates the other's ready plan. All mutation lanes share a pending-operation
fence. Field edits cannot cancel a dispatched commit or imply rollback; older
results retain their captured-project attribution. Worker lifetime remains the
existing detached design, not a new joined guarantee.

## Admission and preserved state

The target must be a direct package with a different exact version, unused by
saved static **and animated** bindings. Unsupported references fail closed; no
reference migration occurs. All other direct selections and complete installed
package identities remain identical.

The single-source lane requires identical dependency maps and package name sets.
The candidate lane permits **additions only**: every old target dependency entry
and exact version remains unchanged in the new map. New reachable candidates
become indirect; unused candidates stay inactive. Candidates cannot reuse active
package names. Existing resolver cycle/conflict/capability checks apply. No
package removal or dependency upgrade is included.

Both lanes reuse the package core's preview and checked-install transaction;
the candidate lane uses its existing dependency variants. Plans pin source
identity and contents, previous/approved locks and saved Room admission.
Candidate updates additionally pin every supplied source, including unused
ones, as directory identity, raw manifest and full inspected package. Exact Room
manifest/root and the existing scene/binding/camera/Frame snapshot are checked
at planning, Apply and again in the writer callback before publication. Stale
files, competing writers or changed saved content refuse publication.

Inputs retain existing bounded-path, regular non-link/non-reparse, portable
asset-path, declared `LICENSE.txt` and supported-runtime checks. License data is
not a rights assessment. Filesystem observations assume trusted, quiescent
ancestry, not hostile concurrent path swapping.

Failure protects the active lock, not the entire filesystem: unreachable
immutable objects may remain staged. Postpublication re-admission errors say the
package was installed and retain the published lock. No automatic rollback or
retry is promised.

## Focused acceptance

The candidate lane exercises a valid Room baseline plus unused A v1→A v2 adding
B→C, with D unused; immutable Preview; preserved old entries; used-target,
dependency-removal/version-change, malformed/conflicting and stale-source/Room
refusals. Actual accessibility-button Update is followed by hiding all source
folders and reading C through installed runtime/Binding/PreparedRoom. A binding
file written by the test is not an Editor Save action. Existing candidate Add
and single Update regressions remain separate. CPU consumer proof is not
physical mouse, native-window or GPU acceptance.

Current `AGENTS.md` Linux-local checks and exact-source independent review govern
integration. Empty hosted checks are not PASS. The old hosted workflow was
retired; the original single-source acceptance tooling below is retained, not a
new merge gate or claim that every row ran.

That tooling fences clean source bytes/modes and fresh producer identities,
hashes logs/binaries, checks exact inventories, and separately validates CPU,
UI, feature isolation and strict Clippy. Its actual UI case uses distinct v1/v2
GLBs, KEY Use/Save, hidden-source reopen, keyboard collection/Restart, pixel/hash
comparison and presentation-free Frame replay. Ignored, blocked, skipped or
unrun rows are not successes. Its software-Vulkan selector requires one pinned
absolute regular Lavapipe manifest; it does not rewrite system driver settings.
See `tools/package_update_gate.py` and its independent rejection tests for the
retained row contract and manifest restrictions.

```sh
PYTHONDONTWRITEBYTECODE=1 python -B tools/test_package_update_gate.py
ORR_PACKAGE_UPDATE_EVIDENCE=/absolute/fresh/evidence \
  python -B tools/package_update_gate.py cpu  # or ui, with its actual prerequisites
```

A local CPU pass never certifies Windows, GPU, browser or hosted CI. Record the
exact executed checks and preserve original failures.
