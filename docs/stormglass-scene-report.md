# Inspect a Stormglass scene without a window

The optional `coop_defense_scene_report` example validates authored scene YAML
through the same strict native `parse_config` used by game consumers. It prints
normalized settings and the initial deterministic CPU Frame identity as JSON.
This works without starting an editor, native player, Node or WASM. It opens no
files, performs no network requests and writes no project or export files.

## Use

```sh
cargo build --offline --locked -p orr_games --features coop-defense-scene --example coop_defense_scene_report
# Bash or cmd.exe: redirect the authored scene into the built executable.
target/debug/examples/coop_defense_scene_report --stdin < my.scene.yaml
```

In PowerShell 7+, pipe UTF-8 text explicitly (Windows PowerShell 5.1 uses a
different native-pipe encoding and is not covered by this example):

```powershell
Get-Content -Raw -Encoding utf8 .\my.scene.yaml | .\target\debug\examples\coop_defense_scene_report.exe --stdin
```

Only `--stdin` or `--help` is accepted. Standard input is limited to 64 KiB of
UTF-8; the reader consumes at most one additional byte to detect oversize input.
Reading a pipe still waits for its producer. Bound or close the producer if it
might stall; this tool does not impose a wall-clock input deadline. Shell text
pipelines may change line endings, so `source_bytes` describes received bytes,
not the original file's size or identity.

Successful inspection emits one `orr.coop.scene-report/1` JSON object containing:
- `admitted`, `source_bytes`, `tick_rate`, and `initial_tick` (zero).
- `build_id` and `initial_checksum` as hexadecimal strings.
- All six normalized authored singleton settings, plus the fixed default seed.
  Counts and composition retain all ten waves, including currently inactive ones.

Missing singletons receive their independent production defaults. Present fields
must satisfy the existing strict schema, exact array sizes and bounds. Unknown
entities, runtime state, prefabs, duplicate fields and extra documents remain
rejected. No admission policy is relaxed.

Exit 0 means admission or help. Usage errors exit 2. Input/admission/output errors
exit 1 and emit a fixed JSON error on stderr: `usage`, `input_read`,
`source_too_large`, `invalid_utf8`, `invalid_scene` or `output_write`. Errors omit
raw YAML/parser text, paths and input values. `invalid_scene` means consult the
[authoring contract](stormglass-authoring.md); it is not a field-level diagnostic.

The checksum identifies initial CPU state under this build, not source bytes,
a signature, runtime trust, later gameplay results or export integrity. The tool
performs no gameplay ticks and does not prove victory, native rendering or Player
portability. A consumer must still validate the actual bytes it later receives.

## Verify

Run `cargo test --offline --locked -p orr_games --features coop-defense-scene
--example coop_defense_scene_report`. Then set `ORR_SCENE_REPORT_BIN` to the
absolute freshly built executable and run
`python tools/test_coop_defense_scene_report.py`. The actual CLI suite requires
that executable; missing setup fails instead of silently skipping.
