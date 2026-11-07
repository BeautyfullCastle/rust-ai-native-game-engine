# Saved Room fixed and carried point lighting

The optional `room-lighting` feature adds one presentation-only, unshadowed point light for schema-2 `room-escape-v1` projects. It is default-off in `orr_sample` and `orr_editor`; simulation, input, replay, Frame bytes and checksums are unchanged. The native light illuminates imported model receivers through the existing renderer; procedural proxies retain their existing shading. This adds no additional lights or new shader.

## Declare and author

Add `"lighting":"room.lighting.json"` to the manifest entry and create that distinct root-level regular file:

```json
{"version":1,"point_light":{"position":[0,4,0],"color":[1,0.9,0.7],"intensity":3,"range":12}}
```

Open the complete project in an editor built with `room-lighting`. Enable **Point light enabled**, edit its position/linear RGB/intensity/range and choose **Apply lighting**, then **Save lighting**. Undo/Redo retain32 applied changes. Reopen restores saved settings.

For a carried light enable **Carry light with player** and edit **Carried offset X/Y/Z** before Apply/Save. The document becomes version2:

```json
{"version":2,"point_light":{"position":[0,4,0],"color":[1,0.9,0.7],"intensity":3,"range":12},"follow_player":[0,2,0]}
```

In Play, editor presentation, native drawing and captures resolve the sole current PLAYER/body from the same immutable rendering Frame. Light position is current world PLAYER position plus saved offset; color, intensity and range remain authored. Offset is in world axes, independent of player rotation/model-local transforms. **Position X/Y/Z** remains the fixed Edit fallback; Edit/Stop shows it. Seek uses the sought Frame directly without motion history. Runtime restart resumes the saved policy at the restarted player position. Disabling carry returns version1 while retaining fallback/appearance; disabling the point light clears the carry policy.

**Discard lighting draft** restores the currently applied settings, including unsaved applied values, without applying, changing history or writing. Invalid drafts can be discarded. Editing requires coherent local Edit mode; Play, remote editing, stale/replaced sources or changed manifest/sidecar bytes refuse authoring. Replacement/restart retires old source tokens. Save As requires reopening the complete project.

## Activate from actual gameplay

With the light enabled, **Light activation** selects **Always**, **Key collected**, **Escaped**, **Before key pickup** or **Before escape**. Apply and Save the choice. Conditional documents use version3 with required `"activation":"key_collected"` or `"activation":"escaped"` (the Before choices use `"key_missing"` and `"before_escape"`); the existing optional `follow_player` array combines with any condition. **Always** removes the condition and returns to version1 (fixed) or2 (carried).

Edit/Stop previews the authored light. Play reads actual `RoomRun` from the same rendering Frame: Key collected enables once `key_collected != 0` and remains enabled after escape; Escaped requires `won != 0`. Before key pickup stays active while `key_collected == 0`; Before escape stays active while `won == 0`, including after pickup. Seeking/restarting before the outcome restores these lights. No interaction key or elapsed tick substitutes for those outcomes. Seek/restart to before pickup/victory disables Key collected/Escaped again, without an event journal. Invalid settings and carried body data still reject while inactive. Disabling the light clears both carry and activation.

## Admission and persistence

The limit is4096 bytes. Version1 requires `point_light` (null disables it) and forbids follow data; its original renderer-compatible bytes are preserved. Version2 requires an enabled point light and `follow_player`, exactly three finite offsets within -32..32. Missing/null/duplicate/unknown fields, positional objects, unsupported versions and invalid numeric values reject. The followed PLAYER position must also be within -32..32 per axis; missing/duplicate roles or missing current Body generation fail closed. Canonical Room scene admission still owns gameplay's narrower spatial contract. Frame APIs require registered Room component types.

Light fallback position is finite within ±1e9; linear RGB/intensity0..1e4; range1e-6..1e9. Paths must be distinct contained regular files, with existing symlink/alias restrictions. Undeclared sidecars are ignored. Feature compilation is not consumer consent: callers use explicit Room lighting capabilities; ordinary `PreparedProject::open` rejects declared lighting.

A failed pre-replacement write preserves bytes/baseline/history. Replacement followed by directory-sync failure reports uncertain durability and advances the saved baseline. This detects ordinary external edits, not hostile filesystem races.

## Use and verification

Run `room_escape` with `room-lighting` or export through `orr_export_room` with `room-lighting,project-export` and a trusted lighting-capable runtime. Export retains exact document bytes and verified package closure. Headless execution validates resolved lighting; explicit `--check-model-poses` additionally prints the carried light position. Conditional lights also print `room lighting active: 0/1` under that explicit option; inactive carried lights print no position. Normal stdout retains the exact export smoke protocol.

Focused CPU acceptance covers strict/legacy documents, real movement, immutable failures, limits, editor controls/history/Save/reopen/Seek/Stop, source retirement and source-hidden exported native headless position parity against deployed model placement. Existing GPU acceptance is separate and requires its explicit adapter/isolation flags. CPU checks do not establish native-window, GPU pixels, audio-device or Windows positive-publication success. Existing Windows publication access failures remain unresolved and are not retried. This adds no shadows, light occlusion, flashlight cone, probes or general light attachment graph.
