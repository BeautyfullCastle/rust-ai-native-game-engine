# Native Stormglass Run Save

The optional `coop-defense-run-save` feature lets a Linux native player save a
bounded defense run and resume it in a later process. It is a native input
journal, not an engine checkpoint, a web Run Save file or a cross-build migration.

## Use

Build the `coop_defense` binary with `orr_sample/coop-defense-run-save`. Supply
`--run-directory /absolute/private-save-folder` to select storage explicitly.
The owner-controlled directory must already exist with mode 0700. Create a new
private folder with `mkdir -m 700 /absolute/private-save-folder`, outside the
checkout, executable and scene directories. The game does not create it or
change its permissions. There is no automatic write to the current
directory or home folder. Keep saves
outside the executable bundle and authored scene directory.

Pause an active battle, then choose **Save Copy**. Each success creates a new
immutable generation and reports its filename. Existing saves are not replaced.
Closing the game does not save automatically.

Use **Load Save** on the title or pause screen, select a listed generation and
confirm replacing an existing run. Loading is cancellable. A successful load
remains paused until a fresh **Continue** action; a failed or cancelled load
keeps the previous paused game and its journal. There is no silent fallback to
another save.

## Native contract

The save pins the exact executable SHA-256, existing scenario/session build
identity and native profile: one local player, slot zero, 60 Hz, input delay two.
A rebuilt or different executable is incompatible, even when its game rules
appear unchanged. Keep the matching binary with your saves. This identity check
does not attest a maliciously replaced running image.

The origin is either the default configuration or the original admitted UTF-8
scene bytes. Saving does not reread a changed scene file, and loading never
follows a path or downloads executable code supplied by the save.

The bounded journal records the inputs actually submitted to the native
Session, their admission order, total advance count and UI sequence high-water.
Neutral admissions are reconstructible defaults. A detached fresh Session
replays that exact stream and must reproduce the saved frame checksum before
commit. A SHA-256 of the canonical versioned payload also covers admissions
that have not reached the visible frame yet. This detects edits without a
matching digest; it is integrity checking, not a signature or proof of authorship.
An intentional editor can recompute the digest.

Native admission number n targets simulation tick n+2. Replaying an ordinary
consumed-input `.orrp` stream through `Session::advance` would apply delay again
and omit admitted future commands. The native journal preserves those commands,
including saves made before they are consumed. The two-tick policy and replay
codecs are unchanged.

Unsubmitted UI commands, selection, gestures, visual effects and accumulated
wall time are not restored. Sequence high-water includes commands issued by the
UI but discarded before Session admission. New commands must not reuse those
sequence numbers. Restart remains part of the journal rather than resetting it.

## Bounds and failures

Limits are 36,000 total advances, a 4 MiB envelope and 64 KiB of scene bytes.
Restart does not reset the advance cap. Beyond the recording limit, new saving
is visibly disabled while play continues; earlier saves remain available. This
is ten minutes of simulated history, not a promise that every authored campaign
fits.

The native parser rejects incompatible identities, versions, unknown/duplicate
fields, malformed inputs, invalid ordinals/sequences and checksum mismatches.
Storage uses bounded regular-file reads, no-follow handling and no-replace
publication. Unsafe storage is rejected rather than changing directory
permissions or overwriting source, bundle or foreign files.
The directory scan is capped at 256 entries, including retained failed staging
files. Old copies are not deleted automatically. Error text shown in the game
is sanitized and bounded; a storage error does not replace the live run.
If directory synchronization fails after publication, the reported generation
may already exist. Inspect that result before making another copy.

## Acceptance boundary

Acceptance requires real CPU egui controls through the production App/Session,
save and load in separate processes, exact frame/checksum and next-input parity,
authored configuration, pending delayed inputs, Restart/terminal states,
cancel/error preservation, repeated operations and hostile storage cases.
Independent review and required exact-head CI precede development integration.

These checks are distinct from native-window, physical-input, OS-dialog, GPU,
Windows and browser acceptance. No native/web save interoperability, automatic
migration, account synchronization or general campaign-unlock persistence is
claimed.
