# Edit selected Room wall height

In **Room level authoring**, choose **Read Room walls** and select an unbound
wall. Edit **Selected wall center Y** and **Selected wall half-height Y**, then
choose **Apply wall height**. The existing wall GUID, name and X/Z footprint are
preserved; the edit is one authoritative undoable transaction.

Center Y must lie in [-4, 4] and half-height in [0.1, 8]. The vertical span must
cover the complete player height plus collision skin. Floating or nonspanning
walls, out-of-bounds values and stale reads are refused without partial changes.
Read again after another scene edit before applying a cached selection.

Use normal Undo, Redo and Save. Reopening retains the changed height. This changes
neither the Room physics rules nor model bindings; it exposes already-supported
wall geometry with the same admission requirements.

The synthetic software-Vulkan UI test creates a wall, verifies visible taller
pixels, invalid/stale refusal, identity and footprint preservation, persistence
and player collision. Its receipt records the actual adapter; it does not claim
physical OS input or hardware-GPU coverage.
