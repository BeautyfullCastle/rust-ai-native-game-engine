# Blockout recipe-space UV density verification — 2026-10-19

Base: `db36175cc6f16f1375737a093040004b351d0a69`. Local Linux verification ran on 2026-10-18 UTC. Independent source review approved the implementation; root inspected the scoped wrapping/default-width delta and before/after checker images. Final workflow review approved SHA prefix `7bf04c26` and validated the actual receipt predicates.

## Final results

- **22 CPU tests passed:** five new caller/state tests, five new public math integration tests and twelve existing public blockout regressions. Feature: `models`; targets/filter: `--lib blockout_uv_density`, `--test blockout_uv_density`, `--test blockout`.
- Default library and minimal `--no-default-features --features models` checks passed. Strict library, math integration and UI integration Clippy passed with `-D warnings`.
- Actual production EditorApp UI **passed 1/1 in 26.10 s**, with eleven captures: `/workspace/shared/uv-density-evidence-20261019/ui-attempt-02`.

The final CPU, boundaries and strict gates ran against the final scoped helper-wrapping source. Logs: `/tmp/uv-density-v4-{state,math-blockout,default,minimal,ui-build,clippy,final-stability}.log`; actual run `/tmp/uv-density-v2-ui.log`. The evidence directory contains `acceptance-receipt.json`, `capture-inventory.json`, screenshots, `run.log` and source/hash proof.

The receipt records recipe-space face edges 3×4, two model units per repeat and a quarter-turn of one producing UV scales `[1.5, 2]`. Nonuniform binding scale `[1, 1.5, 0.75]` is deliberately excluded from this recipe-space calculation. The selected +X checker view changed 343,114 pixels, while the other-face view remained identical. CPU geometry checks cover all six faces and all four quarter-turns against baked physical edge lengths, resize recalculation, invalid/overflow/no-clamp refusal and unchanged source/history/redo.

Calculate and Stage change only the selected draft scale. Existing Apply changes recipe preview/history; the main viewport changes only after existing Save/Bake/Load/Assign. The actual workflow covers those boundaries, resize, Undo/Redo, save/reopen and retained immutable baked revisions.

## Layout scope

The functional workflow uses the existing wide 560-pixel Inspector at 1800×1500 editor size, with a fixed 1010×1383 Yard viewport. A separate helper-focused capture (`04b`) uses the default 340-pixel Inspector with existing Box/component controls collapsed and asserts density units/actions/status remain inside it. Text wrapping was added only inside the new helper scope. This verifies the new helper's narrow layout; it does not claim to repair or comprehensively validate existing reserved blockout/model-panel layout.

## Preserved failures

- First UI-target strict Clippy failed on an unused `Face` import; the test-only import was removed without lint suppression. Log: `/tmp/uv-density-v2-clippy.log`.
- First actual UI failed an exact viewport-size fixture assertion: expected1010×1362, actual1010×1383. The borrowed Room expectation included an extra21-pixel toolbar absent from Yard. The corrected test retains exact viewport bounds. Original `/tmp/uv-density-v1-ui.log` and `ui-attempt-01` evidence are preserved.
- Root requested the separate default-width layout proof before publication. After the scoped production wrapping addition, all affected22 CPU tests and feature boundaries were rerun, alongside strict Clippy and the successful fresh UI execution.

Disk pressure paused additional links while root cleaned inactive regenerable build cache; source, logs and acceptance evidence were preserved.

## Frozen source

Manifest `/tmp/uv-density-v4-source.sha256` SHA256: `244eef833f52dc837f7d67dd10134b028352fce03f7f02e38bf9eec084296ee8`. All seven entries remained unchanged through final UI and follow-on regression checks:

```text
9afb4d05544c54029cc64bed49f0587dc7682a5d415eb2ece5cdc8751077407b  crates/orr_editor/src/app.rs
98010d4aee9b18fa9082c4f5d7c7450cec9b9bea43ea763ca97f567cd3007cdf  crates/orr_editor/src/app/blockout_uv_density.rs
20848e22e16496593a47432ae9cf764401352223a3f004eae99357b48af84cac  crates/orr_editor/src/blockout_uv_density.rs
bd607e444b81611567c9c8d4071d4b4fa36fa830198ddace713c2c92c6fbf711  crates/orr_editor/src/lib.rs
0e47e75737b739ee32daafdcd6afade7fa47672af51bff1649d0e8479cb283df  crates/orr_editor/tests/blockout_uv_density.rs
57115821a87ee3f20ffe62103ea2d6e9a8da7be8212787cf7989b5e8c2fd933a  crates/orr_editor/tests/blockout_uv_density_ui.rs
a60f5aeae15cdccc2f808abd73435979f76f1af7c6d9db0842972bbec6f0b743  docs/blockout-uv-density.md

```

## Limits

Rendering used llvmpipe (LLVM 19.1.7, 256 bits), Vulkan CPU device and synthetic egui input in owned fixtures. No physical OS-input, hardware-GPU or Windows execution claim is made. Density is recipe/model-space only, not world-space density after model binding transforms. Reserved blockout/model-panel implementation, renderer and schema were not changed by this adapter.
