# Collect input action maps verification — 2026-10-19

Base: `c0050905a7330c7e2b113b41387fc009a6ad7b8b`. Local Linux verification ran on 2026-10-18 UTC. Independent source review approved adapter/runtime/bin/view code; root inspected the additional restart-alias regression and final production diff.

## Executed gates

- **Enabled cohort: 40 executions** with `collect-dodge,input-actions`: adapter16 + runtime8 + existing app9 + view2 + CLI5.
- **Disabled cohort: 16 executions** with `collect-dodge`: default mapping4 + existing app9 + CLI3.
- **Collect UI cohort: 23 executions** with `collect-ui`: app policy11 + existing UI12, including pointer capture, DPI/clipping, restart acknowledgement and synthetic pointer-event behavior.

These feature cohorts overlap; they are not 79 unique tests. CLI integration tests exercise pre-window malformed/version/feature/headless/argument refusals and valid-map project admission without rewriting source files. Runtime tests drive the production input policy and real Bridge, covering remapped movement/win/restart/quit traces, inert former bindings, aliases, queue cap, repeats, subtick taps, neutral acknowledgement, focus, synthetic events and capture policy.

Default sample library check and strict library/bin/CLI Clippy passed (`--features collect-dodge,input-actions -- -D warnings`). Logs: `/tmp/collect-input-v1-{enabled,view,cli,disabled,disabled-cli,default,clippy}.log` and `/tmp/collect-input-v2-{ui-policy,ui,build}.log`.

## Separate headless renderer smoke

The production `collect_dodge` binary built with `collect-ui` ran the authored `assets/collect_dodge_project` using `--headless --ticks 20 --hold right --capture ...`. It reached **WON, score2/2, tick20**, checksum `0x8c2569821a7492f1` from initial `0x86a508978d9b36a9`. Capture used llvmpipe (LLVM19.1.7,256bits), explicitly reported software rendering.

Evidence: `/workspace/shared/collect-input-evidence-20261019/headless.png`, `headless.log`, source manifest/stability proof. Screenshot SHA256 `672435920ff5f04db8847f6c092de69dce20045cd8853268a5361a300a53a770`. Built binary SHA256 `5b36e622443fa9e470bcd648b3ac5e1710c83c8612af3f694e9d5ead327e5967` at `/workspace/shared/engine-cargo-target/debug/collect_dodge` (build output may later be replaced by another feature cohort).

This smoke exercises semantic headless commands and rendering; it does not demonstrate remapped native-window keyboard input. Native-window creation, physical keyboard delivery, hardware GPU and Windows execution remain unverified locally.

## Failures and scope

No executed test, CLI integration, strict lint or headless smoke failed. One compilation chain was intentionally interrupted before tests so an optional generic-audio command could be removed after scope clarification; the partial `/tmp/collect-input-v1-ui-policy.log` is retained, followed by the complete v2 run. This checkout has no Collect-specific audio module/feature; no generic audio run was substituted or claimed. Disk pressure paused further builds for root-owned inactive-cache cleanup; source/evidence/current artifacts were retained.

The shared input-map format and Arena behavior were not changed. This adapter admits only supported Collect semantic actions and keyboard bindings; file/path checks retain the existing shared loader trust limits rather than claiming isolation from hostile concurrent filesystem races.

Final independent workflow review approved SHA256 `a79c47fc40413bc0db063b8ae64aa01640445bcf782e35dc28e42c048e76b4f6`; compiled inventories and exact production-binary CLI predicates were verified. Additional predicate evidence: `/tmp/collect-input-workflow-cli-v1`.

## Frozen source

Manifest `/tmp/collect-input-v1-source.sha256` SHA256 `9890c58e0b3b966448c5fea71d201fb691d6aec2050d61a9d5142795065846f0`. All seven production/test entries remained unchanged through all executed cohorts:

```text
3983c8d15c2c853becf3a278cc2aa677539175a09846357c72ea77cf540612b4  crates/orr_sample/src/bin/collect_dodge.rs
5cbd4ed31ccc46105272ed47448050bff8a2be3d6b3771fee409eeabfc0e074f  crates/orr_sample/src/collect_app.rs
39c1dd657b662b1e3a1c3295c895c003bedd181de523ce941888fd8d64d9e3b7  crates/orr_sample/src/collect_app/input_actions.rs
c9059845c9fd343e997806b7cba0f0230def741864e18a757b7d664705ed4b2f  crates/orr_sample/src/collect_app/input_actions_tests.rs
b2b029c3e0c423219e74b4133754e9962eb8f97ef8e8b01873a4f8b69703fd81  crates/orr_sample/src/collect_app/input_runtime_tests.rs
c0feeb0f9efe1025d2130c332f9270018ee8d56dbe2791e77538a8198e6bf7e4  crates/orr_sample/src/collect_view.rs
826fc222bacc0ad9629bb49c1c0d250fd32a6bebb2d4305ad8865a76335939bd  crates/orr_sample/tests/collect_input_cli.rs

```

The subsequently completed user guide and CI workflow are outside this seven-file execution manifest. Windows inventory differences from Unix-only tests are source-derived, not locally executed Windows results.
