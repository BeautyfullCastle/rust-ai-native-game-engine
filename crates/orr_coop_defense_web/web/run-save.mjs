// Optional Stormglass replay saves. No engine checkpoint or migration contract.
export const RUN_SAVE_LIMITS = Object.freeze({fileBytes:2097152,steps:36000,sceneBytes:65536});
const runSaveU32=4294967295, runSaveEncoder=new TextEncoder(), runSaveDecoder=new TextDecoder('utf-8',{fatal:true});
function runSaveFail(message){throw Error('Run save: '+message);}
function runSaveFields(value,names){if(!value||typeof value!=='object'||Array.isArray(value)||Object.keys(value).length!==names.length||names.some(k=>!Object.hasOwn(value,k)))runSaveFail('unknown or missing fields');}
function runSaveUint(value,max=runSaveU32){return Number.isSafeInteger(value)&&value>=0&&value<=max&&!Object.is(value,-0);}
function runSaveParseJson(text){
 if(typeof text!=='string'||text.length>RUN_SAVE_LIMITS.fileBytes||runSaveEncoder.encode(text).length>RUN_SAVE_LIMITS.fileBytes)runSaveFail('file exceeds 2 MiB');
 let pos=0;
 const ws=()=>{while(pos<text.length&&/[ \t\r\n]/.test(text[pos]))pos++;};
 const strPattern=/"(?:[^"\\\u0000-\u001f]|\\(?:["\\/bfnrt]|u[0-9a-fA-F]{4}))*"/y;
 const tokenPattern=/(?:true|false|null|-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?)/y;
 const str=()=>{strPattern.lastIndex=pos;const m=strPattern.exec(text);if(!m)runSaveFail('invalid JSON string');pos=strPattern.lastIndex;return JSON.parse(m[0]);};
 function value(depth){
  if(depth>8)runSaveFail('nesting exceeds 8');ws();const ch=text[pos];
  if(ch==='"')return str();
  if(ch==='{'||ch==='['){const obj=ch==='{',end=obj?'}':']',out=obj?Object.create(null):[];pos++;ws();if(text[pos]===end){pos++;return out;}
   for(;;){ws();let key;if(obj){key=str();if(Object.hasOwn(out,key))runSaveFail('duplicate key');ws();if(text[pos++]!==':')runSaveFail('missing colon');}const item=value(depth+1);if(obj)out[key]=item;else out.push(item);ws();const sep=text[pos++];if(sep===end)return out;if(sep!==',')runSaveFail('invalid separator');}
  }
  tokenPattern.lastIndex=pos;const m=tokenPattern.exec(text);if(!m)runSaveFail('invalid JSON value');pos=tokenPattern.lastIndex;const out=JSON.parse(m[0]);if(typeof out==='number'&&!Number.isFinite(out))runSaveFail('nonfinite number');return out;
 }
 const out=value(0);ws();if(pos!==text.length)runSaveFail('trailing data');return out;
}
function runSaveSceneBytes(initial){
 const s=initial.base64;
 if(typeof s!=='string'||s.length>87384||s.length%4!==0||!/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(s))runSaveFail('invalid scene base64');
 const raw=atob(s);if(raw.length>RUN_SAVE_LIMITS.sceneBytes||btoa(raw)!==s)runSaveFail('invalid scene size or base64');
 const bytes=Uint8Array.from(raw,c=>c.charCodeAt(0));runSaveDecoder.decode(bytes);return bytes;
}
function runSaveValidateInitial(initial){
 if(initial?.kind==='seed'){runSaveFields(initial,['kind','seed']);if(!runSaveUint(initial.seed))runSaveFail('invalid seed');}
 else if(initial?.kind==='scene'){runSaveFields(initial,['kind','base64']);runSaveSceneBytes(initial);}
 else runSaveFail('invalid initial state');
}
function runSaveValidateStep(step,previous){
 if(!Array.isArray(step)||step.length!==4||!runSaveUint(step[0],5)||!runSaveUint(step[1],11)||!runSaveUint(step[2],11)||!runSaveUint(step[3]))runSaveFail('invalid step');
 if(step[0]===0){if(step.some(v=>v!==0))runSaveFail('neutral step must be all zero');return previous;}
 if(step[3]<=previous)runSaveFail('command sequence must increase');return step[3];
}
export function parseRunSave(text){
 const save=runSaveParseJson(text);runSaveFields(save,['format','version','wasmSha256','initial','steps','sequence','checksum']);
 if(save.format!=='orr.stormglass-run'||save.version!==1)runSaveFail('unsupported format/version');
 if(typeof save.wasmSha256!=='string'||! /^[0-9a-f]{64}$/.test(save.wasmSha256))runSaveFail('invalid WASM hash');
 if(typeof save.checksum!=='string'||! /^[0-9a-f]{16}$/.test(save.checksum))runSaveFail('invalid checksum');
 runSaveValidateInitial(save.initial);if(!Array.isArray(save.steps)||save.steps.length>RUN_SAVE_LIMITS.steps)runSaveFail('step limit exceeded');
 let previous=0;for(const step of save.steps)previous=runSaveValidateStep(step,previous);
 if(!runSaveUint(save.sequence)||save.sequence<previous)runSaveFail('invalid sequence high-water');return save;
}
export function sceneInitial(bytes){
 if(!(bytes instanceof Uint8Array)||bytes.length>RUN_SAVE_LIMITS.sceneBytes)runSaveFail('invalid scene bytes');runSaveDecoder.decode(bytes);
 let raw='';for(const byte of bytes)raw+=String.fromCharCode(byte);return {kind:'scene',base64:btoa(raw)};
}
export const sha256 = async (bytes)=>{return Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes)),b=>b.toString(16).padStart(2,'0')).join('');};
export function createRunRecorder(initial, restoredSteps=[]){
 runSaveValidateInitial(initial);const origin=Object.freeze({...initial}),steps=[];let reason='',previous=0;
 if(!Array.isArray(restoredSteps)||restoredSteps.length>RUN_SAVE_LIMITS.steps)runSaveFail('step limit exceeded');
 for(const step of restoredSteps){previous=runSaveValidateStep(step,previous);steps.push(step.slice());}
 return {
  get initial(){return origin;},get stepCount(){return steps.length;},get steps(){return steps.map(s=>s.slice());},get disabledReason(){return reason;},
  record(step){if(reason)return false;if(steps.length>=RUN_SAVE_LIMITS.steps){reason='36,000-step recording limit reached; gameplay continues, save unavailable';return false;}
   try{previous=runSaveValidateStep(step,previous);steps.push(step.slice());return true;}catch(error){reason=error.message;return false;}},
  save(wasmSha256,sequence,checksum){if(reason)runSaveFail(reason);const text=JSON.stringify({format:'orr.stormglass-run',version:1,wasmSha256,initial:origin,steps,sequence,checksum});parseRunSave(text);return text;}
 };
}
// Cooperative deadline only: a synchronous WASM call cannot be preempted.
export const replayRunSave = async (input,{wasmBytes,wasmSha256,instantiate=(bytes,imports)=>WebAssembly.instantiate(bytes,imports),now=()=>performance.now(),yieldTask=()=>new Promise(resolve=>setTimeout(resolve,0)),isCurrent=()=>true}={})=>{
 const began=now();const check=()=>{if(!isCurrent())runSaveFail('stale load');if(now()-began>=5000)runSaveFail('cooperative replay deadline exceeded');};check();
 const save=parseRunSave(JSON.stringify(input));check();if(save.wasmSha256!==wasmSha256)runSaveFail('WASM hash mismatch');
 const result=await instantiate(wasmBytes,{});check();const wasm=result.instance?.exports||result.exports;
 if(!wasm)runSaveFail('missing WASM exports');
 if(save.initial.kind==='seed')wasm.defense_init(save.initial.seed);
 else{const bytes=runSaveSceneBytes(save.initial);if(bytes.length>wasm.defense_scene_capacity())runSaveFail('scene capacity exceeded');new Uint8Array(wasm.memory.buffer,wasm.defense_scene_buffer(),bytes.length).set(bytes);if(wasm.defense_init_scene(bytes.length)!==0)runSaveFail('scene rejected');}
 check();let sliceStart=now(),count=0;
 for(const step of save.steps){check();wasm.defense_step(...step);check();count++;if(count>=64||now()-sliceStart>=8){await yieldTask();check();sliceStart=now();count=0;}}
 const pointer=wasm.defense_snapshot(),length=wasm.defense_snapshot_len();const snapshot=JSON.parse(runSaveDecoder.decode(new Uint8Array(wasm.memory.buffer,pointer,length)));check();
 if(snapshot.checksum!==save.checksum)runSaveFail('checksum mismatch');check();return {wasm,snapshot};
};
