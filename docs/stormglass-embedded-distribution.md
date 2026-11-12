# Stormglass embedded EXE distribution

## Contract

`orr_publish_stormglass` publishes one already-built Windows x64 Stormglass EXE.
The opt-in API is `orr_package::stormglass_embedded_distribution::publish`.
The default `orr`/`orr_pkg` commands and ordinary `coop_defense` are unchanged.
This is a bounded local distribution step, not an automatic compiler, installer,
code sandbox, release service, or clean-machine compatibility certification.

Build the authored runtime explicitly:

```text
cargo build --locked --offline --release -p orr_sample --features coop-defense-embedded-prototype --bin stormglass_embedded
cargo build --locked --offline --release -p orr_package --features stormglass-embedded-distribution --bin orr_publish_stormglass
```

The exact authored source is `crates/orr_sample/assets/stormglass-embedded.scene.yaml`.
Edit it and rebuild the runtime to author a different game. The runtime embeds
that source, uses the shared strict 64 KiB parser, and refuses external `--scene`
overrides. `--embedded-scene-sha256` reports its strictly admitted source hash.
Only the minimal embedded build above is in this contract; extra optional assets,
audio banks, settings, saves, feature combinations, and arbitrary game EXEs are not.

## Usage and trust

Use existing trusted local directories and independently verify the executable's
origin. Obtain its SHA256 from that trusted build. A hash supplied by an unknown
sender is not authentication. `--trusted-runtime` acknowledges the executable,
its dependencies, and all input/output directories. Windows may load DLLs from
the EXE directory and system; neither hashing nor the Job object prevents unsafe
code from acting with the caller's privileges. Do not publish unknown downloads.

```text
orr_publish_stormglass --runtime C:\trusted\stormglass_embedded.exe --runtime-sha256 HEX64 --scene C:\trusted\game.scene.yaml --output C:\delivered\my-game.exe --dry-run
orr_publish_stormglass --runtime C:\trusted\stormglass_embedded.exe --runtime-sha256 HEX64 --scene C:\trusted\game.scene.yaml --output C:\delivered\my-game.exe --trusted-runtime
```

Replace `HEX64` with the independently checked executable hash. The scene must be
an exact byte copy of the embedded source, including comments, whitespace and line endings.
All three paths must be distinct, canonical ASCII absolute drive paths with
backslashes, non-root existing parents, and at most 240 bytes. Components allow
letters, digits, dot, underscore and hyphen, at most 150 bytes. No spaces,
Unicode, DOS device names, short aliases, streams, dot segments, UNC, or reparse
components. Files must be ordinary, single-link, and unchanged; runtime limit
is 128 MiB. The filesystem must be fixed local NTFS. These deliberate limits
keep this first distribution path narrow.

Dry-run validates arguments, captures held inputs, checks the runtime hash/PE
header, strictly parses the scene and checks destination absence. It creates
nothing and executes nothing. Its JSON marks runtime checks **unverified**.
Publishing copies to a create-new retained temporary EXE in the output parent,
then compares complete source/artifact bytes and hash before execution. Held
READ|DELETE, SHARE_READ artifact protection and source/ancestor reservations
span the three checks and native same-parent no-replace rename:

1. Exact embedded-source SHA256.
2. Exact independently simulated 60-idle-tick stdout, empty stderr and exit 0.
3. Exact external-scene refusal with exit 1.

Each child starts suspended, joins a non-breakaway kill-on-close Job, inherits
only standard pipes and OS-derived SystemRoot/WINDIR, and has a 15-second
polling budget plus five seconds for confirmed child/Job cleanup. Combined
stdout/stderr is capped at 64 KiB. Synchronous OS calls are outside these budgets.
No publication follows a failed check or unconfirmed cleanup. Existing outputs
are never overwritten, including a late collision at rename.

## Results and limits

Success prints one JSON receipt to stdout. Errors print one JSON object to stderr
(exit 2 usage, 1 operation). Failed temporary artifacts are retained and reported,
not recursively deleted. A post-rename verification error explicitly reports that
publication may have occurred; inspect the reported paths before retrying.

The delivered EXE has no scene sidecar or extraction launcher. Windows/MSVC/UCRT
and graphics dependencies remain. Reservations do not defend against privileged
writers or pre-existing writable mappings, and creation/open transitions assume
cooperating directories. Headless checks do not prove native input, GPU rendering,
or clean-machine deployment. Mandatory Linux-local checks and independent review
of the exact integration candidate remain separate gates.

Windows regression tests retain evidence. If the host's default temporary path
violates these restrictions, explicitly set `ORR_STORMGLASS_TEST_DIRECTORY` to
an existing canonical ASCII directory, with backslashes. This does not change
global TEMP. Permission-denied link fixtures remain failed/blocked; do not change
permissions, move them elsewhere, or count focused checks as a full pass.
