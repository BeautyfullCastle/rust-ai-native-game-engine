# CoopDefense authored PCM

This optional content adapter connects the existing strict engine-event consumer
to the recovered typed Web Audio owner. It does not alter simulation, gameplay
timing, or the event ABI. It is original sample audio, not source-game audio or
proof of reproduction parity.

The bank is `crates/orr_coop_defense_web/web/assets/audio/bank.json`: ten mono
48 kHz PCM16 objects, each at most one second. Package admission checks fixed
cue IDs, size, SHA-256, headers and peak ≤8192. Use `python3 tools/pack_coop_defense_offline.py path/to/web --audio path/to/bank.json
--output path/to/new.html` with a fresh explicit output path. Omitting `--audio` preserves procedural sound.
The offline packer embeds bytes;
runtime admission repeats integrity checks before allocating a context. There
is no audio network fallback. A malformed explicit bank fails startup.

Only accepted `StrictEventAdapter` results produce sounds: summon, merge,
release, hit and enemy death. Roles 0/1/2/3 select light/heavy/heavy/magic
release; impact uses light/heavy according to role. Support/mining roles have
no inferred combat audio. UI confirm/reject clips remain admitted but unmapped:
the current ABI has no authoritative event for these actions. Existing
procedural `cue()` calls are silent when this adapter is selected.

The engine has one event system (0). Its source/generation/tick/sequence are
preserved; the owner-local rewind serial distinguishes strict resync epochs.
Generation zero is valid. Projectile identity is retained with attackId zero,
because CoopDefense does not publish an attack ID. No attack identity is
invented. Gaps, rejected batches, restart and backward-tick baselines stop
voices and do not replay history. Duplicate accepted ticks remain suppressed.

RunSave candidate replay never observes audio. Only a successful atomic WASM
instance swap calls `replaceInstanceBinding`: stop voices and establish a silent
baseline even when instance-local source/generation repeats or decreases. Ordinary
epoch monotonic checks stay strict. Context, gesture unlock, mute and hidden
settings survive; restoration stays paused. Failed/cancelled imports do not change
the audio binding. Resume consumes only new accepted events, not suppressed history.

Audio starts muted and requires a trusted click/key/pointer/touch gesture.
Pause/hidden/mute stop voices, and dispose closes the context. Voice count is
bounded (default 12, maximum 32), with gain bounded against the admitted PCM
peak. Optional runtime audio failure stops audio without advancing gameplay.

## Verification and provenance

Run `node --test crates/orr_coop_defense_web/web/tests/authored-audio.test.mjs`
and `python3 tools/test_pack_coop_defense_audio.py`. These test real PCM bytes
and the real strict JS adapter with mock Web Audio; they do not prove browser
autoplay, audible output, hardware mixing or user listening quality.

The owner and PCM were recovered from the immutable Windows candidate ZIP
dated 2026-10-25, SHA-256
`dccac82587eb45d6519fc8704092025102a74b009d7c71158029e4a2a9012dea`, under
`repo/examples/reference_battle_host/`. PCM and bank bytes are unchanged. The
owner permits generation zero, a projectile with no separate attack ID, and the
explicit successful-instance replacement boundary described above. The original
PCM generation script, creator attribution and separate audio LICENSE were not
found; byte lineage is not independent verification of distribution rights. Historical mock/offline Kira results are not counted as
verification of this new integration. Native playback and actual browser/device
listening remain separate acceptance work.
