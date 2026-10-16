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
`docs/design-v1.md`.

## Collaboration and authorization

Su coordinates the work directly. Haneul and Ddang implement changes, run the
assigned checks, and cross-review each other's work. Respect the assigned files
and reservations; do not take another owner's work because time has passed.

An explicit task assignment authorizes implementation, appropriate verification,
publication on the agent's own branch, and requested cross-review within that
scope. Continue that work without asking again for the same authorization.

When the user has approved a specific PR's normal merge into the shared
development branch, verify the target branch, exact head, required CI, review,
conflicts, and reservations, then perform that approved merge without repeating
the approval request. Approval applies only to the specified PR and action.
Main/PR1 merges, releases, destructive work, new credentials, and expanded access
require separate explicit user authorization.

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
- Use an existing direct channel only within explicit user authorization. The
  current C4 check confirmed Haneul-to-Su delivery, but Su-to-Haneul direct replies
  failed; direct round-trip communication is unverified. Su commands therefore
  remain short active-PR comments. Keep the existing one-minute backup reads until
  both directions are verified; do not add servers, credentials, or access to
  bypass this limitation.

Coordination references: [communication v1](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/56#issuecomment-5986788670),
[C3 event boundaries](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/56#issuecomment-5986836031),
[C4 observed delivery](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/56#issuecomment-5986960008),
and [A1 scope](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/56#issuecomment-5986858434).
