# Orrery 현재 상태

갱신: 2026-11-19 · [설계 v2](design-v2.md) · [상위 추적 #2](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/2) · [게임 제작 #94](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94)

## 지금 되는 것

아래는 2026-11-11~12에 윈도우 11(NVIDIA RTX 4060)에서 직접 실행해 확인한 항목이다. 실행 방법은 [README](../README.md)에 있다.

- **빌드**: `cargo build-all` 한 줄로 에디터와 게임 실행 파일이 모두 만들어진다.
- **에디터**: 기본 물리 씬, `--game arena`, Room 프로젝트 열기를 화면 캡처로 확인했다.
- **Room 제작 흐름**: `orr_new_room`으로 프로젝트 생성 → `orr_editor --room-project`로 열기 → `room_escape --project`로 실행 → `orr_export_windows`로 내보내기 → 내보낸 폴더의 실행 파일만으로 실행.
- **Arena·Collect 제작 흐름 (2026-11-19, 윈도우)**: `orr_new_arena`로 프로젝트 생성(Arena, Collect, Collect UI, 타일맵, 사용자 스프라이트 템플릿을 실행해 확인했다. 사용자 타일셋 템플릿은 같은 코드 경로지만 실행하지 않았다) → `arena`·`collect_dodge --project --headless --ticks 120 --capture`로 실행 → `orr_export_arena`·`orr_export_collect`(내부는 `orr_export_windows`와 같은 윈도우 내보내기)로 내보내기 → 내보낸 폴더의 `bin\arena.exe`·`bin\collect_dodge.exe`로 실행. 원본과 내보낸 폴더의 120틱 체크섬과 캡처 PNG가 같다. 게임 진행(Collect 최고 기록, Room 클리어)은 `%LOCALAPPDATA%\orrery\games\...`에 저장된다. 터레인 패치 불러오기와 `orr_pkg export-source`, `orr_pkg create-sprite`도 윈도우에서 돈다.
- **에디터 흐름 신호 (2026-11-19, #559 P1-A 첫 조각)**: 선택한 개체(Hierarchy 강조, Inspector 머리말의 이름과 종류), `EDIT`·`PLAY` 표시, 저장 안 된 변경(`Unsaved changes`(경고 아이콘과 함께), 제목의 `*`), Inspector 안의 거부 메시지, 상태 줄의 다음 행동 문장, 내보내기 뒤 `Run the exported game: <경로>`를 기존 패널에 붙였다. 창 최소 크기는 1024x640이다. 한글 이름이 빈 네모로 나오던 문제는 시스템 글꼴을 뒤에 붙여 고쳤다. `egui_kittest`로 상태별 화면을 확인했다(`crates/orr_editor/tests/flow_clarity.rs`). 사람이 윈도우 창에서 직접 해 보는 확인과 100/150/200% DPI 실기 확인은 하지 않았다. 도킹·레이아웃 저장은 P2-B로 남아 있다.
- **에디터 도킹과 레이아웃 저장 (2026-11-19, #559 P2-B)**: Hierarchy·Viewport·Inspector·Timeline 패널을 한 창 안의 분할/탭 도킹(`egui_tiles` 0.17.1, MIT OR Apache-2.0)으로 옮겼다. 패널 이름줄을 끌어 분할·탭으로 옮기고, 경계를 끌어 크기를 바꾼다. Escape는 드래그만 취소하고 선택은 지우지 않는다. `Window` 메뉴에서 패널 표시/숨김, `Layout > Save as… / 프리셋 / Reset to default`, 키보드용 `Move panel`(가장자리 도킹·탭 묶기)을 쓴다. Viewport는 숨기거나 닫을 수 없고, 최소 크기는 100점이다. 레이아웃은 `%LOCALAPPDATA%\orrery\editor\layouts\`(다른 OS는 `~/Library/Application Support` 또는 `$XDG_CONFIG_HOME`)의 `last.json`과 `preset-*.json`에 JSON으로 저장되며(`crates/orr_editor/src/layout.rs`), 씬·Undo·체크섬과 무관하다. 파일은 64KiB, 노드 64개, 깊이 8, 프리셋 16개로 제한하고 임시 파일에 쓴 뒤 이름을 바꿔 저장하므로 쓰기에 실패해도 기존 파일이 남는다. 손상·미지원 버전·모르는 패널·Viewport가 없는 레이아웃·창에 맞지 않는 레이아웃은 기본 배치와 닫을 수 있는 안내 줄(`Layout …`)로 복구한다. 재시작 시 마지막 배치를 복원하며 저장·Play·포커스 이동은 하지 않는다. `--screenshot` 실행은 사용자 설정을 읽거나 쓰지 않는다. `egui_kittest`로 이동·분할·숨김·복구·초기화·프리셋 재시작 복원·복구 경로를 확인했다(`crates/orr_editor/tests/docking.rs`). 사람이 윈도우 창에서 직접 해 보는 확인과 DPI 100/150/200% 실기 확인은 하지 않았다. 알려진 남은 일: 별도 OS 창은 비목표, 기본 Agent 탭이 열릴 때 아래 패널이 커지던 동작은 사라졌다, `PanelState`로 Inspector/Hierarchy 폭을 읽고 쓰던 `--features full` 테스트 약 12개(mesh·blockout GPU 계열)는 도킹으로 옮기지 않았다.
- **에디터 디자인 토큰과 벡터 아이콘 (2026-11-19, #559 1단계)**: 색, 간격(2/4/6/8/12), 글자 크기(11/13/15), 모서리, 버튼 테두리, 포커스 고리를 `theme.rs` 토큰 한 벌로 모으고 상단 바, 메뉴, 도킹 탭, Hierarchy, Inspector, 상태 줄, 알림에 같은 값을 쓴다. 글자 면 위 대비 4.5:1, 컨트롤 테두리와 포커스 3:1을 `egui_kittest` 테스트(`tests/design_tokens.rs`)로 확인한다. 이미지 없이 `Painter`로 그린 아이콘 24개(`icons.rs`)가 Play/Pause/Step/Stop, Save/Undo/Redo, Eye, Lock, Folder, Warning/Error/Info, Layout과 개체 종류 표시를 대신한다. 사람이 윈도우 실제 창에서 하는 DPI 확인은 하지 않았다. 설명은 [editor-design.md](editor-design.md)다.
- **게임**: Stormglass(`coop_defense`)의 창 실행과 창 없는 600틱 실행, `arena`의 60fps 실행과 오디오 출력.
- **야외 장면 렌더러**: `forest_valley`가 숲과 계곡 장면을 1600x900 창에서 수직 동기화 60fps로 그린다. 동기화를 끄면 약 100fps다. 구성은 그림자 두 단계, HDR 다중 샘플 장면, 하늘과 안개, 물, 블룸, 빛줄기, 자동 노출, 톤 매핑이다. 코드는 `crates/orr_scenery`에 있고 `orr_rhi`만 쓴다. 관련 이슈는 [#101](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/101)이다.
- **에디터의 장면 창**: `Edit > Forest Valley scene…`에서 같은 장면을 에디터 안에 띄우고 설정 36개, 켜고 끄기 2개, 색 2개, 품질 2개를 고친다. 값은 다음 프레임에 반영된다. 프리셋을 JSON으로 저장하고 불러온다. 에디터 기능 이름은 `scenery`이고 `full`에 들어 있다. **열린 씬에 적용 (2026-11-19)**: 열린 씬이 `orr_gfx` 컴포넌트를 가진 게임(Yard3D, ViewYard3D)이면 창 위쪽의 `Edit the open scene`을 골라 같은 컨트롤로 그 씬을 고친다. 해의 `Light`와 `Transform3`, `Sky`, `Environment`, `PostProcess` 효과, 모든 `Water`의 필드가 일반 씬 편집(실행 취소, 저장)으로 바뀌고 뷰포트가 바로 보여 준다. 씬에 필드가 없는 컨트롤 11개(그림자 부드러움·가까운 범위, 거품, 식생 전부, 지면 전부)는 흐리게 꺼진다. [rendering-architecture.md](rendering-architecture.md) 1.21절.
- **씬 렌더링 컴포넌트**: 조명(`orr_gfx::Light`, directional·point·spot, 레이어 마스크, 그림자), 렌더 레이어, 환경광, 후처리를 엔티티 컴포넌트와 씬 싱글턴으로 설정한다. 씬 파일·인스펙터·`orr` 명령·실행 취소가 모두 같은 경로를 쓴다. 지금은 Yard3D에 적용했다. 구조와 다음 단계(터레인 컴포넌트, 셰이더 규약, 렌더 단계 확장, 후처리 효과 목록)는 [rendering-architecture.md](rendering-architecture.md)에 있다.
- **표면 셰이더**: 씬 파일 옆 `shaders/*.wgsl`에 `fn surface(in: SurfaceInput) -> Surface`를 쓰면 `orr_gfx::Material` 컴포넌트로 물체에 적용된다. 조명·그림자·레이어·톤 매핑은 엔진이 맡는다. 오류는 파일 줄 번호로 보이고, 파일을 저장하면 에디터가 다시 읽는다. 절차 도형, 정적 모델, 지형에 적용된다(스키닝 모델은 아직). 자세한 내용은 [rendering-architecture.md](rendering-architecture.md) 1.6절.
- **엔진 렌더 컴포넌트 3단계**: 에셋 참조, `Camera`, `PostProcess` 효과 목록(빛줄기, 자동 노출, 색 보정, 비네트, 필름 입자, 높이 안개), `Terrain`(높이맵 + 표면 셰이더), `Sky`, `Water`, `Scatter`, 렌더 패스 등록(`RenderPass`). Forest Valley를 이 컴포넌트만으로 된 씬(`scenes/forest_valley`)으로 다시 만들었다. 자세한 내용은 [rendering-architecture.md](rendering-architecture.md) 1.7절부터.
- **용 비행**: Forest Valley 씬에 동양식 용(`orr_games::Dragon`)이 있다. 에디터 Play에서 `Take control` 후 A/D, W/S, E로 난다. 카메라가 뒤따르고, 몸통은 렌더 패스(`orr_scenery::dragon::DragonPass`)가 경로를 따라 매 프레임 만든다. 자세한 내용은 [rendering-architecture.md](rendering-architecture.md) 1.15절.
- **재질 텍스처와 스키닝 모델**: 표면 셰이더가 `// @texture` 줄로 PNG 이미지를 최대 4장 선언하고 읽는다. 이미지는 밉 단계와 반복 샘플러로 걸러진다. `orr_gfx::Material`의 `texture_0`~`texture_3`이 경로를 가지고, 인스펙터 재질 칸에서 고른다. 같은 셰이더가 스키닝 모델에도 적용된다. Forest Valley 지형이 풀·바위 디테일 이미지를 쓴다. [rendering-architecture.md](rendering-architecture.md) 1.16절.
- **캐스케이드 그림자**: 해 그림자가 최대 4단계로 시점을 따라간다. 가까운 그림자는 선명하고 먼 그림자도 보인다. `orr_gfx::Light::shadow_cascades`로 정한다. [rendering-architecture.md](rendering-architecture.md) 1.17절.
- **입문 문서**: [getting-started.md](getting-started.md). 준비물, 코드로 첫 게임(`cargo run -p orr_sim --example hello`), 시뮬레이션 규칙, 에디터, Room 프로젝트 만들기·실행·내보내기, 에이전트 연결, 문제 해결. 절차는 윈도우에서 직접 따라가 확인했다. 그 과정에서 `type Command = ();`, `orr_sim::fn_systems`, 일반 경로로 출력하는 `orr_new_room`·`orr_export_windows`, 에디터 상태 막대의 `Unsaved: N`을 추가했다.
- **후처리 효과 추가**: `ssao`(주변광 차폐), `toon`(밝기 단계), `outline`(깊이 외곽선), `fxaa`, `upscale`(장면을 작게 그리고 FSR 1 방식 EASU 확대와 RCAS 선명화), `taa`(지터와 물체별 움직임 벡터 재투영, 벡터가 없는 곳은 깊이 재투영, 이웃 범위 제한. `upscale`과 함께면 대상 해상도로 시간 누적 업스케일), `ssr`(화면 공간 반사), `depth_of_field`, `motion_blur`(카메라와 물체 움직임), `chromatic_aberration`, `volumetric_fog`(해 그림자 캐스케이드를 따르는 빛줄기). 표면 셰이더용 `normal_map` 도우미(탄젠트 데이터 없이 화면 미분으로). `PostProcess` 효과 목록에서 고른다. [rendering-architecture.md](rendering-architecture.md) 1.9절.
- **물리 조인트**: 2D 핀·막대·밧줄·미끄럼·스프링, 3D 볼·막대·밧줄·힌지·미끄럼·스프링. 시연 씬은 `orr_editor --game yard3d --scene scenes/joints_demo.scene.yaml`(3D), `orr_editor --game physics --scene scenes/joints_2d_demo.scene.yaml`(2D). 결정론 고정소수점으로 접촉과 함께 푼다. 쓰는 게임만 `register_joints`로 켠다. [physics-joints.md](physics-joints.md).
- **공간 음향**: `Clip::at`으로 소리에 위치를 주면 거리에 따라 작아지고 방향에 따라 좌우로 나뉜다. 듣는 위치가 움직이면 재생 중인 소리도 따라간다. [audio.md](audio.md).
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

- 윈도우에서 아직 안 되는 것은 다음과 같다. (Stormglass의 이어하기 저장 `defense_run_save`는 윈도우에서도 된다. 저장은 `%LOCALAPPDATA%\orrery\games\stormglass\runs`에 쌓이고 `--run-directory`로 다른 폴더를 고른다. 폴더는 `orr_fsid`로 고정하며 링크·정션은 거부한다. 폴더 권한 목록은 읽지 않고 상속에 맡긴다. 창을 띄우고 마우스로 저장 단추를 누르는 확인은 아직 하지 않았고, 같은 App/Session을 두 프로세스로 저장·복원하는 테스트는 윈도우에서 통과했다.) 에디터 메뉴 `File > New Collect project…`와 Arena·Collect 내보내기 창은 리눅스에서만 나온다(같은 일은 `orr_new_arena`, `orr_export_arena`, `orr_export_collect` 명령으로 한다). 타일맵 프로젝트의 내보내기는 윈도우 내보내기가 거부한다. `orr_pkg`는 `export-source`와 `create-sprite`만 윈도우에서 돈다. 나머지 명령(`pack-coop-native`, `create-coop-native-scene`, `build-coop-recipe`, `copy-coop-web-assets`, `pack-coop-offline`, `create-coop-scene`, `create-coop-animation`, `compose-coop-animations`, `create-coop-audio-bank[-from-wav]`, `deployment`, `bootstrap-user-game`, `create-assembled`, `create-uv-model`, `create-retextured-model`, `create-animated-sequence`)는 리눅스 전용이다. 이유는 두 가지다. 첫째, 이 명령들의 폴더 읽기·쓰기가 `rustix`의 `openat`, `renameat2(NOREPLACE)`, `flock`으로 짜여 있어서 윈도우용 저장 계층을 따로 써야 한다. 둘째, `pack-coop-native`는 리눅스 x86-64 ELF 실행 파일, 실행 비트, 프로세스 그룹 종료를 전제로 한다. 윈도우용 실행 파일 묶음은 `orr_export_windows`가 맡는다.
- 윈도우 진행 저장은 `%LOCALAPPDATA%` 아래 폴더의 상속 권한에 기댄다. 리눅스 저장소가 하는 소유자·쓰기 권한 검사는 하지 않는다. 폴더 이음(junction)과 심볼릭 링크는 거부하고, 잠금은 잠금 파일의 공유 모드로 한다.
- 윈도우 터레인 패치 읽기는 파일 번호(inode)가 없어서 생성 시각을 파일 식별자 대신 쓴다. 일반적인 교체는 잡지만, 악의적으로 폴더를 바꿔치기하는 경쟁은 리눅스와 마찬가지로 막지 않는다.
- 윈도우에서 Collect 최고 기록 저장은 창 경로의 단위 테스트(`isolated_process_relaunch_uses_only_external_progress`)로 확인했다. 창을 띄워 사람이 직접 점수를 내 보는 확인은 하지 않았다.
- 에디터 메뉴(`File > New Room project…`, `File > Export Windows game…`)는 사람이 직접 눌러 보는 확인을 아직 하지 않았다. 같은 기능을 명령 도구로 실행해 확인했다.
- 리눅스, macOS, 안드로이드, 브라우저 빌드는 이번에 실행하지 않았다.
- 2026-11-14에 실패하던 `drag_to_viewport_latency`와 `physics3d_1000_tick`은 2026-11-19 main에서 통과한다(앞의 것은 기본 기능 구성에서만 확인).
- 폴더와 파일이 바뀌지 않았는지 확인하는 코드는 공용 크레이트 `orr_fsid`(윈도우는 볼륨 일련번호와 파일 인덱스, 유닉스는 장치·아이노드)로 옮기는 중이다. 열린 핸들로 신원을 읽는 `open_file`, `same_file`, `same_handle`, `file_id_at`을 추가했다. Room 캐릭터·카메라·조명, Collect UI, 연결 프리팹 패널에 더해 `json_output.rs`, 조명·프리팹·벽 스탬프 저장소, `png16`·`raw16` 입력과 `raw16` 출력, `orr_model` 저작 입력 읽기, 지형 패치, 패키지 제거, 메시·모델 입력 읽기도 옮겼다. 이 코드들은 경로로 다시 열어 메타데이터(생성 시각, 크기, 수정 시각)를 비교하던 방식을 버렸다.
- 아직 `orr_fsid`로 옮기지 않은 곳은 유닉스 전용 코드다. `orr_package`, `orr_sample`, `orr_remote`, `orr_navigation_view`의 `openat` 기반 흐름, `mesh_prop_delivery`, `tilemap_panel/pattern_asset.rs`, `orr_model`의 `planar_uv`·`animation_sequence`가 그렇다. 이 코드는 윈도우에서 닫힌 채 실패하도록 만들어져 있어, 옮기면 지원 플랫폼이 바뀐다. 리눅스 전용 모듈(`collect_project`, `arena_export`, `room_wall_stamp_store`의 리눅스 경로)은 옮겼지만 이 머신에서 컴파일하지 못했다. 리눅스에서 `cargo check`와 해당 테스트가 필요하다.
- TAA와 `motion_blur`의 물체별 움직임 벡터는 절차 도형, 정적 모델 인스턴스, 스키닝 모델에만 있다. 표면 셰이더 인스턴스, 렌더 패스(지형, 식생, 수면), 입자는 카메라 움직임만 재투영한다. 자세한 한계는 [rendering-architecture.md](rendering-architecture.md) 1.14절에 있다.

## 다음 작업

0. 뷰 프레임워크(`orr_app`)와 그 위의 튜토리얼([tutorial/README.md](tutorial/README.md), 5단계)을 [view-framework.md](view-framework.md)의 12장 순서로 만들었다. 남은 것은 12장의 "아직 정하지 않은 것"(웹과 모바일, 서버 직접 시뮬레이션 모드 등)이다.
1. [#104](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/104) 흐름을 에디터 화면에서 사람 손으로 끝까지 진행한다: 새 프로젝트 → 개체 이동과 값 수정 → 실행 취소 → 저장 → Play → 내보내기 → 내보낸 게임 실행.
2. 윈도우로 옮기고 남은 것을 옮긴다. 에디터의 Collect 프로젝트 메뉴와 Arena·Collect 내보내기 창(경로 검사가 `/` 기준이다), 그리고 `orr_pkg`의 나머지 명령이다. Arena·Collect 생성·내보내기·진행 저장·터레인 패치 불러오기는 2026-11-19에 옮겼다.
3. 설계 v2의 고정 과제군(배달, 스위치 퍼즐, 액션)으로 무개입 완주율을 처음 측정한다.
4. 장면 창의 설정이 닿지 않는 씬을 줄인다. 열린 씬 적용은 Yard3D, ViewYard3D, 터레인 문서(`TerrainYard3D`) 씬에서 된다. 내비게이션, Room, 메시 게임은 `orr_gfx`를 등록하지 않아 그 모드가 꺼져 있다. 이 게임들에 `orr_gfx::register`와 `init`을 넣으려면 각 게임의 입장 검사와 뷰 스트림 스키마를 함께 손봐야 한다. 터레인 문서는 지형을 `orr_gfx::Terrain`이 아니라 터레인 세션 모델로 그린다. 씬에 대응 필드가 없는 설정(식생 색, 지면 요철, 그림자 부드러움)을 컴포넌트로 만드는 일도 남았다. 뷰포트 전체를 `orr_scenery::Renderer`로 바꾸는 안은 하지 않았다([rendering-architecture.md](rendering-architecture.md) 1.21절).

## 작업 규칙

`main`만 오래 유지한다. 작업마다 짧은 브랜치 하나를 만들고, 빌드와 테스트가 통과하면 바로 `main`에 합친 뒤 브랜치를 지운다. 자세한 규칙은 [AGENTS.md](../AGENTS.md)의 Delivery rules에 있다.
