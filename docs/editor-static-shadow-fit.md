# Fit static shadow bounds

With the optional `mesh-navigation` Editor enabled, open **Mesh game lighting**
and choose **Fit static shadow bounds**. The button proposes a center and radius
for the current authoritative mesh surface and saved, applied static props.
Save or undo pending prop changes first, as required by the existing lighting UI. It does not include agents,
skinned animation, other terrain, or future motion.

This is a one-time **manual draft**, not a live coverage constraint. Refit after
changing geometry or light direction. Review the fields, choose **Apply game
lighting**, then save explicitly. Fit itself changes only the three center
fields and radius; it does not enable shadows, change other lighting, apply a
history entry, write files, or affect simulation. Existing lighting Undo/Redo
and Save/Reopen remain available.

Fit requires a matching editable local scene, a coherent admitted navigation
view, and no unplaced geometry, prop forms, gestures, or pending workspace work.
It uses the currently drafted valid light direction. Failure leaves the four
bounds fields unchanged. Existing scene/Play guards still govern Apply.

## Coverage and limits

The calculation includes transformed primitive geometry, including reflected
primitive/node transforms, using the existing static-prop bounds reader. Existing positive instance-scale
admission is unchanged. Work is bounded
by 128 props, 100,000 vertices per prop and 1,000,000 graph-plus-prop vertices in total.
Finite bounds are accumulated in presentation-only floating-point math. The
result must satisfy existing center/radius limits and contain the bounds with
margin in the renderer's actual texel-snapped projection at 512, 1024 and 2048
shadow-map sizes. Unrepresentable or oversized bounds are refused rather than
silently truncated. Shader, sidecar format, simulation and export policies are
unchanged.

A containing volume does not guarantee good shadow resolution or appearance.
Very distant geometry can make a large, low-detail shadow map. This tool does
not implement a new lighting system, dynamic fitting or a window/runtime input
feature.

## Acceptance

Focused tests cover transformed bounds, signed scales, projection margins,
invalid inputs and work limits, field/history preservation and existing edit
guards. The explicit offscreen Editor test checks Fit before Apply, tiny-volume
and shadow-disabled pixel controls, Undo/Redo and saved reopening. Its observed
adapter/backend is recorded with results; software offscreen evidence is not
physical GPU or native-window acceptance.
