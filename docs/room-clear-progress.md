# Authored Room clear records

`room-progress` is an optional, default-off Linux standalone feature. It records
whether one explicitly identified authored Room Escape challenge has been cleared;
it is not a save game, count, score, cloud service, or renderer capability.

## Authoring and runtime

Keep the existing Room scene, model binding and optional camera/light/character
sidecars. Change the project manifest to schema 3 and supply:

```json
"progress": {
  "schema": 1,
  "game_id": "12345678-1234-4234-8234-123456789abc",
  "profile": "room-escape-clear-v1"
}
```

Use your own canonical UUIDv4 for a distinct game/fork; the example is illustrative.
No identity is generated or silently repaired. Schema 2 without progress continues
to work. A schema-3 Room manifest cannot use the Collect high-score profile, nor can
a Collect game use the Room profile. Consumers must explicitly admit this metadata;
Cargo feature unification alone does not authorize unrelated editor routes.

Build the standalone player with `room-project,room-progress` and any presentation
features required by the authored project. Launch the ordinary windowed project
route. Headless captures, export validation and export smoke tests never load or
write clear records. Export uses the same strict project admission and requires a
compatible supplied player; it never bundles local progress.

## Record contract

On the first successful authoritative `won: 0 → 1` step, the normal native app
attempts a single monotonic clear commit before presentation work. Repeated terminal
ticks, drawing and restarting cannot submit again in that process. A new process
loads a prior clear marker but starts a fresh simulation without a key or win state.
Failures are displayed without aborting gameplay; uncertain durability is reported
as uncertain, with no automatic retry during the current session.

The store lives under `$XDG_DATA_HOME/orrery/games/<game_id>/room-escape-clear-v1/`
(or `$HOME/.local/share/…`), followed by a versioned gameplay digest. It must be
outside the project and executable/export bundle. A read of a missing record creates
nothing. Room uses `completion.json`, its backup and a separate lock; Collect's
high-score bytes, namespace and filenames stay unchanged. Concurrent valid clear
records merge with logical OR under the existing lock/identity/fsync protocol.
Malformed, mismatched and unsafe filesystem objects fail closed rather than being
replaced with default state.

The digest includes admitted initial gameplay geometry, roles, physics configuration,
seed, tick rate and rules revision. Presentation and file/GUID spelling are not a
new challenge. Gameplay changes produce a separate record; changing only a model,
material, camera or path does not. Only the documented closed Room rules are covered.

## Acceptance and limits

Focused tests exercise actual authoritative App input steps, restart/reopen,
read-only headless/export routes, profile isolation, compatibility, concurrency,
backup/failure boundaries and semantic identity. App tests do not claim real OS
keyboard input or a rendered window; existing model/presentation proof is separate.
This feature does not restore position, inventory, animation, frame checksums or
replay state. Record contents are local user data, not anti-cheat evidence.
