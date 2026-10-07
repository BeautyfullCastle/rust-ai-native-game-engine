# Authored CollectDodge progress v1

This default-off Linux `orr_sample/collect-progress` slice persists only the highest
`best_collected` of completed local runs (win, hazard loss, timeout loss).
Restart-aborted runs do not submit. It is not a checkpoint, inventory, replay,
cloud save, identity authentication or anti-cheat system.

## Explicit identity

A progress-enabled `orr.project.json` uses schema 3, its existing
`collect-dodge-v1` entry, and a required declaration:

```json
"progress": {
  "schema": 1,
  "game_id": "12345678-1234-4234-8234-123456789abc",
  "profile": "collect-dodge-highscore-v1"
}
```

The ID above is an example, not a universal default. Supply a fresh lowercase
UUIDv4 when authoring an independent game. Parsing, opening, launching and
exporting never mint or write an identity. Schema 1/2 serialization and play
remain unchanged; no identity means no progress data discovery. A runtime built
without the Linux feature rejects schema 3 rather than pretending to save.
Editor and exporter explicitly support metadata-only admission and never open a
player profile. The copied runtime must support schema 3; export's existing
zero-tick headless smoke checks that capability without profile access.

Ordinary copy, relocation, rename, rebuild and export preserve the ID. Copies
intentionally share progress. For a new independent fork supply a different
UUIDv4; `ProjectProgress::fork_with_game_id` validates that pure change. It does
not copy player data or rewrite projects. Offline duplicate IDs cannot be
reliably detected globally.

The save namespace additionally includes a SHA-256 semantic challenge digest:
length-prefixed domain `orrery.collect-dodge.challenge.v1`, u32 rules revision 1,
u64 seed, u32 tick rate, u32 time limit, u32 actor count, then actors sorted by
(kind, ordinal), each two u32 values and four i64 raw FP values for initial
position/velocity. Integers are little-endian. Cosmetic names, GUID spellings,
paths, YAML formatting and engine package versions are excluded. Gameplay rules
changes must bump the compiled rules revision; gameplay-level changes naturally
form a new namespace. Existing simulation/build identities and goldens do not
change.

## Completion and storage boundaries

A game-specific `SimHost` wrapper observes each authoritative local Record-mode
step before the bridge publishes to its lossy presentation mailbox. It tracks a
bounded monotonic completed maximum, requiring the verified finish event and a
PLAYING-to-terminal transition. Seek/branch/debug/viewer paths cannot award.
Window-side persistence consumes that accumulator; Frame/simulation contain no
filesystem I/O. Editor, replay, headless, capture and export smoke do not construct
the progress session, including when Cargo feature unification enables code.

Store root is absolute `$XDG_DATA_HOME`, falling back to absolute
`$HOME/.local/share`, then `orrery/games/<uuid>/<profile>/<digest>/highscore.json`.
Relative XDG values are ignored; there is no cwd/project/export fallback.
Manifest values never supply filenames or directory fragments. The writer uses
bounded paths, fd-relative no-follow operations, regular user-owned single-link
files and a stable nonblocking advisory lock. It rereads both copies while
holding the lock and merges max(existing, candidate), stages in the same
directory, syncs files, atomically publishes and syncs the directory.

Version 1 is the first envelope, at most 4 KiB. Missing data is in-memory zero,
without writes until a completed run. A matching valid backup may recover a
missing/corrupt primary with notice. Identity mismatch or unknown versions are
preserved read-only. Both corrupt copies remain preserved; there is no automatic
reset or migration. Postpublication sync failure reports durability uncertain
and disables further automatic writes for that session. Gameplay continues on
storage failure and never claims the score was saved.

## Evidence requirements and remaining scope

Run focused identity, challenge, store failure/concurrency/path safety, reliable
observer and window transaction tests, default-feature regression tests and strict
lint. Built production runtime/export smoke must demonstrate schema support and
zero profile access. Source-hidden isolated test-process relaunch proves the
same window persistence components with an external data root; it is not a
physical desktop input/window validation. Keep those evidence categories separate.

Checkpoints, inventory, migrations, multiple save slots, audio, authored sprite/HUD
work, prefab/reimport and the full #94 three-game acceptance remain open.

References: [RFC 9562](https://www.rfc-editor.org/rfc/rfc9562.html#section-8),
[XDG](https://specifications.freedesktop.org/basedir/latest/),
[Rust rename](https://doc.rust-lang.org/std/fs/fn.rename.html),
[Linux fsync](https://man7.org/linux/man-pages/man2/fsync.2.html).

## Explicit acceptance commands

Build production `collect_dodge` and `orr_export_collect` with
`--features collect-progress,project-export`. Build the sample library test binary
with the same features (`cargo test -p orr_sample --lib --no-run
--message-format=json`), select its unique executable artifact and preserve it.
Set absolute `ORR_COLLECT_RUNTIME`, `ORR_COLLECT_EXPORTER` and
`ORR_PROGRESS_TEST_BIN` to those exact built artifacts.

The `collect_project_export` target has four explicit tests with these features:
existing exported CPU/GPU workflows, `collect_progress_source_hidden_relaunch_and_no_headless_writes`
and `collect_progress_noninteractive_syscall_isolation`. Execute each with
`--ignored --exact` and require one actual passing execution. The syscall test
requires `strace`, working ptrace permissions and existing Linux bubblewrap
support; missing permissions fail rather than skip. It is separate from the
functional workflow so an environmental block cannot disguise either result.
Feature-disabled builds retain the original two export tests.

The local restricted executor denied PTRACE_TRACEME/PTRACE_SETOPTIONS (EPERM).
That original failed syscall attempt is preserved. The source-hidden functional
App/store relaunch workflow passed independently; syscall isolation still needs
positive execution in the required Linux CI environment. Source inspection or a
missing data directory is not a substitute for this pending syscall proof.
