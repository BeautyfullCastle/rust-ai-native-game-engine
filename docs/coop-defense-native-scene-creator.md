# Native Stormglass scene creator

`orr_pkg create-coop-native-scene` creates a new Scene YAML file without Node,
Python, a WASM file, network access, or a source checkout at runtime. It uses the
same bounded option parsing and exact YAML writer as `create-coop-scene`, then
admits those bytes through `orr_games::coop_defense_scene::parse_config`, the
strict reflected parser/bake used before the native player opens a window.

## Build and use

```sh
cargo build --locked --release -p orr_package --features coop-native-authoring --bin orr_pkg
cargo build --locked --release -p orr_sample --features coop-defense --bin coop_defense

target/release/orr_pkg create-coop-native-scene --output /existing/directory/new.scene.yaml \
  --starting-gold 250 --reactor-hp 75 --last-wave 3 \
  --spawn-interval-ticks 70 --wave-pairs 2,1,3,4,5,6,7,8,9,10 \
  --enemy-health-multiplier 3 \
  --wave-kinds '3,2,1,0;0,1,2,3;1,2,3,0;2,3,0,1;3,0,1,2;0,0,0,0;1,1,1,1;2,2,2,2;3,3,3,3;0,1,2,3' \
  --opening-bosses 1,0,0,0,1,0,0,0,0,1

target/release/coop_defense --scene /existing/directory/new.scene.yaml
```

Copy the two executables and scene to another directory to deliver the game.
Use `--headless --ticks 420` instead of opening a window for a CPU-only replay.
This authors the six existing singletons: Scenario, Campaign, WavePacing,
WaveCounts, EnemyHealth, and WaveComposition. It does not add maps, scripts,
EnemyMotion, or new gameplay rules. Restart retains all admitted settings.

Starting gold is 0..10000 (default 150), reactor HP 1..1000 (30), last wave 1..10
(10), spawn interval 35..350 (35), and enemy-health multiplier 1..4 (1). Pair
counts require ten integers in 1..28. Composition requires ten rows of four
family codes 0..3, and ten boss flags in 0..1. A boss replaces the first pair;
remaining regular kinds keep their ordinal. Supplying just one composition
option writes both, using the historical default for the other. Omitted optional
singletons remain omitted, preserving the existing web creator's bytes.

## Contract and boundaries

The default-off Linux `coop-native-authoring` feature reuses `coop-web-authoring`
helpers and adds optional `orr_games/coop-defense-scene`. It includes web helper
dependencies at build time but the native command never executes them. Default
and web-only builds reject the new command before opening files. The existing
web command still requires `--wasm` and successful execution in trusted Node;
the native command rejects `--wasm`.

Successful stdout is a distinct JSON receipt with kind
`orr.coop-native-scene/1`, exact scene byte count/SHA-256, native game build ID,
and the fully admitted settings including defaults. It certifies native source
admission, not WASM execution, playability, victory, or a rendered window. It
contains no fabricated WASM hash or initial checksum.

The output parent must already exist with stable trusted ancestry. Invalid or
duplicate options, out-of-range values, traversal, existing outputs, and symlink
destinations/ancestors are refused. Publication uses the existing descriptor-
relative no-replace publisher after admission. Exit 2 means usage/feature error,
1 means admission/publication failure, and 0 means published. A post-publication
warning can report uncertain durability or sibling cleanup: inspect it, do not
retry or delete the published scene. This is not a hostile-filesystem sandbox.

## Reproducible acceptance

Run package native tests and explicit feature-off CLI checks, followed by strict
Clippy for both feature configurations. The default test run does not execute
the two external-binary acceptance tests; run them explicitly:

```sh
ORR_NATIVE_SCENE_WASM=/absolute/exact-source/orr_web.wasm \
  cargo test --locked --release -p orr_package --features coop-native-authoring \
  --test native_scene_cli existing_web_command_executes_real_wasm_and_matches_native_yaml \
  -- --ignored --exact

ORR_NATIVE_SCENE_CREATOR_BIN=/absolute/exact-source/orr_pkg \
ORR_NATIVE_SCENE_PLAYER_BIN=/absolute/exact-source/coop_defense \
  cargo test --locked --release -p orr_sample --features coop-defense --lib \
  defense_app::native_scene_tests::relocated_native_creator_scene_reaches_player_session_and_restart \
  -- --ignored --exact
```

The tests require their inputs, never count missing binaries/Node as success,
and cover empty-PATH copied execution, identical web/native bytes, real WASM
rejection controls, relocated player checksum, authored spawn counts/kinds/HP/
pacing, and native Session Restart. Run the repository Linux aggregate and
independent review against the final candidate before integration. CPU evidence
does not certify physical input, native windows, GPUs, browsers, or other OSes.
