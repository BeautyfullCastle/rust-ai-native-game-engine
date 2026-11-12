# Authored Navigation capture framing

With optional `authored,gpu`, the existing `navigation_player --scene FILE --ticks N
--capture NEW.png` now fits its top-down presentation camera to the generated
navigation display model rather than a fixed box around the world origin.
Terrain, route ribbons and agent geometry all contribute to the bounds. Aspect
ratio and the full height range determine the orthographic extent and depth;
25% planar padding preserves a visible border. The output remains 1000×750.

This is presentation only: the admitted scene, FP simulation, point-agent rules,
pin/revision, Frame and checksum do not change. The model is generated once and
used for both fitting and rendering. Existing exact-f32 display admission still
applies. Nonfinite, degenerate or poorly representable bounds fail closed rather
than clamping simulation values. Existing capture no-overwrite and uncertain
publication behavior is unchanged. The lab's fixed reference cameras are unchanged.

This does not add a native window, keyboard/mouse control, a pause/session API,
or bundle-mode capture. Actual offscreen software GPU tests are not evidence of
physical GPU or native-window operation. Navigation remains a bounded terrain
triangle point agent, not general navmesh or radius/headroom clearance support.
