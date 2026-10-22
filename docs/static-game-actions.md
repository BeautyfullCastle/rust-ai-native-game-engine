# Independent actions for statically registered games

This opt-in path adds up to four independent held actions to a statically linked
custom game. It reuses `orr_input` and the existing typed input, managed-input
claims, simulation and replay paths. It does not add a VM, dynamically loaded
code, or per-frame game mapping callbacks.

## Enable explicitly

- Editor crate: enable `orr_editor/static-game-actions` (also enables
  `static-games` and its existing optional `orr_input` dependency).
- Starter editor application: enable `editor,static-game-actions`.
- Starter standalone application: use `--no-default-features` with
  `standalone,static-game-actions`. The starter's default includes `editor`;
  omitting default features matters for an editor-free standalone graph.

`static-game-actions` is off by default. Enabling it does not opt existing games
in automatically: `StandaloneGame::action_profile()` defaults to `None`.
`StaticGameDescriptor`, `StaticInputDescriptor`, `StaticGameRegistry::new` and
Delivery/Switchboard's legacy 18-state mapping remain unchanged.

## Define one ordered profile and typed mapper

The following functions are application code, not a complete game. Use the input
as your existing game's `Game::Input`, register it under `Starter::INPUT`, and
retain the game's existing scene, simulation and presentation implementations.

```rust
use orr_input::{Action, Button, HeldActionProfile, HeldActions, Key};

fn controls() -> Result<HeldActionProfile, String> {
    HeldActionProfile::new(
        [("jump", Key::Space), ("fire", Key::F),
         ("interact", Key::E), ("dash", Key::Q)]
            .into_iter()
            .map(|(name, key)| Action {
                name: name.into(),
                bindings: vec![Button::Keyboard { key }],
            })
            .collect(),
    )
}

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable,
         orr_reflect::Reflect)]
struct MyInput {
    x: i32,
    y: i32,
    jump: u32,
    fire: u32,
    interact: u32,
    dash: u32,
}

fn map_actions(held: HeldActions) -> MyInput {
    MyInput {
        x: i32::from(held.x),
        y: i32::from(held.y),
        jump: u32::from(held.actions & 1 != 0),
        fire: u32::from(held.actions & 2 != 0),
        interact: u32::from(held.actions & 4 != 0),
        dash: u32::from(held.actions & 8 != 0),
    }
}
```

Declaration order is bit identity: jump is bit 0, fire bit 1, interact bit 2 and
dash bit 3. Keep names, order and bindings stable for the compiled application.
The simulation decides what held versus rising-edge behavior means.

In the existing `impl StandaloneGame for MyGame`, add these feature-gated methods:

```rust
#[cfg(feature = "static-game-actions")]
fn action_profile() -> Option<HeldActionProfile> {
    Some(controls().expect("compiled controls must be valid"))
}

#[cfg(feature = "static-game-actions")]
fn held_actions_input(held: HeldActions) -> Self::Input {
    map_actions(held)
}
```

Keep implementing the existing required `held_input`, `input_value`, `extract`
and `state_value` methods. `input_value` must serialize all six fields exactly;
`Starter::valid_input` must accept only axes in `-1..=1` and action fields in
`0..=1`. Do not clamp invalid samples to hide errors. Profiles and mapping
functions should be pure and stable across editor and standalone startup.

## Register the same mapping in the editor

For an application based on the starter, the existing shared helper is:

```rust
pub fn action_descriptor<G: StandaloneGame>()
    -> Option<orr_editor::static_adapter::StaticActionDescriptor>;
```

It is available only with `editor,static-game-actions`. Given your existing
`Vec<StaticGameDescriptor>` named `descriptors`, opt in your game as follows:

```rust
let profiles = orr_static_starter::integration::action_descriptor::<MyGame>()
    .into_iter()
    .collect();
let registry = orr_editor::static_adapter::StaticGameRegistry::with_action_profiles(
    descriptors,
    profiles,
)?;
```

The companion's `game_id` must match `MyGame::ID` in `descriptors`. Keep the
registry for the application's startup lifetime, for example in a `OnceLock`,
before obtaining `RegisteredGame` handles. The helper does not construct your
host, stream, position or draw descriptor for you. It shares the starter's
sample validator with standalone admission; errors emerge from
`with_action_profiles`, not from the helper returning `Option`.

Applications not using the starter can provide the companion directly:

```rust
pub struct StaticActionDescriptor {
    pub game_id: &'static str,
    pub profile: HeldActionProfile,
    pub held: fn(HeldActions, &orr_reflect::TypeRegistry)
        -> Result<serde_json::Value, String>,
}
```

The callback receives the exact already-admitted input registry and runs once
per finite state at registration, never during Play. It must return the game's
typed input JSON or an admission error. The editor validates reflected decoding;
application-specific semantic checks belong in this callback. Prefer the starter
helper when sharing its typed contract.

## Standalone and admission limits

`Controller::<G>::new(&admitted, replay_bytes)` discovers the profile, admits the
full typed table once, and uses frozen lookups thereafter. It needs no editor
crate at runtime. `FrozenActionInputs::<G>::admit(profile)` exposes the same
standalone admission and rejects invalid lookup axes or masks through `get`.

- Zero to four actions; `9 * 2^N` states, at most 144.
- Each action has 1–8 explicit keyboard bindings. Names are unique, 1–64 ASCII
  letters, digits or underscores. Duplicate physical bindings are rejected.
- WASD/arrows are reserved movement controls. Escape, Enter and Tab cannot be
  bound to actions. Other keys must be supported `orr_input::Key` identities;
  this profile does not admit mouse or gamepad bindings.
- Neutral is exactly `HeldActions { x: 0, y: 0, actions: 0 }` and must decode to
  the typed default, not just a similar JSON representation. The starter also
  checks `G::valid_input` and exact typed-byte/JSON round-trip equality.
- Each encoded input is at most 64 KiB. Editor admission caps aggregate frozen
  JSON at 4 MiB across the registry's schemas, identity metadata, legacy and
  action tables. Standalone caps its action table plus schema/profile/identity
  at 4 MiB. These aggregate scopes differ intentionally.
- Editor registries allow 1–32 games; duplicate or unknown companion IDs fail
  before descriptor callbacks. Invalid axes or undeclared action bits fail
  rather than being masked or clamped.

## Controls and release behavior

WASD/arrows produce signed axes; opposing directions cancel. Actions can be held
simultaneously with movement. In Editor Play, explicitly take control with the
viewport focused. Gameplay presses require physical key identity; Ctrl/Alt/Command-modified
shortcuts and unfocused input are not consumed by this router. The first active
frame is neutral, so press fresh keys after control becomes active.

Releasing a key clears it. Focus loss, pause/release, scene changes, failed or
expired input claims and reacquisition discard held actions. Repeated key events
do not resurrect them. Editor Escape/Tab release control; existing logical
Escape handling also fails closed. Standalone Escape pauses, Enter resumes,
and focus loss pauses. Resume includes a neutral tick before fresh presses can
arm input. Standalone Tab is reserved but is not a resume/pause command.

## Verification and generated projects

Focused CPU coverage exercises admission, independent/simultaneous actions,
managed-input envelopes, release/reset paths and standalone deterministic
simulation/replay behavior. CPU and synthetic-event tests are not evidence that
native window input, focus or rendering works. Native acceptance for this new
profile path is a separate gate and remains pending until its own receipts pass.

The existing bootstrap still pins engine revision `27e53`. This document and the
new opt-in feature do not repin it or automatically upgrade generated projects.
Use a deliberately selected revision containing this implementation before
expecting these controls in a generated application.
