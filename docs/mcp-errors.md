# MCP tool failure details

`orr_mcp` preserves a bounded observation from a received ERP `RpcError` without
changing the public `Fail(pub String)` API or the `orr` CLI contract. This is not a
new ERP error taxonomy, permission grant, or automatic-retry policy.

## Wire contract

For negotiated MCP `2025-06-18`, a tool execution failure that received a typed ERP
error remains a JSON-RPC **result**, not an outer JSON-RPC error:

```json
{
  "content": [
    {"type": "text", "text": "The existing human explanation and hint"},
    {"type": "text", "text": "{\"error\":{\"origin\":\"erp\",\"code\":-32002,\"data\":{\"kind\":\"permission_denied\"}}}"}
  ],
  "structuredContent": {
    "error": {
      "origin": "erp",
      "code": -32002,
      "data": {"kind": "permission_denied"}
    }
  },
  "isError": true
}
```

The second text block is exactly `structuredContent` serialized as JSON. Existing
consumers can continue reading the first block. The code is the actual typed
`RpcError.code`; this adapter does not validate the original wire code again or
change the ERP client's existing malformed-response normalization.

Only literal kinds reviewed in `bridge.rs::safe_kind` are projected. The initial
allowlist covers ordinary scene/proposal/verification, capability, admission and
internal errors already defined in `orr_remote`. A missing, unknown, non-string,
or malformed kind omits the entire `data` member. Identifier-shaped unknown
strings are not trusted. The numeric code is retained even if its kind is unknown.
No other ERP data is forwarded: no `message`, owner, required capability, maximum,
path, URL, input parameters, nested object, or arbitrary host text enters the new
structured projection.

Negotiated `2025-03-26` and `2024-11-05` keep their previous one-text-block tool
failure shape. Unknown requested revisions still negotiate the server's current
revision. Success values, text, tool definitions, schemas and existing success
output behavior are unchanged, including under the older revisions. No new
`outputSchema` is advertised.

Local validation failures, connection/setup/authentication failures that arrive
as connector text, timeouts and disconnections remain text-only tool failures.
They do not acquire a fabricated ERP code. Unknown tools, malformed request or
argument envelopes, uninitialized requests and other MCP protocol failures keep
their existing outer JSON-RPC `error` envelopes. `resources/read` behavior is
unchanged. A genuine ERP `-32603` / `panic` is still a tool failure with
`origin: "erp"`; it is not an MCP internal/protocol failure.

## Failure ownership and uncertain outcomes

Private detailed failures carry their sanitized cause **by value** through the
bridge, tool dispatch and proposal cleanup. Public `Bridge::call`, `tools::run`
and `tools::stage_proposal` keep their signatures and discard only the private
details when returning the legacy `Fail`. There is no mutable last-error cache,
thread-local lookup, or parsing of human explanations to recover error metadata.
An ignored preparatory call or cleanup failure cannot replace the primary cause.

A failed `proposal.apply` still attempts to reject a proposal created by that
call. Ordinary known ERP rejection followed by confirmed cleanup retains the
existing wording. Cleanup failure now says cleanup was not confirmed instead of
incorrectly asserting that the proposal was discarded. Lost apply responses and
internal errors never claim no ops were staged, even when subsequent cleanup
succeeds. An existing proposal is not discarded on failure. No failed operation
is automatically retried; later explicit calls retain existing reconnection
behavior. Inspect current engine/proposal state before deciding how to proceed.
There are no new `retryable`, `no_change`, `safe_retry` or equivalent assertions.

## Targeted credential hygiene, not DLP

The displayed failure text, including a host-supplied kind, removes exact echoes
of the nonempty token configured with `Bridge::to_url`. URL userinfo, queries and
fragments are removed in bridge errors and public RPC text formatters. Startup
and invalid-scheme diagnostics apply the same URL/token rendering. Nonsecret
host/port/path diagnostics are preserved. This does not alter the URL or token
used to connect, and does not change authentication, capabilities or transport.

This is not comprehensive secret detection: public standalone RPC formatters or
custom `Bridge::new` connectors do not know arbitrary credentials. Unknown secret
values, encoded/transformed token echoes, sensitive paths and arbitrary prose
may still occur in legacy human text. Treat host messages as untrusted content.
The new structured projection does not include that text or arbitrary data.

## Verification

Focused tests cover actual ERP permission/unknown-entity/stale-verification and
verification admission and deterministically gated `verify_busy` errors; scripted ERP internal errors; malicious metadata;
primary failures across cleanup success/failure/uncertain responses; independent
calls and ignored preflight errors; all negotiated revisions; unchanged success
shape; legacy tuple construction and wrapper signatures; and targeted token/URL
redaction (including Unicode). Existing subprocess tests retain protocol and
JSON-RPC-only stdout coverage. Run the MCP and CLI suites together and strict
Clippy for both packages. Required exact-head CI and independent review remain
separate from these local checks.

Protocol reference: [MCP 2025-06-18 Tools](https://modelcontextprotocol.io/specification/2025-06-18/server/tools)
(structured content, serialized text compatibility, tool versus protocol errors,
and output sanitization). This is an independent adapter implementation; no
third-party engine source is imported.
