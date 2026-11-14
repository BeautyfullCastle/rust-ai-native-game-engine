# 물리 조인트 (2D, 3D)

갱신: 2026-11-15. 코드: `crates/orr_physics/src/joints.rs`(2D), `crates/orr_physics3d/src/joints.rs`(3D). 테스트: 각 크레이트의 `tests/joints.rs`.

두 바디를 잇는 조인트다. 시뮬레이션 안에서 고정소수점으로 계산되므로 롤백, 리플레이, 체크섬에 그대로 들어간다.

| 종류 | 상수 | 하는 일 |
|---|---|---|
| 핀 | `JOINT_PIN` | 두 고정점을 한 점에 붙인다. 바디는 그 점을 중심으로 자유롭게 돈다(경첩, 진자). |
| 막대 | `JOINT_ROD` | 두 고정점 사이를 `length`로 유지한다. |
| 밧줄 | `JOINT_ROPE` | 두 고정점 사이를 `length` 이하로 유지한다. 당기기만 하고 밀지 않는다. |

```rust
use orr_physics::{add_joint, register, register_joints, Joint, JOINT_PIN};

// 게임의 Game::register에서
register(builder);
register_joints(builder);

// 장면을 만들 때
add_joint(frame, Joint::new(JOINT_PIN, door_frame, door, anchor_a, anchor_b, FP::ZERO));
```

- `Joint`는 자기 엔티티에 붙는 컴포넌트다. 지우려면 그 엔티티를 despawn한다.
- 고정점(`anchor_a`, `anchor_b`)은 각 바디의 무게중심 기준, 회전 전 좌표다.
- 두 바디 모두 `Collider`가 있어야 스텝에 들어간다. 움직이지 않는 바디(정적)에 이으면 세상에 고정된 점이 된다.
- 풀이: 접촉과 같은 순차 임펄스 반복에서, 반복마다 접촉 다음에 조인트를 푼다. 남은 위치 오차는 틱마다 20%씩 속도 항으로 줄인다(Baumgarte). 조인트는 조인트 엔티티 번호 순서로 풀어서 결정론이 유지된다.
- `register_joints`를 부르지 않은 게임은 스텝이 예전과 똑같다. 골든 체크섬도 그대로다.
- 조인트로 이은 두 바디도 서로 충돌한다. 겹쳐 있는 바퀴와 차체처럼 충돌하면 안 되면 `Collider`의 `layer`/`mask`로 거른다.

### 모터와 각도 제한 (2D 핀)

```rust
// 바퀴: 차체에 대해 초당 6라디안으로 돌리고 토크는 40까지
add_joint(frame, Joint::new(JOINT_PIN, chassis, wheel, hub, FPVec2::ZERO, FP::ZERO).with_motor(fp!(6), fp!(40)));
// 문: 닫힌 자리에서 -0.1 ~ 1.6라디안만 열림
add_joint(frame, Joint::new(JOINT_PIN, door_frame, door, anchor_a, anchor_b, FP::ZERO).with_limits(fp!(-0.1), fp!(1.6)));
```

- 상대 각도는 `b`의 각도 빼기 `a`의 각도다. `add_joint`가 그 순간의 상대 각도를 `ref_angle`로 기록하고, 제한은 거기서부터 잰다. 제한은 `(-pi, pi)` 안이어야 한다.
- 모터는 상대 각속도를 `motor_speed`로 맞추되 틱당 임펄스를 `max_motor_torque * dt`로 자른다. 토크를 크게 주고 속도를 0으로 하면 브레이크가 된다.
- 제한은 경계에 아직 닿지 않았을 때도 이번 틱 안에 넘을 회전만 막는다(speculative). 넘은 만큼은 틱마다 20%씩 되돌린다.
- 회전 임펄스는 핀 고정점이 벌어지지 않게 짝이 되는 선형 임펄스와 함께 넣는다. 유효 질량은 고정점을 붙인 상태의 값(2×2 고정점 질량의 Schur complement)이다. 그래서 긴 팔 끝에 달린 가벼운 바디도 제한에서 튕겨 나가지 않는다.
- 모터와 제한은 핀에만 적용된다. 막대와 밧줄에서는 무시한다.

## 3D

`orr_physics3d`에도 같은 구조의 조인트가 있다. 종류는 `JOINT_BALL`(두 고정점을 붙이고 모든 방향으로 돎), `JOINT_ROD`, `JOINT_ROPE`다. 고정점은 각 바디의 회전 전 좌표이고, 매 서브스텝마다 그 서브스텝까지 움직인 자세로 고정점과 유효 질량(3×3)을 다시 계산한다. 접촉의 반복 순서(홀짝으로 번갈아)를 유지한 채 반복마다 접촉 다음에 조인트를 푼다.

- 정적 접촉 제공자 경로(`step_with_static_contacts`, 지형 충돌)에서는 조인트를 풀지 않는다.

## 한계

- 조인트에 매달린 동적 바디는 잠들지 않는다(접촉으로 묶인 섬과 조인트를 합치지 않음). `add_joint`가 두 바디를 깨운다.
- 3D 관절에는 모터와 각도 제한이 없다. 스프링(부드러운 조인트), 프리즘(미끄럼) 조인트는 2D, 3D 모두 없다.
- 위치 보정이 속도 항(Baumgarte)이라 무거운 사슬은 늘어날 수 있다. 반복 횟수(`PhysicsConfig::velocity_iterations`)를 올리면 줄어든다.
- 에디터 인스펙터에서 조인트를 만들거나 고치는 화면은 없다(리플렉션 미등록).
