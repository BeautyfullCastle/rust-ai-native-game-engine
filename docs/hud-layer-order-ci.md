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

## Producer finalization and allocation observations

After a successful Cargo command and the existing owned-process cleanup/ECHILD
proof, output attribution remains provisional until a bounded finalization helper
finishes. The helper admits only fresh, fully audited ELF outputs: selected test
executables, their exact allowed editor companion, or the three selected production
bins. An integration-test ELF has one link; a canonical bin has exactly its two
proven Cargo/compiler aliases. The complete alias set must match `nlink`; an
unknown link, including an external hardlink, fails. The auxiliary editor uses its
own producer full-hash baseline, never the selected test's archive hash.

The source-pinned helper runs through the unchanged owned `Runner`, using a pinned
resolved Python interpreter. It walks directories with descriptor-relative
`O_DIRECTORY|O_NOFOLLOW` opens and opens leaves read-only with `O_NOFOLLOW`,
`O_CLOEXEC` and `O_NONBLOCK`. Before reads or flushes it proves regular type,
owner, inode/device, mode, link count, size, mtime and ctime. It holds every alias
and parent descriptor through full pre/post SHA-256, endpoint/path checks and
close. The ELF header is checked within those same streamed reads. Each unique
inode receives one `os.fsync` API invocation, with no application-level retry.
CPython may internally retry an interrupted syscall; an exception reaching the
helper fails. Read, flush, identity, content or close errors fail the producer.

The finalizer allows only recorded allocation evolution on that proven closure
before committing the first post-finalization inventory. The parent checks the
receipt and exact metadata/alias closure without adding another ELF read pass;
every other provisional target entry and the deduplicated allocation total must
remain accounted for. Receipts are saved before state is committed. Later target
inventory, allocated-block equality, retained hashes and posthash checks remain
mandatory. A failed target fence never triggers a flush, retry or new baseline.

Per producer, the helper admits at most eight distinct inodes, sixteen aliases,
192 MiB per inode and 768 MiB combined logical bytes. Its two hash passes read at
most 1,536 MiB, using a 1 MiB buffer; each request/result is at most 64 KiB and each
relative path at most 512 UTF-8 bytes. The command deadline is 60 seconds within
the unchanged whole-gate limit and existing owner cleanup rules. Existing
producer/source/tool hashes and later retained-hash fences are separate inherited
work, outside this new two-pass budget. Empty Clippy ELF closure needs no helper.
A timeout or unclear cleanup fails; SIGKILL is not claimed to instantly end
uninterruptible kernel I/O. No produced ELF is executed by the helper.

On a later target mutation, preserve the original before/after/delta and failure.
Supplementary diagnosis can read at most two already-known ELF inodes/four aliases,
384 MiB total in one pass under a 30-second owned command. It compares the actual
retained producer SHA and records phase, ctime/allocation endpoints and a bounded
filesystem-type observation. It never calls fsync, admits a replacement inode or
changes acceptance. Missing identity, insufficient time, lost ownership or evidence
errors remain explicit supplementary failures; the original fence stays primary.
No full mount table, mount source or options are retained.

The original exact-head `7ebec9e754f043e333b50a685988c36ddbc037f1` failure preceded
`cpu-exact-0`, after inventory and CPU archive readback. Only allocated accounting
changed by 4,096 bytes on one inode exposed through the two editor aliases. The
old snapshots lacked content hashes and ctime for that inode, and its filesystem
and cause remain unproven. The CPU archive contains a different ELF. Successful
finalization does not explain that historical event or guarantee that future
`st_blocks` observations will never move. Any movement after the new baseline
still fails the exact fence; real exact-head mandatory CI remains required.

These contracts follow [Linux fsync(2)](https://man7.org/linux/man-pages/man2/fsync.2.html),
[Linux open(2)](https://man7.org/linux/man-pages/man2/open.2.html),
[Linux stat fields](https://man7.org/linux/man-pages/man3/stat.3type.html),
[Linux mountinfo](https://man7.org/linux/man-pages/man5/proc_pid_mountinfo.5.html),
and [CPython's fsync wrapper](https://github.com/python/cpython/blob/v3.12.15/Modules/posixmodule.c).
Linux fsync flushes file data and associated metadata, not by itself the containing
directory entry. A new descriptor does not establish that a closed producer never
had an earlier writeback error. This ordinary owned-file I/O creates no new
privilege, security policy, credentials, global sync operation or crash-durability
claim. Deferred allocation is only one possible explanation until measured;
[ext4 allocation documentation](https://docs.kernel.org/filesystems/ext4/allocators.html)
is not evidence that this particular hosted inode used that mechanism.
