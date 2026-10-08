# Collect pickup audio (optional)

This bounded creation feature assigns one installed PCM16 pickup cue to an authored
CollectDodge project. Simulation, event serialization, Frame state, checksums and
replay formats are unchanged. It reuses `orr_audio`/Kira, including the real offline
mixer; it is not a second mixer or a WAV/MP3/OGG importer.

## Create and author

On Linux, build the creator with `project-create,collect-audio,collect-sprites` and
run `orr_new_arena --output /absolute/new/project --template
collect-dodge-audio-2d-v1 --seed example --game-id <canonical UUIDv4>`.
The explicit game identity follows the existing Collect high-score template.
The creator installs the original `collect-audio-v1` package alongside the sprite
package and writes `level.audio.json`. No download, build or generator is run.

Build the editor with `collect-audio`; open `--collect-project /absolute/project`.
The Collect pickup audio panel offers the two already-admitted installed clips,
integer gain 0–1000, mute, Preview/Stop, Undo/Redo and Save. These controls edit a
separate presentation document, not the simulation. Save changes only that
sidecar. External changes or a retired/replaced editor source require reopening.
Save does not silently overwrite another writer; directory-sync failures report
uncertain durability after retaining the actual successful write as the baseline.

Device output additionally requires the explicit `collect-audio-native` feature.
Without it, Preview and Play report native output unavailable. Acceptance uses an
explicit real Kira offline owner; it does not claim that a person heard sound.
Preview has a separate bounded owner from Play, so it cannot consume gameplay
identities. The editor keeps typed Collect events, resync and lifecycle ordering
before the old type-erasure boundary.

## Runtime and export

Build `collect_dodge` with `collect-audio` (and `collect-sprites,collect-progress`
for the generated starter). Native playback additionally requires
`collect-audio-native`. Output defaults to `--audio off`, which opens no device.
`--audio auto` reports output unavailability; `--audio required` fails honestly.
Malformed audio/package bytes fail admission for every policy, including Off.
Headless mode always uses no device and rejects non-Off device policies.

`--headless --ticks 180 --hold right --audio-render-check` consumes the actual
single-poll authoritative pickup events and renders real Kira PCM. It requires
finite bounded, nonzero output and reports started cues, rendered frames and peak.
It fails if no audible pickup is encountered. It does not estimate a waveform or
infer events from score/HUD changes.

Build `orr_export_collect_audio` with `collect-audio,project-export` and any
presentation features used by the source project. Supply an explicitly trusted
runtime and its SHA-256 using the existing export CLI arguments. The closed
profile is `collect-dodge-audio-linux-x86_64-v1`. The older Collect exporter rejects
audio. Export includes the exact admitted sidecar and full locked package closure,
including generator, source descriptions and explicit CC0 license. Relocation
requires no source repository or original project and makes no profile writes in
headless mode. Source-hidden, read-only execution is a mandatory separate gate.

## Contract and lifecycle

The sidecar is an object-only version-1 JSON document, at most 4096 bytes, with
`pickup` (package, manifest path, canonical asset GUID), `gain` (0–1000) and `mute`.
Unknown, duplicate, missing, null and positional-array shapes are rejected. It is
a distinct root-level Collect-only ProjectEntry `audio` path. The package and
consumer explicitly declare `collect-audio` capability.

Whole-bank owned admission verifies locked package version and file hashes,
manifest/type/schema, exact PCM object hashes and lengths, and cumulative decoded
budgets before conversion. PCM is the existing 8-byte little-endian rate/count
header plus mono signed 16-bit samples: 48 kHz, 1–48000 frames, peak <=8192.
Manifest, cooked and decoded memory remain bounded. Paths, symlinks, FIFOs,
nonregular files, oversize and tampering fail closed. This is an ordinary-file
transaction boundary, not a sandbox against concurrent hostile filesystem races.

Each atomic ViewUpdate is consumed once before moving its snapshot. Mute still
consumes/tombstones identities; unmuting never replays missed cues. Ordinary Collect
Restart uses monotonic event ticks and keeps tombstones; a short existing pickup
tail may finish. A new Play session/source resets ownership. Seek/branch/resync
stop tails and establish a silent baseline; forward events strictly beyond that
baseline can sound, while historical events cannot. Pause stops transients and
retains identities. Cancellation/replacement semantics remain EventAudio's existing
contract. Preview can be stopped independently.

## Verification and remaining scope

`collect-audio.yml` contains additive mandatory positive-count gates. The inventory
in `tools/collect_audio_cases.json` and its strict parser reject empty, missing,
failed, ignored or duplicate acceptance results. Original failures must be retained.
Focused checks do not replace the existing required determinism and integration CI.
Independent final-source review and exact-head required CI precede development
integration. Main and PR1 are excluded.

Covered gates include strict admission, real output/gain/mute/alternate clip,
authoritative Frame/event parity, repeated/late events, restart/seek/pause/source
replacement, actual editor widgets and Play, creator transactions, and production
source-hidden read-only export. Hardware listening, native-window interaction,
physical GPU and Windows audio execution are not established by software/offline
gates; #20 remains open. Room audio, general codecs, spatial sound, configurable
buses, streaming and music remain outside this slice and #107 remains open.
