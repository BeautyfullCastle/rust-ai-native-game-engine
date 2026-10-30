# Keep test iteration cheap without dropping coverage

The historical CI harness changes below retained the release profile, every
platform/matrix leg, and the library, integration, golden-checksum, GPU, browser
and FFI checks. Current acceptance follows `AGENTS.md` and
`bash tools/check-linux-local.sh`; historical CI coverage is not a current pass.
`--timings` reports from native and sample jobs measure compilation separately
from test execution; compare equivalent jobs before claiming a speedup.

## Release artifact size

The workspace release profile now uses `debug = 0` and `strip = "debuginfo"`
instead of generating limited debug information. This removes source-file/line
debugging information from release artifacts; it is not `strip = "symbols"`.
The bench profile inherits this change. Dev/test profiles and the existing
web/web-small settings are unchanged. Optimization level, thin LTO, codegen
units, overflow checks, debug assertions, panic strategy, tests and feature
selection are unchanged. See [Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html).

Treat this as a separately validated build-setting change, not a claim of
measured savings or a replacement for previous validation receipts. Before
building, let existing gates finish, preserve their exact source identities,
commands, results and evidence, and establish a safe disk budget. Changing
profiles can create additional artifacts before old ones are retired; adding
another target directory does not itself save space. Do not alter an active
gate's source or shared artifacts.

After review and resource clearance, freeze and record the exact commit SHA
before measurement, then run the unchanged Linux-local gate and compare
representative test lists/results and golden checksums. Record toolchain,
commands, features, profile settings and cache state, plus allocated bytes
(`du` or `stat` blocks, not only logical file sizes), peak target-directory and
filesystem usage, artifact sizes/debug sections and build time. Keep original receipts; do not
claim identical-profile timing comparability across this change. Cross-platform,
browser, hardware/GPU and ignored-test coverage remains unverified unless run.

## Existing harness work

Nine audited binary entrypoints have no unit tests or local modules and now
explicitly use `[[bin]] test = false`:

- Native: `orr_remote_host`, `orr_mcp`, `orr_server`, `orr_tui`, `bench_runner`
- Sample/editor: `orr_editor`, `arena`, `physics`, `physics3d`

This skips only their duplicate empty test harnesses. Every owning package has
integration tests, so the existing package-wide `cargo test` commands still
build the normal binaries for those tests. Historical CI also checked all binary
targets through `cargo clippy --all-targets`; explicit
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

Historical focused harness checks (not the full current acceptance gate):

```sh
cargo test -p orr_tui --test deps
cargo test -p orr_editor --test ui
cargo check -p orr_remote -p orr_mcp -p orr_server -p orr_tui -p orr_wasm_bench -p orr_sample -p orr_editor --bins
python3 tools/test_determinism_workflow.py
```
