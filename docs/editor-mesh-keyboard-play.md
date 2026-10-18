# Mesh keyboard gameplay

Mesh keyboard gameplay lets you retarget the mesh agent with short world-axis goal steps while realtime MeshNavigation Play is active. It uses the existing managed input lease and `MESH_SET_GOAL` simulation path. It is point navigation, not velocity control: releasing a key does not cancel a goal already queued for the agent.

## Use

1. Open or create a MeshNavigation scene and place a valid mesh agent.
2. Start realtime Play and click **Enable mesh goal input** in the **Mesh Play input** panel.
3. Focus the **Mesh gameplay viewport** (or choose **Focus mesh keyboard controls**) before using the keyboard.
4. Press `W`/Up, `S`/Down, `A`/Left, or `D`/Right to move the requested goal by a half-unit step on the world X/Z axes. Diagonal input steps on both axes. OS key-repeat may request additional steps; opposite directions cancel.
5. Disable input or stop Play when finished. Viewport clicks remain available as an alternative goal input.

Each accepted step is projected onto the currently admitted mesh graph and must remain within the managed coordinate bounds and a connected walkable path. Off-graph, disconnected, or out-of-range requests are rejected without replacing the current goal. The agent follows the normal mesh-navigation route on simulation ticks; key release is not a stop command.

Keyboard control is active only for the current live local Play session with the managed input lease. Text-entry focus, dialogs, a different/unfocused window, paused or sought playback, preview state, stale scene/graph context, Escape, modifier shortcuts, or a legacy/incompatible player state blocks input. These cases do not enqueue a goal. Explicitly focus the game viewport or keyboard controls again before resuming input.

## Runtime and limits

The editor and standalone mesh player use the same production viewport input route. Keyboard and click requests change the mesh agent's gameplay goal through the managed simulation input; they do not issue scene-edit patches or create editor undo entries. The feature does not add scene fields, change the `YardInput` layout, provide physics/collision control, or imply support for multiple agents. Player Replay uses recorded-timeline playback through the existing seek/clock path; it is not an independent viewer resimulation.

Interactive exported bundles use the existing strict `mesh-click-to-move-v1` input profile and require a player advertising `ORR_MESH_INPUT_V1`. Bundles without that profile remain replay-only; keyboard goal input is not enabled for legacy players.

## Acceptance

The opt-in hardware UI acceptance uses `ORR_MESH_KEYBOARD_ACCEPTANCE=1`, `ORR_REQUIRE_GPU=1`, `WGPU_BACKEND=vulkan`, and a fresh absolute `ORR_MESH_KEYBOARD_CAPTURE_DIR`. The export flow uses `ORR_MESH_KEYBOARD_EXPORT_OUTPUT` and `ORR_MESH_EXPORT_RUNTIME`. It drives the production UI with synthetic keyboard events and makes no claim of physical OS input. The acceptance also covers invalid-edge, text-focus, pause/focus, and Escape refusal behavior, movement and mission completion, recorded-timeline replay, source-free reopen, and the interactive export profile. This guide describes intended behavior; run receipts record whether a particular validation attempt passed.
