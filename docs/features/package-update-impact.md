# Package update impact preview

In the Room editor's **Content packages** panel, select a local directory with a
new exact version of an installed direct package, then choose **Inspect update
impact**. This read-only report lists saved model-binding GUIDs, asset paths,
saved static/animated kinds, and whether each referenced asset is missing,
changed, or byte-identical in the candidate revision.

The report is a saved-state observation, not an atomic filesystem snapshot or an
update plan. Inspect a quiescent project; concurrent external edits can invalidate
the observation. It never changes package selections,
caches, sources, saved scene/model files, or editor history. Package identity
changes even when one asset's bytes do not. Candidate model decoding and resource
compatibility remain unchecked; byte equality does not establish safety. Zero
referenced bindings is not permission to update. Existing update refusal for
saved bindings remains unchanged.

Inspection and a queued-result recheck run off the UI thread. Changing project
or source, closing the panel, dirty/modal authoring state, and package operations
invalidate the observation. Repeated clicks while busy do not start more jobs.
External edits after the final recheck are not continuously monitored; inspect
again to refresh.

Acceptance coverage includes deterministic asset categories, stale source/lock/
binding rejection, UI generation and ABA changes, actual widget inspection,
no mutation authority, and unchanged live editor selection/checksum/history.
These tests must pass on the exact candidate before publication; source recovery
or static review alone does not establish an executable pass.
