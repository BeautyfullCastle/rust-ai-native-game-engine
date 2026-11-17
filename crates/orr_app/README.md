# orr_app

`Game`을 창에 띄우는 실행 층이다. 설계는 [docs/view-framework.md](../../docs/view-framework.md) 4장이고, 이 크레이트는 12장의 1단계(서버 없이 실행, 기본 2D 렌더러), 2단계(엔티티 뷰, 씬 뷰, 이벤트 상태), 3단계(한 화면 여러 명)까지 만들었다.

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

- **장치 배정:** `KeyLayout::for_slot(slot)`이 슬롯 0에 `KeyLayout::WASD`(Space), 슬롯 1에 `KeyLayout::ARROWS`(Enter)를 준다. `stick_for_slot`은 그 키를 `Stick { x, y, action }`(`x`, `y`는 -1, 0, 1)으로 읽는다. 세 번째 슬롯부터는 기본 키가 없어서 항상 쉬는 입력이다. 직접 배정하려면 `KeyLayout { .. }`을 만들어 `stick(&layout)`에 준다. **지금은 키보드만 읽는다.** 패드는 `orr_app`이 아직 읽지 않는다.
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
| `.entity_view::<C>(factory)` | 예측 프레임에서 컴포넌트 `C`를 가진 엔티티마다 `factory()`로 만든 `EntityView` 하나 |
| `.scene_view(v)` | `SceneView` 하나. 여러 개를 더하면 더한 순서로 불린다 |

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
    fn on_session(&mut self, ctx: &mut ViewContext<'_, PhysGame>, note: Lifecycle) { .. }
    fn on_resync(&mut self, ctx: &mut ViewContext<'_, PhysGame>) { .. }
}
```

모든 메서드의 기본 몸체는 비어 있다. 한 프레임의 호출 순서는 `poll_view` → `on_start`(처음 한 번) → 세션 알림(`on_session`)과 이벤트(`on_event`) → `on_destroy` → `on_create` → 엔티티 `on_update` → 씬 `on_update` → 그리기다.

- **엔티티 키는 `(번호, 버전)`이다.** 번호가 재사용되어 버전이 달라지면 옛 엔티티는 `on_destroy`, 새 엔티티는 `on_create`다. 롤백으로 예측에만 있던 엔티티가 사라져도 `on_destroy`가 불린다. 같은 프레임 안에서 사라졌다 같은 키로 다시 생기는 경우(롤백 재계산 안)는 화면이 프레임 단위로만 보므로 구별되지 않는다.
- **`pose`는 `ViewWorld`가 계산한 `Transform2`이다.** `Extractor`가 그 엔티티를 내놓지 않으면 뷰 월드에 위치가 없어서 `None`이다. `on_update`는 그래도 불린다.
- **`EventState`는 `orr_session::EventStatus`에서 페이로드를 뺀 얇은 열거형이다.** `Predicted`, `Verified`, `Canceled`. `Canceled` 호출에는 `Predicted` 때 받은 페이로드가 그대로 온다(세션이 취소에는 페이로드를 주지 않아서 `Views`가 기억한다). 소리·파티클은 `Predicted`에서 시작하고 `Canceled`에서 멈춘다. 업적·통계는 `Verified`에서만 한다.
- **`ViewContext`:** `predicted()`, `verified()`(`Option`), `tick()`, `verified_tick()`, `local_players()`, `is_local(slot)`, `send_command(cmd)`. 오프라인에서 로컬 플레이어는 `.local_players(n)`의 슬롯 `0..n`이고, `local_players()`와 `is_local(slot)`이 모두 보고한다.
- **창 없이도 쓴다.** 호출 순서를 지키는 코드는 창과 렌더러가 없는 `Views::dispatch(bridge, update, poses)`에 있다. 창 루프는 이것을 부르기만 한다. 테스트는 `tests/views.rs`이고, 롤백이 나는 두 세션(`LoopbackPair`)에서 `on_create`/`on_destroy`와 `Canceled`/`Verified`를 확인한다.

## 명령줄

```text
--screenshot <png>  --frames 개수만큼 그린 뒤 그림 한 장을 저장하고 끝낸다
--frames <n>        그림 전에 그리는 프레임 수 (기본 30)
--ticks <n>         시뮬레이션을 n틱까지 돌리고 거기서 멈춘다
--size <WxH>        창 크기이자 그림 크기 (기본 900x900)
--no-vsync          화면 주사율을 기다리지 않는다
--help
```

- `--ticks`가 있으면 시계를 쓰지 않는다. 프레임마다 시뮬레이션은 최대 240틱씩 돌고, 화면은 프레임당 정확히 1틱 시간만큼 보간한다. 그래서 기계 속도와 관계없이 같은 그림이 나온다.
- 그림은 창과 따로 `--size` 크기의 화면 밖 텍스처에 다시 그려서 읽는다. 창 크기나 화면 배율에 영향을 받지 않는다.
- Esc 또는 창 닫기로 끝낸다.

## 지금 정한 것

- **브리지는 `InProc`이다.** 시뮬레이션이 그리기와 같은 스레드에서 돌아서 `--ticks`가 정확하고, `G::Config`에 `Send`를 요구하지 않는다. 물리 샘플의 `physics` 바이너리는 기본이 `Threaded`다. `App::threaded()`는 만들지 않았다. 오프라인 세션의 `PlayHost`가 `Send`가 아닌 봇 클로저를 들고 있고, `Threaded::spawn`은 호스트를 다른 스레드에서 만들어야 하기 때문이다. `Views::dispatch`는 `&mut dyn Bridge<G>`를 받으므로 `Threaded`에도 그대로 쓸 수 있다.
- **오프라인 세션은 `PlayHost` 위에 있다.** 롤백이 없고, 입력은 로컬 슬롯 `0..n`이다(`PlayHost::with_extra_local_slots`). 롤백이 나는 경우는 `LoopbackPair` 테스트로만 확인했다.
- **시뮬레이션 규칙은 건드리지 않았다.** 화면은 `Snapshot`을 읽고 `set_input`, `send_command`만 보낸다. 3단계에서 `orr_sim`에 `PlayerSlot`의 `Pod`·리플렉션과 `PlayerLink`를 더했고 골든 체크섬은 그대로다.
- **이름은 가칭 그대로 쓴다.** `.scene`은 설계의 `.scene(path)`와 달리 `register` 인자를 받는다. 게임의 리플렉션 등록 함수를 타입으로 찾을 방법이 없기 때문이다.
- **`ViewResync`가 오면** 예측 이벤트 기억을 비우고(이전 `Predicted`의 `Canceled`는 오지 않는다), 씬 뷰의 `on_resync`를 그 프레임의 이벤트보다 먼저 한 번 부른다. `Predicted`에서 시작한 효과는 거기서 멈추고 `ctx.predicted()`로 다시 만든다. 코얼레스된 세션 알림은 `on_session`으로 한 번씩 준다. 엔티티 뷰는 새 프레임과 비교해서 맞춘다.
- **첫 스냅숏보다 먼저 온 이벤트는** 첫 스냅숏이 올 때까지 쌓아 두었다가 `on_start` 다음에 전달한다.

## 남은 것

- **2단계에서 뺀 것:** `ViewContext::query`(`FrameView::iter::<T>()`, `dense::<T>()`가 이미 있어서 조합 질의는 `orr_ecs` 쪽 일이다), `ViewContext::draw()`(기본 2D 그리기가 `Extractor` 항목과 `backdrop`만 받아서 선·글자를 더하려면 그리기 경로를 넓혀야 한다), `SceneView::on_player`(4단계), `App::threaded()`(위).
- **3단계에서 뺀 것:** 패드 입력(키보드만), 서버 쪽 여러 슬롯 클라이언트(릴레이·네트워크는 슬롯 하나), `Threaded`를 쓰는 `App` 경로(위), 여러 슬롯의 `Step` 입력 캡처(`Threaded::try_step`은 첫 슬롯만 고정한다).
- **이후 단계:** 3D 렌더러(`SceneLook`), 서버 연결, 웹·모바일 루프.
