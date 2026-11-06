# Arena sprite camera confinement

The existing `arena --sprite-project PATH` mode now confines its local-player
camera to the simulation's `ARENA_HALF` map rectangle. Build with `sprites` and
use an installed `sample-sprites` project as before. Default shape-only Arena,
authored `--project` playback and Room cameras are unchanged.

Both shape and sprite rendering use `SpriteScene::render_camera(surface_size)`.
It applies the shared 2D confinement API to a copy of the current follow pose,
preserving the target and zoom. The surface size is read for every draw, so
resizing immediately recomputes the visible map boundary. An axis wider than
the whole map is centered; it cannot hide all outside space without changing
zoom. The view operation never changes simulation movement or checksums.

A CPU integration test runs the real rollback-capable Arena bridge to opposite
map corners and compares every snapshot checksum with an unmodified control.
It checks landscape, portrait and oversized viewports, target preservation and
reset. This is automated simulation/presentation coverage, not a recording of
human play or proof of GPU output.
