# orr_tilemap

Optional, GPU-free single finite background tile-layer authoring data. Depends
only on serde, serde_json and orr_sprite. No simulation, filesystem trust,
package installation, image decoding or rendering is implemented here.

Version 1 JSON is at most 64 KiB. Document and sprite reference records are
strict objects: unknown, duplicate and missing fields are rejected. Dimensions
are 1..=64, cells are row-major and exactly width × height (maximum 4096).
A null cell is empty; region ID 0 is an ordinary occupied cell. Only referenced
static atlas regions must match tile_size. Unused differently-sized regions are
allowed. Sprite clips are not sampled.

Document::from_bytes checks bounded JSON and structure. Document::validate takes
an already validated SpriteDocument and returns a private-inner, immutable
ValidatedDocument. with_cell returns a revalidated candidate, leaving its source
unchanged even on failure. placements iterates occupied cells without allocation.
Consumers remain responsible for installed-package authority, combined actor/tile
budgets, atlas caching, save identity, and renderer capability admission.

origin is the upper-left corner in world coordinates. Rows advance downward,
world Y increases upward, and cell centers are origin + ((column + 0.5) × width,
−(row + 0.5) × height), with dimensions scaled by units_per_pixel. Atlas UVs keep
orr_sprite's top-left convention; no second Y inversion or pivot offset applies.
No tile flips, rotation, animation, colliders, navigation or infinite chunks.

The CPU tests use an asymmetric 3×2 map, literal world/UV expectations, strict
JSON cases, bounds, finite arithmetic, region validation and immutable edits.
