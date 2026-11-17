# orr_app

`Game`을 창에 띄우는 실행 층이다. 설계는 [docs/view-framework.md](../../docs/view-framework.md) 4장이고, 이 크레이트는 12장의 1단계(서버 없이 실행, 기본 2D 렌더러)까지 만들었다.

## 쓰는 법

```rust
fn main() -> Result<(), orr_app::Error> {
    App::<PhysGame>::with_config(PhysConfig::new(1000, SceneMode::Rain))
        .tick_rate(TICK_RATE)                 // Game에는 틱 속도가 없어서 직접 준다 (기본 60)
        .players(2)                           // 시뮬레이션 플레이어 수 (기본 1). 로컬 플레이어는 0번
        .bot(move |_slot, tick| bot(tick).0)  // 로컬이 아닌 슬롯의 입력 (선택)
        .input(|keys, _slot| PhysInput::new(/* keys.axis(..), keys.held(..) */))
        .view(Look(PhysExtractor { .. }))     // 무엇을 어떻게 그릴지 (View2D)
        .run()
}
```

전체 예제: [examples/physics2d.rs](examples/physics2d.rs). `cargo run -p orr_app --release --example physics2d`

| 호출 | 뜻 |
|---|---|
| `App::new()` | `G::Config: Default`일 때 |
| `App::with_config(cfg)` | `G::setup(frame, &cfg)`로 시작 |
| `.scene(path, register)` | 씬 파일로 시작한다 (`G::setup`은 부르지 않는다). `register`는 게임의 `register_reflect` 같은 함수 |
| `.tick_rate(n)`, `.players(n)`, `.seed(n)`, `.title(s)` | 기본값은 60, 1, 42, "Orrery" |
| `.input(f)` | `f(&InputContext, PlayerSlot) -> G::Input`. 매 프레임 부른다. `InputContext`는 `held(key)`, `any_held(keys)`, `axis(neg, pos)`를 준다 |
| `.bot(f)` | 로컬이 아닌 슬롯마다 매 틱 `f(slot, tick)`을 부른다 |
| `.view(v)` | `v`는 `View2D`: `Extractor`(엔티티 읽기) + `camera(frame)` + `backdrop(frame, out)` |

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

- **브리지는 `InProc`이다.** 시뮬레이션이 그리기와 같은 스레드에서 돌아서 `--ticks`가 정확하고, `G::Config`에 `Send`를 요구하지 않는다. 물리 샘플의 `physics` 바이너리는 기본이 `Threaded`다. 무거운 장면에서 틱이 그리기를 막지 않게 하려면 이후 `Threaded` 선택을 더한다.
- **오프라인 세션은 `PlayHost` 위에 있다.** 롤백이 없고, 입력은 로컬 슬롯 하나다.
- **시뮬레이션 크레이트는 건드리지 않았다.** 화면은 `Snapshot`을 읽고 `set_input`만 보낸다.
- **이름은 가칭 그대로 쓴다.** `.scene`은 설계의 `.scene(path)`와 달리 `register` 인자를 받는다. 게임의 리플렉션 등록 함수를 타입으로 찾을 방법이 없기 때문이다.

## 2단계에 남은 것

`EntityView`, `SceneView`와 이벤트 전달(지금은 이벤트를 읽고 버린다), 3D 렌더러(`SceneLook`), `Threaded` 선택, 여러 로컬 플레이어(`Bridge::set_input` 확장), 서버 연결, 웹·모바일 루프.
