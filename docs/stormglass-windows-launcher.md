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
launcher never creates them, chooses an environment/cwd fallback, or rewrites
the supplied paths. Runtime checks remain authoritative for NTFS ownership/ACLs,
reparse points, aliases, forbidden roots and atomic persistence. Use the existing
[controls](stormglass-controls.md) and [audio settings](stormglass-audio-settings.md)
guides for provisioning and Save behavior. There is no playback before Start;
saved gain/mute follow the existing runtime rules. Moving the game folder does
not move user data.

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

The 62 contract assertions use an explicitly named Rust **probe**, not the game.
They cover both PowerShell versions, literal argument boundaries, special paths,
relocation, settings forwarding, refusals, exit/stderr and failed process startup.
Focused CI records the exact source head and these contracts on Windows.

The separate runtime smoke uses the actual built executable, a minimal scene and
an original synthetic two-frame test bank. It compares copied and relocated
60-tick summaries with direct execution, including a shared external bank after
game relocation. Missing/invalid external banks fail with Off even when the
sibling bank is valid; normal input inventory/hashes remain unchanged. It does
not validate the Linux authoring CLI, settings UI/persistence,
interactive rendering, device playback or a full distributable. Actual playtests
need separate video evidence. Earlier blocked window-control evidence is retained;
this task makes no new window-control attempt.
