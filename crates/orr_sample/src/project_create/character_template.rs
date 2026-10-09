//! Original, installed Room courier fixture. Creator never executes the generator.
use orr_model_bindings::model_bindings::{
    self, AnimationDescriptor, Binding, Document, LocalTransform, ModelKind, PlaybackMode,
};
pub(super) const PACKAGE: &str = "sample-room-character";
pub(super) const SOURCE: &[(&str, &[u8])] = &[
    (
        "orr.package.json",
        include_bytes!("../../../../assets/room_character_demo/orr.package.json"),
    ),
    (
        "courier.orrmodel.json",
        include_bytes!("../../../../assets/room_character_demo/courier.orrmodel.json"),
    ),
    (
        "LICENSE.txt",
        include_bytes!("../../../../assets/room_character_demo/LICENSE.txt"),
    ),
    (
        "generate.py",
        include_bytes!("../../../../assets/room_character_demo/generate.py"),
    ),
];
pub(super) fn documents(
    project: &orr_package::Project,
    scene: &orr_reflect::Scene,
    mut models: Document,
) -> Result<(Document, crate::room_character::Document), String> {
    use orr_games::room_escape_game::{PLAYER, RoomActor};
    let prepared = crate::room_project::PreparedScene::parse(&scene.to_yaml())?;
    let player = prepared
        .frame()
        .entities()
        .find(|&entity| {
            prepared
                .frame()
                .get::<RoomActor>(entity)
                .is_some_and(|actor| actor.kind == PLAYER)
        })
        .ok_or("Room player missing")?;
    let player = prepared
        .index()
        .guid(player)
        .ok_or("Room player GUID missing")?
        .to_string();
    let asset = model_bindings::load_asset_from_project(
        project,
        PACKAGE,
        "courier.orrmodel.json",
        ModelKind::Animated,
    )?;
    let binding = Binding::from_animated_asset(
        PACKAGE.into(),
        "courier.orrmodel.json".into(),
        &asset,
        AnimationDescriptor {
            clip_index: 0,
            playback: PlaybackMode::Loop,
        },
        LocalTransform::default(),
    )?;
    models.bindings.insert(player.clone(), binding);
    let character = crate::room_character::Document {
        schema: 1,
        player,
        searching: 0,
        carrying: 1,
        escaped: 2,
    };
    character.validate()?;
    Ok((models, character))
}
