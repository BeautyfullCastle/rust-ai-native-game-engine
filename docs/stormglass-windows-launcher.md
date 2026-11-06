# Stormglass Windows folder launcher

`tools/stormglass_windows/run-game.ps1` launches an already trusted Windows game
folder from any working directory. This is a launcher template, not a Windows
package publisher, installer, signature verifier or sandbox. The existing
`pack-coop-native` publisher still targets Linux ELF/shell bundles only.

Build the desired runtime from the same source revision as your game:

```powershell
cargo build --locked -p orr_sample --features coop-defense-controls,audio-native --bin coop_defense
```

Manually copy `target/debug/coop_defense.exe`, your admitted scene as
`game.scene.yaml`, and the launcher into a new trusted folder. Optional presentation
features/assets must be built and supplied according to their existing guides;
this template does not collect assets or DLL dependencies automatically.

```text
My Game/
  coop_defense.exe
  game.scene.yaml
  run-game.ps1
  orr_prepare_stormglass_settings.exe  optional explicit first-use helper
  audio-bank/          optional existing validated fixed bank
    bank.json
    attack-light.pcm   and the other nine fixed cue files
```

Use Windows PowerShell 5.1 or PowerShell 7 with the normal machine execution policy:

```powershell
& 'C:\Games\My Game\run-game.ps1'
& 'C:\Games\My Game\run-game.ps1' -Audio auto
& 'C:\Games\My Game\run-game.ps1' -Headless -Ticks 60 -WithAudioBank
```

Default audio is `off`. Without an external selection, `-Audio auto|required`
selects the fixed `audio-bank` subfolder; `-WithAudioBank` also selects it while
audio is Off. To share an existing bank outside the game folder, supply an
explicit absolute local drive path:

```powershell
& 'C:\Games\My Game\run-game.ps1' -Audio auto -AudioBankDirectory 'C:\GameAudio\My Bank'
& 'C:\Games\My Game\run-game.ps1' -Headless -Ticks 60 -AudioBankDirectory 'C:\GameAudio\My Bank'
```

`-AudioBankDirectory` selects that literal path even with Off and cannot be
combined with `-WithAudioBank`. It never creates/copies a bank or falls back to
the sibling bank; moving the game folder leaves this explicit selection
unchanged. Native Windows NTFS/no-reparse bank admission remains authoritative;
the launcher only rejects non-absolute, UNC and drive-relative selections early.
Missing or invalid
selected banks fail native admission even with Off. Headless mode never opens
an audio device or window and rejects Required audio. Ticks are decimal 0..36000
and require Headless. Unknown/duplicate options fail. The launcher preserves the
native exit status and stderr, including zero exit with diagnostic stderr.

Settings are opt-in and interactive only:

```powershell
& 'C:\Games\My Game\run-game.ps1' `
  -ControlsDirectory 'C:\Users\me\PrivateControls' `
  -AudioSettingsDirectory 'C:\Users\me\PrivateAudio'
```

These must be existing private local directories outside the game folder. The
launcher never creates them unless explicitly preparing a new shared directory
as described below. It never chooses an environment/cwd fallback or rewrites
the supplied paths. Runtime checks remain authoritative for NTFS ownership/ACLs,
reparse points, aliases, forbidden roots and atomic persistence. Use the existing
[controls](stormglass-controls.md) and [audio settings](stormglass-audio-settings.md)
guides for provisioning and Save behavior. There is no playback before Start;
saved gain/mute follow the existing runtime rules. Moving the game folder does
not move user data.

To provision a **new** shared controls/audio directory without editing ACLs in
Explorer, build the optional helper and copy its executable beside the launcher:

```powershell
cargo build --locked -p orr_sample --features coop-defense-controls --bin orr_prepare_stormglass_settings
& 'C:\Games\My Game\orr_prepare_stormglass_settings.exe' --directory 'C:\Users\me\StormglassSettings'
& 'C:\Games\My Game\run-game.ps1' -ControlsDirectory 'C:\Users\me\StormglassSettings' -AudioSettingsDirectory 'C:\Users\me\StormglassSettings'
```

Use an existing parent directory and a new final name on a fixed local NTFS drive.
The helper creates exactly one empty directory with current-user ownership and
a protected ACL allowing only that user, SYSTEM and Administrators. Ancestors
are checked without following reparse points and pinned against rename/deletion
while creation uses the opened volume's GUID. Source/executable roots are refused.
There is no cwd/environment fallback, recursive parent creation, settings-file
write, implicit invocation, or modification of existing ACLs/data. If the name
already exists, choose a new dedicated path or inspect it manually; the helper
does not repair permissions. A rare post-creation verification failure explicitly
reports that a directory was created and leaves it for inspection, without
automatic deletion or retry. Runtime admission at launch remains authoritative,
including the selected scene/audio-bank exclusions unknown to the helper.
The helper supports Windows only and opens no game window or audio device.

For one shared controls/audio directory, use `-UserSettingsDirectory` instead of
the two individual directory options. It cannot be combined with either one and
is interactive-only. Without the preparation switch, this only forwards the
literal directory to both existing native stores; it never creates or repairs it.

```powershell
# Existing private settings:
& 'C:\Games\My Game\run-game.ps1' -UserSettingsDirectory 'C:\Users\me\StormglassSettings'
# Explicit first use: prepare a NEW directory, then launch with both settings paths:
& 'C:\Games\My Game\run-game.ps1' -UserSettingsDirectory 'C:\Users\me\NewStormglassSettings' -PrepareUserSettings
```

`-PrepareUserSettings` requires the fixed sibling preparation executable and the
shared directory option. Runtime/scene and option checks happen first. Helper
failure preserves its stderr/exit status and stops before starting the game;
existing directories are refused. Successful preparation is followed by normal
native admission and launch with both settings options. If the game subsequently
fails, the new directory remains for inspection and the game's exit status is
returned; there is no cleanup or automatic retry. Runtime scene/bank/privacy
admission can still refuse a prepared path. Build the game with
`coop-defense-controls` to support both settings options. Preferences are only
saved using **SAVE** in KEYS or **SAVE AUDIO**; preparation creates no preferences.

Fixed runtime/scene reparse points are refused. This convenience check is not a
security boundary against modification of a trusted folder; native scene/bank
admission remains unchanged. External scene/capture overrides and arbitrary
command strings are intentionally not part of this template.

## Focused verification

Run from PowerShell 7 in a normal Windows user session, with Rust available:

```powershell
& ./tools/test_stormglass_windows_launcher.ps1 -EvidenceDirectory C:\Temp\fresh-contracts
& ./tools/test_stormglass_windows_launcher_runtime.ps1 `
  -Runtime "$PWD/target/debug/coop_defense.exe" -EvidenceDirectory C:\Temp\fresh-runtime
```

The 88 contract assertions use explicitly named Rust **probes**, not the game or
real directory creator.
They cover both PowerShell versions, literal argument boundaries, special paths,
relocation, settings forwarding, refusals, exit/stderr and failed process startup.
Focused CI records the exact source head and these contracts on Windows.

The separate `tools/test_stormglass_settings_launch_windows.ps1` accepts
`-PreparationHelper ABSOLUTE_HELPER_EXE -EvidenceDirectory FRESH_DIRECTORY`.
It uses the actual native preparation executable with a game argv/exit probe:
PS7/5.1 private creation and routing, existing ACL/data preservation, no implicit
creation, helper/input/start failures stopping the game, and retention after game
failure. This filesystem bridge check is not native window or UI-save acceptance.

The separate runtime smoke uses the actual built executable, a minimal scene and
an original synthetic two-frame test bank. It compares copied and relocated
60-tick summaries with direct execution, including a shared external bank after
game relocation. Missing/invalid external banks fail with Off even when the
sibling bank is valid; normal input inventory/hashes remain unchanged. It does
not validate the Linux authoring CLI, settings UI/persistence,
interactive rendering, device playback or a full distributable. Actual playtests
need separate video evidence. Earlier blocked window-control evidence is retained;
this task makes no new window-control attempt.
