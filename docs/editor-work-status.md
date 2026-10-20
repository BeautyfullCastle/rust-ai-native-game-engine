# Popup-free editor work observations (v1)

`editor.work_status` is a read-only ERP observation under the existing `read`
capability. MCP offers `editor_work_status` in the existing `scene` tool group.
Neither endpoint saves, applies, discards, closes, opens UI, changes focus, or
requests a repaint. No close attempt or details tab is needed. No new listener,
token, permission grant, subscription, worker or filesystem probe is introduced.

## Scope and support

The target is the main EditorApp paired to this ERP host, not the caller's visible
window, every editor on the machine, or sidecars in a remote viewer. Initial
backend wiring supports PhysGame, Arena and CollectDodgeV1. Existing headless
factories, remote viewer owners and other game profiles remain unpaired. Their
method-table entry is not evidence of support: a direct call returns
`not_paired`, and discovery has no live editor feature descriptor. The descriptor
appears only for an installed pair and only to Read-capable clients; its current
availability is advisory and can change immediately.

The fixed local inventory is sprite bindings, tilemap, Collect UI document and
preview, model bindings, animation bindings, terrain, irradiance bindings, Room
camera, Mesh geometry/art, camera, lighting, HUD, mission, hazards and animation.
Retained hidden-profile documents are included. A row distinguishes
`feature_disabled`, `not_loaded`, `loaded` and `unknown`; only an evaluated loaded
predicate can be `clean`. `domain_revision` is null because a universal reviewed
revision does not exist. Combined predicates remain combined rather than
inventing separate dirty/staged flags.

Compiled Mesh panels currently expose combined blockers but not reviewed
retained-document presence. A positive predicate is reported with its combined
state; a false predicate is `unknown`, not invented clean/not-loaded. Therefore
even a supported Phys/Arena/Collect host in a feature-rich Mesh build can report
incomplete coverage. Room UI, standalone authoring apps, package-manager mutation
jobs, arbitrary plugins, other windows and future edits are outside this inventory.

## Request and response

ERP arguments must be an object containing only optional integer `max_age_ms`,
1 through 5000, default 1000. Unknown fields, null values, paths and mutation flags
are rejected with a fixed parameter error. The existing MCP server normalizes an
absent/null outer `arguments` member to an empty object; tool-specific fields use
the same strict validator. This does not broaden the direct ERP contract.

The result contains `schema_version: 1`, the fixed scope and target,
`availability`, `observation`, `host_scene`, `editor_local`, `editor_operations`
and `assessment`. It has no paths, titles, labels, tokens, client/transaction-owner
identities or free-form diagnostics. Binding IDs and counters are decimal strings;
the checksum is a hexadecimal string. The mailbox has one fixed-size snapshot,
accepts at most 32 input records, canonicalizes the 16 unique domain IDs, and
bounds the serialized response to 16 KiB. Malformed, missing, duplicated or
inconsistent records are incomplete, never empty clean. MCP additionally validates
the complete bounded success schema before forwarding it; a non-contract success
is a fixed local tool error with no raw payload.

`host_scene` is read together on the ERP host thread at request handling, including
in Play: it is the scene document, not a Play-frame checksum. `editor_local` is the
last completed UI callback. `consistency` is always
`host_at_request_local_at_published_frame`: these are explicitly non-atomic
observations, not a quiescence fence. Save can change dirty without changing
`preview_revision`; rollback may restore an earlier revision. Paths, checksums and
preview revisions are never freshness or persistent identity proofs.

The sole non-clone owner publishes after every completed normal App callback,
including early returns and same-callback widget edits. Reading never refreshes
age or sequence. Try-lock contention returns `busy`; an absent publisher returns
`owner_absent`; no first frame returns `not_ready`; an old frame returns `stale`.
Stopped/minimized UIs may stay stale indefinitely. Owner/backend replacement gets
a new binding; counters do not wrap. IDs are process-local live-host-connection
identities, not persistent globally unique IDs. Discard prior observations on
ERP reconnect or MCP reinitialization. Poll only while needed, at most twice per
second; existing server admission and per-poll bounds still apply.

## Assessment is not permission

- `known_blockers`: fresh positive local blockers or a current host dirty/transaction
  blocker. A current host blocker remains positive even when local coverage is
  stale, absent or incomplete; that limitation is still reported.
- `none_observed`: fresh, complete declared inventory, current host clean and not
  in a transaction, and reviewed idle editor operations. This means only that the
  component observations contain no listed blocker. It is not a save receipt,
  exit consent or a guarantee against data loss.
- `indeterminate`: no fresh positive blocker and unavailable/stale/incomplete data,
  unknown presence, pending mutation or uncertain outcome. Stale local blockers
  are last-observed data, never promoted to current blockers.

Ordinary pending reads are not mutations. Pending gestures/managed input are
reported separately. A disconnected editor or uncertain batch acknowledgement is
`outcome_uncertain`, even after normal connection-loss handling clears queues.
Status does not change #265's close policy, clear its blocked indicator, select a
tab or arrange later auto-close. Existing managed-input/transaction expiration
and concurrent edits still run normally; read-only is not a global freeze.

## Compatibility and verification

ServerConfig's public fields, ErpServer::start, LocalHost::spawn, existing
Phys/Arena/Collect factories, Backend constructors, HostSpec and screenshot APIs
retain their signatures. Additive named status factories call the same private
setup/configuration functions and install the moved service before readiness.
Arena/Collect managed input setup is preserved. Screenshot permits, queues and
encoder lifetimes are independent. `orr status` and CLI exit codes are unchanged.

Successful observations with blockers or availability limits are MCP
`isError: false`; the text explains non-atomicity and the no-exit-permission limit.
Actual ERP failures keep #271's per-call detailed cause and negotiated-version
compatibility. A genuinely older ERP host returns method_not_found. Listing the
static tool performs no network I/O and proves neither permission nor liveness.

The focused gate reuses the existing sample CI job, with no new matrix. It runs
pure mailbox/schema/security tests, real synthetic-owner ERP and authenticated
socket tests, all MCP negotiated versions, actual CPU App callbacks, and a
separately explicit actual App → ERP → MCP subprocess test. The subprocess test is
marked ignored in general sweeps because it requires a prebuilt binary; the gate
builds that exact binary, sets ORR_WORK_STATUS_MCP_BIN, and explicitly executes and
counts the test. Missing binaries never silently skip. Busy is established by
mailbox contention tests and schema-to-MCP forwarding; not_paired uses real
headless ERP, not a fictitious paired App. These are not physical Windows/native
window/GPU acceptance. Existing #265's exact 19 tests and required broader CI
remain independent mandatory checks.
