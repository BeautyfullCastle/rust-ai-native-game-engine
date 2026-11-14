# Orrery 현재 상태

갱신: 2026-11-14 · [설계 v2](design-v2.md) · [상위 추적 #2](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/2) · [게임 제작 #94](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94)

## 지금 되는 것

아래는 2026-11-11~12에 윈도우 11(NVIDIA RTX 4060)에서 직접 실행해 확인한 항목이다. 실행 방법은 [README](../README.md)에 있다.

- **빌드**: `cargo build-all` 한 줄로 에디터와 게임 실행 파일이 모두 만들어진다.
- **에디터**: 기본 물리 씬, `--game arena`, Room 프로젝트 열기를 화면 캡처로 확인했다.
- **Room 제작 흐름**: `orr_new_room`으로 프로젝트 생성 → `orr_editor --room-project`로 열기 → `room_escape --project`로 실행 → `orr_export_windows`로 내보내기 → 내보낸 폴더의 실행 파일만으로 실행.
- **게임**: Stormglass(`coop_defense`)의 창 실행과 창 없는 600틱 실행, `arena`의 60fps 실행과 오디오 출력.
- **야외 장면 렌더러**: `forest_valley`가 숲과 계곡 장면을 1600x900 창에서 수직 동기화 60fps로 그린다. 동기화를 끄면 약 100fps다. 구성은 그림자 두 단계, HDR 다중 샘플 장면, 하늘과 안개, 물, 블룸, 빛줄기, 자동 노출, 톤 매핑이다. 코드는 `crates/orr_scenery`에 있고 `orr_rhi`만 쓴다. 관련 이슈는 [#101](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/101)이다.
- **에디터의 장면 창**: `Edit > Forest Valley scene…`에서 같은 장면을 에디터 안에 띄우고 설정 36개, 켜고 끄기 2개, 색 2개, 품질 2개를 고친다. 값은 다음 프레임에 반영된다. 프리셋을 JSON으로 저장하고 불러온다. 에디터 기능 이름은 `scenery`이고 `full`에 들어 있다.
- **씬 렌더링 컴포넌트**: 조명(`orr_gfx::Light`, directional·point·spot, 레이어 마스크, 그림자), 렌더 레이어, 환경광, 후처리를 엔티티 컴포넌트와 씬 싱글턴으로 설정한다. 씬 파일·인스펙터·`orr` 명령·실행 취소가 모두 같은 경로를 쓴다. 지금은 Yard3D에 적용했다. 구조와 다음 단계(터레인 컴포넌트, 셰이더 규약, 렌더 단계 확장, 후처리 효과 목록)는 [rendering-architecture.md](rendering-architecture.md)에 있다.
- **표면 셰이더**: 씬 파일 옆 `shaders/*.wgsl`에 `fn surface(in: SurfaceInput) -> Surface`를 쓰면 `orr_gfx::Material` 컴포넌트로 물체에 적용된다. 조명·그림자·레이어·톤 매핑은 엔진이 맡는다. 오류는 파일 줄 번호로 보이고, 파일을 저장하면 에디터가 다시 읽는다. 절차 도형, 정적 모델, 지형에 적용된다(스키닝 모델은 아직). 자세한 내용은 [rendering-architecture.md](rendering-architecture.md) 1.6절.
- **엔진 렌더 컴포넌트 3단계**: 에셋 참조, `Camera`, `PostProcess` 효과 목록(빛줄기, 자동 노출, 색 보정, 비네트, 필름 입자, 높이 안개), `Terrain`(높이맵 + 표면 셰이더), `Sky`, `Water`, `Scatter`, 렌더 패스 등록(`RenderPass`). Forest Valley를 이 컴포넌트만으로 된 씬(`scenes/forest_valley`)으로 다시 만들었다. 자세한 내용은 [rendering-architecture.md](rendering-architecture.md) 1.7절부터.
- **용 비행**: Forest Valley 씬에 동양식 용(`orr_games::Dragon`)이 있다. 에디터 Play에서 `Take control` 후 A/D, W/S, E로 난다. 카메라가 뒤따르고, 몸통은 렌더 패스(`orr_scenery::dragon::DragonPass`)가 경로를 따라 매 프레임 만든다. 자세한 내용은 [rendering-architecture.md](rendering-architecture.md) 1.15절.
- **재질 텍스처와 스키닝 모델**: 표면 셰이더가 `// @texture` 줄로 PNG 이미지를 최대 4장 선언하고 읽는다. 이미지는 밉 단계와 반복 샘플러로 걸러진다. `orr_gfx::Material`의 `texture_0`~`texture_3`이 경로를 가지고, 인스펙터 재질 칸에서 고른다. 같은 셰이더가 스키닝 모델에도 적용된다. Forest Valley 지형이 풀·바위 디테일 이미지를 쓴다. [rendering-architecture.md](rendering-architecture.md) 1.16절.
- **캐스케이드 그림자**: 해 그림자가 최대 4단계로 시점을 따라간다. 가까운 그림자는 선명하고 먼 그림자도 보인다. `orr_gfx::Light::shadow_cascades`로 정한다. [rendering-architecture.md](rendering-architecture.md) 1.17절.
- **입문 문서**: [getting-started.md](getting-started.md). 준비물, 코드로 첫 게임(`cargo run -p orr_sim --example hello`), 시뮬레이션 규칙, 에디터, Room 프로젝트 만들기·실행·내보내기, 에이전트 연결, 문제 해결. 절차는 윈도우에서 직접 따라가 확인했다. 그 과정에서 `type Command = ();`, `orr_sim::fn_systems`, 일반 경로로 출력하는 `orr_new_room`·`orr_export_windows`, 에디터 상태 막대의 `Unsaved: N`을 추가했다.
- **후처리 효과 추가**: `ssao`(주변광 차폐), `toon`(밝기 단계), `outline`(깊이 외곽선), `fxaa`, `upscale`(장면을 작게 그리고 FSR 1 방식 EASU 확대와 RCAS 선명화), `taa`(지터와 깊이 재투영, 이웃 범위 제한. `upscale`과 함께면 대상 해상도로 시간 누적 업스케일), `ssr`(화면 공간 반사), `depth_of_field`, `motion_blur`(카메라), `chromatic_aberration`. 표면 셰이더용 `normal_map` 도우미(탄젠트 데이터 없이 화면 미분으로). `PostProcess` 효과 목록에서 고른다. [rendering-architecture.md](rendering-architecture.md) 1.9절.
- **스포트라이트 그림자**: `Light::cast_shadows`를 켠 스포트라이트 최대 4개가 그림자를 드리운다. [rendering-architecture.md](rendering-architecture.md) 1.18절.
- **애니메이션 블렌딩**: 본 마스크, 마스크 블렌드, 가산 블렌드, 1D·2D 블렌드 스페이스, 룩앳, 상태 머신(파라미터, 조건부 전환, 크로스페이드, exit time, 클립 이벤트, 루트 모션), 레이어를 쌓는 `Animator`, 투 본 IK. 코드 API이며 에디터 편집 화면은 아직 없다. [animation.md](animation.md).
- **에디터 `--camera-view`**: 씬 카메라 시점으로 시작한다. `--screenshot`과 함께 쓰면 씬 카메라 화면을 저장한다.
- **테스트(기본 구성)**: `cargo test --workspace --release` 2779개 통과, 실패 0개, 무시 26개.
- **테스트(전체 기능 구성)**: `--features orr_editor/full,orr_sample/full`로 에디터와 샘플 게임을 전체 실행했을 때 3376개 통과, 19개 실패였다. 그 뒤 17건의 원인을 고쳐 해당 테스트를 다시 실행해 통과를 확인했다. 남은 실패는 2건이다.

## 2026-11-11~12 정리 내용

- 흩어져 있던 개발 줄기를 `main`에 합쳤다. `main`은 그전까지 실제 작업보다 약 720커밋 뒤처져 있었다.
- 기능별로 쌓인 GitHub Actions 워크플로 169개를 지웠다.
- 에디터(`orr_editor`)와 샘플 게임(`orr_sample`)에 모든 기능을 한 번에 켜는 `full` 기능을 추가했다. 기본값은 그대로 최소 구성이다. 모바일과 웹 빌드가 `orr_sample`의 최소 구성에 의존하기 때문이다.
- 오래된 브랜치에만 있던 작업 8건을 합쳤다: glTF CUBICSPLINE 애니메이션, P2P 메시 정지·이탈, 원뿔 모델 제작, 스프라이트 격자·핑퐁, 설치 패키지 소스 내보내기, 애니메이션 클립 역재생, 지형 카메라 붐, 지형 키보드 코스.
- 윈도우에서만 드러나던 결함을 고쳤다.
  - `\\?\C:` 형태의 경로에서 내비게이션·지형·조명 프로브 씬을 열지 못하던 문제
  - 폴더 핸들을 열지 못해 스프라이트 클립, RAW16 지형, Room 조명, 연결 프리팹 저장이 실패하던 문제
  - 메시 애니메이션 패키지 가져오기에서 폴더 이동이 거부되던 문제
  - 패키지 의존성 미리보기가 경로 표기 차이로 버려지던 문제
  - 모델 바인딩이 `/remote/scene.yaml` 같은 루트 경로를 상대 경로로 받아들이던 문제

## 알려진 문제

추적 이슈: [#563](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/563)

- Arena·Collect 프로젝트의 새로 만들기와 내보내기, 게임 진행 저장, 지형 패치 불러오기, `orr_pkg`의 번들·웹 명령은 리눅스 전용이다.
- 에디터 메뉴(`File > New Room project…`, `File > Export Windows game…`)는 사람이 직접 눌러 보는 확인을 아직 하지 않았다. 같은 기능을 명령 도구로 실행해 확인했다.
- 리눅스, macOS, 안드로이드, 브라우저 빌드는 이번에 실행하지 않았다.
- 2026-11-14 기준 main에서 원래부터 실패하는 테스트 2건: `crates/orr_editor/tests/measure.rs`의 `drag_to_viewport_latency`(두 번째 드래그 이동이 화면에 반영되지 않음), `crates/orr_wasm_bench/tests/checksums.rs`의 `physics3d_1000_tick` 골든 체크섬 불일치.
- TAA는 카메라 움직임만 재투영한다. 물체별 움직임 벡터가 없어서 움직이는 물체는 이웃 범위 제한에 의존한다.

## 다음 작업

1. [#104](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/104) 흐름을 에디터 화면에서 사람 손으로 끝까지 진행한다: 새 프로젝트 → 개체 이동과 값 수정 → 실행 취소 → 저장 → Play → 내보내기 → 내보낸 게임 실행.
2. Arena 프로젝트 생성과 내보내기를 윈도우로 옮긴다.
3. 설계 v2의 고정 과제군(배달, 스위치 퍼즐, 액션)으로 무개입 완주율을 처음 측정한다.
4. `orr_scenery` 렌더러로 에디터의 지형·모델 씬을 그린다. 지금은 생성된 Forest Valley 장면만 그린다. 에디터에서 만든 씬(Room, Yard3D, 지형 문서)은 여전히 `orr_render`로 그리고, 장면 창의 설정은 그 씬에 적용되지 않는다.

## 작업 규칙

`main`만 오래 유지한다. 작업마다 짧은 브랜치 하나를 만들고, 빌드와 테스트가 통과하면 바로 `main`에 합친 뒤 브랜치를 지운다. 자세한 규칙은 [AGENTS.md](../AGENTS.md)의 Delivery rules에 있다.
