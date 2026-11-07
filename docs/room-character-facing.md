# Room character movement facing

Room character schema 7 can make the animated PLAYER model face its observed movement. This is a presentation option: it changes neither the gameplay Body rotation nor collision, replay inputs or checkpoints.

In the Room character panel, enable movement facing and choose its authored forward direction in Body-local space, after the binding local rotation: +Z, -Z, +X or -X. Apply, then use the existing paired Save action. The saved character policy is used by both the editor and the exported native Room runtime. Inspect the model with its authored local transform when selecting an axis; this option does not infer a mesh forward vector or compensate arbitrary import rotations.

Heading comes from net XZ displacement between forward observations of the same simulation source. It turns immediately; no smoothing or input-direction prediction is applied. A stopped character keeps its last observed direction. The first observation, Edit/Stop, a seek or backward time, restart, or a changed model/document/entity/source resets to the authored orientation until a new forward moving observation. A forward tick gap uses the observed net direction, which can differ from unseen intermediate turns; zero net displacement holds the prior heading. Repeated draws do not accumulate rotation. Vertical-only movement does not select a new heading.

The display transform is body placement × observed yaw × authored model-local TRS. Local translation therefore rotates around the Body origin along with the model; the binding itself is unchanged. Skin poses, playback rates, pickup and escaped phases keep their existing clocks. Models without the policy retain their prior orientation, including non-PLAYER models.

Schema 7 requires a nonnull closed facing-axis value inside motion, and may also retain the schema-6 pickup policy. Schemas 1–6 retain their existing contract and omit facing. Turning facing off preserves the other authored animation options.

Focused automated acceptance covers schema rejection and legacy compatibility, cardinal/diagonal headings, idle and discontinuity behavior, exact Frame preservation, UI Apply/history/paired Save/reopening and CPU/native source-independent placement. These checks do not establish Windows execution, GPU pixels or physical play coverage.