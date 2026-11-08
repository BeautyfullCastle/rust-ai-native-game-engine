# Place a Mesh point light from the viewport

Mesh game lighting supports selecting the X/Z location of an enabled fixed point light. Enable the point light, choose Pick point light X/Z, then click the viewport. The camera ray intersects the world Y=0 plane; the hit supplies X/Z while the existing light height remains unchanged.

The operation stages a lighting draft. Color, intensity, range and other lighting settings remain unchanged. Use Apply game lighting to update applied lighting, then Save game lighting to persist it. Undo and Redo use the existing lighting history. The scene Frame and simulation history do not change.

Pick requires the matching local Edit scene and a valid lighting draft. Perspective and orthographic views are supported. Parallel or backward rays, invalid cameras, viewport misses and invalid coordinates leave the draft unchanged. A completed current viewport frame is required; failure or unavailable rendering cancels the request.

Escape or Cancel point light pick cancels selection. Focus loss, source/context changes, lighting edits, preset loads and history actions retire the old request. Restoring the original values does not restore an old ticket. The picker owns a cancelled press through physical release, so that click cannot select objects, move the camera or start another gesture; release before picking again. Wheel input received during selection is prevented from becoming camera zoom while its smoothing drains; new scrolling works after it settles.

The saved lighting file is checked at pick start and immediately before staging. External changes require reloading. This retains the existing bounded local-file and optimistic content checks; it does not add concurrency isolation. Invalid operations preserve the existing draft and saved settings.

Focused automated acceptance covers ray geometry, preservation of height/settings, source and draft lifetime, real widget Apply/history/Save/reopening, App input fencing and existing saved/native point-light consumers. These are CPU and filesystem checks. Windows execution, GPU pixels, physical play and video remain unverified unless separately demonstrated.