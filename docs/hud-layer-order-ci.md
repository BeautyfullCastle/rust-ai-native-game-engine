# Mandatory exact-head HUD layer-order acceptance

The reusable `.github/workflows/hud-layer-order.yml` lane is called from
`determinism.yml`. The existing required `cross-platform checksum comparison`
aggregate depends on its result and accepts only `success`; failure, cancellation,
skips and missing results fail the aggregate. Existing baseline jobs remain.

Pull requests explicitly check out the requested PR head, not GitHub's synthetic
merge commit. Checkout credentials are not persisted. The gate records that exact
HEAD/tree, complete tracked input hashes/modes checked against every Git tree blob, clean compiler configuration,
resolved tool hashes/versions, producer Cargo JSON and verbose compiler/extern
provenance. Evidence contains no full runner environment or Git configuration.

## Required acceptance

1. Fresh `orr_editor` integration producer with
   `--no-default-features --features collect-ui,project-create`; exact complete
   inventory of the three application/keyboard/clipping cases, no ignored CPU
   cases, and one successful run of each exact case
2. Dedicated combined-feature release all-target Clippy with `-D warnings`
3. Fresh `orr_sample` generator, Collect runtime and exporter from the explicit
   combined tool feature closure; exact package/target/features/profile checks,
   compiler-output attribution and full executable identities
4. Fresh GPU test executable with exactly two ignored cases. Explicit main gate
   execution must run the source-hidden/read-only exported-bundle helper once,
   retain its strictly framed result and bounded proof, reject helper stdout/stderr
   panic markers, and verify all 13 software-GPU images
5. A source-pinned omitted-reorder negative, with unchanged oracles, must fail at
   the exact document-order assertion. Setup/compile/transport failure cannot pass
   this negative. Its original image precedes that assertion; no independently
   observed pixel-negative claim is made
6. Exact source restoration, fresh producer and complete inventories, full ELF
   size/SHA equality with the positive, complete archive readback, then the full
   GPU acceptance again

Six producer archives have streaming compressed-byte limits and complete gzip
readbacks. Restored uses the positive archive only after full ELF equality.
Retained fixture source masks, command results, PNG headers and complete file
hashes are rechecked at the end. The actual Rust acceptance also verifies image
pixels, geometry/actions, complete Frames and readonly/source-hidden export.

## Ownership and resources

The acceptance process runs unprivileged inside the existing hosted job. The
workflow performs the baseline ephemeral `sudo apt-get` provisioning of Mesa and
bubblewrap on Ubuntu 22.04. It creates no new privileged service or cgroup and
changes no namespace/security policy or persistent credential. Runner migration
must preserve actual namespace acceptance; missing capabilities fail rather than
skipping. Profile leaves, including the isolated XDG runtime directory, are created
with mode 0700 and checked for an ordinary-directory type, the owned profile UID
and exact permissions. This follows the private-directory requirement in the
[XDG specification](https://specifications.freedesktop.org/basedir/latest/).

A process-local child subreaper owns each serial command. WNOWAIT holds the direct
leader until the final exact-group signal. After reaping that leader, pidfds plus
kernel child-wait ownership fence every adopted descendant signal/reap, including
orphaned descendants that started another session. The command cannot complete
without kernel ECHILD and closed output pipes. Lost ownership, cleanup exceptions,
output overflow or timeout fail the lane; UNKNOWN prevents further commands and
source restoration. This new owner needs independent review and actual hosted
validation; pure controls alone do not prove OS behavior.

The job timeout, whole-gate/per-command deadlines, stream/sample limits, streaming
archives and fixture-owned retained-write/copy limits are explicit. The fixture
reserves retained bytes before writing and checks copies before every chunk.
Actual full ELF sizes determine the generator/exporter/test copies and two runtime
copies. Temporary and retained copies are both charged, and shared filesystems
receive one common free-space floor rather than being counted as two pools.
No source mask or image buffer is reduced to fit.

Free space, target/fixture allocations, host memory availability and observed
child max-RSS are sampled and retained. These are observations, not an aggregate
memory proof or a hard filesystem quota. External driver/profile/cache allocations
remain subject to the existing hosted job and monitored free-space floor. A floor
breach, OOM, missing evidence or resource exhaustion fails acceptance. We do not
claim prevention of arbitrary overshoot between observations. Infrastructure
changes to add stronger isolation require separate approval; no such privileged
service, cgroup or security-policy setup is included here.

The raw RGBA image is 4 MiB at 1024x1024. The 8 MiB retained-PNG allowance is not a
proved maximum encoder output size. The 13 positive, one order-negative and 13
restored retained images are budgeted separately from logs and six archives.
Native-window/physical-device acceptance remains outside this offscreen lane.

## Verification and attribution

Run `python3 tools/test_hud_layer_order_gate.py` for pure/mocked parser, inventory,
identity, workflow, archive and owner controls. They launch no Cargo, test binary
or namespace. The mandatory lane also runs `python3 tools/test_determinism_workflow.py`,
including synthetic Bash aggregate and fixture regression controls. The real gate is `python3 tools/hud_layer_order_gate.py --run` in the
mandatory hosted lane; it refuses accidental local execution.

The gate freshly tests the published head. Earlier scoped evidence is preserved
separately: historical Phase A head `00159a57e63a0eb7e2112016c4db4e47044a7b61`,
Phase B head `7127c43e4487f3dd67b676ec5ee25c910019fb0e`. Neither is relabeled as
execution of the new bounded fixture or this hosted gate.

Producer framing, full ELF/archive and output-attribution helpers were explicitly
transplanted from independently reviewed focused HUD controller sources. The
workflow structure comes from the authentic maintained baseline. No unmerged
PR181/169 feature or workflow union is used. The source preparation packet records
full helper origin hashes and AST-preservation checks.

The child-ownership API contract follows the Linux man-pages project's
[pidfd_open](https://raw.githubusercontent.com/mkerrisk/man-pages/master/man2/pidfd_open.2),
[pidfd_send_signal](https://raw.githubusercontent.com/mkerrisk/man-pages/master/man2/pidfd_send_signal.2),
[prctl](https://raw.githubusercontent.com/mkerrisk/man-pages/master/man2/prctl.2) and
[wait](https://raw.githubusercontent.com/mkerrisk/man-pages/master/man2/wait.2) documentation.
