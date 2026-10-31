import { AuthoredAssetAudio, loadAuthoredAssetBank } from './authored-asset-audio.mjs';

// Optional content policy, not a second event consumer or simulation clock.
// Only StrictEventAdapter's accepted output can select sounds. The current
// DefenseGame has one system (0); unsupported producers fail closed.
const COOP_AUDIO_IDS = Object.freeze({
  light: 'a_0000000054440001', heavy: 'a_0000000054440002', magic: 'a_0000000054440003',
  hitLight: 'a_0000000054440004', hitHeavy: 'a_0000000054440005',
  summon: 'a_0000000054440006', merge: 'a_0000000054440007', death: 'a_000000005444000a',
});
const COOP_RELEASE = Object.freeze([COOP_AUDIO_IDS.light, COOP_AUDIO_IDS.heavy, COOP_AUDIO_IDS.heavy, COOP_AUDIO_IDS.magic]);

export class CoopDefenseAudio {
  constructor(options) { this.owner = new AuthoredAssetAudio(options); this.rewindSerial = 0n; this.epoch = null; this.error = null; }
  cue() { return false; } // Never play heuristic/procedural cues alongside typed events.
  unlock(event) { return this.owner.unlock(event); }
  setMuted(value) { return this.owner.setMuted(value); }
  setPaused(value) { return this.owner.setPaused(value); }
  setHidden(value) { return this.owner.setHidden(value); }
  reset() { this.owner.reset(); }
  dispose() { this.owner.dispose(); }
  stats() { return { ...this.owner.stats(), adapterError: this.error, mode: 'coop-authored-pcm' }; }
  replaceInstanceBinding(presentation) {
    if (this.error) return false;
    try {
      const epoch = { source: presentation.source, generation: presentation.generation,
        tick: presentation.tick, rewindSerial: String(this.rewindSerial + 1n) };
      this.owner.replaceInstanceBinding(epoch);
      this.rewindSerial++; this.epoch = epoch;
      return true;
    } catch (error) {
      this.error = String(error?.message || error);
      this.owner.cancelAll('runtime-failure');
      return false;
    }
  }
  observe(result, presentation) {
    if (this.error) return;
    try {
      if (result.status === 'rejected') { this.owner.reset(); this.epoch = null; return; }
      if (result.status === 'duplicate') return;
      if (!result.observation) return;
      if (result.reset || !this.epoch) {
        this.epoch = { source: presentation.source, generation: presentation.generation,
          tick: presentation.tick, rewindSerial: String(++this.rewindSerial) };
        this.owner.beginEpoch(this.epoch, result.reason);
        return; // Baselines, rewinds and gaps never replay catch-up audio.
      }
      if (result.status !== 'applied') throw Error('Unsupported audio adapter result');
      const batch = [];
      for (const event of result.observation.events) {
        const key = event.engineKey;
        const raw = presentation.events.find(e => e.tick === key.tick && e.system === key.system && e.seq === key.seq);
        if (!raw || key.system !== 0 || key.source !== this.epoch.source || key.generation !== this.epoch.generation || raw.kind !== event.engineKind) throw Error('Audio event identity mismatch');
        let assetId;
        if (raw.kind === 'summon') assetId = COOP_AUDIO_IDS.summon;
        else if (raw.kind === 'merge') assetId = COOP_AUDIO_IDS.merge;
        else if (raw.kind === 'death') assetId = COOP_AUDIO_IDS.death;
        else if (raw.kind === 'release' || raw.kind === 'hit') {
          if (!COOP_RELEASE[raw.role]) throw Error('Unsupported audio role');
          assetId = raw.kind === 'release' ? COOP_RELEASE[raw.role] : [1, 2].includes(raw.role) ? COOP_AUDIO_IDS.hitHeavy : COOP_AUDIO_IDS.hitLight;
        }
        if (assetId) batch.push({ assetId, key: `${key.source}/${key.generation}/${this.epoch.rewindSerial}/${key.tick}/0/${key.seq}`,
          attribution: { unitId: raw.unitId, attackId: 0, projectileId: raw.projectileId } });
      }
      // Validate the complete mapping before playing any part of a batch.
      for (const cue of batch) this.owner.playAsset(cue.assetId, cue.key, cue.attribution);
    } catch (error) {
      this.error = String(error?.message || error);
      this.owner.cancelAll('runtime-failure');
      // Audio is optional; fail silent without advancing or stopping gameplay.
    }
  }
}

export async function loadAuthoredAudio(document, options = {}) {
  if (document.querySelectorAll && document.querySelectorAll('[id="authored-audio-bank"]').length > 1) throw Error('Duplicate audio bank');
  const embedded = document.getElementById('authored-audio-bank');
  if (!embedded) return null;
  if (embedded.textContent.length > 3 * 1024 * 1024) throw Error('Embedded audio exceeds budget');
  const data = JSON.parse(embedded.textContent);
  const names = ['attack-light', 'attack-heavy', 'attack-magic', 'impact-light', 'impact-heavy', 'summon', 'merge', 'ui-confirm', 'ui-reject', 'death'];
  if (!Array.isArray(data.manifest?.cues) || data.manifest.cues.some(r => r.assetId !== `a_000000005444${(names.indexOf(r.cue) + 1).toString(16).padStart(4, '0')}`)) throw Error('Unsupported CoopDefense cue IDs');
  const bank = await loadAuthoredAssetBank(data.manifest, async (file, size) => {
    const encoded = data.objects?.[file];
    if (typeof encoded !== 'string' || encoded.length !== Math.ceil(size / 3) * 4 || !/^[A-Za-z0-9+/]*={0,2}$/.test(encoded)) throw Error('Invalid embedded PCM');
    return Uint8Array.from(atob(encoded), c => c.charCodeAt(0));
  });
  // Explicit content admission errors propagate to startup; never use network
  // or silently replace broken authored content with procedural sound.
  return new CoopDefenseAudio({ ...options, bank });
}
