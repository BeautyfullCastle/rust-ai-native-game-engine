# User-facing authoring CLI migration map

These are **13 Linux Rust replacement paths for 14 inventoried Python tools**, not
an engine-completeness percentage. All listed `orr_package` authoring features are
optional and default-off. Build the required feature explicitly:

```sh
cargo build --locked --release -p orr_package --bin orr_pkg --features FEATURE
```

| Retained legacy tool (`tools/`) | Rust replacement (`orr_pkg`) | Feature / guide |
| --- | --- | --- |
| `create_assembled_model_package.py` | `create-assembled` | `model-authoring`; [static packages](rust-assembled-model-package.md) |
| `create_animated_sequence_package.py` | `create-animated-sequence` | `animation-authoring`; [animated packages](rust-animated-sequence-package.md) |
| `create_sprite_package.py` | `create-sprite` | `sprite-authoring`; [sprites](rust-sprite-package.md) |
| `create_coop_defense_audio_bank.py` | `create-coop-audio-bank` | `audio-authoring`; [soundbanks](rust-coop-audio-bank.md) |
| `create_coop_defense_audio_bank_from_wav.py` | `create-coop-audio-bank-from-wav` | `audio-authoring`; [soundbanks](rust-coop-audio-bank.md) |
| `deployment_manifest.py` | `deployment create/validate/export/id` | `deployment-authoring`; [deployment records](rust-deployment-manifest.md) |
| `bootstrap_user_game.py` | `bootstrap-user-game` | `bootstrap-authoring`; [fixed-source bootstrap](rust-user-game-bootstrap.md) |
| `create_coop_defense_animation.py` | `create-coop-animation` | `coop-web-authoring`; [animation](rust-coop-web-animation.md) |
| `compose_coop_defense_animations.py` | `compose-coop-animations` | `coop-web-authoring`; [animation](rust-coop-web-animation.md) |
| `create_coop_defense_scene.py` | `create-coop-scene` | `coop-web-authoring`; [scenes](rust-coop-web-scene.md) |
| `pack_coop_defense_offline.py` | `pack-coop-offline` | `coop-web-authoring`; [offline HTML](rust-coop-offline.md) |
| `build_coop_defense_recipe.py` | `build-coop-recipe` | `coop-web-authoring`; [workflow](rust-coop-workflow.md) |
| `copy_coop_defense_web_assets.py` | `copy-coop-web-assets` | `coop-web-authoring`; [workflow](rust-coop-workflow.md) |
| `subset_game_ui_font.py` | No Rust replacement yet | Developer font-subsetting Python tool remains |

## Dependency and evidence boundaries

- `tools/build_coop_defense_web.sh` now invokes Cargo and native Rust copy/HTML
  commands, not Python. Use a fresh output directory: existing default offline
  HTML is refused before Cargo or writes. Rust/Cargo and the WASM target remain
  build dependencies; this is not a standalone prebuilt SDK claim.
- Node is retained for actual authored scene/WASM and animation/JavaScript gates.
  Offline packaging with those optional inputs must pass the applicable gates;
  they are not replaced by structural checks. Plain asset copying does not run Node.
- `tools/check_coop_defense_web.sh`, legacy compatibility tools and differential
  tests still use Python. Removing Python from the official **build** is not
  removing it from the official **verification** workflow or repository.
- Font subsetting remains a separate Python developer step. Existing assets and
  legacy tools are preserved; no automatic migration/deletion is performed.
- Follow each guide's bounds, no-follow and output-publication contract. Linux
  focused execution does not certify Windows, browser gameplay, GPU rendering or
  physical audio playback. The original 22 follow-up issues and #94 authoring
  scope remain separate from this migration inventory.
