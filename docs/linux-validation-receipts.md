# Same-source Linux validation receipts (limited adapter)

`tools/linux_local_gate.py` offers an executable adapter **only** for source-policy
phases 17/18/20: observation pacing, standalone focus synchronization and native UI
settling. These inspected scripts import only pathlib/unittest/re and read fixed
repository files; they execute no network, compiler, subprocess or custom imports.
The original test bodies and test counts remain unchanged. This is not a solution
for expensive workspace test/build reuse and not a replacement full gate.

After independent review, invoke a selected phase explicitly:

```
python3 -I -S -B tools/linux_local_gate.py \
  --phase static_game_standalone_focus_sync --receipt-dir /outside/checkout/receipts
```

The shell gate opts in only when `ORR_LOCAL_POLICY_RECEIPTS` names an external
receipt directory. Only these three dispatches change; unset means the original
commands. Adapter failure propagates immediately with no silent fallback. All
other mandatory commands remain unchanged. No Cargo/full-gate execution is needed to test the
policy: `python3 -B tools/test_linux_local_gate.py` uses fake commands only.

The new explicit isolated Python execution policy disables Python startup hooks,
user site-packages and ambient import-path injection (`-I -S`); optimization is
forbidden so assertion-based regressions still run. It retains the actual child
environment unchanged: no proxy deletion, credential capture/replay, network
setting or sandbox change occurs. Source contents, cwd, exact command, adapter,
Python executable and standard-library Python/extension payloads are pinned and
rechecked after execution. Receipt logs and artifacts must still match their hashes.
Unknown source/dependency changes invalidate reuse; no crate closure is claimed.

A names-only comparison of two tool executions observed changing
`CODEX_TOOL_CALL_ID` and proxy variables. Their enumerated names are omitted from
keys **only for the three audited local-only phases**, not for arbitrary commands.
Unknown environment keys, compiler/runtime/path/home/temp inputs remain included.
Receipts contain an aggregate fingerprint and names, never raw environment values
or individual value hashes. Historical aggregate hashes cannot prove which old
values changed: historical receipts are not imported or retrospectively approved.

CARGO_NET_OFFLINE constrains Cargo fetching, not build scripts/proc macros/test
binaries. Therefore it does not justify ignoring proxy changes for heavy phases.
Those still require a dependency/runtime network-usage proof or approved enforced
outbound-deny boundary with needed loopback fixtures. Removing proxies alone could
cause direct connection attempts and is not an acceptable shortcut.

The generic receipt primitives retain all 23 legacy phase definitions and require
full coverage for a composed aggregate, but are not a heavy-phase execution adapter.
Only matching PASS or explicit non-signal formatting ADVISORY is reusable. FAIL,
interrupted, missing-terminal, missing/tampered log evidence is never reused.
Composed coverage never claims an original uninterrupted shell exit. Linux evidence
does not certify Windows/macOS/Android/browser/GPU/ignored UI tests.

The transport-relevance policy pins exact independently reviewed script SHA256s;
changed scripts require a policy update before reuse or execution. Fixed source
reads are bound independently of Git's index, including all three pacing inputs.
Script/dependency symlinks are rejected. Runtime inventory includes readable `.pyc`
caches and import archives (or their absence); unknown import paths fail closed.
A persisted latest-attempt pointer prevents clock rollback resurrecting old PASS.

This is a trusted-workspace receipt system, not a signature or hostile-writer
sandbox: checkout, Python/runtime and receipt storage must remain controlled and
stable during checks. Pre/post hashes detect ordinary drift but are not a defense
against an adversary changing and restoring files between observations.
