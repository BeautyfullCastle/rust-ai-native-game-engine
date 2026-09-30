# Orrery 엔진 — 설계 v1

작성: 2026-09-28 · 이전 버전: v0 (리서치 종합) · 이번 변경: 결정사항 반영, 결정론 멀티플레이/리플레이, 시뮬-뷰 분리와 브리지
리서치: 서브에이전트 7개 (Sonnet×6, Haiku×1)

> **이름: Orrery** — 태엽으로 행성 운행을 재현하는 기계식 천체 모형. "같은 입력이면 언제나 같은 결과"라는 엔진의 핵심(결정론)과, 기계(시뮬)와 보이는 모습(뷰)이 분리된 구조를 뜻한다. 크레이트 접두어 `orr_`.

---

## 0. 확정된 결정

| # | 항목 | 결정 |
|---|---|---|
| 1 | ECS | **직접 구현** |
| 2 | 셰이더 언어 | **WGSL로 시작**, 콘솔/고급 백엔드 착수 시 Slang 재평가 (§4.3 설명) |
| 3 | 씬/데이터 포맷 | **작성 포맷 = Strict YAML + 자동 생성 JSON Schema**, 실행 포맷 = 바이너리 (§6.3) |
| 4 | 에디터 뷰포트 | 시뮬-뷰 분리로 자연스럽게 해결: **에디터 = 뷰 레이어 호스트**, 시뮬은 브리지 너머 (§7.1) |
| 5 | 수치 | **고정소수점이 시뮬레이션의 기본이자 유일한 수치 타입.** f32는 뷰에서만 |
| 6 | 이름 | **Orrery** (`orr_`) |
| 7 | 멀티플레이 | **결정론 락스텝 + 예측/롤백.** 서버는 기본적으로 입력 릴레이, 각 클라가 시뮬. 서버 권위 모드 선택 가능 |
| 8 | 리플레이 | 입력 로그 기반, 전 모드 공통 |
| 9 | 구조 | **시뮬 레이어와 뷰 레이어 엄격 분리 + 브리지** |
| 10 | 시뮬 로직 반복 속도 | **함수 단위 핫패치** (subsecond 방식). 시뮬 로직은 Rust만. 시스템 호출은 개발 빌드에서 핫패치 가능한 간접 호출을 거친다. 패치가 적용되면 빌드 해시가 바뀐다. 멀티플레이에서는 모든 피어의 빌드 해시가 같아야 하고, 리플레이에는 빌드 해시가 기록된다. 패치 후에는 기준 리플레이를 다시 돌려 검증한다 |
| 11 | 외부 AI 에이전트 연결 | **CLI `orr`가 주 경로**(셸 에이전트가 Bash로 호출, `--json` 출력, 컨텍스트 비용 없음), MCP 어댑터 `orr_mcp`는 셸 없는 클라이언트용 보조. 둘 다 ERP의 얇은 클라이언트. **에디터는 수락 절차를 두지 않는다**: 사람이 시킨 일을 에이전트가 끝까지 하고(필요하면 제안 → 리플레이 검증 → 조건 통과 시 스스로 수락), 에디터 Agent 탭은 활동 피드(누가 무엇을, diff, 검증 리포트, 뷰포트 강조)로 보여만 준다. 되돌리기는 undo (2026-09-30) |
| 12 | 뷰 교체 가능성 (언어 중립 뷰 경계) | **뷰는 Rust 렌더러·에디터가 아니어도 된다.** ① 에디터도 브리지 너머의 시뮬만 본다: 시뮬·편집 문서는 시뮬 쪽 호스트(스레드/프로세스)에 있고 에디터는 `Bridge`+`SimControl`+스냅샷만 쓴다(에디터가 `orr_session`·게임 크레이트에 직접 의존하지 않음). ② 시뮬→뷰는 Frame이 아니라 **뷰 스트림**: 틱마다 안정 ID·종류/스타일·이전/현재 트랜스폼·게임 정의 속성, 이벤트 3상태, 생성/파괴. 레이아웃은 `orr_reflect` 스키마로 기술. 뷰→시뮬은 Input/Command 바이트뿐. ③ 전송: C ABI 동적 라이브러리 `orr_ffi`(같은 프로세스: Unity P/Invoke, Unreal C++, Godot GDExtension, 게임기 네이티브) + 같은 메시지의 소켓 전송(다른 프로세스·기기). ④ 증명: 뷰 스트림만 읽는 터미널 뷰어 `orr_tui`와 C 테스트 프로그램 (2026-09-30) |

---

## 1. 설계 원칙

1. **시뮬은 순수 함수다** — `Frame(t+1) = step(Frame(t), Inputs(t), Commands(t))`. 이 한 줄이 멀티플레이, 롤백, 리플레이, AI 검증, 버그 재현을 전부 떠받친다.
2. **시뮬은 아무것도 보지 않는다** — 렌더링, 오디오, 벽시계 시간, OS 스레드 순서, float, HashMap 순회 순서를 시뮬은 모른다.
3. **뷰는 시뮬을 읽기만 한다** — 뷰가 시뮬에 영향을 주는 유일한 길은 Input과 Command.
4. **Frame은 memcpy 가능해야 한다** — 스냅샷/복원이 싸야 롤백이 가능하다. 이게 ECS 저장소 설계를 결정한다.
5. **성능 예산은 롤백 기준** — 틱 1회 비용 × 최대 롤백 깊이가 렌더 1프레임 안에 들어가야 한다.
6. **에이전트는 1급 사용자** — 사람이 할 수 있는 건 전부 원격 프로토콜로 가능하고, 모든 변경은 기록되며 되돌릴 수 있다.

---

## 2. 전체 구조

```
          ┌──────────── VIEW LAYER (f32, 비결정론 OK) ──────────────┐
          │ EntityView 매핑 · 보간 · 렌더 · 오디오 · VFX · UI · 입력 수집 │
          │ Editor UI · 디버그 오버레이                               │
          └───────▲──────────────────────────────┬──────────────────┘
       FrameView(읽기전용)                     Input / Command
       Events(Predicted/Verified/Canceled)       │
       Lifecycle 콜백                            │
          ┌───────┴────────────── BRIDGE ─────────▼──────────────────┐
          │ 전송 교체 가능: 같은 스레드 · 다른 스레드 · 다른 프로세스 · 네트워크 │
          └───────▲──────────────────────────────┬──────────────────┘
          ┌───────┴──────────── SESSION RUNNER ──▼──────────────────┐
          │ 틱 클럭 · 입력 버퍼 · 예측 · 롤백 · 스냅샷 링 · 체크섬 · 리플레이 │
          │ 모드: Local / Client / Server(Relay·Validate·Auth) / Replay │
          └───────▲──────────────────────────────┬──────────────────┘
          ┌───────┴─────────── SIM LAYER (고정소수점, 결정론) ─────────┐
          │ Frame(ECS 월드) · 시스템 · FP 수학 · 결정론 물리 · RNG · 내비 │
          └────────────────────────────────────────────────────────┘
                        │ 네트워크 (입력만 오감)
                  ┌─────▼──────┐
                  │ Relay 서버 │ (+선택: 헤드리스 시뮬 = 권위 모드)
                  └────────────┘
```

### 크레이트 구성

| 레이어 | 크레이트 | 역할 |
|---|---|---|
| 공용 | `orr_fp` | 고정소수점 스칼라/벡터/쿼터니언/행렬, CORDIC·LUT 삼각함수 |
| 공용 | `orr_ecs` | ECS 저장소, 쿼리, 커맨드 버퍼, 관계, 변경 감지 (Frame 친화 설계) |
| 공용 | `orr_reflect` | 타입 레지스트리, JSON Schema 생성, 직렬화 |
| 시뮬 | `orr_sim` | Frame, 스케줄러(결정론), 결정론 RNG, 시스템 API. **float 금지 lint** |
| 시뮬 | `orr_physics` | 결정론 고정소수점 물리 (2D 먼저, 3D 다음) |
| 시뮬 | `orr_nav` | 고정소수점 내비메시/경로탐색 |
| 세션 | `orr_session` | 틱 클럭, 예측/롤백, 스냅샷 링, 체크섬, 리플레이 기록/재생 |
| 세션 | `orr_net` | 전송(quinn/QUIC=네이티브, WebTransport=브라우저, WebSocket=UDP 차단 시 대체), 시간 동기화 |
| 세션 | `orr_server` | 릴레이 서버 바이너리 + 선택적 헤드리스 시뮬 |
| 브리지 | `orr_bridge` | FrameView, 이벤트 채널, Input/Command 전송, 전송 어댑터 |
| 뷰 | `orr_view` | EntityView 매핑, 보간, 예측 오차 보정, 이벤트 디스패치 |
| 뷰 | `orr_rhi`, `orr_render` | RHI trait + wgpu 구현, 렌더 그래프, Forward+ |
| 뷰 | `orr_audio`, `orr_ui`, `orr_input` | kira, taffy+cosmic-text, 액션 매핑 |
| 공용 | `orr_asset` | GUID 에셋 DB, 쿠커, 스트리밍 (시뮬 에셋 / 뷰 에셋 구분) |
| 도구 | `orr_remote` | ERP(원격 프로토콜) 서버 |
| 도구 | `orr_editor`, `orr_cli`(`orr`), `orr_mcp` | 에디터 바이너리, 에이전트·사람용 CLI(주 경로), MCP 어댑터(보조) |

**의존성 규칙 (CI에서 강제)**: `orr_sim` 이하는 `orr_view`/`orr_render`/`wgpu`/`std::time`/float 수학에 의존 불가. 시뮬 크레이트는 `no_std + alloc`로 빌드 가능해야 한다 (플랫폼 의존 제거 증거).

---

## 3. 시뮬 레이어

### 3.1 수치: 고정소수점 `FP`

| 항목 | 결정 |
|---|---|
| 기본 타입 | `FP` = **Q48.16** (i64, 소수 16비트, 정밀도 1/65536). Photon Quantum과 동일 포맷, 실전 검증됨 |
| 곱셈 | 빠른 경로: i64 곱 후 `>>16` (값이 대략 ±4.6만 이내일 때 안전, 디버그 빌드에서 오버플로 검사) / 넓은 경로: `mul_wide`(i128) |
| 보조 타입 | `FP32` = Q16.16 (i32) — 핫패스(대량 유닛 이동 등)에서 선택. i64 중간값만 쓰므로 wasm/32bit ARM에서도 빠름 |
| 삼각/제곱근 | 자체 구현: sin/cos/atan2는 **LUT + 선형 보간**(속도), sqrt는 정수 뉴턴법, 정확도 검증용으로 `cordic` 크레이트와 비교 테스트 |
| 기반 크레이트 | 스칼라 연산은 `fixed` 크레이트 참고/활용, **벡터·쿼터니언·행렬은 자체 구현** (Rust에 쓸 만한 고정소수점 선형대수 크레이트 없음) |
| 리터럴 | `fp!(1.5)` 매크로 — 컴파일 타임에 정확히 변환. 데이터 파일의 `1.5`도 파서가 결정론적으로 변환 |
| SIMD | 정수 SIMD(i32x4/i64x2)는 플랫폼 간 결과 동일 → `wide` 또는 `std::simd`로 배치 연산 |

**wasm 주의**: wasm32에는 네이티브 i128이 없어 `mul_wide`가 느리다. M0에서 벤치마크하고, 빠른 경로가 대부분을 처리하도록 설계.

### 3.2 결정론 규칙 (lint + CI로 강제)

- 시뮬 크레이트에서 `f32`/`f64` 금지 (clippy `disallowed_types`).
- `HashMap`/`HashSet` 금지 → `BTreeMap`, 정렬된 `Vec`, 또는 고정 시드 해셔 래퍼.
- `usize`를 상태/직렬화에 사용 금지 (wasm32는 32비트) → `u32`/`u64` 명시.
- 벽시계, OS RNG, 스레드 ID, 포인터 주소 사용 금지.
- 병렬 시스템 실행 허용: 정수 연산은 순서와 무관하게 결과가 같다. 단, 엔티티 생성/삭제는 커맨드 버퍼를 **(시스템 순서, 엔티티 ID)로 정렬해** 적용.
- **교차 플랫폼 체크섬 CI**: 동일 리플레이를 Linux x64 / Windows x64 / macOS ARM64 / Android ARM64 / wasm32에서 돌려 N틱마다 Frame 해시 비교. 한 비트라도 다르면 실패.

### 3.3 Frame과 ECS 저장소 (롤백 친화)

롤백은 틱마다 Frame을 저장하고 되돌리는 일이다. 그러려면 Frame은 **포인터 없는 연속 메모리**여야 한다.

- **Frame = 몇 개의 큰 아레나**. 모든 시뮬 컴포넌트, 리소스, 엔티티 테이블, RNG 상태, 틱 번호가 그 안에 있다.
- **시뮬 컴포넌트는 POD**: `Copy + #[repr(C)]`, 힙 포인터 금지 (`Vec`, `String`, `Box` 불가). 가변 길이 데이터는 **Frame 힙**(Frame 내부 할당자)의 `FrameList<T>` / `FrameMap<K,V>` 핸들로.
- 에셋 참조는 `AssetRef`(GUID 기반 u64). 시뮬 에셋(스탯, 충돌 형상, 내비메시)은 불변이라 Frame에 복사하지 않는다.
- 저장소: 아키타입 테이블(SoA) 기본 + 컴포넌트별 스파스셋 선택 (v0와 같음). 단, 테이블 메모리가 Frame 아레나 안에 있다.
- **엔티티 ID 결정론**: `EntityRef = (index u32, version u32)`. Frame 내부 free-list로 할당하므로 같은 입력이면 모든 클라이언트에서 같은 ID. 롤백하면 ID 할당 상태도 함께 복원된다.
- **스냅샷** = 아레나 memcpy. 링 버퍼로 최근 N틱 보관. v1에서는 더티 페이지 추적으로 차분 스냅샷(copy-on-write)을 추가.
- 관계 `(Relation, Target)`, 변경 감지 틱도 모두 Frame 안에 있다.

### 3.4 스케줄러 (시뮬)

- 시스템 순서는 등록 순서와 명시적 제약으로 **고정**. 접근 집합이 겹치지 않는 시스템은 병렬로 실행하지만, 결과는 순차 실행과 비트 단위로 같다.
- 실행기는 rayon으로 시작. 브라우저에서 스레드를 못 쓰는 환경을 위해 단일 스레드 실행기도 정식 지원.
- **틱 예산**: 목표 60Hz, 최대 롤백 8틱 → 틱 1회 ≤ 약 1ms (목표 엔티티 수 기준). 매 CI 벤치마크에서 확인.
- **목표 엔티티 수**: 물리 바디 1,000개 (2026-09-29 확정). 이보다 많은 게임은 예산을 보장하지 않는다.

### 3.5 결정론 물리 `orr_physics`

Rust에는 고정소수점 물리 엔진이 없다 (Rapier는 float이고 `enhanced-determinism`은 IEEE 준수를 믿는 방식이라 우리 기준에 맞지 않음). **자체 구현**한다.

- **2D (M2)**: 원/AABB/OBB/폴리곤/캡슐, 동적 AABB 트리 또는 균일 그리드 broad-phase, SAT/GJK narrow-phase, 순차 임펄스 솔버, 트리거, 레이/셰이프 캐스트, 키네마틱 캐릭터 컨트롤러.
- **3D (M5 이후)**: 같은 구조로 확장. 알고리즘은 Rapier·Box2D·Jolt 설계를 참고.
- 물리 상태 역시 Frame 안에 있어 롤백 시 함께 되돌아간다.

---

## 4. 뷰 레이어

### 4.1 EntityView와 보간

- 시뮬 엔티티와 뷰 오브젝트를 연결하는 `EntityView` 컴포넌트는 뷰 월드에 있다. 뷰 월드도 ECS지만 **별개의 월드**이고 f32를 쓴다.
- 보간 모드 (엔티티마다 선택):
  - **Prediction**: 예측 Frame 두 개 사이를 보간하고, 롤백으로 위치가 튀면 몇 프레임에 걸쳐 오차를 부드럽게 보정. 내 캐릭터용.
  - **Snapshot**: 확정(verified) Frame 버퍼를 약간 늦게 재생. 틀린 모습이 안 보인다. 원격 엔티티/관전용.
  - **None**: 틱 단위로 즉시 반영.
- 고정소수점 → f32 변환은 뷰 쪽에서만 일어난다.

### 4.2 렌더링 · 플랫폼 (v0와 같음)

- wgpu + 얇은 `orr_rhi` trait (콘솔 백엔드 자리). Bevy식 extract → 렌더 월드 → 렌더 그래프, 파이프라인드 렌더 스레드.
- Clustered Forward+ 기본, 비저빌리티 버퍼 + 메시렛은 고사양 opt-in.
- winit + gilrs. 모바일 suspend/resume, Web(COOP/COEP) 대응. 빌드는 cargo-mobile2, trunk.

### 4.3 셰이더 언어: WGSL vs Slang (결정 2 설명)

셰이더는 GPU에서 도는 작은 프로그램이다 (픽셀 색 계산, 정점 변환 등). 어떤 언어로 쓰느냐의 문제다.

| | WGSL | Slang |
|---|---|---|
| 정체 | WebGPU 표준 셰이더 언어. wgpu가 기본으로 받는다 | NVIDIA가 시작해 Khronos가 관리하는 범용 셰이더 언어. HLSL과 비슷 |
| 장점 | 추가 툴체인 없음, 웹 그대로, Rust 생태계(naga, naga_oil)와 한 몸 | 한 소스로 SPIR-V/DXIL/Metal/WGSL 출력, 모듈·제네릭·인터페이스 등 언어 기능이 강력함, 콘솔/네이티브 백엔드로 갈 때 유리 |
| 단점 | 언어 기능이 빈약함 (제네릭 없음), 콘솔용 변환 경로가 약함 | 외부 C++ 컴파일러 의존, 빌드 복잡도 증가, Rust 통합 사례가 적음 |

**결정**: 지금은 wgpu 하나로 모든 플랫폼을 커버하니 **WGSL + naga_oil**로 시작한다. 콘솔이나 네이티브 고급 경로(메시 셰이더, 레이트레이싱)에 착수할 때 Slang으로 옮길지 다시 평가한다. 셰이더 조합 시스템은 추상화해 두어 교체 비용을 낮춘다.

---

## 5. 브리지 `orr_bridge` — 시뮬과 뷰의 유일한 통로

### 5.1 뷰 → 시뮬 (쓰기 경로는 이 둘뿐)

| 종류 | 특성 | 예 |
|---|---|---|
| **Input** | 매 틱, 플레이어당 고정 크기 POD (비트 플래그 + FP 축), 예측 가능 (못 받으면 직전 입력 반복) | 이동, 조준, 버튼 |
| **Command** | 가끔, 가변 크기, 신뢰 전송, 예측하지 않음, 확정된 틱에 적용 | 구매, 건설, 스킬 선택, 채팅, 디버그/에디터 명령 |

```rust
bridge.set_input(player, MyInput { move_x: fp!(1), buttons: FIRE });
bridge.send_command(BuildTower { cell, kind });
```

입력 타입과 커맨드 타입은 게임이 정의하고 `#[derive(SimInput)]` / `#[derive(SimCommand)]`로 직렬화, 스키마, 델타 압축 코드를 자동 생성한다.

### 5.2 시뮬 → 뷰

| 채널 | 설명 |
|---|---|
| **FrameView** | 읽기 전용 Frame 접근: `predicted()`, `predicted_prev()`(보간용), `verified()`. 뷰는 시뮬 컴포넌트를 쿼리할 수 있지만 수정할 수 없다 |
| **Events** | 시뮬 시스템이 `frame.emit(Hit{..})`로 발생. 뷰에 도착하는 상태는 세 가지: **Predicted**(예측 틱에서 발생), **Verified**(확정됨), **Canceled**(롤백으로 무효). 이벤트마다 결정론적 키(틱, 발생 시스템, 순번)가 있어 롤백 뒤 같은 이벤트가 다시 나오면 중복 없이 매칭 |
| **이벤트 정책** | 이벤트 타입마다 지정: `on_predicted`(즉시 반응, 취소 가능해야 함. 사운드·이펙트) / `on_verified`(되돌리면 안 되는 것. 보상 팝업, 업적, 분석 로그) |
| **Lifecycle** | 엔티티 생성/파괴, 세션 시작/종료, 롤백 발생(틱 범위), 디싱크 감지 |

### 5.3 전송 어댑터 (같은 API, 다른 거리)

| 어댑터 | 쓰임 |
|---|---|
| `InProc` | 같은 스레드. 테스트, 모바일/웹 단일 스레드 |
| `Threaded` (기본) | 시뮬 스레드가 Frame을 확정할 때마다 불변 스냅샷을 발행(트리플 버퍼 / `ArcSwap`). 뷰는 잠금 없이 최신 것을 읽음. Input/Command는 lock-free 큐 |
| `Remote` | 다른 프로세스·기기. ERP 위에서 Frame 델타 스트림 + 이벤트 스트림. **에디터, 원격 디버깅, AI 에이전트 관찰**에 사용 |

뷰 코드는 어댑터를 몰라도 된다. 이 구조 덕분에 에디터 연결, 헤드리스 서버, 리플레이 뷰어, AI 관찰자가 모두 **같은 뷰 API를 쓰는 서로 다른 소비자**가 된다.

---

## 6. 세션: 멀티플레이 · 리플레이

### 6.1 세션 모드

| 모드 | 시뮬 위치 | 설명 |
|---|---|---|
| **Local** | 클라 | 싱글플레이. 입력 소스가 로컬뿐 |
| **Relay (기본)** | 각 클라 | 서버는 입력을 모아 틱 번호를 붙여 재방송만 한다. 늦은 입력은 서버가 "직전 입력 반복"으로 확정. 서버 비용 최소 |
| **Relay + Validate** | 각 클라 | 서버가 입력 범위, 빈도, 커맨드 합법성을 검사. 시뮬은 하지 않음 |
| **Authoritative** | 각 클라 + 서버 | 서버도 같은 `orr_sim`을 헤드리스로 돌린다. 체크섬 심판(디싱크 시 서버 상태가 정답), 늦은 참가자용 스냅샷 제공, 서버 전용 커맨드(AI 플레이어, 이벤트), 치트 판정 |
| **Replay** | 클라/헤드리스 | 입력 소스가 파일. 탐색, 배속, 되감기 가능 |

같은 `orr_sim` 바이너리 코드가 클라, 서버, 리플레이, CI에서 그대로 돈다.

브라우저 클라는 릴레이 서버에 WebTransport(HTTP/3 위 QUIC, 데이터그램 + 스트림)로 붙는다. UDP가 막힌 환경에서는 WebSocket으로 대체한다(이때 비신뢰 채널도 TCP라 입력 지연을 더 크게 잡는다). 서버가 항상 가운데 있는 구조라 피어 간 연결용 WebRTC는 쓰지 않는다 (2026-09-30 결정).

### 6.2 예측 · 롤백 흐름

```
verified 틱 V ─────────── predicted 틱 P (= V + 최대 N)
  확정 입력 도착 → 예측과 비교
     같으면: verified 전진, 그 Frame 스냅샷 삭제
     다르면: Frame(V) 복원 → 확정 입력으로 V..P 재시뮬 → 뷰에 Rollback(V..P) 알림
```

- **입력 지연**: 기본 2~3틱 (Relay 모드는 RTT에 따라 자동 조정). 예측 한도 기본 8틱 (@60Hz, 약 133ms). 이를 넘으면 시뮬이 잠시 멈춰 기다린다.
- **시간 동기화**: 서버 틱이 기준. 클라는 입력 버퍼 여유분을 보고 시뮬 속도를 ±2% 범위에서 미세 조정 (시계 드리프트 흡수).
- **디싱크 감지**: N틱(기본 30)마다 확정 Frame 체크섬(xxh3) 교환. 불일치 시 진단 덤프(마지막 스냅샷 + 입력). Authoritative 모드에서는 서버 스냅샷으로 재동기화.
- **늦은 참가**: Relay 모드에서는 다른 클라가 확정 스냅샷을 업로드해 서버가 중계한다. Authoritative 모드에서는 서버가 직접 제공한다.
- **대역폭**: 입력은 작고 델타 압축된다. 예: 8명 × 60Hz × 약 4B ≈ 2KB/s 수준.

### 6.3 리플레이 파일 `.orrp`

```
Header    : 엔진·게임 빌드 해시, 시뮬 에셋 DB 해시, 세션 설정, 시드, 플레이어 목록
Inputs    : 틱별 확정 입력 스트림 (델타 + zstd)
Commands  : (틱, 플레이어, 커맨드) 목록
Snapshots : K틱마다 전체 Frame (탐색/되감기용, 선택)
Checksums : N틱마다 해시 (검증용)
Markers   : 이벤트 북마크 (킬, 라운드 등, 선택)
```

용도: 경기 다시보기, 관전, **버그 재현 (QA가 리플레이 파일 하나만 첨부하면 됨)**, 치트 검증(헤드리스로 다시 돌려 체크섬 비교), 교차 플랫폼 결정론 CI, **AI 에이전트 검증** (변경 전후 리플레이 결과 비교).

---

## 7. 에디터

### 7.1 뷰포트란 (결정 4 설명)

**뷰포트** = 에디터 한가운데에 있는, 게임 세계가 보이는 창이다. 여기서 카메라를 돌려 보고, 오브젝트를 클릭해 선택하고, 기즈모(화살표 핸들)로 옮긴다. Unity의 Scene 뷰가 이것이다.

v0에서 물었던 것은 "뷰포트 그림을 누가 그리느냐"였다. 시뮬-뷰 분리 덕분에 답이 정해진다.

- 에디터 프로세스가 **뷰 레이어를 직접 호스트**한다 (렌더러, 보간, 기즈모).
- 시뮬은 브리지 너머에 있다. 편집 모드에서는 에디터 안 시뮬 스레드(`Threaded`), 플레이 테스트에서는 별도 게임 프로세스나 원격 기기(`Remote`).
- 결과: 게임이 크래시해도 에디터는 살아 있고, 모바일 기기에서 도는 게임에 에디터 뷰포트를 붙여 볼 수 있다.

### 7.2 편집 모드 vs 플레이 모드

- **편집 모드**: 씬 파일(YAML)을 편집한다. 모든 편집은 트랜잭션으로 기록되고 undo 가능하다. 저장 시 텍스트 diff가 깔끔하다.
- **플레이 모드**: 씬을 Frame으로 구워 세션을 시작한다. 플레이 중 인스펙터 수정은 **디버그 Command**로 시뮬에 들어가고 리플레이에 기록된다 (결정론 유지).
- **타임라인 스크러버**: 플레이 모드에서 스냅샷 링과 리플레이를 이용해 **되감기, 틱 단위 전진, 특정 틱부터 다시 실행**. 결정론 엔진이라 가능한 기능이다.

### 7.3 기본 레이아웃

```
┌──────────────────────────────────────────────────────────────────────┐
│ File Edit View Sim AI Window  [▶][⏸][⏭틱] Net:[Local▾] [Workspace▾] Ctrl+K │
├──────────────┬─────────────────────────────────────┬─────────────────┤
│ HIERARCHY    │                                     │ INSPECTOR       │
│ [공간|목록|관계] │          VIEWPORT                   │ Entity 42 v3    │
│ ▸ World      │   기즈모 · 그리드 · 카메라               │ ├ Transform(FP) │
│   ▸ Towers   │   오버레이: 충돌체, 내비, 예측오차         │ ├ Health        │
│   ▸ Enemies  │                                     │ [+ Component]   │
│ ▸ Resources  │                                     │ Archetype 17    │
├──────────────┴─────────────────────────────────────┴─────────────────┤
│ TIMELINE  |◀ ◀◀ ▶ ▶▶| tick 1832 ─────●──────────── (verified 1829 / predicted 1832) │
├───────────────────────────┬──────────────────────┬───────────────────┤
│ ASSETS│CONSOLE│PROFILER│NET │ AI AGENT              │ QUERY/SCHEDULE     │
│                           │ > 웨이브 5 난이도 올려    │ 시스템 순서/시간      │
│                           │ ◇ ~ wave5.yaml (+2 적) │ 롤백 횟수/재시뮬 비용 │
│                           │ ✓ verify p2 2/2 (300틱) │                  │
└───────────────────────────┴──────────────────────┴───────────────────┘
```

- **NET 패널**: RTT, 입력 지연, 롤백 빈도/깊이, 재시뮬 시간, 디싱크 로그.
- **Workspaces**: Level / Gameplay / Netcode Debug / AI / Profiling.
- **기능 우선순위**: v0의 MVP/v1/Later 목록을 유지하고 다음을 추가한다 — MVP: 틱 스텝, 체크섬 표시. v1: 타임라인 스크러버, NET 패널, 리플레이 뷰어, 2클라 로컬 시뮬레이션(가짜 지연/손실 주입).

---

## 8. AI-Native (v0에서 변경 없음 + 결정론 활용)

- **ERP (Engine Remote Protocol)**: world/schema/watch/tx/sim/verify/asset/script 메서드 그룹 (v0 §6.1). 권한 토큰.
- **결정론 덕분에 가능한 것**:
  - 에이전트가 수정한 뒤 **기준 리플레이를 헤드리스로 다시 돌려** 결과(체크섬, 메트릭, 스크린샷)를 비교하는 자동 검증.
  - 에이전트가 만든 버그를 리플레이 한 파일로 재현.
  - 밸런스 튜닝: 같은 입력 시퀀스를 파라미터만 바꿔 대량 헤드리스 실행.
  - RL/봇: 봇은 그냥 Input을 생산하는 또 하나의 플레이어.
- 에이전트 연결은 CLI `orr`가 주 경로, MCP는 엔진 밖 별도 어댑터 프로세스(셸 없는 클라이언트용). 에디터의 AI AGENT 영역은 승인 창이 아니라 활동 피드다(결정 11). Luau 스크립팅은 **뷰/툴 쪽 전용**이다. 시뮬 로직은 Rust라서 결정론을 보장할 수 있다. 시뮬 스크립팅이 필요하면 WASM(정수 전용 + 결정론 설정)을 v2에서 검토한다.
- 엔진 버전에 고정된 `AGENTS.md` / `llms.txt` 자동 생성.

### 데이터 포맷 (결정 3)

요구: 사람이 편집하기 편할 것 + AI가 읽기 편할 것.

| 후보 | 사람 편집 | AI 친숙도 | 비고 |
|---|---|---|---|
| JSON | 보통 (주석 불가, 따옴표 많음) | 최상 | |
| **YAML** | **좋음 (주석, 들여쓰기)** | **최상** | 함정 있음 (`no`→false 같은 암묵 변환, 앵커) |
| RON | 보통 | 낮음 | Rust 전용, LLM 학습량 적음 |
| TOML | 깊은 중첩에서 불편 | 좋음 | 씬 트리에 부적합 |

**결정: Strict YAML** — YAML의 문제 기능을 막은 부분집합.
- 금지: 앵커/별칭, 태그, 암묵적 타입 변환 (타입은 JSON Schema로 결정).
- 엔티티마다 안정 GUID 키, 키 정렬 저장 → diff와 머지가 깔끔하다.
- `orr_reflect`에서 **JSON Schema 자동 생성** → 에디터 자동완성·검증, LLM에게 스키마 제공, 저장 전 검증.
- 고정소수점 값은 십진수 그대로 (`speed: 1.5`). 로더가 결정론적으로 FP로 변환한다.
- 실행 시에는 쿠커가 바이너리로 굽는다. YAML 파싱 속도는 런타임 성능과 무관하다.

```yaml
# scenes/wave5.scene.yaml
schema: orr.scene/1
entities:
  e_7f3a91c2:
    name: Spawner_North
    Transform: { pos: [12.5, 0, -4], rot: 90 }
    WaveSpawner:
      enemy: asset://enemies/grunt
      count: 12
      interval: 0.75
```

---

## 9. 서브시스템 (v0 대비 변경)

| 영역 | 결정 |
|---|---|
| 물리 | **자체 결정론 FP 물리** (Rapier는 시뮬에서 사용 불가) |
| 내비 | **자체 FP 내비메시/그리드 A\*·플로우필드** (oxidized_navigation은 float라 뷰 전용 참고) |
| 네트워크 | 자체 세션 + 전송(quinn/QUIC, WebTransport, WebSocket). GGRS/fortress-rollback은 설계 참고 |
| 애니메이션 | 뷰 전용 (게임플레이에 영향 주는 타이밍은 시뮬에 FP로 따로 둔다) |
| 오디오/UI/입력 | 뷰 전용: kira, taffy+cosmic-text, 액션 매핑 → Input 구조체로 변환 |
| 스크립팅 | Luau = 툴/뷰, 시뮬 스크립팅은 v2에서 검토 |
| 프로파일링 | tracing + Tracy. 시뮬 틱/재시뮬 구간 별도 표시 |

---

## 10. 로드맵 (갱신)

| 단계 | 산출물 | 완료 조건 |
|---|---|---|
| **M0 기반** | `orr_fp`, `orr_ecs`(Frame 아레나), `orr_sim` 스케줄러, 결정론 lint | 1만 엔티티 틱 < 0.5ms, 스냅샷 memcpy 측정, 5개 플랫폼 체크섬 CI 통과 |
| **M1 세션** | 스냅샷 링, 롤백, Input/Command, 리플레이 기록/재생, Local/Replay 모드 | 가짜 지연 주입 로컬 2클라이언트에서 롤백 동작, 리플레이 비트 단위 일치 |
| **M2 브리지·뷰·물리2D** | `orr_bridge`(InProc/Threaded), `orr_view` 보간, 이벤트 3상태, wgpu 렌더 기초, FP 물리 2D | 샘플 게임이 60fps로 돌고 롤백 중 시각적 튐 없음 |
| **M3 네트워크** | Relay 서버, QUIC(네이티브) / WebTransport(브라우저) / WebSocket 대체, 시간 동기화, 디싱크 감지, 늦은 참가. WebRTC는 계획하지 않음 — WebTransport가 특정 대상에서 실패할 때만 재검토 | 4인 원격 플레이 150ms RTT에서 플레이 가능 |
| **M4 에디터 MVP** | ERP + `Remote` 어댑터, egui 에디터, YAML 씬, 타임라인, undo | 씬 편집 → 플레이 → 되감기 루프 |
| **M5 AI** | MCP 어댑터, 스키마 제공, 리플레이 기반 자동 검증, Agent 패널 | 에이전트가 수정 → 검증 → 수락까지 한 번에 |
| **M6 확장** | Authoritative 서버, FP 물리 3D, 고급 렌더, 모바일/웹 최적화 | |

---

## 11. 리스크 · 열린 질문

- **자체 FP 물리 비용**이 가장 큰 작업이다. 2D부터 범위를 좁혀 시작한다.
- **Q48.16 곱셈의 wasm 성능**은 M0에서 반드시 측정한다.
- ~~시뮬 로직 반복 속도~~ → 함수 핫패치로 결정 (#10). 남은 리스크: subsecond는 WASM 미지원이고 바이너리 크레이트에서만 동작 → 핫패치는 데스크톱 개발 환경 전용.
- M0 ECS 저장소는 스파스셋으로 구현 (구조 변경이 싸서 재시뮬에 유리). 아키타입 테이블은 벤치 결과를 보고 추가 여부 결정. 현재 1M 엔티티 순회 1.9ms (순수 Vec 1.2ms).
- 브리지 `Remote` 어댑터의 Frame 델타 스트리밍 대역폭 (큰 월드에서 에디터 연결 시).
- Quantum의 정확한 기본값(HardTolerance, 예측 한도)은 공개 문서에 없어 자체 튜닝이 필요하다.
- **WebTransport 브라우저 호환**: Safari(26.4+)는 옛 WebTransport 방식을 써서 일부 Rust `h3` 서버와 연결이 안 되는 사례가 있다(hyperium/h3 #347). Safari는 `serverCertificateHashes`를 지원하지 않아 개발 중에도 정식 CA 인증서가 필요하고, Firefox도 로컬 인증서가 실패할 수 있다. 자체 서명 해시 방식은 ECDSA P-256·유효기간 14일 이하 조건이 있다. 기존 QUIC(`orrery/1`)과 WebTransport(`h3`)가 한 UDP 포트를 같이 쓸 수 있는지 미검증 — 본 구현 전 시험 구현으로 확인한다.

## 참고 출처 (v1 추가분, v0 출처는 이전 문서 참조)

- Photon Quantum — Frames https://doc.photonengine.com/quantum/current/manual/frames · Commands https://doc.photonengine.com/quantum/current/manual/commands · Replay https://doc.photonengine.com/quantum/current/manual/replay · EntityView https://doc.photonengine.com/quantum/current/manual/entityview · Fixed Point https://doc.photonengine.com/quantum/current/manual/quantum-ecs/fixed-point · Server API https://doc.photonengine.com/quantum/v3/addons/plugin-sdk/quantum-server-api
- bevy_ggrs architecture https://github.com/gschup/bevy_ggrs/blob/main/docs/architecture.md · ggrs https://github.com/gschup/ggrs
- SnapNet — Rollback https://www.snapnet.dev/blog/netcode-architectures-part-2-rollback/
- 1500 Archers (AoE) https://www.gamedeveloper.com/programming/1500-archers-on-a-28-8-network-programming-in-age-of-empires-and-beyond
- Riot — Determinism in LoL https://technology.riotgames.com/news/determinism-league-legends-implementation
- fixed https://docs.rs/fixed · cordic https://docs.rs/cordic · Rapier determinism https://rapier.rs/docs/user_guides/rust/determinism/
- WebAssembly 128-bit 논의 https://github.com/WebAssembly/design/issues/1522 · relaxed-simd FMA https://github.com/WebAssembly/relaxed-simd/issues/44
