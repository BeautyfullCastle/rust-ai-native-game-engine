# Orrery 진행 현황

최종 갱신: 2026-09-29 · 설계: `docs/design-v1.md`
코드: https://github.com/BeautyfullCastle/rust-ai-native-game-engine (`main`)

## 완료

| 크레이트 | 내용 | 수치 |
|---|---|---|
| `orr_fp` | Q48.16 `FP`, Q16.16 `FP32`, Vec2/3, Quat, PCG32 `FrameRng`, `fp!` 컴파일타임 리터럴, LUT sin/cos/atan/atan2(CORDIC 기준과 비트 동일, CORDIC 버전은 `*_cordic`으로 유지), 정수 sqrt, exp/ln. no_std, float 금지 lint | mul 0.72ns (f32 0.70), div 5.4ns, sqrt 81ns, sin/cos 7.0ns (CORDIC 66ns), atan 7.0ns (74ns), atan2 10ns, 연속 입력 19ns/호출 (CORDIC 93ns, f32 11ns). 골든 `0x1555d30109e62487` |
| `orr_ecs` | Frame(스파스셋, Pod 컴포넌트), 결정론 엔티티 할당, 쿼리 1~4개 + Without, Commands, 싱글톤, FrameList, FrameRing, xxh3 체크섬 | 1M 순회 1.9ms (순수 Vec 1.2ms), 10만 스냅샷 0.66ms, 체크섬 0.48ms. 골든 `14364510420768418636` |
| `orr_sim` | Game/System/SimContext, TickInputs, 이벤트 키, 핫패치 간접 호출(`hotpatch` 기능, subsecond 연결), 빌드 해시 | |
| `orr_session` | 예측/롤백, stall, 이벤트 3상태 조정, 체크섬 비교, `.orrp` 리플레이(lz4) + 빌드 해시 검사 | 1만 엔티티 틱 180µs, 8틱 재시뮬 787µs. 리플레이 약 19B/틱(2인) |
| `orr_ecs` 직렬화 | `Frame::to_bytes`/`from_bytes` (ORRF v1, LE, u32 길이, 체크섬 검증, 잘못된 입력은 Err). `.orrp` v2 키프레임 + `seek` | 직렬화/역직렬화 1만 179/136µs, 10만 3.5/3.1ms. 키프레임 50틱 간격 시 400틱 2인 리플레이 7.5KB→10.6KB |
| 늦은 참가 | 확정 틱(`verified_tick`) Frame 스냅샷 + lz4, 빌드 해시·설정·체크섬 검증. 빈 슬롯은 호스트가 기본 입력으로 채움. 기존 피어는 `authored_since`로 입력 재전송. `.orrp` 파서: 신뢰할 수 없는 개수·lz4 크기·틱 경계 검사 | 3번째 피어가 150틱에 참가, 600라운드까지 체크섬 일치(롤백 1028회) |
| 참가 복구 | 기존 피어가 `ORRB` 알림으로 보유 입력 구간을 알림 → 참가자가 빈틈을 시간 없이 판정(`InputGap`). 시도 번호로 재요청·새 스냅샷, 참가 중 입력 보관(hold), `max_join_attempts`(기본 3), `mark_slot_vacant`로 재입장 | 느린 전송(80라운드, 보관 4틱)도 재시도 없이 수렴. 누락→재요청, 재입장 모두 체크섬 일치 |
| `orr_testgame` | arena 테스트 게임, 루프백 네트워크 2클라 2000틱 테스트 | 골든 `0x13cdc3c810d65459` |
| CI | `.github/workflows/determinism.yml` — x64/ARM/Win/mac + wasm32-wasip1 골든 비교 | 2026-09-29 5개 플랫폼 모두 통과, 체크섬 일치 |

## 다음

1. 참가 후속: `join_backlog_peers` 기본값 0이면 빈틈 검사 없음(참가자가 피어 수를 스스로 알 방법 필요). 자리 비우기 시 피어마다 떠난 플레이어 입력을 받은 범위가 다르면 채우지 않음(조정자 필요). 스냅샷 전 도착한 알림은 호출자가 버퍼링. 폐기된 시도의 링크 정리. Relay 서버 연동. 조작된 리플레이 입력이 debug 빌드에서 FP 오버플로 패닉
2. `orr_bridge` (InProc/Threaded) + `orr_view` 보간 → M2
3. 결정론 FP 물리 2D
