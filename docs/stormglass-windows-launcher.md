# Stormglass Windows folder launcher

`tools/stormglass_windows/run-game.ps1` launches a trusted Windows game folder
from any working directory using Windows PowerShell 5.1 or PowerShell 7 and the
normal machine execution policy. It is a launcher template, not an installer,
signature verifier, sandbox or dependency collector. `pack-coop-native` remains
Linux-only. Build from the same revision as your game:

```powershell
cargo build --locked -p orr_sample --features coop-defense-controls,audio-native --bin coop_defense
cargo build --locked -p orr_sample --features coop-defense-controls --bin orr_prepare_stormglass_settings
```

Copy the desired executables, launcher, admitted scene and optional presentation
assets into a trusted folder. The preparation helper is optional unless requested.
Additional presentation features/assets follow their existing guides.

```text
My Game/
  coop_defense.exe
  game.scene.yaml
  run-game.ps1
  orr_prepare_stormglass_settings.exe  optional
  audio-bank/                        optional fixed validated bank
```

## Launch and audio

```powershell
& 'C:\Games\My Game\run-game.ps1'
& 'C:\Games\My Game\run-game.ps1' -Audio auto
& 'C:\Games\My Game\run-game.ps1' -Headless -Ticks 60 -WithAudioBank
& 'C:\Games\My Game\run-game.ps1' -Audio auto -AudioBankDirectory 'C:\GameAudio\My Bank'
```

Default audio is `off`. Without an external selection, `-Audio auto|required`
selects the sibling `audio-bank`; `-WithAudioBank` selects it even with Off.
`-AudioBankDirectory` selects a literal absolute local-drive path and conflicts
with `-WithAudioBank`. Missing/invalid selected banks fail native admission even
with Off; there is no creation, copying, normalization or sibling fallback.
Native Windows NTFS/no-reparse checks remain authoritative. Headless opens no
window/device, rejects Required audio and settings, and accepts decimal ticks
0..36000. Ticks require Headless. Unknown/duplicate options fail. Native stderr
and exit status are preserved, including diagnostic stderr with a zero exit.

## Persistent controls and audio

```powershell
# Explicit first use: one NEW private directory, then launch:
& 'C:\Games\My Game\run-game.ps1' -UserSettingsDirectory 'C:\Users\me\NewStormglassSettings' -PrepareUserSettings
# Later launches: reuse it without the preparation switch:
& 'C:\Games\My Game\run-game.ps1' -UserSettingsDirectory 'C:\Users\me\NewStormglassSettings'
```

`-UserSettingsDirectory` forwards one literal directory to both native stores.
It conflicts with individual `-ControlsDirectory`/`-AudioSettingsDirectory`, which
remain available for separate paths. Settings are interactive-only; the shared
option requires `coop-defense-controls`. Ordinary launch never prepares,
repairs, rewrites or chooses a cwd/environment fallback directory.

`-PrepareUserSettings` requires the shared option and fixed sibling helper.
Option/runtime/scene checks happen first. The helper creates exactly one new
empty private directory on fixed local NTFS; parents must exist. Existing names
are refused without ACL/data modification. Ancestors are pinned without reparse
traversal and private owner/DACL apply at creation. See the
[setup guide](stormglass-settings-setup.md) for standalone usage and filesystem
bounds, including post-creation verification failures left for inspection.

Helper failure preserves stderr/exit status and stops before the game. Success
continues with normal native admission. If admission/game fails, the created
directory remains, the failure is reported, and there is no rollback/retry.
Runtime still checks privacy, aliases and source/executable/scene/bank exclusions;
the helper cannot prevalidate all game inputs. Keep settings outside those roots.
Moving the game folder never moves user settings or an explicit external bank.

Preparation writes no preferences. Save explicitly in **KEYS → SAVE** or
**SAVE AUDIO**; see [controls](stormglass-controls.md) and
[audio settings](stormglass-audio-settings.md). No playback happens before Start;
saved gain/mute retain the existing runtime gates. Fixed runtime/scene/helper
reparse points are refused, but trusted-folder modification is not sandboxed.
External scene/capture overrides and command-string evaluation are unsupported.

## Focused verification

Run tests from PowerShell 7 with fresh evidence directories and Rust available:

```powershell
& ./tools/test_stormglass_windows_launcher.ps1 -EvidenceDirectory C:\Evidence\fresh-contracts
& ./tools/test_stormglass_settings_launch_windows.ps1 -PreparationHelper "$PWD/target/debug/orr_prepare_stormglass_settings.exe" -EvidenceDirectory C:\Evidence\fresh-bridge
```

The 88 hosted/local contracts use game/helper argv/exit **probes**, not real
preparation or gameplay. The separate 16-case bridge test uses the actual native
creator plus a game probe on PS7/5.1: private creation/routing, no implicit
creation, ACL/data preservation, helper/input/start refusals and game-error
retention. The existing `test_stormglass_windows_launcher_runtime.ps1` separately
checks actual copied/relocated CPU/headless execution and external-bank admission.
None establishes settings UI persistence, native window gameplay, video or device
playback. Earlier blocked window-control evidence is retained without retry.
