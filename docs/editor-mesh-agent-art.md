# Mesh agent art

The optional `mesh-navigation` editor feature can attach a static model as
presentation art for a mesh-navigation agent. The model is cosmetic: it does
not change the navigation graph, route, collision, or simulation state.

## Open a mesh-navigation scene

Build and launch the editor with the optional feature enabled:

```powershell
cargo run -p orr_editor --features mesh-navigation -- --game mesh-navigation --scene "C:\Games\mesh.scene.yaml"
```

In the inspector, use **Static-mesh navigation** to create or load a graph and
place a mesh agent. Save the scene before authoring art.

## Load and save art

Open **Mesh agent art**, select a local package source directory, and choose its
declared model and license assets. Review the license yourself and acknowledge
that review before selecting **Load agent art**. Loading prepares a candidate;
it does not install a package or apply the model. Set the offset and scale,
select **Apply agent art**, and use the explicit art Undo/Redo controls to
inspect the in-memory change. Select **Save agent art** to persist it for the
scene. Removal requires its separate confirmation control.

The saved art lives in a scene-associated sidecar named by appending
`.mesh-art.json` to the complete scene filename (for example,
`room.scene.yaml.mesh-art.json`). It embeds the validated model and license
data, so reopening does not require the original package directory. The scene
graph and agent route remain the authoritative simulation data. Keep the
sidecar with the scene when moving or sharing it. After **Save As**, select
**Save agent art** explicitly to create the sidecar beside the new scene path.
The art-specific Undo/Redo history is bounded to four snapshots.

Source files must be declared by the local package manifest and the model must
be a supported, self-contained static GLB. License acknowledgement records your
review; it is not a legal determination. This feature does not install packages,
fetch content, execute scripts, add collision, or change gameplay. It does not
currently provide standalone mesh-navigation export.

Use stable local source and scene directories rather than symlink/junction
paths. Stored package and asset digests record the provenance inspected when
the snapshot was created; they are not authentication and do not assert that
the package is currently installed.

The filesystem checks use path validation, reparse-point checks, and optimistic
prior-hash checks to detect observed conflicts. They are not a capability-safe
or linearizable conditional write/delete against hostile concurrent path
replacement, and they do not promise crash or power-loss durability. The
embedded cooked-model digest detects accidental payload changes; it is not an
authentication mechanism.

The editor validates the saved model and its placement when loading and while
presenting the scene. It does not exhaustively admit every possible simulation
pose or render every pose at a guaranteed frame rate.
