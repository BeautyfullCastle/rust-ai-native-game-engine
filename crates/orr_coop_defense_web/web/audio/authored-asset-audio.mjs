// Recovered reviewed sample owner; CoopDefense permits generation zero and
// projectile attribution without an invented attack ID (its ABI has none).
// ReferenceEventCursor remains the event-delivery authority; no simulation/bus/clock.
const AUDIO_CUES = Object.freeze(['attack-light','attack-heavy','attack-magic','impact-light','impact-heavy','summon','merge','ui-confirm','ui-reject','death']);
const AUDIO_MAX_COOKED = 2 * 1024 * 1024;
const AUDIO_MAX_DECODED = 4 * 1024 * 1024;
export const audioGain = voices => Math.min(.5, .75 / (voices * .25));
const preparedBanks = new WeakMap();
const u32 = x => Number.isInteger(x) && x >= 0 && x <= 0xffffffff;
const decimal = x => typeof x === 'string' && /^(0|[1-9][0-9]{0,19})$/.test(x) && BigInt(x) <= 18446744073709551615n;
function epochFields(p) {
  if (!decimal(p?.source) || p.source === '0' || !u32(p.generation) || !decimal(p.rewindSerial) || !decimal(p.tick)) throw Error('Invalid audio epoch');
  return {source:p.source, generation:p.generation, rewindSerial:p.rewindSerial, tick:p.tick};
}
function deliveryFields(key) {
  if (typeof key !== 'string' || key.length > 100) throw Error('Invalid audio delivery key');
  const parts=key.split('/');
  if(parts.length!==6 || !/^(0|[1-9][0-9]{0,9})$/.test(parts[1]) || parts[4]!=='0' || !/^(0|[1-9][0-9]{0,9})$/.test(parts[5]) || !u32(Number(parts[5]))) throw Error('Invalid audio delivery key');
  return {...epochFields({source:parts[0],generation:Number(parts[1]),rewindSerial:parts[2],tick:parts[3]}), seq:Number(parts[5])};
}
async function audioDigest(bytes) {
  if (!globalThis.crypto?.subtle) throw Error('SHA-256 unavailable; bank remains disabled');
  const hash = await globalThis.crypto.subtle.digest('SHA-256', bytes);
  return [...new Uint8Array(hash)].map(v => v.toString(16).padStart(2,'0')).join('');
}
export async function loadAuthoredAssetBank(manifest, readObject, digest = audioDigest) {
  if (manifest?.version !== 1 || manifest.sampleRate !== 48000 || !Array.isArray(manifest.cues) || manifest.cues.length !== AUDIO_CUES.length) throw Error('Unsupported sample bank');
  const records=manifest.cues.map(record=>({...record}));
  let cooked = 0, decoded = 0;
  const seen = new Set(), ids = new Set();
  for (const r of records) {
    if (!AUDIO_CUES.includes(r.cue) || seen.has(r.cue) || !/^a_[0-9a-f]{16}$/.test(r.assetId) || r.assetId === 'a_0000000000000000' || ids.has(r.assetId) || r.file !== `${r.cue}.pcm` || !/^[0-9a-f]{64}$/.test(r.sha256) || !Number.isInteger(r.frames) || r.frames < 1 || r.frames > 48000 || r.bytes !== 8+r.frames*2) throw Error('Invalid sample bank record');
    cooked += r.bytes; decoded += r.frames*8; seen.add(r.cue); ids.add(r.assetId);
  }
  if (cooked > AUDIO_MAX_COOKED || decoded > AUDIO_MAX_DECODED) throw Error('Sample bank exceeds budget');
  // Admit and hash every object before allocating decoded samples.
  const objects = [];
  for (const r of records) {
    const source = await readObject(r.file, r.bytes);
    if (!(source instanceof Uint8Array) || source.byteLength !== r.bytes) throw Error(`Invalid audio object: ${r.cue}`);
    const bytes = source.slice();
    if (await digest(bytes) !== r.sha256) throw Error(`Invalid audio object: ${r.cue}`);
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    if (view.getUint32(0,true) !== 48000 || view.getUint32(4,true) !== r.frames) throw Error('Invalid PCM header');
    for (let i=0;i<r.frames;i++) if (Math.abs(view.getInt16(8+i*2,true)) > 8192) throw Error('PCM peak exceeds contract');
    objects.push({r, bytes});
  }
  const clips = new Map();
  for (const {r,bytes} of objects) {
    const view = new DataView(bytes.buffer), samples = new Float32Array(r.frames);
    for (let i=0;i<r.frames;i++) samples[i] = view.getInt16(8+i*2,true)/32768;
    clips.set(r.assetId, samples);
  }
  // Buffers are private in the owner after construction. No external paths remain.
  const bank=Object.freeze({assetIds:Object.freeze(records.map(r=>r.assetId)), sampleRate:48000, cookedBytes:cooked, decodedStereoBytes:decoded});
  preparedBanks.set(bank,clips); return bank;
}
async function audioReadBounded(response, maximum) {
  if (!response.ok || !response.body?.getReader) throw Error('Audio fetch unavailable');
  const reader=response.body.getReader(), chunks=[]; let length=0;
  try {
    for (;;) { const {done,value}=await reader.read(); if(done)break; length+=value.length; if(length>maximum)throw Error('Audio fetch exceeds budget'); chunks.push(value); }
  } catch(error) { await reader.cancel().catch(()=>{}); throw error; }
  const bytes=new Uint8Array(length); let offset=0;
  for(const chunk of chunks){bytes.set(chunk,offset);offset+=chunk.length;}
  return bytes;
}
export async function loadSampleAssetBank({baseURL=new URL('assets/audio/',document.baseURI), embedded=document.getElementById('authored-audio-bank'),timeoutMs=5000}={}) {
  if (embedded) {
    if (embedded.textContent.length > 3*1024*1024) throw Error('Embedded audio bank exceeds budget');
    const data=JSON.parse(embedded.textContent);
    return loadAuthoredAssetBank(data.manifest, async (file, size)=>{
      const encoded=data.objects?.[file];
      if(typeof encoded!=='string' || encoded.length>Math.ceil(size/3)*4)throw Error('Invalid embedded audio object');
      return Uint8Array.from(atob(encoded), c=>c.charCodeAt(0));
    });
  }
  if(!Number.isInteger(timeoutMs)||timeoutMs<1||timeoutMs>30000)throw Error('Invalid audio load timeout');
  const controller=new AbortController(),timer=setTimeout(()=>controller.abort(),timeoutMs);
  try {
    const bytes=await audioReadBounded(await fetch(new URL('bank.json',baseURL),{signal:controller.signal}),128*1024);
    const manifest=JSON.parse(new TextDecoder().decode(bytes));
    return await loadAuthoredAssetBank(manifest,async(file,size)=>audioReadBounded(await fetch(new URL(file,baseURL),{signal:controller.signal}),size));
  } catch(error) { if(controller.signal.aborted)throw Error('Audio loading timed out; bank remains disabled');throw error; }
  finally {clearTimeout(timer);}
}

export class AuthoredAssetAudio {
  #factory;#maxVoices;#bank;#buffers;#context;#master;#voices;#muted;#paused;#hidden;#unlocked;#disposed;#played;#dropped;#suppressed;#epoch;#watermark;#failed;#lastResult;#error;#unlockPromise;#syncPromise;#syncVersion;#bindingSerial=0n;
  get maxVoices(){return this.#maxVoices;}
  constructor({contextFactory, maxVoices=12, bank=null}={}) {
    if(!Number.isInteger(maxVoices)||maxVoices<1||maxVoices>32)throw Error('Audio voices must be 1..32');
    this.#factory=contextFactory||(()=>{const C=globalThis.AudioContext||globalThis.webkitAudioContext;return C?new C():null;});
    this.#maxVoices=maxVoices;this.#bank=null;this.#buffers=new Map();this.#context=null;this.#master=null;this.#voices=new Set();
    this.#muted=true;this.#paused=false;this.#hidden=false;this.#unlocked=false;this.#disposed=false;
    this.#played=0;this.#dropped=0;this.#suppressed=0;this.#epoch=null;this.#watermark=null;this.#failed=null;this.#lastResult=null;this.#error=null;this.#unlockPromise=null;this.#syncPromise=null;this.#syncVersion=0;
    if(bank)this.setBank(bank);
  }
  setBank(bank) {
    if(this.#disposed||this.#failed)throw Error('Stopped audio owner');
    if(this.#context)throw Error('Set bank before creating audio context');
    const admitted=preparedBanks.get(bank);
    if(!admitted || bank.sampleRate!==48000 || admitted.size!==10)throw Error('Only an admitted typed bank is accepted');
    // The loader snapshots metadata/bytes and owns decoded PCM. Each runtime gets
    // its own copy; no caller can mutate the samples or replace an admitted ID.
    this.#bank={...bank, clips:new Map([...admitted].map(([id,pcm])=>[id,pcm.slice()]))};
    this.#error=null;
  }
  failBank(error){this.#error=String(error?.message||error);this.#bank=null;this.#unlocked=false;this.stopVoices();return this.sync();}
  get enabled(){return this.#unlocked&&!!this.#bank&&!this.#failed&&!this.#muted&&!this.#paused&&!this.#hidden&&!this.#disposed;}
  async unlock(event) {
    if(this.#disposed||this.#failed||!event?.isTrusted||!['click','pointerdown','keydown','touchend'].includes(event.type))return false;
    if(this.#unlockPromise)return this.#unlockPromise;
    try {
      if(!this.#bank)throw Error('Authored soundbank unavailable');
      if(!this.#context){
        const context=this.#factory();if(!context)throw Error('Web Audio unavailable');
        let master;
        try {
          master=context.createGain();master.gain.value=0;master.connect(context.destination);
          const buffers=new Map();
          for(const [cue,pcm]of this.#bank.clips){const buffer=context.createBuffer(1,pcm.length,48000);buffer.copyToChannel(pcm,0);buffers.set(cue,buffer);}
          this.#context=context;this.#master=master;this.#buffers=buffers;
        } catch(error) {
          try{master?.disconnect();context.close()?.catch?.(()=>{});}catch{}
          throw error;
        }
      }
      this.#error=null;this.#unlocked=true;
      this.#unlockPromise=this.sync().then(()=>this.enabled&&this.#context.state==='running').finally(()=>{this.#unlockPromise=null;});
      return await this.#unlockPromise;
    }catch(error){this.#error=String(error?.message||error);this.#unlocked=false;return false;}
  }
  sync(){
    if(!this.#context||this.#disposed)return Promise.resolve();
    this.#syncVersion++;this.#master.gain.value=this.enabled?audioGain(this.#maxVoices):0;
    if(!this.enabled)this.stopVoices();if(this.#syncPromise)return this.#syncPromise;
    let settled=this.#syncVersion;
    this.#syncPromise=(async()=>{let observed;do{observed=this.#syncVersion;settled=observed;if(this.#disposed)break;
      try{if(this.enabled&&this.#context.state!=='running')await this.#context.resume();else if(!this.enabled&&this.#context.state==='running')await this.#context.suspend();}
      catch(error){this.#error=String(error?.message||error);this.#unlocked=false;this.#master.gain.value=0;this.stopVoices();break;}
      finally {
        // dispose may have closed the context while resume/suspend was pending.
        // Reassert ownership after either settlement, including rejection; no
        // late browser operation may leave a disposed owner running again.
        if(this.#disposed){
          this.#master.gain.value=0;this.stopVoices();
          if(this.#context.state!=='closed')try{await this.#context.close();}catch{}
        }
      }
      if(this.#disposed)break;
      if(!this.enabled){this.#master.gain.value=0;this.stopVoices();}
    }while(observed!==this.#syncVersion);})().finally(()=>{this.#syncPromise=null;if(!this.#disposed&&settled!==this.#syncVersion)return this.sync();});return this.#syncPromise;
  }
  setMuted(value){this.#muted=!!value;return this.sync();}
  setPaused(value){this.#paused=!!value;return this.sync();}
  setHidden(value){this.#hidden=!!value;return this.sync();}
  stopVoices(){for(const voice of [...this.#voices]){try{voice.source.stop();}catch{}voice.cleanup();}}
  // Only a successfully committed new simulation instance may cross this boundary.
  // Instance-local source/generation values can repeat or decrease; this is not
  // an ordinary rewind and must never be called by speculative candidate replay.
  replaceInstanceBinding(presentation) {
    if(this.#disposed||this.#failed)throw Error('Stopped audio owner');
    const next=epochFields(presentation); // Validate before changing live ownership.
    this.stopVoices();this.#epoch=Object.freeze(next);this.#watermark=null;
    this.#bindingSerial++;
    // Context, decoded buffers and all activation settings deliberately survive.
    return Object.freeze({status:'baselined',reason:'instance-replaced'});
  }
  beginEpoch(presentation, reason='resync') {
    if(this.#disposed||this.#failed)throw Error('Stopped audio owner');
    const next=epochFields(presentation), old=this.#epoch;
    if(old&&old.source===next.source) {
      if(next.generation<old.generation || (next.generation===old.generation&&BigInt(next.rewindSerial)<BigInt(old.rewindSerial)))throw Error('Stale audio epoch');
      if(next.generation===old.generation&&next.rewindSerial===old.rewindSerial) {
        const floor=this.#watermark?.tick??old.tick;
        if(BigInt(next.tick)<BigInt(floor))throw Error('Audio rewind requires a newer rewind serial');
      }
    }
    this.stopVoices();this.#epoch=Object.freeze(next);this.#watermark=null;
    return Object.freeze({status:'baselined',reason});
  }
  cancelAttack(unitId,attackId,_reason) {
    if(!u32(unitId)||!unitId||!u32(attackId)||!attackId)throw Error('Invalid attack cancellation');
    let stopped=0;
    for(const voice of [...this.#voices]) {
      // A consumed attacker cannot cancel its already released projectile audio.
      if(voice.attribution.unitId===unitId&&voice.attribution.attackId===attackId&&voice.attribution.projectileId===0){try{voice.source.stop();}catch{}voice.cleanup();stopped++;}
    }
    return stopped;
  }
  cancelAll(reason='reset') {
    if(reason==='dispose'){this.dispose();return;}
    if(reason==='runtime-failure'||reason==='fault')this.#failed=String(reason);
    this.stopVoices();if(this.#failed){this.#unlocked=false;this.sync();}
    // Never clear epoch/watermark here: pause/hidden/reset cannot enable replay.
  }
  playAsset(assetId,deliveryKey,attribution) {
    if(this.#disposed||this.#failed)throw Error('Stopped audio owner');
    if(typeof assetId!=='string'||!this.#bank?.clips.has(assetId))throw Error('Unadmitted audio asset ID');
    const key=deliveryFields(deliveryKey),epoch=this.#epoch;
    if(!epoch||key.source!==epoch.source||key.generation!==epoch.generation||key.rewindSerial!==epoch.rewindSerial)throw Error('Audio event does not match admitted epoch');
    if(!attribution||Object.keys(attribution).some(k=>!['unitId','attackId','projectileId'].includes(k))||!['unitId','attackId','projectileId'].every(k=>u32(attribution[k])))throw Error('Invalid audio attribution');
    if((attribution.attackId&&!attribution.unitId)||(attribution.projectileId&&!attribution.unitId))throw Error('Incomplete audio attack attribution');
    const last=this.#watermark;
    if(BigInt(key.tick)<=BigInt(epoch.tick)||(last&&(BigInt(key.tick)<BigInt(last.tick)||(key.tick===last.tick&&key.seq<=last.seq))))throw Error('Stale or duplicate audio delivery');
    // A bounded defensive admission watermark, not a second event cursor/history:
    // upstream ReferenceEventCursor alone decides event validity and resets.
    // Suppressed events are retired too, so unlock/unpause cannot replay backlog.
    this.#watermark={tick:key.tick,seq:key.seq};
    const suppressed=reason=>{this.#suppressed++;return this.#lastResult=Object.freeze({status:'suppressed',reason});};
    if(this.#muted)return suppressed('muted');if(this.#paused)return suppressed('paused');if(this.#hidden)return suppressed('hidden');
    if(!this.#unlocked)return suppressed('gesture-locked');
    if(!this.#context||this.#context.state==='closed')throw Error('Audio context unavailable');
    if(!this.enabled||this.#context.state!=='running')return suppressed('context-not-running');
    const buffer=this.#buffers.get(assetId);if(!buffer)throw Error('Admitted audio buffer missing');
    if(this.#voices.size>=this.#maxVoices){this.#dropped++;return suppressed('voice-limit');}
    let source,voice;
    try{
      source=this.#context.createBufferSource();source.buffer=buffer;source.connect(this.#master);
      let cleaned=false;voice={source,attribution:Object.freeze({...attribution}),deliveryKey,assetId,cleanup:()=>{if(cleaned)return;cleaned=true;try{source.disconnect();}catch{}this.#voices.delete(voice);}};
      source.onended=voice.cleanup;this.#voices.add(voice);source.start(this.#context.currentTime);this.#played++;
      return this.#lastResult=Object.freeze({status:'played',assetId,deliveryKey});
    }catch(error){
      try{source?.stop();}catch{}voice?.cleanup();try{source?.disconnect();}catch{}
      this.#error=String(error?.message||error);this.cancelAll('runtime-failure');throw error;
    }
  }
  reset(){this.stopVoices();}
  dispose(){if(this.#disposed)return;this.#disposed=true;this.reset();try{this.#master?.disconnect();this.#context?.close()?.catch?.(()=>{});}catch{}this.#buffers.clear();this.#bank=null;}
  stats(){return{bank:this.#bank?'authored-ready':'unavailable',bufferCount:this.#buffers.size,unlocked:this.#unlocked,muted:this.#muted,paused:this.#paused,hidden:this.#hidden,contextState:this.#context?.state||'not-created',voices:this.#voices.size,played:this.#played,dropped:this.#dropped,maxVoices:this.#maxVoices,masterGain:audioGain(this.#maxVoices),error:this.#error,failed:this.#failed,disposed:this.#disposed,suppressed:this.#suppressed,bindingSerial:String(this.#bindingSerial),epoch:this.#epoch?{...this.#epoch}:null,lastResult:this.#lastResult};}
}
