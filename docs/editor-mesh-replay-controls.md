# Mesh Player recorded timeline controls

The local MeshNavigation Player can inspect its existing Play-session recording
without starting a separate simulation or changing the scene document.

## Player workflow

- **Pause** stops live playback or recorded playback. **Replay** pauses the host,
  seeks to the first recorded tick, and plays the recorded timeline.
- **Recorded tick** scrubs within the session's first and last recorded ticks.
  Scrubbing stops recorded playback and leaves the host paused. Pause live
  playback before scrubbing. Seeking does not discard the recorded future or
  create a branch.
- The speed selector offers **0.25×, 0.5×, 1×, 2×, and 4×**. It controls the UI
  clock used for recorded playback only; it does not change the host's live
  simulation speed. Changing speed reanchors at the current recorded tick,
  without jumping. The selection survives Pause and Replay; restarting the
  session resets the Player controls.
- **Resume** from an earlier recorded tick continues recorded playback. At the
  recorded end, Resume can return to live playback. Recorded playback itself
  never advances beyond the existing recording.
- Live goal controls are disabled while a recorded playback/seek context is
  active. Pause exits that context. Session restart confirmation remains a
  separate action.

## Boundaries and safeguards

Seek requests outside the recorded range are rejected rather than clamped.
Recorded controls require a local, mutable MeshNavigation Play session, not a
viewer or remote timeline. Scrubbing never issues live-play, step, or branch
commands. Replay uses the host's existing recorded seek path; it does not
implement independent resimulation or import a separate recording.

Scene path/load identity, timeline epoch, recorded endpoints, tick rate, current
head, and live-playing state are checked before continuing an active recorded
context. External timeline changes invalidate that context. Automatic replay
waits for a matching paused snapshot before requesting another tick. Its clock
must be finite and monotonic, and automatic seek advancement is capped at 120
ticks per UI frame even at higher selected speeds. This bounds per-frame advance;
it is not a wall-clock performance guarantee.

## Focused regression coverage

The dedicated `app::mesh_recorded_playback::tests` module includes clock and speed
arithmetic, per-frame bounds, paused backward/forward seeks, checksum and recorded
future preservation, speed reanchoring, live/out-of-range refusal, external
branch/session invalidation, and same-path scene reload invalidation. Local-host
tests additionally require the `mesh-mission` feature. Test execution results
should be recorded separately; this document does not claim they have passed.
