# 2D map camera bounds

`Camera::constrain_to_bounds(min, max, viewport)` keeps a 2D view within an
axis-aligned map rectangle. It accounts for the viewport aspect ratio. Each
axis that cannot fit inside the map is centered on the map; zoom is preserved.

Apply confinement after following or manual navigation, before drawing:

```rust
use orr_render::{camera::CameraFollow, Camera};

let mut camera = Camera::new([0.0, 0.0], 5.0);
let mut follow = CameraFollow::new();
follow.follow(42_u64);
follow.update(&mut camera, |_| Some([99.0, 0.0]));
assert!(camera.constrain_to_bounds([-20.0, -20.0], [20.0, 20.0], (800, 600)));
// The target remains selected and the view stays inside the map.
assert_eq!(follow.target(), Some(&42));
```

Reapply after resize or zoom. Invalid/non-finite camera or rectangle values,
empty/reversed bounds and zero-sized viewports return false and preserve the
pose. Valid input returns true even when no movement is needed.

CPU tests cover landscape/portrait edges, resize and zoom, small maps,
following back from an edge, rejected inputs and extreme finite coordinates.
This is a reusable view API; no editor control, authored camera schema, game
movement constraint or automatic sample integration is added.
