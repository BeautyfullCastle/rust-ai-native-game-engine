# Remember Stormglass audio controls

Build the native Linux or Windows player with `coop-defense-audio-settings` and
launch with `--audio-settings-directory ABSOLUTE_PRIVATE_DIRECTORY`. Create the
directory outside the source checkout, executable directory, scene directory and
audio bank. On Linux it must belong to you and have mode 0700. On Windows use an
existing directory on a fixed local NTFS drive, e.g. `C:\Private\StormglassAudio`,
owned by your current user, with access granted only to that user, SYSTEM and
Administrators. In Explorer's Advanced Security settings disable inheritance and
remove other grants on this dedicated directory before launching the game.
No environment or working-directory fallback is used. The option is unavailable
without the feature and on other platforms, and is rejected in headless mode.

Volume and mute changes affect the current session. Click **SAVE AUDIO** to
remember both controls for the next launch. Controls also work without an active
output device, so you can configure the next playback session. Startup reads the
file without creating directories or files. Missing settings start muted at 100%
gain. A saved unmuted preference remains subject to the normal Start, pause and
focus audio gates; loading settings does not start the game or play a cue.

`audio-settings.json` uses schema 1/profile `stormglass-audio-v1`, integral gain
0–100 and a Boolean mute value. Malformed, oversized, inaccessible or unsupported
files produce muted defaults and disable Save for that launch. Existing bytes are
preserved. The panel reports session changes, success and failure separately.

Saving uses a stable advisory lock and compares the original file identity and
bytes. A stale session refuses to overwrite another writer's change; restart to
reload it. Private single-link regular files and no-symlink directory traversal
are required. On Linux a temporary file is synced and atomically renamed before
syncing the directory. Admission pins its inode; subsequent Linux saves still
target that directory if another process renames its pathname. Failures before
publication preserve the old primary. If Linux directory
sync fails after publication, the panel reports uncertain durability and tracks
the visible new value, without automatic retry or rollback. This coordinates
cooperating game instances; it is not an atomic compare-and-swap against arbitrary
same-user filesystem manipulation.

Windows creates lock/staging files with a private ACL from the outset. It pins
ancestors with directory-list access and no delete sharing, and uses a volume GUID
path for subsequent operations. Renaming an admitted directory is blocked while
the store is open. UNC/device/drive-relative paths, reparse points, alternate data
streams, reserved DOS names, ambiguous components and input paths over 240 bytes
are rejected. Existing single-link files must satisfy the same private ownership
and ACL checks; null or unsupported ACLs are refused. Locking is nonblocking.

Windows flushes the staged file before same-volume replacement using
[MoveFileExW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw)
with `REPLACE_EXISTING | WRITE_THROUGH`. This does not promise Linux-style
directory-fsync or power-loss durability. A reported rename error is checked
against the visible file identity: known publication reports uncertain durability;
an unrecognized outcome disables further saves until restart. Cleanup marks only
the verified staging handle for deletion, never a subsequently resolved pathname.
Windows persistence works even with output disabled. Authored banks also have a
[Windows loader](stormglass-windows-audio-bank.md); native output additionally needs
`audio-native`. No Windows hardware-audio playback is established here.

Focused acceptance covers strict schema admission, zero-write startup, explicit
save/reopen, rejected links and permissions, stale writers, busy/stable locks,
injected failures before and after publication, substituted staging files, and
CPU egui/App plus offline PCM gating. These checks do not demonstrate a native
window or hardware audio playback.
Windows acceptance additionally saves and restores in two separate CPU test
processes and checks ACLs, junctions, sharing violations, pinned-directory rename
blocking and uncertain publication. Original failures are retained in evidence.
Reproduce on a clean committed Windows checkout with
`tools/check_stormglass_audio_settings_windows.ps1 -EvidenceDirectory C:\Evidence\fresh-run`.
Use the normal current-user token: restricted sandbox tokens can be denied by the
intentionally user-private ACL. The gate records exact source/tree, requires test
counts and preserves logs outside the checkout. It does not open a game window.
