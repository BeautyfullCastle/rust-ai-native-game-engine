# Frame-owned point navigation

`orr_navigation_runtime` is an optional, deterministic adapter from the validated `orr_navigation` heightfield graph to an `orr_ecs::Frame`. It supports one point agent on terrain of at most 17 × 17 vertices and a route of at most 512 triangles. Radius, headroom, and maximum step height are zero. Slope is a nonnegative rise/run ratio; `distance_per_tick` is a nonnegative fixed-point 3D distance budget. It does not offer clearance-aware navmesh routing or a funnel.

Register with `NavigationRuntime::register(builder)`. `admit(frame, terrain, graph, spec)` validates the terrain, canonical graph, profile, and complete route before changing the Frame. It spawns the one derived agent and stores canonical terrain bytes, graph bytes, and `NavigatorSnapshot` bytes in three `FrameList<u8>` values. The `RuntimeState` singleton holds those handles and the agent entity; `RuntimeAgent` holds the request. All authoritative route state is therefore in the Frame, including its checksum, clone, serialized snapshot, and rollback state. There is no external authoritative cache.

`step(frame)` reconstructs and validates these bounded values from the Frame on every call, advances once, and publishes the new navigator bytes only after success. Decoding caps canonical terrain, graph, and navigator lists at 2,866, 34,410, and 32,942 bytes respectively before constructing the graph or route. No filesystem, wall-clock, physics solver, or uncaptured terrain source is read during ticks. Restore or seek the whole Frame through the ECS snapshot/copy mechanism, then call `step` again to deterministically replay. The adapter's read-only `validate`, `agent`, `decode_scene`, and `decode_readback` APIs support host and view integration. The byte lists can be read through `FrameView` without making the view authoritative.

Admission, restore, and stepping are failure-atomic: malformed counts, cursors, status, route records, dependency revisions, stale handles, missing/deleted agents, unsupported profiles, unreachable paths, and out-of-bounds data fail without publishing a partial state. The `Navigator` loader compares every stored route record with the canonical route recomputed from the supplied terrain and graph. It also verifies the current position is a safe continuation within its corridor triangle. A validated snapshot is **not** proof that a particular history of movement produced that point: applications requiring anti-cheat provenance must authenticate full Frame snapshots or replay trusted inputs. Never treat user-supplied bytes as authenticated movement history.

The runtime does not silently replan when terrain or graph bytes change. Admission must be performed explicitly with validated dependencies. It rejects an oversized terrain before graph reconstruction. Core `orr_navigation` continues to support its existing, larger limits; these smaller limits apply only to this adapter.

Run focused checks with:

    cargo test -p orr_navigation -p orr_navigation_runtime
    cargo clippy -p orr_navigation -p orr_navigation_runtime --all-targets -- -D warnings
