# Authored Room camera and PLAYER tracking

A schema-2 `room-escape-v1` project entry may name a root-level camera sidecar. Camera data belongs to presentation, affects exported content identity, and never changes Frame bytes, checksums, physics, input or replay. Arena/Collect cannot declare this profile. Generated Room projects include `room.camera.json`; older projects with no camera retain their previous native/capture/editor defaults.

## Data contract

Schema 1 requires `schema`, `target:[x,y,z]`, `yaw`, `pitch`, `distance` and `projection`. It remains a fixed camera with byte-compatible serialization. Schema 2 requires the same fields plus `follow_player:[offset_x,offset_y,offset_z]`. Missing/null/wrong-sized offsets, schema 1 with an offset, unknown/duplicate fields and invalid numeric values reject. Schema 3 additionally requires `target_bounds:{"min":[x,y,z],"max":[x,y,z]}`. Both three-component arrays are required; nested unknown/duplicate fields, nulls and positional objects reject. Bounds are forbidden in schemas 1/2, preserving their bytes.

- Authored/session target components: finite -64..64.
- Follow offset components: finite -32..32.
- Tracking bounds: finite -64..64, minimum <= maximum on each axis; equal endpoints lock that target axis.
- Yaw: -pi..pi; pitch: -1.4..1.4 radians; distance: 1..128.
- Perspective: `{"type":"perspective","fov_y_degrees":20..100}`.
- Orthographic: `{"type":"orthographic","half_height":1..64}`. Vertical half-height is `half_height * session_distance / authored_distance / min(aspect,1)`.
- Dimensions: 1..16384 per axis; document limit:4096 bytes.

Projection shapes are closed: fields for the other projection, nulls, arrays instead of objects and missing/duplicate fields reject. Matrices and screen rays must remain finite. These fixed clipping planes suit bounded Room levels, not a general large-world rig.

## Author and use

In the **Room camera** panel, enable **Follow player during play**, set the three follow offsets and choose **Apply camera**, then **Save camera**. Apply/Undo/Redo reset the session view to the applied profile. History holds32 applied changes, independent of scene/model history. A different successful Apply clears Redo; invalid or identical Apply preserves it. Save preserves history; reopening starts empty. Invalid Apply leaves the applied camera and preview intact. Editing is disabled in Play or incoherent local snapshots.

Edit displays the authored `target`. During Play, the camera resolves the sole current PLAYER and its current Body from the same immutable Frame used for rendering. Its target is:

`PLAYER position + saved follow offset + (session target - authored target)`

Enable **Limit tracking target** and author **Target min/max X/Y/Z** before Apply/Save to constrain tracking. Schema 3 clamps the complete player + offset + session bias result to these inclusive bounds. Schemas 1/2 keep their previous behavior. This limits the target, not the entire view frustum; it does not prevent visible space outside a level or camera/wall overlap.

Without authored bounds, resolved target components clamp to -64..64, so extreme valid session pans stay finite as the player moves. Missing/duplicate PLAYER, missing current Body, invalid session or player positions outside the camera's -32..32 numeric envelope fail closed. Normal Room scene admission remains responsible for gameplay's narrower spatial contract.

Pan changes a transient bias relative to the moving player; it does not suspend tracking. Orbit and zoom retain their projection-aware behavior. Reset view clears the bias and restores authored angles/distance. Disabling target limits returns to schema 2. Disabling follow or **Reset camera defaults** clears tracking bounds and restores a fixed profile. Stop returns to the Edit view. Seek follows the player in the sought Frame; no motion history or smoothing clock is needed. Runtime Restart restores the admitted profile/session.

Viewport capture records Edit framing and preserves the saved follow offset, bounds and policy. It does not convert a transient Play tracking target into authored data. Source/capture/native/editor camera construction uses the shared resolver; picking and overlays use the camera for the same presentation. A failed presentation may retain its last image, while picking/overlays are disabled rather than using new-size rays on old-size pixels.

## Saving and source lifetime

Save replaces only the camera sidecar, not scene/model files or manifest. It pins the project directory, compares exact admitted manifest and saved bytes, rejects changed/nonregular/symlink paths, writes and syncs a same-directory temporary file, then atomically replaces. Unix directory-sync failure reports uncertain durability and advances the baseline to the already replaced bytes. This is ordinary filesystem validation, not hostile-race isolation or a multi-file crash transaction.

Opening a scene, even at the same path, clears project camera admission; reopen the complete project. Successful Save As elsewhere clears it; failed Open/Save As preserves it. Returning to an old path cannot silently reactivate admission.

## Verification scope

Follow-camera acceptance covers strict/legacy data, boundary saturation/axis locks, real game movement, finite projection/rays/aspect extremes, read-only Frame/session behavior, policy-preserving capture, actual headless editor input/Save/reopen/pan/reset/Seek/Stop and source-hidden deployed game/native headless bounded-target parity. Old GPU test targets are compile-only in this focused workflow. Headless `--check-model-poses` additionally prints a tracking target; normal stdout retains the exact export smoke protocol. No current native-window, GPU pixel, audio-device or positive Windows publication claim is made. Existing Windows publication access failures remain unresolved and are not retried. Damping, collision avoidance, cutscenes and generic camera rigs remain outside this feature.
