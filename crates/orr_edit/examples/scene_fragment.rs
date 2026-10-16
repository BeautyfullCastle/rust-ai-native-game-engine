//! Headless authoring example; writes only to the explicitly supplied directory.
//! cargo run -p orr_edit --example scene_fragment -- /tmp/orr-fragments

use orr_edit::{EditorDoc, FragmentTranslation2D, Op, Origin, PlayController, SceneFragment};
use orr_fp::{FPVec2, FP};
use orr_reflect::{TypeRegistry, Value};
use orr_sample::physics_game::{register_reflect, PhysGame};
use orr_session::ControlOp;
use orr_sim::Simulation;

fn types() -> TypeRegistry {
    let mut types = TypeRegistry::new();
    register_reflect(&mut types);
    types
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .ok_or("pass an output directory (for example /tmp/orr-fragments)")?;
    let output = std::path::Path::new(&output);
    let mut doc = EditorDoc::from_yaml(
        include_str!("../../../scenes/physics_demo.scene.yaml"),
        types(),
        Simulation::<PhysGame>::build_registry(),
        7,
    )?;
    let selected: Vec<_> = doc
        .scene()
        .entities
        .iter()
        .filter(|(_, e)| matches!(e.name.as_deref(), Some("body_01" | "body_02")))
        .map(|(g, _)| g.clone())
        .collect();
    assert_eq!(selected.len(), 2);
    let fragment = SceneFragment::capture(doc.scene(), &selected)?;
    std::fs::create_dir_all(output)?;
    let fragment_path = output.join("body-pair.scene.yaml");
    std::fs::write(&fragment_path, fragment.to_yaml())?;
    let fragment = SceneFragment::from_yaml(&std::fs::read_to_string(&fragment_path)?, &types())?;
    let place = |x, y| FragmentTranslation2D {
        component: "orr_physics::Body".into(),
        path: "pos".into(),
        delta: FPVec2::new(FP::from_int(x), FP::from_int(y)),
    };
    let first = doc.instantiate_fragment(&fragment, Some(&place(4, 2)), Origin::User)?;
    let second = doc.instantiate_fragment(&fragment, Some(&place(-4, 4)), Origin::User)?;
    let before_override = doc.checksum();
    doc.apply(
        Op::SetField {
            guid: first.guids[&selected[0]].clone(),
            component: "orr_physics::Body".into(),
            path: "vel".into(),
            value: Value::Vec2(FPVec2::new(FP::ONE, FP::ZERO)),
        },
        Origin::User,
    )?;
    let after_override = doc.checksum();
    doc.undo()?;
    assert_eq!(doc.checksum(), before_override);
    doc.redo()?;
    assert_eq!(doc.checksum(), after_override);
    // Undo the override and the entire second instance, then redo both.
    doc.undo()?;
    doc.undo()?;
    assert!(second
        .guids
        .values()
        .all(|g| !doc.scene().entities.contains_key(g)));
    doc.redo()?;
    doc.redo()?;
    assert_eq!(doc.checksum(), after_override);
    let scene_path = output.join("instanced.scene.yaml");
    std::fs::write(&scene_path, doc.save_yaml())?;
    let reopened = EditorDoc::from_yaml(
        &std::fs::read_to_string(&scene_path)?,
        types(),
        doc.frame_registry().clone(),
        7,
    )?;
    assert_eq!(reopened.scene(), doc.scene());
    assert_eq!(reopened.checksum(), doc.checksum());
    let run = || -> Result<u64, Box<dyn std::error::Error>> {
        let mut play =
            PlayController::<PhysGame>::start_play(&reopened, reopened.play_config(2, 60))?;
        play.control(ControlOp::Step(60));
        assert_eq!(play.session().head_tick(), 60);
        Ok(play.session().frame().checksum())
    };
    let played = run()?;
    assert_eq!(played, run()?);
    println!(
        "Saved {} independent entities to {}; 60-tick deterministic play checksum {played:#018x}",
        doc.scene().entities.len(),
        scene_path.display()
    );
    println!(
        "First instance: {:?}\nSecond instance: {:?}",
        first.guids, second.guids
    );
    Ok(())
}
