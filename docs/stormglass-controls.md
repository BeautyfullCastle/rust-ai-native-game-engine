# Stormglass gameplay keys

Build the Linux or Windows native player with `coop-defense-controls`. Open **KEYS**
in the top bar to change Summon, Forge/Upgrade, Invest and Restart. No audio bank
or output device is needed. Without a settings directory, Apply changes only the
current session.

To remember controls, supply `--controls-directory ABSOLUTE_PRIVATE_DIRECTORY`.
The directory must already exist outside source, executable, scene and audio-bank
roots. Linux requires current-user ownership and mode 0700. Windows requires a
fixed local NTFS drive and a user-owned private ACL granting access only to that
user, SYSTEM and Administrators. The [audio settings guide](stormglass-audio-settings.md)
describes the same directory setup. No cwd or environment fallback is used.
The option requires this feature and is rejected in headless mode.

## Editing

1. Select an action and press a fresh key. Keys already held when capture begins
   must be released first. Letters, arrows, Space and Tab are supported. Each
   action needs one unique keyboard key; mouse/gamepad bindings are not accepted.
2. **APPLY** replaces the complete session map. **SAVE** applies the draft and
   remembers it. If saving fails, the session change remains and the panel reports
   that it was not saved; existing disk data is preserved before publication.
3. **DEFAULTS** changes only the draft until Apply or Save. **CLOSE** discards the
   unapplied draft; already applied session changes remain. Esc cancels capture,
   or closes the panel when capture is not active.

Opening KEYS pauses a started defense. Closing never resumes it automatically;
continue explicitly. Panel/capture/apply/save/reset/close frames do not emit game
commands. Focus loss cancels capture and suppresses held controls until release.
Apply also requires fresh presses after keys held during editing are released.

Start remains Enter/Space, pause remains P/Esc, and Continue remains Enter. P, Esc
and Enter cannot be rebound. With this feature, keyboard identity uses the event's
physical key, falling back to its logical key when physical identity is absent.
Capture, gameplay and fixed safety shortcuts share that normalization. Ctrl, Alt,
Command and Mac Command are rejected using each event's modifiers, not the final
frame modifiers. Shift-only presses retain ordinary action behavior; modifier
chords are not stored. Without the feature, the existing shortcut behavior remains.

HUD and introduction hints reflect the active bindings. Pointer game controls and
canonical simulation commands retain their existing meaning.

## Storage and acceptance

`controls-settings.json` uses schema 1/profile `stormglass-controls-v1` and the
existing ActionMap version 1, bounded to 8 KiB. Startup only reads it. Missing data
uses default keys; malformed, unsupported, unknown-field, duplicate, reserved or
invalid maps use defaults and disable Save for that launch while preserving bytes.

The existing private settings protocol is reused with fixed independent primary,
lock and staging names. Audio filenames, schema and limits remain unchanged. Only
the explicitly selected profile is read or written, and the two locks do not
conflict. Audio and controls directories may be the same private directory.
Enabling controls does not opt into reading or writing audio preferences.

Saving uses baseline identity/byte conflict checks and atomic publication, with
separate prepublication failure and postpublication uncertainty. Windows retains
its documented crash-durability limitations; unknown publication disables further
saves. There is no automatic retry or rollback and no arbitrary same-user CAS
guarantee. See the audio guide for the shared filesystem boundary.

Focused gates exercise actual private files, process relaunch, modal pointer and
keyboard ownership, modifiers, focus, held/repeat state, simulation-frame equality,
invalid data, conflict/failure preservation and audio-profile isolation. Run
`tools/check_stormglass_controls_windows.ps1 -EvidenceDirectory C:\Evidence\fresh-controls`
from a clean committed Windows checkout with the normal current-user token. Logs
record exact source/tree outside the checkout. The Linux focused CI tests the same
candidate and its Linux storage path. CPU/filesystem tests do not establish native
window gameplay, hardware audio or a playtest video.
