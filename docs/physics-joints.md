# 물리 조인트 (2D, 3D)

갱신: 2026-11-16. 코드: `crates/orr_physics/src/joints.rs`(2D), `crates/orr_physics3d/src/joints.rs`(3D). 테스트: 각 크레이트의 `tests/joints.rs`.

두 바디를 잇는 조인트다. 시뮬레이션 안에서 고정소수점으로 계산되므로 롤백, 리플레이, 체크섬에 그대로 들어간다.

| 종류 | 상수 | 하는 일 |
|---|---|---|
| 핀 | `JOINT_PIN` | 두 고정점을 한 점에 붙인다. 바디는 그 점을 중심으로 자유롭게 돈다(경첩, 진자). |
| 막대 | `JOINT_ROD` | 두 고정점 사이를 `length`로 유지한다. |
| 밧줄 | `JOINT_ROPE` | 두 고정점 사이를 `length` 이하로 유지한다. 당기기만 하고 밀지 않는다. |
| 프리즘(미끄럼) | `JOINT_PRISMATIC` | `b`가 `a`의 한 축을 따라서만 미끄러진다. 상대 회전은 잠근다. |
| 스프링 | `JOINT_SPRING` | 두 고정점을 `length`로 당기는 부드러운 막대. 세기는 진동수(Hz)와 감쇠비로 정한다. |

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
- `register_joints`를 부르지 않은 게임은 스텝이 예전과 똑같다. 골든 체크섬도 그대로다. `PhysGame`은 이제 `register_joints`를 부르므로 컴포넌트 목록이 달라져, 그 세 골든(`golden_phys_*`)만 새 값으로 바꿨다. 시뮬레이션 동작은 같다.
- 조인트로 이은 두 바디도 서로 충돌한다. 겹쳐 있는 바퀴와 차체처럼 충돌하면 안 되면 `Collider`의 `layer`/`mask`로 거른다.

### 모터와 각도 제한 (2D 핀)

```rust
// 바퀴: 차체에 대해 초당 6라디안으로 돌리고 토크는 40까지
add_joint(frame, Joint::new(JOINT_PIN, chassis, wheel, hub, FPVec2::ZERO, FP::ZERO).with_motor(fp!(6), fp!(40)));
// 문: 닫힌 자리에서 -0.1 ~ 1.6라디안만 열림
add_joint(frame, Joint::new(JOINT_PIN, door_frame, door, anchor_a, anchor_b, FP::ZERO).with_limits(fp!(-0.1), fp!(1.6)));
```

- 상대 각도는 `b`의 각도 빼기 `a`의 각도다. `add_joint`가 그 순간의 상대 각도를 `ref_angle`로 기록하고, 제한은 거기서부터 잰다. 제한은 `(-pi, pi)` 안이어야 한다.
- 모터는 상대 각속도를 `motor_speed`로 맞추되 틱당 임펄스를 `max_motor_torque * dt`로 자른다. 음수 토크는 절댓값으로 본다. 토크를 크게 주고 속도를 0으로 하면 브레이크가 된다.
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

### 스프링 (`JOINT_SPRING`)

```rust
// 서스펜션: 1.5Hz로 출렁이고, 감쇠비 0.5라서 두어 번 흔들리다 멈춘다
add_joint(frame, Joint::spring(chassis, wheel, mount, FPVec2::ZERO, fp!(0.6), fp!(1.5), fp!(0.5)));
// 감쇠비 1: 넘치지 않고 제자리로 돌아온다
add_joint(frame, Joint::spring(hook, bob, FPVec2::ZERO, FPVec2::ZERO, fp!(2), FP::ONE, FP::ONE));
```

- `Joint::spring(a, b, anchor_a, anchor_b, length, hertz, damping_ratio)`다. 두 고정점이 `length`보다 멀면 당기고 가까우면 민다. `length`는 쉬는 길이다. 막대처럼 딱 붙들지 않고, 벗어난 만큼 힘이 걸린다.
- 진동수 `hertz`는 정적인 고정점에 질량을 달았을 때 1초에 몇 번 오가는지다. 주기는 약 `1 / hertz`초다. 감쇠비 `damping_ratio`는 0이면 감쇠가 없고, 1이면 넘치지 않고 가장 빨리 멈춘다.
- 새 종류를 만든 이유: 막대에 옵션을 붙이면 막대의 안 쓰던 필드에 우연히 값이 남은 기존 조인트가 달라질 수 있다. 새 종류는 기존 종류의 동작을 비트 단위로 그대로 둔다. 스프링은 막대의 풀이를 그대로 쓰고 계수만 부드럽게 바꾼다.
- `Joint` 레이아웃은 그대로다. 진동수는 `motor_speed`, 감쇠비는 `max_motor_torque`에 들어간다. 스프링에서 모터와 제한(`flags`)은 무시한다.
- 진동수가 0이거나 음수이면 막대와 비트 단위로 똑같이 푼다. 감쇠비가 음수이면 0으로 본다. 진동수는 1000Hz, 감쇠비는 1000까지만 쓴다.
- 풀이는 Box2D의 soft step이다. 스텝 길이 `h`, `omega = 2 pi * hertz`로 `a1 = 2 * 감쇠비 + h * omega`, `a2 = h * omega * a1`, `a3 = 1 / (1 + a2)`를 구한다. 위치 오차는 `omega / a1`의 비율로 속도 항이 되고, 임펄스는 `a2 * a3`배의 질량 비율로 쓴다. 이미 쓴 임펄스의 `a3`배를 뺀다(그 스텝 안에서 누적). 막대와 달리 오차를 20%씩 지우는 Baumgarte를 쓰지 않는다. 2D는 틱마다, 3D는 서브스텝마다 계수를 구한다.
- 풀이가 암시적이라 감쇠비 0이어도 스윙마다 에너지가 조금 줄어든다. 60Hz에서 1Hz 스프링은 한 번 흔들 때 진폭이 4분의 1쯤 준다. 진동수가 높을수록(스텝당 `h * omega`가 클수록) 더 빨리 준다.
- 쉬는 길이가 거의 0이거나 두 고정점이 거의 겹치면(0.001 이하) 방향을 알 수 없어 그 스텝은 쉰다.

## 3D

`orr_physics3d`에도 같은 구조의 조인트가 있다. 종류는 `JOINT_BALL`(두 고정점을 붙이고 모든 방향으로 돎), `JOINT_ROD`, `JOINT_ROPE`, `JOINT_SPRING`이다. 스프링은 2D와 같고 `Joint::spring(a, b, anchor_a, anchor_b, length, hertz, damping_ratio)`로 만든다(서브스텝마다 계수를 구한다). 고정점은 각 바디의 회전 전 좌표이고, 매 서브스텝마다 그 서브스텝까지 움직인 자세로 고정점과 유효 질량(3×3)을 다시 계산한다. 접촉의 반복 순서(홀짝으로 번갈아)를 유지한 채 반복마다 접촉 다음에 조인트를 푼다.

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
- 축은 `a`의 회전 전 좌표로 준다. `add_joint`가 그 순간 자세로 `b` 쪽 축(`axis_b`)과 각도 기준 방향(`ref_a`, `ref_b`)을 채운다. 각도는 `b`가 `a`에 대해 축 둘레로 돈 양이고, 추가한 순간이 0이다. 축이 영벡터이면 기록하지 않고 축 제약 없이 볼 조인트처럼 푼다(힌지의 모터와 제한, 볼 조인트의 원뿔도 쉰다).
- 모터, 제한의 의미와 풀이는 2D 핀과 같다(speculative 제한, 틱이 아니라 서브스텝마다 임펄스 상한 `max_motor_torque * h`, 음수 토크는 절댓값).
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
- 스윙은 두 바디 축 사이의 각이고 `swing` 이하로 막는다(원뿔). 축이 갈라진 방향(`n_a × n_b`) 둘레의 한쪽 제한 한 줄이다. 두 축이 정반대여서 갈라진 방향이 없으면(사인 0.001 이하, 코사인 음수) `n_a`에 수직인 한 방향을 대신 쓰므로 스윙 제한이 계속 당긴다.
- 비틀림은 두 축의 이등분선 둘레로, 기준 방향을 그 축에 수직인 평면에 투영해 잰 각이다. 아래·위 한쪽 제한 두 줄이다. 축이 거의 반대 방향이면 이등분선이 없어서 그동안 비틀림 제한을 쉰다. 그래서 `swing`은 pi보다 작게 둔다.
- 세 줄 모두 힌지 제한과 같은 speculative 한쪽 제한이고, 고정점 임펄스와 짝을 이룬다(힌지와 같은 `limit` 함수). 경계에서 먼 동안에는 임펄스가 0이어서 자유 볼 조인트와 비트 단위로 같게 움직인다.

## 에디터와 씬 파일 (3D)

`orr_physics3d::register_joints_reflect`가 `orr_physics3d::Joint`를 리플렉션에 등록한다. Yard3D는 `register_joints`와 함께 이것을 부른다. 그래서 Yard3D 씬에서는 다음이 된다.

- 인스펙터의 "+ Add component"로 아무 엔티티에나 `orr_physics3d::Joint`를 붙인다. 조인트는 바디와 다른 자기 엔티티에 둔다.
- `a`, `b` 필드는 씬의 엔티티 목록에서 고른다(콤보 상자, "none"이 맨 위). 엔티티를 가리키는 다른 필드(`Scatter::terrain` 등)도 같은 방식이다. 플레이 중에는 읽기만 된다.
- `kind`(ball, rod, rope, hinge, prismatic, spring), `flags`(motor, limit), 고정점, 길이, 축, 각도 범위, 모터, `swing`을 고친다. 스프링은 진동수를 `motor_speed`, 감쇠비를 `max_motor_torque` 칸에 넣는다(인스펙터 설명에 적혀 있다).
- 씬 파일에는 이 필드들만 저장한다. `b` 쪽 축과 각도 기준(`axis_b`, `ref_a`, `ref_b`)은 저장하지 않는다. `step`이 각도 기준이 아직 없는 조인트(`ref_a`가 0)를 만나면 그 스텝 시작의 자세로 기록한다. 그래서 씬에 저장된 자세, 또는 인스펙터에서 조인트를 붙인 자세가 각도 0이다. `add_joint`로 만든 조인트와 결과가 비트 단위로 같다(테스트로 확인).
- 뷰포트는 조인트를 표지로 그린다. 두 고정점을 잇는 하늘색 선(스프링도 막대처럼 그린다), 첫 고정점의 작은 십자, 힌지와 프리즘 조인트와 제한된 볼 조인트는 축 방향의 주황색 선이다. 선택한 조인트는 굵게 그린다.

## 에디터와 씬 파일 (2D)

`orr_physics::register_joints_reflect`가 `orr_physics::Joint`를 리플렉션에 등록한다. 2D 물리 게임(`PhysGame`, 에디터의 `physics` 게임)은 `register_joints`와 함께 이것을 부른다. 그래서 이 게임의 씬에서는 3D와 같은 일이 된다.

- 인스펙터의 "+ Add component"로 `orr_physics::Joint`를 붙이고, `a`, `b`를 엔티티 목록에서 고른다.
- `kind`(pin, rod, rope, prismatic, spring), `flags`(motor, limit), 고정점, `length`, `lower`, `upper`, `motor_speed`, `max_motor_torque`를 고친다. 프리즘 조인트의 `length`는 미끄럼 축의 각도(라디안)이고, 스프링의 진동수와 감쇠비는 `motor_speed`와 `max_motor_torque` 칸에 넣는다.
- 씬 파일에는 `ref_angle`을 저장하지 않는다. `step`이 `ref_angle`이 아직 없는 조인트(씬 파일이나 인스펙터에서 온 조인트)를 만나면 그 스텝 시작의 상대 각도로 기록한다. 그래서 씬에 저장된 자세가 각도 0이다. 같은 자세에서 `add_joint`로 만든 조인트와 결과가 비트 단위로 같다(테스트로 확인). 없는 상태는 `ref_angle`이 100라디안인 값으로 표시한다. `add_joint`와 `Joint::new`로 만든 조인트는 이 값을 쓰지 않아서 예전과 똑같이 돈다.
- 프리즘 조인트의 `lower`, `upper`는 씬에서 `add_joint`가 저장하는 값 그대로다. 두 고정점이 벌어진 양을 축에 투영한 값이고, 고정점이 겹쳐 있으면 0이다. 씬에서 읽을 때는 시작 자세를 더하지 않는다.
- 시연 씬은 `scenes/joints_2d_demo.scene.yaml`이다(핀 진자, 모터 프로펠러, 미끄럼 블록, 스프링). `orr_editor --game physics --scene scenes/joints_2d_demo.scene.yaml`로 연다.
- 2D 뷰포트에는 조인트 표지가 없다. 조인트는 인스펙터에서만 고친다.

## 한계

- 조인트에 매달린 동적 바디는 잠들지 않는다(접촉으로 묶인 섬과 조인트를 합치지 않음). `add_joint`가 두 바디를 깨운다.
- 3D 볼 조인트의 스윙 제한은 원형 원뿔 하나다. 방향마다 각이 다른 타원 원뿔은 없다.
- 위치 보정이 속도 항(Baumgarte)이라 무거운 사슬은 늘어날 수 있다. 반복 횟수(`PhysicsConfig::velocity_iterations`)를 올리면 줄어든다.
- Yard3D와 `PhysGame` 외의 게임은 `register_joints`, `register_joints_reflect`를 직접 불러야 한다.
- 2D 뷰포트에는 조인트 표지가 없다. 고정점을 뷰포트에서 끌어 옮기는 기즈모도 없다. 숫자로 고친다.
- 인스펙터에서 조인트를 고치면 컴포넌트가 씬 값으로 다시 쓰여 기록이 지워지고, 다음 스텝에 그때 자세로 다시 기록된다. 코드에서 이미 기록된 조인트의 `axis_a`를 바꿀 때는 `ref_a`를 0으로 두어야 다시 기록된다.
