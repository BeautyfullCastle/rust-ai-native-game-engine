# Import raw Mesh agent animation

Build the editor with the existing optional `mesh-animated-models` feature. Open
an admitted local MeshNavigation scene with a saved graph and agent. Under
**Mesh agent animation**, expand **Import animation source** and provide a local
self-contained animated `.glb`, a nonempty UTF-8 license file, an unused package
directory, a package name and version. Explicitly acknowledge rights before
choosing **Import animation package**. A license file and acknowledgement are not
legal review or proof of ownership.

This connects the existing animated-model importer to the existing Mesh animation
consumer. It does not add glTF features: the current importer subset and all
animation/model limits remain authoritative. See the [glTF specification](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html)
for the source format, not a claim that this importer supports every glTF feature.
External resources, scripts, network fetches and animation-free models are refused.
Source GLB bytes are limited to 16 MiB and license text to 1 MiB.

The new package preserves the original GLB and license bytes as `animated.glb`
and `LICENSE.txt`, plus `orr.package.json`. It declares exactly `animation`, no
package dependencies, and passes the unchanged Mesh animation Snapshot admission.
It does not install into a project or relax the separate Room models+animation
profile. The raw GLB path keeps its existing external-resource restrictions. Fully embedded
`.animated.json` and `.orrmodel.json` sources can retain bounded provenance metadata;
see [cooked variants](mesh-cooked-animation.md). Historical inputs are never read
and their declared hashes are not independently verified.

Import runs on a retained worker. Scene, scene-load epoch, graph revision and
request fields are captured at dispatch. A stale completion never replaces the
current candidate. If a package was already published it remains at its captured
path and is reported, not silently deleted. There is no automatic retry or cancel.
Normal close and workspace replacement wait for terminal, joined worker ownership.
Hard process termination is outside that guarantee.

Successful import stages an animation candidate only. Use the existing Preview,
**Apply mesh animation**, and **Save mesh animation** explicitly. Apply and its
Undo/Redo remain presentation history; importing alone does not modify the scene,
active animation, history or saved sidecar. A saved self-contained sidecar can be
reopened without the original GLB, license or package directory. Runtime animation
time still follows the authoritative Frame tick.

Publication uses a new bounded sibling staging directory and native no-replace
publication. Existing destinations, unsafe ancestry and changed inputs are
refused. Retained identities restrict cleanup to owned staging data. Use trusted,
quiescent local directories; these path checks are not a sandbox against hostile
concurrent writers. A post-publication durability/readback warning means the
output already exists or may exist: inspect it before explicitly retrying.

Focused acceptance covers package bytes and strict capability/provenance admission,
invalid input/output preservation, stale worker handoff and retained worker close
behavior, then actual import widgets, candidate preview, Apply/Undo/Redo/Save,
source-hidden fresh reopen and two time-based pose/pixel states. Headless software
GPU evidence is not native-window, physical-input, Windows or hardware-GPU
acceptance.
