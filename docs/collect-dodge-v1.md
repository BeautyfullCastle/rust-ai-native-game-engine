# CollectDodgeV1: bounded gameplay foundation

This default-off `orr_games/collect-dodge` module implements a real deterministic
`Game`, not an Arena tag or presentation-side score. It adds no dependency, core
API, Arena component, network protocol, package schema, asset or filesystem call.
The core itself has no project loader. The optional [authored adapter](collect-dodge-authored.md) adds a separate validated project route.

## Rules and limits

- One controlled player, 1–32 collectibles and 0–16 hazards, in a closed square
  from -256 to +256. Every actor is an axis-aligned square with half extent 2;
  admitted centers are in [-254,254]. Contacts include the boundary.
- Input slot 0 controls each axis, clamped to [-1,1] units/tick. Diagonal movement
  is intentionally per-axis, not normalized. Missing/disconnected input is neutral;
  other slots, reserved input bytes and unknown button bits have no effects.
- Hazards move by validated [-1,1] velocities per axis and reflect overshoot at
  the arena boundary. Contacts use post-movement discrete AABBs, not swept CCD.
- Each tick: move player/hazards; hazard contact loses immediately; otherwise
  collect touching active collectibles in level-ordinal order; collecting all
  wins; otherwise reaching the 1–36000-tick deadline loses. Thus hazard takes
  precedence over collection and timeout; winning on the deadline is allowed.
- Each collectible scores exactly one, becomes inactive, and never despawns.
  Terminal states freeze actors, score and elapsed gameplay time, and emit no
  repeated finish events. Engine ticks and restart-edge state still advance.
- A rising RESTART bit resets every actor's initial position/velocity/active flag
  and run progress. That tick does not move or collect. Holding restart does not
  restart repeatedly. Release/repress works during playing and terminal states.
- There is no implicit RNG, wall clock, floating point, disk lookup, asset reread
  or external mutable state. Config is validated before construction and private
  thereafter. Initial and current actor state, goal, limit, score, phase, elapsed
  ticks and restart latch are all part of the rollback Frame.

## Admission boundary

`CollectLevel::new` is the core level constructor. The optional authored adapter admits initial-only scene data through the same bounds.
Calling `Simulation::from_frame` on bytes previously emitted from a valid run is
supported and tested, including a fresh registry and replay after reset. Raw
engine `frame_mut`, edited frame bytes or arbitrary scene baking are not game
admission APIs. Future project/editor adapters must validate actor cardinality,
kind/ordinal uniqueness, bounds, motion, canonical reserved fields and run-state
invariants before admitting such data. Engine layout decoding alone cannot do it.

The core introduces no persisted high score, stable project/save identity, arbitrary
script loading, authored renderer, UI win/loss panel, project generator template
or exported runtime selection. The authored adapter separately defines a
versioned game profile/build identity and initial-scene admission.
Those are dependent slices, not implied by this game module. Full #94 acceptance
still requires the actual authored 2D game flow and persistence/export proof.

## Focused verification

- `cargo test --release -p orr_games --features collect-dodge --test collect_dodge`
- `cargo run --release -p orr_games --example collect_dodge --features collect-dodge`
- `cargo test --release -p orr_games` preserves existing sample-game goldens
- `cargo clippy --release -p orr_games --all-targets --features collect-dodge -- -D warnings`

The example exercises actual Simulation ticks: collect both targets, win, restart,
walk into a hazard and lose. It is a headless oracle, not a graphical host. Tests
cover cold serialized-Frame replay, snapshot restore, every-tick checksums/events,
restart latching, terminal freeze, simultaneous precedence, exact contacts, motion
bounds and malformed input/config. Required feature CI and development union
verification remain separate from focused local checks.
