# CoopDefense web build asset handoff

`bash tools/build_coop_defense_web.sh /absolute/generated/output` builds the
optional game's WASM, copies its web files, then invokes the existing strict
offline packer. Rust's `wasm32-unknown-unknown` target, Python and the normal
project build dependencies are required.

The copy step now derives its required files from
`pack_coop_defense_offline.ASSETS`, excluding `game.wasm` (supplied by Cargo).
Nested module directories are created. This prevents the official clean build
from omitting `run-save.mjs`, `audio/authored-asset-audio.mjs` and
`audio/coop-defense-audio.mjs`, which the packer already requires. The complete
repository `web/assets` directory is also preserved for optional served art,
metadata and licensing files. Missing required source files are reported before
the copy helper creates or changes generated files.

This is a build-directory copy, **not an atomic release publisher**. Existing
generated files are replaced as before; unrelated output files are not deleted.
Use a clean generated directory to establish a self-contained build. Repository
and output directories are trusted and quiescent; this helper is not a sandbox
or concurrent-writer protection mechanism. The existing packer's admission and
output rules are unchanged.

Acceptance separates three types of evidence: the original copy-set failure
(missing required modules, before WASM execution), focused copy/packaging fixtures
(which may use non-executable placeholder WASM), and an actual clean official
Cargo→copy→packer build with real WASM. Successful packaging is not browser,
GPU, hardware audio or gameplay certification. The optional project-recipe
feature is a separate follow-up, not part of this correction.
