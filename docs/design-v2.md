# Orrery 설계 v2: 작은 게임의 무개입 완주

2026-10-28 · **실제 게임 제작 우선 설계와 후속 제안**. 평가 결과는 아니다. [v1](design-v1.md)의 결정론·롤백·시뮬/뷰 경계는 유지한다.

## 1. 목표와 범위

**실제 게임 제작이 우선이다.** 자연어 요구 → 사용자 소유 소스 → 실행·요구 확인·수정 → 재현 가능한 전달을 연결한다. Stormglass Foundry/Coop Defense의 전투·웨이브·승패·재시작과 제작 공백부터 고친다. 기능·PR·테스트 수는 제작 성공을 대체하지 않는다.

AI 제작·조작·검증은 **구조화된 API**가 기본이며 화면 조작은 선행 조건이 아닌 보완 QA다. Windows native 직접 플레이와 headless/API 검증을 구분하고 다른 게임/oracle 성공으로 화면 증거를 대체하지 않는다.

## 2. 북극성 지표와 과제군

**무개입 성공률 = 100 × 모든 수용 조건을 예산 안에 충족하고 사람의 중간 개입이 없었던 게임 과제 실행 수 / 사전에 등록한 전체 게임 과제 실행 수.** 과제별 성공률도 같이 공개한다. 실패·중단·미시작·미해결도 분모에 포함한다. 미완료는 잠정치·미해결 수를 표시하고 재시도 원본을 보존한다. 현재 **미측정**이다.

초기 제안: 자연어 3과제 × 사전 고정 3seed = 9회, 각각 새 사용자 프로젝트에서 시작한다.

1. 배달 게임: 이동·수집·목표 도착·승리 상태를 요구대로 구현하고 전달한다.
2. 스위치 퍼즐: 여러 스위치의 조건과 문 상태, 실패/재시작을 구현한다.
3. 액션 게임: 이동과 둘 이상의 독립 행동, 성공/실패와 재시작을 구현한다. 기존 private fixture 복사를 완성품으로 인정하지 않는다.

각 프롬프트와 독립 oracle을 실행 전에 동결한다. 요구 상태 전이·입력 해제·저장/재시작·원본 replay 재검증·최종 player의 실제 실행을 성공 조건으로 삼는다. 사람의 코드 수정, 힌트, 수동 복구는 개입이며 무개입 성공에서 제외한다. 사전 환경 준비는 별도 기록한다.

## 3. 기존 harness를 확장하는 측정 계약

`tools/arena_agent_trial.py`, `tools/arena_trial/{runner,protocol}.py`와 [현재 계약](arena-agent-trial.md)을 재사용한다. 현재 다섯 Arena 목표는 작성 → 이동/해제 → 발사/기록 → guarded 수정/undo/redo → 재시작/원본 재현이다. runner가 oracle과 기록을 소유하고 tick 0–50의 51개 checksum을 대조한다. 현재 mock 전용 protocol은 임의 게임 소스 작성/빌드를 수행하지 못한다. 실제 모델 adapter와 위 게임별 oracle/제작 단계는 **후속 구현**이며 별도 평가 체계를 새로 만들지 않는다.

- 재현: source/tree·fixture·binary hash, game/build/schema, 모델/provider/version·설정, prompt, seed, 실행 환경, task/attempt 수와 oracle 버전을 실행 전에 고정한다. 실패 기록과 resume segment를 모두 보존한다.
- 예산: mock 기본은 단계 600초·attempt 2400초·명령 30초·experiment 2000 requests·개입 0이며 제작 성능 기준이 아니다. 새 suite는 버전 manifest에 게임당 wall 60분·requests 300회·개입 0을 제안한다. 빌드/다운타임/재개를 포함하고 실패 후 늘리지 않는다. 승인된 provider의 token·비용 상한과 계측을 실행 전에 고정한다. null usage는 0이나 준수 증거가 아니다.
- 실패 분류: 요구/게임 로직, 작성·컴파일, 입력/뷰/전달, 결정론·재현, budget 초과, infrastructure 중단, 사람 개입, 미시작/미해결. 원인 태그와 실행 상태를 분리하여 중단 후 성공도 이력을 유지한다.
- 함께 볼 값: 완료 wall time·빌드 대기, requests, token/비용(알 수 없으면 unknown), replay 일치율, 개입 횟수. 하나의 scripted/mock 성공률이나 CI 개수로 북극성을 대신하지 않는다.

실제 모델 호출·업로드·비용은 별도 승인·격리 후 시작한다. Same-account protocol은 OS sandbox가 아니다. 성공·실패·중단 raw evidence는 git 밖에 정확한 소스와 연결하고 기존 CI 증거를 보존한다.

## 4. 로직 작성 경로 결정

**통합·첫 기준선은 정적 Rust, 결정론 WASM scripting은 보류한다.** `orr_sim::Game`/순서 고정 `System`, Delivery/Switchboard, compiled registry와 독립 bootstrap을 재사용한다. Rust wasm32/`orr_wasm_bench`는 배포·검증 경로이지 동적 script runtime이 아니며 v1 desktop hotpatch도 WASM scripting이 아니다.

정적 Rust의 compile/link 대기부터 측정한다. WASM prototype 진입 기준은 첫 suite의 중앙값 build/link 비중 30% 이상으로 제안한다. 같은 과제·seed·모델·예산에서 중앙값 수정→실행 시간을 25% 이상 줄이고 무개입 성공률 저하·replay 불일치가 없어야 승격 검토한다. 이는 관측값이 아닌 사전 기준이며 표본·실행별 결과를 공개한다.

향후 migration 경계는 `System` 뒤의 **선택적 게임 로직 adapter**다. ECS/session/replay를 교체하지 않는다. 정수/고정소수점 ABI, 안정 ID와 제한된 command/state 입출력만 허용하고 wall clock·OS·WASI·비결정적 import를 배제한다. module/runtime/ABI identity를 build/replay identity에 묶고, 모든 mutable guest state를 snapshot/rollback에 포함하거나 명시적으로 무상태화한다. fuel/메모리 상한·trap의 결정론적 실패 처리를 정의한다. 기존 Rust와 같은 입력의 checksum/replay·rollback·교차 플랫폼 검증을 통과하기 전 기본 경로로 승격하지 않는다. 이 경계는 미구현 설계다.

## 5. 구현 경계와 전달 순서

결정론 코어와 사용자 게임 패키지를 분리한다. Stormglass는 starter의 선택적 `stormglass` feature와 등록 프로젝트로 기존 `DefenseGame`을 재사용한다. 엔진의 게임별 enum·복제 시뮬레이션은 금지한다. 각 게임은 등록·typed host·입력 codec·presenter를 제공하고, 에디터와 ERP는 공통 lifecycle·lease·sequence·인증 계약을 유지한다. Stormglass는 공통 ERP 편집·history·undo·save/reopen으로 `Scenario`의 시작 gold(0–10000, 기본 150)와 reactor HP(1–1000, 기본 30)를 작성한다. 저장 Scene은 선언 필드를 모두 요구하며 최초 필드 편집은 reflected default를 쓴다. 선택적 `coop-defense-scene` 경로를 공유하는 native `--scene PATH`는 창 생성 전 파일을 검증하고 headless/windowed에 같은 Scenario를 전달한다. 기본 빈 scene 동작은 유지한다. Browser 로컬 Scene loader는 후속 변경에 구현·WASM/mock-DOM 검증되었으나 개발 미병합이다. 실제 browser/offline-filepicker QA·embedded exporter·임의 웨이브·cross-game 전환은 미완료다. [API 계약](registered-gameplay-api.md)을 따른다.

Windows native에서 10-wave 승리(369 kills, 29 merges, core 8/30), Wave 2 패배와 R 재시작이 관측되었다([#94 증거](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6430939337)). 새 등록/API 테스트는 별도 CPU 검증이며 GPU나 동일 산출물의 화면 재검증을 뜻하지 않는다. 입력·편집·재현·전달 연결이 우선이며 평가 준비로 미루지 않는다.

[진행 상태](progress.md)에 통합·미검증 범위를 기록한다. 제거된 hosted workflow는 통과가 아니다. 정확한 후보의 `bash tools/check-linux-local.sh`·집중 검사·독립 리뷰 후 승인된 **개발 브랜치**에만 통합하며 **main/PR #1은 제외**한다. [PR #309](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/309)는 정확한 후보 `af91fd9`의 필수 Linux 검사·집중 검사·독립 리뷰 후 2026-10-28 공유 개발에 병합되었다. 병합 `a6844afb`의 tree `ca3a225b`는 검증 후보와 동일하다. main 반영·엔진 완성은 아니다.

기존 테스트·보안·인증·민감정보 제한과 과거 실패 증거를 보존한다. 서버 측 required check가 남으면 별도 차단점으로 보고하고 보호 설정을 우회하지 않는다. source tree·toolchain·명령·종료 상태·미검증 범위를 남긴다. 동일 소스·환경의 증거는 재사용하되 실제 변경에 필요한 검사를 생략하지 않는다. Linux/API 통과는 별도 실행 증거가 없는 Windows·macOS·Android·browser·hardware/GPU 또는 source-hidden 전달을 증명하지 않는다.
