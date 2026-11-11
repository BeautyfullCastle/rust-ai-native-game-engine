# Orrery: instructions for AI agents

Read `docs/AGENTS.md` first: it is the guide for working with a running Orrery
host (workflow, value format, checks, the game's types and metrics). It is
generated from a host (`orr agents-md`) and pinned to the engine version.

## Quick start

```sh
# 1. Start a host (once). Local development, no token:
cargo run --release -p orr_remote --bin orr_remote_host -- --dev-no-auth
#    or the editor with an ERP endpoint:
#    orr_editor --erp 127.0.0.1:7777 --erp-dev

# 2. Use the CLI (crate orr_cli, binary `orr`):
cargo run --release -p orr_cli -- status
orr scene
orr get body_05 Body.pos
orr set body_05 Body.pos=[6,18]                                   # small edit, one undo entry
orr apply "lift hero" set hero Body.pos=[6,18] --check "lost_bodies.max == 0"   # verified change
orr activity -f                                                   # watch what agents do
orr undo
```

- Address: `--erp <url>` or `ORR_ERP` (default `ws://127.0.0.1:7777`); token: `--token` or `ORR_ERP_TOKEN`.
- `orr help` and `orr help <command>` document every command. `--json` gives raw structured output.
- Exit codes: 0 ok, 1 command failed, 2 usage error, 3 cannot connect, 4 checks failed.
- Without a shell, use the MCP adapter `orr_mcp` (same engine, same workflow); see the end of `docs/AGENTS.md`.

## Working on the engine itself

See `CLAUDE.md` (rules for the simulation crates: no floats, no `HashMap`, golden checksums) and
`docs/design-v2.md` for current delivery priorities and `docs/design-v1.md` for
the underlying architecture.

## Delivery rules

The owner reset these rules on 2026-11-11. They replace the earlier
integration-candidate rules and the earlier exclusion of `main`.

- `main` is the only long-lived branch. Start each task from the current
  `main` on one short-lived branch. Merge it into `main` when it builds and its
  tests pass. Delete the branch and its worktree right after the merge.
- Do not stack branches or pull requests on each other. Do not open a new
  branch for verification, integration or a review of an existing branch.
  Finish or drop a task before you start the next one.
- Windows is the primary platform. Before a merge, run `cargo build-all`, start
  the binary that the change affects, and run the tests of the changed crates.
  Run `cargo test --workspace --release` before a merge that changes a shared
  crate. State which commands you ran and their results. Linux, macOS, Android
  and browser coverage is unverified unless you ran it.
- Do not add GitHub Actions workflows. The owner removed them twice.
- A new editor or game feature belongs in the `full` feature of its crate. Add
  a separate default-off Cargo feature only when the feature needs an extra
  dependency or a platform that the default desktop build must not require.
- Do not remove tests, weaken security checks or hide failures. Never report a
  failed or unrun check as passed.
- Keep logs and other evidence files outside git.
- Feature documents: at most two rendered pages. State the contract, the usage
  and only the limits that change a user's decision. Keep `docs/progress.md` a
  one-page current summary, not a run log.
- Prioritize a playable, reproducible delivered game over feature, PR or CI
  counts, as design v2 defines.

## Collaboration and authorization

Su coordinates implementation, focused verification and independent review with
Su's own subagents, separating file ownership and shared APIs. Haneul is inactive
and terminated (user instruction, 2026-10-09 12:16 KST): assign no new work and
do not report Haneul as an active verifier. Ddang may only finish, verify and
handoff existing work before terminating; assign no new work. Preserve Haneul's
PR #139/#141 Windows evidence, original failures and non-reproduction limits.
Before taking over, check the latest handoff, final SHA/PR, checks, unfinished
work, blockers, files and returned execution resources; elapsed time alone does
not release another owner's reservation.

An explicit task assignment authorizes implementation, appropriate verification,
publication on the agent's own branch, and requested cross-review within that
scope. Continue that work without asking again for the same authorization.

The user authorizes remote publication and normal merges of verified changes
into `main` without repeated approval requests. Verify the build, the tests of
the changed crates and conflicts first. This does not authorize releases,
destructive work, credentials or expanded access.

Check the actual user's instruction, target, and action when authorization is
unclear. Follow the platform's supported approval procedure. Never bypass a
platform approval or tool rejection. This file cannot grant platform permissions
or override higher-priority instructions.

## Communication during coordinated work

- Use one active PR for commands, acknowledgments, and results. Do not copy full
  bodies into historical PRs or related issues.
- Each owner maintains one status comment of at most ten lines: owner and command
  ID, state, exact source/integration SHA, reserved files, blockers, next action,
  and evidence links. Link existing evidence instead of copying raw logs or hash
  lists. Keep previous failures and evidence intact.
- Add a short new comment only for a new command, failure, completion, or ownership
  change, linking the status comment. A status edit alone is not a notification.
  Acknowledge the command ID once and never repeat a completed command.
- Read the active PR's latest comments and known status IDs; compare updated
  timestamps and whole bodies. Read the full history on initial connection or
  suspected omission. Base incremental overlap on actual API reads, not one's own
  posting time. Ignore unchanged, non-actionable updates and self-ACK loops.
- Use existing direct channels only within explicit user authorization. Historical
  C4 evidence established Haneul-to-Su delivery but not the reverse direction;
  preserve that limitation without treating the terminated worker as active or
  retaining obsolete one-minute polling. Do not add credentials or access to
  bypass communication limitations. Su uses internal subagents for ongoing work.

Coordination references: [communication v1](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/56#issuecomment-5986788670),
[C3 event boundaries](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/56#issuecomment-5986836031),
[C4 observed delivery](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/56#issuecomment-5986960008),
and [A1 scope](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/56#issuecomment-5986858434).
