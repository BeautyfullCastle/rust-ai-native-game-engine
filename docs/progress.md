# Orrery 현재 상태

갱신: 2026-11-05 UTC · [설계 v2](design-v2.md) · [상위 추적 #2](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/2) · [게임 제작 #94](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94)

## 실제 게임 제작 경로

자연어 요구에서 사용자 소유 게임의 작성·실행·수정·재현·전달까지 연결한다. Stormglass Foundry/Coop Defense를 실제 게임 기준으로 삼고 구조화된 API를 기본으로 사용한다. 화면 조작은 보완 QA다. 기능·PR·테스트 수는 게임 완성률이 아니며 **실제 모델 기반 무개입 제작 성공률은 아직 미측정**이다. Mock/scripted 성공으로 대신하지 않는다.

- 결정론 코어의 고정소수점·순서 고정·롤백·리플레이·시뮬/뷰 경계를 유지한다. 선택적 게임 패키지는 기존 Game/등록/입력 codec/presenter를 사용하며 엔진에 게임별 시뮬레이션을 복제하지 않는다.
- Stormglass는 공통 ERP/등록 Editor의 편집·Undo/Redo·Save/reopen과 native/WASM 소비를 연결한다. 현재 작성 계약은 Scenario, Campaign, WavePacing, WaveCounts, EnemyHealth, WaveComposition의 **여섯 singleton**이다. 자원·HP·마지막 웨이브·생성 간격·웨이브별 수·적 종류/첫 보스를 작성한다. [범위와 예제](stormglass-authoring.md)를 따르며 보류 중 EnemyMotion을 포함하지 않는다.
- Browser 로컬 Scene import와 authored-scene offline packing은 구현돼 있다. 단일 HTML에 고정된 WASM/자산/장면을 넣어 전달할 수 있다. [Run Save v1](coop-defense-run-save.md)은 같은 WASM의 유한 입력 기록을 재생하는 저장/복원이며 일반 Frame checkpoint·버전 migration이 아니다. 온라인 협동·계정·매칭·production balance는 완료 주장에 포함하지 않는다.
- 복원 실패는 기존 게임/기록을 보존한다. 다만 포커스 상실은 별도 안전 이벤트로, 복원 중에도 전투를 일시정지하고 대기 입력/제스처를 취소한다. 실패/취소한 복원이 이를 되돌리거나 명시적 계속 없이 전투를 재개해서는 안 된다.

## 확인한 개발 통합과 실제 증거

[PR #426](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/426)은 MeshHazardsV1의 animation Load → Apply/Undo/Redo → Save/reopen → 원본 숨김 native 소비를 통합했다. 별도 capability·provenance·runtime trust와 기존 schema/default-off 경계를 유지한다. [원래 수용 기록](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/426#issuecomment-6639723379)은 당시 tree에 한정된다.

[PR #427](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/427)/[#428](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/428)은 복원 중 포커스 상실과 해제된 드래그 부활을, [#434](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/434)는 Ctrl/Cmd+R이 Invest로 소비되던 결함을 수정했다. 실제 WASM/mock-DOM 및 독립 산출물 재실행 증거이며 실제 브라우저 수용은 아니다.

Native [#429](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/429)는 이전 클릭의 hover 오해석을, [#430](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/430)은 Pause/Continue·포커스 정지를, [#431](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/431)은 선택 Save Copy/Load를 추가했다. [#432](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/432)는 원본 숨김 authored audio → Kira PCM을, [#433](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/433)은 delay2 소비 전 빠른 이동 선택 보존을 검증했다. [#435](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/435)는 승인된 terminal Restart가 포커스 복귀만으로 실행되지 않고 명시적 Continue를 기다리도록 수정했다. 각 PR의 정확한 CI·독립 산출물 기록과 실패 이력을 보존하며 창·장치·스피커 청취 증거로 확대하지 않는다.

[PR #436](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/436)은 등록된 기본 Delivery/Switchboard를 실제 완주할 수 있도록 프로젝트 소유 move/select 제어와 단발 입력·정확한 소비 확인을 연결했다. 개발 merge c607d380, 검토 tree ef86ee87의 [정확 head CI](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/actions/runs/41966442733) 및 [독립 산출물 수용](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/436#issuecomment-6649744451)이 통과했다. 고정 130개 테스트·4개 strict 구성·다운로드 실행 파일·native 식별 쿼리 4개를 확인했고, 원래 실패와 새 build ID의 이전 replay 거부를 보존했다.

[PR #437](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/437)은 같은 등록 Stormglass 세션에 읽기 전용 OVS 관측을 연결했다. 개발 merge 2c8be46d, tree 21ac7e52의 [정확 head CI](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/actions/runs/41974416216)와 [독립 다운로드 수용](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/437#issuecomment-6650833158)이 통과했다. 159개 CPU 테스트와 6개 strict 구성을 확인했다. 스트림은 최신 상태이며 epoch-fenced 명령 영수증이 아니다. 원래 실패와 기능 경계 수정·산출물 감사 실패를 보존한다.

- **과거 Windows 직접 플레이:** 10-wave 승리, 369 kills, 29 merges, core 8/30 및 Wave 2 패배 → R 재시작을 관측했다. [원래 증거](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6430939337)는 당시 산출물에만 적용하며 현재 소스의 Windows 재검증으로 옮기지 않는다.
- **현재 Stormglass 실행 범위:** 실제 native/WASM consumed-input trace와 source-hidden packed 소비, mock-DOM 제어/저장 검사는 실제 브라우저 파일 선택·표시·물리 입력과 별개다. 2026-11-04 Chromium 사전 검사는 페이지 로드 전 Unix socket EPERM으로 실패했고 정식 확대 실행도 executor 초기화에서 실패했다. 실제 브라우저 플레이는 **미검증**이며 다른 환경 실행으로 이 제한을 우회하지 않았다.

## 남은 우선순위와 경계

1. 실제 제작·완주 경로를 우선한다. [작성한 Switchboard 목표가 플레이어에 보이지 않는 공백](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6651054198)을 실제 CPU 컨트롤러로 재현했다. 목표 3/5가 시작 전과 같은 입력 소비 후에도 동일한 렌더 항목을 만든다. 기존 현재 스위치는 보존하고 별도 목표 행과 native/editor 안내를 추가하며, 규칙·입력·build identity는 바꾸지 않는다. 구현·검증 중이고 실제 창의 가독성·완료·병합을 주장하지 않는다.
2. 허용된 실행 환경이 있을 때 현재 산출물의 브라우저/native-window 플레이를 별도 수용한다. 화면 증거가 없으면 성공률·모바일·Windows·hardware GPU까지 확대하지 않는다.
3. [설계 v2](design-v2.md)의 사전 고정 과제/seed/oracle·예산·실제 모델 기준선을 준비한다. 모델 호출·업로드·비용은 별도 승인 범위이며 아직 측정 결과가 없다.

Su가 구현·검증·독립 리뷰와 파일 소유권을 조정한다. 종료된 외부 담당에게 새 작업을 배정하지 않는다. Held Collect/CUBICSPLINE과 게시가 거부된 EnemyMotion을 인수하거나 재게시하지 않는다. ERP의 명시적 loopback/token·capability·lease·sequence 계약은 [등록 API 문서](registered-gameplay-api.md)를 유지한다.

## 보존할 검사와 실패

[PR #309의 정확한 Linux-local baseline](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/309#issuecomment-6460288981)과 [후속 PR #404/#405 결합 검증](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6616200365)은 당시 source/환경의 증거다. 변경된 경로에는 해당 검사를 새로 실행하고 필수 CI·독립 리뷰 후 개발 브랜치에만 통합한다. Retired/failed/unrun 검사를 성공으로 옮기지 않으며 보안·인증·보호 설정을 우회하지 않는다.

Advisory formatting debt, [원래 compile/중단 기록](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6450381753), PR #139/#141의 Windows 실패/미재현 한계, #426의 최초 compile/EXDEV 실패를 보존한다. Raw logs·ZIP·JSON은 git 밖에 source/tree·toolchain·명령·종료 상태와 연결한다. #2의 기존 분모 22는 그대로이며 #94 하위 기능/PR 수로 엔진 완성률을 재계산하지 않는다.
