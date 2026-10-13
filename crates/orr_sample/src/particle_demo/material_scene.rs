//! Original, bounded sample geometry. No importer, filesystem or simulation writes.
use super::*;
use orr_model::{Dependency, IDENTITY, Image, ModelSource, Primitive, StaticModel, Vertex, Wrap};
use orr_render::{MaterialShading, ModelRenderer, ScalarMaterial, StaticInstance};
use sha2::{Digest, Sha256};

pub const MATERIAL_HELP: &str = "material_demo --materials legacy|showcase --particles off|dust|sparks [--headless] [--showcase]\nWASD/arrows: move and collect gold spheres. R: new session. Escape: exit.\nlegacy/showcase use identical original geometry, camera and light.\nshowcase: matte floor, coated blue player, metallic gold pickups.\n--showcase cycles genuine pickups; --headless validates CPU data only.\nScalar materials use approximate ambient; no environment reflections or full glTF PBR.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialMode {
    Legacy,
    Showcase,
}

pub struct MaterialOptions {
    pub materials: MaterialMode,
    pub particles: Options,
}
impl MaterialOptions {
    pub fn parse(args: Vec<String>) -> Result<Self, String> {
        let mut args = args.into_iter();
        let mut materials = None;
        let mut remaining = Vec::new();
        while let Some(arg) = args.next() {
            if arg == "--materials" {
                if materials.is_some() {
                    return Err(MATERIAL_HELP.into());
                }
                materials = Some(match args.next().as_deref() {
                    Some("legacy") => MaterialMode::Legacy,
                    Some("showcase") => MaterialMode::Showcase,
                    _ => return Err(MATERIAL_HELP.into()),
                });
            } else if arg == "--particles" {
                // Keep the option/value pair intact; never lift a material flag
                // out of another option's value position.
                remaining.push(arg);
                remaining.push(args.next().ok_or_else(|| MATERIAL_HELP.to_string())?);
            } else {
                remaining.push(arg);
            }
        }
        Ok(Self {
            materials: materials.ok_or_else(|| MATERIAL_HELP.to_string())?,
            particles: Options::parse(remaining).map_err(|_| MATERIAL_HELP.to_string())?,
        })
    }
}

const SEGMENTS: usize = 24;
const LATITUDES: usize = 16;
// This immutable recipe is the original source asset; its digest describes the
// authored parameters, not platform-identical libm results or a cooked-file hash.
const RECIPE: &str = "orr-material-demo-v1\nlicense=MIT OR Apache-2.0\nplane=xz[-1,1],normal=+Y,indices=0,2,1,0,3,2\nsphere=Y-up,unit-radius,24-azimuth,16-latitude-intervals,15-rings,unique-poles\nring=(sin(theta)*cos(phi),cos(theta),sin(theta)*sin(phi)),theta=pi*j/16,phi=tau*i/24\nwinding=outward;uv=(i/24,j/16);white=255,255,255,255\nfloor-linear=.09,.13,.2,1;player-linear=.1,.42,.95,1;pickup-linear=1,.76,.08,1\n";
const COLORS: [[f32; 4]; 3] = [
    [0.09, 0.13, 0.2, 1.0],
    [0.1, 0.42, 0.95, 1.0],
    [1.0, 0.76, 0.08, 1.0],
];
const PRESETS: [(f32, f32); 3] = [(0.0, 0.75), (0.0, 0.25), (1.0, 0.18)];

fn vertex(position: [f32; 3], normal: [f32; 3], uv: [f32; 2]) -> Vertex {
    Vertex {
        position,
        normal,
        uv,
    }
}
fn plane() -> (Vec<Vertex>, Vec<u32>) {
    (
        [
            [-1.0, 0.0, -1.0],
            [1.0, 0.0, -1.0],
            [1.0, 0.0, 1.0],
            [-1.0, 0.0, 1.0],
        ]
        .into_iter()
        .map(|p| vertex(p, [0.0, 1.0, 0.0], [p[0] * 0.5 + 0.5, p[2] * 0.5 + 0.5]))
        .collect(),
        vec![0, 2, 1, 0, 3, 2],
    )
}
fn sphere() -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = Vec::with_capacity(2 + SEGMENTS * (LATITUDES - 1));
    vertices.push(vertex([0.0, 1.0, 0.0], [0.0, 1.0, 0.0], [0.5, 0.0]));
    for j in 1..LATITUDES {
        let theta = std::f32::consts::PI * j as f32 / LATITUDES as f32;
        for i in 0..SEGMENTS {
            let phi = std::f32::consts::TAU * i as f32 / SEGMENTS as f32;
            let p = [
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            ];
            vertices.push(vertex(
                p,
                p,
                [i as f32 / SEGMENTS as f32, j as f32 / LATITUDES as f32],
            ));
        }
    }
    let bottom = vertices.len() as u32;
    vertices.push(vertex([0.0, -1.0, 0.0], [0.0, -1.0, 0.0], [0.5, 1.0]));
    let ring = |j: usize, i: usize| (1 + j * SEGMENTS + i % SEGMENTS) as u32;
    let mut indices = Vec::with_capacity(6 * SEGMENTS * (LATITUDES - 1));
    for i in 0..SEGMENTS {
        indices.extend([0, ring(0, i + 1), ring(0, i)]);
        for j in 0..LATITUDES - 2 {
            let (a, b, c, d) = (
                ring(j, i),
                ring(j, i + 1),
                ring(j + 1, i),
                ring(j + 1, i + 1),
            );
            indices.extend([a, b, c, b, d, c]);
        }
        indices.extend([ring(LATITUDES - 2, i), ring(LATITUDES - 2, i + 1), bottom]);
    }
    (vertices, indices)
}
fn models() -> Result<[StaticModel; 3], String> {
    let digest = Sha256::digest(RECIPE.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let make = |index: usize| {
        let asset_id = format!("material-demo/recipe-v1-{index}.model");
        let (vertices, indices) = if index == 0 { plane() } else { sphere() };
        StaticModel::new(ModelSource {
            format: "orr_static_model".into(),
            version: 1,
            asset_id: asset_id.clone(),
            dependencies: vec![Dependency {
                uri: "$source".into(),
                sha256: digest.clone(),
            }],
            primitives: vec![Primitive {
                id: format!("{asset_id}#node=0/mesh=0/primitive=0"),
                vertices,
                indices,
                material: 0,
                transform: IDENTITY,
            }],
            materials: vec![orr_model::Material {
                base_color: COLORS[index],
                image: 0,
                linear_filter: false,
                wrap_s: Wrap::Clamp,
                wrap_t: Wrap::Clamp,
            }],
            images: vec![Image {
                width: 1,
                height: 1,
                rgba8: vec![255; 4],
            }],
        })
        .map_err(|error| error.to_string())
    };
    Ok([make(0)?, make(1)?, make(2)?])
}

struct Placements {
    floor: [StaticInstance; 1],
    player: Vec<StaticInstance>,
    pickups: Vec<StaticInstance>,
}
impl Placements {
    fn new() -> Self {
        Self {
            floor: [StaticInstance {
                translation: [0.0; 3],
                scale: [26.0, 1.0, 26.0],
                ..Default::default()
            }],
            player: Vec::with_capacity(1),
            pickups: Vec::with_capacity(PICKUPS.len()),
        }
    }
    fn extract(&mut self, demo: &Demo) -> Result<(), String> {
        self.player.clear();
        self.pickups.clear();
        let snapshot = demo
            .snapshot
            .as_ref()
            .ok_or("material preview requires a snapshot")?;
        for (_, actor) in snapshot.predicted().iter::<CollectActor>() {
            self.push_actor(actor)?;
        }
        if self.player.len() != 1 {
            return Err("material preview requires exactly one player".into());
        }
        Ok(())
    }
    fn push_actor(&mut self, actor: &CollectActor) -> Result<(), String> {
        if actor.active == 0 {
            return Ok(());
        }
        let (output, limit) = match actor.kind {
            PLAYER => (&mut self.player, 1),
            COLLECTIBLE => (&mut self.pickups, PICKUPS.len()),
            _ => return Err("material preview encountered an unsupported actor tag".into()),
        };
        if output.len() == limit {
            return Err("material preview actor bound exceeded".into());
        }
        output.push(StaticInstance {
            translation: [
                fp_to_f32(actor.position.x) * 0.1,
                0.2,
                fp_to_f32(actor.position.y) * 0.1,
            ],
            scale: [0.2; 3],
            ..Default::default()
        });
        Ok(())
    }
}

pub(super) fn validate_headless(demo: &Demo) -> Result<(), String> {
    models()?;
    Placements::new().extract(demo)
}

pub(super) struct MaterialScene<B: Rhi> {
    scene: ImportedSceneRenderer<B>,
    floor: ModelRenderer<B>,
    player: ModelRenderer<B>,
    pickups: ModelRenderer<B>,
    placements: Placements,
    particles: Option<ParticleRenderer<B>>,
}
impl<B: Rhi> MaterialScene<B> {
    pub(super) fn new(
        gpu: &B,
        format: TextureFormat,
        particles: Mode,
        mode: MaterialMode,
    ) -> Result<Self, String> {
        // Admission and scalar validation precede all renderer/device allocations.
        let [floor_model, player_model, pickup_model] = models()?;
        let scalars = PRESETS.map(|(metal, rough)| {
            ScalarMaterial::new(metal, rough).expect("bounded authored preset")
        });
        let scene = ImportedSceneRenderer::new(gpu.clone(), format).map_err(|e| e.to_string())?;
        let make = |model, scalar| {
            match mode {
                MaterialMode::Legacy => {
                    ModelRenderer::new(gpu.clone(), scene.scene_format(), model)
                }
                MaterialMode::Showcase => ModelRenderer::new_with_materials(
                    gpu.clone(),
                    scene.scene_format(),
                    model,
                    &[MaterialShading::MetallicRoughness(scalar)],
                ),
            }
            .map_err(|e| e.to_string())
        };
        // White image bytes are identical; each renderer owns its GPU resources.
        let floor = make(floor_model, scalars[0])?;
        let player = make(player_model, scalars[1])?;
        let pickups = make(pickup_model, scalars[2])?;
        let particles = if particles == Mode::Off {
            None
        } else {
            Some(
                ParticleRenderer::new(gpu.clone(), scene.scene_format())
                    .map_err(|e| e.to_string())?,
            )
        };
        Ok(Self {
            scene,
            floor,
            player,
            pickups,
            placements: Placements::new(),
            particles,
        })
    }
    pub(super) fn draw(
        &mut self,
        view: &B::TextureView,
        size: (u32, u32),
        demo: &Demo,
    ) -> Result<(), String> {
        self.draw_grounded(view, size, demo, true)
    }
    fn draw_grounded(
        &mut self,
        view: &B::TextureView,
        size: (u32, u32),
        demo: &Demo,
        shadows: bool,
    ) -> Result<(), String> {
        self.placements.extract(demo)?;
        let (camera, mut lighting) = grounded_view(
            self.placements.player[0].translation,
            size,
            demo.list.lighting,
        )?;
        lighting.shadows = shadows;
        let target = ImportedSceneTarget {
            view,
            size,
            format: self.scene.format(),
            sample_count: 1,
        };
        let mut batches = [
            ImportedBatch::StaticInstances {
                renderer: &mut self.floor,
                instances: &self.placements.floor,
            },
            ImportedBatch::StaticInstances {
                renderer: &mut self.player,
                instances: &self.placements.player,
            },
            ImportedBatch::StaticInstances {
                renderer: &mut self.pickups,
                instances: &self.placements.pickups,
            },
        ];
        let particles = self.particles.as_mut().map(|renderer| ParticleBatch {
            renderer,
            items: &demo.draws,
        });
        self.scene
            .draw_with_particles(
                target,
                &camera,
                &lighting,
                &PointLightSettings::default(),
                &mut batches,
                particles,
            )
            .map_err(|e| e.to_string())
    }
}

// Presentation follows the authoritative player; simulation and procedural lighting are untouched.
fn grounded_view(
    player: [f32; 3],
    size: (u32, u32),
    mut light: orr_render::Lighting,
) -> Result<(Camera3D, orr_render::Lighting), String> {
    if size.0 == 0 || size.1 == 0 || size.0 > 8192 || size.1 > 8192 {
        return Err(orr_render::ImportedSceneError::InvalidTarget.to_string());
    }
    let aspect = size.0 as f32 / size.1 as f32;
    let half_tan = 20.0_f32.to_radians().tan() * (2.0 / aspect).min(1.0);
    let fov_degrees = (2.0 * half_tan.atan()).to_degrees();
    let eye = [player[0] + 2.4, player[1] + 3.6, player[2] - 1.8];
    light.shadows = true;
    light.shadow_center = player;
    light.shadow_radius = 6.5;
    Ok((Camera3D::perspective(eye, player, fov_degrees), light))
}

#[cfg(test)]
#[path = "material_scene_tests.rs"]
mod tests;
