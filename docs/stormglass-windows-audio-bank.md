# Windows Stormglass audio banks

The Windows `coop-defense-audio` consumer can load the existing version-one bank.
Bank authoring remains the existing Linux CLI workflow: copy the published directory
to Windows without altering its manifest or ten PCM objects. No new schema, codec,
resampling, content or asset rights are introduced.

Build and launch from PowerShell:

```powershell
cargo build --release -p orr_sample --features coop-defense-audio-settings,audio-native --bin coop_defense
.\target\release\coop_defense.exe --audio-bank C:\Games\StormglassBank --audio auto --audio-settings-directory C:\Private\StormglassAudio
```

The settings directory is optional and follows the separate
[private settings contract](stormglass-audio-settings.md). Keep it outside the bank.
The game starts muted unless an explicitly saved preference restores otherwise;
Start, pause and focus gates still suppress audio. Bank loading happens before
window, GPU or output creation. `--audio off` still validates an explicitly supplied
bank and never opens an output device. `auto` may continue silently if output is
unavailable; `required` reports output failure and is rejected in headless mode.

Use an existing non-root, absolute drive path of at most 240 UTF-8 bytes on a fixed
local NTFS volume. UNC/device/drive-relative paths, alternate streams, reserved DOS
names, dot components, trailing dots/spaces and reparse ancestry are rejected.
Only `bank.json` (128 KiB maximum) and the ten fixed cue `.pcm` names (96008 bytes
each maximum) are opened. Manifest file paths never select filesystem reads.
Inputs must be regular single-link files without reparse points; copy hardlinked
assets into independent files before loading. Read-only assets are supported.
No private ACL is required for banks because the loader only reads assets.

The drive root is opened once, converted to a stable volume GUID path, then all
ancestry traversal and file reads use that volume path. Directory-list handles
reserve against ancestor rename, while input handles deny write/delete sharing.
All eleven input handles stay live through admission. Snapshot attributes and
actual bounded read lengths are checked. Sharing conflicts fail without waiting
for an exclusive writer to close. The loader creates no directory, lock or file.
See Microsoft's [volume GUID path API](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfinalpathnamebyhandlew)
and [file sharing contract](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilea).

The resulting bytes enter exactly the existing pure admission: strict JSON,
fixed cue/asset identities, 48 kHz mono PCM16, frame/byte counts, SHA-256, peak and
aggregate memory budgets. Linux admission and its filesystem implementation remain
unchanged. This is bounded admission using ordinary Windows sharing reservations,
not a sandbox against privileged filesystem/kernel interference.

For a clean committed checkout on the connected Windows PC, run
`tools/check_stormglass_windows_bank.ps1 -EvidenceDirectory C:\Evidence\fresh-bank-run`
with the normal current-user token. The gate requires exact source/tree and test
counts, preserves failures, exercises actual NTFS inputs and offline Kira/App,
and compile-checks native output. Restricted tokens can refuse the junction/rename
test setup; that is recorded separately. The Linux focused CI checks the same
candidate's shared admission and existing audio/settings paths.

Automatic filesystem/CPU tests and native-feature compilation do not establish
native-window gameplay, successful device opening or audible hardware playback.
