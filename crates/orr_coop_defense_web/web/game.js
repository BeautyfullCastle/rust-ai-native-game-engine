import { CombatPresentation } from './combat-presentation.mjs';
import { StrictEventAdapter, applyStrictSnapshot } from './strict-event-adapter.mjs';
import { RUN_SAVE_LIMITS, parseRunSave, createRunRecorder, sceneInitial, sha256, replayRunSave } from './run-save.mjs';
// Presentation/input adapter only. All gameplay and command acceptance remain in Rust.
const $=id=>document.getElementById(id),canvas=$('game'),c=canvas.getContext('2d');
const roles=[['여우 궁수','빠른 단일 사격','#c57b32'],['멧돼지 포격수','넓은 범위의 폭발 공격','#c45d4c'],['아르마딜로 전사','적의 장갑을 파쇄','#9773bb'],['올빼미 마법사','적의 이동 속도를 늦춤','#4f9faf'],['토끼 고수','내 보드 전체 공격 지원','#629455'],['두더지 광부','추가 자원 생산','#b58d36']];
$('codex').innerHTML=roles.map(r=>`<div style="--role:${r[2]}"><b>${r[0]}</b><br>${r[1]}</div>`).join('');
let sceneLoadGeneration=0,startupPending=false;
let loadedWasmBytes=null,loadedWasmSha256=null,runRecorder=null,runImportActive=false,runImportBusy=false;
let wasm,state,selected=-1,ready=true,paused=false,seq=0,queue=[],last=0,acc=0,sound=false,seed=71825;
let towerAtlas=null,heroRects=null,enemyAtlas=null,enemyRects=null,gesture=null,hover=-1,pending=null,feedback=[],oldEnemies=new Map(),lastNotice='';
// The verified reference uses a portrait stacked arena on every screen.
const mobile=()=>true;
function dimensions(){canvas.width=mobile()?500:1000;canvas.height=mobile()?720:640;}
// Every board-owned object uses this same view transform. The Rust coordinates are unchanged.
// Final horizontal lane segments converge on one visible castle, independent of board scale.
function project(x,y,board){const m=mobile(),s=board?(m?.5:.82):(m?1:1.05),ox=board?(m?-125:89):(m?0:8),oy=board?(m?-50:-20):(m?90:-90);if(y>=590){const start=50+board*500,end=500,t=Math.max(0,Math.min(1,(x-start)/(end-start))),a=[start*s+ox,590*s+oy],z=m?[250,665]:[520,565];if(m&&board){const nodes=[a,[20,a[1]],[20,665],z],lengths=[Math.abs(a[0]-20),665-a[1],230],total=lengths.reduce((n,v)=>n+v,0);let d=t*total;for(let i=0;i<3;i++){if(d<=lengths[i]){const k=d/lengths[i];return[nodes[i][0]+(nodes[i+1][0]-nodes[i][0])*k,nodes[i][1]+(nodes[i+1][1]-nodes[i][1])*k];}d-=lengths[i];}return z;}return[a[0]+(z[0]-a[0])*t,a[1]+(z[1]-a[1])*t];}return[x*s+ox,y*s+oy];}
function scale(board){return board?(mobile()?.5:.82):(mobile()?1:1.05);}
const presentation=new CombatPresentation();
const visualAdapter=new StrictEventAdapter({project,scale});
const animationImages=new Map();
function observePresentation(){const result=applyStrictSnapshot(presentation,visualAdapter,state);const out=$('event-status');if(out)out.textContent=`엔진 이벤트 v1 · ${result.status} · ${result.reason}`;return result;}
function syncPresentation(){presentation.setPaused(paused||ready);presentation.setHidden(document.hidden);}
function paintFrame(frame,image,size,fallbackRect){const rect=frame?.rect||fallbackRect;if(!image||!rect)return false;const logical=frame?.logicalSize||[rect[2],rect[3]],k=Math.min(size/logical[0],size/logical[1]),anchor=frame?.anchor||[.5,.86],pivot=frame?.pivot||[rect[2]*anchor[0],rect[3]*anchor[1]];c.drawImage(image,...rect,-pivot[0]*k,20-pivot[1]*k,rect[2]*k,rect[3]*k);return true;}
function unitAt(cell){return state?.units.find(u=>u[0]===0&&u[1]===cell);}
function snapshot(){const p=wasm.defense_snapshot(),n=wasm.defense_snapshot_len();return JSON.parse(new TextDecoder().decode(new Uint8Array(wasm.memory.buffer,p,n)));}
function notice(t){lastNotice=t;$('notice').textContent=t;}
function tone(){presentation.audio.cue('ui');}
function blocked(){return runImportActive?'저장된 방어전을 검증 중입니다.':!state?'게임을 준비하고 있어요.':ready?'방어 시작을 먼저 눌러 주세요.':paused?'일시정지 중입니다. 계속하기를 눌러 주세요.':state.phase?'방어전이 끝났습니다. 다시 도전해 보세요.':'';}
function actionReason(action,source,target){let reason=blocked();if(reason)return reason;if(queue.length>=32)return'입력 처리 중입니다. 잠시 기다려 주세요.';if(action===2){const u=unitAt(source),v=unitAt(target);if(!u)return'먼저 내 수호대를 선택하세요.';if(source===target)return'다른 칸에 놓아 주세요.';if(target<0||target>=12)return'내 보드 안에 놓아 주세요.';if(v&&(u[2]!==v[2]||u[3]!==v[3]))return'같은 역할·등급만 합성 가능 · 선택한 수호대를 탭하면 선택 해제';if(v&&u[3]>=5)return'최고 등급은 더 합성할 수 없어요.';return'';}const index=action===1?0:action===3?1:2;if(state.gold[0]<state.costs[index])return`자원이 ${state.costs[index]-state.gold[0]} 부족해요.`;if(action===1&&state.units.filter(u=>u[0]===0).length>=12)return'보드가 가득 찼어요. 같은 수호대를 합성하세요.';if(action===3&&state.upgrades[0]>=8)return'전투 강화가 최고 단계입니다.';if(action===4&&state.investment[0]>=5)return'경제 투자가 최고 단계입니다.';return'';}
function command(action,source=selected,target=selected){if(!Number.isInteger(action)||action<1||action>4||!Number.isInteger(source)||!Number.isInteger(target)||source < -1||source>11||target < -1||target>11)return false;if(seq>=0xffffffff){notice('입력 번호가 모두 사용되었습니다. 새 장면을 불러오세요.');return false;}const reason=actionReason(action,source,target);if(reason){notice(reason);return false;}queue.push([action,Math.max(0,source),Math.max(0,target),++seq]);return true;}
function cancelGesture(){if(gesture&&canvas.hasPointerCapture?.(gesture.id))canvas.releasePointerCapture(gesture.id);gesture=null;hover=-1;}
function invalidateRunImport(){runImportActive=false;return ++sceneLoadGeneration;}
function start(){if(startupPending)return;invalidateRunImport();if(!wasm)return;if(state){if(seq>=0xffffffff){notice('입력 번호가 모두 사용되었습니다. 새 장면을 불러오세요.');return;}recordedStep([5,0,0,++seq]);}else{wasm.defense_init(seed);runRecorder=createRunRecorder({kind:'seed',seed});}resetView();}
function resetView(){cancelGesture();state=snapshot();selected=-1;queue=[];pending=null;acc=0;paused=true;ready=true;$('ready').hidden=false;$('result').hidden=true;oldEnemies.clear();feedback=[];presentation.reset();visualAdapter.reset();observePresentation();$('pause').textContent='정지';notice('내 보드에 수호대를 소환하세요. 동료는 로컬 AI입니다.');update();}
// Run files contain data only. Candidate replay never touches the live exports.
function updateRunSave(){
 const reason=runImportActive?'저장된 방어전 검증 중':runRecorder?.disabledReason||(!runRecorder?'게임 준비 중':'');
 $('run-save').disabled=!!reason;
 $('run-status').textContent=reason||`저장 가능 · 실행 ${runRecorder.stepCount} / ${RUN_SAVE_LIMITS.steps} 틱 · 동일 게임 코어 전용`;
}
function runSaveText(){
 if(runImportActive||!wasm||!runRecorder)throw Error('지금은 저장할 수 없습니다.');
 return runRecorder.save(loadedWasmSha256,seq,snapshot().checksum);
}
function downloadRun(){
 let url=null;
 try{
  const blob=new Blob([runSaveText()],{type:'application/json'});
  url=URL.createObjectURL(blob);
  const link=document.createElement('a');link.href=url;link.download='stormglass-run-v1.json';
  document.body.appendChild(link);try{link.click();}finally{link.remove();}
  notice('방어전 파일을 다운로드했습니다. 대기 중 입력은 저장하지 않습니다.');
 }catch(error){notice('방어전 저장 실패: '+error.message);}
 finally{if(url)setTimeout(()=>URL.revokeObjectURL(url),0);updateRunSave();}
}
async function loadRunFile(file){
 if(startupPending)return false;
 const generation=invalidateRunImport();
 if(!file||!wasm)return false;
 if(runImportBusy){notice('이전 불러오기를 취소했습니다. 검증 종료 후 파일을 다시 선택하세요.');return false;}
 runImportBusy=true;runImportActive=true;updateRunSave();
 const current=()=>generation===sceneLoadGeneration;
 let committed=false;
 try{
  if(!Number.isSafeInteger(file.size)||file.size<0||file.size>RUN_SAVE_LIMITS.fileBytes)throw Error('방어전 파일은 최대 2 MiB입니다.');
  const bytes=new Uint8Array(await file.arrayBuffer());
  if(!current())return false;
  if(bytes.length>RUN_SAVE_LIMITS.fileBytes)throw Error('방어전 파일은 최대 2 MiB입니다.');
  const saved=parseRunSave(new TextDecoder('utf-8',{fatal:true}).decode(bytes));
  const replayStarted=performance.now();
  const restored=await replayRunSave(saved,{wasmBytes:loadedWasmBytes,wasmSha256:loadedWasmSha256,isCurrent:current});
  if(!current())return false;
  const recorder=createRunRecorder(saved.initial,saved.steps);
  if(!current())return false;
  if(performance.now()-replayStarted>=5000)throw Error('복원 협력적 제한 시간 초과');
  // All checks precede this synchronous commit. No await can split the swap.
  wasm=restored.wasm;state=restored.snapshot;runRecorder=recorder;seq=saved.sequence;committed=true;
  selected=-1;queue=[];pending=null;acc=0;last=0;paused=true;ready=false;
  // Pointer capture may already have been lost while candidate replay yielded.
  const capturedGesture=gesture;gesture=null;hover=-1;
  try{if(capturedGesture&&canvas.hasPointerCapture?.(capturedGesture.id))canvas.releasePointerCapture(capturedGesture.id);}catch{/* Capture was lost; logical selection is already clear. */}
  oldEnemies.clear();feedback=[];presentation.reset();visualAdapter.reset();observePresentation();
  $('ready').hidden=true;$('result').hidden=true;$('pause').textContent='계속';
  $('scene-source').textContent=saved.initial.kind==='scene'?'방어전 파일의 제작 장면':'방어전 파일의 기본 장면';
  notice('방어전을 복원했습니다. 일시정지 상태입니다. 계속 버튼으로 재개하세요.');
  return true;
 }catch(error){if(current())notice((committed?'방어전은 복원됐지만 화면 갱신에 실패했습니다: ':'방어전 불러오기 실패 · 기존 게임 유지: ')+error.message);return committed;}
 finally{runImportBusy=false;if(current())runImportActive=false;update();}
}
$('run-save').onclick=downloadRun;
$('run-file').onchange=e=>{void loadRunFile(e.target.files?.[0]);e.target.value='';};
// Shared local and embedded byte path. Rust validates before replacing its client.
function loadSceneBytes(bytes){
 if(!(bytes instanceof Uint8Array)||bytes.length>65536||bytes.length>wasm.defense_scene_capacity())throw Error('64 KiB 초과 또는 장면 바이트 오류');
 const pointer=wasm.defense_scene_buffer();
 new Uint8Array(wasm.memory.buffer,pointer,bytes.length).set(bytes);
 const status=wasm.defense_init_scene(bytes.length);
 if(status!==0)throw Error(['','64 KiB 초과','UTF-8 오류','지원하지 않는 장면 또는 값'][status]||'장면 오류');
 seq=0;runRecorder=createRunRecorder(sceneInitial(bytes));resetView();
}
async function loadEmbeddedScene(wasmBytes){
 const scene=document.getElementById('authored-scene'),receiptNode=document.getElementById('authored-scene-receipt');
 if(!scene&&!receiptNode)return false;
 if(!scene||!receiptNode)throw Error('내장 장면 증빙 누락');
 const encoded=scene.textContent.trim();
 if(encoded.length>87384||encoded.length%4!==0||! /^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(encoded))throw Error('내장 장면 인코딩 오류');
 const decoded=atob(encoded),bytes=Uint8Array.from(decoded,v=>v.charCodeAt(0));
 if(bytes.length>65536||btoa(decoded)!==encoded)throw Error('내장 장면 크기 또는 인코딩 오류');
 if(receiptNode.textContent.length>1024)throw Error('내장 장면 증빙 크기 오류');
 const receipt=JSON.parse(receiptNode.textContent);
 if(receipt?.version!==1||receipt.sceneBytes!==bytes.length||!/^[0-9a-f]{64}$/.test(receipt.sceneSha256)||!/^[0-9a-f]{64}$/.test(receipt.wasmSha256)||!/^[0-9a-f]{16}$/.test(receipt.initialChecksum))throw Error('내장 장면 증빙 오류');
 const hash=async data=>Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',data)),v=>v.toString(16).padStart(2,'0')).join('');
 if(await hash(bytes)!==receipt.sceneSha256||await hash(wasmBytes)!==receipt.wasmSha256)throw Error('내장 장면 해시 불일치');
 loadSceneBytes(bytes);
 if(state.tick!==0||state.checksum!==receipt.initialChecksum)throw Error('내장 장면 초기 체크섬 불일치');
 $('scene-source').textContent='내장 제작 장면';
 notice('제작 장면이 준비되었습니다. 방어 시작을 누르세요.');
 return true;
}
// Local bytes only: never upload scene contents or fetch a file-derived URL.
async function loadSceneFile(file){
 if(startupPending)return;
 const generation=invalidateRunImport();
 if(!file||!wasm)return;
 const capacity=wasm.defense_scene_capacity();
 if(file.size>capacity){notice('장면은 64 KiB 이하여야 합니다. 기존 게임을 유지합니다.');return;}
 try{
  const bytes=new Uint8Array(await file.arrayBuffer());
  if(generation!==sceneLoadGeneration)return;
  loadSceneBytes(bytes);
  $('scene-source').textContent=file.name;
  notice('로컬 장면을 불러왔습니다. 방어 시작을 누르세요.');
 }catch(error){if(generation===sceneLoadGeneration)notice('장면을 불러오지 못했습니다. 기존 게임 유지: '+error.message);}
}
$('scene-file').onchange=e=>{void loadSceneFile(e.target.files?.[0]);e.target.value='';};
function togglePause(){if(runImportActive||!state||ready||state.phase)return;cancelGesture();paused=!paused;acc=0;$('pause').textContent=paused?'계속':'정지';notice(paused?'일시정지 · 계속 버튼 또는 Space로 재개':'수호대를 끌거나 탭해서 배치하세요.');update();}
$('summon').onclick=()=>command(1);$('upgrade').onclick=()=>command(3);$('invest').onclick=()=>command(4);$('again').onclick=start;$('reset').onclick=start;
$('begin').onclick=e=>{if(sound)void presentation.audio.unlock(e);if(runImportActive||startupPending||!wasm||!state)return;ready=false;paused=false;last=0;acc=0;$('ready').hidden=true;notice('소환 버튼으로 첫 수호대를 불러오세요!');update();};
$('pause').onclick=togglePause;$('sound').onclick=e=>{sound=!sound;void presentation.audio.setMuted(!sound);if(sound)void presentation.audio.unlock(e);$('sound').textContent=sound?'♪ ON':'♪ OFF';$('sound').setAttribute?.('aria-label',sound?'소리 끄기':'소리 켜기');};
addEventListener('keydown',e=>{if(runImportActive)return;if(e.repeat||['INPUT','TEXTAREA','SELECT'].includes(e.target?.tagName))return;if(e.target instanceof HTMLButtonElement&&['Space','Enter'].includes(e.code))return;const actions={KeyQ:1,KeyE:3,KeyR:4};if(actions[e.code]){e.preventDefault();command(actions[e.code]);}if(e.code==='Space'){e.preventDefault();togglePause();}if(e.code==='Escape'){cancelGesture();selected=-1;update();}});
document.addEventListener('visibilitychange',()=>{if(runImportActive)return;presentation.setHidden(document.hidden);if(document.hidden){cancelGesture();queue=[];if(state&&!paused){paused=true;$('pause').textContent='계속';acc=0;update();}}});
addEventListener('blur',()=>{if(runImportActive)return;cancelGesture();queue=[];if(state&&!paused&&!ready&&!state.phase){paused=true;acc=0;$('pause').textContent='계속';notice('창을 벗어나 자동으로 일시정지했어요.');update();}});addEventListener('resize',()=>{if(runImportActive)return;cancelGesture();dimensions();});
function pointerPoint(e){const r=canvas.getBoundingClientRect();return[(e.clientX-r.left)*canvas.width/r.width,(e.clientY-r.top)*canvas.height/r.height];}
function cellAt(p){return state?.cells.filter(a=>a[0]===0).find(a=>{const q=project(a[2],a[3],0);return Math.abs(p[0]-q[0])<=34&&Math.abs(p[1]-q[1])<=34;})?.[1]??-1;}
canvas.addEventListener('pointerdown',e=>{if(e.button!==undefined&&e.button!==0)return;if(blocked()){notice(blocked());return;}if(gesture)return;const p=pointerPoint(e),cell=cellAt(p);if(cell<0)return;e.preventDefault?.();gesture={id:e.pointerId,start:p,p,cell,source:unitAt(cell)?cell:-1,selectedBefore:selected,drag:false};canvas.setPointerCapture?.(e.pointerId);});
canvas.addEventListener('pointermove',e=>{if(runImportActive)return;const p=pointerPoint(e);hover=cellAt(p);if(!gesture||gesture.id!==e.pointerId)return;gesture.p=p;if(gesture.source>=0&&Math.hypot(p[0]-gesture.start[0],p[1]-gesture.start[1])>8){gesture.drag=true;selected=gesture.source;}if(gesture.drag)e.preventDefault?.();});
canvas.addEventListener('pointerup',e=>{if(runImportActive)return;if(!gesture||gesture.id!==e.pointerId)return;const g=gesture,target=cellAt(pointerPoint(e));cancelGesture();if(blocked()){notice(blocked());return;}if(g.drag){if(target<0){notice('이동 취소 · 내 보드 안에 놓아 주세요.');selected=g.source;}else if(target!==g.source)command(2,g.source,target);}else if(target===g.cell){if(g.selectedBefore>=0&&unitAt(g.selectedBefore)&&target!==g.selectedBefore){const a=unitAt(g.selectedBefore),b=unitAt(target);if(b&&(a[2]!==b[2]||a[3]!==b[3]||a[3]>=5)){selected=target;notice('수호대 선택 · 빈 칸이나 같은 역할·등급 위에 놓으세요.');}else command(2,g.selectedBefore,target);}else{selected=unitAt(target)?(g.selectedBefore===target?-1:target):-1;notice(selected>=0?'빈 칸으로 이동 · 빛나는 같은 수호대 위에 놓으면 합성':'수호대를 선택하거나 소환하세요.');}}update();});
canvas.addEventListener('pointercancel',()=>{if(runImportActive)return;cancelGesture();notice('이동을 취소했습니다.');});canvas.addEventListener('lostpointercapture',()=>{if(runImportActive)return;gesture=null;hover=-1;});canvas.addEventListener('pointerleave',()=>{if(runImportActive)return;if(!gesture)hover=-1;});
function update(){updateRunSave();syncPresentation();if(!state)return;$('health').textContent=state.hp;$('healthbar').style.width=Math.max(0,Math.min(100,state.hp/state.maxHp*100))+'%';$('wave').textContent=`${String(state.wave).padStart(2,'0')} / ${state.lastWave}`;$('campaign-intro').textContent=`최대 ${state.lastWave} 웨이브, 숲을 함께 지켜요.`;$('gold').textContent=state.gold[0];$('kills').textContent=state.kills;for(const [i,id]of['summon','upgrade','invest'].entries()){const b=$(id);b.querySelector('span').textContent=`● ${state.costs[i]} · ${['Q','E','R'][i]}`;b.disabled=false;b.setAttribute?.('aria-disabled',String(!!actionReason([1,3,4][i],0,0)));}const u=unitAt(selected);$('selected').textContent=u?`${roles[u[2]][0]} ${'★'.repeat(u[3])}`:'수호대를 소환하세요';$('detail').textContent=u?roles[u[2]][1]+' · 끌기 / 탭으로 이동':'같은 역할·등급 둘을 합성 · 소환은 첫 빈 칸';if(state.phase){cancelGesture();$('result').hidden=false;$('outcome').textContent=state.phase===1?'숲을 지켜냈어요!':'다시 함께 도전해요';$('summary').textContent=`${state.wave} 웨이브 · ${state.kills} 처치 · ${state.merges} 합성 · ${state.leaks} 돌파`;}}
function recordedStep(a){wasm.defense_step(...a);runRecorder.record(a);}
function execute(a){if(runImportActive)return;const before=state;recordedStep(a);state=snapshot();observePresentation();if(!a[0])return;let success=false,message='',f=440;if(a[0]===1){success=state.units.filter(u=>u[0]===0).length>before.units.filter(u=>u[0]===0).length;message='새 수호대가 합류했어요!';f=530;}if(a[0]===3){success=state.upgrades[0]>before.upgrades[0];message='내 수호대의 공격력이 올랐어요!';}if(a[0]===4){success=state.investment[0]>before.investment[0];message='경제 투자를 완료했어요!';f=350;}if(a[0]===2){const old=before.units.find(u=>u[0]===0&&u[1]===a[1]),now=unitAt(a[2]),wasTarget=before.units.find(u=>u[0]===0&&u[1]===a[2]);success=!!old&&!!now&&!unitAt(a[1])&&now[2]===old[2]&&now[3]===old[3]+(wasTarget?1:0);message=wasTarget?'합성 성공! 더 강한 수호대가 되었어요.':'배치를 옮겼어요.';if(success){selected=a[2];}}if(success){notice(message);if(a[0]!==2)tone(f);}else notice('입력이 적용되지 않았어요. 현재 배치와 자원을 확인하세요.');}
function circle(x,y,r,fill,stroke,width=1){c.beginPath();c.arc(x,y,r,0,Math.PI*2);if(fill){c.fillStyle=fill;c.fill();}if(stroke){c.strokeStyle=stroke;c.lineWidth=width;c.stroke();}}
function line(points,color,width){c.beginPath();points.forEach((p,i)=>i?c.lineTo(...p):c.moveTo(...p));c.strokeStyle=color;c.lineWidth=width;c.lineCap='round';c.lineJoin='round';c.stroke();}
function text(t,x,y,color='#36543b',size=12,align='center'){c.fillStyle=color;c.font=`800 ${size}px system-ui,sans-serif`;c.textAlign=align;c.fillText(t,x,y);}
function rounded(x,y,w,h,r,fill,stroke){c.beginPath();c.roundRect(x,y,w,h,r);c.fillStyle=fill;c.fill();if(stroke){c.strokeStyle=stroke;c.lineWidth=2;c.stroke();}}
function drawHero(u,pos=null,opacity=1,poseOverride=null,frameOverride=null){const p=pos||project(u[5],u[6],u[0]),s=scale(u[0]),role=u[2],col=roles[role][2],pose=poseOverride||presentation.pose(visualAdapter.unitId(u)),frame=frameOverride||presentation.frame(visualAdapter.unitId(u),pose);c.save();c.translate(...p);c.scale(s,s);c.globalAlpha=opacity;circle(0,15,24,'#315a4333');circle(0,6,23,'#fff4c9',col,2);c.save();c.translate(pose.x,pose.y);c.rotate(pose.rotation);c.scale(pose.scaleX,pose.scaleY);if(pose.flash)c.filter=`brightness(${1+pose.flash*1.4})`;if(!paintFrame(frame,animationImages.get(frame?.image)||towerAtlas,70,heroRects?.[role])){const ears=role===3?26:12;circle(-11,-29,ears/2,col,'#514c37',2);circle(11,-29,ears/2,col,'#514c37',2);circle(0,-10,18,col,'#514c37',2);circle(-6,-11,2.5,'#fff');circle(6,-11,2.5,'#fff');circle(-6,-10,1.3,'#352e29');circle(6,-10,1.3,'#352e29');circle(0,-2,5,'#f7e0af');line([[20,-8],[28,-28]],'#fff1b2',5);circle(28,-30,5,col);}c.restore();rounded(-20,23,40,14,5,'#fff7d8',col);text('★'.repeat(u[3]),0,34,col,9);c.restore();}
function drawEnemy(e,ghost=null,ghostPose=null,ghostFrame=null){const p=ghost?[ghost.x,ghost.y]:project(e[6],e[7],e[1]),s=ghost?.scale||scale(e[1]),boss=e[2]===4,r=boss?22:12,pose=ghostPose||presentation.pose(visualAdapter.enemyId(e)),frame=ghostFrame||presentation.frame(visualAdapter.enemyId(e),pose);c.save();c.translate(...p);c.scale(s,s);c.globalAlpha=pose.opacity;circle(0,6,r+2,'#315a4333');c.save();c.translate(pose.x,pose.y);c.rotate(pose.rotation);c.scale(pose.scaleX,pose.scaleY);if(pose.flash)c.filter=`brightness(${1+pose.flash*1.7})`;if(!paintFrame(frame,animationImages.get(frame?.image)||enemyAtlas,boss?65:37,enemyRects?.[[2,0,1,0,3][e[2]]??0])){circle(0,0,r,e[5]?'#82c9d0':boss?'#9d79ad':'#7fac65','#426345',2);circle(-r*.35,-2,3,'#fff4d7');circle(r*.35,-2,3,'#fff4d7');circle(-r*.35,-2,1.5,'#473b3a');circle(r*.35,-2,1.5,'#473b3a');}c.restore();if(ghost){c.restore();return;}rounded(-r,-r-11,r*2,5,2,'#666746');c.fillStyle='#ef8070';c.fillRect(-r,-r-11,r*2*Math.max(0,e[3]/e[4]),5);c.restore();}
function draw(){const W=canvas.width,H=canvas.height,m=mobile();c.clearRect(0,0,W,H);c.fillStyle='#b8d58f';c.fillRect(0,0,W,H);for(let i=0;i<70;i++){const x=(i*173+29)%W,y=(i*97+13)%H;circle(x,y,3+i%5,i%3?'#94bd783c':'#e4e9b680');}if(!state)return;
for(let b=0;b<2;b++){const path=state.paths[b].slice(0,-1).map(p=>project(...p,b));for(let i=1;i<=40;i++)path.push(project((50+b*500)+(500-(50+b*500))*i/40,590,b));line(path,'#7f9e61',39*scale(b)+6);line(path,'#e8d6a0',34*scale(b));line(path,'#f5e5b8',23*scale(b));const cells=state.cells.filter(a=>a[0]===b),points=cells.map(a=>project(a[2],a[3],b)),s=scale(b),minX=points[0][0]-40*s,minY=points[0][1]-40*s,w=335*s,h=230*s;rounded(minX-5,minY+5,w+10,h+5,13*s,'#729b58');rounded(minX-5,minY-5,w+10,h,13*s,'#e2e8b3','#83a366');text(b?'동료 · 로컬 AI':'내 수호대',minX+w/2,minY-16,b?'#577148':'#385d3b',b&&m?12:16);for(const [board,cell,x,y]of cells){const p=project(x,y,b),a=unitAt(selected),v=unitAt(cell),legal=b===0&&a&&cell!==selected&&!actionReason(2,selected,cell),merge=legal&&v;rounded(p[0]-33*s,p[1]-33*s,66*s,66*s,7*s,(cell+Math.floor(cell/4))%2?'#c3d89a':'#d6e3ae');if(b===0&&(cell===selected||merge||hover===cell)){c.save();c.setLineDash(merge?[4,3]:[]);rounded(p[0]-34,p[1]-34,68,68,8,cell===selected?'#ffdd6555':legal?'#fff49b66':'#ed807055',cell===selected?'#bf9027':legal?'#4b9b60':'#bb6855');c.restore();}if(gesture?.drag&&legal&&!v)circle(p[0],p[1],7,'#639d6d88');}if(b===1)text(`● ${state.gold[1]} · ${state.units.filter(u=>u[0]===1).length}/12`,minX+w/2,minY+h+23,'#5d7451',m?10:12);}
const chosen=unitAt(selected);if(chosen&&!gesture?.drag&&chosen[2]<4){const radius=chosen[2]===1?600:390;for(const e of state.enemies){if(e[1]!==0&&!(chosen[2]===1&&e[7]>=590))continue;if(Math.abs(chosen[5]-e[6])+Math.abs(chosen[6]-e[7])<=radius){const p=project(e[6],e[7],e[1]);circle(...p,18*scale(e[1]),'#fff5b522','#f6b950',1);}}text(`공격 범위 ${radius} · 금빛 원의 적을 공격 가능`,m?250:265,m?615:485,'#6b7c45',m?10:11);}
const z=m?[250,665]:[520,565];circle(z[0],z[1]+12,43,'#74935c55');rounded(z[0]-27,z[1]-20,54,45,7,'#fbebc8','#a68c60');for(const dx of[-28,18]){rounded(z[0]+dx,z[1]-34,12,50,3,'#fff1cd','#a68c60');for(let k=0;k<3;k++)c.fillRect(z[0]+dx+k*4,z[1]-38,3,7);}rounded(z[0]-8,z[1]+1,16,23,7,'#9d7855');circle(z[0],z[1]-13,8,'#77c7d0','#538d9c');text('함께 지키는 성',z[0],z[1]+44,'#587148',11);
presentation.draw(c,'under');presentation.drawGhosts((actor,pose,frame)=>{if(!actor.renderData)return;if(actor.visualKey.startsWith('enemy:'))drawEnemy(actor.renderData,actor,pose,frame);else drawHero(actor.renderData,[actor.x,actor.y],pose.opacity,pose,frame);});
state.units.slice().sort((a,b)=>a[6]-b[6]).forEach(u=>drawHero(u,null,gesture?.drag&&u[0]===0&&u[1]===gesture.source ? .8 : 1));state.enemies.forEach(e=>drawEnemy(e));
presentation.draw(c,'over');if(gesture?.drag){const u=unitAt(gesture.source);if(u){drawHero(u,[gesture.p[0],gesture.p[1]-18],.85);const target=cellAt(gesture.p),reason=target<0?'내 보드 안에 놓기':actionReason(2,gesture.source,target),label=target===gesture.source?'원래 자리':reason|| (unitAt(target)?'합성!':'이동');text(label,gesture.p[0],gesture.p[1]-65,reason?'#af5449':'#366b43',13);}}
feedback=feedback.filter(f=>f.life-->0);for(const f of feedback){c.globalAlpha=f.life/32;circle(f.x,f.y,30+(32-f.life),null,'#fff5b0',3);text(f.label,f.x,f.y-35-(32-f.life),'#8a641e',17);}c.globalAlpha=1;const bosses=state.enemies.filter(e=>e[2]===4);if(bosses.length){const hp=bosses.reduce((n,e)=>n+e[3],0),max=bosses.reduce((n,e)=>n+e[4],0),x=m?125:370,y=m?236:30,w=m?250:260;rounded(x,y,w,13,6,'#8d7565');rounded(x,y,w*Math.max(.01,hp/max),13,6,'#d67b71');text('우두머리',x+w/2,y-7,'#895543',12);}if(paused&&!ready&&!state.phase){c.fillStyle='#eff4d5bb';c.fillRect(0,0,W,H);text('잠깐 쉬어가기',W/2,H/2,'#3c6949',27);text('계속 버튼 또는 Space',W/2,H/2+30,'#647b54',13);}}
function animate(time){if(runImportActive){requestAnimationFrame(animate);return;}const delta=Math.min(100,time-(last||time));last=time;if(wasm&&state&&!paused&&!ready&&!state.phase){acc+=delta;let ticks=0;while(acc>=1000/60&&ticks++<6){execute(queue.shift()||[0,0,0,0]);acc-=1000/60;}update();}presentation.advance(delta);draw();requestAnimationFrame(animate);}
// Authored delivery admits the existing FPS/rectangle contract, not another animation format.
function animationObject(value){return value!==null&&typeof value==='object'&&!Array.isArray(value);}
function animationKeys(value,required,optional=[]){
 if(!animationObject(value)||required.some(key=>!Object.hasOwn(value,key))||Object.keys(value).some(key=>!required.includes(key)&&!optional.includes(key)))throw Error('Animation object fields invalid');
}
function parseAnimationJson(text,limit=65536){
 if(typeof text!=='string'||text.length>limit||new TextEncoder().encode(text).length>limit)throw Error('Animation JSON exceeds byte limit');
 let offset=0;
 const space=()=>{while(/[ \t\r\n]/.test(text[offset]||'!'))offset++;};
 const string=()=>{const match=/^"(?:[^"\\\u0000-\u001f]|\\(?:["\\/bfnrt]|u[0-9a-fA-F]{4}))*"/.exec(text.slice(offset));if(!match)throw Error('Animation JSON string invalid');offset+=match[0].length;return JSON.parse(match[0]);};
 const value=depth=>{
  if(depth>16)throw Error('Animation JSON nesting exceeds limit');space();const ch=text[offset];
  if(ch==='"')return string();
  if(ch==='{'||ch==='['){
   const object=ch==='{',out=object?Object.create(null):[];offset++;space();
   if(text[offset]===(object?'}':']')){offset++;return out;}
   for(;;){
    space();let key;if(object){key=string();if(Object.hasOwn(out,key))throw Error('Animation JSON duplicate key');space();if(text[offset++]!==':')throw Error('Animation JSON colon missing');}
    const child=value(depth+1);if(object)out[key]=child;else out.push(child);space();
    const separator=text[offset++];if(separator===(object?'}':']'))return out;if(separator!==',')throw Error('Animation JSON separator invalid');
   }
  }
  const token=/^(?:true|false|null|-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?)/.exec(text.slice(offset));
  if(!token)throw Error('Animation JSON value invalid');offset+=token[0].length;const result=JSON.parse(token[0]);if(typeof result==='number'&&!Number.isFinite(result))throw Error('Animation JSON nonfinite number');return result;
 };
 const result=value(0);space();if(offset!==text.length)throw Error('Animation JSON trailing data');return result;
}
function animationImageName(name){return typeof name==='string'&&name.length<=128&&name.endsWith('.png')&&name.split('/').every(part=>/^[A-Za-z0-9_-][A-Za-z0-9_.-]*$/.test(part));}
function validateAnimationManifest(manifest,inventory=null){
 if(!animationObject(manifest)||Object.keys(manifest).length<1||Object.keys(manifest).length>11)throw Error('Animation roles invalid');
 const names=new Set(),pair=(value,valid)=>Array.isArray(value)&&value.length===2&&value.every(n=>typeof n==='number'&&Number.isFinite(n)&&valid(n));
 for(const [role,definition]of Object.entries(manifest)){
  if(!/^(?:unit:[0-5]|enemy:[0-4])$/.test(role))throw Error('Animation role invalid');
  animationKeys(definition,['image','clips'],['anchor','pivot','logicalSize']);
  if(!animationImageName(definition.image))throw Error('Animation image path invalid');names.add(definition.image);
  for(const [key,valid]of [['anchor',n=>Math.abs(n)<=16],['pivot',n=>Math.abs(n)<=4096],['logicalSize',n=>n>0&&n<=4096]])if(Object.hasOwn(definition,key)&&!pair(definition[key],valid))throw Error('Animation '+key+' invalid');
  const size=inventory?.get(definition.image);if(inventory&&!size)throw Error('Animation image inventory missing');
  const width=size?.width??4096,height=size?.height??4096;
  if(!animationObject(definition.clips)||Object.keys(definition.clips).length<1||Object.keys(definition.clips).length>5)throw Error('Animation clips invalid');
  for(const [name,clip]of Object.entries(definition.clips)){
   if(!['idle','move','attack','hit','death'].includes(name))throw Error('Animation clip invalid');
   animationKeys(clip,['frames'],['fps','loop','pivots',...(name==='attack'?['windupMs','releaseMs','recoveryMs']:[])]);
   if(!Array.isArray(clip.frames)||clip.frames.length<1||clip.frames.length>256)throw Error('Animation frame count invalid');
   for(const rect of clip.frames)if(!Array.isArray(rect)||rect.length!==4||!rect.every(n=>typeof n==='number'&&Number.isFinite(n))||rect[0]<0||rect[1]<0||rect[2]<=0||rect[3]<=0||rect[0]+rect[2]>width||rect[1]+rect[3]>height)throw Error('Animation rectangle invalid');
   if(Object.hasOwn(clip,'fps')&&(!Number.isFinite(clip.fps)||clip.fps<1||clip.fps>120))throw Error('Animation fps invalid');
   if(Object.hasOwn(clip,'loop')&&typeof clip.loop!=='boolean')throw Error('Animation loop invalid');
   if(Object.hasOwn(clip,'pivots')&&(!Array.isArray(clip.pivots)||clip.pivots.length!==clip.frames.length||!clip.pivots.every(p=>pair(p,n=>Math.abs(n)<=4096))))throw Error('Animation pivots invalid');
   for(const key of ['windupMs','releaseMs','recoveryMs'])if(Object.hasOwn(clip,key)&&(!Number.isFinite(clip[key])||clip[key]<1||clip[key]>2000))throw Error('Animation timing invalid');
  }
 }
 return names;
}
function animationBase64(node,limit){
 const encoded=node.textContent.trim();if(encoded.length>4*Math.ceil(limit/3)||encoded.length%4||!/^[A-Za-z0-9+/]*={0,2}$/.test(encoded))throw Error('Animation base64 invalid');
 const raw=atob(encoded);if(raw.length>limit||btoa(raw)!==encoded)throw Error('Animation base64 noncanonical');return {bytes:Uint8Array.from(raw,v=>v.charCodeAt(0)),encoded};
}
// Envelope/CRC validation bounds work and excludes APNG; Image still performs the real pixel decode.
function validateAnimationPng(bytes,width,height){
 if(bytes.length<33||![137,80,78,71,13,10,26,10].every((n,i)=>bytes[i]===n))throw Error('Animation PNG signature invalid');
 const view=new DataView(bytes.buffer,bytes.byteOffset,bytes.byteLength),table=new Uint32Array(256);
 for(let i=0;i<256;i++){let value=i;for(let bit=0;bit<8;bit++)value=(value>>>1)^((value&1)?0xedb88320:0);table[i]=value>>>0;}
 let offset=8,headers=0,idat=false,ended=false;
 while(offset<bytes.length){
  if(bytes.length-offset<12)throw Error('Animation PNG chunk truncated');
  const length=view.getUint32(offset),end=offset+12+length,kind=String.fromCharCode(...bytes.subarray(offset+4,offset+8));
  if(end>bytes.length||!/^[A-Za-z]{4}$/.test(kind))throw Error('Animation PNG chunk invalid');
  let crc=0xffffffff;for(let i=offset+4;i<end-4;i++)crc=(crc>>>8)^table[(crc^bytes[i])&255];
  if(((crc^0xffffffff)>>>0)!==view.getUint32(end-4))throw Error('Animation PNG CRC invalid');
  if(['acTL','fcTL','fdAT'].includes(kind)||!(bytes[offset+4]&32)&&!['IHDR','PLTE','IDAT','IEND'].includes(kind))throw Error('Animation PNG unsupported chunk');
  if(offset===8&&(kind!=='IHDR'||length!==13))throw Error('Animation PNG header invalid');
  if(kind==='IHDR'){headers++;if(headers!==1||length!==13||view.getUint32(offset+8)!==width||view.getUint32(offset+12)!==height)throw Error('Animation PNG dimensions invalid');}
  if(kind==='IDAT')idat=true;
  offset=end;
  if(kind==='IEND'){if(length||offset!==bytes.length)throw Error('Animation PNG end invalid');ended=true;break;}
 }
 if(headers!==1||!idat||!ended)throw Error('Animation PNG incomplete');
}
async function animationHash(bytes){return Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes)),v=>v.toString(16).padStart(2,'0')).join('');}
async function embeddedAnimations(image){
 const nodes=[...document.querySelectorAll('[id^="authored-animation-"]')];
 if(!nodes.length)return null;
 for(const id of ['wasm-core','tower-atlas','enemy-atlas','character-atlas-meta','authored-scene','authored-scene-receipt'])if(document.querySelectorAll('[id="'+id+'"]').length>1)throw Error('Animation duplicate dependency ID');
 const byId=new Map();for(const node of nodes){if(byId.has(node.id))throw Error('Animation duplicate element ID');byId.set(node.id,node);}
 const manifestNode=byId.get('authored-animation-manifest'),receiptNode=byId.get('authored-animation-receipt');
 if(!manifestNode||!receiptNode||$('combat-animation-manifest'))throw Error('Animation embedded payload incomplete or ambiguous');
 if(manifestNode.tagName!=='SCRIPT'||manifestNode.type!=='application/octet-stream'||receiptNode.tagName!=='SCRIPT'||receiptNode.type!=='application/json')throw Error('Animation payload element invalid');
 const {bytes:manifestBytes}=animationBase64(manifestNode,65536),receipt=parseAnimationJson(receiptNode.textContent,8192);
 animationKeys(receipt,['version','manifestBytes','manifestSha256','images']);
 if(receipt.version!==1||receipt.manifestBytes!==manifestBytes.length||!/^[0-9a-f]{64}$/.test(receipt.manifestSha256)||await animationHash(manifestBytes)!==receipt.manifestSha256)throw Error('Animation manifest receipt mismatch');
 const manifest=parseAnimationJson(new TextDecoder('utf-8',{fatal:true}).decode(manifestBytes)),names=validateAnimationManifest(manifest);
 if(!Array.isArray(receipt.images)||receipt.images.length!==names.size||receipt.images.length>11||nodes.length!==receipt.images.length+2)throw Error('Animation image inventory invalid');
 const inventory=new Map(),payloads=[];let total=0;
 for(const [index,entry]of receipt.images.entries()){
  animationKeys(entry,['name','bytes','sha256','width','height','id']);
  if(!names.has(entry.name)||inventory.has(entry.name)||entry.id!==`authored-animation-image-${index}`||!Number.isInteger(entry.bytes)||entry.bytes<1||entry.bytes>8388608||!Number.isInteger(entry.width)||entry.width<1||entry.width>4096||!Number.isInteger(entry.height)||entry.height<1||entry.height>4096||!/^[0-9a-f]{64}$/.test(entry.sha256))throw Error('Animation image receipt invalid');
  total+=entry.bytes;if(total>33554432)throw Error('Animation image total exceeds limit');
  const node=byId.get(entry.id);if(!node||node.tagName!=='SCRIPT'||node.type!=='application/octet-stream')throw Error('Animation image payload missing');
  const {bytes,encoded}=animationBase64(node,8388608);
  if(bytes.length!==entry.bytes||await animationHash(bytes)!==entry.sha256)throw Error('Animation PNG bytes or hash invalid');
  validateAnimationPng(bytes,entry.width,entry.height);
  inventory.set(entry.name,entry);payloads.push([entry,encoded]);
 }
 validateAnimationManifest(manifest,inventory);
 const images=new Map();for(const [entry,encoded]of payloads){const decoded=await image('data:image/png;base64,'+encoded,true);if(decoded.naturalWidth!==entry.width||decoded.naturalHeight!==entry.height)throw Error('Animation decoded dimensions mismatch');images.set(entry.name,decoded);}
 return {manifest,images};
}
async function loadAtlas(){
 async function image(src,required=false){return new Promise((resolve,reject)=>{const i=new Image();i.onload=()=>resolve(i);i.onerror=()=>required?reject(Error('Animation PNG decode failed')):resolve(null);i.src=src;});}
 // Build a complete replacement off to the side. A rejected authored payload cannot partially adopt images or roles.
 const embedded=$('tower-atlas'),nextTower=await image(embedded?'data:image/png;base64,'+embedded.textContent.trim():new URL('assets/units.png',document.baseURI).href);
 const meta=$('character-atlas-meta');let data=null,nextEnemy=null;
 try{data=meta?JSON.parse(meta.textContent):await(await fetch(new URL('assets/characters.json',document.baseURI))).json();}catch{}
 if(data?.enemies){const ee=$('enemy-atlas');nextEnemy=await image(ee?'data:image/png;base64,'+ee.textContent.trim():new URL('assets/'+(data.enemyImage||'enemies.png'),document.baseURI).href);}
 const nextImages=new Map([['units.png',nextTower],['enemies.png',nextEnemy]]),manifest={};
 // Existing single images remain explicit procedural fallbacks, not authored clips.
 data?.heroes?.forEach((r,i)=>{manifest[`unit:${i}`]={image:'units.png',anchor:[.5,1],clips:{idle:{frames:[r],fps:1,loop:true}}};});
 [2,0,1,0,3].forEach((index,i)=>{if(data?.enemies?.[index])manifest[`enemy:${i}`]={image:'enemies.png',anchor:[.5,1.16],clips:{idle:{frames:[data.enemies[index]],fps:1,loop:true}}};});
 const embeddedAuthored=await embeddedAnimations(image),animationSource=$('combat-animation-manifest');
 if(embeddedAuthored){Object.assign(manifest,embeddedAuthored.manifest);for(const [name,value]of embeddedAuthored.images)nextImages.set(name,value);}
 else if(animationSource){
  if(document.querySelectorAll('[id="combat-animation-manifest"]').length!==1)throw Error('Animation duplicate manifest ID');
  const spec=parseAnimationJson(animationSource.textContent);let authored=spec;
  if(animationObject(spec)&&Object.hasOwn(spec,'src')){
   animationKeys(spec,['src']);if(typeof spec.src!=='string'||!spec.src||spec.src.length>1024)throw Error('Animation source invalid');
   const response=await fetch(new URL(spec.src,document.baseURI));if(!response.ok)throw Error('Animation source fetch failed');
   // Bound streamed external manifests before accumulating them in memory.
   const reader=response.body.getReader(),chunks=[];let length=0;
   try{for(;;){const {done,value}=await reader.read();if(done)break;length+=value.length;if(length>65536)throw Error('Animation JSON exceeds byte limit');chunks.push(value);}}finally{await reader.cancel();}
   const bytes=new Uint8Array(length);let offset=0;for(const chunk of chunks){bytes.set(chunk,offset);offset+=chunk.length;}
   authored=parseAnimationJson(new TextDecoder('utf-8',{fatal:true}).decode(bytes));
  }
  const names=validateAnimationManifest(authored),inventory=new Map();
  for(const name of names){let value=nextImages.get(name);if(!value)value=await image(new URL('assets/'+name,document.baseURI).href,true);nextImages.set(name,value);inventory.set(name,{width:value.naturalWidth,height:value.naturalHeight});}
  validateAnimationManifest(authored,inventory);Object.assign(manifest,authored);
 }
 towerAtlas=nextTower;enemyAtlas=nextEnemy;heroRects=data?.heroes??null;enemyRects=data?.enemies??null;
 animationImages.clear();for(const [name,value]of nextImages)animationImages.set(name,value);presentation.setManifest(manifest);
}
try{let bytes;startupPending=true;const embedded=document.getElementById('wasm-core');if(embedded){bytes=Uint8Array.from(atob(embedded.textContent.trim()),v=>v.charCodeAt(0));}else{const res=await fetch(new URL('game.wasm',import.meta.url));if(!res.ok)throw Error(`HTTP ${res.status}`);bytes=await res.arrayBuffer();}loadedWasmBytes=new Uint8Array(bytes).slice();loadedWasmSha256=await sha256(loadedWasmBytes);const module=await WebAssembly.instantiate(loadedWasmBytes,{});wasm=module.instance.exports;await loadAtlas();dimensions();const authoredReady=await loadEmbeddedScene(bytes);startupPending=false;if(!authoredReady)start();$('loading').hidden=true;window.defenseDebug={presentationStats:()=>({...presentation.stats(),transport:visualAdapter.stats()}),presentationPose:id=>presentation.pose(id),presentationFrame:id=>presentation.frame(id),snapshot:()=>snapshot(),command,start,setPaused:v=>{if(runImportActive)return;paused=v;update();},step:n=>{if(runImportActive)return;for(let i=0;i<n;i++)execute(queue.shift()||[0,0,0,0]);presentation.advance(1000/60);update();draw();}};addEventListener('pagehide',e=>{if(runImportBusy){invalidateRunImport();return;}paused=true;queue=[];cancelGesture();syncPresentation();if(!e.persisted)presentation.dispose();});addEventListener('pageshow',e=>{if(e.persisted){$('pause').textContent='계속';update();}});requestAnimationFrame(animate);}catch(e){wasm=null;state=undefined;queue=[];paused=true;$('ready').hidden=true;$('loading').hidden=false;$('loading').textContent='게임 코어를 불러오지 못했습니다. '+e.message;console.error(e);}
