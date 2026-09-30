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
