//! What a game tells the default 3D renderer: which entities to draw and how, the
//! sun and the sky light, and where the camera looks.
//!
//! [`Look3D`] is a ready-made [`View3D`]. A game that needs nothing else calls
//! `.view3d(Look3D::default())` and gets a picture.

use std::collections::BTreeMap;
use std::rc::Rc;

use orr_bridge::FrameView;
use orr_ecs::Entity;
use orr_gfx::{Material, Transform3 as PlacedTransform, ViewRef};
use orr_gfx_view::{ViewAsset, ViewAssets};
use orr_physics3d::{Body, Collider, BODY_DYNAMIC, SHAPE_BOX, SHAPE_CAPSULE, SLEEP_FLAG};
use orr_render::{Camera3D, Lighting, Material as SurfaceMaterial, OrbitCamera, RenderList3D, StaticInstance};
use orr_view::{fp_to_f32, fp_to_transform3, fp_to_vec3, Extracted3, Extractor3, InterpMode, Quat, RenderItem3, Shape3, Style3, Transform3, Vec3, ViewWorld3};

const WALL_COLOR: [f32; 3] = [0.20, 0.24, 0.34];
const GROUND_COLOR: [f32; 3] = [0.30, 0.34, 0.28];
const SPHERE_COLOR: [f32; 3] = [0.05, 0.50, 0.48];
const BOX_COLOR: [f32; 3] = [0.80, 0.26, 0.05];
const CAPSULE_COLOR: [f32; 3] = [0.50, 0.12, 0.70];
/// The color of an entity that has a [`Material`] and a placement but no physics body.
const MARKED_COLOR: [f32; 3] = [0.62, 0.64, 0.70];
/// Speed (units per second) at which a moving body is shown at full brightness.
const FULL_BRIGHT_SPEED: f32 = 6.0;
/// A static box at least this wide and deep (half extents, in units) is the ground.
const GROUND_MIN_HALF: f32 = 20.0;

/// The game's 3D look. The [`Extractor3`] part reads drawable entities out of a sim frame
/// (shape, color, and how the entity is smoothed between ticks). The methods below add the
/// rest of the picture and have a default each. All of them only read the frame.
pub trait View3D: Extractor3 {
    /// Where the camera looks in the newest predicted frame. By default the scene's own
    /// camera (the active `orr_gfx::Camera` with the highest priority), else [`default_orbit`].
    fn camera(&self, frame: FrameView<'_>) -> Camera3D {
        scene_camera(frame, &default_orbit())
    }

    /// The sun and the ambient light when the scene has no `orr_gfx::Light` of its own
    /// (a scene light, environment or post effect replaces this). By default [`default_lighting`].
    fn lighting(&self, frame: FrameView<'_>) -> Lighting {
        let _ = frame;
        default_lighting()
    }

    /// The view assets that the `ViewRef` ids of the scene resolve to. An entity whose id is
    /// here is drawn with the model of its asset. An entity whose id is not here (or a view
    /// without assets, the default) keeps its stand-in box, so a missing asset can be seen.
    fn view_assets(&self) -> Option<&ViewAssets> {
        None
    }
}

/// A model to draw this frame: where, and which file. The draw list of the 3D look holds these
/// next to the shapes (see `fill_draw_list3`); the window turns them into draws of the model.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelDraw {
    /// The view asset that asked for the model.
    pub asset: u64,
    /// Model file, relative to the asset folder.
    pub model: String,
    /// Position, rotation and scale in the world, smoothed like any other entity.
    pub instance: StaticInstance,
}

/// The camera of the scene (`orr_gfx::Camera`), or `fallback` when the scene has none.
pub fn scene_camera(frame: FrameView<'_>, fallback: &OrbitCamera) -> Camera3D {
    orr_gfx_view::main_camera(frame).map_or_else(|| fallback.camera(), |look| look.camera)
}

/// The camera of [`Look3D`] when nothing else is set: it orbits a point just above the
/// origin from the front right, high enough to see a floor of about 50 by 50 units.
pub fn default_orbit() -> OrbitCamera {
    OrbitCamera::new([0.0, 1.0, 0.0], 0.55, 0.64, 40.0)
}

/// The sun of [`Look3D`] when nothing else is set: a low afternoon sun with shadows over
/// a radius of 36 units around the origin, and a blue sky light.
pub fn default_lighting() -> Lighting {
    Lighting {
        direction: [-0.42, -0.72, -0.55],
        color: [1.0, 0.94, 0.82],
        intensity: 2.1,
        sky: [0.50, 0.66, 1.0],
        ground: [0.26, 0.24, 0.22],
        ambient: 0.30,
        shadows: true,
        shadow_center: [0.0, 0.0, 0.0],
        shadow_radius: 36.0,
        tonemap: true,
        exposure: 1.0,
    }
}

/// The default 3D look. It draws, without any code of the game:
///
/// - every entity with a physics `Body` and `Collider`: a sphere, box or capsule of the collider's
///   size, moving bodies brighter than resting ones, a large static box as a checkered ground;
/// - every entity with an `orr_gfx::Transform3` and an `orr_gfx::Material` or `orr_gfx::ViewRef` (and
///   no body): a box of the transform's scale, as a stand-in for its model. The surface shader of
///   the material is not applied yet.
/// - an entity with a `ViewRef` whose asset the look knows ([`Look3D::assets`]) is drawn with the
///   model of the asset instead of its shape, at its (smoothed) place, scaled by its
///   `Transform3` (a body keeps scale 1) and by the scale of the asset.
///
/// The scene's lights, camera and environment (`orr_gfx` components) are used when the scene has
/// them. Else the sun is [`default_lighting`] and the camera is [`default_orbit`]. Both can be replaced.
#[derive(Clone, Debug)]
pub struct Look3D {
    orbit: OrbitCamera,
    lighting: Lighting,
    assets: Option<ViewAssets>,
}

impl Default for Look3D {
    fn default() -> Self {
        Self { orbit: default_orbit(), lighting: default_lighting(), assets: None }
    }
}

impl Look3D {
    /// The default look.
    pub fn new() -> Self {
        Self::default()
    }

    /// The camera to use when the scene has none.
    pub fn orbit(mut self, orbit: OrbitCamera) -> Self {
        self.orbit = orbit;
        self
    }

    /// The sun and ambient light to use when the scene has no light of its own.
    pub fn lighting(mut self, lighting: Lighting) -> Self {
        self.lighting = lighting;
        self
    }

    /// The view assets of the project: an entity with a `ViewRef` is drawn with the model of
    /// its asset (see [`ViewAssets::load_dir`]).
    pub fn assets(mut self, assets: ViewAssets) -> Self {
        self.assets = Some(assets);
        self
    }
}

impl Extractor3 for Look3D {
    fn extract(&self, frame: FrameView<'_>, out: &mut Vec<Extracted3>) {
        extract_default(frame, out);
    }
}

impl View3D for Look3D {
    fn camera(&self, frame: FrameView<'_>) -> Camera3D {
        scene_camera(frame, &self.orbit)
    }

    fn lighting(&self, frame: FrameView<'_>) -> Lighting {
        let _ = frame;
        self.lighting
    }

    fn view_assets(&self) -> Option<&ViewAssets> {
        self.assets.as_ref()
    }
}

/// Bodies with a collider, then marked entities without a body. Dynamic bodies are predicted
/// (rollback corrections are smoothed away), the rest shows the newest tick as it is.
pub fn extract_default(frame: FrameView<'_>, out: &mut Vec<Extracted3>) {
    if frame.registers::<Body>() && frame.registers::<Collider>() {
        out.reserve(frame.count::<Body>() as usize);
        for (entity, body) in frame.iter::<Body>() {
            let Some(collider) = frame.get::<Collider>(entity) else { continue };
            out.push(extract_body(entity, body, collider, orr_gfx_view::layers_of(frame, entity)));
        }
    }
    if frame.registers::<Material>() && frame.registers::<PlacedTransform>() {
        for (entity, _) in frame.iter::<Material>() {
            if frame.registers::<Body>() && frame.has::<Body>(entity) {
                continue;
            }
            let Some(placed) = frame.get::<PlacedTransform>(entity) else { continue };
            out.push(extract_marked(entity, placed, orr_gfx_view::layers_of(frame, entity)));
        }
    }
    if frame.registers::<ViewRef>() && frame.registers::<PlacedTransform>() {
        for (entity, _) in frame.iter::<ViewRef>() {
            let has_material = frame.registers::<Material>() && frame.has::<Material>(entity);
            let has_body = frame.registers::<Body>() && frame.has::<Body>(entity);
            if has_material || has_body {
                continue;
            }
            let Some(placed) = frame.get::<PlacedTransform>(entity) else { continue };
            out.push(extract_marked(entity, placed, orr_gfx_view::layers_of(frame, entity)));
        }
    }
}

/// The rotation of `orr_gfx::Transform3::rotation` (pitch, yaw, roll in degrees; all zero faces +z,
/// positive pitch turns the entity downward) as a quaternion: roll, then pitch, then yaw.
fn euler_degrees(degrees: [f32; 3]) -> Quat {
    let [pitch, yaw, roll] = degrees.map(f32::to_radians);
    let about = |axis: [f32; 3], angle: f32| Quat::from_axis_angle(Vec3::new(axis[0], axis[1], axis[2]), angle);
    about([0.0, 1.0, 0.0], yaw) * about([1.0, 0.0, 0.0], pitch) * about([0.0, 0.0, 1.0], roll)
}

fn extract_marked(entity: orr_ecs::Entity, placed: &PlacedTransform, layers: u32) -> Extracted3 {
    let scale = [fp_to_f32(placed.scale.x), fp_to_f32(placed.scale.y), fp_to_f32(placed.scale.z)];
    let degrees = [fp_to_f32(placed.rotation.x), fp_to_f32(placed.rotation.y), fp_to_f32(placed.rotation.z)];
    let half = scale.map(|s| (s * 0.5).abs().max(1e-3));
    let mut style = Style3::new(Shape3::Box { half }, MARKED_COLOR);
    style.layers = layers;
    Extracted3 { entity, transform: Transform3::new(fp_to_vec3(placed.position), euler_degrees(degrees)), mode: InterpMode::Prediction, style }
}

/// `layers` are the entity's render layers (see `orr_gfx::RenderLayers`).
fn extract_body(entity: orr_ecs::Entity, body: &Body, collider: &Collider, layers: u32) -> Extracted3 {
    let shape = &collider.shape;
    let mut transform = fp_to_transform3(body.pos, body.rot);
    let dynamic = body.kind == BODY_DYNAMIC;
    let (shape3, base, material) = match shape.kind {
        SHAPE_BOX => {
            let half = [fp_to_f32(shape.half.x), fp_to_f32(shape.half.y), fp_to_f32(shape.half.z)];
            if !dynamic && half[0] > GROUND_MIN_HALF && half[2] > GROUND_MIN_HALF {
                // The floor slab: draw its top face as a checkered plane.
                transform.pos.y += half[1];
                let style = Style3 {
                    shape: Shape3::Plane { half_x: half[0], half_z: half[2] },
                    color: [GROUND_COLOR[0], GROUND_COLOR[1], GROUND_COLOR[2], 1.0],
                    roughness: 0.95,
                    metallic: 0.0,
                    checker: true,
                    layers,
                };
                return Extracted3 { entity, transform, mode: InterpMode::None, style };
            }
            (Shape3::Box { half }, if dynamic { BOX_COLOR } else { WALL_COLOR }, (0.7, 0.0))
        }
        SHAPE_CAPSULE => (Shape3::Capsule { half_length: fp_to_f32(shape.half.y), radius: fp_to_f32(shape.radius) }, CAPSULE_COLOR, (0.35, 0.45)),
        _ => (Shape3::Sphere { radius: fp_to_f32(shape.radius) }, SPHERE_COLOR, (0.3, 0.1)),
    };
    let k = if dynamic {
        // Moving bodies are bright, resting ones dim, sleeping ones dimmest.
        let speed = fp_to_vec3(body.vel).length();
        let asleep = if body.sleep & SLEEP_FLAG != 0 { 0.85 } else { 1.0 };
        (0.62 + 0.38 * (speed / FULL_BRIGHT_SPEED).min(1.0)) * asleep
    } else {
        1.0
    };
    let style = Style3 { shape: shape3, color: [base[0] * k, base[1] * k, base[2] * k, 1.0], roughness: material.0, metallic: material.1, checker: false, layers };
    Extracted3 { entity, transform, mode: if dynamic { InterpMode::Prediction } else { InterpMode::None }, style }
}

/// The view shared by the extracting world and the draw-list builder.
#[derive(Clone)]
pub(crate) struct SharedView3(pub Rc<dyn View3D>);

impl Extractor3 for SharedView3 {
    fn extract(&self, frame: FrameView<'_>, out: &mut Vec<Extracted3>) {
        self.0.extract(frame, out);
    }
}

/// The view assets that the entities of a frame ask for.
struct ModelLookup<'a> {
    assets: &'a ViewAssets,
    refs: BTreeMap<Entity, u64>,
}

impl<'a> ModelLookup<'a> {
    /// `None` when the view has no assets or the game has no `ViewRef`: nothing to look up.
    fn new(assets: Option<&'a ViewAssets>, frame: FrameView<'_>) -> Option<Self> {
        let assets = assets.filter(|a| !a.is_empty())?;
        if !frame.registers::<ViewRef>() {
            return None;
        }
        Some(Self { assets, refs: frame.iter::<ViewRef>().map(|(entity, view_ref)| (entity, view_ref.asset)).collect() })
    }

    fn asset_of(&self, entity: Entity) -> Option<&'a ViewAsset> {
        self.assets.get(*self.refs.get(&entity)?)
    }
}

/// The model draw of `item`: its smoothed place, the scale of its `Transform3` (1 for a body,
/// which is sized by its collider) times the scale of the asset.
fn model_draw(item: &RenderItem3, asset: &ViewAsset, frame: FrameView<'_>) -> Option<ModelDraw> {
    let is_body = frame.registers::<Body>() && frame.has::<Body>(item.entity);
    let placed = if is_body { None } else { frame.get::<PlacedTransform>(item.entity) };
    let scale = placed.map_or([1.0; 3], |p| [fp_to_f32(p.scale.x), fp_to_f32(p.scale.y), fp_to_f32(p.scale.z)]).map(|s| s * asset.scale);
    if !scale.iter().all(|s| s.is_finite() && *s > 0.0) {
        return None;
    }
    let instance = StaticInstance { translation: item.transform.pos.to_array(), rotation: item.transform.rot.to_array(), scale };
    Some(ModelDraw { asset: asset.id, model: asset.model.clone(), instance })
}

/// Moves the view items into a render list (instances are sorted by mesh kind by the list).
/// An item whose entity asks for a known view asset becomes a model draw instead of a shape.
fn fill_items(items: &[RenderItem3], list: &mut RenderList3D, models: &mut Vec<ModelDraw>, lookup: Option<(&ModelLookup<'_>, FrameView<'_>)>) {
    for item in items {
        if let Some((lookup, frame)) = lookup {
            if let Some(draw) = lookup.asset_of(item.entity).and_then(|asset| model_draw(item, asset, frame)) {
                models.push(draw);
                continue;
            }
        }
        let (p, r) = (item.transform.pos.to_array(), item.transform.rot.to_array());
        let s = &item.style;
        let m = SurfaceMaterial { color: [s.color[0], s.color[1], s.color[2]], roughness: s.roughness, metallic: s.metallic, emissive: 0.0, checker: s.checker, layers: s.layers };
        match s.shape {
            Shape3::Sphere { radius } => list.sphere(p, r, radius, &m),
            Shape3::Box { half } => list.cuboid(p, r, half, &m),
            Shape3::Capsule { half_length, radius } => list.capsule(p, r, half_length, radius, &m),
            Shape3::Plane { half_x, half_z } => list.plane(p, half_x, half_z, &m),
        }
    }
}

/// Fills `list` with the lights of the scene (or the view's sun when the scene has none),
/// then every entity of `world`, and returns the camera. The entities with a known view asset
/// go to `models` and not to `list`.
pub(crate) fn fill_draw_list3(
    view: &SharedView3,
    world: &ViewWorld3<SharedView3>,
    frame: FrameView<'_>,
    items: &mut Vec<RenderItem3>,
    list: &mut RenderList3D,
    models: &mut Vec<ModelDraw>,
) -> Camera3D {
    items.clear();
    world.render_items(items);
    list.clear();
    models.clear();
    orr_gfx_view::scene_look(frame, &view.0.lighting(frame)).apply(list);
    let lookup = ModelLookup::new(view.0.view_assets(), frame);
    fill_items(items, list, models, lookup.as_ref().map(|l| (l, frame)));
    view.0.camera(frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_ecs::{ComponentRegistryBuilder, Frame};
    use orr_fp::{fp, FPVec3, FP};
    use orr_physics3d::Shape;
    use orr_render::MeshKind;
    use orr_view::ViewConfig;

    fn frame() -> Frame {
        let mut registry = ComponentRegistryBuilder::new();
        orr_physics3d::register(&mut registry);
        orr_gfx::register(&mut registry);
        orr_gfx::register_view_ref(&mut registry);
        Frame::new(registry.build())
    }

    fn asset(path: &str, model: &str, scale: f32) -> ViewAsset {
        ViewAsset::parse(path, &format!(r#"{{"version":1,"model":"{model}","scale":{scale}}}"#)).unwrap()
    }

    /// The model draws and the shape list of `frame` as the 3D look makes them.
    fn drawn(look: Look3D, frame: &Frame) -> (Vec<ModelDraw>, RenderList3D) {
        let view = SharedView3(Rc::new(look));
        let mut world = ViewWorld3::new(view.clone(), ViewConfig::default());
        let mut now = Vec::new();
        view.extract(FrameView::of(frame), &mut now);
        world.push_predicted(1, &[], &now, false);
        world.advance(0.0);
        let (mut items, mut list, mut models) = (Vec::new(), RenderList3D::new(), Vec::new());
        fill_draw_list3(&view, &world, FrameView::of(frame), &mut items, &mut list, &mut models);
        (models, list)
    }

    fn avatar(frame: &mut Frame, path: &str, position: FPVec3, scale: FP) -> orr_ecs::Entity {
        let entity = frame.spawn();
        frame.add(entity, PlacedTransform { position, scale: FPVec3::new(scale, scale, scale), ..Default::default() });
        frame.add(entity, ViewRef::at(path));
        entity
    }

    fn dynamic(frame: &mut Frame, pos: FPVec3, shape: Shape) -> orr_ecs::Entity {
        let entity = frame.spawn();
        frame.add(entity, Body::new_dynamic(pos, &shape, fp!(1)));
        frame.add(entity, Collider::new(shape));
        entity
    }

    fn fixed(frame: &mut Frame, shape: Shape) -> orr_ecs::Entity {
        let entity = frame.spawn();
        frame.add(entity, Body::new_static(FPVec3::ZERO));
        frame.add(entity, Collider::new(shape));
        entity
    }

    fn extracted(frame: &Frame) -> Vec<Extracted3> {
        let mut out = Vec::new();
        extract_default(FrameView::of(frame), &mut out);
        out
    }

    #[test]
    fn bodies_become_shapes_of_their_collider_and_a_big_static_box_becomes_the_ground() {
        let mut f = frame();
        let ball = dynamic(&mut f, FPVec3::new(fp!(1), fp!(2), fp!(3)), Shape::sphere(fp!(0.5)));
        let ground = fixed(&mut f, Shape::cuboid(fp!(24), fp!(0.5), fp!(24)));
        let wall = fixed(&mut f, Shape::cuboid(fp!(1), fp!(2), fp!(3)));

        let items = extracted(&f);

        let of = |e| items.iter().find(|i| i.entity == e).unwrap();
        assert_eq!(of(ball).style.shape, Shape3::Sphere { radius: 0.5 });
        assert_eq!((of(ball).transform.pos.to_array(), of(ball).mode), ([1.0, 2.0, 3.0], InterpMode::Prediction));
        assert_eq!(of(ground).style.shape, Shape3::Plane { half_x: 24.0, half_z: 24.0 });
        assert!(of(ground).style.checker);
        assert_eq!(of(ground).transform.pos.y, 0.5);
        assert_eq!((of(wall).style.shape, of(wall).mode), (Shape3::Box { half: [1.0, 2.0, 3.0] }, InterpMode::None));
    }

    #[test]
    fn an_entity_with_a_material_and_a_placement_is_a_box_of_its_scale_and_a_body_is_not_drawn_twice() {
        let mut f = frame();
        let marked = f.spawn();
        let placed = PlacedTransform { position: FPVec3::new(fp!(4), fp!(0), fp!(0)), scale: FPVec3::new(fp!(2), fp!(4), fp!(6)), ..Default::default() };
        f.add(marked, placed);
        f.add(marked, Material::new("lava"));
        let plain = f.spawn();
        f.add(plain, PlacedTransform::default());
        let with_body = fixed(&mut f, Shape::sphere(fp!(1)));
        f.add(with_body, PlacedTransform::default());
        f.add(with_body, Material::new("lava"));

        let items = extracted(&f);

        assert_eq!(items.len(), 2, "the plain placement is not drawn and the body once");
        let item = items.iter().find(|i| i.entity == marked).unwrap();
        assert_eq!(item.style.shape, Shape3::Box { half: [1.0, 2.0, 3.0] });
        assert_eq!(item.transform.pos.to_array(), [4.0, 0.0, 0.0]);
    }

    #[test]
    fn rotation_in_degrees_turns_forward_like_the_gfx_facing() {
        for degrees in [[0.0, 0.0, 0.0], [30.0, 0.0, 0.0], [0.0, 90.0, 0.0], [-20.0, 40.0, 15.0]] {
            let forward = euler_degrees(degrees).rotate(Vec3::new(0.0, 0.0, 1.0)).to_array();
            let expected = orr_gfx_view::facing(degrees);
            for i in 0..3 {
                assert!((forward[i] - expected[i]).abs() < 1e-5, "{degrees:?}: {forward:?} vs {expected:?}");
            }
        }
    }

    #[test]
    fn a_frame_without_physics_or_gfx_components_draws_nothing_and_does_not_panic() {
        let f = Frame::new(ComponentRegistryBuilder::new().build());
        assert!(extracted(&f).is_empty());
    }

    #[test]
    fn the_draw_list_gets_the_view_sun_the_entities_and_the_default_camera() {
        let mut f = frame();
        dynamic(&mut f, FPVec3::ZERO, Shape::sphere(fp!(1)));
        fixed(&mut f, Shape::cuboid(fp!(1), fp!(1), fp!(1)));
        let view = SharedView3(Rc::new(Look3D::default().lighting(Lighting { intensity: 7.0, ..default_lighting() })));
        let mut world = ViewWorld3::new(view.clone(), ViewConfig::default());
        let mut now = Vec::new();
        view.extract(FrameView::of(&f), &mut now);
        world.push_predicted(1, &[], &now, false);
        world.advance(0.0);
        let (mut items, mut list) = (Vec::new(), RenderList3D::new());

        let camera = fill_draw_list3(&view, &world, FrameView::of(&f), &mut items, &mut list, &mut Vec::new());

        assert_eq!(list.lighting.intensity, 7.0);
        assert_eq!((list.instances(MeshKind::Sphere).len(), list.instances(MeshKind::Box).len()), (1, 1));
        assert_eq!(camera, default_orbit().camera());
        // A second fill replaces the first.
        fill_draw_list3(&view, &world, FrameView::of(&f), &mut items, &mut list, &mut Vec::new());
        assert_eq!(list.instance_count(), 2);
    }

    #[test]
    fn an_entity_with_a_known_view_asset_is_drawn_as_its_model_and_not_as_a_box() {
        let mut f = frame();
        avatar(&mut f, "avatars/knight.orrview", FPVec3::new(fp!(4), fp!(0), fp!(-2)), fp!(2));
        let assets = ViewAssets::from_assets(".", [asset("avatars/knight.orrview", "models/knight.glb", 1.5)]).unwrap();

        let (models, list) = drawn(Look3D::default().assets(assets), &f);

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].asset, ViewRef::at("avatars/knight.orrview").asset);
        assert_eq!(models[0].model, "models/knight.glb");
        assert_eq!(models[0].instance.translation, [4.0, 0.0, -2.0]);
        assert_eq!(models[0].instance.scale, [3.0, 3.0, 3.0], "the scale of the entity times the scale of the asset");
        assert_eq!(list.instance_count(), 0, "no stand-in box next to the model");
    }

    #[test]
    fn an_entity_whose_asset_is_unknown_keeps_its_stand_in_box_so_the_gap_can_be_seen() {
        let mut f = frame();
        avatar(&mut f, "avatars/missing.orrview", FPVec3::ZERO, fp!(1));
        let assets = ViewAssets::from_assets(".", [asset("avatars/knight.orrview", "models/knight.glb", 1.0)]).unwrap();

        let (models, list) = drawn(Look3D::default().assets(assets), &f);
        assert!(models.is_empty());
        assert_eq!(list.instances(MeshKind::Box).len(), 1);

        // A look without assets draws the box too, for the same reason.
        let (models, list) = drawn(Look3D::default(), &f);
        assert!(models.is_empty());
        assert_eq!(list.instances(MeshKind::Box).len(), 1);
    }

    #[test]
    fn a_body_with_a_view_asset_is_drawn_as_the_model_at_scale_one_and_once() {
        let mut f = frame();
        let ball = dynamic(&mut f, FPVec3::new(fp!(1), fp!(2), fp!(3)), Shape::sphere(fp!(0.5)));
        f.add(ball, PlacedTransform { scale: FPVec3::new(fp!(9), fp!(9), fp!(9)), ..Default::default() });
        f.add(ball, ViewRef::at("ball.orrview"));
        let both = avatar(&mut f, "ball.orrview", FPVec3::new(fp!(7), fp!(0), fp!(0)), fp!(1));
        f.add(both, Material::new("lava"));
        let assets = ViewAssets::from_assets(".", [asset("ball.orrview", "ball.glb", 2.0)]).unwrap();

        let (models, list) = drawn(Look3D::default().assets(assets), &f);

        assert_eq!(models.len(), 2, "one model per entity, also with a material");
        let of_body = models.iter().find(|m| m.instance.translation == [1.0, 2.0, 3.0]).expect("the body is placed by its body");
        assert_eq!(of_body.instance.scale, [2.0; 3], "a body is sized by the asset only");
        assert_eq!(list.instance_count(), 0);
    }

    #[test]
    fn a_game_without_view_refs_draws_as_before() {
        let mut f = Frame::new({
            let mut registry = ComponentRegistryBuilder::new();
            orr_physics3d::register(&mut registry);
            registry.build()
        });
        dynamic(&mut f, FPVec3::ZERO, Shape::sphere(fp!(1)));
        let assets = ViewAssets::from_assets(".", [asset("a.orrview", "a.glb", 1.0)]).unwrap();

        let (models, list) = drawn(Look3D::default().assets(assets), &f);

        assert!(models.is_empty());
        assert_eq!(list.instances(MeshKind::Sphere).len(), 1);
    }

    #[test]
    fn the_camera_of_the_scene_wins_over_the_orbit() {
        let mut f = frame();
        let eye = f.spawn();
        let placed = PlacedTransform { position: FPVec3::new(fp!(0), fp!(5), fp!(-10)), ..Default::default() };
        f.add(eye, placed);
        f.add(eye, orr_gfx::Camera::default());

        let camera = Look3D::default().camera(FrameView::of(&f));

        assert_eq!(camera.eye, [0.0, 5.0, -10.0]);
        assert_ne!(camera, default_orbit().camera());
    }
}
