# Stormglass Foundry

An original, optional cooperative tower-defense sample for Orrery. Two foundry
terraces defend one reactor through ten waves, including bosses on waves 5 and
10. The second board is controlled by a **local AI ally**, not an online player.
Names, industrial-garden presentation, rules and balance are original.

## Play

Enable `orr_sample/coop-defense` and run the `coop_defense` binary. The browser
adapter in `orr_coop_defense_web` runs the same Rust simulation, not a JavaScript
reimplementation; use `tools/build_coop_defense_web.sh` for its web distribution.
Both frontends show their current controls on screen.
Browser shortcuts use **Q** (Summon), **E** (Upgrade), **R** (Invest),
**Space** (Pause/Continue) and **Escape** (clear selection). Ctrl, Meta, Alt
and text-composition events are left to the browser or input method without
changing game inputs. Shift alone preserves these controls.

- Summon costs 25 gold, randomly chooses a role and fills the first empty tile
  on your board.
- Select a unit, then an empty tile to move it. Select an occupied tile with
  the same role and tier to combine **two** units into the next tier, up to V.
  An incompatible occupied tile is not a merge.
- Upgrade improves your board's attack strength. Invest increases periodic
  income. Both have increasing prices and capped levels.
- Rivet tripods fire rapidly; kiln mortars splash groups; prism towers bypass
  armor; moss condensers slow enemies. Haste beacons accelerate your board's
  firing, while mining drills earn extra gold.
- Enemy families have different movement, armor and health. All escaped enemies
  damage the shared reactor. Projectiles apply damage when they arrive, not
  when their firing animation starts.
- Victory or defeat can restart from the same seed.
  Replaying identical actions with that seed gives an identical checksum.

An accepted browser move or merge ends any pointer gesture that began against
the previous board state. Its destination stays selected; start a fresh gesture
to move again. Cancelling an old drag cannot select its now-empty source.

Native pointer actions use the control or board cell where a primary press and
release occurred, not the final hover position of a rendered frame. Both ends
must identify the same target without a layout change; a drag, cancelled pointer
or lost focus cannot turn into a click. At most one eligible pointer target is
handled per frame. Board movement remains two clicks, not drag-to-move.

Native board clicks pause briefly while a board action is queued or awaiting its
two-tick consumption. Click again afterward; clicks are not buffered. A pending click cannot clear selection or
become a later action.

In native play, use **P**, **Escape** or **Pause** to stop active combat. Losing
window focus also pauses combat or a terminal run with an admitted Restart
still waiting to start play. Returning does not resume: press
**Enter** or click **Continue**. While paused, the game frame and economy stay
frozen; gameplay keys and board clicks are blocked. Continue only resumes and
does not summon, move or restart. Pause and focus loss take priority over other
controls in the same input frame. Unsubmitted UI commands and old gestures are
cancelled, and elapsed paused time is not caught up. Inputs already admitted to
the deterministic session retain their existing two-tick delay and may finish
after explicit resume; pause does not rewind accepted input.

The optional Linux `coop-defense-run-save` feature adds explicit native Save
Copy/Load controls and a player-selected save directory. See
[native run saves](coop-defense-native-run-save.md) for same-executable identity,
the ten-minute journal cap and failure behavior. The web
[Run Save v1](coop-defense-run-save.md) format is separate.

The optional Linux [native audio consumer](coop-defense-native-audio.md) uses
the existing authored bank with real Session events. Device output is explicit;
muted startup and offline mixer checks do not prove physical audibility.

## Integration boundary

`orr_games/coop-defense` is disabled by default. It contains only bounded,
integer-valued simulation state in Orrery's Frame, with fixed actor pools and
monotonic input action IDs. `orr_sample/coop-defense` adds the native view and
input adapter. `orr_coop_defense_web` is a separate opt-in browser adapter.
Neither presentation is allowed to mutate gameplay state directly.

The simulation's 1000 by 700 logical coordinates are shared by both views. Each
of two boards has 4 by 3 tiles. Input cell indices are local to the human board;
the local ally has its own economy and ordinary legal action policy. Input
sequence numbers must remain increasing across restarts, so held or repeated
samples cannot spend twice. No account, matchmaking, external assets or network
service is needed for this local cooperative slice.

## Acceptance and limits

`cargo test --release -p orr_games --features coop-defense` checks simulation
contracts. The `coop_defense_run` example exercises legal strategy inputs to
completion and replays the exact input history; `--afk` verifies that leaving
all decisions to the ally is not enough. Use `--seed N` and `--cadence N` to vary
the seed and the human action interval in ticks. This automated oracle is not a human playtest.

Native Linux and browser acceptance are separate. Actual-WASM/mock-DOM tests
check shortcut dispatch and resource/recording neutrality, not real browser
shortcut handling, pixels, physical input or Windows execution.
There is no claim of networked co-op, cross-build save migration or production balance.
