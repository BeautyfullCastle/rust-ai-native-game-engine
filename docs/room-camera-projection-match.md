# Room camera projection matching

Room camera authoring can explicitly switch between perspective and orthographic projection while preserving scale on the camera's target plane at a chosen aspect ratio. Existing direct projection radios keep their behavior. This action changes the draft only.

Open **Room camera** and its **Match projection scale** section in Edit mode. Set **Match projection aspect width** and **Match projection aspect height** to the intended output dimensions, then click **Switch projection keeping target-plane scale**. The default is 1280×720. These dimensions describe an explicit aspect; they are not the current live viewport and are not saved in the camera document.

Review the staged projection value, click **Apply camera** to commit it to camera history, then use existing Undo/Redo and **Save camera**. Reopening or exporting uses the saved camera. Switching does not automatically apply, save or reset the live session camera. Controls remain read-only during Play.

The match preserves the projected size of points on the plane through the authored target, perpendicular to the view direction. At other depths, perspective and orthographic projection differ. This does not preserve every scene pixel, lens distortion, depth perspective or wall occlusion. Choose the aspect where framing matters; a different aspect can produce a different match.

For perspective, the orthographic half-height is computed from authored distance and vertical FOV, multiplied by the shorter-axis aspect factor. The reverse uses the inverse angle relation. The adapter reuses Room's actual camera construction before and after conversion, including finite values, viewport bounds and supported projection limits. A conversion that exceeds those limits fails; it is not clamped. The candidate, applied document, history and files remain unchanged on failure.

Only the projection is replaced. Target, yaw, pitch, distance, schema, follow offset and tracking target bounds are retained. Calculations use the authored document, without capturing transient navigation. Existing orthographic session zoom behavior is preserved. Follow offsets and bounds remain presentation policies and do not change simulation state.

Focused tests compare actual camera screen coordinates against independent target-plane oracles for portrait, landscape and square aspects, both directions, roundtrips and zoom. They also check invalid dimensions/projections, preserved tracking fields, actual matching controls, stage/apply/undo/redo/save/reopen and runtime/export data. Existing camera save/error guards remain authoritative.

CPU camera matrices and UI-harness checks do not establish actual GPU appearance or native window/input/audio/video. A headless exported run is reported separately from a playtest. The connected computer remains locked, and historical Windows publication failures are not retried. Actual playtest video requires separate evidence.
