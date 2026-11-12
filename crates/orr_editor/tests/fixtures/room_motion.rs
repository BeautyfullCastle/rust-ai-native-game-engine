// Test-only derivative: append a named constant-rest idle clip without changing repository assets.
pub const PACKAGE: &str = "test-room-idle";
pub const ASSET: &str = "courier.orrmodel.json";
pub const IDLE: u32 = 3;
pub fn install(root: &std::path::Path, source: &std::path::Path) {
    use orr_model::animation::{
        AnimatedModel, AnimationChannel, AnimationClip, ChannelValues, Interpolation,
    };
    use orr_model_bindings::model_bindings::{self, ModelKind};
    use orr_package::{Project, Runtime};
    let loaded = model_bindings::load_asset_for_kind(
        root,
        "sample-room-character",
        ASSET,
        ModelKind::Animated,
    )
    .unwrap();
    let mut model = loaded.animated_model().unwrap().source().clone();
    assert_eq!(model.clips.len(), IDLE as usize);
    model.clips.push(AnimationClip {
        name: "idle".into(),
        channels: vec![AnimationChannel {
            node: 0,
            interpolation: Interpolation::Linear,
            times: vec![0.0, 1.0],
            values: ChannelValues::Translation(vec![model.nodes[0].rest.translation; 2]),
        }],
    });
    AnimatedModel::new(model.clone()).unwrap();
    std::fs::create_dir(source).unwrap();
    std::fs::write(source.join(ASSET), serde_json::to_vec(&model).unwrap()).unwrap();
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/room_character_demo/LICENSE.txt"),
        source.join("LICENSE.txt"),
    )
    .unwrap();
    std::fs::write(source.join("orr.package.json"),serde_json::to_vec(&serde_json::json!({"schema":1,"name":PACKAGE,"version":"1.0.0","engine":"*","capabilities":["animation"],"dependencies":{},"files":[ASSET,"LICENSE.txt"]})).unwrap()).unwrap();
    Project::open_for_install(root, Runtime::content_only().engine_version)
        .unwrap()
        .install(&[source.to_path_buf()])
        .unwrap();
}
