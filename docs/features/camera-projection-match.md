# Camera projection matching

The optional mesh camera authoring panel can switch perspective/orthographic
projection while preserving apparent scale **at the camera target plane** for an
explicit aspect ratio. It supports the existing MeshNavigation/MeshHazards camera
profiles; it is not a renderer or sidecar schema change.

Enter the intended aspect width and height (integer 1..16384, default16:9), then
choose **Switch projection keeping target-plane scale**. This is explicitly not
an automatic measurement of the current viewport. Perspective-to-orthographic
computes half-height; the reverse computes vertical field of view. Existing
portrait expansion is accounted for. Distance, pose, follow offset and clip planes
stay unchanged. The ordinary projection radio controls still allow manual switching.

Matching changes only the draft form. **Apply game camera** and **Save game camera**
remain separate actions. Play, stale scene/graph/load identity and invalid sidecar
restrictions remain in force. Invalid input or a result outside existing camera
limits produces an error without changing the draft projection. Values are not
silently clamped. Aspect controls are transient authoring aids, not saved settings.

Acceptance checks cover target-plane screen coordinates at square, landscape and
portrait aspects, round trips, invalid inputs/unrepresentable outputs, and real
panel interaction with explicit Apply/Save boundaries. This does not preserve
perspective appearance at other depths or promise identical framing at other
aspect ratios. Existing camera export/Player consumers need no new capability.
