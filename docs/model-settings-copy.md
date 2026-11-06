# Reuse the selected model's assigned settings

With the editor's optional `models` feature, select an entity with an assigned
model in a matching local Yard3D or Room scene. In **Static model binding**, choose
**Use selected model settings**. Animated bindings also require their existing
animated-model feature/profile support.

The action fills the Package, Asset, Model kind, local translation/rotation/scale
and, for animation, clip and Once/Loop choices from that binding. It also restores
the already decoded candidate so clip choices are available. It copies the local
model transform, not the scene body's world position. It does not assign, save,
change selection, dirty the binding document, add Undo history or change the
simulation.

Edit the desired values, then use the existing **Assign static/animated model**
and **Save model bindings** actions. Assignment rereads and verifies installed
content and applies normal scene, selection, bounds and animation-workload checks.
Copying cached settings is not approval to use a changed package; stale content
still requires an explicit reload and confirmation. The open binding project is
retained. Changing selection does not automatically overwrite the draft; click
again when you want the new selection's settings.

The button is unavailable for an unbound selection and while normal editing is
blocked. Missing cached models, stale binding/model identities, incoherent rows,
Play, previews, remote sessions or another scene leave the form unchanged and
report the existing validation error. Static settings reset the inactive animation
draft to clip0/Once; animated settings retain their exact stored descriptor.

Focused acceptance uses CPU hosts/widgets: copy is read-only, Undo still targets
the previous actual edit, edited transforms survive Save/reopen, invalid and Play
copies preserve drafts, and saved animation drives the host-timeline sampled
pose even when the unassigned form contains different clip settings. Actual
native-window/GPU/player appearance and playtest video remain unverified.
