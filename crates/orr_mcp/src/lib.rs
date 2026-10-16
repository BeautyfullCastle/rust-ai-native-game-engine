//! `orr_mcp`: the Orrery MCP adapter.
//!
//! A Model Context Protocol server over stdio that bridges an AI agent to a
//! running Orrery host (the headless `orr_remote_host`, or the editor started
//! with `--erp`) through ERP, the Engine Remote Protocol of `orr_remote`.
//! It is a separate, thin process: it holds no scene and runs no simulation.
//! Everything the agent does goes through the host's `EditorDoc`, so the
//! agent's accepted changes are ordinary history entries (`agent:<client>`)
//! that a person can undo.
//!
//! # The workflow the tools give an agent
//!
//! 1. Look: `scene_overview`, `get_entity`, `get_schema`.
//! 2. `propose_changes`: stage edits on a private copy; get a diff. The scene
//!    is untouched.
//! 3. `verify_proposal`: the engine replays the scene with and without the
//!    proposal on recorded or scripted inputs (deterministically) and compares
//!    checksums and metrics; `checks` such as `lost_bodies.max == 0` turn it
//!    into pass/fail.
//! 4. `accept_proposal` (one undoable history entry) or `reject_proposal`.
//!
//! The command line `orr` (crate `orr_cli`) is the primary path for agents
//! with a shell; it reuses this crate's connection ([`Bridge`]), tool
//! functions ([`tools::run`], [`tools::stage_proposal`]) and report texts
//! ([`report`]). This adapter is for clients without one.
//!
//! Also `list_proposals`, `history`, `undo`, `sim_run` and `sim_input`.
//! Discover structured input with `get_schema` `{ "input": true }`, start a
//! session, set an input for a player and step. The host owns input validation;
//! not every game exposes structured input. Exact decimal values stay exact.
//! Tool groups
//! (`scene`, `propose`, `verify`, `sim`, `history`) switch on and off with
//! `--tools`, to keep the prompt small. Resources: `orrery://scene`,
//! `orrery://schema` and `orrery://agents` (the generated AGENTS.md).
//!
//! # Running it
//!
//! Start a host, then point the adapter at it:
//!
//! ```text
//! orr_remote_host --token claude:s3cret:read,scene_edit    # an agent may propose and verify, not accept
//! orr_remote_host --token claude:s3cret:all                # ... or accept too
//! orr_mcp --erp ws://127.0.0.1:7777 --token s3cret
//! ```
//!
//! The token may also come from the environment (`ORR_ERP_TOKEN`), which
//! keeps it out of the process list. A host started with `--dev-no-auth`
//! needs no token. The `approve` capability is what `accept_proposal` needs:
//! leave it out of the agent's token to keep the decision with a person.
//!
//! ## Claude Code
//!
//! ```text
//! claude mcp add orrery -- orr_mcp --erp ws://127.0.0.1:7777 --token s3cret
//! claude mcp add orrery --env ORR_ERP_TOKEN=s3cret -- orr_mcp --erp ws://127.0.0.1:7777
//! claude mcp add orrery -- orr_mcp --erp ws://127.0.0.1:7777 --tools scene,propose,verify
//! ```
//!
//! ## Other MCP clients
//!
//! Any client that can start a stdio server takes the same command. In a JSON
//! config (Claude Desktop, Cursor, and so on):
//!
//! ```json
//! { "mcpServers": { "orrery": {
//!     "command": "orr_mcp",
//!     "args": ["--erp", "ws://127.0.0.1:7777", "--tools", "scene,propose,verify,history"],
//!     "env": { "ORR_ERP_TOKEN": "s3cret" }
//! } } }
//! ```
//!
//! ## AGENTS.md
//!
//! `orr agents-md` (and `orr_mcp --print-agents-md`) prints an agent guide
//! generated from the host: engine version and build id, the type registry,
//! the metric names, the `orr` workflow, and the tools as the MCP
//! alternative. It is pinned to the version it was generated
//! from; `docs/AGENTS.md` is the one for the demo scene (a test regenerates
//! it and fails when it is stale: run it with `ORR_UPDATE_AGENTS=1` to
//! rewrite the file). The same text is the resource `orrery://agents`.
//!
//! # Protocol notes
//!
//! Newline-delimited JSON-RPC 2.0, MCP revision `2025-06-18`, implemented by
//! hand in [`server`]: `initialize`, `notifications/initialized`, `ping`,
//! `tools/list`, `tools/call`, `resources/list`, `resources/read`. Only
//! JSON-RPC messages are written to stdout; logs go to stderr. A failed tool
//! is a result with `isError: true` and the engine's message; protocol errors
//! (unknown method, bad params, not initialized) are JSON-RPC errors. The
//! connection to the host is made when first needed and made again after a
//! failure, so the adapter can start before the host.

pub mod agents_md;
pub mod bridge;
pub mod report;
pub mod server;
pub mod tools;

pub use agents_md::generate as generate_agents_md;
pub use bridge::{Bridge, Connector, ErpCall, Fail, Style};
pub use server::{serve, McpServer, PROTOCOL_VERSION};
pub use tools::{ToolGroups, GROUPS};
