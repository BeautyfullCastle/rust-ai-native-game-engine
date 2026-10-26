# Orrery 현재 상태

갱신: 2026-10-26 UTC · [설계 v2](design-v2.md) · [상위 추적 #2](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/2) · [게임 제작 #94](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94)

## 지금의 제품 경로

지금은 **Stormglass Foundry/Coop Defense를 실제 실행하고 직접 플레이하는 일**이 최우선이다. 조작, 전투, 웨이브 진행, 승리·패배, 재시작을 확인하고 플레이 차단 문제를 수정한 뒤 같은 게임에서 재검증한다. 패키지 및 일반 도구 개선은 실제 플레이의 확인된 차단 문제일 때만 우선한다. 기능 수·PR 수·테스트 통과 수는 게임 플레이 성공이나 엔진 완성도의 대체 증거가 아니다.

자연어 요구에서 게임 제작·실행·수정·재현·전달로 이어지는 정적 Rust 경로는 유지한다. 무개입 제작 성공률은 후속 측정 지표이며 실제 모델 기반 기준선은 아직 없다. 이 지표의 준비로 현재 게임 플레이 확인을 미루지 않는다.

- 결정론 코어: 고정소수점, 순서가 고정된 Game/System, 롤백·리플레이와 시뮬/뷰 분리가 구현되어 있다. Linux 로컬 검증으로 전환하며, 교차 플랫폼 동일성은 이번 실행으로 증명하지 않는다.
- 제작 경로: `examples/static-game-starter`의 Delivery/Switchboard와 독립 사용자 프로젝트 생성기가 있다. 사용자 소스·자체 lockfile·native 실행·source-hidden 전달은 각각 해당 수용 검사로 판단한다. 생성 성공만으로 게임 전달 완료를 선언하지 않는다.
- 다중 액션: 선택적 `static-game-actions`와 명시적 `multi-action-preview`가 있다. 기존 기본 프로필을 조용히 새 pin으로 바꾸지 않는다. private fixture는 새 사용자 게임 완료 수로 세지 않는다.
- 에이전트 평가: `tools/arena_agent_trial.py`의 다섯 목표·실제 host·독립 oracle·중단/재개 기록이 있다. 현재 solver는 mock이며, scripted control이나 mock 통과를 실제 모델 성공률로 환산하지 않는다.

## 실제 플레이 상태와 다음 행동

- 기준: PR299 `1c08eca7577b4155f9833708e8b9a1de4bbeee83`의 소스 안내와 새 Windows PC의 기존 작업 산출물을 조사했다. **이 PC의 Stormglass 직접 플레이는 미확인**이다. 로컬 저장소/기존 target에서 `game.wasm`·완성 web dist·`coop_defense.exe`를 확인하지 못했다. `coop_defense_run`은 headless oracle이므로 직접 플레이 증거가 아니다. 다른 게임이나 다른 PC의 성공으로 대체하지 않는다.
- 기존 Library의 Stormglass 오프라인 HTML 존재는 확인했다. 공식 로컬 가져오기 helper가 Windows의 확장 속성 API 미지원으로 실패하여 정상 가져오기·직접 실행은 미확인이다. helper를 우회하지 않는다. 다음 대안은 [게임 실행 안내](coop-defense.md)의 `orr_coop_defense_web` 전용 web 빌드 또는 `orr_sample/coop-defense`의 `coop_defense` 단일 binary 빌드다. 아직 시작하지 않았으며 전체 workspace 재빌드는 하지 않는다.
- 실행 경로를 확보한 뒤 실제 화면에서 조작·전투·웨이브·승패·재시작을 관찰한다. 성공/실패/미확인을 나누고, 차단 문제를 고쳤다면 같은 산출물 계보의 게임에서 재검증한다. headless·모의 DOM·테스트 개수는 이 관측을 대신하지 않는다.
- 동료는 로컬 AI다. 온라인 협동·계정·매칭·진행 저장과 production balance는 미구현/미확인 범위를 유지한다. 원작 전체 재현 완료도 주장하지 않는다. 이번 문서 작업은 게임 실행 성공이나 병합 완료 선언이 아니다.

## 이전 통합 상태와 인계 기록

아래 2026-10-23 기록과 실패 근거는 보존한다. 현재 작업 순서는 위 실제 플레이 경로이며, 이번 단계에서는 main·PR1을 포함한 어떤 브랜치도 병합하지 않는다.

이번 요약의 소스 기준은 [PR #289](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/289)의 `f20097d6aeb2b4a110a7145b58ea07e0c7a0e8e6`, tree `b85846f63f77773624208fd5d93cf18f09d87da7`이다. 이것은 게시된 후보 기준이지 통합 완료 선언이 아니다. 새 Linux-only 후보는 이 계보의 후속 후보이며, 로컬 검증 및 리뷰가 끝나기 전 통과나 병합 완료로 표시하지 않는다.

1. 기존 후보들을 하나의 개발 통합 후보로 정리한다. ancestry와 실제 변경의 포함 여부를 확인하고 충돌/누락만 해결한다. 이미 검토한 동일 소스의 광범위 감사를 반복하지 않는다. 고아 브랜치는 개별 변경을 살릴지/보류할지 분류하고, 통합·보존 여부와 삭제 대상을 확인한 병합 브랜치만 정리한다. 단순 브랜치 개수나 ancestry만으로 고유 작업을 버리지 않는다.
2. 사용자 지시 “리눅스 로컬빌드로만 검증해 ci 다 없애고”에 따라 GitHub Actions workflow 47개를 제거한다. `bash tools/check-linux-local.sh`와 변경 경로별 Linux 검사를 정확한 최종 tree에서 실행하고 독립 리뷰를 완료한다. 현재 실행 결과는 **대기**이며, 진행 중·미실행·실패는 통과가 아니다.
3. 이전 필수 hosted CI 및 Room mirror Windows hardware/UI 병합 조건은 **사용자 지시로 대체**되었다. 과거 실패를 성공으로 바꾸거나 Windows·macOS·Android·browser·GPU 검증을 주장하지 않는다. 검증과 독립 리뷰 후 사용자 명시적 요청대로 hardening 다음 main에 통합한다. 아직 병합 완료를 선언하지 않으며 PR #1 자체 병합은 포함하지 않는다. 서버 측 required check가 남으면 별도 차단점으로 보고하며 보호 설정은 변경하지 않는다.
4. 통합된 제작 경로로 작은 게임 하나를 끝까지 전달한 뒤, 설계 v2의 고정 과제군을 실행한다. 새 스크립트 런타임보다 기존 경로의 실패·빌드 대기·중간 개입을 먼저 줄인다.

## 최근 실패와 증거 위치

- [#94 reflection 기록](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6356552702): 필수 정확 테스트 목록 누락을 보정했다. 원래 실패 artifact `12474014053`을 보존하며, 보정 소스와 hosted 통과를 구별한다.
- [#94 compile 예산 기록](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6356609143): 첫 cold compile의 900초 실패 뒤 총 2700초 예산을 재배분했다. 원래 실패 artifact `12472729915`과 총 예산을 유지한다.
- [#94 fixture 기록](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6356618808): private binary의 실제 계약 테스트를 harness에 노출했다. 기존 guard나 테스트를 삭제하지 않았다.
- [이전 진행 기록 전체](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/blob/f20097d6aeb2b4a110a7145b58ea07e0c7a0e8e6/docs/progress.md)에 CI #57 픽셀 실패와 #58 수정, #75 relay bootstrap stall, #76 Windows idle-timeout 실패, GPU VIEW_FORMATS 실패 등 당시 근거가 남아 있다. 이 요약은 과거 실패를 새 성공으로 바꾸거나 미해결 항목을 닫지 않는다.

새 Linux 실행 로그·ZIP·JSON 결과는 성공·실패·중단 모두 git 밖에 보관하고 정확한 source tree·toolchain·명령·종료 상태를 기록한다. 기존 CI artifact와 workflow 원본의 외부 보존본은 삭제하지 않는다. 저장소에는 재현 가능한 테스트·작은 고정 fixture·필수 계약만 두고, 이 문서에는 source SHA, 결과, 차단점, 다음 행동, 증거 링크만 유지한다. 전체 기능/브랜치/CI 개수를 완성률로 쓰지 않는다.
