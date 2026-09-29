# AI-Native Rust ECS 게임엔진 — 설계 v0 (리서치 종합)

작성: 2026-09-28 · 상태: 초안 (결정 필요 항목은 §9)
리서치: 서브에이전트 5개 (Sonnet×4, Haiku×1) — ECS 코어 / 렌더링·플랫폼 / AI-native / 에디터 / 서브시스템

---

## 0. 한 줄 요약

**"결정론적 ECS 코어 + 리플렉션 기반 원격 프로토콜(ERP)"을 엔진의 척추로 삼고, 에디터·AI 에이전트·MCP·테스트 러너가 모두 같은 프로토콜의 클라이언트가 되게 한다.** 렌더링은 wgpu 위에 얇은 RHI 경계를 두고 Bevy의 검증된 패턴(extract/prepare/queue, render graph, 파이프라인드 렌더 스레드)을 따른다.

## 1. 설계 원칙

1. **성능은 데이터 레이아웃에서 나온다** — SoA 아키타입 테이블, 청크 정렬, 프레임 아레나, 구조 변경은 커맨드 버퍼로 지연.
2. **결정론은 옵션이 아니라 모드다** — 시뮬레이션 월드는 고정 틱 + 결정론적 시스템 순서 + (선택) 고정소수점. 네트워크 롤백, 리플레이, AI 검증이 전부 이 위에 선다.
3. **모든 상태는 리플렉션으로 기술 가능해야 한다** — 에디터 인스펙터, 씬 직렬화, 원격 프로토콜, LLM 스키마가 하나의 타입 레지스트리를 공유.
4. **에이전트는 1급 사용자** — 사람이 에디터에서 할 수 있는 건 전부 프로토콜로도 가능. 모든 변경은 트랜잭션 로그에 남고 undo 가능.
5. **멀티플랫폼은 처음부터** — Win/macOS/Linux/iOS/Android/Web. 콘솔은 RHI 경계만 먼저 설계. Web 단일 스레드 폴백은 정식 실행 모드.
6. **Bevy에서 배우되 의존하지 않는다** — Bevy의 API 변동성·컴파일 시간 문제가 커뮤니티에서도 지적됨(Discussion #21838). 패턴은 차용, 핵심 코어는 자체 소유.

## 2. 아키텍처 레이어

```
┌──────────────────────────────────────────────────────────────┐
│  Clients:  Editor (egui)  │  MCP Adapter  │  Test/CI Runner  │ RL Gym
└─────────────┬─────────────┴───────┬───────┴────────┬─────────┘
              │   ERP (Engine Remote Protocol, JSON-RPC / binary)
┌─────────────▼───────────────────────────────────────────────┐
│  Runtime Host  — 세션/권한/트랜잭션 로그/스냅샷/리플레이        │
├─────────────────────────────────────────────────────────────┤
│  Gameplay: Scripting(Luau) · Physics · Anim · Audio · Net · UI │
├─────────────────────────────────────────────────────────────┤
│  Render: Render World · Render Graph · Material/Shader · RHI    │
├─────────────────────────────────────────────────────────────┤
│  Assets: GUID DB · Importer/Cooker · Streaming · Hot reload     │
├─────────────────────────────────────────────────────────────┤
│  Core: ECS · Scheduler/Jobs · Reflect/TypeRegistry · Math · Alloc│
├─────────────────────────────────────────────────────────────┤
│  Platform: winit · gilrs · fs/vfs · threads(wasm) · time        │
└─────────────────────────────────────────────────────────────┘
```

### 크레이트 구성 (안)

| 크레이트 | 역할 |
|---|---|
| `xx_core` | 수학(glam), 할당자(bumpalo 프레임 아레나), 해시, 로그/tracing |
| `xx_ecs` | 월드, 아키타입/스파스셋 저장소, 쿼리, 커맨드 버퍼, 관계, 변경 감지 |
| `xx_sched` | 시스템 그래프, 접근 집합 분석, 병렬 실행기, 고정 틱, 결정론 모드 |
| `xx_reflect` | 타입 레지스트리, 스키마(JSON Schema) 생성, serde 브리지 |
| `xx_asset` | GUID 에셋 DB, 핸들, 임포터/쿠커, 스트리밍, 핫리로드 |
| `xx_rhi` | 렌더 하드웨어 인터페이스 trait + `xx_rhi_wgpu` 구현 |
| `xx_render` | 렌더 월드, 렌더 그래프, 머티리얼, 라이팅, 포스트 |
| `xx_remote` | ERP 서버(HTTP/WebSocket/UDS), 트랜잭션 로그, 권한 |
| `xx_script` | Luau(mlua) 바인딩, 샌드박스, 핫리로드 |
| `xx_physics`, `xx_audio`, `xx_net`, `xx_anim`, `xx_ui` | 서브시스템 통합 |
| `xx_editor` | 별도 바이너리. ERP 클라이언트 + egui_dock UI |
| `xx_mcp` | 별도 프로세스. MCP ↔ ERP 어댑터 |

컴파일 시간 관리: 크레이트 경계를 촘촘히, 제네릭 단형화 억제, 매크로 확장 범위 제한, 개발 빌드는 dylib + mold/lld.

## 3. ECS 코어

| 항목 | 결정(안) | 근거 |
|---|---|---|
| 저장소 | **하이브리드**: 기본 아키타입 테이블(SoA), 컴포넌트별 `SparseSet` opt-in | 순회 성능 + 잦은 토글 컴포넌트(상태 태그) 비용 회피. Bevy가 검증한 방식 |
| 청크 | 테이블 컬럼을 16KB 청크로 분할, 병렬 잡은 청크 단위 | false sharing 회피, SIMD 친화 (Unity DOTS 16KB) |
| 구조 변경 | 커맨드 버퍼(스레드 로컬) → 동기 지점에서 일괄 적용 | 사실상 업계 표준 |
| 변경 감지 | 컴포넌트별 `added_tick`/`changed_tick` | 저비용, Bevy에서 검증 |
| 관계 | **flecs식 `(Relation, Target)` 쌍을 1급 개념**으로 (ChildOf, IsA/프리팹, 커스텀) | 계층·소유·AI 관계를 땜질 없이. Bevy는 아직 부분적 |
| 이벤트 | 버퍼형 `Message` + 즉시형 `Observer` 분리 | Bevy 0.17의 정리된 모델 |
| 엔티티 ID | 32bit index + 32bit generation, 네트워크/저장용 안정 ID는 별도 컴포넌트 | 로컬 ID 재사용과 영속 ID 분리 |

### 스케줄러
- 시스템의 read/write 접근 집합으로 충돌 그래프 생성 → 자동 병렬화. 명시적 순서 제약은 SystemSet.
- 실행기: **처음엔 rayon 기반**, 프로파일링 후 커스텀 work-stealing(프레임 예산 인지)으로 교체 가능하도록 trait 뒤에 숨김.
- 스케줄: `Startup` / `FixedSim`(결정론) / `Update`(가변) / `Extract`(렌더 월드로) / `Last`.
- **결정론 모드**(`FixedSim`): 시스템 순서를 위상정렬 결과로 고정, 쿼리 순회 순서 안정화(엔티티 ID 정렬 옵션), 시드 RNG 리소스, 부동소수점 대신 고정소수점 타입(`Fx32`/`Fx64`) 제공. 크로스플랫폼 float 결정론은 신뢰하지 않음.
- 롤백: `FixedSim` 월드의 스냅샷/복원 API를 코어에 내장 → GGRS류 롤백 넷코드, 리플레이, AI 검증에 공용.

### 핫리로드
- MVP: 데이터(씬/에셋/셰이더/Luau 스크립트) 핫리로드.
- v1: dylib 기반 게임 로직 리로드(hot-lib-reloader 방식).
- 장기: subsecond식 함수 단위 핫패치(Bevy 0.17 채택, 바이너리 크레이트 한정·WASM 미지원 제약 인지).

## 4. 렌더링 & 플랫폼

- **API**: wgpu로 시작 (Vulkan/Metal/DX12/GL/WebGPU, WebGL2 폴백). 그 위에 얇은 **`xx_rhi` trait** — 콘솔(NDA SDK) 백엔드와 고급 기능(메시 셰이더/HW RT) 네이티브 경로를 나중에 끼울 자리.
  - wgpu의 bindless/메시 셰이더/레이트레이싱은 네이티브에서 실험적, WebGPU에선 미지원 → 고급 경로는 opt-in.
  - rend3는 2025-06 아카이브 — 사용 불가.
- **구조**: Bevy 패턴 차용 — 메인 월드 → `Extract` → 렌더 월드(prepare/queue) → 렌더 그래프. **파이프라인드 렌더링**(시뮬 N+1과 렌더 N 병렬).
- **렌더 패스**: 기본은 **Clustered Forward+** (모바일/웹/투명/MSAA 친화). 고사양 opt-in으로 **비저빌리티 버퍼 + 메시렛 GPU-driven** 경로 (Bevy 0.16 virtual geometry 참고).
- **셰이더**: 작성 언어 후보 Slang vs WGSL — §9 결정. 조합/퍼뮤테이션은 naga_oil 방식(`#import`, `#define`), 퍼뮤테이션 해시 키 파이프라인 캐시 + 디스크 캐시.
- **플랫폼**: winit + gilrs 시작, 콘솔 단계에서 SDL3 재검토. 모바일은 suspend/resume 시 surface 손실을 렌더 그래프 리소스 수명의 1급 개념으로.
- **Web**: 멀티스레드는 COOP/COEP 헤더 필요 (SharedArrayBuffer). 스케줄러는 **단일 스레드 실행기**를 정식 지원.
- **빌드**: cargo-mobile2(모바일), trunk + wasm-bindgen(웹).

## 5. 에셋 파이프라인

- GUID 기반 에셋 DB, `Handle<T>`(가볍고 안정)와 에셋 데이터(교체 가능) 분리 → 핫리로드/스트리밍/스레드 접근이 자연스럽게 해결.
- 소스 → **프로세서(쿠커)** → 콘텐츠 해시 기반 캐시 (Bevy Asset v2 모델).
- 텍스처: **KTX2 + Basis Universal**, 로드 시 BCn(데스크톱)/ASTC(모바일) 트랜스코딩. 메시/씬 인입은 glTF(+`KHR_texture_basisu`).
- 스트리밍은 처음부터 설계 (우선순위/거리 기반 요청 큐).
- 에셋 메타데이터에 태그·설명·임베딩 필드 → AI 시맨틱 검색의 토대.

## 6. AI-Native 설계

### 6.1 ERP — Engine Remote Protocol (척추)
Bevy BRP를 참고하되 트랜잭션/권한/검증까지 포함.

| 그룹 | 메서드 예 |
|---|---|
| World | `world.query`, `world.spawn`, `world.despawn`, `world.get/insert/remove/patch`, `world.reparent`, `resource.*` |
| Schema | `registry.schema`(모든 컴포넌트·리소스 JSON Schema), `rpc.discover`(OpenRPC) |
| Watch | `world.watch` (쿼리 결과 diff 스트리밍) |
| Tx | `tx.begin/commit/rollback`, `history.list`, `history.undo/redo` — **사람·에이전트 편집이 같은 undo 스택** |
| Sim | `sim.pause/step(n)/run`, `sim.snapshot/restore`, `sim.replay(log)` |
| Verify | `view.screenshot(camera)`, `view.diff(baseline)`, `metrics.frame`, `log.tail` |
| Asset | `asset.search(semantic)`, `asset.import`, `asset.meta` |
| Script | `script.write/reload`, `script.eval`(샌드박스) |

- 전송: HTTP/WebSocket(원격·에디터) + Unix 소켓/named pipe(로컬 저지연). 에디터용 바이너리 인코딩(bincode/postcard) 옵션.
- **권한**: 연결마다 capability 토큰 (read-only / scene-edit / script / asset-write / sim-control). 조사한 어떤 엔진도 제대로 안 하는 영역 → 차별점.

### 6.2 MCP 어댑터
- 엔진 코어에 MCP를 넣지 않고 **별도 얇은 프로세스**로 (Unity/Godot/Unreal/Bevy 사례 공통). ERP는 안정, MCP 툴 표면은 빠르게 진화.
- 툴 그룹 on/off로 프롬프트 크기 관리(CoplayDev unity-mcp 패턴).

### 6.3 LLM 친화 포맷과 문서
- 씬/프리팹: **텍스트 포맷, diff 친화**. RON vs YAML은 §9 결정. 스키마 버전 필드 필수.
- 엔진 버전에 고정된 `AGENTS.md` / `llms.txt`를 빌드에서 자동 생성 (타입 레지스트리 → API 요약). 빠르게 바뀌는 API에서 LLM 정확도를 올리는 검증된 방법(Bevy #23867 논의).

### 6.4 게임플레이 스크립팅
- Rust 컴파일 루프는 에이전트 반복에 너무 느림 → **Luau(mlua)** 게임플레이 스크립트: 타입, 샌드박스, 빠른 핫리로드, LLM 학습 데이터 풍부. 성능 핫패스는 Rust 시스템.
- 대안: WASM(wasmtime) — 모드/플러그인용으로 v2.

### 6.5 헤드리스 & 검증 루프
- `--headless` (윈도우 없음, 오프스크린 렌더 옵션) 정식 실행 모드.
- 에이전트 루프: 변경 → `sim.step` → 스크린샷/메트릭 → 베이스라인 diff → 커밋/롤백. 결정론 모드 덕분에 재현 가능.

### 6.6 런타임 AI (후순위)
- 추론: `ort`(ONNX Runtime, 사전학습 모델 인입) 우선, `candle`(LLM/임베딩), `burn`(wgpu 백엔드) 검토.
- RL: ML-Agents Gym API 모양을 본뜬 헤드리스 고속 시뮬 하네스.

## 7. 에디터

### 7.1 기술 선택
- **별도 프로세스 + ERP** (Bevy가 가는 방향). 게임 크래시에도 에디터 생존, 모바일/원격 기기 attach, 에이전트와 동일 경로.
- UI: **egui + egui_dock** (MVP) — 인스펙터·프로파일러처럼 매 프레임 갱신되는 밀집 뷰에 즉시모드가 적합. 레이아웃 유연성 필요 시 egui_tiles.
- 뷰포트: 게임 프로세스가 오프스크린 렌더 → 공유 텍스처(동일 기기) 또는 스트림(원격). MVP는 동일 기기 공유 텍스처 또는 에디터 내 렌더러 임베드로 시작 (§9).

### 7.2 기본 레이아웃

```
┌──────────────────────────────────────────────────────────────────┐
│ File Edit View Systems AI Window    [▶][⏸][⏭ step] [Workspace▾] Ctrl+K │
├──────────────┬──────────────────────────────────┬────────────────┤
│ HIERARCHY    │                                  │ INSPECTOR      │
│ [Spatial|Flat│          VIEWPORT                │ Entity #42 gen3│
│  |Relations] │   gizmo · grid · 카메라 · 오버레이   │ ├ Transform    │
│ ▸ World      │   (Play 중 파란 테두리)             │ ├ MeshRef      │
│   ▸ Player   │                                  │ ├ AI::Chase    │
│   ▸ Enemies  │                                  │ [+ Component]  │
│ ▸ Resources  │                                  │ Archetype: 17  │
│              │                                  │ (340 entities) │
├──────────────┴─────────────┬────────────────────┼────────────────┤
│ ASSETS │ CONSOLE │ PROFILER │  AI AGENT           │ QUERY/SCHEDULE │
│ (탭)                        │  > 순찰 적 추가해줘   │ (Pos,Vel)→340  │
│                             │  ◇ +3 entities      │ system graph   │
│                             │  ◇ ~ enemy.luau     │                │
│                             │  [Accept][Reject]   │                │
└─────────────────────────────┴────────────────────┴────────────────┘
```

- **Workspaces** (Blender식 프리셋): Level Design / Gameplay-Script / AI Debug / Profiling / Asset.
- **Command palette (Ctrl+K)**: 게임 에디터엔 드문 패턴 → 차별점. 자연어 입력 시 AI 패널로 라우팅.

### 7.3 기능 우선순위

**MVP**
- 도킹 + 기본 레이아웃 저장/복원
- Hierarchy (Spatial/Flat 전환), Inspector (리플렉션 필드 편집, 컴포넌트 추가/삭제)
- Viewport + 이동/회전/스케일 기즈모, 에디터 카메라
- Asset Browser (드래그 투 씬), Console
- Play/Pause/Step/Stop (스냅샷 복원)
- **Undo/Redo — 사람·에이전트 편집 공용** (나중에 붙이기 가장 비싼 기능)

**v1 (ECS-native + AI 차별화)**
- 아키타입 뷰, Resources 뷰, Query 디버거 ("왜 이 엔티티에 시스템이 안 도나")
- AI Agent 패널: `@엔티티/@에셋/@시스템` 컨텍스트, **변경 단위 diff + 개별 Accept/Reject**
- Agent Action Log (콘솔과 분리, 항목별 revert)
- Command palette, 프로파일러(시스템별 프레임 타임라인), 프리팹 편집
- 스크립트 에디터(Luau, LSP)

**Later**
- 시스템 스케줄 그래프 시각화 (read/write 충돌 표시) — 선례 거의 없음
- Tracy 프로토콜 호환, 원격 기기 attach, 멀티 클라이언트 동시 편집
- 뷰포트/인스펙터 인라인 AI 제안(고스트 프리뷰)

## 8. 서브시스템 선택

| 영역 | 1순위 | 비고 |
|---|---|---|
| 물리 | **Rapier** (2D/3D, 결정론 문서화) | Avian은 Bevy 종속, Jolt 바인딩은 성숙도 낮음. 결정론 모드에선 `enhanced-determinism` 기능 |
| 오디오 | **kira** (+cpal) | 공간음향 기본 제공. FMOD 바인딩은 커뮤니티 수준 (재확인 필요) |
| 네트워크 | 결정론 롤백: **GGRS / fortress-rollback** · 서버권위: 자체(lightyear 참고) · 전송: quinn(QUIC), matchbox(WebRTC, 웹) | 코어 스냅샷 API와 연동 |
| 애니메이션 | 자체 (스켈레탈, 블렌드 트리, GPU 스키닝) | IK는 FABRIK/CCD 자체 구현 |
| 런타임 UI | **taffy**(레이아웃) + **cosmic-text** + 자체 렌더 | 디버그 UI는 egui |
| 스크립팅 | **mlua (Luau)** | WASM(wasmtime)은 모드용 v2 |
| 프로파일링 | **tracing + Tracy**(tracy-client), 필요시 puffin | 시스템/잡 단위 스팬 자동 삽입 |
| 입력 | 자체 액션 매핑 (leafwing 모델 참고) | |
| 내비 | oxidized_navigation / rerecast 평가 | |
| 로컬라이제이션 | fluent | |
| 저장 | serde + postcard/bincode, 스키마 버전 + 마이그레이션 | 리플렉션 레지스트리 재사용 |
| 플랫폼 서비스 | steamworks-rs 등 커뮤니티 바인딩 평가, 콘솔은 NDA FFI | Haiku 리포트의 "Steam Rust SDK 없음"은 부정확 — steamworks-rs 존재 |

## 9. 결정 필요 항목

1. **ECS 자체 구현 vs `bevy_ecs` 단독 사용** — 권고: 자체 구현 (결정론·관계·ERP 통합 제어권, Bevy API 변동 회피). 비용: 3~6개월 코어 작업. 대안: bevy_ecs로 프로토타입 후 교체.
2. **셰이더 작성 언어**: Slang(멀티 백엔드·콘솔 대비) vs WGSL(wgpu 네이티브, 툴체인 단순). 권고: MVP는 WGSL+naga_oil, 콘솔/고급 경로 착수 시 Slang 도입 재평가.
3. **씬 텍스트 포맷**: RON(serde 네이티브) vs YAML(LLM 학습 데이터 풍부) vs 자체 DSL. 권고: RON 계열 + JSON Schema로 검증.
4. **에디터 뷰포트**: 게임 프로세스 렌더 공유(텍스처 공유/스트림) vs 에디터 프로세스가 월드 미러를 직접 렌더. 권고: MVP는 미러 렌더(구현 단순), v1에서 공유 텍스처.
5. **결정론 수학**: 고정소수점 전면 vs 결정론 모드에서만. 권고: `FixedSim` 스케줄의 게임로직만 고정소수점, 렌더/연출은 f32.
6. **엔진 이름/크레이트 접두어** (`xx_` 자리).

## 10. 로드맵 (초안)

| 단계 | 기간(추정) | 산출물 |
|---|---|---|
| M0 코어 | 0–3개월 | xx_ecs, xx_sched(병렬+결정론+단일스레드), xx_reflect, 벤치마크 스위트(vs bevy_ecs/flecs) |
| M1 렌더 기초 | 2–5개월 | RHI+wgpu, 렌더 월드/그래프, Forward+, glTF, 윈도우/입력, Web·모바일 빌드 |
| M2 ERP + 헤드리스 | 3–5개월 | xx_remote(query/tx/undo/snapshot/screenshot), MCP 어댑터, AGENTS.md 생성 |
| M3 에디터 MVP | 4–8개월 | egui 에디터 MVP 기능 전체 |
| M4 게임플레이 | 6–10개월 | Luau, Rapier, kira, 애니메이션, UI, 에셋 쿠커/KTX2 |
| M5 AI 에디터 v1 | 8–12개월 | Agent 패널 diff/accept, Action Log, 시각 검증 루프 |
| M6 네트워크 | 10–14개월 | 롤백 + 서버권위, 리플레이 |

**성능 게이트** (매 단계 CI): 1M 엔티티 단순 쿼리 순회, 10만 엔티티 add/remove, 병렬 스케줄러 스케일링(1→16코어), 프레임 타임 p99, WASM 단일스레드 성능, 증분 컴파일 시간.

## 11. 주의·불확실성

- wgpu 메시 셰이더/bindless/RT 일정은 유동적 — 착수 전 재확인.
- "UE 5.8 공식 MCP 지원", 특정 Bevy PR 번호 등 일부 서브에이전트 인용은 2차 출처 — 인용 전 원문 확인 필요.
- 서브시스템 리포트(Haiku)의 크레이트 버전 표기는 부정확할 수 있음.
- 콘솔 지원은 공개 자료로 검증 불가 (NDA).

## 참고 출처 (주요)

- Bevy 0.16 / 0.17 릴리스 노트 — https://bevy.org/news/bevy-0-16/ · https://bevy.org/news/bevy-0-17/
- Bevy Archetypes & Storage — https://deepwiki.com/bevyengine/bevy/2.7-archetypes-and-storage
- flecs Relationships — https://www.flecs.dev/flecs/md_docs_2Relationships.html
- EnTT — https://github.com/skypjack/entt · shipyard — https://github.com/leudz/shipyard
- GGRS — https://docs.rs/ggrs · fortress-rollback — https://github.com/wallstop/fortress-rollback
- Bevy governance 논의 #21838 — https://github.com/bevyengine/bevy/discussions/21838
- Virtual Geometry in Bevy 0.16 — https://jms55.github.io/posts/2025-03-27-virtual-geometry-bevy-0-16/
- naga_oil — https://github.com/bevyengine/naga_oil · Slang (Khronos) — https://www.khronos.org/news/tags/tag/slang
- wasm-bindgen-rayon — https://github.com/piotr-roslaniec/wasm-bindgen-rayon · android-activity — https://github.com/rust-mobile/android-activity
- Bevy KTX2 PR — https://github.com/bevyengine/bevy/pull/18411
- bevy_remote (BRP) — https://docs.rs/bevy/latest/bevy/remote/index.html · BRP PR #14880 — https://github.com/bevyengine/bevy/pull/14880
- CoplayDev unity-mcp — https://github.com/CoplayDev/unity-mcp · Unity MCP 공식 — https://docs.unity3d.com/Packages/com.unity.ai.assistant@2.0/manual/unity-mcp-overview.html
- Godot MCP — https://github.com/Coding-Solo/godot-mcp · Unreal MCP — https://github.com/chongdashu/unreal-mcp
- Roblox Cube — https://github.com/Roblox/cube/
- Bevy AGENTS.md 논의 #23867 — https://github.com/bevyengine/bevy/issues/23867
- nAIVE Engine — https://naive.dev/
- burn — https://github.com/tracel-ai/burn · ML-Agents Gym — https://unity-technologies.github.io/ml-agents/Python-Gym-API/
- egui_dock — https://docs.rs/egui_dock · egui_tiles — https://lib.rs/crates/egui_tiles
- bevy_editor_pls — https://github.com/jakobhellermann/bevy_editor_pls
- Flax 에디터 / Tracy — https://docs.flaxengine.com/manual/editor/interface.html · https://docs.flaxengine.com/manual/editor/profiling/tracy.html
- Rapier — https://rapier.rs/docs/ · kira — https://docs.rs/kira · lightyear — https://github.com/cBournhonesque/lightyear · mlua — https://github.com/mlua-rs/mlua
