# Offline CI timing reports

`tools/ci_cost_report.py` compares timing records that a caller has already
collected. It reads a versioned JSON document and emits JSON (the default) or
Markdown. It does not run CI, fetch artifacts, verify a checksum, or attest that
the declared source and measurements are genuine. Keep the original run and
artifact links with the report.

## Input

The top-level schema is `orr.ci-cost-input/1`. `records` contains one record
per run/job/attempt; retain separate attempts and repeats. `comparisons` names
baseline/current record IDs. Each record declares:

- `provenance`: exact lowercase 40-character `source_sha`, `checkout_sha`,
  `source_tree_sha`, and `checkout_tree_sha`; positive `run_id`, `run_attempt`,
  and `job_id`; and `artifact` with `name`, `locator`, and a 64-character
  lowercase `sha256`.
- `conditions`: `platform` (`os`, `arch`, `image`), `runner` (`class`,
  `hardware`, `isolation`), `toolchain`, `profile`, `target`, `features`, exact
  `command` argv, and `cache` (`state`, `key`). A comparable pair requires all
  compared condition fields to be known and equal; `runner.isolation` must be
  `exclusive`, and cache state must be `cold` or `warm`.
  These are the only supported keys at the `conditions` root and inside
  `platform`, `runner`, and `cache`. Any additional key is retained with an
  `unsupported condition field` record validation error and blocks every
  metric involving that record, even when both records supply the same extra
  value or `null`. For example, `conditions.env.RUSTFLAGS` is not validated by
  this version and cannot silently produce a comparable pair. Extending the
  condition contract requires explicit validation and comparison support.
- `status` and `exit_code`. A comparable timing requires `success` and exit
  code zero. Failed, cancelled, timed-out, and unknown outcomes remain in the
  report but do not produce ratios.
- `measurements.compile`, `.runtime`, and `.total`, each with `seconds`,
  `source`, and `scope`. Seconds may be `null` when unknown. Supported scopes
  are `cargo-build-wall` for compile, `test-execution-wall` for runtime, and
  `command-wall` or `job-wall` for total. Both sides of a metric must use the
  same scope and a positive, known duration. Never derive one duration by
  subtracting another from wall time.
  `source` is a nonempty evidence locator or description of the captured
  interval, such as `run/1001/logs/build.txt:42-60`; it is not a measurement
  method identifier. The two records normally refer to different evidence,
  so their source strings need not match. Unknown source values still block
  that metric. `scope` defines the caller-declared timing interval: Cargo
  build wall time, test execution wall time, or command/job wall time. Equal
  scopes declare the same interval semantics; they do not attest collector
  accuracy or equivalent measurement methods. Keep the actual collection
  method with the linked evidence and use matching interval semantics before
  supplying a pair; this offline parser does not verify that claim.

The source SHA may differ between baseline and current. Their runner,
platform, toolchain, profile, target, features, command, cache state/key, and
measurement scope must still match. The report marks each metric comparable
independently: missing provenance or mismatched conditions blocks ratios;
unknown duration or a mixed/unsupported scope blocks that metric. A pair can
therefore be `partially_comparable`. Missing metadata remains visible through
unknown comparison reasons; malformed typed details are retained with record
validation errors rather than silently discarded. Zero seconds
has no ratio. `change_seconds` is current minus baseline: positive means
slower, negative means faster, and zero means unchanged. The ratio is
baseline/current; values above one mean the current duration is lower. These
are paired observations, not a causal or statistical speedup claim.

An unknown, unsupported, or mixed measurement scope blocks only that metric;
other metrics can still be compared. A malformed non-string nested field (for
example, an object where the string `conditions.runner.class` is expected) adds a record
validation error and blocks all comparisons involving that record. The JSON
report's `records[].input` retains the parsed logical record contents, but it
does not preserve the original JSON bytes or every original numeric type:
fractional JSON numbers are parsed as exact decimals and emitted as decimal
strings in the echoed record, while integer values remain JSON numbers.

Within each run, `source_tree_sha` must equal `checkout_tree_sha`; commit IDs
may differ when the checked-out commit is a synthetic merge, so keep both SHA
fields and both tree fields. Comparisons also require distinct run, attempt,
or job identity, preventing the same capture from being compared with itself.

Example with two successful, synthetic Windows release captures:

```json
{
  "schema": "orr.ci-cost-input/1",
  "records": [
    {
      "id": "base-run",
      "provenance": {
        "source_sha": "1111111111111111111111111111111111111111",
        "checkout_sha": "1111111111111111111111111111111111111111",
        "source_tree_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "checkout_tree_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "run_id": 1001, "run_attempt": 1, "job_id": 2001,
        "artifact": {"name": "native-timings", "locator": "run/1001/artifacts/1", "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}
      },
      "conditions": {
        "platform": {"os": "windows", "arch": "x86_64", "image": "windows-2022"},
        "runner": {"class": "windows-2022", "hardware": "8-core", "isolation": "exclusive"},
        "toolchain": "rustc 1.97.1; cargo 1.97.1", "profile": "release",
        "target": "x86_64-pc-windows-msvc", "features": [],
        "command": ["cargo", "test", "--release", "--workspace"],
        "cache": {"state": "cold", "key": "windows-release-v1"}
      },
      "status": "success", "exit_code": 0,
      "measurements": {
        "compile": {"seconds": 123.4, "source": "cargo timing intervals", "scope": "cargo-build-wall"},
        "runtime": {"seconds": 59.1, "source": "test execution interval", "scope": "test-execution-wall"},
        "total": {"seconds": 201.7, "source": "job elapsed interval", "scope": "job-wall"}
      }
    },
    {
      "id": "current-run",
      "provenance": {
        "source_sha": "2222222222222222222222222222222222222222",
        "checkout_sha": "2222222222222222222222222222222222222222",
        "source_tree_sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "checkout_tree_sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "run_id": 1002, "run_attempt": 1, "job_id": 2002,
        "artifact": {"name": "native-timings", "locator": "run/1002/artifacts/1", "sha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"}
      },
      "conditions": {
        "platform": {"os": "windows", "arch": "x86_64", "image": "windows-2022"},
        "runner": {"class": "windows-2022", "hardware": "8-core", "isolation": "exclusive"},
        "toolchain": "rustc 1.97.1; cargo 1.97.1", "profile": "release",
        "target": "x86_64-pc-windows-msvc", "features": [],
        "command": ["cargo", "test", "--release", "--workspace"],
        "cache": {"state": "cold", "key": "windows-release-v1"}
      },
      "status": "success", "exit_code": 0,
      "measurements": {
        "compile": {"seconds": 118.2, "source": "cargo timing intervals", "scope": "cargo-build-wall"},
        "runtime": {"seconds": 57.8, "source": "test execution interval", "scope": "test-execution-wall"},
        "total": {"seconds": 193.5, "source": "job elapsed interval", "scope": "job-wall"}
      }
    }
  ],
  "comparisons": [{"baseline": "base-run", "current": "current-run"}]
}
```

The sample values are illustrative, independently declared measurements; the
reporter does not assume compile plus runtime equals total.

## Run it

The parser uses only Python's standard library and runs offline. From the
repository root:

```sh
python tools/ci_cost_report.py timing-input.json
python tools/ci_cost_report.py timing-input.json --format markdown
cat timing-input.json | python tools/ci_cost_report.py -
```

Input is limited to 2 MiB, 1,000 records, and 1,000 comparisons. Duplicate JSON
keys, non-finite numbers, invalid JSON, unknown top-level fields, and
comparisons referring to missing records are rejected with exit code 2. A
valid document with incomplete or incomparable measurements is still reported
with exit code 0 and no ratio for the affected metric. JSON output preserves
each input record, per-record validation errors, and comparison reasons.

This parser is stage 1 only. It does not supply real paired Windows/Linux
measurements or establish a CI speedup. Keep #9 open until its full acceptance
conditions, including platform-specific paired results or explicit access
limits, are handled.

## Opt-in local collection

N9-1 adds a separate local collector to the same tool. The offline v1 parser
above retains its input and comparison rules. A local capture has a local ID
and actual process provenance; it does not invent GitHub run, job, or artifact
IDs to satisfy the CI parser. Its local report omits ratios. Collection is
opt-in and requires a frozen plan and the coordinated resource lane; validating
a plan does not launch Cargo.

The selected workload is an entire existing Cargo command, including its
normal unit, integration, and documentation tests. For the current development
source, the recipes in `.github/workflows/determinism.yml` are:

```sh
cargo test --workspace --release --timings --exclude orr_sample --exclude orr_editor --exclude orr_web_gpu
cargo test -p orr_sample -p orr_view -p orr_bridge -p orr_rhi -p orr_render -p orr_editor -p orr_web_gpu --release --timings
```

These are selected native and sample commands, not the entire CI job. The
collector does not remove tests, change profiles, loosen assertions or test
timeouts, disable required platform/GPU/browser gates, or dispatch CI. It
records command wall time with a monotonic clock and keeps the original binary
stdout and stderr plus their sizes and SHA256 hashes. UTC start/end timestamps
and monotonic duration are separate evidence. Build and test execution time
remain `null`: `cargo test` schedules builds and suites together, and subtracting
Cargo HTML or a `Finished` duration from total would not establish an independent
test execution interval. Keep any Cargo timing artifacts as separate build
evidence; command wall time alone does not measure compiler CPU time or CI cost.

### Freeze the plan before collecting

1. Pin exact baseline/current commit and tree IDs, clean checkouts, the native
   or sample recipe, and an equivalent workload. Compare the relevant Cargo
   manifests, lockfile, source and fixtures as well as the test inventory. A
   changed suite is a workload difference, even if the command text is equal.
2. Record installed real Cargo/Rust executables and versions, OS/architecture,
   hardware, all supported build environment values, and cache preparation.
   Each capture uses its own target directory. A fresh target does not establish
   a cold registry, filesystem, or operating-system cache. Unknown cache inputs
   remain unknown; inherited dependency caches are not an optimization result.
3. Select the resource contract below and bind its actual receipt files by
   SHA256. On the same resource, N6-1 must finish before N9-1. Windows keeps
   this ordering. A separate Linux resource may use the explicit independent
   contract for its four captures. Read current #28 reservations and live
   processes immediately before taking the lane. A previously idle machine
   is not a future guarantee, and a coordinated local lane is not complete
   isolation from desktop services.
4. Declare one attempt per selected capture, at most six captures total, a
   finite watchdog and output bound, and the owned process-tree cleanup method.
   No automatic retries or cleanup of other owners' processes is permitted.
   Crossing the watchdog or log limit marks failure and stops later captures;
   it never automatically terminates Cargo or its descendants. Keep the lane
   held until the owned process naturally exits, streams reach EOF, and the
   owned Job/process-group census is empty. A timeout is not completed cleanup.
   Both pipe readers start before the child. Reader startup failure launches
   no command. A raw storage failure preserves the first failure and continues
   discard-draining to EOF; hashes cover only bytes actually saved. If Windows
   cannot resume a created child, the collector retains its process, thread,
   Job and pipe handles plus the pending lane receipt while awaiting external
   resolution and natural exit. A suspended child is never reported as cleanup.

Use the `orr.ci-cost-local-plan/1` schema documented by
`validate_local_plan` in the tool. `--validate-plan PLAN.json` validates its
structure without launching a process. `--collect PLAN.json --output NEW_DIR`
performs live preflight before executing the declared commands. The output
directory and capture markers are never reused. The first failure, timeout,
output limit, source change, or missing condition stops further captures and
preserves the outcome and raw evidence. Do not restart a completed collection
to obtain a more favorable observation.

### Same resource and independent Linux admission

The existing exact `coordination` object remains supported:

```json
{
  "lane_receipt": {"path": "/absolute/current-lane.json", "sha256": "actual-file-sha256"},
  "n6_receipt": {"path": "/absolute/n6-completion.json", "sha256": "actual-file-sha256"},
  "n6_completed": true
}
```

This is the same-resource contract. Receipt hashes bind saved bytes; N6
completion and lane ownership remain caller declarations backed by the linked
owner evidence. Collection rechecks these receipt hashes before each launch.
Do not set completion to true for a pending Windows N6 run.

For su's separate Linux runner, use this mutually exclusive exact object:

```json
{
  "mode": "independent-linux",
  "lane_receipt": {"path": "/absolute/linux-admission.json", "sha256": "actual-file-sha256"},
  "resource_receipt": {"path": "/absolute/linux-resource.json", "sha256": "actual-file-sha256"},
  "n6_receipt": null,
  "n6_completed": false
}
```

The SHA strings in these examples are placeholders, not executable plans.
The independent variant requires actual Linux OS and architecture, the
installed toolchain, POSIX process-group cleanup, and exactly four captures:
one baseline/current native pair sharing a lane and one baseline/current
sample pair sharing a different lane. It does not admit Windows or add slots
to the six-capture campaign. Keep a single campaign ledger for Linux four and
Windows two; the per-plan collector cannot coordinate counters across separate
machines. A resource receipt has exactly these keys:

```json
{
  "schema": "orr.ci-cost-independent-resource/1",
  "owner": "su",
  "runner_id": "su-linux-n9",
  "n6_runner_id": "ddang-windows-n6",
  "runner_machine_id_sha256": "actual-machine-id-file-sha256",
  "runner_os": "Linux",
  "runner_arch": "actual-platform.machine-value",
  "separate_resources": true
}
```

The independent resource owner must be `su`. The collector checks distinct
resource labels (ignoring case) and matching actual OS,
architecture and SHA256 of the raw `/etc/machine-id` file. Its stripped content
must be a valid 32-hex machine ID, excluding all-zero and all-f placeholders.
This identifies an OS installation;
it does not prove separate physical hardware, CPU allocation or isolation.
The owner attests the association with the N6 resource label. Receipt hashes
are mismatch detectors, not issuer authentication. Obtain the owner's actual
separate-resource evidence instead of manufacturing a completion receipt.

The lane receipt has exactly `schema` (`orr.ci-cost-local-admission/1`),
`runner_id`, `observed_at_utc`, `quiet` (literal `true`), and
`active_related_pids` (an empty list). Its runner ID must match the resource
receipt. The aware UTC observation must be between zero and 120 seconds old
at admission; a future observation is refused. Both receipt files are read,
hashed and parsed once at admission, and their bound snapshots are retained.
This timestamp is not reused as evidence of fresh quiet conditions after a
long capture.

Immediately before each independent Linux launch, the collector obtains a
new `ps` PID/executable-name census and refuses malformed, duplicate or related
build/engine process records. The report keeps the observation UTC, collector
PID, process count, census text hash and related-PID result without argv or
environment contents. The hash covers the metadata probe's normalized text,
not original subprocess bytes. Executable names can be truncated or changed;
this point-in-time check cannot establish whole-machine isolation or future
quiet. Existing owned-process natural exit, stream EOF and process-group
cleanup checks remain necessary before the next capture and lane return.

### N9-1 revision and access boundaries

The starting development revision is
`ef61e959306e71b186b9355f237b458392be073f`. The historical optimization pair
`d54f26ddfdb37b6d5b6303fa9824611706cc6b10` to
`7047f412c9e17dceda58235b6a4df9fc4077891e` also adds guard tests. It is not an
equivalent full-suite pair merely because it includes empty-harness removal.
An old source revision versus the current engine also includes many added
features and tests. Neither difference establishes an optimization speedup.

A controlled current-source pair can use EF as baseline and a collector-only
revision as current, after proving that Cargo/workflow/runtime inputs and suite
inventory are identical. Such a pair measures repeated current workload cost;
it does not measure the benefit of the historical optimization. Record that
distinction and retain unchanged or slower observations.

The six-slot plan consists of baseline/current Windows native, Linux native,
and Linux sample. On 2026-10-05, the available local environment is Windows 11
build 26200, i7-14700 (20 cores/28 logical processors), with installed
Rust/Cargo 1.97.1 on `x86_64-pc-windows-msvc`. The read-only WSL inventory reports
that WSL is not installed. The Linux native/sample execution owner is su;
the collector's three source files remain owned by Haneul. Su must verify the
Linux environment, installed toolchain and frozen four-capture plan before
execution. Those results are pending and cannot be inferred from Windows.
Do not install WSL, buy resources,
add credentials, trigger CI manually, replace those slots with Windows runs,
or count old completed CI as new paired captures. Linux/native/GPU results
cannot be inferred from the Windows environment.

Collector validation and static source preparation may proceed while N6-1
owns the heavy lane. Neither a plan, a synthetic child regression, nor a local
collector report closes #9 or establishes full Windows/Linux paired results.
