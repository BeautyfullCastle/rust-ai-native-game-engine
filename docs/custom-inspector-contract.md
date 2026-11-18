# Static custom Inspectors (experimental)

Issue #562, opt-in CPU-tested consumer slice. Enable `orr_editor/custom-inspector`
and install a `Registry` with `EditorApp::with_custom_inspectors` before the first
frame. Disabled by default; enabling it installs no panels.
Each app owns its registry. There is no game-specific editor dispatch or global
registry. This slice covers reflected entity components and authored singletons.
Specialized 3D panels that bypass the generic component Inspector are excluded.

## Existing Arena example

From the repository root:

```sh
cargo run --locked -p orr_editor --features custom-inspector --example arena_custom_inspector -- crates/orr_editor/examples/arena_custom_inspector/projectile.scene.yaml
```

Select `hero` in the hierarchy. Its custom PlayerTag panel assigns slots 0 through 7.
Select `projectile` for its Bullet lifetime and velocity presets. Direction presets
set a whole exact Vec2 at 2 world units per simulation tick; stopped sets zero.
Repeated selection of the current preset adds no Undo step. Use generic fields
for arbitrary exact velocity values. Save As your own scene, reopen it with the
same command with your saved path replacing the fixture path, then Play/Step. The supplied scene
is also the test input, not a new runtime or player firing-speed setting.
`examples/arena_custom_inspector/panels.rs` owns both registrations and draw
functions. Adding the second component required no editor/core/game-enum changes.
Arena has no health or inventory, so these examples do not complete #562's
health/inventory consumer requirement. No new game runtime was created.

Choose **Use generic fields** to edit that component using the existing widgets.
This presentation-only choice is kept in egui memory, never in the saved scene.
Custom callbacks and the registered component's edits are disabled outside Edit,
during proposal previews, under disabled parent UI, and when the editor cannot
mutate. Unregistered types retain the existing generic Inspector policy.

## Compatibility and edit authority

Use `SchemaKey::for_type` on the compiled game's reflected registry. It captures
the exact deterministic type JSON Schema and reflected type name. The app derives
the current key from its verified registry; no invented numeric fingerprint or
hash can make a changed schema match. Different text fails closed to generic
fallback, even for semantically equivalent schema text. Type or field renames,
ranges or schema documentation changes may require a matching registration.

An extension declares a nonempty stable ID, API version 1 and signed priority.
Higher priority wins independently of registration order. Duplicate IDs and equal
priorities for the same key reject without changing the registry. Unsupported
versions reject. This experimental Rust API has no stable binary ABI;
the earlier unpublished numeric fingerprint field is replaced by exact schema.
`FieldEdit.field` now uses `FieldTarget` instead of `String`: migrate direct
String assignments with `.into()`. Existing `"name".into()` callers need no change;
API version 1 denotes the statically compiled contract, not binary compatibility.

Callbacks receive egui UI, a scoped ID and an owned read-only `Snapshot`. The
snapshot rejects malformed values and duplicate fields. Return at most one
`FieldEdit` per frame. Exact top-level bool, integer, fixed-point, Vec2, enum and flag
fields are accepted after reflection kind/range checks. Whole Vec2 edits validate both axes
and create one Set/Undo operation. `FieldTarget::ArrayElement { field, index }`
also addresses exactly one scalar element of a top-level fixed array. Reflection
checks its fixed length, element kind and range before generating `field[index]`.
Lists, recursive arrays, array Vec2, structs and arbitrary path strings reject.
Vec3 and other nested/structural/entity edits remain generic-Inspector work. UI state stays outside gameplay data.

Valid intents become ordinary `InspEvent::Set` and use the existing
`Editor::set_field`/host validation, Undo/Redo and dirty-state path. The app does
not retain intents across frames. Invalid intents only report errors. For users
of the lower-level registry API, binding the same target/schema/generation and
rechecking current host authorization remain caller obligations.

Callbacks are trusted native Rust code, not sandboxed plugins. This API supplies
no host, document, disk or network handles, but cannot prevent native code using
ambient authority, blocking, recursing or panicking. No dynamic loader, registry
service, listener or credential path is added.

## Validation and remaining acceptance

Run `custom_inspector_contract` and `custom_inspector_app` with the feature
enabled, plus `inspector_number_focus` with or without it. CPU egui tests cover
custom/generic edits, invalid
intent preservation, Undo/Redo, dirty/Save/reopen, Play suppression, selection,
preview, exact-schema fallback, duplicate fields and disabled parent callbacks.
The supplied projectile scene is saved/reopened and stepped in the real Arena
host to verify the edited velocity changes movement and Stop restores the scene.
The example executable is build-checked; tests use actual EditorApp and local
hosts with injected egui input, not physical input or native window/GPU proof.

The Stormglass starter now consumes the contract for enemy health
(`CoopDefense::EnemyHealth`) and opening bosses (`CoopDefense::WaveComposition`);
see the Korean section below. Native Windows visual/physical-input acceptance,
disconnect recovery, full required checks and development integration remain
pending. Generic numeric edits now commit once on Enter, Tab, pointer or
programmatic focus transfer. Escape, disabled UI and a hidden Inspector discard
drafts without document mutation. The original focus-transfer failure and initial
compile failure are retained alongside the regression results.
Publication is separately blocked by the Windows GitHub network policy;
local validation does not waive that restriction or complete #562.

## 한국어 요약: 정적 Custom Inspector 확장 (#562)

### API와 등록

- 기능 `orr_editor/custom-inspector`를 켜고, 앱을 만들 때 `EditorApp::with_custom_inspectors(registry)`로 `Registry`를 설치한다. 기능이 꺼져 있거나 등록이 없으면 기존 일반 Inspector만 쓴다.
- 등록은 소스에 직접 쓴 정적 Rust 코드다. 앱마다 레지스트리를 하나씩 소유한다. 전역 레지스트리, 파일 탐색, 동적 DLL 로딩, 핫 리로드는 없다.
- `Extension { id, schema, priority, api_version, draw }` 하나가 컴포넌트 타입 하나를 담당한다.
- `draw`는 `fn(&mut Ui, Id, &Snapshot) -> Option<FieldEdit>`이다. 프레임마다 편집 의도를 하나까지만 돌려준다. host, 문서, 디스크 핸들은 받지 않는다.

### 타입·필드 식별과 우선순위

- 타입 식별은 `SchemaKey::for_type(&types, "이름")`으로 만든다. 반사 타입 이름과 정확한 JSON Schema 문자열이 모두 같아야 맞는다. 임의 숫자 버전이나 해시는 없다.
- 필드 식별은 `FieldTarget::Field("이름")`(최상위 필드) 또는 `FieldTarget::ArrayElement { field, index }`(최상위 고정 배열의 원소 하나)다. 중첩 경로 문자열은 거절한다.
- `priority`가 높은 확장이 등록 순서와 관계없이 이긴다.

### 편집 경로와 검증

- 유효한 의도는 일반 필드와 같은 `InspEvent::Set` → `Editor::set_field` 경로를 지난다. 그래서 검증, Undo/Redo, dirty, 저장/재열기, Play 중 편집 거절이 똑같이 적용된다.
- 범위를 벗어난 값, 형식이 다른 값, 없는 필드, 배열 범위 밖 인덱스는 거절한다. 거절해도 문서, 체크섬, 이력은 바뀌지 않는다.
- Play 중에는 사용자 정의 패널이 편집을 제공하지 않고, 호스트도 같은 편집을 거절한다.
- 사용자 정의 패널의 **Use generic fields** 선택은 화면 상태일 뿐 저장되는 게임 데이터가 아니다. 같은 컴포넌트는 언제나 일반 Inspector로도 편집할 수 있다.

### 스키마·버전·폴백 정책

- `api_version`은 현재 1만 지원한다. 다른 값은 등록 시 거절한다.
- 같은 `id`를 두 번 등록하거나, 같은 스키마에 같은 `priority`를 등록하면 거절하고 레지스트리를 바꾸지 않는다.
- 컴파일된 게임의 현재 스키마와 다르거나(공백 한 칸도 다름), 알 수 없는 타입이면 사용자 정의 패널을 쓰지 않고 일반 Inspector로 돌아간다. 문서는 바뀌지 않는다.
- 선언 API(`Extension`, `FieldEdit`, `FieldTarget` 등)가 바뀌면 이 문서의 호환성 메모에 이전 방법을 적는다. 이 API는 실험 단계이며 안정된 바이너리 ABI가 없다.

### 안전 안내

`draw`는 신뢰하는 네이티브 Rust 코드이며 샌드박스가 아니다. 이 API는 host/문서/디스크/네트워크 핸들을 주지 않지만, 네이티브 코드가 가진 기본 권한(블로킹, 재귀, 패닉 등)까지 막지는 못한다. 신뢰할 수 없는 코드를 등록하지 않는다.

### 소비자 예제 (Stormglass 스타터)

`examples/static-game-starter/src/stormglass/`가 같은 레지스트리에 확장을 등록한다. 에디터 코어, 앱 분기, 게임 enum은 건드리지 않는다.

| 확장 | 컴포넌트 | 패널 |
| --- | --- | --- |
| `stormglass-enemy-health` (`health_inspector.rs`) | `CoopDefense::EnemyHealth` | 체력 배율 막대, "최대(x4)로 설정", "기본(x1)으로 되돌리기" |
| `stormglass-opening-boss` (`boss_inspector.rs`) | `CoopDefense::WaveComposition` | 웨이브별 시작 보스 추가/제거 (`opening_boss[i]`) |
| `stormglass-wave-cadence`, `stormglass-wave-counts` | `WavePacing`, `WaveCounts` | 기존 예제 |

새 확장을 추가하려면 새 파일 하나와 `stormglass.rs`의 `mod` 한 줄, `cadence_inspector::registry()`의 `register` 호출 한 줄이 필요하다.

검증 명령:

```sh
cargo test -j 8 -p orr_static_starter --features stormglass,api-acceptance --lib _inspector
```
