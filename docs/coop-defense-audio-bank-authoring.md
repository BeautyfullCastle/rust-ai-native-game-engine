> Python-free PCM/WAV authoring is now available through [the optional Rust CLI](rust-coop-audio-bank.md). Legacy commands below remain for compatibility; the final HTML packer still requires Python.

# Author a CoopDefense soundbank

This optional Linux authoring tool builds the bank consumed by the existing
CoopDefense `--audio` offline exporter. It changes neither simulation events nor
the runtime sound policy. Existing sample audio is not copied automatically.

Provide all ten fixed cues using repeated `--cue NAME=PATH` options, an explicit
`--license LICENSE.txt`, and `--output /trusted/new-bank`. Cue names are
`attack-light`, `attack-heavy`, `attack-magic`, `impact-light`, `impact-heavy`,
`summon`, `merge`, `ui-confirm`, `ui-reject`, and `death`.

```
python3 tools/create_coop_defense_audio_bank.py --help
# Supply --cue once for each name listed above:
# python3 tools/create_coop_defense_audio_bank.py --cue attack-light=light.pcm ... \
#   --license LICENSE.txt --output /trusted/new-bank
python3 tools/pack_coop_defense_offline.py /trusted/web \
  --audio /trusted/new-bank/bank.json --output /trusted/new-game.html
```

Inputs use the engine's cooked Impact PCM payload, **not WAV**: little-endian
u32 sample rate 48000, u32 frame count 1..48000, then mono i16 samples with peak
at most8192. The existing WAV package cooker produces this payload in its
`cooked/objects/<sha256>.bin`; its Collect package does not itself bind web events.
There is no resampling, mixing, normalization, format conversion or preview here.

The tool snapshots all inputs, generates stable cue IDs and exact sizes/hashes,
and runs the unchanged production soundbank admission before publication.
Output contains exactly `bank.json`, ten named PCM files and `LICENSE.txt`.
The explicit nonempty UTF-8 license (at most64KiB) is copied unchanged. Supplying
one does not establish distribution rights. No built-in asset provenance or
license claim is inherited. Cue assignment order does not change output bytes.

Existing output files, directories and links are refused. Source leaves and
ancestors use the existing Linux no-follow bounded reader; FIFO/devices are
rejected without blocking, and lexical parent traversal is forbidden. The output
parent must exist. Creation uses a held parent descriptor, exclusive directory
and file creation, and no-follow operations. Use trusted, quiescent directories:
publication is not an atomic directory transaction, and a killed process can
leave partial output. Wait for successful exit before consuming it. Ordinary
write failure removes only this invocation's files/directory; incomplete cleanup
is reported. Inputs are never modified.

The existing strict event adapter retains authoritative cue mapping, muted
startup, trusted-gesture unlock, pause/hidden/voice limits and RunSave semantics.
`ui-confirm`/`ui-reject` remain admitted but unmapped; this tool invents no events.

Verification commands:
```
python3 tools/test_create_coop_defense_audio_bank.py
node tools/test_create_coop_defense_audio_bank_actual.mjs
```
The consumer test uses synthetic authored PCM, real CLI/offline admission/strict
runtime code and mock WebAudio. It verifies exact samples and event suppression,
not actual listening, a browser, WASM gameplay or audio hardware.
