use orr_render::camera3d::{Camera3D, OrbitCamera, Projection as RenderProjection};
use serde::{Deserialize, Deserializer, Serialize};

pub(crate) const MAX_BYTES: usize = 16 * 1024;
const MAX_VIEWPORT: u32 = 16_384;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum Projection {
    Perspective,
    Orthographic { half_height: f32 },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Document {
    pub(crate) target: [f32; 3],
    pub(crate) yaw: f32,
    pub(crate) pitch: f32,
    pub(crate) distance: f32,
    pub(crate) fov_y_degrees: f32,
    pub(crate) projection: Projection,
    pub(crate) near: f32,
    pub(crate) far: f32,
}

fn deserialize_object<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Object<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Object<T> {
        type Value = T;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a mesh camera JSON object")
        }

        fn visit_map<M: serde::de::MapAccess<'de>>(self, map: M) -> Result<T, M::Error> {
            T::deserialize(serde::de::value::MapAccessDeserializer::new(map))
        }
    }
    deserializer.deserialize_map(Object(std::marker::PhantomData))
}

impl<'de> Deserialize<'de> for Projection {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        fn present_float<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<f32>, D::Error> {
            f32::deserialize(deserializer).map(Some)
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            #[serde(rename = "type")]
            kind: String,
            #[serde(default, deserialize_with = "present_float")]
            half_height: Option<f32>,
        }

        let wire: Wire = deserialize_object(deserializer)?;
        match (wire.kind.as_str(), wire.half_height) {
            ("perspective", None) => Ok(Self::Perspective),
            ("perspective", Some(_)) => Err(serde::de::Error::custom(
                "perspective projection forbids half_height",
            )),
            ("orthographic", Some(half_height)) => Ok(Self::Orthographic { half_height }),
            ("orthographic", None) => Err(serde::de::Error::missing_field("half_height")),
            (kind, _) => Err(serde::de::Error::unknown_variant(
                kind,
                &["perspective", "orthographic"],
            )),
        }
    }
}

impl<'de> Deserialize<'de> for Document {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            target: [f32; 3],
            yaw: f32,
            pitch: f32,
            distance: f32,
            fov_y_degrees: f32,
            projection: Projection,
            near: f32,
            far: f32,
        }

        let wire: Wire = deserialize_object(deserializer)?;
        Ok(Self {
            target: wire.target,
            yaw: wire.yaw,
            pitch: wire.pitch,
            distance: wire.distance,
            fov_y_degrees: wire.fov_y_degrees,
            projection: wire.projection,
            near: wire.near,
            far: wire.far,
        })
    }
}

#[derive(Serialize)]
struct BoundDocument<'a> {
    schema: u32,
    scene_filename: &'a str,
    graph_revision: [u8; 32],
    camera: &'a Document,
}

impl Document {
    pub(crate) fn from_orbit(orbit: OrbitCamera) -> Result<Self, String> {
        let document = Self {
            target: orbit.target,
            yaw: orbit.yaw,
            pitch: orbit.pitch,
            distance: orbit.distance,
            fov_y_degrees: orbit.fov_y_degrees,
            projection: Projection::Perspective,
            near: 0.1,
            far: 500.0,
        };
        document.validate()?;
        Ok(document)
    }

    pub(crate) fn with_projection(
        mut self,
        projection: Projection,
        near: f32,
        far: f32,
    ) -> Result<Self, String> {
        self.projection = projection;
        self.near = near;
        self.far = far;
        self.validate()?;
        Ok(self)
    }

    pub(crate) fn orbit(&self) -> OrbitCamera {
        OrbitCamera {
            target: self.target,
            yaw: self.yaw,
            pitch: self.pitch,
            distance: self.distance,
            fov_y_degrees: self.fov_y_degrees,
        }
    }

    pub(crate) fn orbit_delta(&self, orbit: &mut OrbitCamera, delta: [f32; 2]) {
        if !delta
            .iter()
            .all(|value| value.is_finite() && value.abs() <= 1_000_000.0)
            || self.camera(orbit, (1, 1)).is_err()
        {
            return;
        }
        let mut candidate = *orbit;
        let pi = std::f64::consts::PI;
        let yaw = orbit.yaw as f64 - delta[0] as f64 * 0.006;
        candidate.yaw = ((yaw + pi).rem_euclid(2.0 * pi) - pi) as f32;
        candidate.pitch = (orbit.pitch as f64 + delta[1] as f64 * 0.006).clamp(-1.5, 1.5) as f32;
        self.commit_orbit(orbit, candidate, (1, 1));
    }

    pub(crate) fn pan(&self, orbit: &mut OrbitCamera, delta: [f32; 2], size: (u32, u32)) {
        if !delta
            .iter()
            .all(|value| value.is_finite() && value.abs() <= 1_000_000.0)
        {
            return;
        }
        let Ok(camera) = self.camera(orbit, size) else {
            return;
        };
        let half_height = match camera.projection {
            RenderProjection::Perspective { fov_y, .. } => {
                orbit.distance as f64 * (fov_y as f64 * 0.5).tan()
            }
            RenderProjection::Orthographic { half_height, .. } => half_height as f64,
        };
        let per_pixel = 2.0 * half_height / size.1 as f64;
        let right = camera.right();
        let up = camera.camera_up();
        let mut candidate = *orbit;
        for component in 0..3 {
            let shift = (-(right[component] as f64) * delta[0] as f64
                + up[component] as f64 * delta[1] as f64)
                * per_pixel;
            candidate.target[component] = (orbit.target[component] as f64 + shift) as f32;
        }
        self.commit_orbit(orbit, candidate, size);
    }

    pub(crate) fn zoom(&self, orbit: &mut OrbitCamera, steps: f32) {
        if !steps.is_finite() || steps.abs() > 1_000_000.0 || self.camera(orbit, (1, 1)).is_err() {
            return;
        }
        let mut candidate = *orbit;
        let distance = (orbit.distance as f64).ln() + steps as f64 * 0.9f64.ln();
        candidate.distance = distance
            .clamp(
                (OrbitCamera::MIN_DISTANCE as f64).ln(),
                (OrbitCamera::MAX_DISTANCE as f64).ln(),
            )
            .exp() as f32;
        self.commit_orbit(orbit, candidate, (1, 1));
    }

    fn commit_orbit(&self, orbit: &mut OrbitCamera, candidate: OrbitCamera, size: (u32, u32)) {
        if self.camera(&candidate, size).is_ok() {
            *orbit = candidate;
        }
    }

    pub(crate) fn camera(&self, orbit: &OrbitCamera, size: (u32, u32)) -> Result<Camera3D, String> {
        self.validate()?;
        validate_pose(orbit.target, orbit.yaw, orbit.pitch, orbit.distance)?;
        let aspect = aspect(size)?;
        let eye = orbit.eye();
        let camera = match self.projection {
            Projection::Perspective => Camera3D {
                eye,
                target: orbit.target,
                up: [0.0, 1.0, 0.0],
                projection: RenderProjection::Perspective {
                    fov_y: self.fov_y_degrees.to_radians(),
                    near: self.near,
                    far: self.far,
                },
            },
            Projection::Orthographic { half_height } => {
                let scale = orbit.distance as f64 / self.distance as f64;
                let viewport_scale = 1.0 / f64::from(aspect.min(1.0));
                let world_half_height = (half_height as f64 * scale * viewport_scale) as f32;
                Camera3D {
                    eye,
                    target: orbit.target,
                    up: [0.0, 1.0, 0.0],
                    projection: RenderProjection::Orthographic {
                        half_height: world_half_height,
                        near: self.near,
                        far: self.far,
                    },
                }
            }
        };
        validate_camera(&camera, size)?;
        Ok(camera)
    }

    pub(crate) fn encode(
        &self,
        scene_filename: &str,
        graph_revision: [u8; 32],
    ) -> Result<Vec<u8>, String> {
        self.validate()?;
        validate_scene_filename(scene_filename)?;
        let mut bytes = serde_json::to_vec_pretty(&BoundDocument {
            schema: 1,
            scene_filename,
            graph_revision,
            camera: self,
        })
        .map_err(|error| format!("serialize mesh camera: {error}"))?;
        bytes.push(b'\n');
        if bytes.len() > MAX_BYTES {
            return Err(format!("mesh camera exceeds {MAX_BYTES} bytes"));
        }
        Ok(bytes)
    }

    pub(crate) fn decode(
        bytes: &[u8],
        scene_filename: &str,
        graph_revision: [u8; 32],
    ) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err(format!("mesh camera exceeds {MAX_BYTES} bytes"));
        }
        validate_scene_filename(scene_filename)?;
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            schema: u32,
            scene_filename: String,
            graph_revision: [u8; 32],
            camera: Document,
        }
        let mut deserializer = serde_json::Deserializer::from_slice(bytes);
        let bound: Wire = deserialize_object(&mut deserializer)
            .map_err(|error| format!("mesh camera JSON: {error}"))?;
        deserializer
            .end()
            .map_err(|error| format!("mesh camera JSON: {error}"))?;
        if bound.schema != 1
            || bound.scene_filename != scene_filename
            || bound.graph_revision != graph_revision
        {
            return Err("mesh camera belongs to another scene or graph revision".into());
        }
        bound.camera.validate()?;
        Ok(bound.camera)
    }

    fn validate(&self) -> Result<(), String> {
        validate_pose(self.target, self.yaw, self.pitch, self.distance)?;
        bounded("fov_y_degrees", self.fov_y_degrees, 5.0, 150.0)?;
        bounded("near", self.near, 0.001, 100.0)?;
        bounded("far", self.far, 0.001, 1_000_000.0)?;
        if self.far <= self.near {
            return Err("mesh camera far plane must exceed near plane".into());
        }
        if let Projection::Orthographic { half_height } = self.projection {
            bounded("half_height", half_height, 0.01, 10_000.0)?;
        }
        Ok(())
    }
}

fn validate_scene_filename(filename: &str) -> Result<(), String> {
    if filename.is_empty()
        || filename.len() > 255
        || filename == "."
        || filename == ".."
        || filename
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\' | ':'))
    {
        return Err("mesh camera requires a bounded scene filename".into());
    }
    Ok(())
}

fn validate_pose(target: [f32; 3], yaw: f32, pitch: f32, distance: f32) -> Result<(), String> {
    for coordinate in target {
        bounded("target", coordinate, -1_000_000.0, 1_000_000.0)?;
    }
    bounded("yaw", yaw, -1_000_000.0, 1_000_000.0)?;
    bounded("pitch", pitch, -1.5, 1.5)?;
    bounded("distance", distance, 0.5, 400.0)
}

fn bounded(field: &str, value: f32, min: f32, max: f32) -> Result<(), String> {
    if value.is_finite() && (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(format!(
            "mesh camera {field} must be finite and in {min}..={max}"
        ))
    }
}

fn aspect(size: (u32, u32)) -> Result<f32, String> {
    if !(1..=MAX_VIEWPORT).contains(&size.0) || !(1..=MAX_VIEWPORT).contains(&size.1) {
        return Err(format!(
            "mesh camera viewport dimensions must be in 1..={MAX_VIEWPORT}"
        ));
    }
    Ok(size.0 as f32 / size.1 as f32)
}

fn validate_camera(camera: &Camera3D, size: (u32, u32)) -> Result<(), String> {
    let aspect = aspect(size)?;
    if !camera
        .view_proj(aspect)
        .0
        .iter()
        .flatten()
        .all(|value| value.is_finite())
    {
        return Err("mesh camera produced a non-finite matrix".into());
    }
    for point in [
        [0.0, 0.0],
        [size.0 as f32, 0.0],
        [0.0, size.1 as f32],
        [size.0 as f32, size.1 as f32],
        [size.0 as f32 * 0.5, size.1 as f32 * 0.5],
    ] {
        let (origin, direction) = camera.screen_ray(point, size);
        if !origin
            .iter()
            .chain(direction.iter())
            .all(|value| value.is_finite())
        {
            return Err("mesh camera produced a non-finite screen ray".into());
        }
    }
    let center = camera
        .world_to_screen(camera.target, size)
        .ok_or("mesh camera target is behind the camera")?;
    if (center[0] - size.0 as f32 * 0.5).abs() > 0.1
        || (center[1] - size.1 as f32 * 0.5).abs() > 0.1
    {
        return Err("mesh camera target does not project to the viewport center".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "mesh_camera/tests.rs"]
mod tests;
