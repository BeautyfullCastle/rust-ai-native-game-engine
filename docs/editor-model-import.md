# Importing raw GLB models in the Room editor

The editor can turn a local, self-contained GLB and a separate UTF-8 license
text file into a new local content-package source. It does not download model
content, execute package scripts, or install or assign the package as part of
import. The workflow is available in a build with `package-manager`; animated
model import for Room requires `room-animated-models`.

## Start a Room project

On Windows x64, build the editor with both features:

```powershell
cargo run -p orr_editor --features "windows-projects room-animated-models" -- --room-project "E:\Games\MyRoom"
```

The project must already be a valid Room project. A clean local Physics editor
can create one through **File → New Room project…**. The project workflow
preserves the normal Room admission, camera, and model-sidecar checks.

## Import and use a model

1. Open **File → Content packages…** and expand **Import a raw GLB package**.
2. Choose the `.glb`, a UTF-8 license text file, a new package destination
   directory, and a package name/version. The destination's parent must exist;
   the destination itself must not. Keep generated package sources outside the
   project and its `.orr` store.
3. Select **Static model** or **Animated model**. Animated import is available
   only when the editor was built with `room-animated-models`.
4. Review the content rights yourself and check **I have reviewed rights to
   distribute this content**. The acknowledgement is required for each import
   and clears when either input file changes and after an attempt.
5. Click **Import raw GLB package**. Inspect the generated `orr.package.json`,
   `model.glb`, and `LICENSE.txt` in the new source directory.
6. Use **Preview package addition**, **Add previewed package**, and **Verify
   installed packages** as separate actions. Only a verified installed model
   appears in the corresponding **Use model …** or **Use animated model …**
   action.
7. Select the intended entity, use the verified model, and explicitly save
   model bindings in the Inspector. Room scene saves and model-binding saves
   remain separate; reopen the project to confirm the saved binding.

The importer validates the source as a GLB before publishing a new package
folder and copies the model bytes unchanged. External buffer/image resources
are rejected, even if a neighboring file exists. Inputs and output paths are
bounded and checked for Windows filename/path hazards; existing destinations
are refused rather than overwritten. A supplied license file is metadata, not
a rights determination or legal review. Import does not establish that the
package's claimed capabilities are supported by every consumer; installed
package verification and model loading perform their own checks.

## Windows UI acceptance

The UI acceptance test is opt-in and requires a real hardware Vulkan adapter.
For a retained project, raw source files, package sources, screenshot, and JSON
receipt, point `ORR_RAW_MODEL_IMPORT_OUTPUT` and
`ORR_RAW_MODEL_IMPORT_CAPTURE_DIR` at fresh absolute paths. Set
`WGPU_BACKEND=vulkan` and `ORR_RAW_MODEL_IMPORT_ACCEPTANCE=1`, then run:

```powershell
cargo test -p orr_editor --features "windows-projects room-animated-models" --test raw_model_import_ui -- --exact raw_static_and_animated_models_import_through_editor_and_survive_reopen --nocapture
```

To include a same-editor Windows export, also set
`ORR_WINDOWS_EXPORT_RUNTIME` to a trusted runtime executable and
`ORR_RAW_MODEL_IMPORT_EXPORT_BUNDLE` to a fresh absolute destination. The test
fails closed if an export destination is supplied without a runtime.

Animation acceptance samples the assigned clip at an authoritative paused
simulation tick and compares the pose used by the editor with the clip sample
at that time. This checks the sampled pose and save/reopen/play lifecycle; it
does not exhaustively prove every intermediate pose, frame rate, or animation
state for all possible models.
