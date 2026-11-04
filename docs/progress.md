# Orrery 현재 상태

갱신: 2026-11-04 UTC · [설계 v2](design-v2.md) · [상위 추적 #2](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/2) · [게임 제작 #94](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94)

## 실제 게임 제작 경로

자연어 요구에서 사용자 소유 게임의 작성·실행·수정·재현·전달까지 연결한다. Stormglass Foundry/Coop Defense를 실제 게임 기준으로 삼고 구조화된 API를 기본으로 사용한다. 화면 조작은 보완 QA다. 기능·PR·테스트 수는 게임 완성률이 아니며 **실제 모델 기반 무개입 제작 성공률은 아직 미측정**이다. Mock/scripted 성공으로 대신하지 않는다.

- 결정론 코어의 고정소수점·순서 고정·롤백·리플레이·시뮬/뷰 경계를 유지한다. 선택적 게임 패키지는 기존 Game/등록/입력 codec/presenter를 사용하며 엔진에 게임별 시뮬레이션을 복제하지 않는다.
- Stormglass는 공통 ERP/등록 Editor의 편집·Undo/Redo·Save/reopen과 native/WASM 소비를 연결한다. 현재 작성 계약은 Scenario, Campaign, WavePacing, WaveCounts, EnemyHealth, WaveComposition의 **여섯 singleton**이다. 자원·HP·마지막 웨이브·생성 간격·웨이브별 수·적 종류/첫 보스를 작성한다. [범위와 예제](stormglass-authoring.md)를 따르며 보류 중 EnemyMotion을 포함하지 않는다.
- Browser 로컬 Scene import와 authored-scene offline packing은 구현돼 있다. 단일 HTML에 고정된 WASM/자산/장면을 넣어 전달할 수 있다. [Run Save v1](coop-defense-run-save.md)은 같은 WASM의 유한 입력 기록을 재생하는 저장/복원이며 일반 Frame checkpoint·버전 migration이 아니다. 온라인 협동·계정·매칭·production balance는 완료 주장에 포함하지 않는다.
- 복원 실패는 기존 게임/기록을 보존한다. 다만 포커스 상실은 별도 안전 이벤트로, 복원 중에도 전투를 일시정지하고 대기 입력/제스처를 취소한다. 실패/취소한 복원이 이를 되돌리거나 명시적 계속 없이 전투를 재개해서는 안 된다.

## 확인한 개발 통합과 실제 증거

[PR #426](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/426)은 개발 브랜치 su/engine-correctness-hardening에 정상 병합됐다. 확인한 merge는 ffccd8ae999b0852df7977c3ccbd49ea433a277a, tree는 21f955a3aaabdcf3dcd479b070b5ea45628586b2다. 검토·로컬 검증·필수 hosted CI가 같은 tree를 사용했다. Main 및 PR #1은 병합하지 않았다.

이 선택 MeshHazardsV1 경로는 animation Load → Apply/Undo/Redo → Save → fresh reopen → schema 7 export → 원본을 숨긴 relocated native player를 연결한다. 별도 animation capability·bounded embedded provenance·exact inventory·runtime trust를 유지하며 schemas 1–6/default-off 동작을 보존한다. 새 interpolation 지원이나 코어 시뮬 변경이 아니다.

후속 [PR #427](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/427)은 복원 중 포커스 상실 뒤 전투가 자동 재개되던 결함을 수정했다. 개발 merge e074e5510ace4c7bb24b21b9eee5de5b0a11a3ac의 tree 9771b95d161e7166befae2cf2fb2a509617557ad는 로컬·독립 리뷰·[필수 CI](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/actions/runs/41897428527) 후보와 같다. 관련 23개 검사와 seed/scene의 10개 lifecycle 사례가 통과했으며 실제 WASM/mock-DOM 증거다.

[PR #428](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/428)은 복원 실패 뒤 해제된 드래그가 남아 새 입력을 막거나 의도치 않은 이동을 낳던 결함을 수정했다. 개발 merge f3f70b42a67ab31dd23a87be659ec297030dbf26, tree 3ada46ab934d911d2e1cc0c225c852a2e2df4ffe의 [필수 CI](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/actions/runs/41900065738)와 독립 산출물 검증을 완료했다. 포인터 24개·포커스 10개·관련 23개 및 기존 입력 검사 2개를 확인했으며 실제 브라우저 증거가 아니다.

[PR #429](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/429)은 네이티브의 마지막 hover가 이전 클릭을 Restart·구매·보드 명령으로 잘못 바꾸던 결함을 수정했다. 개발 merge b0e7a6cf6d85c95b3b17bed432e95bb87fe7b9a8, tree 87dbc8d07a77b63eccd93a036172d9a77a97b3ea에서 [필수 CI와 독립 산출물 검증](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/429#issuecomment-6640886369)을 완료했다. UI 17개·엄격 검사·기능 제외 빌드와 다운로드한 native 실행 파일의 authored 3-wave 승리(tick 1884, reactor 75)를 확인했다. CPU egui/Session 증거이며 창·물리 입력 성공으로 확대하지 않는다.

- [정확한 head의 필수 CI](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/actions/runs/41892258398)는 첫 실행에 성공했다. [Artifact](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/actions/runs/41892258398/artifacts/13354138228)의 source/tree·실행 수 8+2+1+1+1+2·runtime digest·tick 0/30/restart parity를 다운로드해 검증했다. 로컬 집중 검사와 독립 리뷰 범위는 [검증 기록](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/426#issuecomment-6639723379)에 있다. 전체 workspace 신규 실행이나 GPU/창 입력 증거는 아니다.
- **과거 Windows 직접 플레이:** 10-wave 승리, 369 kills, 29 merges, core 8/30 및 Wave 2 패배 → R 재시작을 관측했다. [원래 증거](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6430939337)는 당시 산출물에만 적용하며 현재 소스의 Windows 재검증으로 옮기지 않는다.
- **현재 Stormglass 실행 범위:** 실제 native/WASM consumed-input trace와 source-hidden packed 소비, mock-DOM 제어/저장 검사는 실제 브라우저 파일 선택·표시·물리 입력과 별개다. 2026-11-04 Chromium 사전 검사는 페이지 로드 전 Unix socket EPERM으로 실패했고 정식 확대 실행도 executor 초기화에서 실패했다. 실제 브라우저 플레이는 **미검증**이며 다른 환경 실행으로 이 제한을 우회하지 않았다.

## 남은 우선순위와 경계

1. 실제 플레이의 입력·승패·재시작·저장복원 단절을 먼저 고친다. 현재 native에는 수동 정지·계속 조작이 없고 포커스를 되찾으면 전투가 즉시 재개된다. [현재 범위](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6641137007)는 #106의 pause/resume 목표에 맞는 선택 native 플레이 기능이다. 정지 중 실제 Session Frame 보존, 명시적 계속, 미제출 입력·옛 제스처 취소, 시간 누적 방지를 CPU UI→생산 Session 경로에서 검증한다. 이미 Session에 승인된 지연 입력의 의미와 코어 시뮬레이션은 바꾸지 않는다.
2. 허용된 실행 환경이 있을 때 현재 산출물의 브라우저/native-window 플레이를 별도 수용한다. 화면 증거가 없으면 성공률·모바일·Windows·hardware GPU까지 확대하지 않는다.
3. [설계 v2](design-v2.md)의 사전 고정 과제/seed/oracle·예산·실제 모델 기준선을 준비한다. 모델 호출·업로드·비용은 별도 승인 범위이며 아직 측정 결과가 없다.

Su가 구현·검증·독립 리뷰와 파일 소유권을 조정한다. 종료된 외부 담당에게 새 작업을 배정하지 않는다. Held Collect/CUBICSPLINE과 게시가 거부된 EnemyMotion을 인수하거나 재게시하지 않는다. ERP의 명시적 loopback/token·capability·lease·sequence 계약은 [등록 API 문서](registered-gameplay-api.md)를 유지한다.

## 보존할 검사와 실패

[PR #309의 정확한 Linux-local baseline](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/309#issuecomment-6460288981)과 [후속 PR #404/#405 결합 검증](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6616200365)은 당시 source/환경의 증거다. 변경된 경로에는 해당 검사를 새로 실행하고 필수 CI·독립 리뷰 후 개발 브랜치에만 통합한다. Retired/failed/unrun 검사를 성공으로 옮기지 않으며 보안·인증·보호 설정을 우회하지 않는다.

Advisory formatting debt, [원래 compile/중단 기록](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6450381753), PR #139/#141의 Windows 실패/미재현 한계, #426의 최초 compile/EXDEV 실패를 보존한다. Raw logs·ZIP·JSON은 git 밖에 source/tree·toolchain·명령·종료 상태와 연결한다. #2의 기존 분모 22는 그대로이며 #94 하위 기능/PR 수로 엔진 완성률을 재계산하지 않는다.
