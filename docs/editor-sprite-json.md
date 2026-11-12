# Create a sprite package from JSON

Build the editor with the default-off `sprite-json-authoring` feature. It adds
the sprite package installer and PNG decoder dependencies, but does not enable
grid or rectangle authoring.

```sh
cargo run -p orr_editor --features sprite-json-authoring
```

Open a matching local editable Arena or Collect scene with sprite bindings.
In the Sprite panel, expand **Create sprite package from JSON**, enter paths to
the `SpriteDocument` JSON and its PNG, then choose **Inspect sprite JSON and
PNG**. The editor validates the bounded JSON document, decodes the explicitly
selected PNG, and checks that their atlas dimensions agree. Review the reported
dimensions, regions, clips, and SHA-256 hashes before proceeding.

Enter a license file, package name and version, and a new package destination.
Select **Sprite JSON rights acknowledgement** and choose **Create sprite JSON
package**. Creation rechecks the JSON against its inspected hash, then writes
the package through the existing checked package authoring path. Choose a new
destination for each creation. This step only creates files: it does not
install the package, load its sprite document, assign bindings, or edit or save
the scene. Use the existing local package preview and install controls, then
load and assign the document explicitly when ready.

The clip sequence editor exports JSON only. To package that result, provide the
exported JSON and its atlas PNG as separate inputs here. The selected PNG is
hashed, dimension-checked, and decoded; those checks do not prove that its
pixels visually match the regions and clips in the JSON. Confirm that it is the
intended atlas before creating the package. See [Author a sprite clip from
admitted regions](sprite-clip-sequence.md) for the JSON export workflow.
