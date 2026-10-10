# glTF CUBICSPLINE animation subset

Issue: #100, tracked under #94. This adds interpolation to the existing
`AnimatedModel::sample_clip` path used by character rendering. It does not add
crossfades, animation graphs, transition history, IK, retargeting or root motion.

## Data and sampling contract

- STEP and LINEAR keep their existing cooked representation and sampling.
- `cubic_spline` channels in the version-1 cooked animation schema store three
  output elements per timestamp: in-tangent, value, out-tangent. Older readers
  reject this unknown enum value rather than silently misreading it.
- glTF `CUBICSPLINE` requires at least two finite, nonnegative, strictly increasing
  timestamps. Output count must be exactly three times input count.
- Translation and scale use component-wise cubic Hermite interpolation; outgoing
  and incoming tangents are multiplied by the segment duration in seconds.
- Rotation uses the same component-wise polynomial and then normalizes the
  quaternion. It does not use LINEAR's shortest-path sign flip. A zero/nonfinite
  interpolated quaternion is an error.
- Before/after the channel range, values clamp to endpoint values, never tangents.
  Absent tracks still start from rest on every sample.
- Every tangent must be finite, including unused first-in and last-out tangents.
  Tangents may be zero or negative, including for scale and rotation. Values
  retain the existing strict translation/unit-quaternion/positive-scale bounds.
- All sampled poses still pass TRS, hierarchy and skin-matrix validation. Cubic
  overshoot producing invalid scale or translation fails closed; validating keys
  alone is not a guarantee that all interior times are renderable.
- Tripled tangent/value entries count against the aggregate 262,144 output-element
  budget. Bytes include all timestamps and all output entries within the existing
  16 MiB animation budget. Import checks unreferenced samplers too.

No simulation state or frame format changes. Existing immutable absolute-tick
sampling remains authoritative for presentation: pause repeats the same sample,
seek has no remembered pose, Stop selects rest, restart samples time zero. No
claim is made that phase transitions now crossfade.

## Verification scope

Focused `orr_model` animation import/runtime tests cover imported/cooked roundtrip,
non-unit-duration tangent scaling, endpoint clamps, quaternion normalization,
CPU skinned deformation/bounds, repeated seeks, malformed counts/times/tangents,
scale overshoot, degenerate quaternion and tangent-inclusive budgets. Existing
STEP/LINEAR tests remain regressions. GPU capture and a cubic-authored project
acceptance remain separate evidence, not implied by these CPU tests.

Reference: [glTF 2.0 Appendix C.5](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#interpolation-cubic).
