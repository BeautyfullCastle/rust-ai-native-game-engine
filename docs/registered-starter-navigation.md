# Play the registered starter scenes

Delivery and Switchboard use the existing registered Editor, semantic ERP bridge
and project-owned presenter. Their shipped command-mode projects retain the
original objectives: deliver at `[4, 0]`, or match switch target `5`. The shared
router, authentication, leases and scene authoring contract are unchanged.

## Controls

Open either shipped `.project.json` with the starter editor's `--project` option,
choose Play, then explicitly take control. The presenter emits one intent at a
time; it never changes simulation state directly. API clients discover the same
catalog through `editor.gameplay.describe` and use the existing fenced
submit/status protocol in [the ERP guide](registered-gameplay-api.md).

- Delivery `move {"dx":1,"dy":0}` moves one unit right. Both integer fields are
  required, each is in −1…1, and the zero vector is rejected. Diagonals move both
  axes once. Four completed right moves reach the stock goal; `act {"press":1}`
  then wins. Movement after victory is rejected until Act restarts the game.
- Switchboard `select {"delta":1}` selects the next switch once, wrapping modulo
  four; `-1` selects the previous one. Zero is rejected. Act toggles the selected
  switch. Act on switch zero, two completed next selections, then Act on switch
  two solves the stock puzzle. Selection alone never toggles or resets it.
- A completed Act after victory restarts. Delivery preserves its existing reset
  to `[0, 0]`, including with an authored nonzero starting position. Switchboard
  resets bits/moves/solved while retaining selection and cursor y.

Wait for command completion before submitting the next intent. Consumption
requires the expected exact game state; completion additionally requires a newer
frame after neutral admission. Neither a queued request nor an input ACK proves
that the requested game action happened.

## Versioned input behavior

The existing `deliver` and `toggle` fields now have three named modes:

- `0`: released primary action; ordinary nonzero axes still repeat every tick.
- `1`: existing edge-triggered primary action with ordinary per-tick axes.
- `2`: navigation-only pulse. Axes apply once on a released-to-held edge. Holding
  the pulse, or changing its direction while held, cannot repeat movement.

The existing boolean latch covers both nonzero modes. Switching 1→2 without
neutral cannot navigate. Switching 2→1 cannot trigger a primary action, although
mode-1 axes resume their existing per-tick behavior. All-zero default input is
the router's neutral release. Switchboard retains its existing x canonicalization
on mode-0/1 ticks. Legacy keyboard mappings remain mode 0/1.

Delivery pulse endpoints use checked fixed-point arithmetic. If either axis
would overflow, the simulation consumes the latch without moving either axis;
the semantic encoder rejects that request before submission. Switch selection
sets x from the resulting selected index and preserves y, even if the authored
cursor x was noncanonical. Navigation never delivers, toggles or restarts.

Finite traces/replays admit bounded axes, modes 0…2 and zero Switchboard padding.
Structured ERP enforces reflected mode bounds, but retains its existing broader
direction domain. Binary/direct typed inputs bypass those validators; unknown
mode 3 or greater is an explicit state no-op in both games. This does not add a
general binary-input validator to the engine.

Current builds are Delivery `0x5354415254563201` and Switchboard
`0x5354415254563202`. Input structs, field names and layouts are unchanged, but
input meaning has expanded. Old-build manifests and ORRP recordings are rejected,
not silently migrated or relabeled. Historical evidence remains tied to its old
build. New recordings may mix ordinary held input and navigation pulses.

## Interrupted play and evidence

A delayed observation below lease expiry sees a stable pulse result. Pause,
Stop, release, focus cleanup, cancellation or expiry can invalidate proof after
an admitted pulse already moved once. Cancellation is not rollback. Reclaim
establishes neutral; it never automatically resubmits an uncertain command.
The API-controller focus exception remains as documented in the ERP guide.

CPU tests cover unchanged stock completion/restart, exact receipts, delayed
observation, lifecycle interruption, custom states, replay/seek and rejection
boundaries. The focused gate binds test inventories and copied executables to
the exact source and preserves failures. In-process ERP and CPU egui evidence do
not certify a native window, physical input, browser, GPU or real-model game
creation success. Existing native-window and packaging gates remain separate.
