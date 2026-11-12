
## Explicit multi-action preview (not a stable or recommended release)

This profile uses a different, exact reviewed engine commit recorded in
provenance.json. It does not upgrade an existing project or the legacy profile.
Hosted native CI and actual generated-project action execution are UNVERIFIED.
No creation receipt certifies runtime, hardware, source-hidden or merge acceptance.

The static-game-actions feature is opt-in, never a default. The included
portable tests/multi_action_contract.rs demonstrates a typed four-action mapper
and exercises admission, neutral input, and lifecycle handling in the standalone
controller. It is a test fixture, not a third playable game. From this workspace:

    cargo test --offline --locked --no-default-features --features standalone,static-game-actions --test multi_action_contract
    cargo test --offline --locked --features static-game-actions --test acceptance

These are commands to run, not results already obtained for your project.
Implement your game's StandaloneGame action_profile and held_actions_input,
and its editor registration with StaticGameRegistry::with_action_profiles and
StaticActionDescriptor.
The original Delivery/Switchboard remain legacy one-action samples. Enabling
this feature does not automatically redesign those games' controls.

Repository-only multi-action X11 harnesses and their private player/support
fixtures are omitted along with the legacy source-fence test. Native validation
of a generated user game requires a separately provenance-bound harness; do not
use the engine's native receipts as proof for your modified user code.
