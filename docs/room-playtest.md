# Saved Room gameplay checks

Build `orr_playtest_room` with `room-playtest`. On Linux, run `orr_playtest_room --project PATH --script route.json`. This reads an admitted
plain Room project and simulates its authored scene without windows or project writes. It does not load or save checkpoint progress.

A JSON script uses `schema: 1` and a nonempty `steps` array. Every step
requires six fields: `ticks`, `move_x`, `move_z`, `interact`, `expect_key`, and
`expect_won`. Axes are integers -1, 0, or 1; the last three fields are booleans.
Each step replaces logical input for its duration, then compares
key possession and victory with the expected values. A zero-tick step checks
state without changing input history. Use a positive-duration `interact: false`
step to release interaction before a second press.

The generated Room starter accepts:

```json
{"schema":1,"steps":[
  {"ticks":1,"move_x":0,"move_z":0,"interact":true,"expect_key":true,"expect_won":false},
  {"ticks":60,"move_x":1,"move_z":0,"interact":false,"expect_key":true,"expect_won":false},
  {"ticks":1,"move_x":0,"move_z":0,"interact":true,"expect_key":true,"expect_won":true}
]}
```

Limits are 64 KiB, 256 steps and 6000 total ticks. Unknown/duplicate fields,
missing fields, invalid axes, script-leaf symlinks and nonregular script files are rejected.
The script is fully validated before project admission or simulation. Paths may
contain Unicode. Each successful boundary prints its tick, key/win state and
Frame checksum; failure identifies the first mismatching step and returns exit
1. Bad CLI usage returns 2; only all satisfied assertions return 0. A successful
script need not end in victory unless it explicitly expects victory.

This is a repeatable CPU gameplay check, not an automatic solver or a recording
format. Schema 1 does not test physical keys, authored key bindings, presentation,
audio, exported launchers or native windows. Schema-1 scripts admit plain Room
projects only; optional controls, animated models, lighting, character and progress declarations remain
unsupported even when dependencies have those features enabled. Validated camera and static model metadata are admitted but not rendered or exercised.
Output includes SHA-256 of the captured script bytes and admitted scene text, plus
the engine package version. These identify gameplay inputs, not the full project
or executable. Keep source and executable hashes with comparison results.

## Structured results for authoring

Append `--json` to the command to emit one schema-1 JSON object on stdout, with
no progress text or stderr diagnostics. Exit codes remain 0 (all assertions
passed), 1 (execution/admission/assertion failure), and 2 (usage). An exact
`--json` anywhere in the arguments requests JSON errors, even for malformed
arguments; valid syntax requires it once, last. `--help` alone stays text;
`--help --json` is a usage error.

The report has `schema`, `status` (`passed` or `error`) and `steps`. Each executed
boundary contains a **one-based** `step`, cumulative simulation `tick` (initial
state is tick 0), `expected` and `observed` key/won booleans, hexadecimal string
`checksum` and `passed`. It includes the first failing boundary and stops there;
later boundaries are absent. Zero-duration steps retain the current tick. Only
successful runs include total `ticks`. Admission errors have empty `steps`, not
invented expectations or observations.

`script_sha256` appears only after complete script validation; `scene_sha256`
and `initial_checksum` appear only after project admission and simulation
construction. Hashes keep their limited meaning described above. `engine_version`
is included for execution reports. Errors include `kind` (`usage`, `script`,
`project`, `unsupported_project`, `simulation`, or `unsupported_target`) and a
human-readable `message`. Assertion failures use `kind: "assertion"`. Branch on
`kind` and structured state, not message wording. Execution error messages are
capped at 2048 UTF-8 bytes with `message_truncated`; usage messages are fixed.
Reports are bounded below 256 KiB by the script-step and diagnostic limits.

## Check authored controls with supplied key identities

Build with both `room-playtest,room-input-actions` and use a **schema-2** script
for a project with admitted authored controls. Each step requires `ticks`,
`held`, `expect_key` and `expect_won`. For example, after assigning Space to
interact and L to right:

```json
{"schema":2,"steps":[
  {"ticks":1,"held":["space"],"expect_key":true,"expect_won":false},
  {"ticks":60,"held":["l"],"expect_key":true,"expect_won":false},
  {"ticks":1,"held":["space"],"expect_key":true,"expect_won":true}
]}
```

`held` replaces the entire held set for that step: at most eight distinct
snake-case engine key names, such as `a`, `space`, or `arrow_right`. Unknown,
duplicate, R and Escape keys are rejected. Empty `held` is neutral. Unbound keys
are valid but have no action. Aliases do not multiply movement and opposing
movement actions cancel. A zero-tick boundary does not release interaction;
release for at least one simulation tick before pressing again. All existing
byte, step, tick, file and project-admission limits remain.

This is CPU translation of **supplied physical-key identities**, not actual
hardware input, window dispatch or focus handling. Schema 2 requires the saved
project's controls; it never substitutes defaults. Schema 1 retains its logical
input contract and still refuses projects declaring controls. Other optional
project capabilities remain unsupported.

After a valid schema-2 script is parsed, JSON reports use `schema: 2`, including
project errors. Once the project and simulation are admitted, reports include
`controls_sha256` of the captured, validated map bytes and
`input_mode: "supplied_physical_key_identities"`. Every executed step adds
`input` with resolved `move_x`, `move_z` and `interact`. A zero-tick boundary
reports the requested translation without consuming it. Parsing/usage errors
retain schema-1 error envelopes because no valid script has been accepted.
Map hashes identify control bytes only. Each invocation admits current saved files afresh.
