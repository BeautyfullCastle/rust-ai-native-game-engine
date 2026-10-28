# Orrery 현재 상태

갱신: 2026-10-28 13:25 KST (개발 병합 확인) · [설계 v2](design-v2.md) · [상위 추적 #2](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/2) · [게임 제작 #94](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94)

## 지금의 제품 경로

**실제 게임 제작 기능 구현이 우선**이다. 자연어 요구에서 사용자 소유 게임의 작성·실행·수정·재현·전달까지 연결한다. Stormglass Foundry/Coop Defense를 실제 게임 기준으로 삼되, AI 제작·조작·검증은 구조화된 API를 기본으로 하고 화면 조작은 보완 QA로 구분한다. 기능·PR·테스트 수는 게임 완성률이 아니다. 무개입 제작 성공률과 실제 모델 기반 기준선은 아직 미측정이며 mock/scripted 성공으로 대체하지 않는다.

- 결정론 코어: 고정소수점, 순서 고정 Game/System, 롤백·리플레이, 시뮬/뷰 경계를 유지한다.
- 제작 경로: 정적 Rust starter의 Delivery/Switchboard, 등록 프로젝트, CLI/ERP, 에디터와 standalone/bundle을 연결한다. 생성 성공과 source-hidden 전달 성공은 별도 수용 검사다.
- 패키지 경계: 선택적 `stormglass` feature는 기존 `DefenseGame`을 등록한다. 엔진에 게임별 enum이나 복제 시뮬레이션을 추가하지 않는다. 공통 ERP 편집·history·undo·save/reopen으로 `Scenario` 시작 gold(0–10000, 기본 150)와 reactor HP(1–1000, 기본 30)를 작성한다. 저장 Scene은 모든 선언 필드를 요구하며 최초 필드 편집에는 reflected default를 쓴다. 임의 웨이브 제작과 런타임 cross-game 전환은 미완료다.

## 실제 플레이와 API 검증

- **Windows native 직접 플레이 관측 완료:** 10-wave 승리, 369 kills, 29 merges, core 8/30 및 Wave 2 패배 → R 재시작. [#94 관측 기록](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6430939337), [PR #302 정정 근거](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/302#issuecomment-6430941055). PR #302의 이전 “새 Windows PC에서 미확인” 문구를 현재 상태로 옮기지 않는다. 이 관측은 당시 native 산출물의 증거이지 새 통합 후보 전체의 검증이 아니다.
- **현재 개발 소스의 CPU/API 검증:** 등록 프로젝트와 공통 presenter/ERP 경로에서 입력 접수·실제 소비·neutral 완료를 구분한다. 선택적 `coop-defense-scene`은 기존 parse/bake/reflection을 재사용하며 코어에 ERP/editor를 강제하지 않는다. Native `coop_defense --scene PATH`는 창/세션 생성 전에 bounded regular UTF-8 파일을 검사하고 headless/windowed에 동일 Scenario를 전달한다. 실제 headless 실행에서 authored HP 75/기본 HP 30과 누락 파일 exit 1을 확인했다. Native HUD와 ERP/descriptor는 공통 Scenario/build identity를 사용한다. 창/GPU 실행 증거는 아니다. [원래 PR #306 집중 결과](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/306#issuecomment-6444452695), [native 작성 기록](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6450626300).
- ERP listener는 명시적 token을 요구하는 numeric loopback opt-in이다. 기본은 비활성이며 무인증 listener 옵션은 없다. command-line token은 로컬 프로세스 검사에 노출될 수 있다. [등록/API 계약과 한계](registered-gameplay-api.md)를 유지한다.
- 동료는 로컬 AI다. 온라인 협동·계정·매칭·진행 저장, production balance와 원작 전체 재현 완료는 주장하지 않는다.

## 개발 통합 완료와 검증 경계

[PR #309](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/309)는 2026-10-28 13:25 KST에 `su/engine-correctness-hardening`으로 병합되었다. 병합 SHA `a6844afbf17187f169a8364b6cfa4f19ad5a88b9`의 tree **`ca3a225bfb5a26155dee04ebb45fb3d78073cb2f`**는 검증 후보 `af91fd9f881225c7503f7abf8c2a5bb8c6e1ef5d`와 동일하다. 구현·독립 리뷰·개발 후보 조합·공유 개발 통합은 완료했으나 **main 및 PR #1은 병합하지 않았다**. 누적 71개 파일에는 PR #298 Pick, #300 browser 옵션, #301 입력 초안 충돌 preview와 등록/API·authored Scenario·native scene 경로가 포함된다. 예약된 `app.rs` timeline 작업은 포함하지 않았다. [통합 확인](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6460651885).

- **필수 Linux-local 게이트 PASS(exit 0):** `bash tools/check-linux-local.sh`의 float guard/fixtures, workspace release build/tests, static-model import 예제, workspace all-targets Clippy, standalone/static-game-actions, input/editor 검사, 실제 multi-action 3 tests, Python 6 suites/63 tests. Standalone X11 acceptance는 `--no-run` 컴파일만 완료했다. 기존 release profile을 유지했다. [정확한 후보의 결과](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/309#issuecomment-6460288981).
- **추가 집중 검사 PASS:** scene 21, remote defense 9, registered Stormglass 24, native defense 10, Pick 5, input authoring 5, browser-options Python 4. 입력 CLI/GPU acceptance 2개는 ignored 그대로 미실행이며 browser-options는 browser E2E가 아니다. Feature-gated 0 tests를 기능 검증으로 계산하지 않는다.
- **한계:** advisory repo-wide rustfmt exit 1은 기존 formatting debt로 남는다. Hosted workflows 제거는 CI 성공이나 필수 검증 면제가 아니다. Windows/macOS/Android/browser E2E/hardware GPU와 source-hidden 전달은 이 Linux 결과로 인증하지 않는다. 보안·인증·보호 설정은 우회하지 않았다.

## 담당·남은 일

Su가 내부 서브에이전트의 구현·집중 검증·독립 리뷰와 파일 소유권을 조정한다. 하늘은 종료되어 새 배정이 없으며 땅은 기존 작업 검증·인계 후 종료만 수행한다. 외부 인수 전 최종 SHA/검사/미완료/파일·자원 반환을 확인한다. 위 PR #309 개발 통합과 필수 게이트는 완료됐으며 파일 충돌은 발견되지 않았다. **후속 변경(개발 미병합)**은 공통 scene admission을 재사용하는 browser 로컬 Scene loader다. 구현·집중 검사·독립 리뷰를 완료했으며 Rust 9/Node 42/Python 6 tests, 실제 WASM·offline 검사·lint가 통과했다. 이는 WASM/mock-DOM 증거다. 실제 Chromium은 페이지 로드 전 socket 단계에서 실행 실패했으므로 browser/offline file-picker QA는 미확인이다. 다음은 이 QA와 embedded exporter 연결이며 PR #309 통과를 후속 변경의 전체 게이트로 이전하지 않는다. 무개입 실제 모델 기준선은 미측정이다.

기존 후속 이슈 분모는 **22**로 고정하며 완료 분자는 이 문서에서 재집계하지 않는다. #94 게임 제작 범위는 별도이며 하위이슈·역할·예약을 중복 합산하지 않는다. 이슈 규모나 병합 파일 수는 엔진 완성도가 아니다.

## 보존할 증거

[#94 reflection 보정](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6356552702), [compile 예산 실패](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6356609143), [private fixture 계약](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6356618808)과 [이전 실패 기록](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/blob/f20097d6aeb2b4a110a7145b58ea07e0c7a0e8e6/docs/progress.md)을 보존한다. 새 실행의 raw logs·ZIP·JSON은 git 밖에 source tree·toolchain·명령·종료 상태와 함께 남긴다. 실패·중단을 지우거나 통과로 바꾸지 않는다.

이전 후보의 `orr_editor` rustc SIGKILL 9/Cargo 101은 원인 미확정(OOM 입증 없음)으로 보존한다. 세션 유실은 UNKNOWN이지 테스트 성공/실패가 아니다. 환경 교체 후 소스 복구·공식 dependency fetch 회복과 후속 성공은 원래 실패를 지우지 않는다. [복구 기록](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6450381753). PR #139/#141의 과거 독립 Windows 실패·미재현 한계도 현재 담당/통과로 바꾸지 않는다.
