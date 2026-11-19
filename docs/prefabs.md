# 프리팹 (일반 게임용, #105 일부)

에디터는 등록된 모든 게임(PhysGame, Arena, Yard3D 등)의 개체를 프리팹 파일로 저장하고, 씬에 인스턴스로 놓고, 값을 덮어쓰고, 프리팹이 바뀌면 인스턴스를 갱신한다. CollectDodge 전용 프로필(`collect-actors-v1`, [linked-collect-prefabs.md](linked-collect-prefabs.md))과는 별개이며 서로 섞이지 않는다. 모델 다시 가져오기(reimport)는 이 조각에 없다.

## 쓰는 법 (에디터)

Inspector 위쪽의 `Prefabs` 패널에서 한다. 씬 파일이 열려 있고, 로컬 호스트이고, Edit 모드일 때만 보인다.

| 하는 일 | 방법 |
| --- | --- |
| 선택에서 프리팹 만들기 | 개체를 고르고(2D는 Ctrl+클릭으로 여러 개) 파일 이름을 적고 `Create prefab from selection`. 씬 파일 옆에 `<이름>.prefab.yaml`이 생긴다. 이미 있는 파일은 덮어쓰지 않는다. `Link the selection to the new prefab`을 켜 두면 선택한 개체가 그 프리팹의 인스턴스가 된다 |
| 인스턴스 놓기 | 목록에서 파일을 고르고 `Instantiate prefab`. 새 개체 ID가 붙고, 새 인스턴스가 선택된다. Undo 한 번에 통째로 사라진다 |
| 덮어쓴 필드 보기 | 인스턴스의 개체를 선택하면 프리팹과 다른 필드 옆에 주황 점과 `Revert` 버튼이 보인다. 구조체·배열 머리말에는 점만 붙는다 |
| 필드 되돌리기 | 필드의 `Revert` (Undo 한 번) |
| 전부 되돌리기 | `Revert all overrides` (Undo 한 번). 지운 멤버 개체, 추가·삭제한 컴포넌트, 바꾼 이름도 되돌린다 |
| 프리팹에 반영 | `Apply to prefab`. 이 인스턴스의 덮어쓰기를 프리팹 파일에 쓰고 그 프리팹의 모든 인스턴스를 갱신한다. 이 인스턴스의 덮어쓰기는 사라진다 |
| 파일 변경 따라가기 | 파일이 디스크에서 바뀌면(약 0.4초 안에 감지) 그 프리팹의 인스턴스를 자동으로 갱신한다. `Update from prefab file`은 같은 일을 직접 시킨다 |
| 연결 끊기 | `Detach instance`. 개체와 값은 그대로 남는다 |

갱신 결과(몇 개 갱신, 몇 개체 추가·삭제, 충돌)는 패널 아래와 상태 줄에 비모달로 나온다. 충돌은 노란 줄로 쌓인다. 창을 띄우지 않는다.

## 데이터 형식

프리팹 파일은 개체만 담은 `orr.scene/1` 조각이다([scene-fragments.md](scene-fragments.md)). 개체 ID는 파일 안에서만 쓰는 `e_00000001`부터 차례로 붙은 지역 ID이고, 개체 사이 참조(`Joint.a` 같은 `EntityGuid`)는 이 지역 ID를 가리킨다. 선택 밖의 개체를 가리키는 개체가 있으면 **만들기를 거절한다**(참조를 비우지 않는다). 메시지는 `cannot create a prefab: ... reference 'e_…' is outside the fragment selection. Select the referenced entity too, or clear the reference first`이다.

씬 파일에는 인스턴스가 평탄화된 일반 개체로 들어 있고, 링크는 `orr.scene/2`의 `prefabs:` 아래에 `profile: generic-v1`로 붙는다. 링크가 없으면 파일은 그대로 `orr.scene/1`이다.

```yaml
schema: orr.scene/2
entities:
  e_0000000a: { name: motor_hub, ... }     # 인스턴스 개체: 보통 개체와 같다
prefabs:
  e_0000000a:                              # 키: 인스턴스 개체 중 하나
    profile: generic-v1
    source: "motor.prefab.yaml"            # 프리팹 파일 이름(라벨일 뿐, 코어는 파일을 열지 않는다)
    digest: "<baseline의 SHA-256>"
    baseline: "<마지막으로 맞춘 프리팹 조각 텍스트(따옴표 문자열)>"
    guids:                                 # 프리팹 지역 ID -> 씬 개체 ID
      e_00000001: e_0000000a
```

### 덮어쓰기는 저장하지 않는다

덮어쓴 값은 따로 기록하지 않고 **계산한다**. 인스턴스 개체의 실제 값과 `baseline`(참조는 씬 ID로 바꿔서)을 비교해 다른 잎(leaf) 값이 덮어쓰기다. 구조체, 같은 종류의 태그 값, 길이가 같은 배열은 안으로 들어가 비교하고, 그 밖의 값(`Vec2`, 숫자, 열거형, 길이가 다른 배열 등)은 값 하나로 본다. 경로는 `pos`, `shape.radius`, `verts[2]`처럼 `world.patch`가 받는 경로와 같다. 덮어쓰기로 세는 것은 필드 값, 이름, 추가한 컴포넌트, 지운 컴포넌트, 지운 멤버 개체다.

그래서 Inspector, 에이전트(`world.patch`), 기즈모, Undo/Redo 어느 경로로 바꿔도 덮어쓰기 표시가 항상 맞고, 따로 맞출 장부가 없다. 값을 프리팹 값과 같게 만들면 덮어쓰기가 아니다. (프리팹 값과 같은 값으로 "명시적으로 고정"하는 개념은 없다.)

### 프리팹이 바뀔 때 (3-way 병합)

`baseline`(이전 프리팹), 새 프리팹, 인스턴스 현재 값을 비교한다. 참조는 모두 씬 ID로 바꾼 뒤 한다.

- 인스턴스 값이 baseline과 같으면 새 프리팹 값을 따른다.
- 인스턴스 값이 다르고 프리팹은 그대로면 인스턴스 값을 지킨다.
- 둘 다 바뀌어 서로 다르면 **인스턴스 값을 지키고 충돌로 보고한다**.
- 프리팹에 새 컴포넌트·개체가 생기면 인스턴스에 추가한다. 프리팹에서 컴포넌트가 사라지면 덮어쓰지 않은 인스턴스에서는 지우고, 덮어쓴 곳은 지키고 충돌로 보고한다. 프리팹에서 개체가 사라지면 덮어쓰지 않았고 다른 곳이 참조하지 않을 때만 지운다. 아니면 일반 개체로 남기고(링크에서 빠진다) 보고한다.
- 인스턴스에서 지운 멤버는 프리팹이 바뀌어도 지워진 채로 둔다. 인스턴스에서 따로 추가한 컴포넌트는 지킨다.
- 갱신은 같은 프리팹 라벨의 모든 인스턴스를 한 번의 Undo 항목으로 처리한다. 이미 같은 내용이면 기록을 남기지 않는다.

`Apply to prefab`은 baseline에 이 인스턴스의 덮어쓰기를 반영한 새 텍스트를 만들어(덮어쓴 값의 참조를 지역 ID로 되돌린다) 파일에 쓰고, 이어서 위 갱신을 한다. 덮어쓴 값이 인스턴스 밖 개체를 가리키면 거절한다. 파일 쓰기는 Undo 대상이 아니다. Undo는 씬(모든 인스턴스와 링크)만 되돌리며, 파일은 새 내용으로 남는다. 필요하면 `Update from prefab file`로 다시 맞춘다. 마지막으로 읽은 뒤 디스크에서 파일이 바뀌었으면 `Apply`는 거절하고 먼저 갱신하라고 알려 준다.

## 중첩 (정책: 한 단계도 허용하지 않고 거절)

프리팹 안에 다른 프리팹 인스턴스를 넣는 것은 지원하지 않는다.

- 인스턴스 멤버가 들어 있는 선택으로 프리팹을 만들면 `... nested prefabs are not supported. Detach that instance first`로 거절한다.
- 링크(`prefabs:`)가 든 파일을 프리팹으로 쓰면 `a prefab file cannot contain prefab instances (nested prefabs are not supported); detach the inner instance first`로 거절한다.
- 멤버를 고른 상태에서는 `Create prefab from selection` 버튼 대신 인스턴스 동작이 나온다.

## 호스트 방법 (ERP)

기능은 `orr_remote/linked-prefabs`(에디터에서는 `scene-prefabs`) 아래에 있다. 모든 변경 방법은 `scene_edit` 권한, Edit 모드, 닫힌 트랜잭션, 정확한 `expected_revision`이 필요하고 모르는 파라미터는 거절한다. 읽기 방법은 `read` 권한이면 된다. 파일은 읽거나 쓰지 않는다.

| 방법 | 하는 일 |
| --- | --- |
| `prefab_instance.list` / `.entity` | 인스턴스, 멤버, 덮어쓰기 목록 / 한 개체의 소속과 덮어쓰기 |
| `prefab_instance.create` | 선택 → 프리팹 텍스트와 `지역 ID -> 개체 ID` (씬 변경 없음) |
| `prefab_instance.link` | 기존 개체를 프리팹 텍스트의 인스턴스로 연결 |
| `prefab_instance.instantiate` | 새 인스턴스 |
| `prefab_instance.revert` | 필드/컴포넌트(`component`,`path`) 또는 인스턴스 전체(`entity`만) 되돌리기 |
| `prefab_instance.apply_text` | 덮어쓰기를 반영한 프리팹 텍스트 계산(변경 없음) |
| `prefab_instance.update` | 같은 라벨의 모든 인스턴스를 새 텍스트로 갱신, 보고서 반환 |
| `prefab_instance.detach` | 연결 끊기 |

코어 API는 `orr_edit`의 `EditorDoc::{create_prefab_source, link_prefab_instance, instantiate_prefab, prefab_overrides, prefab_instances, revert_prefab_field, revert_prefab_instance, prefab_text_with_overrides, update_prefab_instances, detach_prefab_instance}`(`crates/orr_edit/src/prefab.rs`)이다.

## 한계 (지금 조각에서 하지 않은 것)

- **모델 다시 가져오기**: 하지 않았다. 이미지 한 종류의 명시적 reimport는 [#146](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/pull/146)에 별도로 있다.
- **중첩·변형(variant) 프리팹**: 위 정책대로 거절한다.
- 3D 뷰포트는 선택이 한 개체뿐이라 화면에서는 한 개체짜리 프리팹만 만든다. 여러 개체 프리팹은 `prefab_instance.create`로 만든다.
- 인스턴스를 통째로 지우는 명령은 없다. 멤버를 지우면 덮어쓰기(지운 멤버)로 세고, 멤버가 모두 사라진 인스턴스의 링크는 다음 `update`(프리팹 갱신) 때 정리된다.
- 인스턴스에 새로 만든 개체를 인스턴스에 넣는 기능, 프리팹 변형, 에디터 안의 프리팹 파일 편집기는 없다.
- `world.schema`의 JSON Schema는 `generic-v1` 링크를 설명하지 않는다(로더는 검증한다). CollectDodge 씬은 자기 프로필만 쓴다. 한 씬에 두 프로필을 섞을 수 없다.
- 크기 한도: 프리팹 파일 1MiB·개체 256개, 인스턴스 1024개, 씬 파일 4MiB, 파일에 박힌 baseline 합계 2MiB. 같은 프리팹의 인스턴스마다 baseline 텍스트가 들어간다(중복 제거 없음). Collect 프로필의 64KiB 한도는 그대로다.
- 프리팹 파일 읽기·쓰기는 씬 폴더 안의 납작한 `*.prefab.yaml` 이름만, 링크가 아닌 일반 파일만 다루는 확인형 로컬 I/O이다. 악의적인 파일 시스템 경쟁을 막는 샌드박스가 아니다.
- 윈도우 11에서만 실행했다. 화면 확인은 `egui_kittest`(헤드리스)와 소프트웨어 GPU 캡처이며, 사람이 창에서 직접 해 보는 확인은 하지 않았다.

## 확인

```sh
cargo test -j 8 -p orr_reflect --features linked-prefabs --test generic_linked_scene
cargo test -j 8 -p orr_edit --features linked-prefabs --test prefab
cargo test -j 8 -p orr_remote --features linked-prefabs --test prefab_instance
cargo test -j 8 -p orr_editor --features scene-prefabs --test prefab_ui
# 인스펙터 스크린샷 (GPU 필요)
ORR_PREFAB_SHOTS=<폴더> ORR_REQUIRE_GPU=1 cargo test -p orr_editor --features scene-prefabs --test prefab_ui screenshot_of_the_inspector_with_override_marks -- --ignored
```

에디터 테스트는 PhysGame(2D 관절 데모: 허브·프로펠러·핀 3개체와 내부 참조)과 Yard3D에서 만들기, 거절, 인스턴스, 표시와 되돌리기, 적용, 디스크 변경 갱신(충돌 보고 포함), Undo/Redo, 저장·다시 열기, 연결 끊기를 확인한다.
