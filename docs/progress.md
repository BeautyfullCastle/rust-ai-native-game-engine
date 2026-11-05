# Orrery 현재 상태

갱신: 2026-11-05 UTC · [설계 v2](design-v2.md) · [상위 추적 #2](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/2) · [게임 제작 #94](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94)

## 실제 게임 제작 경로

자연어 요구에서 사용자 소유 게임의 작성·실행·수정·재현·전달까지 연결한다. Stormglass Foundry/Coop Defense를 실제 게임 기준으로 삼고 구조화된 API를 기본으로 사용한다. 화면 조작은 보완 QA다. 기능·PR·테스트 수는 게임 완성률이 아니며 **실제 모델 기반 무개입 제작 성공률은 아직 미측정**이다. Mock/scripted 성공으로 대신하지 않는다.

- 결정론 코어의 고정소수점·순서 고정·롤백·리플레이·시뮬/뷰 경계를 유지한다. 선택적 게임 패키지는 기존 Game/등록/입력 codec/presenter를 사용하며 엔진에 게임별 시뮬레이션을 복제하지 않는다.
- Stormglass는 공통 ERP/등록 Editor의 편집·Undo/Redo·Save/reopen과 native/WASM 소비를 연결한다. 현재 작성 계약은 Scenario, Campaign, WavePacing, WaveCounts, EnemyHealth, WaveComposition의 **여섯 singleton**이다. 자원·HP·마지막 웨이브·생성 간격·웨이브별 수·적 종류/첫 보스를 작성한다. [범위와 예제](stormglass-authoring.md)를 따르며 보류 중 EnemyMotion을 포함하지 않는다.
- Browser 로컬 Scene import와 authored-scene offline packing은 구현돼 있다. 단일 HTML에 고정된 WASM/자산/장면을 넣어 전달할 수 있다. [Run Save v1](coop-defense-run-save.md)은 같은 WASM의 유한 입력 기록을 재생하는 저장/복원이며 일반 Frame checkpoint·버전 migration이 아니다. 온라인 협동·계정·매칭·production balance는 완료 주장에 포함하지 않는다.
- 복원 실패는 기존 게임/기록을 보존한다. 다만 포커스 상실은 별도 안전 이벤트로, 복원 중에도 전투를 일시정지하고 대기 입력/제스처를 취소한다. 실패/취소한 복원이 이를 되돌리거나 명시적 계속 없이 전투를 재개해서는 안 된다.

## 확인한 개발 통합과 실제 증거

[PR #426](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/426)은 선택 MeshHazardsV1 animation Load → Apply/Undo/Redo → Save/reopen → schema 7 export → 원본 숨김 native 소비를 개발 브랜치에 통합했다. 별도 capability·bounded provenance·runtime trust와 schemas 1–6/default-off를 보존했다. [정확한 CI·산출물](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/actions/runs/41892258398)과 [수용 기록](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/426#issuecomment-6639723379)은 당시 tree에 한정된다.

[PR #427](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/427)과 [#428](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/428)은 복원 중 포커스 상실 뒤 자동 재개와 해제된 드래그의 부활을 수정했다. 개발 통합의 [포커스 CI](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/actions/runs/41897428527)·[포인터 CI](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/actions/runs/41900065738) 및 독립 산출물 검증으로 관련23·포커스10·포인터24·입력 검사2를 확인했다. 실제 WASM/mock-DOM 증거이며 실제 브라우저 수용이 아니다.

[PR #429](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/429)은 네이티브의 마지막 hover가 이전 클릭을 Restart·구매·보드 명령으로 잘못 바꾸던 결함을 수정했다. 개발 merge b0e7a6cf6d85c95b3b17bed432e95bb87fe7b9a8, tree 87dbc8d07a77b63eccd93a036172d9a77a97b3ea에서 [필수 CI와 독립 산출물 검증](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/429#issuecomment-6640886369)을 완료했다. UI 17개·엄격 검사·기능 제외 빌드와 다운로드한 native 실행 파일의 authored 3-wave 승리(tick 1884, reactor 75)를 확인했다. CPU egui/Session 증거이며 창·물리 입력 성공으로 확대하지 않는다.

[PR #430](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/430)은 native Pause/Continue와 포커스 복귀 후 정지 유지를 추가했다. 개발 merge 055b10c308d37ecbcf335da0777a00961b0ae4f2, tree f39beeb8d4a982891f299c72a60b6c7ab17effd6에서 [필수 CI·독립 산출물 재실행](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/430#issuecomment-6641518896)을 완료했다. App20/UI17/View2·엄격 검사·기능 제외·production 빌드와 동일한 authored 승리를 확인했다. 승인된 delay2 입력은 보존하고 미제출 UI 큐만 취소한다.

[PR #431](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/431)은 선택 Linux native Save Copy/Load를 추가했다. 개발 merge d62eeaad3c562c20484d1b3483606aa4f1420910, tree c7e173059dc83efb5f1c2b217777a4d47359f94e의 [필수 CI·독립 산출물 검사](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/431#issuecomment-6642870240)를 완료했다. 동일 실행 파일·제한된 입력 승인 기록으로 delay2와 미래 입력을 보존한다. 기능66/호환39·strict·별도 프로세스31 Frame 동등성과 실제 headless 승리를 확인했다. 창·장치 검증은 아니다.

[PR #432](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/432)은 선택 Linux native authored audio를 추가했다. 개발 merge 9625fcc9479ca055bcf0b2900258e156a4bdfdaa, tree ecf8a9eda1d2bb3fb8925bfbb72380105448fda9의 [필수 CI·독립 산출물 재실행](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/432#issuecomment-6644273066)을 완료했다. 원본을 숨긴 CLI bank → 실제 App/Session Kira PCM과 음소거·정지·재시작·저장복원을 확인했다. 산출물의 source-path 회귀도 원래 byte 검사를 보존해 수정했다. 실제 스피커 청취는 미검증이다.

[PR #433](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/433)은 native delay2 소비 전 다음 셀 클릭이 선택을 잃던 결함을 수정했다. 개발 merge 40cfab12a873ef196f4995dd4044bafc7943c2f4, tree 9890f9c4cab20b9f174c2d634db6a67b13b9c525의 [필수 CI](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/actions/runs/41944029205)와 독립 산출물 재실행을 완료했다. Pending 표시·선택 보존·소비 후 새 클릭·pause/focus/restore 경계를 검증했으며 클릭을 자동 버퍼링하지 않는다.

[PR #434](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/434)은 Ctrl/Cmd+R이 Invest로 소비되던 결함을 수정했다. 개발 merge 125d95e99b2c17a92ecca903ea904416cfca9e14, tree ed9eefb33e4e1089cefead8c2370cec22afe8e07의 [필수 CI](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/actions/runs/41947729478)와 독립 산출물 재실행이 통과했다. 키보드112·기존 포커스10/포인터24·관련23(건너뜀0)을 확인하고 plain/Shift 입력을 보존했다.

[PR #435](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/435)은 승인된 종료 후 Restart가 포커스 복귀만으로 실행되던 결함을 수정했다. 개발 merge c16efdd3c9df767277aadc4a03e65bcfae3409c0, tree 043dcceaa7f0f930480ae6800062ee7d98e7a2cf의 [필수 CI·독립 산출물 재실행](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/435#issuecomment-6647819957)이 통과했다. 기존 미래 입력을 보존하며 명시적 Continue를 기다리고, LOST/WON·반복 포커스·저장복원 경계를 검증했다.

- **과거 Windows 직접 플레이:** 10-wave 승리, 369 kills, 29 merges, core 8/30 및 Wave 2 패배 → R 재시작을 관측했다. [원래 증거](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6430939337)는 당시 산출물에만 적용하며 현재 소스의 Windows 재검증으로 옮기지 않는다.
- **현재 Stormglass 실행 범위:** 실제 native/WASM consumed-input trace와 source-hidden packed 소비, mock-DOM 제어/저장 검사는 실제 브라우저 파일 선택·표시·물리 입력과 별개다. 2026-11-04 Chromium 사전 검사는 페이지 로드 전 Unix socket EPERM으로 실패했고 정식 확대 실행도 executor 초기화에서 실패했다. 실제 브라우저 플레이는 **미검증**이며 다른 환경 실행으로 이 제한을 우회하지 않았다.

## 남은 우선순위와 경계

1. 실제 제작·완주 경로를 우선한다. [등록된 starter 기본 장면의 이동 제어 공백](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6648286419)을 재현했다. Command-mode 경로는 Act만 제공해 Delivery의 원래 목표와 Switchboard의 target 5에 도달하지 못한다. 기존 일반 입력은 두 장면을 완주한다. [프로젝트 소유 단발 이동·선택 계약](registered-starter-navigation.md)과 입력 build migration을 구현하고, 지연 관찰·중단·기록 재생까지 검증한다. 아직 최종 검증·병합 완료가 아니다.
2. 허용된 실행 환경이 있을 때 현재 산출물의 브라우저/native-window 플레이를 별도 수용한다. 화면 증거가 없으면 성공률·모바일·Windows·hardware GPU까지 확대하지 않는다.
3. [설계 v2](design-v2.md)의 사전 고정 과제/seed/oracle·예산·실제 모델 기준선을 준비한다. 모델 호출·업로드·비용은 별도 승인 범위이며 아직 측정 결과가 없다.

Su가 구현·검증·독립 리뷰와 파일 소유권을 조정한다. 종료된 외부 담당에게 새 작업을 배정하지 않는다. Held Collect/CUBICSPLINE과 게시가 거부된 EnemyMotion을 인수하거나 재게시하지 않는다. ERP의 명시적 loopback/token·capability·lease·sequence 계약은 [등록 API 문서](registered-gameplay-api.md)를 유지한다.

## 보존할 검사와 실패

[PR #309의 정확한 Linux-local baseline](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/309#issuecomment-6460288981)과 [후속 PR #404/#405 결합 검증](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6616200365)은 당시 source/환경의 증거다. 변경된 경로에는 해당 검사를 새로 실행하고 필수 CI·독립 리뷰 후 개발 브랜치에만 통합한다. Retired/failed/unrun 검사를 성공으로 옮기지 않으며 보안·인증·보호 설정을 우회하지 않는다.

Advisory formatting debt, [원래 compile/중단 기록](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6450381753), PR #139/#141의 Windows 실패/미재현 한계, #426의 최초 compile/EXDEV 실패를 보존한다. Raw logs·ZIP·JSON은 git 밖에 source/tree·toolchain·명령·종료 상태와 연결한다. #2의 기존 분모 22는 그대로이며 #94 하위 기능/PR 수로 엔진 완성률을 재계산하지 않는다.
