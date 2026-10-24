# Stormglass Foundry

An original, optional cooperative tower-defense sample for Orrery. Two foundry
terraces defend one reactor through ten waves, including bosses on waves 5 and
10. The second board is controlled by a **local AI ally**, not an online player.
The design takes broad genre inspiration from randomized cooperative defense;
names, industrial-garden presentation, rules, costs and balance are original.

## Play

Enable `orr_sample/coop-defense` and run the `coop_defense` binary. The browser
adapter in `orr_coop_defense_web` runs the same Rust simulation, not a JavaScript
reimplementation; use `tools/build_coop_defense_web.sh` for its web distribution.
The native and browser frontends provide their current controls on screen.

- Summon costs 25 gold, randomly chooses a role and fills the first empty tile
  on your board.
- Select a unit, then an empty tile to move it. Select an occupied tile with
  the same role and tier to combine **two** units into the next tier, up to V.
  An incompatible occupied tile is not a merge. This is our rule, not a claim
  about another game's exact merge recipe.
- Upgrade improves your board's attack strength. Invest increases periodic
  income. Both have increasing prices and capped levels.
- Rivet tripods fire rapidly; kiln mortars splash groups; prism towers bypass
  armor; moss condensers slow enemies. Haste beacons accelerate your board's
  firing, while mining drills earn extra gold.
- Enemy families have different movement, armor and health. All escaped enemies
  damage the shared reactor. Projectiles apply damage when they arrive, not
  when their firing animation starts.
- A terminal run shows victory or defeat and can restart from the same seed.
  Replaying identical actions with that seed gives an identical checksum.

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
the seed and the human action interval in ticks. This is an automated gameplay
oracle, not a human playtest or proof that the balance is fun.

Native Linux and browser acceptance are separate. Build success alone does not
prove browser interaction, screenshots, Windows execution or online multiplayer.
There is no claim of networked co-op, save progression or production balance.
