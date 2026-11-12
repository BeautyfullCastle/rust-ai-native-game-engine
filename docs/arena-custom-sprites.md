# Arena starter with custom actor art

`orr_new_arena` can install an existing sprite package when creating a **new**
Arena project on Linux. Only the camera-follow hero receives the chosen art.
The second player keeps the bundled art; scene bytes, two-player simulation,
spawn positions, seed 42, 60 Hz, camera-follow target and shared preferences stay
unchanged. This is an editable Arena starter, not a new gameplay system.

## Create

Build the existing `project-create` feature, then run:

```sh
cargo build --release -p orr_sample --features project-create --bin orr_new_arena
./target/release/orr_new_arena \
  --output /absolute/new-project \
  --template arena-custom-sprites-2d-v1 --seed my-arena \
  --sprite-package /absolute/my-sprites \
  --idle-clip idle --walk-clip walk --units-per-pixel 2
```

Both clip IDs are required and must exist in the same sprite document. They may
be equal, for example when a package intentionally supplies only one clip.
Animation uses the existing tick-driven locomotion consumer, including its
existing loop/once behavior. Scale must be finite and between 0.0001 and 100
world units per pixel, inclusive. This changes presentation, not collision.

The package directory must contain exactly four regular, nonsymlink files:

- `orr.package.json`: valid existing package manifest, capability `sprite` only,
  no dependencies, a package name other than `sample-sprites`.
- `sprites.json`: valid existing sprite document with the selected clips.
- One portable PNG atlas leaf named by that document.
- `LICENSE.txt`: nonempty UTF-8, at most 64 KiB, with your asset license/provenance.

The manifest must declare precisely the latter three files. Each source file is
limited to 1 MiB. Source bytes and the complete resulting starter are each
limited to 2 MiB; the bundled actor package also counts toward the output limit.
Existing PNG import/package tools can produce this four-file layout. Packages
with extra raw atlas files or dependency closures are not accepted by this
bounded starter. No download, asset generation or package code is executed.

## Use and verify

Open with a sprites-enabled `orr_editor --project /absolute/new-project`, or run
with a project-enabled `arena --project /absolute/new-project`. Creation uses the
existing package installer, full lock verification, strict PNG decoder and
`PreparedRuntime` admission before no-replace publication. The source directory
is unchanged and is not needed afterward. Installed sprite, PNG and license
payloads are byte-preserved; only the package manifest is canonicalized. Original
sample MIT notices remain with the scene and target package.

Focused tests cover real CLI creation, repeated output equality, hero-only
bindings, source-hidden runtime consumption, malformed packages/clips/scales,
concurrent source changes, stage tampering, publication failure and destination
collisions. Run the library `project_create` tests and integration test
`custom_sprite_delivery` with `project-create`; run existing
`custom_sprite_delivery` with `project-create,project-export` also exercises the
actual exporter and relocated headless player. Existing `custom_tileset_delivery`
with `project-create,project-export,tilemap` covers CLI regression. New actor tests
are CPU/CLI checks, not native-window or GPU acceptance.

Only trusted directories not being modified concurrently are supported; ordinary
edit detection is not a hostile-filesystem sandbox or power-loss guarantee.
Existing projects are never attached to or overwritten. Custom actor and custom
tileset creation are separate templates, not composable CLI options. Creation
does not certify redistribution rights. Use only assets you can distribute.
