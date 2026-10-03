# ADR 초안: 최소 에셋 파이프라인 v1

상태: **core/cooker/static fixture 구현, opt-in audio 집중 검증 / 최종 원격 통합 CI 대기**. 추적: [#24](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/24). 2026-10-03 기준.

## 1. 결정 요약

첫 경로는 `MotionProfileV1`(FP 이동량 한 필드)와 `ImpactPcm16V1`(절차 생성 효과음 한 종류)만 지원한다. 소스 파일을 명시적 GUID로 등록하고, 재현 가능한 cooker가 정렬된 manifest와 content-addressed artifact를 만든다. sim은 빌드에 포함된 읽기 전용 typed table만 참조한다. view는 시작 시 검증·디코딩한 clip을 소유한다. 틱/오디오 콜백에서 파일을 열거나 cook하지 않는다.

기존 Arena/PhysGame 기본 경로, 골든, ORRF v2와 ORRP v3를 변경하지 않는다. 별도 opt-in fixture game으로 참조→cook→resolve→tick→replay 및 event→clip 경로를 증명한다. 범용 런타임 sim-resource 주입이나 외부 sim bundle 교체는 후속 ADR로 남긴다. 정적 sim table을 선택한 이유는 현재 `Game::systems()`와 replay 복원이 config를 시스템에 전달하는 에셋 저장소 API를 제공하지 않기 때문이다. 제안 API를 이미 존재하는 기능처럼 설명하지 않는다.

## 2. 현재 근거와 제안의 구분

읽은 로컬 checkout: `d71fcc426536f60d8ddb75c961ada22944754235`. 로컬이 원격보다 뒤처져 있어 PR #30 및 `87e4a526e0e2cb8726cfc4c06de01f7cfea6a41c`의 deployment tool, frame compatibility, system interface를 GitHub connector로 추가 확인했다. 구현 착수자는 최신 충돌 파일과 기준 HEAD를 다시 확인한다.

현재 구현 (아래는 설계 작성 시점의 조사 기록):
- `docs/design-v1.md` §2·§3.3의 `orr_asset` 및 GUID-u64 `AssetRef`는 설계 방향이다. workspace에는 `orr_asset` crate와 asset DB/cooker가 없다
- `crates/orr_sim/src/game.rs`, `system.rs`, `context.rs`, `simulation.rs`: `Game::setup(&mut Frame, &Config)`, 인자 없는 `Game::systems()`, `System::run(&mut SimContext)`가 존재한다. 현재 `SimContext`에는 asset store가 없고 `Simulation`은 생성 시 시스템 목록을 구성한다
- `crates/orr_session/src/replay.rs`: ORRP writer v3, reader v1–v3. header에 `build_hash`는 있으나 asset digest는 없다. unchecked `seek`는 build hash를 검사하지 않는다. checked verify/seek도 header 또는 expected ID가 0이면 wildcard로 허용한다
- `docs/frame-compatibility.md`, `crates/orr_ecs/src/codec.rs`: Frame ORRF v2. `frame_build_id`는 game ID와 Frame version을 묶지만 game code 변경을 탐지하지 않는다
- 원격 `tools/deployment_manifest.py`(PR #30): `orr.deployment/1`, release owner가 부여한 nonzero `game_code_id`, `frame_build_id`로 계산한 `build_id`; provenance는 기록이지 서명이나 바이너리-소스 관계의 증명이 아니다. 이 tool의 strict schema에 임의 asset 필드를 추가하지 않는다
- `docs/audio.md`, `crates/orr_sample/src/arena_audio.rs`: 현재 Arena 효과음은 런타임 float 수식으로 생성한 clip. asset DB가 아니며 이 수식의 서로 다른 플랫폼 PCM byte 동일성을 전제하지 않는다. event 중복 억제·취소·음성 수 상한과 offline PCM 검사는 이미 존재한다
- `CLAUDE.md`: sim에 float/HashMap/시간/포인터 상태 금지. 설계 문서의 장래 no_std/병렬 실행 목표를 전체 workspace의 현재 보장으로 간주하지 않는다

설계 작성 시에는 Cargo 실행이나 production 파일 변경을 하지 않았다. #32는 `orr_asset`의 POD `AssetRef`, manifest metadata로 검증한 typed reference, 불변 sorted sim table, `MotionProfileV1` 정수 payload codec, bounded ORAM v1 sim/view manifest codec만 구현한다. canonical GUID 문자열에서 0은 null로 roundtrip하되 live manifest/table/required typed reference에서는 거부한다. SHA-256 필드는 opaque bytes이며 hashing/실제 artifact 검증은 core에 없다. 64/16 record 및 schema별 payload 길이 제한 때문에 유효한 v1 manifest의 실제 최대 길이는 각각 3600/912 bytes이며, 128 KiB 일반 입력 상한도 먼저 검사한다. view payload decoder, cooker, fixture, release binding, replay/audio 통합과 성능 검증은 아직 구현되지 않았다. 아래 후속 API/명령 예와 완료 기준을 구현 완료 주장으로 읽지 않는다.

## 3. ID와 참조 수명

`#[repr(transparent)] struct AssetRef(u64)`를 POD wire 값으로 사용한다. 값 0은 null/unset이며 필수 참조에서 오류다. 런타임 정렬은 numeric u64 오름차순이다. JSON/authoring 표현은 `a_` + 정확히 16자리 소문자 hex (`a_0000000000001001`). JavaScript Number나 YAML number를 통해 u64를 전달하지 않는다.

GUID는 경로·이름·content hash에서 유도하지 않는다. 등록 도구가 authoring 단계에서 nonzero u64를 할당하고 프로젝트 index 전체에 대한 중복 검사를 한다. 무작위 충돌 가능성을 무시하지 않으며 merge 후 중복을 CI에서 차단한다. GUID 생성의 OS RNG는 도구에만 허용된다. 동일 원본의 rename/move는 GUID를 유지하고 index의 source path만 바꾼다. 복제/import-as-new는 새 GUID다. 내용 수정은 동일 논리 에셋이면 GUID 유지, artifact hash 변경이다.

삭제는 index에 tombstone을 남긴다. 사용 중인 GUID 삭제는 의존성 검사가 거부한다. 참조를 제거한 후 삭제할 수 있지만 그 GUID를 다른 에셋에 재사용하지 않는다. schema가 의미적으로 다른 타입으로 바뀌면 새 GUID; 같은 타입의 새 schema는 명시적 version으로 cook한다. v1은 version 1만 읽고 자동 변환하지 않는다. 지원하지 않는 버전은 fail-closed다.

Entity GUID(`e_...`)와 asset GUID(`a_...`)는 별개 namespace다. 기존 `orr_reflect` entity 참조 의미를 asset 참조로 재사용하지 않는다. 첫 fixture는 asset 전용 index/parser와 typed validation만 사용하고 ERP/schema/scene serializer를 확장하지 않는다.

## 4. authoring 입력과 두 asset 형식

새 경로 예: `assets/index.json`, `assets/source/motion.json`, `assets/source/impact.json`. index entry에는 `id`, `type`, `schema_version`, `source` 또는 `tombstone`을 둔다. source는 package root 아래 상대 UTF-8 경로만 허용한다. 절대경로, `..`, 중복 정규화 경로, symlink escape는 거부한다. source 경로는 runtime manifest에 넣지 않는다. v1 dependency graph는 비어 있으며, asset 간 dependency가 선언되면 지원하지 않는 형식으로 오류를 낸다.

```json
{"id":"a_0000000000001001","type":"sim.motion_profile","schema_version":1,"source":"source/motion.json"}
{"speed_per_tick":"0.0625"}
{"id":"a_0000000000002001","type":"view.impact_pcm16","schema_version":1,"source":"source/impact.json"}
{"generator":"triangle_decay_v1","sample_rate":48000,"frames":7200,"period_frames":192,"peak_pcm16":6000}
```

위 JSON들은 각각의 독립 entry/source 예이며 하나의 JSON 문서로 이어 붙이지 않는다. index 파일은 `format: "orr.asset-index/1"`과 `entries` 배열을 갖는다. live entry는 위 네 필드만 허용하고 tombstone entry는 `id`, `tombstone: true`만 가진다. cooker는 fixture의 선언된 initial asset roots를 받아 참조된 tombstone을 거부한다. 임의의 다른 외부 문서가 가진 참조까지 자동 탐색한다고 주장하지 않는다. 중복 JSON key, unknown field, 잘못된 UTF-8, 잘린 입력, 범위 초과를 거부한다.

### sim.motion_profile/1

`MotionProfileV1 { speed_per_tick: FP }`; 허용 범위 `0 < speed_per_tick <= 16`. authoring은 문자열 decimal만 받으며 `FP::parse`의 정수-only round-nearest/ties-away-from-zero 규칙을 사용한다. locale, f64, display 재출력을 경유하지 않는다. 예시 값의 raw는 4096이다. cooker가 parse된 FP의 raw i64 little-endian 8바이트만 payload로 쓴다.

이 값은 매 tick 위치를 변경하는 읽기 전용 상수다. fixture Frame에는 `Motion { profile: AssetRef }`, `Position { x: FP }`를 두며 asset 본문·Vec·Arc·파일 핸들은 넣지 않는다. 입력 축은 -1/0/1; tick은 `x += profile.speed_per_tick * axis`로 진행한다. run length 및 좌표 상한을 고정해 overflow를 막는다. 기존 게임의 상수를 바꾸는 방식으로 골든을 재설정하지 않는다.

### view.impact_pcm16/1

150 ms, 48 kHz, mono, 7200 sample의 자체 절차 음원이다. PNG/MP3/WAV decoder와 외부 음원은 도입하지 않는다. cooker는 다음 정수 수식으로 sample을 만든다: `p=i%192`, `tri=4*p-192` if `p<96`, 아니면 `576-4*p`; `sample = trunc_toward_zero(6000 * tri * (7200-i) / (192*7200))`. i64 중간값으로 계산하고 각 결과를 signed i16 LE로 기록한다. 다른 authoring 값은 동일 수식의 bounds-checked 매개변수로 대체하며 period는 4의 배수 [4,48000], peak [1,8192], frames [1,48000]이다. 샘플레이트는 v1에서 48000만 허용한다.

payload는 `sample_rate:u32 LE`, `frame_count:u32 LE`, 뒤에 정확히 `frame_count*2` bytes다. 선언 길이와 EOF가 일치해야 한다. native/offline view adapter가 검증된 i16을 `sample / 32768.0`로 stereo에 복제해 `orr_audio::Clip::from_stereo`로 만든다. float 변환은 view에만 존재한다. 최종 mixer 출력/스피커 음질까지 byte-identical이라고 주장하지 않는다. reproducibility의 대상은 cooked PCM16이다.

## 5. canonical manifest와 cook

`orr_asset`는 no_std-compatible 참조/typed table/정수 codec 코어를 기본으로 두고, std filesystem·JSON·SHA-256·CLI는 별도 도구 crate `orr_asset_cook`에 둔다. view codec adapter는 새 `orr_asset_fixture` crate의 opt-in `audio` feature에만 둔다. 의존 방향은 fixture → orr_asset/orr_sim/orr_session 및 선택적 orr_audio이며, orr_sample이나 production sim crate가 fixture를 역으로 의존하지 않는다. sim crate에서 cooker/audio를 의존하지 않는다. 구현 전 dependency DAG를 리뷰하고 compile check로 분리를 증명한다.

runtime manifest를 sim과 view로 분리한다. 각각 고정 binary canonical encoding을 사용한다:

- header: magic `ORAM` 4 bytes, manifest version `u32 LE=1`, domain `u32 LE` (1=sim, 2=view), entry count `u32 LE`
- 각 entry: `id:u64 LE`, `type_id:u32 LE` (1=motion, 2=impact), `schema_version:u32 LE=1`, `payload_len:u64 LE`, `payload_sha256:[u8;32]`
- numeric ID 오름차순, 중복/0 금지, 정확한 EOF. domain과 type가 맞아야 한다. type ID는 registry 상수로 예약하며 Rust type_name/hash를 사용하지 않는다
- manifest digest = SHA-256(위 manifest 전체 bytes). artifact digest = SHA-256(payload 정확한 bytes). 길이·타입·버전은 manifest digest에 포함된다. ID와 artifact hash는 서로 다른 개념이다
- artifact 위치는 `objects/<64 lowercase hex>.bin`. 사용자 source path는 resolve에 사용하지 않는다. 읽기 시에도 package root escape 방지와 digest 검증을 한다

가독성용 inspect JSON은 별도 출력이며 runtime identity의 입력이 아니다. manifest에 자기 hash, timestamps, absolute paths, OS 순회 순서, host info, JSON formatting을 넣지 않는다. hash는 무결성 및 content identity용이며 악의적 package 작성자에 대한 authenticity/signature를 제공하지 않는다.

cook 흐름:
1. index 전체 검증 → GUID 정렬 → 명시된 source만 읽는다. 디렉터리 탐색 결과로 의미를 정하지 않는다
2. 각 source의 bounded strict parse 및 type-specific 검증 → canonical payload 생성
3. payload SHA-256 → content-addressed object → sim/view manifests 생성
4. sim manifest에서 `generated_sim.rs`를 생성한다. 고정 순서의 `const/static` typed records, 기대 sim manifest digest, artifact raw bits가 포함된다. 시뮬이 Rust source를 생성하거나 parse하지 않는다
5. 결과는 새 임시 output 디렉터리에서 완전히 작성·검증 후 publish한다. 기존 release output은 overwrite하지 않는다. 실패한 cook은 last-known-good bundle을 바꾸지 않는다

같은 source bytes/index/options/cooker version이면 모든 target에서 payload, manifests, generated Rust bytes가 같아야 한다. index entry 순서와 source 위치만 바뀌면 runtime payload/manifest는 같아야 한다. source raw hash와 진단 경로 등 authoring provenance는 별도 report에 둔다.

cache key는 SHA-256(domain separator + cooker format/version + importer version + type/schema + semantic options의 canonical encoding + source raw bytes hash)다. ID는 runtime manifest 조립 단계에 반영한다. v1 dependency가 없으므로 transitive closure 문제를 미루되 dependency가 생기면 key 설계를 먼저 바꾼다. cache hit에서도 길이·hash·schema를 확인하고 오염/부분 write는 discard 후 recook한다. raw whitespace 수정은 recook을 유발할 수 있으나 canonical 결과가 같으면 runtime digest는 유지된다. mtime만으로 cache hit를 허용하지 않는다. 새 cooker/importer version은 강제 무효화한다.

## 6. resolve API, 소유권과 실패

제안 API 스케치(그대로 존재하는 코드가 아님):

```rust
#[repr(transparent)]
struct AssetRef(u64); // Pod/Zeroable; 0은 unresolved/null 값
struct TypedRef<T> { raw: AssetRef, marker: PhantomData<fn() -> T> }
trait SimAsset { const TYPE_ID: u32; const SCHEMA_VERSION: u32; }
struct SimTable<'a, T> { entries: &'a [(AssetRef, T)] } // immutable sorted view
impl<T: SimAsset> SimTable<'_, T> {
    fn resolve(&self, id: TypedRef<T>) -> Result<&T, AssetError>;
}
// authoring/lifecycle 경계에서만 fallible validation
fn prepare_fixture(bundle: &BundleBytes, release: &ReleaseBinding)
    -> Result<PreparedFixture, AssetError>;
```

typed reference 생성은 manifest의 type/version을 검사한다. raw POD 참조만 받아서 T로 unchecked cast하지 않는다. endian decode에 `transmute`/unaligned cast를 사용하지 않는다. v1 각 타입별 정렬 배열+binary search를 선택하며 정렬에 따른 index를 Frame에 저장하지 않는다. GUID는 record 위치가 바뀌어도 안정적이다.

`generated_sim.rs`는 읽기 전용 static table이고 `AssetFixtureGame::systems()`가 이를 참조한다. global mutable map, lazy filesystem loader, mutable singleton service, hot reload는 없다. 배열의 canonical 재인코딩 hash를 startup에서 manifest와 대조한다. 모든 시작/restore 참조는 table에 존재해야 한다. fixture Input/Command는 새 GUID를 만들 수 없고 Command 목록은 항상 비어 있다. 그러나 ORRP v3의 별도 DebugCommand stream은 SetField/Spawn/AddComponent로 참조를 바꿀 수 있으므로 모든 debug command를 금지한다(아래 admission 계약). live debug, frame_mut, arbitrary restore, hotpatch, mutation API도 wrapper 밖으로 노출하지 않는다. 현재 System::run은 ()를 반환하므로 fallible session-stop API가 있다고 전제하지 않는다. admission 이후 남은 resolve 실패는 harness가 실패로 처리하는 fatal invariant panic/abort 경로다. default asset 대체나 다음 tick 계속 실행은 금지하며, 정상 잘못된 입력의 예상 처리는 항상 admission의 Result 오류다. fatal 경로 설명은 현재 API 한계에 대한 문서 계약만이며 새 panic/abort injection이나 replay panic 회귀 테스트는 이번 구현 범위 밖이다.

공개 fixture launcher는 `PreparedFixture` 없이 start/verify/seek를 노출하지 않는다. preparation에서 initial Frame 및 사용할 replay keyframes의 모든 asset refs를 검증한다. decoded Frame의 registry/type/범위 검증도 유지한다. Simulation/Frame의 mutable 소유권은 반환하지 않고 tick/x/checksum과 필요한 event의 읽기 전용 값만 반환한다. core의 기존 unchecked API를 새 보장 아래에 있다고 광고하지 않는다. 직접 generic APIs를 호출하는 사용자는 이 wrapper 밖이며 별도 통합이 필요하다.

sim table은 프로세스 수명 동안 살아 있고 snapshot clone/restore에 복사하지 않는다. 따라서 동일 바이너리의 keyframe 복원 뒤에도 동일 table을 조회한다. dynamic session마다 서로 다른 sim bundle을 주입하는 기능은 없음. 이 제약을 해제하려면 `Simulation::from_frame`, replay seek/verify, session/bridge/server 등 모든 생성·복원 경로에 immutable store를 전달하는 별도 lifecycle 설계가 먼저 필요하다.

view loader는 caller/main owner에서 한 번 preload하며 audio callback에 I/O·allocation·hashing을 넘기지 않는다. `Clip` backing samples는 view owner가 공유하고 음성 lifetime은 기존 `orr_audio` 계약을 따른다. event consume는 `Bridge::poll_view()` 한 번, source ID 및 ordered lifecycle를 기존 규칙대로 전달한다. asset lookup이 예측/검증/취소 dedupe를 대체하지 않는다.

오류 코드 예: InvalidId, DuplicateId, Tombstoned, MissingArtifact, WrongType, UnsupportedVersion, LengthMismatch, DigestMismatch, BudgetExceeded, SimBindingMismatch, UntrackedReplay. sim 오류는 첫 tick 이전 fail-closed. view 오류는 대체 content를 자동 탐색하지 않는다. `required` fixture에서는 시작 실패, `auto`에서는 해당 효과음을 명시적으로 disable하고 원인 보고, sim은 계속한다. 이 muted mode는 검증 실패를 숨기는 성공으로 보고하지 않는다. audio off는 device를 열지 않는다.

## 7. 하드 상한과 실행 예산

v1 fixture 정책 상한(측정된 성능 결과가 아님): index/source 입력 총 1 MiB, index 1024 records(삭제 tombstone 포함), 개별 source 64 KiB, manifest 128 KiB, live sim records 64, view records 16, sim payload 합 64 KiB, cooked view payload 합 2 MiB. 개별 PCM ≤1초/48000 frames. decoded stereo PCM 합 ≤4 MiB. 레코드/길이는 큰 allocation 전에 checked arithmetic와 위 한도로 검사한다. manifest 선언만 신뢰하지 않고 실제 파일 크기 및 합계도 확인한다. JSON nesting은 고정 schema로 제한한다.

cook는 첫 단계 단일 worker; runtime preload도 한 번에 한 asset만 처리한다. 추가 queue, background streaming, concurrent decode는 만들지 않는다. source/cache/cooked/decoded가 동시에 존재할 때의 peak memory를 child #4가 기록하고, 상한 초과는 reject한다. 오디오 callback은 기존 32 voices/4096 history 정책을 유지한다. 1초 fixture limit은 현재 adapter의 일반 10초 limit보다 더 좁다.

성능 목표는 sim resolve에서 I/O/lock/allocation 0, 64-entry binary search; actual tick cost는 기존/no-assets fixture baseline과 비교한다. 60 Hz/1 ms 엔진 목표를 이 설계만으로 달성했다고 주장하지 않는다. runtime startup blocking load에는 bytes 상한을 제공하지만 OS I/O elapsed-time hard bound는 보장하지 않는다. wasm/device 없는 검사는 sim/cook/offline만, browser audio 지원은 제외한다.

## 8. 호환성 결정: 포맷 보존과 release binding

이번 단계에서 ORRF 2, ORRP writer 3/readers 1–3, 기존 ERP envelope와 default game component registry/골든을 그대로 둔다. 새 fixture의 assetref component와 전용 골든은 신규이며 기존 Arena/PhysGame 골든을 대체하지 않는다. asset identity를 Frame checksum에 임의 prepend하지 않는다. Frame 체크섬이 외부 table 내용까지 검증한다고 설명하지 않는다.

별도 `orr.asset-release/1` binding을 제안한다. 기존 `orr.deployment/1`과 다른 파일이고 해당 tool을 몰래 확장하지 않는다. fixture release의 정확한 필드는 `format: "orr.asset-release/1"`, `game`, `game_code_id`(canonical decimal string), `build_id`(같은 방식), `frame_format_version=2`, `sim_manifest_sha256`, `view_manifest_sha256`. binding은 고정 필드만 허용하는 strict JSON이며 duplicate/unknown field, leading-zero decimal ID, 소문자 64 hex가 아닌 digest를 거부한다. JSON bytes나 별도 binding digest를 identity로 사용하지 않고 검증된 각 필드를 비교한다. fixture의 game 이름은 기존 배포 도구가 허용하는 arena/physics에 억지로 끼워 넣지 않는다. 전용 test release writer에서 같은 공개 `frame_build_id` 규칙을 사용한다.

배포/시작 절차:
1. release owner가 code semantics 또는 sim manifest 변경 시 새 nonzero raw game ID를 할당한다. release index는 한 `(game, raw ID)`가 서로 다른 sim digest를 가리키면 거부한다. view-only 변경에는 sim compatibility ID를 바꿀 필요가 없으나 새 package와 view manifest digest를 기록한 binding을 배포한다
2. binary에 포함한 기대 sim manifest digest 및 build ID를 binding과 비교한다. sim manifest와 각 object, 생성 static table 재인코딩을 검사한다. 일치해야 `PreparedFixture`를 만든다. runtime override로 expected digest를 덮어쓰지 못한다
3. 아래 고정 fixture replay를 생성할 때 nonzero `build_hash_of(build_id, 0)` 사용, 같은 release binding/manifests/objects를 release별로 보존한다. keyframe에 table을 복사하지 않는다
4. replay verify/seek admission에서 bundle 검증 + 아래 trusted generated-fixture byte 허용 목록 검사 + header game ID 확인 + header build_hash가 정확한 nonzero expected hash인지 검사한다. 기존 checked helpers의 0-wildcard보다 엄격한 wrapper 정책이다. patch_generation!=0/hotpatch replay는 v1 fixture에서 미지원으로 거부한다
5. seek helper가 내부적으로 build_id=0 Simulation을 반환하는 현재 동작에 주의한다. fixture v1 seek는 읽기 전용 재생/검사까지만 제공한다. seek 결과로 branch/new recording을 만들지 않는다. branch를 도입하려면 identity propagation을 별도 고치고 검증해야 한다

바이너리의 embedded full sim SHA-256과 checked package preflight가 **로컬 sim 내용**을 묶는다. 기존 u64 build_id/build_hash는 non-cryptographic compatibility label이며 모든 게임 코드를 hash하지 않는다. owner가 같은 ID를 잘못 재사용한 서로 다른 바이너리를 네트워크가 완벽히 탐지한다는 보장도 없다. 첫 fixture는 network join/late join을 지원하지 않는다. 이러한 제한을 문서와 진단에 표시한다.

원본 release가 없는 replay는 다른 최신 bundle로 대체 재생하지 않고 필요한 release를 찾으라는 오류를 낸다. 기존 legacy replay는 기존 게임 경로에서만 그대로 지원한다. asset fixture로 자동 승격하지 않는다. artifact GC는 보존된 release/replay references를 pin한 뒤에만 가능하며 자동 GC는 v1 비목표다.

**migration trigger**: (a) 같은 binary/ID로 여러 외부 sim bundle을 실행, (b) asset fixture network/late-join, (c) 외부 binding 없이 자기완결적인 replay 검증, (d) asset digest를 교환·검증해야 하는 안전 요구, (e) 새로운 asset schema 의미 또는 Frame reference layout 변경. 이 중 하나라도 필요하면 구현에 앞서 새 ADR로 ORRP header/handshake의 full sim digest field, protocol version negotiation, old-reader rejection, old-bundle retention, golden migration을 결정한다. 단순히 version 번호를 relabel하거나 checksum을 덮어써 이전 replay가 검증됐다고 하지 않는다. Frame POD layout을 바꾸지 않는 외부 metadata 추가가 반드시 ORRF bump인 것은 아니며 실제 호환성 차이에 따라 별도 판단한다.

## 9. 첫 end-to-end 수직 경로

1. fixture 두 source와 index 등록 → `orr_asset_cook cook --index ... --out <new-dir>` (제안 명령)
2. cooker가 artifacts + sim/view manifests + generated sim Rust + 진단 report 출력
3. test fixture target은 명시적 regeneration/check 단계로 생성물을 포함한다. 정상 cargo build에서 몰래 네트워크·source 탐색·비재현 cook을 수행하지 않는다. CI `cook --check`가 checked-in generated 파일의 drift를 거부한다
4. launcher가 release binding, bytes, table, initial refs를 검증한 뒤 아래 고정 FP movement 300 ticks 실행; 정해진 입력 edge에서 fixture event를 발행하고 view는 지정된 impact clip을 재생
5. audio-free/offline-audio 각각 같은 입력에서 모든 Frame checksum과 event stream이 동일해야 한다. 신규 fixture replay roundtrip, rollback/resimulation 및 keyframe seek가 동일 table 아래 동일 checksum을 내야 한다
6. content 변경은 같은 GUID/new payload hash/new sim manifest로 cook; old binary는 시작 전에 reject. 새 release ID와 rebuild 후에만 새 시뮬을 실행. rename/reorder는 새 digest를 만들지 않는다

기존 Arena `hit_clip` 교체, editor asset browser, ERP asset method, scene 파일 포맷, streaming, texture importer, spatial audio, mod/DLC, dependency cycles, remote fetching, signed distributions, sim hot reload는 이 경로에 포함하지 않는다.

### 9.1 고정 fixture와 replay admission의 실행 가능한 계약

fixture game ID는 `asset_fixture_v1`, tick_rate=60, player_count=1, seed=1, N=300이다. Input은 `#[repr(C)] { axis: i32 }`(4 bytes POD), axis는 -1/0/1이다. initial Frame은 entity 한 개의 x=0, profile=`a_0000000000001001`, speed raw=4096이다. tick 1–120은 +1, 121–180은 0, 181–300은 -1; x(120)=7.5, x(180)=7.5, x(300)=0이다. 모든 admitted initial/keyframe/seek Frame의 좌표 범위는 `0 <= x <= 7.5`이며 entity 수는 정확히 1이다. 게임 Command는 보내지 않는다. tick 1과 181에서 POD event `Impact { cue: u32 }`의 cue=1을 각각 한 번 내보내며 view mapping이 cue 1을 `a_0000000000002001`에 연결한다. asset ID를 input/debug로 바꾸지 않는다. initial Frame/등록 순서는 fixture source에 고정한다.

첫 구현의 replay 입력은 **동일 승인 release에서 harness가 생성하고 source-controlled expected SHA-256 목록에 고정한 fixture bytes만**이다. 사용자가 제공한 arbitrary ORRP, runtime에서 자기 hash를 신고한 파일, release mismatch는 parse 전에 거부한다. replay bytes SHA는 compressed file 전체에 대한 digest이며 asset manifest digest와 별개다. N=300 recording은 매 tick 1..300 checksum을 하나씩 기록하고 keyframe은 정확히 60/120/180/240/300에 기록한다. generator는 DebugCommand를 아예 받지 않는 API를 제공한다. hash 갱신은 명시적 fixture regeneration/review 작업이고 runtime에서 자동 허용하지 않는다. 기존 parser의 내부 decompress bound가 있더라도 이 경로는 untrusted replay에 대한 메모리·CPU 안전을 증명했다고 주장하지 않는다.

admission 순서(새 core API나 private-field 접근 불필요):
1. 입력 1 MiB 상한 및 고정 expected replay digest 검사 → 기존 `ReplayReader::parse` → 고정 header fields/input_size=4/nonzero exact build hash 검사
2. public `first_tick()==1`, `last_tick()==300`, `tick_count()==300` 검사; `tick(t)`가 모든 t=1..300에 존재하고 input 수=1, axis가 위 schedule과 일치하며 Command vec가 비었는지 검사. tick 0 record는 불허
3. public `debug_commands(t)`가 t=0..300 전체에서 비었는지 검사. raw debug map은 private이라 이 public API만으로 실행 범위 밖의 encoded debug record를 열거할 수 있다고 주장하지 않는다. 밖의 command를 포함한 다른 bytes는 단계 1의 closed allowlist가 거부하며 generator는 어디에도 debug를 쓰지 않는다. 따라서 이 제한된 fixture 경로는 모든 debug를 금지하지만 임의 ORRP의 일반-purpose debug scanner는 아니다
4. public `checksums` Vec 길이가 300이고 정확히 순서대로 `(1,h1)..(300,h300)`인지 검사. 중복/누락/역순/범위 밖 checksum을 거부한다. 빈 table은 성공으로 보지 않는다
5. `nearest_keyframe(last_tick)`부터 시작하여 key k를 수집하고 k>0이면 `nearest_keyframe(k-1)`로 내려간다. k=0은 underflow 없이 종료한다. 최대 64개, 수집 개수=`keyframe_count()`인지 확인하여 last_tick 뒤의 future keyframe도 거부한다. 이 fixture에서는 수집한 key 집합이 정확히 {60,120,180,240,300}이어야 한다
6. 각 key k에서 새 고정 Config로 `reader.seek(config,k)`를 probe한다. nearest key가 정확히 k이므로 기존 구현은 해당 Frame을 decode/restore하고 **한 tick도 resimulate하지 않는다**. 내부적으로 생긴 Simulation은 wrapper 안에서만 보유한다. `frame()`으로 모든 Motion refs/Position 범위/엔티티 수/등록 상태를 검사하고 tick 및 checksum이 해당 recorded checksum과 같은지 확인한다. initial setup Frame도 동일 검사를 한다. 실패 시 final seek/verify를 호출하지 않는다
7. 모든 preflight 성공 후 existing checked verify를 호출한다. `ticks_simulated==300`, `checksums_checked==300`, `mismatch.is_none()`가 모두 필요하다. 읽기 전용 target seek의 결과 checksum은 t=0이면 고정 initial checksum, t=1..300이면 recorded h[t]와 비교한다. 내부 Simulation은 외부에 넘기지 않는다

회귀 검사는 malformed bytes/panic에 의존하지 않는다. 정상 ReplayWriter로 생성한 유효한 SetField(Motion.profile), Spawn, AddComponent debug command가 포함된 recording, t=0/t=301 debug recording을 만들어 closed replay digest admission에서 거부되는지 확인한다. 실행 범위 내 debug 검사도 fixture test 내부의 준비 단계 단위 검사로 검증하되 production allowlist 우회 API는 노출하지 않는다. producer API가 debug 입력을 받지 않는지도 테스트한다. 누락/중복 checksum, sparse ticks, 잘못된 axis, future keyframe도 정상 encode된 fixture 변형으로 거부 검사를 한다. input별 failure는 resimulation counter=0이어야 한다. digest 단계에서만 실패하는 통합 검사는 후단 validator의 정확성을 증명하지 않으므로 header/tick/input/checksum/keyframe validator도 내부 단위 테스트에서 직접 성공·실패 분기를 검증한다. 이 테스트 전용 호출은 public allowlist 우회 API로 노출하지 않는다. 이 테스트는 arbitrary 파일 로더 지원을 뜻하지 않는다.

## 10. 검증·완료 기준

설계 승인과 구현 완료는 다르다. 아래는 **실행 예정 검사**이며 통과 보고가 아니다.

- identity/index: 0/duplicate/overflow GUID, rename/move 유지, clone 신규 ID, delete tombstone 및 referenced-delete reject, >2^53 ID roundtrip
- cooker: 두 clean output dirs, cold/warm cache, reversed index order, relocated source root, fixed source fixture → manifest/payload/generated Rust bytes exact equality. Linux/Windows에서 비교; source FP halfway·overflow·exponent 지원 여부를 parser 계약에 맞춰 고정
- parser/loader: unknown type/version, wrong domain, byte flip, length under/overflow/trailing bytes, duplicate JSON key, path escape, over-budget, corrupted cache를 모두 reject; 불완전 cook이 publish되지 않음
- boundary: `orr_asset` core가 std/audio/cooker/fs를 의존하지 않는 target check와 dependency inspection; sim table immutable; tick-side allocator/filesystem 접근 없이 resolve. static raw bits와 cooked payload가 일치하는지 재인코딩 검사
- runtime: 잘못된/없는 ref는 start 및 keyframe admission에서 첫 tick 전에 거부; 같은 ID/다른 sim digest, zero replay hash, wrong game/build, missing old bundle도 거부. unchecked generic replay API는 테스트 대상 보장으로 혼동하지 않음
- determinism: fixture 300 ticks + rollback + keyframe seek checksum 고정; audio on/off 이벤트·checksum 일치; 기존 golden 기대값 무변경. Linux/Windows/macOS ARM/wasm/Android 중 실제 수행한 target을 명시하고 실행하지 않은 target은 미검증 표시
- audio: offline renderer 비영/finite/bounded sample, 기존 event dedupe/cancel/reset/voice saturation 회귀, PCM16 cook의 platform exact bytes. native compile 성공은 물리적 청취 증명이 아님
- budget: 최대·초과 입력, checked size arithmetic, decode peak memory와 resolve/tick 비용 baseline 비교. fixture replay 입력은 1 MiB compressed 파일·고정 300 ticks·5 keyframes이며 parser 전 closed digest allowlist를 적용한다. public keyframe enumeration에는 방어적으로 64개 cap을 둔다. untrusted replay ingestion은 제공하지 않고 승인 release가 생성한 fixture replay만 테스트한다. 이를 새 asset loader의 보장으로 덮지 않는다

예정 명령 시작점: `cargo test -p orr_asset -p orr_asset_cook`, `cargo check -p orr_asset --no-default-features --target wasm32-unknown-unknown`, 새 fixture 통합 테스트, `cargo test -p orr_audio`, `cargo test -p orr_sample --features audio --lib arena_audio`, `cargo test -p orr_session --test arena`, 최종 `cargo test --workspace --release`와 `cargo clippy --workspace --all-targets`. 새로운 crate/target 명은 구현 child에서 확정한다. mandatory CI workflow 편집은 기존 workflow 담당자 handoff 후 한다.

## 11. 선택의 대가와 후속 설계

검토한 대안:
- setup에서 asset 내용을 Frame singleton으로 복사: API 변경은 작지만 불변 외부 asset을 snapshot에 복사하지 않는 목적을 충족하지 않아 선택하지 않음
- `Game::systems(config)` 또는 `SimContext.assets`로 `Arc<ImmutableStore>` 주입: 일반 게임에는 더 적합하지만 from_frame/replay/session/bridge/host의 모든 재구성 경로와 store의 identity/lifetime를 함께 변경해야 함. 현재 범위에서 일부 경로만 바꾸면 seek/rollback 때 다른 store를 사용하는 위험이 있어 후속 ADR로 분리
- process-global 교체 가능한 registry: 두 session의 bundle 충돌과 nondeterministic hot reload 위험으로 거부
- 이번 static fixture: 범용 immutable asset registry로 가는 identity/cook/typed resolve 계약을 시험하는 첫 단계. 일반 게임 사용자가 파일에서 임의 sim asset을 로드할 수 있는 완성된 asset DB라고 부르지 않음. 그 기능을 요청하는 즉시 context injection 및 replay/network full digest binding 설계 게이트를 먼저 통과해야 함


- 정적 sim table: 현재 replay 생성·복원 구조를 유지하고 rollback lifetime을 쉽게 보장한다. 대신 content 변경에 rebuild가 필요하며 이 첫 경로는 범용 runtime asset service가 아니다
- u64 GUID: 작고 POD-friendly하나 전역 UUID 수준의 충돌 공간은 아니다. 프로젝트 registry와 tombstone 정책이 필수다
- manifest SHA-256: 변조/손상 식별과 캐시 identity는 제공하지만 서명·신뢰 공급망은 제공하지 않는다
- PCM16 정수 generator: 추가 codec 없이 재현성 검사가 가능하다. 현재 Arena 수식과 소리가 다르며 교체하지 않는다
- 동일 sim ID에서 view 업데이트: gameplay replay는 보존하지만 시청각적으로 같은 리플레이를 원하면 view manifest까지 별도 pin해야 한다. fixture release package는 두 digest를 모두 보존한다
- 일반 sim-store injection, deployment tool 통합, network handshake, reflective AssetRef UI, streaming/hot reload는 각각 후속 설계가 필요하다

승인 게이트: asset/replay 소유자가 static fixture 제한, zero-ID reject, release binding 유지 방식, migration trigger에 동의한 뒤 child를 열고 파일 잠금을 인계한다. #24 epic은 구현 child 및 통합 검증이 끝나기 전까지 닫지 않는다.

## 12. 주요 근거 링크

- [범위 #24](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/24)
- [원격 배포 manifest tool](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/blob/87e4a526e0e2cb8726cfc4c06de01f7cfea6a41c/tools/deployment_manifest.py)
- [PR #30](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/30)
- [Frame 호환성](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/blob/87e4a526e0e2cb8726cfc4c06de01f7cfea6a41c/docs/frame-compatibility.md)
- [기존 설계](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/blob/87e4a526e0e2cb8726cfc4c06de01f7cfea6a41c/docs/design-v1.md)
- [replay API](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/blob/87e4a526e0e2cb8726cfc4c06de01f7cfea6a41c/crates/orr_session/src/replay.rs)
- [audio 경계](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/blob/87e4a526e0e2cb8726cfc4c06de01f7cfea6a41c/docs/audio.md)



## 13. 구현된 opt-in audio 경로와 검증 범위 (#35)

`orr_asset_fixture`의 기본 feature는 audio-free다. 선택적 `audio` feature만
`orr_audio`와 `orr_bridge`를 추가하며 cooker/sample/native-device 의존은 추가하지
않는다. `audio::FixtureAudio`는 먼저 기존 strict `PreparedFixture`를 요구한다.
별도 view copy를 전체 manifest SHA, object SHA/길이/PCM header로 다시 검증하고
PCM16을 stereo Clip으로 preload한 뒤 caller bytes를 보유하지 않는다.

`required`는 view 오류를 반환하고 `auto`는 `Muted { reason }`를 명시적으로
반환한다. `off`는 view preload와 mixer 생성을 생략한다. 이 정책은 **이미 정상
admission을 통과한 sim의 presentation copy**에만 적용한다. 기존 package
preparation의 필수 view 검사나 replay allowlist/header/keyframe/300-checksum
검사를 완화하지 않는다. 불완전 release package를 auto로 실행하는 bypass는 없다.

한 번의 `Bridge::poll_view()`가 반환한 batch를 시각화와 audio에 공유하며 ordered
lifecycle/resync를 그대로 전달한다. source ID는 bridge/session owner가 소유한다.
`Impact { cue: 1 }`만 고정 `IMPACT_ID`로 매핑한다. 기존 32 voices/4096 history,
predicted→verified dedupe, cancel/replacement/reset 규칙은 EventAudio를 재사용한다.

4 MiB decoded 상한은 bank가 보유하는 stereo sample 합이다. 전체 수량을 먼저
검사한 뒤 clip을 순차 변환한다. Vec→Arc 변환 때 최대 한 clip (384000 bytes)의
추가 sample buffer가 일시적으로 존재할 수 있으며 input/mixer/allocator overhead도
있다. 전체 프로세스 peak RSS는 이 4 MiB와 같다고 주장하지 않는다. 현재 v1의
manifest 최대 912 bytes 및 cooked 최대 1536128 bytes는 일반 128 KiB/2 MiB byte
상한보다 작다. 따라서 byte guard 자체의 최대/+1 산술 검사와 실제 유효 package의
record/frame/decoded 최대/+1 검사를 구분한다.

현재 determinism workflow는 native core/cook/fixture 및 cooked `--check`,
Linux/Windows opt-in audio, WASI 및 SIMD fixture, Android fixture, browser-target
**compile-only** fixture를 연결한다. 실행되지 않은 target을 통과로 보고하지 않는다.
기존 workspace release/clippy와 aggregate mandatory gate는 유지한다. 하드웨어
청취/browser audio/streaming/spatialization은 지원·검증 범위가 아니다.

실행 명령, baseline 비교와 RSS 관측치는
[fixture audio validation](../crates/orr_asset_fixture/AUDIO_VALIDATION.md)에 기록한다.
위 설계 시점의 미구현 설명은 역사적 조사이며, 구현 API/제약은
[fixture README](../crates/orr_asset_fixture/README.md)와 해당 검증 기록을 따른다.
#24는 children 완료와 최종 통합 CI 전에 닫지 않는다.
