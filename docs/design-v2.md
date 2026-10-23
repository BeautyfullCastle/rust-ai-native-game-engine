# Orrery 설계 v2: 작은 게임의 무개입 완주

2026-10-23 · 상태: **제품 방향과 후속 구현 제안**. 이 문서 자체는 런타임 변경이나 평가 실행 결과가 아니다. [v1](design-v1.md)의 결정론·롤백·시뮬/뷰 경계는 유지한다.

## 1. 목표와 범위

자연어 요구 → 사용자 소유 게임 소스 → 실행 → 요구 확인 → 수정 → 재현 가능한 전달을 하나의 경로로 만든다. 우선 기존 정적 Rust 제작 경로를 통합한다. 새 렌더 기능·플랫폼·검증 프레임워크의 수를 목표로 삼지 않는다. 편집, 입력, 재시작, 재현, 전달을 실제 작은 게임에서 연결하는 작업이 우선이다.

## 2. 북극성 지표와 과제군

**무개입 성공률 = 100 × 모든 수용 조건을 예산 안에 충족하고 사람의 중간 개입이 없었던 게임 과제 실행 수 / 사전에 등록한 전체 게임 과제 실행 수.** 과제별 성공률도 같이 공개한다. 실패·중단·미시작·미해결 실행을 분모에서 빼지 않는다. 미완료 실험은 잠정치와 미해결 수를 표시하고, 재시도는 원래 실행의 기록을 지우지 않는다. 지표의 현재 값은 **미측정**이다.

초기 제안은 3개 자연어 게임 과제 × 사전 고정 3개 seed, 총 9회다. 각 실행은 새 사용자 프로젝트에서 시작한다.

1. 배달 게임: 이동·수집·목표 도착·승리 상태를 요구대로 구현하고 전달한다.
2. 스위치 퍼즐: 여러 스위치의 조건과 문 상태, 실패/재시작을 구현한다.
3. 액션 게임: 이동과 둘 이상의 독립 행동, 성공/실패와 재시작을 구현한다. 기존 private fixture 복사를 완성품으로 인정하지 않는다.

각 프롬프트와 독립 oracle을 실행 전에 동결한다. 요구 상태 전이·입력 해제·저장/재시작·원본 replay 재검증·최종 player의 실제 실행을 성공 조건으로 삼는다. 사람의 코드 수정, 힌트, 수동 복구는 개입이며 무개입 성공에서 제외한다. 사전 환경 준비는 별도 기록한다.

## 3. 기존 harness를 확장하는 측정 계약

`tools/arena_agent_trial.py`, `tools/arena_trial/{runner,protocol}.py`와 [현재 계약](arena-agent-trial.md)을 재사용한다. 현재 다섯 Arena 목표는 작성 → 이동/해제 → 발사/기록 → guarded 수정/undo/redo → 재시작/원본 재현이다. runner가 oracle과 기록을 소유하고 tick 0–50의 51개 checksum을 대조한다. 현재 mock 전용 protocol은 임의 게임 소스 작성/빌드를 수행하지 못한다. 실제 모델 adapter와 위 게임별 oracle/제작 단계는 **후속 구현**이며 별도 평가 체계를 새로 만들지 않는다.

- 재현: source/tree·fixture·binary hash, game/build/schema, 모델/provider/version·설정, prompt, seed, 실행 환경, task/attempt 수와 oracle 버전을 실행 전에 고정한다. 실패 기록과 resume segment를 모두 보존한다.
- 예산: 현재 mock 기본은 단계 600초, attempt 2400초, 명령 30초, 전체 experiment 2000 requests, 개입 0이다. 이를 실제 제작 성능 기준으로 오인하지 않는다. 새 9회 suite는 별도 버전 manifest에서 **게임당 wall 60분, tool requests 300회, 개입 0**을 초기 예산으로 제안한다. 빌드/다운타임/재개를 포함하고 예산을 실패 후 늘리지 않는다. 모델 token·비용 상한과 집행 가능한 계측은 승인된 provider를 정한 뒤 실행 전에 고정한다. 현재 null usage는 0이나 예산 준수가 아니다.
- 실패 분류: 요구/게임 로직, 작성·컴파일, 입력/뷰/전달, 결정론·재현, budget 초과, infrastructure 중단, 사람 개입, 미시작/미해결. 원인 태그와 실행 상태를 분리하여 중단 후 성공도 이력을 유지한다.
- 함께 볼 값: 완료 wall time·빌드 대기, requests, token/비용(알 수 없으면 unknown), replay 일치율, 개입 횟수. 하나의 scripted/mock 성공률이나 CI 개수로 북극성을 대신하지 않는다.

실제 모델 호출·업로드·비용은 별도 승인과 필요한 격리를 확보한 뒤 시작한다. 기존 same-account protocol은 OS sandbox가 아니다. CI는 raw evidence를 성공/실패 모두 artifact로 남긴다.

## 4. 로직 작성 경로 결정

**이번 통합과 첫 기준선은 정적 Rust로 진행한다. 결정론 WASM scripting 도입은 보류한다.** 근거는 `orr_sim::Game`의 타입 계약과 순서 고정 `System`, starter의 Delivery/Switchboard 구현, compiled registry, 독립 bootstrap이 이미 같은 경로를 사용한다는 점이다. Rust의 wasm32 빌드와 `orr_wasm_bench`는 배포/검증 대상이며 동적 게임 script runtime이 아니다. v1의 desktop hotpatch 또한 WASM scripting 구현을 뜻하지 않는다.

정적 Rust는 기존 타입/리플레이 계약을 재사용하지만 컴파일·link 대기가 제작 반복을 늦출 수 있다. WASM은 bounded game-logic module 교체를 가능하게 할 후보지만 새 ABI, runtime/메모리 상태, fuel과 trap, 재현 검증 비용을 추가한다. 먼저 suite의 실패와 빌드 대기 시간을 측정한다. 제안된 진입 기준은 첫 suite의 중앙값 build/link 대기 비중이 wall time의 30% 이상인 경우다. 실행 전에 기준을 고정하고, 같은 과제·seed·모델·예산의 paired prototype에서 중앙값 수정→실행 시간을 25% 이상 줄이면서 무개입 성공률 저하와 replay 불일치가 없어야 승격 검토를 시작한다. 이 수치는 관측 결과가 아닌 초기 제안이며, 표본 수와 실행별 결과를 함께 보고한다.

향후 migration 경계는 `System` 뒤의 **선택적 게임 로직 adapter**다. ECS/session/replay를 교체하지 않는다. 정수/고정소수점 ABI, 안정 ID와 제한된 command/state 입출력만 허용하고 wall clock·OS·WASI·비결정적 import를 배제한다. module/runtime/ABI identity를 build/replay identity에 묶고, 모든 mutable guest state를 snapshot/rollback에 포함하거나 명시적으로 무상태화한다. fuel/메모리 상한·trap의 결정론적 실패 처리를 정의한다. 기존 Rust와 같은 입력의 checksum/replay·rollback·교차 플랫폼 검증을 통과하기 전 기본 경로로 승격하지 않는다. 이 경계는 설계 제안이며 아직 구현되지 않았다.

## 5. 전달 순서

기존 후보 통합 → 정확 통합 head의 필수 CI/리뷰 → hardening 반영 → main 승격 → 게임 1개 end-to-end 전달 → 실제 모델 adapter/9회 기준선 순서다. 현재 사용자 지시는 hardening과 main 통합을 포함한다. 각 정확 통합 head의 필수 CI·리뷰와 hardware/UI 수용 조건은 유지하며 PR #1 병합을 자동으로 포함하지 않는다. main 통합 뒤에는 main에서 짧은 기능 브랜치를 만들고 검증된 변경을 자주 반영하며 장기 stack을 늘리지 않는다. 변경마다 관련 집중 검사, 통합 경계마다 필수 전체 검사를 수행하고 동일 tree의 광범위 감사·원시 증거 복제는 반복하지 않는다.
