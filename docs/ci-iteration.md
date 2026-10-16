# Keep test iteration cheap without dropping coverage

CI still uses the release profile, every existing platform/matrix leg, and the
same library, integration, golden-checksum, GPU, browser and FFI checks.
`--timings` reports from native and sample jobs measure compilation separately
from test execution; compare equivalent jobs before claiming a speedup.

Nine audited binary entrypoints have no unit tests or local modules and now
explicitly use `[[bin]] test = false`:

- Native: `orr_remote_host`, `orr_mcp`, `orr_server`, `orr_tui`, `bench_runner`
- Sample/editor: `orr_editor`, `arena`, `physics`, `physics3d`

This skips only their duplicate empty test harnesses. Every owning package has
integration tests, so the existing package-wide `cargo test` commands still
build the normal binaries for those tests. CI also checks all binary targets
through its unchanged `cargo clippy --all-targets` commands; explicit
`cargo build`/`cargo run --bin ...` still work. The `orr` CLI's binary harness
stays enabled: its `cmds` and `ops` modules contain real tests.

The sole `ui_frames_record_bounded_wall_time_samples` test moved verbatim from
`orr_editor/tests/diagnostics.rs` into `orr_editor/tests/ui.rs`. The combined
harness has the same 18 tests (17 UI + 1 diagnostics); its diagnostic assertions
are unchanged. It owns its editor/host and egui context, does not change process
environment or shared files, and has no wall-time threshold. Other independent
integration suites retain their own processes.

When adding tests to a testless entrypoint, put the testable code in the library
or an integration test, or remove `test = false` first. The existing
`orr_tui/tests/deps.rs` harness uses Cargo metadata to check every testless
binary source (currently these nine) for test attributes,
`cfg(test)`, local modules, `include!` and macro definitions. Its conservative
text guard is not a Rust macro expander: review new external/procedural macros
for generated tests and re-enable the harness if needed. New testless binaries
must also be audited; do not disable harnesses with a workspace-wide rule.

Focused checks (no release-profile or test-matrix reduction):

```sh
cargo test -p orr_tui --test deps
cargo test -p orr_editor --test ui
cargo check -p orr_remote -p orr_mcp -p orr_server -p orr_tui -p orr_wasm_bench -p orr_sample -p orr_editor --bins
python3 tools/test_determinism_workflow.py
```
