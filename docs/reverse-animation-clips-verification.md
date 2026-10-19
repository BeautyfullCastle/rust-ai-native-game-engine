# Reverse animation clip verification

The reverse clip change is covered by the additive
[`reverse-animation-clips.yml`](../.github/workflows/reverse-animation-clips.yml)
workflow. The existing determinism acceptance remains in place; its original
forward extraction, retiming, installed-package and GPU success criteria are
preserved.

## Required acceptance

- Linux and Windows release CPU tests require an exact test inventory and zero
  ignored cases. Existing model regressions, default-feature checks and strict
  model/renderer-consumer lint remain required.
- The actual feature-disabled executable must exit with status 2. The enabled
  executable must expose help, publish output without replacing existing files,
  preserve its input, and reject invalid arguments without staging leftovers.
- Omitted direction and explicit `--direction forward` must produce identical
  bytes. Explicit reverse must produce a distinct, importable animation.
- Linux software Vulkan runs the reverse GPU test explicitly with `--ignored
  --exact`, requiring exactly one passed and zero ignored cases. Missing devices,
  captures, or the actual CLI executable fail acceptance.
- Cargo compiler-artifact records identify each retained executable by package,
  manifest, source target, features and release profile. Hashes fence those
  executables throughout acceptance. The GPU test receives that exact CLI path.
- The GPU path authors a GLB through the CLI, installs it, hides source files,
  cooks and reloads installed content, and checks sampled readbacks against an
  independent source-time/analytic oracle. Wrong-time controls must visibly
  differ. Original forward GPU capture requirements remain unchanged.

## Evidence and reproducibility

Each workflow run retains command logs, subprocess receipts, executable
provenance, source fingerprints and GPU capture hashes, including partial failure
evidence. Source and lockfile fingerprints must remain unchanged during the
acceptance run. A failed run is not replaced by a later success; both remain
available in their respective workflow runs.

For local execution, use the commands and environment documented in the
workflow. Resource-constrained local debug checks are useful development
evidence, but do not replace the required release CI jobs. A green release CI
result must be checked on the exact reviewed commit before merge.

## Local debug result — 2026-10-19

All 42 focused CPU tests passed with zero ignored cases. Strict model and
renderer-consumer lint, default checks, feature-disabled refusal, and real CLI
publication/safety checks passed. The reverse software-GPU case passed with all
233 required captures; the original forward case passed with all 133 original
captures and criteria intact. The frozen 1,736-file source/configuration cohort
and dependency lockfile remained unchanged.

The full model regression is **not locally complete**: one unchanged Unix socket
fixture could not bind under the execution environment's permissions. The
supported elevated retry could not start, so it produced no test result. No test
was skipped or changed to evade this restriction. The complete release regression
remains required on CI before merge; these focused debug results do not replace it.

Earlier failures remain retained. A first attempt stopped at the four-GiB free
space guard before compilation. A later failed attempt used a local one-GiB
reserve and found a new test's incorrect exact raw-quaternion expectation.
Runtime endpoint sampling normalizes rotations; the corrected test now checks
exact sampled endpoint/accessor bits while retaining original raw keys and the
unchanged independent analytic oracle. After cache cleanup, the final successful
focused and GPU checks restored the four-GiB reserve. Only the debug executable
size ceiling differed by build profile; functional asset budgets and release CI
resource limits were unchanged.

Evidence is software Vulkan and offscreen rendering, not physical-GPU performance
or native-window interaction. Draft publication enables required remote checks;
full-regression success and merge readiness are not claimed here.
