# Remember Stormglass audio controls

Build the native Linux player with `coop-defense-audio-settings` and launch with
`--audio-settings-directory /absolute/private/directory`. Create the directory
outside the source checkout, executable directory, scene directory and audio bank;
it must already exist, belong to you and have mode 0700. No environment or working
directory fallback is used. The option is unavailable without the feature and on
other platforms, and is rejected in headless mode.

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
are required. A temporary file is synced and atomically renamed before syncing
the directory. Admission pins the directory inode; subsequent saves still target
that directory if another process renames its pathname. Failures before publication
preserve the old primary. If directory
sync fails after publication, the panel reports uncertain durability and tracks
the visible new value, without automatic retry or rollback. This coordinates
cooperating game instances; it is not an atomic compare-and-swap against arbitrary
same-user filesystem manipulation.

Focused acceptance covers strict schema admission, zero-write startup, explicit
save/reopen, rejected links and permissions, stale writers, busy/stable locks,
injected failures before and after publication, substituted staging files, and
CPU egui/App plus offline PCM gating. These checks do not demonstrate a native
window or hardware audio playback.
