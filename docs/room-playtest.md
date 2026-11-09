# Saved Room gameplay checks

Build `orr_playtest_room` with the default-off `room-playtest` feature. On Linux,
run `orr_playtest_room --project PATH --script route.json`. This reads an admitted
plain Room project and simulates its authored scene without opening a window or
writing project files. It does not load or save checkpoint progress.

A strict JSON script uses `schema: 1` and a nonempty `steps` array. Every step
requires six fields: `ticks`, `move_x`, `move_z`, `interact`, `expect_key`, and
`expect_won`. Axes are integers -1, 0, or 1; the last three fields are booleans.
Each step replaces the complete logical input for its duration, then compares
key possession and victory with the expected values. A zero-tick step checks
state without changing input history. Use a positive-duration `interact: false`
step to release interaction before a second press.

For the generated Room starter, this sequence completes the game:

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
format. It does not test physical keys, authored key bindings, presentation,
audio, exported launchers or native windows. The consumer admits plain Room
projects only; optional controls, animated models, lighting, character and progress declarations remain
unsupported even when dependencies have those features enabled. Validated camera and static model metadata are admitted but not rendered or exercised.
Output includes SHA-256 of the captured script bytes and admitted scene text, plus
the engine package version. These identify gameplay inputs, not the full project
or executable. Keep source and executable hashes with comparison results.
