# Opt-in compact local validation

Run `python3 -B tools/check_linux_compact.py --target-dir /absolute/stable/target`.
Choose one target per compatible toolchain/platform/work stream and reuse it
across attempts instead of making a new target for every source snapshot.
Use a trusted, user-owned directory; do not share the target with untrusted writers.
The launcher runs the existing `tools/check-linux-local.sh` unchanged and
propagates its exit status. It does not waive hosted CI, review, native/UI
acceptance, or any previously failing gate. Cargo decides which artifacts are
fresh; this does not reuse prior test results or prove source identity.

Only the launcher's child environment changes: incremental compilation is off,
and dev/test debug information is level 1 rather than the default level 2.
Normal developer Cargo invocations and repository profiles are unchanged.
Level 1 retains line information for backtraces but omits detailed variable/type
debug information; incremental-off can make repeated source edits slower.
Release debug information, optimization, assertions, features and test selection
remain unchanged. Existing RUSTFLAGS/Cargo configuration can override effective
compiler settings; inspect verbose compiler commands when measuring.

The explicit target path prevents accidental per-attempt cache creation. Compact
launchers serialize on that target; other Cargo commands retain Cargo's own
locks, and external binaries/readers still require owner coordination. The
launcher never deletes or strips existing artifacts, evidence or source. The
first compact build can temporarily add artifacts beside older configurations.

Free space is checked before creating a target and again before running the gate.
The default reserve is 8 GiB; `--min-free-gib` may set a measured reserve of at
least 1 GiB. This is a starting-space check, not a guarantee that a full cold build
fits or a runtime quota. Do not lower it just to force a large build to start.
Use the ordinary gate for full debugger detail and record which mode generated
each result. Never describe wrapper fixtures as a full engine gate pass.
