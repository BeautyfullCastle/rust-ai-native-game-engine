# orr_app

`Game`을 창에 띄우는 실행 층이다. 설계는 [docs/view-framework.md](../../docs/view-framework.md) 4장이고, 이 크레이트는 12장의 1단계(서버 없이 실행, 기본 2D 렌더러), 2단계(엔티티 뷰, 씬 뷰, 이벤트 상태), 3단계(한 화면 여러 명), 4단계(플레이어 신호, 릴레이 서버로 실행), 6a단계(기본 3D 렌더러)까지 만들었다.

처음이라면 [튜토리얼 "별 줍기"](../../docs/tutorial/README.md)부터 따라 하세요. 빈 프로젝트에서 시작해 이 문서의 기능을 5단계로 하나씩 씁니다. 코드는 [tutorial/star_pick](../../tutorial/star_pick)에 있습니다.

## 쓰는 법

```rust
fn main() -> Result<(), orr_app::Error> {
    App::<PhysGame>::with_config(PhysConfig::new(1000, SceneMode::Rain))
        .tick_rate(TICK_RATE)                 // Game에는 틱 속도가 없어서 직접 준다 (기본 60)
        .players(2)                           // 시뮬레이션 플레이어 수 (기본 1). 로컬 플레이어는 0번부터 .local_players(n)개
        .bot(move |_slot, tick| bot(tick).0)  // 로컬이 아닌 슬롯의 입력 (선택)
        .input(|keys, _slot| PhysInput::new(/* keys.axis(..), keys.held(..) */))
        .view(Look(PhysExtractor { .. }))     // 무엇을 어떻게 그릴지 (View2D)
        .entity_view::<PaddleTag>(PaddleView::default) // PaddleTag가 있는 엔티티마다 뷰 하나 (EntityView)
        .scene_view(EventCounter::default())  // 화면 전체에 하나 (SceneView). 여러 개 가능
        .run()
}
```

전체 예제: [examples/physics2d.rs](examples/physics2d.rs). `cargo run -p orr_app --release --example physics2d`

## 한 화면 여러 명

```rust
App::<PhysGame>::with_config(PhysConfig::new(600, SceneMode::Rain))
    .local_players(2)                       // 슬롯 0, 1이 이 화면의 플레이어. players(2)는 따로 안 줘도 된다
    .input(|keys, slot| {
        let stick = keys.stick_for_slot(slot); // 슬롯 0: W A S D + Space, 슬롯 1: 방향키 + Enter
        PhysInput::new(stick.x, stick.y, 0, stick.action)
    })
    .entity_view::<PaddleTag>(PaddleView::default) // ctx.is_local(slot)은 두 슬롯 모두 true
    .view(Look(PhysExtractor { remote_mode: InterpMode::Prediction, local_slot: 0 }))
    .run()
```

예제: [examples/physics2d_coop.rs](examples/physics2d_coop.rs). `cargo run -p orr_app --release --example physics2d_coop`

- **장치 배정:** `KeyLayout::for_slot(slot)`이 슬롯 0에 `KeyLayout::WASD`(Space), 슬롯 1에 `KeyLayout::ARROWS`(Enter)를 준다. `stick_for_slot`은 그 키를 `Stick { x, y, action }`(`x`, `y`는 -1, 0, 1)으로 읽는다. 세 번째 슬롯부터는 기본 키가 없어서 항상 쉬는 입력이다. 직접 배정하려면 `KeyLayout { .. }`을 만들어 `stick(&layout)`에 준다. `stick_for_slot`은 키보드만 읽는다. 패드까지 따르려면 `stick_of(slot)`을 쓴다. 슬롯 0은 키보드(WASD), 슬롯 1은 첫 번째 패드(패드가 없으면 방향키), 슬롯 2는 두 번째 패드이고, `App::assign_device(slot, Device)` 또는 `InputContext::assign`으로 바꾼다. 패드는 `gilrs`(기능 `gamepad`, 기본 켜짐)가 읽고, `pad_stick(번호)`(왼쪽 스틱 또는 십자키, 남쪽 버튼이 `action`), `pad_axis`, `pad_button`으로 읽는다. 창 없이 쓰려면 `InputContext::apply_pad(PadEvent)`로 사건을 직접 넣는다.
- **게임 화면 (`Screens`):** 제목, 일시정지, 승리, 패배, 다시 시작 화면과 상태 전환(제목 → 플레이 ⇄ 일시정지 → 승리/패배 → 새 판)입니다. 확인(Enter, Space, 패드 A), 일시정지(P, Start), 다시 시작(R, Select)이 기본 동작이고 `ActionMap`으로 바꿉니다. 입력 함수는 `screens.input(keys, 읽는_함수)`를 부릅니다. 플레이 화면이 아니면 `Input::default()`(중립)를 돌려주고 읽는 함수는 부르지 않으며, 화면을 닫은 키는 뗄 때까지 게임에 보이지 않습니다. 시뮬레이션은 `Input` 값만 보므로 결정론이 유지되고, 시계는 화면이 떠 있어도 멈추지 않습니다. 결과는 뷰가 `win()`/`lose()`로 알립니다. **다시 시작은 새 세션이 아니라 게임이 정의한 리셋 `Command`입니다.** 뷰가 `take_restart()`가 `true`일 때 그 명령을 보냅니다(리플레이와 서버 대전에서도 같음, 프레임 난수는 이어짐). 사용 예: `tutorial/star_pick/src/step2.rs`, 설명: `docs/tutorial/step2-solo.md`.
- **글자 진단:** 오버레이 글꼴에 없는 글자는 속이 빈 네모로 그려지고 `Overlay::missing_glyphs()`에 쌓이며, 새 글자마다 stderr에 한 번 알립니다. 제목과 안내는 창 가운데에 놓이고 창 너비에 맞춰 줄바꿈합니다. 글꼴은 아래 "글꼴과 글자 상자"를 보세요.
- **로컬 슬롯 밖은 거부한다:** `Bridge::set_input(slot, ..)`은 `Bridge::local_slots()`에 있는 슬롯만 받는다. 다른 슬롯은 `BridgeError::NotLocalPlayer`이고 아무것도 바뀌지 않는다. 릴레이·네트워크 클라이언트는 슬롯 하나를 가지므로 `local_slots()`가 `[local_slot()]`이다.
- **`PlayerLink { player }`:** "이 엔티티는 플레이어 N의 것"이라는 표준 컴포넌트(`orr_sim`). 게임이 `register_player_link(builder)`와 `register_player_link_reflect(types)`로 켠다. 뷰는 `ctx.is_local(link.player)`로 내 것인지 본다. `PlayerSlot`은 `Pod`이고 씬 파일과 인스펙터에서 숫자(0..255)로 보인다. `PlayerSlot::NONE`(0xFF)은 "플레이어 없음"이며 릴레이 프로토콜의 `NO_SLOT`과 같다.
- **서로 다른 슬롯의 입력 시점:** 첫 로컬 슬롯의 입력은 `step(n)`을 부른 순간 고정되지만, 나머지 로컬 슬롯의 입력은 틱이 돌 때 마지막으로 받은 값을 읽는다. `App`은 매 프레임 입력을 보내고 `InProc`을 쓰므로 차이가 없다.

| 호출 | 뜻 |
|---|---|
| `App::new()` | `G::Config: Default`일 때 |
| `App::with_config(cfg)` | `G::setup(frame, &cfg)`로 시작 |
| `.scene(path, register)` | 씬 파일로 시작한다 (`G::setup`은 부르지 않는다). `register`는 게임의 `register_reflect` 같은 함수 |
| `.tick_rate(n)`, `.players(n)`, `.seed(n)`, `.title(s)` | 기본값은 60, 1, 42, "Orrery" |
| `.local_players(n)` | 한 화면을 같이 쓰는 플레이어 수 (기본 1). 슬롯 `0..n`이 로컬이다. 시뮬레이션 플레이어 수는 `max(players, n)`이다 |
| `.input(f)` | `f(&InputContext, PlayerSlot) -> G::Input`. 매 프레임 **로컬 슬롯마다 한 번씩** 그 슬롯 번호와 함께 부른다. `InputContext`는 `held(key)`, `any_held(keys)`, `axis(neg, pos)`, `stick_for_slot(slot)`, `stick(&KeyLayout)`을 준다 |
| `.bot(f)` | 로컬이 아닌 슬롯마다 매 틱 `f(slot, tick)`을 부른다 |
| `.view(v)` | `v`는 `View2D`: `Extractor`(엔티티 읽기) + `camera(frame)` + `backdrop(frame, out)` |
| `.view3d(v)` | `v`는 `View3D`: `Extractor3` + `camera(frame)` + `lighting(frame)`. 기본은 `Look3D::default()`. `.view(v)`와 같이 쓸 수 없다 |
| `.entity_view::<C>(factory)` | 예측 프레임에서 컴포넌트 `C`를 가진 엔티티마다 `factory()`로 만든 `EntityView` 하나 |
| `.scene_view(v)` | `SceneView` 하나. 여러 개를 더하면 더한 순서로 불린다 |
| `.sound(name, clip)` | 뷰의 `ctx.audio().play_named(name)`이 켜는 소리를 등록한다 (아래 "글자, 선, 소리") |
| `.font(name, bytes)`, `.font_file(path)`, `.bitmap_font()` | 화면 글자의 글꼴. 바이트(`include_bytes!`)나 파일을 주거나, 5x7 점 글꼴로 고정한다. 안 주면 시스템 한글 글꼴 (아래 "글꼴과 글자 상자") |

## 3D (6a단계)

```rust
fn main() -> Result<(), orr_app::Error> {
    App::<Yard3D>::with_config(YardConfig::new(400))
        .tick_rate(TICK_RATE)
        .input(|keys, _slot| /* keys.held(..)로 YardInput을 만든다 */)
        .view3d(Look3D::default())   // 뷰 코드 없이 그림이 나온다
        .run()
}
```

예제: [examples/yard3d.rs](examples/yard3d.rs) (17줄). `cargo run -p orr_app --release --example yard3d`

`.view(..)`(2D) 대신 `.view3d(..)`를 부르면 같은 실행 함수가 3D로 그린다. 한 실행은 2D 아니면 3D 하나이고, 둘 다 주면 `run()`이 창을 열기 전에 오류를 낸다. 엔티티 뷰, 씬 뷰, 플레이어, `--host`/`--connect`, `--screenshot`/`--ticks`는 2D와 같게 동작한다. 엔티티 뷰는 `on_update` 대신 `on_update3(ctx, entity, pose: Option<Transform3>)`를 받는다.

- **`Look3D`가 그리는 것 (뷰 코드 없이).** 물리 `Body`와 `Collider`가 있는 엔티티는 콜라이더 모양(구, 상자, 캡슐)으로 그린다. 움직이는 몸은 밝게, 쉬는 몸은 어둡게 보인다. 가로세로가 20 단위를 넘는 정적 상자는 체크무늬 바닥으로 그린다. `orr_gfx::Transform3`과 `orr_gfx::Material`이 있고 `Body`가 없는 엔티티는 `Transform3`의 크기만 한 상자로 그린다(모델의 자리 표시). **`Material`의 표면 셰이더는 아직 적용하지 않는다.**
- **보간.** 2D와 같다. `ViewWorld3`가 움직이는 몸을 예측 틱 사이에서 보간하고 롤백 튐을 보정한다.
- **기본 카메라.** 씬에 활성 `orr_gfx::Camera`가 있으면 그 카메라(우선순위가 가장 높은 것)다. 없으면 원점 바로 위를 보는 궤도 카메라 `default_orbit()`이다. 50 x 50 단위 바닥이 한눈에 들어온다. `Look3D::default().orbit(OrbitCamera::new(target, yaw, pitch, distance))`로 바꾼다.
- **기본 조명.** 씬에 `orr_gfx::Light`, `Environment`, 후처리가 있으면 `orr_gfx_view::scene_look`이 읽은 그 값이 이긴다. 없으면 낮게 뜬 오후 해와 그림자(`default_lighting()`, 원점 둘레 36 단위)다. `Look3D::default().lighting(..)`으로 바꾼다.
- **직접 만들기.** `View3D`는 `Extractor3`에 `camera(frame) -> Camera3D`와 `lighting(frame) -> Lighting`을 더한 트레이트다. 둘 다 기본 구현이 있어서 `Extractor3`만 쓰면 된다. 기본 추출은 `extract_default(frame, out)`로 부를 수 있다.
- **그림이 같다.** `--screenshot --ticks 300`의 Yard3D 그림은 Yard3D 샘플의 그리기 경로(`YardExtractor`, `fill_list`, `scene_look`, `yard_camera`)로 같은 틱을 그린 그림과 픽셀까지 같다(900x900, MSAA 4, 같은 GPU). `Look3D`의 색과 바닥 규칙은 `YardExtractor`와 같게 맞췄다.
- **렌더러 설정은 `Settings3D::default()`다**(MSAA 4, 그림자 지도 2048). 바꾸는 옵션은 아직 없다.

## ViewRef, 뷰 에셋, 프리팹 (6b단계)

같은 컴포넌트 구성이어도 엔티티마다 다른 모델을 그리는 방법(`ViewRef`와 뷰 에셋)과, 시뮬레이션 시스템이 프리팹으로 엔티티를 만드는 방법이다.

```rust
// 게임: 등록 (Game::register)
orr_gfx::register_view_ref(builder);          // ViewRef 컴포넌트 (orr_gfx::register에는 들어 있지 않다)
orr_ecs::prefab::register(builder);           // 프리팹 보관소

// 게임: 시작 (Game::setup). 프리팹은 시뮬레이션이 만들어지기 전에 구워 둔 값이다
orr_ecs::prefab::install(frame, &config.avatars)?;

// 게임: PlayerAdded를 받는 곳 (Players::on_player). 파일을 읽지 않는다
let avatar = orr_ecs::spawn_prefab(frame, PrefabId(u32::from(slot.0) % orr_ecs::prefab::prefab_count(frame)));

// 실행: 프로젝트의 뷰 에셋을 읽어 3D 룩에 준다
let assets = ViewAssets::load_dir("assets")?;
App::<Lobby>::with_config(config).view3d(Look3D::default().assets(assets)).run()
```

예제: [examples/avatars.rs](examples/avatars.rs). `cargo run -p orr_app --release --example avatars` (WASD와 방향키로 두 플레이어가 한 화면에서 움직인다). 서버로도 된다(`-- --host`, `-- --connect 127.0.0.1:7777`). 시작하면 각 플레이어마다 `PlayerAdded`로 아바타가 생기고, 아바타는 뷰 에셋이 가리키는 모델(`examples/avatars/*.glb`)로 그려진다. 그림은 `--screenshot out.png --ticks 120`.

- **`ViewRef { asset: u64 }`** (`orr_gfx`). 시뮬레이션에 들어가는 `Pod` 컴포넌트다. 시뮬레이션은 번호를 읽기만 하고 뜻을 모른다. 번호는 뷰 에셋 파일 경로(에셋 폴더 기준, `/` 구분)의 64비트 FNV-1a 해시다(`orr_gfx::view_asset_id(path)`, `ViewRef::at(path)`). 0은 "없음"이라 해시가 0이면 1로 바꾼다. 경로가 번호를 정하므로 파일을 옮기면 번호가 달라진다(인스펙터는 이때 "missing asset"을 보여 준다).
- **씬 파일과 인스펙터.** `ViewRef`는 `orr_gfx::ViewRef: { asset: 123 }`으로 저장된다(`u64` 전체 범위). 리플렉션에는 정수 필드의 `#[reflect(asset_id = "view")]` 속성을 더했다(`Kind::Int`의 `asset`, `ASSET_KINDS`의 `"view"` 종류, 확장자 `orrview`). 에디터 인스펙터는 이 필드를 숫자 대신 씬 폴더의 뷰 에셋 목록(파일 이름)에서 고르는 콤보 박스로 보여 준다.
- **뷰 에셋 파일 `*.orrview`** (JSON, 버전 1). `{ "version": 1, "name": "Knight", "model": "models/knight.glb", "scale": 1.0, "tint": [1, 0.8, 0.6], "sound": "step", "entity_view": "knight" }`. `version`과 `model`만 있으면 된다. `model`은 에셋 폴더 기준 `.glb`/`.gltf` 경로다. 모르는 키는 오류다. `ViewAssets::load_dir(폴더)`가 폴더 안(4단계 깊이까지, 숨김 폴더 제외)의 모든 `.orrview`를 읽고, 모델 파일이 없거나 두 파일의 번호가 같으면 오류를 낸다. `scale`만 그리기에 쓴다. `tint`, `sound`, `entity_view`는 읽어서 보관만 한다(아래 "남은 것").
- **그리기.** `Look3D`는 `Transform3`과 `ViewRef`가 있는 엔티티를 (`Material`과 마찬가지로) 자리 표시 상자로 뽑아 다른 엔티티와 같이 보간한다. 그릴 때 `Look3D::assets(..)`에 그 번호의 에셋이 있으면 상자 대신 에셋의 모델을, 보간된 자리에, `Transform3`의 크기(몸이 있으면 1) 곱하기 에셋의 `scale`로 그린다. 번호에 맞는 에셋이 없으면(또는 `assets`를 주지 않으면) 자리 표시 상자가 그대로 남아서 빈 곳이 눈에 보인다. `Body`가 있는 엔티티가 `ViewRef`를 가지면 몸이 자리를 정하고 모델이 모양을 대신한다. 창 없이 확인할 수 있는 결과는 `ModelDraw { asset, model, instance }` 목록이다(`fill_draw_list3`).
- **모델이 있는 프레임은 합성 경로로 그린다.** 도형과 모델을 한 장에 같은 깊이, 해, 그림자로 그리려고 `ImportedSceneRenderer`(도형은 `ImportedBatch::Procedural`, 모델은 `StaticInstances`)를 쓴다. 이 경로는 1샘플이라서 **MSAA와 톤 매핑이 없다**. 모델이 없는 프레임은 예전 경로 그대로이고, 그림은 바뀌지 않는다. 모델 파일마다 렌더러 하나를 처음 쓸 때 만들고, 읽기에 실패하면 stderr에 한 번 알리고 그 모델은 빼고 그린다. 모델 하나는 프레임당 256개까지 그린다(`MAX_STATIC_INSTANCES`). 그리기 수(모델의 primitive 수 × 인스턴스 수를 모든 모델에 걸쳐 합한 값)에는 한도가 있어서(`model_draw_budget`), 한도를 넘는 인스턴스는 모델 이름 순서대로 앞의 것만 남기고 버린다. 버린 수는 세고 stderr에 한 번 알리며, 도형과 나머지 모델은 그대로 그린다.
- **프리팹 (`orr_ecs::prefab`).** `Prefab`은 컴포넌트 바이트의 목록이다. `Prefab::with(registry, 값)`으로 코드에서, 또는 `orr_reflect::Scene::bake_prefabs(types, registry)`로 씬 조각에서 굽는다(엔티티 하나가 프리팹 하나, GUID 순서가 `PrefabId`). 조각 안의 엔티티끼리 가리키는 참조는 오류다. `install`이 프리팹을 **프레임 안**(`PrefabLibrary` 싱글턴과 `PrefabPart` 리스트)에 복사한다. 시스템과 `on_player`가 게임의 `Config`를 볼 수 없어서(`Config`는 `setup`만 받는다)이고, 덕분에 스냅숏, 체크섬, 중간 입장 스냅숏에 프리팹이 같이 들어가 롤백 재시뮬레이션과 리플레이가 같은 엔티티를 만든다. `spawn_prefab`은 프레임만 읽는다(파일, 시계, 해시맵 없음). 없는 `PrefabId`는 아무것도 바꾸지 않고 `None`을 준다. 컴포넌트가 128바이트보다 크면 여러 부분으로 나누어 저장한다.
- **기존 `scene linked` 프리팹은 쓰지 않았다.** 그것은 에디터가 원본 스냅숏과 다이제스트를 검증하는 편집용 메타데이터(`PrefabLink`)라서 실행 중 생성 경로가 아니다. 대신 같은 씬 파일 형식과 `Scene::parse`, 리플렉션의 `desc().write`로 값을 바이트로 만든다.
- **테스트 게임 `orr_testgame::lobby::Lobby`.** `Added`가 오면 `slot % 프리팹 수`번 프리팹(시작 데이터 `Profile.choice`가 있으면 그것)으로 `Transform3`, `ViewRef`, `PlayerLink`가 있는 아바타를 만들고 `PlayerLink`와 자리를 맞춘다. `Removed`는 아바타를 지운다. 입력 `LobbyInput`으로 땅 위를 걷는다.

## 엔티티 뷰와 씬 뷰

```rust
impl EntityView<PhysGame> for PaddleView {
    fn on_create(&mut self, ctx: &mut ViewContext<'_, PhysGame>, entity: Entity) { .. }
    fn on_update(&mut self, ctx: &mut ViewContext<'_, PhysGame>, entity: Entity, pose: Option<Transform2>) { .. }
    fn on_destroy(&mut self, ctx: &mut ViewContext<'_, PhysGame>, entity: Entity) { .. }
}

impl SceneView<PhysGame> for EventCounter {
    fn on_start(&mut self, ctx: &mut ViewContext<'_, PhysGame>) { .. }
    fn on_update(&mut self, ctx: &mut ViewContext<'_, PhysGame>) { .. }
    fn on_event(&mut self, ctx: &mut ViewContext<'_, PhysGame>, event: &PhysEvent, state: EventState) { .. }
    fn on_player(&mut self, ctx: &mut ViewContext<'_, PhysGame>, signal: &RawPlayerSignal, state: EventState) { .. }
    fn on_session(&mut self, ctx: &mut ViewContext<'_, PhysGame>, note: Lifecycle) { .. }
    fn on_resync(&mut self, ctx: &mut ViewContext<'_, PhysGame>) { .. }
}
```

모든 메서드의 기본 몸체는 비어 있다. 한 프레임의 호출 순서는 `poll_view` → `on_start`(처음 한 번) → 세션 알림(`on_session`), 플레이어 신호(`on_player`), 이벤트(`on_event`) → `on_destroy` → `on_create` → 엔티티 `on_update` → 씬 `on_update` → 그리기다.

- **엔티티 키는 `(번호, 버전)`이다.** 번호가 재사용되어 버전이 달라지면 옛 엔티티는 `on_destroy`, 새 엔티티는 `on_create`다. 롤백으로 예측에만 있던 엔티티가 사라져도 `on_destroy`가 불린다. 같은 프레임 안에서 사라졌다 같은 키로 다시 생기는 경우(롤백 재계산 안)는 화면이 프레임 단위로만 보므로 구별되지 않는다.
- **`pose`는 `ViewWorld`가 계산한 `Transform2`이다.** `Extractor`가 그 엔티티를 내놓지 않으면 뷰 월드에 위치가 없어서 `None`이다. `on_update`는 그래도 불린다.
- **`EventState`는 `orr_session::EventStatus`에서 페이로드를 뺀 얇은 열거형이다.** `Predicted`, `Verified`, `Canceled`. `Canceled` 호출에는 `Predicted` 때 받은 페이로드가 그대로 온다(세션이 취소에는 페이로드를 주지 않아서 `Views`가 기억한다). 소리·파티클은 `Predicted`에서 시작하고 `Canceled`에서 멈춘다. 업적·통계는 `Verified`에서만 한다.
- **`ViewContext`:** `predicted()`, `verified()`(`Option`), `tick()`, `verified_tick()`, `local_players()`, `is_local(slot)`, `send_command(cmd)`, `draw()`(글자와 선), `audio()`(소리). 마지막 둘은 아래 "글자, 선, 소리". 오프라인에서 로컬 플레이어는 `.local_players(n)`의 슬롯 `0..n`이고, `local_players()`와 `is_local(slot)`이 모두 보고한다.
- **창 없이도 쓴다.** 호출 순서를 지키는 코드는 창과 렌더러가 없는 `Views::dispatch(bridge, update, poses)`에 있다. 창 루프는 이것을 부르기만 한다. 테스트는 `tests/views.rs`이고, 롤백이 나는 두 세션(`LoopbackPair`)에서 `on_create`/`on_destroy`와 `Canceled`/`Verified`를 확인한다.

## 글자, 선, 소리

뷰가 기본 렌더러 위에 글자와 선을 그리고 소리를 냅니다. 둘 다 `ViewContext`로 씁니다.

```rust
impl SceneView<StarPick> for Juice {
    fn on_event(&mut self, ctx: &mut ViewContext<'_, StarPick>, event: &Picked, state: EventState) {
        match state {
            // 예측 틱에서 났다. 롤백이 지울 수 있으니 손잡이를 기억해 둔다.
            EventState::Predicted | EventState::Verified => self.sound = ctx.audio().play_named("pickup"),
            // 롤백이 이벤트를 되돌렸다. 소리를 끈다.
            EventState::Canceled => {
                if let Some(sound) = self.sound.take() {
                    ctx.audio().stop(sound);
                }
            }
        }
    }

    fn on_update(&mut self, ctx: &mut ViewContext<'_, StarPick>) {
        let draw = ctx.draw();
        draw.text([16.0, 14.0], 21.0, [1.0, 0.85, 0.25, 1.0], "점수 12"); // 화면 좌표: 픽셀, 왼쪽 위 원점
        draw.line([0.0, 0.0], [2.0, 1.0], [1.0, 0.0, 0.0, 1.0]);          // 세계 좌표
        draw.rect([-1.0, -1.0], [1.0, 1.0], [0.0, 1.0, 0.0, 0.3]);
    }
}

App::<StarPick>::new()
    // ...
    .sound("pickup", clip)   // Clip::from_stereo(rate, 샘플)로 만든 소리. 앱을 만들 때 한 번 등록한다
    .scene_view(Juice::default())
```

- **`ctx.draw()`는 `Overlay`를 준다.** 프레임마다 비워진다(`Views::dispatch`가 시작할 때). 그래서 보이게 하려면 매 프레임 부른다. 호출은 `text(pos, size, color, &str)`(화면 좌표. `pos`는 대문자 윗선의 왼쪽 위, `size`는 대문자 높이 픽셀), `text_box(rect, size, color, &str, align)`(줄바꿈과 정렬, 아래 "글꼴과 글자 상자"), `text_anchored(앵커, 이동, size, color, &str)`, `line(a, b, color)`, `rect(min, max, color)`(채운 사각형), 3D 실행용 `line3(a, b, color)`와 `aabb3(min, max, color)`다. 2D 실행은 `line3`의 `x`와 `y`만 그린다. 3D 실행에서 `rect`는 `z = 0` 평면의 윤곽선이다. 폭을 재려면 `ctx.draw().measure(text, size)`다(5x7 점 글꼴만의 `text_size(text, size)`도 있다).
- **무엇이 그리는가.** 선과 사각형은 기본 렌더러의 그리기 목록 끝에 더해 같은 패스에서 그린다. 글자는 렌더러가 그린 같은 그림 위에 한 번 더 그린다(`TextPass`). 그래서 글자는 항상 맨 위이고 `--screenshot` 그림에도 `--size` 크기로 나온다. 모델이 있는 3D 프레임에도 같다. 글꼴이 있으면 글리프 아틀라스의 텍스처 사각형이고, 없으면 `orr_render::text`의 5x7 점 글꼴을 색 사각형으로 바꿔 그린다.
- **글꼴이 없을 때(대체).** 5x7 점 글꼴은 ASCII만이다. 숫자, 영문 대문자(소문자도 대문자로 보인다), `. : , / | - + = _ ( ) [ ] % ! ?`가 있고, 한글은 그리지 못한다.
- **글자 진단.** 선택한 글꼴에 없는 글자(점 글꼴이면 한글도 포함)는 속이 빈 네모로 그려지고 `Overlay::missing_glyphs()`에 쌓이며, 새 글자마다 stderr에 한 번 알린다.
- **`ctx.audio()`는 `Audio`를 준다.** `play(&clip) -> Option<SoundHandle>`, `play_named("이름")`(`App::sound`로 등록한 소리), `play_at(&clip, [x, y, z], near, far)`(그 점에서 난 것처럼. `near` 안에서 크고 `far`부터 들리지 않으며 그쪽 스피커로 치우친다. 귀의 자리는 `set_listener`, 기본은 원점), `stop(손잡이)`(짧게 줄이며 끈다), `is_playing(손잡이)`다. `Predicted`에서 켜고 손잡이를 기억했다가 `Canceled`에서 `stop`한다. 이미 끝난 소리를 `stop`하는 것은 아무 일도 아니다. `ViewResync`가 오면 그 전에 `Predicted`로 켠 소리의 `Canceled`는 오지 않으므로, `on_resync`에서 그 소리들을 끈다.
- **장치는 앱이 연다.** 첫 `play`에서 시스템 오디오 장치(`orr_audio`의 `NativeAudio`, 기능 `audio-native`, 기본으로 켜져 있다)를 연다. 장치가 없거나 열지 못하면 소리 없이 계속 돌고 stderr에 한 번 알린다. 도중에 장치 오류가 나면 매 프레임 확인해서 출력을 버리고 한 번 알린다. `--screenshot` 실행은 장치를 열지 않는다. `audio-native`를 끄고 빌드하면 소리는 항상 조용하다.
- **소리는 시뮬레이션에 닿지 않는다.** 게임 규칙, 체크섬, 리플레이와 상관없다. 소리 클립은 `Clip`(`orr_audio`, 다시 내보냄)이고 한 번 만든 것을 여러 번 켠다(데이터는 공유된다). 10초까지다.
- **창 없이도 쓴다.** `Views::overlay()`로 마지막 `dispatch`가 그린 것을 읽는다. `Views::set_sound_out(출력)`에 `SoundOut`을 주면 그 출력으로 소리를 낸다. `Rc<RefCell<OfflineAudio>>`를 주면 소리가 PCM으로 렌더링되어 테스트가 들을 수 있다(`OfflineAudio::render`). 테스트는 `tests/draw_audio.rs`(그리기 목록이 채워지고 다음 프레임에 비워진다. 소리가 나고 `stop` 뒤에 조용하다. 롤백이 낸 `Canceled`가 소리를 멈춘다. 출력이 없으면 아무 일도 없다)와 `src/overlay.rs`의 단위 테스트(글꼴 폭, 글자 사각형, 상자 모서리)다.
- **음량, 반복, WAV, 이벤트 소리, 저장.** `Audio`의 `play_named_with`, `play_looped`, `set_volume`, `set_master_volume`, `App::sound_file`/`sound_wav`(WAV 읽기), `App::on_event_sound`(롤백과 리싱크를 한 곳에서 처리하는 이벤트 소리), 저장 슬롯과 체크포인트(`App::saves`, `save_key`, `load_key`, `checkpoint_on`, `ctx.save_game`/`load_game`)는 [docs/app-saves-audio.md](../../docs/app-saves-audio.md)에 있다.
- **튜토리얼.** [2~5단계](../../docs/tutorial/step2-solo.md)가 점수와 안내를 한글로 그린다. 2단계는 WAV 파일 소리와 이벤트 소리와 저장(F5, F9)을 쓰고, 3~5단계는 생성한 "딩" 소리(`tutorial/star_pick/src/sound.rs`)를 뷰 안에서 켜고 끈다.

## 글꼴과 글자 상자

글자는 진짜 글꼴(TrueType/OpenType)로 그립니다. 글리프 래스터라이저는 이미 `Cargo.lock`에 있던 `ab_glyph`(MIT OR Apache-2.0)입니다. **글꼴 파일은 저장소에 넣지 않습니다.**

```rust
App::<StarPick>::new()
    .font_file("C:/Windows/Fonts/malgun.ttf")   // 파일 경로
    // .font("내 글꼴", include_bytes!("font.ttf")) // 또는 바이트 (게임이 글꼴을 함께 배포할 때)
    // .bitmap_font()                              // 5x7 점 글꼴로 고정 (어느 컴퓨터에서나 같은 그림)
```

- **기본 글꼴.** `.font`/`.font_file`/`.bitmap_font`을 안 부르면 시스템 글꼴을 찾습니다. 후보는 Windows의 `malgun.ttf`(에디터의 `crates/orr_editor/src/theme.rs`와 같은 생각), Linux의 Noto Sans CJK와 NanumGothic, macOS의 Apple SD Gothic Neo 순이고 `system_font_candidates()`로 읽습니다. 하나도 없으면 한 번 알리고 5x7 점 글꼴(ASCII만)로 돌아갑니다. 준 글꼴이 읽히지 않으면(`.font`, `.font_file`) 실행이 오류로 멈춥니다.
- **글리프 아틀라스.** 글자는 처음 필요할 때 한 번만 래스터라이즈해 1024x1024 텍스처에 쌓고(글자와 크기가 한 키), 다음 프레임부터는 그대로 씁니다. 가득 차면 아틀라스를 한 번 비우고 그 프레임의 글자를 다시 넣습니다. 한 프레임 글자가 아틀라스보다 많으면 못 들어간 글자는 그리지 않고 한 번 알립니다. 2D, 3D, 모델이 있는 3D, `--screenshot`이 모두 같은 경로입니다.
- **크기.** `size`는 대문자 높이(픽셀)입니다. 글꼴의 em은 `size / 0.7`로 정수 픽셀에 맞추고 4에서 400 사이로 자릅니다. 그래서 점 글꼴로 맞춘 기존 배치가 거의 그대로 맞습니다. 한글은 대문자보다 조금 위아래로 나옵니다.
- **`text_box(rect, size, color, &str, align) -> [폭, 높이]`.** `rect`는 `ScreenRect { x, y, w, h }`(픽셀, 왼쪽 위 원점)입니다. 공백에서 줄을 나누고, 한 단어가 줄보다 길면(띄어쓰기 없는 한글) 글자(음절) 사이에서 나눕니다. `
`은 새 줄입니다. `align`은 `Align::Left`, `Center`, `Right`입니다. `rect.h`는 자르지 않습니다(줄이 아래로 이어집니다). `rect.w`가 0이면 줄바꿈 없이 한 줄입니다. 돌려주는 값은 가장 긴 줄의 폭과 전체 높이라서, 그 아래에 다른 글자를 놓을 수 있습니다.
- **화면에 붙이기.** `ctx.screen_size()`(`Views::set_screen_size`)가 창 크기를 줍니다. `--screenshot`이면 그림 크기입니다. 창 없는 `Views`는 `set_screen_size`를 부르기 전까지 `[0, 0]`입니다. `ctx.draw().screen_rect()`가 창 전체 `ScreenRect`이고, `rect.inset(여백)`과 `rect.anchored(Anchor::Center, w, h, 이동)`으로 가운데나 모서리에 놓습니다. 한 줄은 `text_anchored(Anchor::TopRight, [-16.0, 14.0], 21.0, 색, "글")`처럼 쓰고, 이동은 오른쪽 아래가 양수입니다. 창 크기를 모르면 글자가 왼쪽 위 모서리에서 시작합니다.
- **`Screens`.** 제목, 일시정지, 승리, 패배 글자를 창 가운데에 놓고 창 너비에 맞춰 줄바꿈합니다. 글자는 `Labels`에 한글로 줍니다.
- **비용(대략).** 글자 310개, 1280x720 화면 밖 그림, **디버그 빌드**에서 아틀라스가 식은 첫 프레임이 3.7 ms, 아틀라스가 데워진 뒤 프레임이 1.8 ms입니다(따로 잰 값: `text_box` 10번의 줄바꿈과 배치 1.2 ms, 글자 사각형 만들기 1.1 ms. 두 값은 따로 쟀으므로 더해도 프레임 시간과 같지 않습니다). 릴리스 빌드는 재지 않았습니다. 한 프레임의 일은 글자 수에 비례하고, GPU는 텍스처 사각형 인스턴스 그리기 한 번입니다.

### 뺀 것

- **글자 모양 잡기(shaping).** 글자 하나가 글리프 하나입니다. 한글 음절, 가나, 한자, 라틴은 맞지만 합자, 아랍어와 인도 문자의 모양 바꾸기, 조합형 자모(`ㅎ` + `ㅏ` + `ㄴ`을 이어 쓰기)는 못 그립니다. 자간 조정(kerning)도 없습니다.
- **IME 입력.** 글자를 입력하는 쪽(텍스트 상자, 조합 중인 글자)은 없습니다. 그리기만 합니다.
- **글꼴 대체 사슬.** 글꼴은 하나입니다. 그 글꼴에 없는 글자(이모지 등)는 다른 글꼴로 넘어가지 않고 빈 네모로 그리고 진단합니다.
- **줄 머리·끝 금칙**(문장부호가 줄 맨 앞에 오지 않게 하기)과 양쪽 정렬, 글자 굵기와 기울임, 외곽선과 그림자, 서브픽셀 위치(글리프는 정수 픽셀에 놓입니다)와 DPI 크기 조절.

## 플레이어 신호 (4a단계)

플레이어가 들어오고, 끊기고, 돌아오고, 나가는 일을 게임에 알리는 신호다. 신호는 입력의 일부라서 모든 피어가 같은 틱에 같은 신호를 받고, 리플레이와 롤백 재계산에서도 같은 틱에 다시 적용된다.

```rust
impl Players for MyGame {
    type Player = Profile;                       // 고정 크기 Pod. 이름, 색, 고른 캐릭터 같은 시작 데이터
    fn on_player(frame: &mut Frame, signal: PlayerSignal<Profile>) {
        match signal {
            PlayerSignal::Added { slot, data } => { /* 아바타 만들기 */ }
            PlayerSignal::Disconnected { slot } => { /* 멈추기, AI가 대신 조종 */ }
            PlayerSignal::Reconnected { slot } => { /* 조종권 돌려주기 */ }
            PlayerSignal::Removed { slot } => { /* 아바타 지우기 */ }
        }
    }
}
// Game::systems()의 맨 앞에 넣는다:  let mut s = vec![player_signals::<Self>()]; s.extend(fn_systems([..]));
```

- **`Players`는 선택이다.** 기존 `Game` 구현은 바꾸지 않는다. `player_signals::<G>()`를 `systems()` 맨 앞에 넣은 게임만 신호를 `on_player`로 받는다. 넣지 않은 게임에서 신호는 시뮬레이션이 무시한다(`TickInputs::signals()`로는 읽을 수 있다). 신호가 없는 틱은 비용이 없다(빈 목록). 신호를 쓰지 않는 게임의 체크섬과 리플레이 파일은 그대로다.
- **신호의 모양.** `RawPlayerSignal { kind, slot, data: Vec<u8> }`이다. `data`는 `Added`의 `Players::Player` 바이트이고, 비어 있거나 크기가 다르면 `Player::default()`다. `raw.typed::<Profile>()`이 `PlayerSignal<Profile>`을 준다. 한 틱의 신호는 슬롯 순서로 적용되고, 같은 슬롯에서는 넣은 순서다(`insert_signal`).
- **서버 없이 실행하면** `App`이 첫 틱(틱 1)에 모든 슬롯(`players`와 `local_players` 중 큰 수)에 `Added`를 넣는다(`PlayHost::with_announced_players`). 데이터는 기본값이다. 그 신호는 기록에 들어가므로 리플레이를 다시 돌려도 같은 월드가 된다. 틱 1 한 번뿐이다. 되감기(`seek`)는 기록된 신호를 다시 적용하지만 뷰에 알림은 다시 보내지 않는다.
- **씬 뷰의 `on_player`.** 신호는 이벤트와 같은 세 상태(`Predicted`, `Verified`, `Canceled`)로 온다. 브리지가 새 `BridgeEvent::Player { key, status }`로 전달하고, 키는 `(틱, 슬롯, 그 슬롯의 순번)`이다(`orr_sim::signal_key`). 롤백 재계산에서도 같다. 한 틱의 신호는 그 틱의 게임 이벤트보다 먼저 온다. 오프라인에서는 예측이 없어서 모두 바로 `Verified`다. `Canceled`에는 `Predicted` 때의 신호가 그대로 온다. `ViewResync`가 오면 기억을 비운다. 신호는 이미 프레임에 적용돼 있으므로 `Predicted` 때 `ctx.predicted()`에 아바타가 있다.
- **왜 게임 이벤트가 아니라 브리지 이벤트인가.** 게임이 `on_player`에서 이벤트를 다시 내보내게 하면 게임마다 이벤트 타입을 정해야 하고 `Players`가 `SimContext`에 묶인다. 신호는 입력이라서 세션이 이미 틱 기록을 가지고 있다. 세션이 그 기록에서 `Predicted`/`Verified`/`Canceled`를 직접 만드는 쪽이 이벤트 조정(`diff_events`)과 똑같이 동작하고 게임 코드가 필요 없다.
- **세션 쪽.** `Session::schedule_signal(tick, signal)`이 신호를 틱 입력에 넣는다. 이미 예측으로 지난 틱이면 다음 `step`/`poll_confirmed`가 그 틱으로 롤백해서 다시 계산한다. 검증이 끝난 틱은 `SignalError`다. 모든 피어가 같은 신호를 같은 틱에 넣어야 한다(릴레이 서버가 보내는 `from_tick`이 그 틱이다. 아래 "서버로 실행"). `PlaySession::push_signal`은 다음 틱에 넣고 기록한다. 리플레이 파일은 신호가 있을 때만 형식 4(신호 표가 끝에 붙는다)로 쓰고, 없으면 지금까지처럼 3이다.
- **예제:** `examples/physics2d_coop.rs`의 `Roll`이 신호를 출력한다. `PhysGame`은 패들을 `setup`에서 만들고, 이것을 신호로 바꾸면 골든 체크섬이 바뀌어서 `Players`를 쓰지 않았다. 신호로 아바타를 만드는 게임은 `orr_testgame::roster::Roster`다(테스트 전용 게임).
- **테스트:** `orr_testgame/tests/roster.rs`(시뮬레이션), `orr_session/tests/player_signals.rs`(리플레이, 되감기, 가지치기, 오프라인 알림, 롤백 후 같은 체크섬, 알림 상태), `orr_bridge/tests/player_signals.rs`, `orr_app/tests/player_signals.rs`(씬 뷰), `orr_server/tests/player_signals.rs`(서버와 클라이언트 두 명: 재접속, 보관 만료, 중간 입장, 손실이 있는 네트워크), `orr_app/src/net.rs`의 테스트(진짜 소켓의 창 두 개).

## 서버로 실행 (4b단계)

같은 `App` 코드가 세 가지 방식으로 돈다. 명령줄 인자만 다르다. 기본은 서버 없이 실행이다.

```text
roster                                   서버 없이 (한 화면 여러 명은 .local_players(n))
roster --host [포트 | 주소:포트]          릴레이 서버를 이 프로세스에서 띄우고, 이 창도 거기에 접속한다 (기본 127.0.0.1:7777)
roster --connect 127.0.0.1:7777          그 서버에 접속하는 다른 창 (--slot n --token t: 닫았던 창이 자리를 되찾는다)
```

- **창 하나가 플레이어 한 명이다.** 서버는 `.players(n)`과 `.local_players(n)` 중 큰 수만큼 자리를 만들고, 자리가 다 차면 게임을 시작한다(그 전에는 창이 열리지 않는다). `--host` 창이 0번 슬롯이고, `--connect` 창은 빈 슬롯을 받는다. 접속하면 `playing as slot 1 of 2 ... --slot 1 --token ...` 줄을 출력한다. 창을 닫았다가 그 인자로 다시 열면 같은 슬롯을 되찾고, 다른 창은 `Disconnected` → `Reconnected`를 본다.
- **입력 함수의 `slot`.** 접속한 창에서 `.input(|keys, slot| ..)`의 `slot`은 서버가 준 슬롯이 아니라 이 창의 로컬 번호(0)다. 그래서 창마다 WASD가 먹는다. 뷰의 `ctx.is_local(slot)`과 `local_players()`는 서버가 준 실제 슬롯을 쓴다.
- **전송.** `ws://` WebSocket이다(인증서가 필요 없다). 같은 PC와 LAN용이다. 다른 PC가 접속하려면 `--host 0.0.0.0:7777`로 띄운다. 인터넷 서버는 `orr_server` 바이너리와 인증서를 쓴다. 두 쪽은 게임 타입 이름에서 만든 빌드 번호가 같아야 접속된다(`net::build_id`).
- **봇과 씬 파일.** 서버로 실행하면 `.bot(..)`은 쓰지 않는다(다른 슬롯은 사람 창이다). `.scene(..)`은 아직 안 된다(오류로 알려 준다). `G::Config`는 `Send`다(`Game`이 이미 요구한다).
- **`--ticks`, `--screenshot`.** 서버 시계를 따라가므로 시뮬레이션을 그 틱까지 돌리지 못하고, 그 틱에 닿을 때까지 기다린 뒤 프레임을 센다. 그림은 두 창이 같은 틱에서 찍히지 않을 수 있다.
- **예제.** `examples/roster.rs`(신호로 아바타가 생기고, 자리를 비우면 흐려진다), `examples/physics2d_coop.rs`(같은 코드가 한 화면 2인과 창 두 개 모두 된다).

```text
cargo run -p orr_app --release --example roster -- --host
cargo run -p orr_app --release --example roster -- --connect 127.0.0.1:7777
```

### 서버가 보내는 신호

서버(`orr_server`)는 자리가 바뀔 때마다 모든 플레이어에게 알리고, 클라이언트는 그것을 `from_tick`의 신호로 `Session::schedule_signal`에 넣는다. 그래서 모든 창이 같은 틱에 같은 신호를 받는다.

| 일 | 메시지 | 신호 |
|---|---|---|
| 게임 시작 | 자리마다 `Presence { present: true, from_tick: 1 }` | `Added` (틱 1) |
| 중간 입장(새 슬롯) | `Presence { present: true }` | `Added` |
| 연결 끊김, 자리는 보관 중 | `Presence { present: false }` | `Disconnected` |
| 같은 토큰으로 돌아옴 | `SeatChange { RECONNECTED }` (프로토콜 v3 새 메시지, 종류 13) | `Reconnected` |
| 보관 시간(`reserve_ticks`) 끝 또는 강퇴 | `SeatChange { REMOVED }` | `Removed` |

- **`from_tick`은 알린 틱보다 `signal_lead_ticks`(기본 10) 뒤다.** 알림은 신뢰 채널로, 확정 묶음은 비신뢰 채널로 간다. 패킷을 잃으면 클라이언트가 묶음을 먼저 받아 그 틱을 검증해 버릴 수 있다. 여유를 두면 신뢰 채널이 다시 보낼 시간이 있다. 시작 때의 `Added`(틱 1)에는 여유가 없다.
- **중간 입장.** 서버는 지난 자리 변경을 `retain_ticks`만큼 기억한다. 스냅숏을 받는 창에게는 스냅숏 틱보다 뒤의 변경만 다시 보낸다. 스냅숏에 이미 들어 있는 신호는 오지 않는다. 클라이언트는 세션이 만들어질 때까지 받은 신호를 쌓아 둔다.
- **메시지.** `Presence`는 그대로다. `SeatChange`는 `Reconnected`와 `Removed`일 때만 나간다. 프로토콜 버전은 하나라서(`orr_proto::PROTOCOL_VERSION`) 다른 버전의 클라이언트는 접속하지 못한다.
- **권한 있는 서버 방(authoritative).** 서버 시뮬레이션이 신호를 받지 못하므로 신호를 보내지 않는다. 클라이언트도 적용하지 않는다. 옛 `Presence`만 간다.

## 명령줄

```text
--screenshot <png>  --frames 개수만큼 그린 뒤 그림 한 장을 저장하고 끝낸다
--frames <n>        그림 전에 그리는 프레임 수 (기본 30)
--ticks <n>         시뮬레이션을 n틱까지 돌리고 거기서 멈춘다
--size <WxH>        창 크기이자 그림 크기 (기본 900x900)
--no-vsync          화면 주사율을 기다리지 않는다
--host [포트|주소]  릴레이 서버를 이 프로세스에서 띄우고 접속한다 (기본 127.0.0.1:7777)
--connect <주소>    릴레이 서버에 접속한다 (host:port)
--slot <n>          --connect와 함께: 이 슬롯을 요청한다
--token <n>         --connect와 함께: 창이 출력한 토큰으로 자리를 되찾는다
--help
```

- `--ticks`가 있으면 시계를 쓰지 않는다. 프레임마다 시뮬레이션은 최대 240틱씩 돌고, 화면은 프레임당 정확히 1틱 시간만큼 보간한다. 그래서 기계 속도와 관계없이 같은 그림이 나온다(서버로 실행할 때는 위).
- 그림은 창과 따로 `--size` 크기의 화면 밖 텍스처에 다시 그려서 읽는다. 창 크기나 화면 배율에 영향을 받지 않는다.
- Esc 또는 창 닫기로 끝낸다.

## 지금 정한 것

- **서버 없이 실행하는 브리지는 `InProc`이다.** 서버로 실행하면 `Threaded` 위의 릴레이 클라이언트다(`net.rs`, 내부 `Driver`가 둘을 한 `Bridge`로 묶는다). `InProc`은 시뮬레이션이 그리기와 같은 스레드에서 돌아서 `--ticks`가 정확하고, `G::Config`에 `Send`를 요구하지 않는다. 물리 샘플의 `physics` 바이너리는 기본이 `Threaded`다. `App::threaded()`는 만들지 않았다. 오프라인 세션의 `PlayHost`가 `Send`가 아닌 봇 클로저를 들고 있고, `Threaded::spawn`은 호스트를 다른 스레드에서 만들어야 하기 때문이다. `Views::dispatch`는 `&mut dyn Bridge<G>`를 받으므로 `Threaded`에도 그대로 쓸 수 있다.
- **오프라인 세션은 `PlayHost` 위에 있다.** 롤백이 없고, 입력은 로컬 슬롯 `0..n`이다(`PlayHost::with_extra_local_slots`). 롤백이 나는 경우는 `LoopbackPair` 테스트로만 확인했다.
- **시뮬레이션 규칙은 건드리지 않았다.** 화면은 `Snapshot`을 읽고 `set_input`, `send_command`만 보낸다. 3단계에서 `orr_sim`에 `PlayerSlot`의 `Pod`·리플렉션과 `PlayerLink`를 더했고 골든 체크섬은 그대로다.
- **이름은 가칭 그대로 쓴다.** `.scene`은 설계의 `.scene(path)`와 달리 `register` 인자를 받는다. 게임의 리플렉션 등록 함수를 타입으로 찾을 방법이 없기 때문이다.
- **`ViewResync`가 오면** 예측 이벤트 기억을 비우고(이전 `Predicted`의 `Canceled`는 오지 않는다), 씬 뷰의 `on_resync`를 그 프레임의 이벤트보다 먼저 한 번 부른다. `Predicted`에서 시작한 효과는 거기서 멈추고 `ctx.predicted()`로 다시 만든다. 코얼레스된 세션 알림은 `on_session`으로 한 번씩 준다. 엔티티 뷰는 새 프레임과 비교해서 맞춘다.
- **첫 스냅숏보다 먼저 온 이벤트는** 첫 스냅숏이 올 때까지 쌓아 두었다가 `on_start` 다음에 전달한다.

## 남은 것

- **2단계에서 뺀 것:** `ViewContext::query`(`FrameView::iter::<T>()`, `dense::<T>()`가 이미 있어서 조합 질의는 `orr_ecs` 쪽 일이다), `ViewContext::draw()`(나중에 더했다. 아래 "글자, 선, 소리"), `App::threaded()`(위). `SceneView::on_player`는 4a단계에서 더했다.
- **3단계에서 뺀 것:** 서버 쪽 여러 슬롯 클라이언트(릴레이·네트워크는 슬롯 하나), `Threaded`를 쓰는 `App` 경로(위), 여러 슬롯의 `Step` 입력 캡처(`Threaded::try_step`은 첫 슬롯만 고정한다).
- **4b단계에서 뺀 것:** 플레이어별 시작 데이터(`Added`의 `data`는 서버로 실행해도 비어 있다. `Hello`에 시작 데이터가 없다. 8.3), 서버 쪽 신호 기록은 방이 사는 동안만 있다(서버를 다시 띄우면 사라진다), 시작 때의 `Added`(틱 1)는 `signal_lead_ticks` 여유가 없다, 권한 있는 서버 방은 신호가 없다, `.scene(..)`과 `.bot(..)`은 서버로 실행할 때 쓰지 않는다, `ws://`만 지원한다(QUIC·인증서 옵션 없음), 원격 뷰 스트림(`orr_viewstream`)은 신호를 아직 내보내지 않는다. 신호를 취소하는 API가 없어서 `Canceled`는 합성 업데이트 테스트로만 확인했다.
- **6a단계에서 뺀 것:** `Material`의 표면 셰이더(`Transform3`과 `Material`이 있는 엔티티는 단색 상자로 그린다), `SceneLook`의 후처리(블룸 등)와 하늘, 물, 지형, 흩뿌리기 그리기(조명과 데칼만 쓴다), 3D 마우스 입력(`InputContext`는 키만 읽는다. 예제 `yard3d`는 화면 가운데로 쏜다), 3D 뷰의 `ViewConfig` 바꾸기(기본값). 화면 글자 겹치기는 "글자, 선, 소리"에서 더했다.
- **6b단계에서 뺀 것:** 뷰 에셋의 `tint`(색 입히기), `sound`, `entity_view`는 읽기만 하고 쓰지 않는다. `App::entity_view_for("종류", factory)`(뷰 에셋의 `entity_view` 이름으로 엔티티 뷰를 붙이기)는 만들지 않았다. 지금은 `entity_view::<ViewRef>(..)`처럼 컴포넌트로 붙인다. 모델이 있는 그림은 MSAA와 톤 매핑이 없다(위). 에디터 뷰포트는 `ViewRef` 모델을 그린다. 다만 `Yard3D` 같은 기존 에디터 게임은 `ViewRef`를 등록하지 않아서(등록하면 그 게임의 프레임 배치와 골든 체크섬이 바뀐다) 에디터 전용 게임 `ViewYard3D`로 연다(`orr_editor --game view-yard3d --scene scenes/view_yard3d/view_yard3d.scene.yaml`, `docs/editor-view-ref.md`). 서버로 실행할 때 `Added.data`는 아직 비어 있어서(4b) 아바타는 슬롯 번호로 정한다. 애니메이션 모델(`SkinnedModelRenderer`)은 쓰지 않는다.
- **글자, 선, 소리에서 뺀 것:** 복합 문자 모양 잡기와 IME 입력(위 "글꼴과 글자 상자"의 "뺀 것"), 소리의 속도(재생 속도) 조절, WAV 말고 다른 소리 파일 형식(ogg, mp3), 서버로 실행하는 창끼리의 소리 맞춤(각 창이 자기 이벤트로 낸다), 웹과 모바일의 소리(`audio-native`는 CPAL 장치다).
- **이후 단계:** 웹·모바일 루프.
