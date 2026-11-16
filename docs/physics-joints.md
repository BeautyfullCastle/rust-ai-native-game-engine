# 물리 조인트 (2D, 3D)

갱신: 2026-11-16. 코드: `crates/orr_physics/src/joints.rs`(2D), `crates/orr_physics3d/src/joints.rs`(3D). 테스트: 각 크레이트의 `tests/joints.rs`.

두 바디를 잇는 조인트다. 시뮬레이션 안에서 고정소수점으로 계산되므로 롤백, 리플레이, 체크섬에 그대로 들어간다.

| 종류 | 상수 | 하는 일 |
|---|---|---|
| 핀 | `JOINT_PIN` | 두 고정점을 한 점에 붙인다. 바디는 그 점을 중심으로 자유롭게 돈다(경첩, 진자). |
| 막대 | `JOINT_ROD` | 두 고정점 사이를 `length`로 유지한다. |
| 밧줄 | `JOINT_ROPE` | 두 고정점 사이를 `length` 이하로 유지한다. 당기기만 하고 밀지 않는다. |
| 프리즘(미끄럼) | `JOINT_PRISMATIC` | `b`가 `a`의 한 축을 따라서만 미끄러진다. 상대 회전은 잠근다. |

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
- 모터와 제한은 핀과 프리즘 조인트에만 적용된다. 막대와 밧줄에서는 무시한다.

### 프리즘(미끄럼) 조인트 (`JOINT_PRISMATIC`)

```rust
// 피스톤: 실린더 기준 +x축으로만 움직이고, 넣은 자리에서 0 ~ 2미터만 밀려 나온다
add_joint(frame, Joint::prismatic(cylinder, piston, FPVec2::ZERO, FPVec2::ZERO, FPVec2::new(FP::ONE, FP::ZERO)).with_limits(FP::ZERO, fp!(2)));
// 미닫이문: 초당 1.5미터로 밀고 힘은 80까지
add_joint(frame, Joint::prismatic(wall, door, anchor_a, anchor_b, FPVec2::X).with_motor(fp!(1.5), fp!(80)));
```

- 축은 `a`의 회전 전 좌표로 준다. 두 고정점은 `a`의 고정점을 지나는 축 위에 있어야 한다. 축에서 벗어난 만큼은 틱마다 20%씩 되돌린다(Baumgarte).
- 두 가지를 잠근다. 축에 수직인 방향의 이동 한 줄, 상대 회전 한 줄이다. 두 줄은 2×2 블록으로 함께 푼다. `add_joint`가 그 순간의 상대 각도(`ref_angle`)를 기록하고 `b`는 그 각도를 유지한다.
- 축은 새 필드 없이 `length`에 `a` 기준 각도(라디안)로 들어간다. `Joint::prismatic`이 축 벡터를 각도로 바꿔 넣는다.
- 제한(`with_limits`)은 축 방향 거리다. 두 고정점이 벌어진 양을 축에 투영한 값에서 `add_joint`를 부른 순간의 값을 뺀 것을 0으로 잰다. 그래서 추가한 자세가 0이고, `add_joint`가 그만큼을 `lower`, `upper`에 더해 저장한다. 제한은 speculative 한쪽 제한이다.
- 모터(`with_motor(speed, max_force)`)는 `b`가 `a`에 대해 축을 따라 `speed`로 움직이게 하되, 틱당 임펄스를 `max_force * dt`로 자른다. 속도를 0으로 하면 브레이크가 된다.
- `a`가 도는 중에도 맞게 풀리도록, `a`가 받는 힘의 작용점은 `b`의 고정점 밑에 있는 `a`의 점으로 계산한다.

## 3D

`orr_physics3d`에도 같은 구조의 조인트가 있다. 종류는 `JOINT_BALL`(두 고정점을 붙이고 모든 방향으로 돎), `JOINT_ROD`, `JOINT_ROPE`다. 고정점은 각 바디의 회전 전 좌표이고, 매 서브스텝마다 그 서브스텝까지 움직인 자세로 고정점과 유효 질량(3×3)을 다시 계산한다. 접촉의 반복 순서(홀짝으로 번갈아)를 유지한 채 반복마다 접촉 다음에 조인트를 푼다.

- 정적 접촉 제공자 경로(`step_with_static_contacts`, 지형 충돌)에서는 조인트를 풀지 않는다.
- 점 제약의 3×3 유효 질량은 역행렬을 만들지 않고 여인수와 행렬식으로 푼다(i128 누적). 가볍고 고정점에서 먼 바디는 `K^-1` 원소가 고정소수점 몇 단위로 떨어져 정밀도가 사라지기 때문이다.

### 힌지 (`JOINT_HINGE`)

```rust
// 문: 문틀 기준 y축으로 돌고, 닫힌 자리에서 0 ~ 1.6라디안만 열림
add_joint(frame, Joint::hinge(frame_body, door, hinge_on_frame, hinge_on_door, FPVec3::new(FP::ZERO, FP::ONE, FP::ZERO)).with_limits(FP::ZERO, fp!(1.6)));
// 바퀴: 차축(x) 둘레로 초당 8라디안, 토크 60까지
add_joint(frame, Joint::hinge(chassis, wheel, axle, FPVec3::ZERO, FPVec3::new(FP::ONE, FP::ZERO, FP::ZERO)).with_motor(fp!(8), fp!(60)));
```

- 볼 조인트에 두 바디의 축을 맞추는 회전 제약 두 줄(축에 수직인 두 방향)을 더한 것이다. 두 줄은 2×2 블록으로 함께 푼다.
- 축은 `a`의 회전 전 좌표로 준다. `add_joint`가 그 순간 자세로 `b` 쪽 축(`axis_b`)과 각도 기준 방향(`ref_a`, `ref_b`)을 채운다. 각도는 `b`가 `a`에 대해 축 둘레로 돈 양이고, 추가한 순간이 0이다.
- 모터, 제한의 의미와 풀이는 2D 핀과 같다(speculative 제한, 틱이 아니라 서브스텝마다 임펄스 상한 `max_motor_torque * h`).
- 회전 임펄스(정렬, 모터, 제한)는 모두 고정점이 벌어지지 않게 짝이 되는 선형 임펄스와 함께 넣고, 유효 질량은 3×3 고정점 질량의 Schur complement다.

### 프리즘(미끄럼) 조인트 (`JOINT_PRISMATIC`)

```rust
// 피스톤: 실린더 기준 y축으로만 움직이고, 넣은 자리에서 0 ~ 2미터만 밀려 나온다
add_joint(frame, Joint::prismatic(cylinder, piston, FPVec3::ZERO, FPVec3::ZERO, FPVec3::Y).with_limits(FP::ZERO, fp!(2)));
// 미닫이문: 벽 기준 x축으로 초당 1.5미터, 힘은 80까지
add_joint(frame, Joint::prismatic(wall, door, anchor_a, anchor_b, FPVec3::X).with_motor(fp!(1.5), fp!(80)));
```

- `b`는 `a`의 축 하나를 따라서만 움직이고 상대 회전은 모두 잠긴다. 축은 `a`의 회전 전 좌표로 준다. 축 길이는 상관없다(`add_joint`가 정규화한다).
- `add_joint`가 그 순간 자세로 기록한다. `axis_b`에 `b` 쪽 축, `ref_a`에 `a` 기준 `b`의 회전(사원수의 벡터 부분), `ref_b`에 사원수의 실수 부분(`x`)과 그때의 미끄러진 거리(`y`)가 들어간다. 그래서 `Joint` 레이아웃은 그대로다. 기록 여부는 `axis_b`가 0인지로 본다.
- 제한과 모터의 의미는 2D와 같다. 제한은 추가한 자세가 0이고 `lower`, `upper`는 축 방향 거리다. 모터의 임펄스 상한은 서브스텝마다 `max_motor_torque * h`다(여기서는 힘).
- 서브스텝마다 자세로 다시 계산한다. 푸는 순서는 모터, 제한, 축에 수직인 이동 두 줄, 회전 세 줄이다.
- 수직 이동 두 줄은 볼 조인트의 3×3 질량에서 축 방향을 빼고 축 방향에 1을 넣은 행렬로, 볼 조인트와 같은 여인수 풀이(i128)로 푼다. 회전 세 줄은 두 바디의 역관성을 더한 3×3 행렬로 푼다. 회전 오차는 `b`의 자세와 `a`의 자세에 기록한 회전을 곱한 자세의 차이 벡터(작은 각도)를 20%씩 되돌린다.
- 2D와 마찬가지로 `a`가 받는 힘의 작용점은 `b`의 고정점 밑에 있는 `a`의 점이다.

### 볼 조인트의 스윙·비틀림 제한

```rust
// 어깨: 팔이 몸 기준 +x 축에서 1.2라디안 원뿔 안으로만 휘고, 그 축 둘레로 -0.5 ~ 0.5라디안만 비틀림
add_joint(frame, Joint::new(JOINT_BALL, body, arm, shoulder, FPVec3::ZERO, FP::ZERO).with_cone(FPVec3::new(FP::ONE, FP::ZERO, FP::ZERO), fp!(1.2), fp!(-0.5), fp!(0.5)));
```

- `with_cone(axis, swing, lower, upper)`는 `JOINT_LIMIT`을 켜고 축(`a`의 회전 전 좌표), 스윙 각(`swing`), 비틀림 범위(`lower`, `upper`)를 넣는다. 힌지처럼 `add_joint`가 그 순간 자세로 `b` 쪽 축과 비틀림 기준 방향을 채우므로 추가한 자세가 스윙 0, 비틀림 0이다.
- 스윙은 두 바디 축 사이의 각이고 `swing` 이하로 막는다(원뿔). 축이 갈라진 방향(`n_a × n_b`) 둘레의 한쪽 제한 한 줄이다.
- 비틀림은 두 축의 이등분선 둘레로, 기준 방향을 그 축에 수직인 평면에 투영해 잰 각이다. 아래·위 한쪽 제한 두 줄이다. 축이 거의 반대 방향이면 이등분선이 없어서 그동안 비틀림 제한을 쉰다. 그래서 `swing`은 pi보다 작게 둔다.
- 세 줄 모두 힌지 제한과 같은 speculative 한쪽 제한이고, 고정점 임펄스와 짝을 이룬다(힌지와 같은 `limit` 함수). 경계에서 먼 동안에는 임펄스가 0이어서 자유 볼 조인트와 비트 단위로 같게 움직인다.

## 에디터와 씬 파일 (3D)

`orr_physics3d::register_joints_reflect`가 `orr_physics3d::Joint`를 리플렉션에 등록한다. Yard3D는 `register_joints`와 함께 이것을 부른다. 그래서 Yard3D 씬에서는 다음이 된다.

- 인스펙터의 "+ Add component"로 아무 엔티티에나 `orr_physics3d::Joint`를 붙인다. 조인트는 바디와 다른 자기 엔티티에 둔다.
- `a`, `b` 필드는 씬의 엔티티 목록에서 고른다(콤보 상자, "none"이 맨 위). 엔티티를 가리키는 다른 필드(`Scatter::terrain` 등)도 같은 방식이다. 플레이 중에는 읽기만 된다.
- `kind`(ball, rod, rope, hinge, prismatic), `flags`(motor, limit), 고정점, 길이, 축, 각도 범위, 모터, `swing`을 고친다.
- 씬 파일에는 이 필드들만 저장한다. `b` 쪽 축과 각도 기준(`axis_b`, `ref_a`, `ref_b`)은 저장하지 않는다. `step`이 각도 기준이 아직 없는 조인트(`ref_a`가 0)를 만나면 그 스텝 시작의 자세로 기록한다. 그래서 씬에 저장된 자세, 또는 인스펙터에서 조인트를 붙인 자세가 각도 0이다. `add_joint`로 만든 조인트와 결과가 비트 단위로 같다(테스트로 확인).
- 뷰포트는 조인트를 표지로 그린다. 두 고정점을 잇는 하늘색 선, 첫 고정점의 작은 십자, 힌지와 프리즘 조인트와 제한된 볼 조인트는 축 방향의 주황색 선이다. 선택한 조인트는 굵게 그린다.

## 한계

- 조인트에 매달린 동적 바디는 잠들지 않는다(접촉으로 묶인 섬과 조인트를 합치지 않음). `add_joint`가 두 바디를 깨운다.
- 3D 볼 조인트의 스윙 제한은 원형 원뿔 하나다. 방향마다 각이 다른 타원 원뿔은 없다. 스프링(부드러운 조인트)은 2D, 3D 모두 없다.
- 위치 보정이 속도 항(Baumgarte)이라 무거운 사슬은 늘어날 수 있다. 반복 횟수(`PhysicsConfig::velocity_iterations`)를 올리면 줄어든다.
- 에디터와 씬 파일은 3D 조인트만 다룬다. 2D 조인트(`orr_physics::Joint`)는 리플렉션에 등록되지 않았다. Yard3D 외의 3D 게임은 `register_joints`, `register_joints_reflect`를 직접 불러야 한다.
- 고정점을 뷰포트에서 끌어 옮기는 기즈모는 없다. 숫자로 고친다.
- 인스펙터에서 조인트를 고치면 컴포넌트가 씬 값으로 다시 쓰여 기록이 지워지고, 다음 스텝에 그때 자세로 다시 기록된다. 코드에서 이미 기록된 조인트의 `axis_a`를 바꿀 때는 `ref_a`를 0으로 두어야 다시 기록된다.
