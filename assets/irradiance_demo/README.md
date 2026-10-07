# Authored irradiance demonstration

This is a hand-authored 4 × 4 × 4 SH9 diffuse-irradiance grid. It is **not baked
GI**, an environment capture, a reflection probe, or a physically measured light
field. `generate.py` creates a gentle spatial RGB gradient and deliberately
signed directional coefficients in all nine basis terms. Its provenance is
`authored`; importing a copy into the editor marks that copy `imported`.

- File: `authored-room.irradiance.json`, v1, 64 x-fastest nodes
- Origin: `[-4.5, -3.0, -4.5]`; spacing: `[3, 3, 3]`
- Maximum node: `[4.5, 6.0, 4.5]`
- Required compiled/runtime capability: `irradiance-probes`
- Coefficients are linear RGB irradiance, already cosine-convolved
- Positive real basis: `Y00`, `y`, `z`, `x`, `xy`, `yz`, `3z²−1`, `xz`, `x²−y²`,
  with factors `0.2820948`, `0.48860252` (three), `1.0925485` (two),
  `0.31539157`, `1.0925485`, `0.54627424`

Runtime coefficient trilinear interpolation happens before SH evaluation and
one final nonnegative clamp. A one-cell interior fade blends diffuse irradiance
with the normal hemisphere ambient; coverage faces have zero weight and points
outside use the original ambient. Lambertian surfaces consume irradiance as
`E / π` before albedo. Direct sunlight, point light, shadows, and emission are
unaffected.

Install through the normal offline package path. The editor imports through
verified `Project::read_asset`; it never rewrites installed package snapshots.
Make edits in the scene's separate `<scene>.irradiance.json` sidecar, whose save
and undo are independent of the deterministic scene. No bake or reflection UI is
provided. Full signed-SH9 JSON can be imported; the constant-node controls set
`c0 = RGB × intensity / Y00` and zero terms 1–8.

Basis reference: Ramamoorthi and Hanrahan, *An Efficient Representation for
Irradiance Environment Maps*, equation 3:
https://graphics.stanford.edu/papers/envmap/envmap.pdf
