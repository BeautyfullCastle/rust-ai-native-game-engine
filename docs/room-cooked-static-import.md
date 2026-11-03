# Import a cooked static model into a Room package

The optional `package-manager` UI can wrap an existing, self-contained
`orr_static_model` document in a new immutable model package. This connects
cooked `.model.json` output from existing model-authoring tools to the Room
Content workflow without manually writing a package manifest.

## Workflow

Open an admitted Room project and expand **Import a model package**. Choose
**Cooked static model**, supply the `.model.json` file, a nonempty UTF-8 license
file, a new destination directory and package name/version. Review the rights
acknowledgement, then **Import cooked static package**.

The package contains the original model bytes as `model.orrmodel`, the exact
license bytes as `LICENSE.txt`, and its manifest. It declares `models` only.
Importing does not install the package, assign a model or save the scene. Use the
existing package Preview/Add flow, then Preview static model and Use model.
Use model explicitly assigns the selected entity; Save model bindings separately.

The fixed output filename matches the existing Content preview interface. It
neither rewrites the model's embedded asset identity nor expands the generic
model catalog's filename rules. Raw GLB remains the default import mode, with
its existing static/animated choices and output unchanged.

## Preserved contract

The complete input is validated by the existing `StaticModel::from_bytes`
loader without a GLB fallback. Geometry, images, materials, embedded asset ID
and every historical dependency digest remain byte-for-byte unchanged.
Historical dependency paths record provenance; they are not package dependency
declarations and are not fetched or evidence that original files are available.
No format conversion, texture editing, decoder, scene schema or runtime changes
are involved. Cooked animated inputs and Animated-kind combinations are rejected.
MeshNavigation's separate provenance/capability admission is unchanged.

The existing 64 MiB model-file limit and model validation limits apply. License
input is bounded to 1 MiB. Invalid content, missing rights acknowledgement or an
existing destination rejects without replacing existing output. A supplied
license is metadata, not legal advice or proof of rights.

## Publication and context

The existing create-new publisher requires trusted, quiescent paths. It does not
provide hostile concurrent-path replacement protection or atomic directory
visibility. On output failure, cleanup and any remaining partial output are
reported; successful publication is separate from project activation.

Format/source changes invalidate the acknowledgement. A worker owns captured
inputs, and a completion for a changed project/source context does not select
its package into the current project. A package already published for an old
context is reported honestly rather than described as rolled back.

## Acceptance scope

Focused checks cover exact bytes/provenance/license, malformed data and limits,
mode separation, no-clobber/failure behavior and stale completion. The actual
Editor acceptance imports an authored cooked asset, explicitly installs and
assigns it, saves bindings, hides source paths and reopens a fresh Room consumer
with matching rendering. Software offscreen rendering is not native-window,
physical GPU, Windows distribution or export acceptance.
