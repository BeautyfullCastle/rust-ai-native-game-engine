# Package-backed sprite Arena sample

This is an optional presentation slice of the existing deterministic Arena. It
uses the same keyboard mapping, bridge, prediction, rollback smoothing and bot.
The local player is drawn with the original lantern-keeper atlas, switching
between `idle` and `walk` from actual presented movement. The camera follows the
same generation-aware entity and smoothed position. Other players, bullets and
the Arena floor remain shape-rendered. Space still fires; this is not a new
platformer simulation.

From the repository root:

```sh
cargo run -p orr_package --bin orr_pkg -- install examples/sprite_arena --path assets/sprite_demo
cargo run -p orr_sample --features sprites --bin arena -- --sprite-project examples/sprite_arena --audio off
```

WASD/arrows move, space fires, Escape exits. Existing `--bridge`, latency,
remote interpolation, relay and `--seconds` options still apply. For example:

```sh
cargo run -p orr_sample --features sprites --bin arena -- --sprite-project examples/sprite_arena --bridge inproc --seconds 10 --audio off
cargo test -p orr_sample --features sprites --lib sprite_scene::tests
```

`sprites` enables `orr_sprite`, `orr_package` and `orr_render/sprites` only for
this optional sample. Without the feature, `--sprite-project` gives an explicit
error; omitting the option preserves the usual shape Arena. Headless bot mode
rejects the presentation option.

The install copies the declared original assets into the project's verified
local package store and writes its lock. Runtime reads only from that store,
including the atlas path from the sprite document, and checks hashes and actual
compiled capabilities. Missing, removed, incompatible or tampered packages fail
with an error; there is no embedded-image or repository-path fallback. The PNG
must be a static RGBA8 image (APNG is rejected) and match the document; decoding is bounded to 2048 pixels per
axis and 16 MiB. This sample does not hot-reload installed assets while running.

The package's LICENSE.txt covers the original artwork. No external art was
copied. `.orr` and the generated lock are local install products, ignored here.

This is a runnable-consumer candidate, not completion of editor issue #98.
Editor-persistent sprite bindings keyed by stable authored scene identity,
clip selection UI, preview and undo/save integration remain separate work.
The local Arena entity binding is runtime-only and is reset on bridge resync.
