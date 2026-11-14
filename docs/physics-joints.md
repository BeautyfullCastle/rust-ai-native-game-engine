# 2D 물리 조인트

갱신: 2026-11-15. 코드: `crates/orr_physics/src/joints.rs`. 테스트: `crates/orr_physics/tests/joints.rs`.

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
add_joint(frame, Joint { a: door_frame, b: door, anchor_a, anchor_b, length: FP::ZERO, kind: JOINT_PIN, _pad: 0 });
```

- `Joint`는 자기 엔티티에 붙는 컴포넌트다. 지우려면 그 엔티티를 despawn한다.
- 고정점(`anchor_a`, `anchor_b`)은 각 바디의 무게중심 기준, 회전 전 좌표다.
- 두 바디 모두 `Collider`가 있어야 스텝에 들어간다. 움직이지 않는 바디(정적)에 이으면 세상에 고정된 점이 된다.
- 풀이: 접촉과 같은 순차 임펄스 반복에서, 반복마다 접촉 다음에 조인트를 푼다. 남은 위치 오차는 틱마다 20%씩 속도 항으로 줄인다(Baumgarte). 조인트는 조인트 엔티티 번호 순서로 풀어서 결정론이 유지된다.
- `register_joints`를 부르지 않은 게임은 스텝이 예전과 똑같다. 골든 체크섬도 그대로다.

## 한계

- 조인트에 매달린 동적 바디는 잠들지 않는다(접촉으로 묶인 섬과 조인트를 합치지 않음). `add_joint`가 두 바디를 깨운다.
- 모터, 각도 제한, 스프링(부드러운 조인트), 프리즘(미끄럼) 조인트는 없다.
- 위치 보정이 속도 항(Baumgarte)이라 무거운 사슬은 늘어날 수 있다. 반복 횟수(`PhysicsConfig::velocity_iterations`)를 올리면 줄어든다.
- 3D 물리(`orr_physics3d`)에는 아직 조인트가 없다.
- 에디터 인스펙터에서 조인트를 만들거나 고치는 화면은 없다(리플렉션 미등록).
