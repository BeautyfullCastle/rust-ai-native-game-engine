# Make a prop package from one GLB subtree

In Model import, choose a raw GLB and Static mode, enable subtree extraction, inspect the source and select an original node index. Names are advisory and may repeat; the index identifies the selection. Only geometry-bearing nodes in the imported default scene are offered. The default scene is the explicit GLB scene, or scene zero.

Choose the new logical .glb asset identity, package name/version, an existing license file, an unused destination and explicit license acknowledgement. Import produces an immutable cooked-static package containing only the selected subtree primitives. Install it and assign its model.orrmodel asset through the normal static-model controls; Save persists the assignment.

The selected geometry keeps its source-world transforms, primitive order and material/image tables. Extraction does not recenter a pivot, discard unsupported sibling content, generate collision or modify the original GLB. The whole supported source is validated before selection. Only bounded embedded GLB resources are accepted; external resource resolution is excluded. Whole-GLB import remains the default when extraction is off.

Changing source, selection, project or package inputs invalidates the prior authoring consent. Import checks the inspected source digest again before publication. Save/reopen and CPU Room geometry tests verify the installed selected prop. Windows runtime, GPU pixels, native export, physical play and video remain unverified.
