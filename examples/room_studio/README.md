# Studio Room: Key Courier

A small, ready-to-open RoomEscapeV1 sample using the engine's original CC0 Key
Courier and imported-scene assets. The authored visual changes are a correctly
scaled bronze key, a slightly lower/closer camera and one warm point light. Run with the optional studio look:

```sh
cargo run --locked --release -p orr_sample --features room-lighting,room-character --bin room_escape -- --project "$(pwd)/examples/room_studio" --studio
```

WASD/arrows move, E interacts with the key and then the mint exit, and R restarts.
The camera and imported models are presentation only. The scene, collision shapes,
key interaction, deterministic seed and gameplay rules are the original sample.

For a same-fixture baseline, omit `--studio`. For an actual GPU capture append
`--headless --ticks 0 --capture /absolute/new.png`. The studio renderer requires
an HDR-capable GPU; it uses directional shadows, SDR ACES output, and restrained
bloom. The single point light does not cast point-light shadows. No GI or ray
tracing is used. The amber wall strips are emissive materials, not extra lights.

The original Orrery imported-scene fixtures and Orrery Key Courier are CC0-1.0.
All immutable package objects, SHA-256 lock entries and original license notices
are included. No download, authentication, or asset-generation script is needed.
The inert original authoring scripts are retained solely as package provenance.
