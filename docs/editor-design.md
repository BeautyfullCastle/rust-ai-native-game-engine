# 에디터 디자인 토큰과 벡터 아이콘

에디터 셸(상단 바, 메뉴, 도킹 탭, Hierarchy, Inspector, 상태 줄, 알림)의 색, 간격, 글자 크기, 아이콘은 두 파일에서만 온다. 패널 코드는 자기 값을 쓰지 않고 이 값을 가져다 쓴다. (#559 1단계)

- `crates/orr_editor/src/theme.rs`: 색, 간격, 글자 크기, 모서리, 공통 틀
- `crates/orr_editor/src/icons.rs`: 벡터 아이콘 24개와 `IconButton`

## 토큰

| 묶음 | 토큰 | 값 / 쓰임 |
| --- | --- | --- |
| 면 | `INPUT` `BG` `BAR` `PANEL` `CARD` `CARD_HEADER` | 깊은 곳(입력칸)에서 높은 곳 순서. `BAR`는 상단 바, 상태 줄, 패널 제목 띠 |
| 면 | `BUTTON` `BUTTON_HOVER` `BUTTON_ACTIVE` | 버튼 기본, 올림, 누름 |
| 글자 | `TEXT` `TEXT_STRONG` `TEXT_WEAK` | 본문, 강조, 보조. 모든 글자 면 위에서 4.5:1 이상 |
| 강조 | `ACCENT` | 선택, 주요 동작(Play), 패널 제목 표시 |
| 강조 | `FOCUS` | 키보드 포커스 고리 (`ACCENT`와 같은 값) |
| 강조 | `ON_ACCENT` | `ACCENT`나 `PLAY` 칸 위의 글자와 아이콘 |
| 상태 | `PLAY` `WARN` `ERROR` `INFO` | 재생 중, 주의(저장 안 됨), 거부/오류, 정보 |
| 선 | `LINE` | 패널 사이, 카드 테두리, 구분선 |
| 선 | `CONTROL_EDGE` | 버튼과 입력칸 테두리. 패널 위에서 3:1 이상 |
| 종류 | `kind::*` | 개체 종류 아이콘 색 (Dragon, Light, Camera, Terrain, Water, Scatter) |
| 간격 | `space::XS SM MD LG XL` | 2, 4, 6, 8, 12 pt |
| 글자 크기 | `text::CAPTION BODY TITLE` | 11, 13, 15 pt. 서로 2 pt 이상 떨어져 있다 |
| 모서리 | `RADIUS` `WINDOW_RADIUS` | 2, 4 |
| 높이 | `CONTROL_HEIGHT` | 20 pt (컨트롤과 목록 줄) |

상태 구분: hover는 `BUTTON_HOVER` 칸과 `ACCENT` 테두리, 선택은 `ACCENT_FILL` 칸과 `TEXT_STRONG` 글자, 비활성은 egui의 흐림과 이유를 적은 툴팁, 포커스는 `FOCUS` 2 pt 고리다. 색만으로 상태를 구분하지 않는다. 선택 줄은 칸과 테두리가 함께 바뀌고, 오류 알림은 아이콘과 문장이 함께 나온다.

## 아이콘

16x16 격자에 선 굵기 1.5로 egui `Painter`가 그린다. 이미지 파일, SVG, 아이콘 글꼴은 없다. 색은 호출하는 쪽이 정하므로 글자 색과 같은 상태를 따른다.

| 아이콘 | 쓰는 곳 |
| --- | --- |
| Play, Pause, Step, Stop | 상단 바 재생 묶음 (글자와 함께), 알림 |
| Rewind, Play, Pause, Step | Timeline 버튼 (아이콘만, 툴팁이 이름) |
| Save, Undo, Redo, Folder | File, Edit 메뉴 줄 |
| Eye, EyeOff | Window 메뉴의 패널 표시 여부 (줄 오른쪽 끝) |
| Layout | Focus viewport 버튼, 저장한 레이아웃 줄 |
| Lock | 재생 기록 보기(읽기 전용) 표시 |
| Warning | 저장 안 된 변경, 레이아웃 알림 |
| Error | 거부된 편집, 호스트 중단, 상태 줄 오류 |
| Info | 상태 줄 메시지, 마지막 에이전트 동작 |
| Entity, Body, Light, Camera, Terrain, Water, Scatter, Dragon | Hierarchy 줄과 Inspector 머리말 |

아이콘 단추(`IconButton`)는 항상 이름을 가진다. 글자를 숨긴 단추는 같은 글자를 툴팁과 접근성 이름으로 쓴다. 장식용 아이콘은 같은 뜻의 글자 옆에만 놓는다.

## 새 값을 더할 때

1. 색, 간격, 크기는 `theme.rs`에 토큰으로 더하고 `crates/orr_editor/tests/design_tokens.rs`의 대비 목록에 넣는다.
2. 아이콘은 `Icon`에 항목과 `shapes`의 그림을 더한다. 점은 격자 1.5~14.5 안에 둔다. 테스트가 사각형 밖으로 나가는지 확인한다.
3. 메뉴 줄에서 글자의 왼쪽 시작선을 같게 둔다. 아이콘이 없는 줄은 `IconButton::plain`, 오른쪽 끝에 놓을 아이콘은 `icon_trailing`.

## 남은 일과 한계

- 뷰포트 위 그림 글자(오류, 경고, 힌트)와 패널별 입력 위젯(기능 패널 파일들)은 이 단계에서 `theme` 토큰으로 옮기지 않았다. `Color32::RED`처럼 직접 쓴 색은 `app.rs`에서만 토큰으로 바꿨다.
- Inspector 숫자 칸의 드래그 손잡이(`↔`, `inspector.rs`)는 글자 글리프로 남았다. `tests/ui.rs`가 이 글자로 손잡이를 찾으므로 아이콘으로 바꾸려면 테스트의 찾는 이름도 함께 바꿔야 한다.
- 밝은 테마는 만들지 않았다. 테마 전환도 없다.
- 확인은 `egui_kittest` 렌더와 CPU 측정이다. 윈도우 실제 창에서 사람이 하는 100/150/200% DPI 확인은 하지 않았다.
