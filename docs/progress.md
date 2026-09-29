# Orrery 진행 현황

최종 갱신: 2026-09-29 · 설계: `docs/design-v1.md`
코드: https://github.com/BeautyfullCastle/rust-ai-native-game-engine (`main`)

## 완료

| 크레이트 | 내용 | 수치 |
|---|---|---|
| `orr_fp` | Q48.16 `FP`, Q16.16 `FP32`, Vec2/3, Quat, PCG32 `FrameRng`, `fp!` 컴파일타임 리터럴, LUT sin/cos(CORDIC 기준과 비트 동일) + CORDIC atan, 정수 sqrt, exp/ln. no_std, float 금지 lint | mul 0.72ns (f32 0.70), div 5.4ns, sqrt 81ns, sin/cos 6.5ns (CORDIC 66ns), atan2 79ns. 골든 `0x1555d30109e62487` |
| `orr_ecs` | Frame(스파스셋, Pod 컴포넌트), 결정론 엔티티 할당, 쿼리 1~4개 + Without, Commands, 싱글톤, FrameList, FrameRing, xxh3 체크섬 | 1M 순회 1.9ms (순수 Vec 1.2ms), 10만 스냅샷 0.66ms, 체크섬 0.48ms. 골든 `14364510420768418636` |
| `orr_sim` | Game/System/SimContext, TickInputs, 이벤트 키, 핫패치 간접 호출(`hotpatch` 기능, subsecond 연결), 빌드 해시 | |
| `orr_session` | 예측/롤백, stall, 이벤트 3상태 조정, 체크섬 비교, `.orrp` 리플레이(lz4) + 빌드 해시 검사 | 1만 엔티티 틱 180µs, 8틱 재시뮬 787µs. 리플레이 약 19B/틱(2인) |
| `orr_testgame` | arena 테스트 게임, 루프백 네트워크 2클라 2000틱 테스트 | 골든 `0x13cdc3c810d65459` |
| CI | `.github/workflows/determinism.yml` — x64/ARM/Win/mac + wasm32-wasip1 골든 비교 | 2026-09-29 5개 플랫폼 모두 통과, 체크섬 일치 |

## 다음

1. `orr_ecs` Frame 바이트 직렬화 → 리플레이 스냅샷, 늦은 참가
2. ~~고정소수점 sin/cos LUT 경로~~ 완료 (66ns → 6.5ns, 골든 불변). 남음: atan/atan2 LUT (현재 CORDIC 약 80ns)
3. `orr_bridge` (InProc/Threaded) + `orr_view` 보간 → M2
4. 결정론 FP 물리 2D
