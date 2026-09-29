# Orrery — 결정론 롤백 ECS 게임엔진 (Rust)

설계: `docs/design-v1.md` (확정 결정은 §0) · 진행 현황/다음 작업: `docs/progress.md` · 초기 리서치: `docs/design-v0-research.md`

## 구조
- `orr_fp` 고정소수점(Q48.16 `FP`) 수학 · `orr_ecs` Frame(스파스셋, Pod 컴포넌트, 스냅샷/체크섬)
- `orr_sim` Game/System/이벤트/핫패치 훅 · `orr_session` 예측·롤백·리플레이 · `orr_testgame` arena 테스트 게임

## 절대 규칙 (시뮬 크레이트: orr_fp, orr_ecs, orr_sim, orr_session)
- f32/f64 금지 (뷰 레이어와 `float-interop` 기능만 예외). 모든 시뮬 수치는 `FP`.
- HashMap/HashSet/Instant/SystemTime 금지 (clippy.toml `disallowed-types`).
- usize를 해시·직렬화 상태에 쓰지 않는다 (u32/u64).
- 시뮬 컴포넌트는 `bytemuck::Pod` (패딩·힙 포인터 없음).
- 뷰는 시뮬을 읽기만 한다. 뷰→시뮬 경로는 Input/Command뿐.
- 골든 체크섬 테스트 값은 의도적 변경이 아니면 바꾸지 않는다. 바꾸면 커밋 메시지에 이유를 쓴다.

## 확인 명령
- `cargo test --workspace --release`
- `cargo clippy --workspace --all-targets`
- `cargo bench -p orr_ecs` / `-p orr_session` / `-p orr_fp`

## 작업 방식
- 서브에이전트는 낮은 모델(Sonnet/Haiku)로 돌린다 (프로젝트 지침).
- 성능 수치는 변경 전후를 벤치로 비교해 기록한다.
