# Orrery 진행 현황

최종 갱신: 2026-09-29 · 설계: `docs/design-v1.md`
코드: https://github.com/BeautyfullCastle/rust-ai-native-game-engine (`main`)

## 완료

| 크레이트 | 내용 | 수치 |
|---|---|---|
| `orr_fp` | Q48.16 `FP`, Q16.16 `FP32`, Vec2/3, Quat, PCG32 `FrameRng`, `fp!` 컴파일타임 리터럴, CORDIC 삼각함수, 정수 sqrt, exp/ln. no_std, float 금지 lint | mul 0.72ns (f32 0.70), div 5.4ns, sqrt 81ns, sin 66ns, atan2 79ns. 골든 `0x1555d30109e62487` |
| `orr_ecs` | Frame(스파스셋, Pod 컴포넌트), 결정론 엔티티 할당, 쿼리 1~4개 + Without, Commands, 싱글톤, FrameList, FrameRing, xxh3 체크섬 | 1M 순회 1.9ms (순수 Vec 1.2ms), 10만 스냅샷 0.66ms, 체크섬 0.48ms. 골든 `14364510420768418636` |
| `orr_sim` | Game/System/SimContext, TickInputs, 이벤트 키, 핫패치 간접 호출(`hotpatch` 기능, subsecond 연결), 빌드 해시 | |
| `orr_session` | 예측/롤백, stall, 이벤트 3상태 조정, 체크섬 비교, `.orrp` 리플레이(lz4) + 빌드 해시 검사 | 1만 엔티티 틱 180µs, 8틱 재시뮬 787µs. 리플레이 약 19B/틱(2인) |
| `orr_ecs` 직렬화 | `Frame::to_bytes`/`from_bytes` (ORRF v1, LE, u32 길이, 체크섬 검증, 잘못된 입력은 Err). `.orrp` v2 키프레임 + `seek` | 직렬화/역직렬화 1만 179/136µs, 10만 3.5/3.1ms. 키프레임 50틱 간격 시 400틱 2인 리플레이 7.5KB→10.6KB |
| `orr_testgame` | arena 테스트 게임, 루프백 네트워크 2클라 2000틱 테스트 | 골든 `0x13cdc3c810d65459` |
| CI | `.github/workflows/determinism.yml` — x64/ARM/Win/mac + wasm32-wasip1 골든 비교 | 2026-09-29 5개 플랫폼 모두 통과, 체크섬 일치 |

## 다음

1. 늦은 참가 (Frame 바이트 API 위에 전송 경로). 리플레이 파서의 `Vec::with_capacity`를 신뢰할 수 없는 개수로 부르는 부분 강화
2. 고정소수점 sin/cos LUT 경로 (현재 CORDIC 66ns → 목표 <10ns)
3. `orr_bridge` (InProc/Threaded) + `orr_view` 보간 → M2
4. 결정론 FP 물리 2D
