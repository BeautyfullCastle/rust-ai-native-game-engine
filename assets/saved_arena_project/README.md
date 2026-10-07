# Saved Arena editor project

A self-contained two-entity Arena scene with independent, GUID-keyed sprite
bindings and saved camera-follow identity. The second actor intentionally uses
`walk` while stationary and `idle` while moving, making accidental shared
animation state visible.

Open this directory, or a complete copy of it, without an install step:

```sh
cargo run -p orr_editor --features sprites -- --project assets/saved_arena_project
```

A built `orr_editor` binary can open an absolute project path from any working
directory. Include the hidden `.orr` directory when copying this fixture.
`orr.packages.lock.json` and `.orr/packages/objects` contain the installed
`sample-sprites` package; its `LICENSE.txt` covers the original lantern-keeper
art. The source asset directory and its generator are not runtime dependencies.

The installed bytes were generated with the existing package API through:

```sh
cargo run -p orr_package --bin orr_pkg -- install assets/saved_arena_project --path assets/sprite_demo
```

Do not manually modify the immutable object or its lock hashes. To edit the
sample, copy the whole project, edit in the editor, save the scene, and separately
save the sprite bindings. See [saved projects](../../docs/editor-saved-projects.md)
for the exact admission and persistence boundary.
