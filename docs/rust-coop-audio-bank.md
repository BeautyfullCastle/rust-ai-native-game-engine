# Rust CoopDefense soundbank authoring

Build `cargo build --release -p orr_package --bin orr_pkg --features audio-authoring`.
The default-off Linux frontend needs no Python, Node or external importer:

```sh
orr_pkg create-coop-audio-bank --cue attack-light=light.pcm ... \
  --license LICENSE.txt --output new-bank
orr_pkg create-coop-audio-bank-from-wav --cue attack-light=light.wav ... \
  --license LICENSE.txt --output new-wav-bank
```

`...` stands for the remaining explicit cue assignments, not a literal argument.
Supply each exactly once: attack-light, attack-heavy, attack-magic, impact-light,
impact-heavy, summon, merge, ui-confirm, ui-reject, death. No `--importer`, shell,
arbitrary executable or implicit PATH lookup. OS-native cue paths follow the
first `=`. Inputs in `.orr` remain allowed; traversal, NUL, symlink components,
nonregular files and output collisions are refused. As deliberate safety bounds,
argv is at most 64 KiB/24 arguments and absolute paths at most4096 bytes/128
components. These extra bounds are not claimed as universal legacy equivalence.

The existing strict Rust WAV decoder is shared with the legacy example, not
reimplemented: mono PCM16, 48kHz, exact RIFF/chunk/padding admission, 1..48000
frames, signed peak inclusive ±8192. No resampling/channel mixing/normalizing or
clipping. PCM uses the existing native payload validator. Every input is
snapshotted before output: WAV<=1MiB each, PCM<=96008 bytes, nonempty UTF-8
user-supplied license<=64KiB. Aggregate cooked<=2MiB and decoded<=4MiB remain
checked. In-process bounded decoding is not a subprocess wall-clock timeout.

Output has exactly ten cue-named `.pcm` files, `bank.json`, and `LICENSE.txt`.
Cue order, fixed asset IDs0001..000a, lowercase SHA256 and manifest bytes match
the original Python producer. License text is preserved, not verified as rights.
This is a soundbank, not an engine-installable content package. No device,
playback, runtime capability grant or project installation is performed.

Publication uses a held no-follow parent and exclusive file creation, with
identity/inventory/byte checks. Trusted stable/quiescent ancestry is required;
this is not a hostile-concurrent-filesystem sandbox. Output is visible while
being written, **not atomic or crash-durable**. Failures after directory creation
leave partial output/foreign content untouched and explicitly require manual
inspection: never blindly retry, overwrite or remove it. Once all12 files are
verified, directory/parent sync failure reports a complete existing bank with
uncertain durability, not absence. This conservative error policy intentionally
differs from the old Python cleanup; successful12file bytes remain identical.

Legacy Python tools and native WAV CLI remain for compatibility. The final web HTML packer and official build now also have a
[Python-free Rust workflow](rust-coop-workflow.md); authored runtime validation
still requires trusted Node. Tests run the unchanged strict packer
admission and actual Node consumer with mock WebAudio, including exact samples,
mute/pause and duplicate suppression. That is not hardware playback or listening
certification. UI cues do not become invented authoritative engine events.
