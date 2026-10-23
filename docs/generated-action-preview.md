# Independent multi-action authoring preview

The default bootstrap continues to use engine commit
`27e53b356b25aa505995f3e8b322093948ef9db0`. Existing projects are never upgraded.
For explicit source-level experimentation only, `--profile multi-action-preview`
selects commit `6f97ac85a80aabda26679b482d182c9365bff92e`, tree
`cd7cc2a03990a8778a99e60a82c3cfbb425c4d87`. This is reviewed source, **not a stable
or recommended release**. Its hosted native CI is unverified. Bootstrap success
proves neither a Cargo build nor window/input/rendering/source-hidden acceptance.

Use a separate, clean checkout of that exact engine commit; every tracked blob
and executable mode is verified against the bundled complete inventory. An
arbitrary current branch, caller-supplied pin, modified engine, extra Cargo config,
symlink or mismatched inventory is not accepted. Do not overwrite the checkout
used by an existing legacy project. The bootstrap itself never checks out Git,
fetches dependencies, compiles, executes a game or modifies engine source.

From the checkout containing this bootstrap tool, on Linux:

```sh
python3 -B tools/bootstrap_user_game.py \
  --profile multi-action-preview \
  --engine /absolute/separate-pinned-engine \
  --destination /absolute/new-user-project \
  --name my-action-game
```

The destination must not exist and its parent must already exist. Atomic
no-replace publication and the legacy filesystem protections remain in force.
The generated project owns its manifest, Rust sources, scenes and future lockfile.
The manifest resolves dependencies to the supplied exact immutable engine path,
not the bootstrap tool's current checkout. The receipt explicitly identifies the
preview profile, commit, tree, initial file hashes and unverified native status.

The template exposes `static-game-actions` as an **opt-in Cargo feature**, not a
default dependency of the editor. It preserves the weak
`orr_editor?/static-game-actions` dependency so standalone builds do not enable the
editor. It copies the existing portable `tests/multi_action_contract.rs` unchanged.
This private fixture demonstrates four independent held actions and admission /
lifecycle handling; it is not a new playable game. Delivery and Switchboard keep
their original controls. Authors implement `StandaloneGame::action_profile` and
`held_actions_input`, and editor `StaticGameRegistry::with_action_profiles` with
`StaticActionDescriptor`. All mapping values are frozen at admission, not supplied
as arbitrary per-frame runtime callbacks.

After independently generating and reviewing the new project's lockfile, run
these from that new workspace, not from the engine:

```sh
cargo test --offline --locked --no-default-features --features standalone,static-game-actions --test multi_action_contract
cargo test --offline --locked --features static-game-actions --test acceptance
```

These commands are required future checks, not results certified by generation.
The current preview adds no generated-project native evidence harness. Both
multi-action X11 harnesses, their private player and fixture module, and the old
source-fence test/support are omitted because they assume engine-only acceptance
context. Do not substitute an upstream engine receipt for the user project's own
source commit, binary hashes and actual native execution. Existing source-hidden
bundle and mandatory Windows hardware UI requirements are not relaxed.

Core/package boundary: the Rust deterministic simulation, replay wire format and
production controller are unchanged. This is a creator-tool selection layer over
the existing optional action package. Future completion requires a clean generated
project Cargo build, authored action game, and provenance-bound editor/standalone
native runs. The existing legacy generated-user native CI gate remains unchanged.

A separate mandatory `generated-user-game-actions-cpu` CI candidate validates this
explicit preview against its complete pinned engine inventory. It creates an own
user-source commit and lockfile, checks exact Cargo package/source/artifact identity,
and executes portable feature-off, standalone-action and editor-action contracts.
It preserves bounded command logs and partial failure evidence. Its success receipt
explicitly sets native acceptance to false: CPU contracts cannot replace real
window, input, rendering, source-hidden delivery or Windows hardware acceptance.
Local Python fixture tests validate the gate logic; actual generated Rust execution
and hosted CI for this candidate remain unverified until those jobs run.
