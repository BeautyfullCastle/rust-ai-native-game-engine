> New user-facing Linux workflows should use the [optional Rust CLI](rust-coop-audio-bank.md). The Python commands below remain compatibility and differential-test references.

# WAV cues into an optional CoopDefense soundbank

The PCM bank authoring CLI accepts engine-cooked samples. This separate frontend
bridges ordinary WAV files through the **existing strict native importer**, then
uses the unchanged bank publisher and offline audio admission. It installs no
package and changes no runtime, simulation, event contract or built-in sound.

Build the importer from this checkout:

```sh
cargo build --locked --release -p orr_asset_cook --example wav_audio_package
python3 tools/create_coop_defense_audio_bank_from_wav.py \
  --importer target/release/examples/wav_audio_package \
  --license my-sounds/LICENSE.txt --output new-bank \
  --cue attack-light=my-sounds/attack-light.wav \
  --cue attack-heavy=my-sounds/attack-heavy.wav \
  --cue attack-magic=my-sounds/attack-magic.wav \
  --cue impact-light=my-sounds/impact-light.wav \
  --cue impact-heavy=my-sounds/impact-heavy.wav \
  --cue summon=my-sounds/summon.wav --cue merge=my-sounds/merge.wav \
  --cue ui-confirm=my-sounds/ui-confirm.wav \
  --cue ui-reject=my-sounds/ui-reject.wav --cue death=my-sounds/death.wav
```

Supply every cue exactly once. The output parent must exist; output itself must
not exist. Feed `new-bank/bank.json` to the existing offline packer's `--audio`
option. UI confirm/reject remain admitted but **unmapped** by the current runtime;
creating the bank does not invent successful-command or rejection events.

## Contract

Each WAV is bounded to 1 MiB including metadata. The reused importer admits only
RIFF PCM16 mono at 48 kHz, 1–48,000 frames, with samples −8192…8192. It rejects
unsupported or malformed input rather than resampling, downmixing or clipping.
The WAV standard supports other formats; they are deliberately outside this
frontend's contract ([Python WAV format documentation](https://docs.python.org/3/library/wave.html)).
License text is explicit, nonempty UTF-8, at most 64 KiB. Copying it is not a
copyright or ownership verification.

All inputs are snapshotted as bounded regular files before importing. Source
symlinks, special files, parent traversal and existing outputs are refused.
Ten temporary native packages are conversion containers only, never installed.
Each cooked object's digest is checked before the unchanged publisher admits the
whole bank. Failed conversion creates no final output; private scratch is cleaned
on normal completion or handled exceptions. Each native process has a 30-second
timeout, no shell, no standard input and no buffered diagnostic output.

Use a **trusted local importer executable and trusted/quiescent directories**.
This is not an executable sandbox, concurrent-adversary defense or crash-atomic
publication. Process termination can leave temporary scratch. Final publication
and exceptional cleanup inherit the existing bank publisher's guarantees and
limits; no unrelated files or existing output are removed.

## Acceptance

Focused tests invoke the real native importer, compare all cooked bytes/digests
through production bank admission, and cover invalid WAV formats/bounds,
input preservation, relocation determinism, output collisions, links/FIFOs and
late conversion failure cleanup. This is offline content production evidence,
not editor UI, browser playback, physical listening or hardware certification.
