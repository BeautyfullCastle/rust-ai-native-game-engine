# 에디터에서 뷰 에셋 고르기 (`ViewRef`)

엔티티의 `orr_gfx::ViewRef`에 뷰 에셋(`*.orrview`)을 고르면, 에디터 뷰포트가 그 엔티티를 에셋의 모델로 그린다. 설계는 [view-framework.md](view-framework.md) 11장과 12장 6b단계다.

## 열기

```sh
cargo run --release -p orr_editor --features full -- \
  --game view-yard3d --scene scenes/view_yard3d/view_yard3d.scene.yaml
```

모델을 그리려면 `models` 기능이 필요하다(`full`에 들어 있다). `models`가 없으면 인스펙터 선택기와 편집은 되고 모델 대신 자리 표시 상자만 보인다.

데모 씬 `scenes/view_yard3d/`에는 바닥과 폰 둘이 있다. `blue_pawn`은 `blue.orrview`를 가리켜 모델로 그려지고, `red_pawn`은 에셋이 없어서(`asset: 0`) 상자로 보인다. 계층 창에서 `red_pawn`을 고르고 인스펙터의 `orr_gfx::ViewRef` 칸에서 `red`를 고르면 그 자리에 빨간 폰 모델이 나타난다. 한 번 고르는 것이 되돌리기 한 단계다. 모델 파일 `blue.glb`와 `red.glb`는 `orr_app`의 `avatars` 예제와 같은 CC0 파일이다(`LICENSE.txt`).

## 어떻게 동작하나

- **게임.** 에디터 게임 `ViewYard3D`(`orr_games::view_yard3d_game`)는 Yard3D의 물리, 조인트, `orr_gfx` 컴포넌트에 `ViewRef`를 더한 게임이다. 플레이어 입력, 비, 용 같은 게임 규칙은 없고 물리 시스템만 돈다. `Yard3D`는 `ViewRef`를 등록하지 않는다. 프레임 배치가 체크섬에 들어가서 등록하면 `Yard3D`의 골든 체크섬이 바뀌기 때문이다. 호스트는 `orr_remote::view_yard3d`다.
- **고르기.** 인스펙터는 씬 파일이 있는 폴더의 `*.orrview` 파일 이름을 목록으로 보여 준다. 고른 파일 경로의 해시가 `ViewRef.asset`에 들어간다.
- **그리기.** 뷰포트는 프레임에서 `Transform3`과 `ViewRef`가 있는 엔티티를 읽는다(`YardFrame::view_refs`). 번호가 씬 폴더의 뷰 에셋과 맞으면 에셋의 모델을 `Transform3`의 자리와 크기에 에셋의 `scale`을 곱해서 그린다. 그 엔티티의 자리 표시 상자는 숨긴다. 번호가 0이거나 맞는 에셋이 없으면 상자가 남고, 맞는 에셋이 없을 때는 뷰포트 아래에 이유가 보인다. 코드는 `orr_editor/src/view_models.rs`다.
- **한도.** `orr_app` 창과 같은 한도를 지킨다. 한 모델은 인스턴스 256개와 primitive 그리기 4096번까지, 모든 모델을 합쳐 남은 그리기 예산까지, 뷰포트가 받는 서로 다른 모델 8개까지다. 넘으면 앞에 있는 엔티티부터 남기고 나머지는 상자로 둔다.
- **파일이 바뀔 때.** 뷰 에셋 폴더는 0.5초마다 다시 보고, 모델 파일은 수정 시각과 크기가 바뀔 때만 다시 읽는다.

## 확인

```sh
cargo test --release -j 8 -p orr_editor --features full --test view_ref_models
```

창 없는 테스트가 데모 씬을 열고, 인스펙터에서 에셋을 골라 문서에 번호가 들어갔는지와 뷰포트의 모델 목록에 그 모델이 있는지를 본다. GPU가 있으면 뷰포트 그림이 바뀌는지도 본다. `ORR_SHOT_DIR`을 주면 그 폴더에 `editor_viewref.png`를 저장한다.

## 남은 것

- 몸(`Body`)이 있는 엔티티의 `ViewRef`는 에디터에서 모델로 그리지 않는다(몸의 모양이 그대로 보인다). `orr_app` 창은 몸이 자리를 정하고 모델이 모양을 대신한다.
- `Yard3D`, 지형, 내비게이션, 메시 게임 같은 다른 에디터 게임은 `ViewRef`를 등록하지 않아서 그 게임의 씬에는 이 기능이 없다.
- 뷰 에셋의 `tint`, `sound`, `entity_view`는 읽기만 한다. 애니메이션 모델은 그리지 않는다.
- 그리기 한도를 정하는 함수가 `orr_app/src/models.rs`의 `fit_models`와 따로 있다. 두 곳을 한 함수로 합치면 좋다.
