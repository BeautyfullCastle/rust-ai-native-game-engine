# Find an installed model in Content packages

Build the optional editor `package-manager` feature and open a saved Room
project in Edit mode. Choose **File > Content packages…**, then **Verify
installed packages**.

Use **Filter verified models** to find assets by package name or relative asset
path. Matching is case-insensitive; every whitespace-separated word must occur
in the combined `package/path`. For example, `characters hero` narrows a large
library to matching character models. Search uses literal text, not globs.

The result count counts GLB, glTF and Orrmodel assets once each, even when an
asset offers both static and animated actions. **Clear model filter** restores
all models. Changing projects clears the filter and the verified inventory.

Choose a visible **Use model** action to assign through the existing selected
entity workflow, then save model bindings separately. Filtering only changes
the displayed list; it does not install, verify, modify or save content, or
bypass the normal edit-mode and compiled-capability restrictions.

To inspect an asset without selecting an entity, use its **Preview static model**
action. The separate viewer offers orbit, zoom, Reset and Close without changing
bindings or project files. See [Static package preview](editor-package-preview.md).
