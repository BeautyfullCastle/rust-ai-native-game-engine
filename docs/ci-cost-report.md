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
