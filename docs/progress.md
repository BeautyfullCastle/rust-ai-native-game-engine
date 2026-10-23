# Orrery 현재 상태

갱신: 2026-10-23 UTC · [설계 v2](design-v2.md) · [상위 추적 #2](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/2) · [게임 제작 #94](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94)

## 지금의 제품 경로

자연어 요구를 받은 에이전트가 작은 게임을 만들고, 직접 실행·수정·재현하여 전달하는 것이 목표다. 현재 경로는 정적 Rust 게임과 CLI/ERP, 에디터, standalone player, bundle이다. 다음 성과는 기능·PR·테스트 개수 증가가 아니라 **사람의 중간 수정 없이 끝난 게임 제작 과제의 비율**로 측정한다. 실제 모델 기반 기준선은 아직 없다.

- 결정론 코어: 고정소수점, 순서가 고정된 Game/System, 롤백·리플레이와 시뮬/뷰 분리가 구현되어 있다. 필수 교차 플랫폼 검증을 유지한다.
- 제작 경로: `examples/static-game-starter`의 Delivery/Switchboard와 독립 사용자 프로젝트 생성기가 있다. 사용자 소스·자체 lockfile·native 실행·source-hidden 전달은 각각 해당 수용 검사로 판단한다. 생성 성공만으로 게임 전달 완료를 선언하지 않는다.
- 다중 액션: 선택적 `static-game-actions`와 명시적 `multi-action-preview`가 있다. 기존 기본 프로필을 조용히 새 pin으로 바꾸지 않는다. private fixture는 새 사용자 게임 완료 수로 세지 않는다.
- 에이전트 평가: `tools/arena_agent_trial.py`의 다섯 목표·실제 host·독립 oracle·중단/재개 기록이 있다. 현재 solver는 mock이며, scripted control이나 mock 통과를 실제 모델 성공률로 환산하지 않는다.

## 통합 상태와 차단점

이번 요약의 소스 기준은 [PR #289](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/289)의 `f20097d6aeb2b4a110a7145b58ea07e0c7a0e8e6`, tree `b85846f63f77773624208fd5d93cf18f09d87da7`이다. 이것은 게시된 후보 기준이지 통합 완료 선언이 아니다. 현재 CI의 최종 판정은 해당 head의 실행 결과로만 갱신한다.

1. 기존 후보들을 하나의 개발 통합 후보로 정리한다. ancestry와 실제 변경의 포함 여부를 확인하고 충돌/누락만 해결한다. 이미 검토한 동일 소스의 광범위 감사를 반복하지 않는다. 고아 브랜치는 개별 변경을 살릴지/보류할지 분류하고, 통합·보존 여부와 삭제 대상을 확인한 병합 브랜치만 정리한다. 단순 브랜치 개수나 ancestry만으로 고유 작업을 버리지 않는다.
2. 최종 통합 tree에서 필수 CI, 필요한 native/source-hidden 수용, 독립 리뷰를 완료한다. 특히 Room mirror의 Windows hardware/UI 조건은 남아 있다. 진행 중·미실행·실패는 통과가 아니다.
3. 사용자가 요청한 순서대로 검증된 후보를 hardening에 통합한 뒤 main으로 올린다. 각 통합 head의 필수 CI·리뷰·hardware/UI 조건을 충족하기 전에는 병합하지 않는다. PR #1 자체의 병합을 자동으로 수행한다는 뜻은 아니다. 이후 main에서 짧게 분기하고 검증된 변경을 자주 통합한다. 일일 통합을 위해 게이트를 건너뛰지 않는다.
4. 통합된 제작 경로로 작은 게임 하나를 끝까지 전달한 뒤, 설계 v2의 고정 과제군을 실행한다. 새 스크립트 런타임보다 기존 경로의 실패·빌드 대기·중간 개입을 먼저 줄인다.

## 최근 실패와 증거 위치

- [#94 reflection 기록](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6356552702): 필수 정확 테스트 목록 누락을 보정했다. 원래 실패 artifact `12474014053`을 보존하며, 보정 소스와 hosted 통과를 구별한다.
- [#94 compile 예산 기록](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6356609143): 첫 cold compile의 900초 실패 뒤 총 2700초 예산을 재배분했다. 원래 실패 artifact `12472729915`과 총 예산을 유지한다.
- [#94 fixture 기록](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6356618808): private binary의 실제 계약 테스트를 harness에 노출했다. 기존 guard나 테스트를 삭제하지 않았다.
- [이전 진행 기록 전체](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/blob/f20097d6aeb2b4a110a7145b58ea07e0c7a0e8e6/docs/progress.md)에 CI #57 픽셀 실패와 #58 수정, #75 relay bootstrap stall, #76 Windows idle-timeout 실패, GPU VIEW_FORMATS 실패 등 당시 근거가 남아 있다. 이 요약은 과거 실패를 새 성공으로 바꾸거나 미해결 항목을 닫지 않는다.

새 실행 로그·ZIP·JSON 결과는 성공/실패 모두 CI artifact에 보관한다. 저장소에는 재현 가능한 테스트·작은 고정 fixture·필수 계약만 두고, 이 문서에는 source SHA, 결과, 차단점, 다음 행동, 증거 링크만 유지한다. 전체 기능/브랜치/CI 개수를 완성률로 쓰지 않는다.
