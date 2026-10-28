# Orrery 현재 상태

갱신: 2026-10-27 UTC · [설계 v2](design-v2.md) · [상위 추적 #2](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/2) · [게임 제작 #94](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94)

## 지금의 제품 경로

**실제 게임 제작 기능 구현이 우선**이다. 자연어 요구에서 사용자 소유 게임의 작성·실행·수정·재현·전달까지 연결한다. Stormglass Foundry/Coop Defense를 실제 게임 기준으로 삼되, AI 제작·조작·검증은 구조화된 API를 기본으로 하고 화면 조작은 보완 QA로 구분한다. 기능·PR·테스트 수는 게임 완성률이 아니다. 무개입 제작 성공률과 실제 모델 기반 기준선은 아직 미측정이며 mock/scripted 성공으로 대체하지 않는다.

- 결정론 코어: 고정소수점, 순서 고정 Game/System, 롤백·리플레이, 시뮬/뷰 경계를 유지한다.
- 제작 경로: 정적 Rust starter의 Delivery/Switchboard, 등록 프로젝트, CLI/ERP, 에디터와 standalone/bundle을 연결한다. 생성 성공과 source-hidden 전달 성공은 별도 수용 검사다.
- 패키지 경계: 선택적 `stormglass` feature는 기존 `DefenseGame`을 등록한다. 엔진에 게임별 enum이나 복제 시뮬레이션을 추가하지 않는다. 현재 빈 초기 preset만 지원하며 임의 웨이브/scene 제작이나 런타임 cross-game 전환은 완료 범위가 아니다.

## 실제 플레이와 API 검증

- **Windows native 직접 플레이 관측 완료:** 10-wave 승리, 369 kills, 29 merges, core 8/30 및 Wave 2 패배 → R 재시작. [#94 관측 기록](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6430939337), [PR #302 정정 근거](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/302#issuecomment-6430941055). PR #302의 이전 “새 Windows PC에서 미확인” 문구를 현재 상태로 옮기지 않는다. 이 관측은 당시 native 산출물의 증거이지 새 통합 후보 전체의 검증이 아니다.
- **PR #306 `da0a7307` 집중 검증:** starter library 23 tests PASS. 실제 Editor로 Stormglass manifest를 열고 summon 입력 소비·neutral 완료·Stop 원본 보존을 확인했다. 별도 실제 loopback WebSocket 테스트는 Delivery/Switchboard 두 등록 게임의 reader/controller 권한, 입력 소비·neutral 완료·Stop을 확인한다. [현재 결과 기록](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/306#issuecomment-6444452695). CPU/API 검증이며 렌더 창·GPU·원격 배포·bad-token·reconnect 또는 전체 workspace 인증이 아니다.
- ERP listener는 명시적 token을 요구하는 numeric loopback opt-in이다. 기본은 비활성이며 무인증 listener 옵션은 없다. command-line token은 로컬 프로세스 검사에 노출될 수 있다. [등록/API 계약과 한계](registered-gameplay-api.md)를 유지한다.
- 동료는 로컬 AI다. 온라인 협동·계정·매칭·진행 저장, production balance와 원작 전체 재현 완료는 주장하지 않는다.

## 개발 통합 후보와 남은 게이트

문서 조정 전 후보는 `1ba6c8f833f91a078e9153eedce9474a204a8724`이다. 최신 PR #306 `da0a7307`에 검토된 PR #298 `389b521b`(타일맵 Pick), #300 `5032b474`(browser sandbox/명시적 GPU 옵션), #301 `08653f36`(입력 초안 충돌 preview)의 변경을 충돌 없이 반영했다. `app.rs`의 예약된 timeline 작업은 포함하지 않았다. PR #302 `acdaea9c`의 실제 게임 우선 의도는 이 문서에 조정 반영하며 오래된 실행 상태나 main 병합 문구는 승계하지 않는다.

1. 이 문서 변경까지 포함한 **정확한 최종 후보**에서 `bash tools/check-linux-local.sh`와 변경 경로별 집중 검사, 독립 리뷰를 완료한다. 현재 전체 필수 Linux 게이트와 통합 리뷰는 **미완료**다. 개별 PR의 과거 통과를 새 후보 통과로 이전하지 않는다.
2. 결과·실패·미검증 범위를 기록하고 검증된 후보를 게시하여 PR을 갱신한 뒤 승인된 개발 브랜치로 병합한다. 개발 브랜치 병합은 아직 완료되지 않았다. 최신 사용자 지시로 **main 및 PR #1 병합은 제외**한다.
3. 이후 실제 게임 제작 흐름의 확인된 빈틈을 우선 고친다. 독립 사용자 소스·입력·편집/undo·재현·최종 전달을 연결하며 새 범용 도구나 평가 준비로 게임 제작을 미루지 않는다.

2026-10-23 지시로 hosted workflows가 제거되었지만 필수 검증을 면제한 것은 아니다. 기존 테스트·보안·인증·민감정보 제한은 유지한다. 서버 측 required check가 남으면 별도 차단점으로 보고하며 보호 설정을 바꾸거나 우회하지 않는다. Linux/API 성공은 별도 증거가 없는 플랫폼·hardware/GPU 검증이 아니다.

## 보존할 증거

[#94 reflection 보정](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6356552702), [compile 예산 실패](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6356609143), [private fixture 계약](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6356618808)과 [이전 실패 기록](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/blob/f20097d6aeb2b4a110a7145b58ea07e0c7a0e8e6/docs/progress.md)을 보존한다. 새 실행의 raw logs·ZIP·JSON은 git 밖에 source tree·toolchain·명령·종료 상태와 함께 남긴다. 실패·중단을 지우거나 통과로 바꾸지 않는다.
