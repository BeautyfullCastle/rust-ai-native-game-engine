//! Authored, presentation-only room camera. Session gestures never edit this document.
//!
//! Schema 1 is strict JSON: all fields are required, and unknown or duplicate
//! fields are errors. Angles are radians; the perspective field of view is degrees.
//! Orthographic `half_height` fits the shorter viewport axis and zooms in proportion
//! to the session distance. The same camera construction serves capture and play.

use orr_render::camera3d::{Camera3D, OrbitCamera};
use serde::{Deserialize, Serialize};

pub const MAX_BYTES: usize = 4096;
const MAX_VIEWPORT: u32 = 16_384;
const PI: f32 = std::f32::consts::PI;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Document {
    pub schema: u32,
    pub target: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub projection: Projection,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Projection {
    Perspective { fov_y_degrees: f32 },
    Orthographic { half_height: f32 },
}

// Derived structs accept positional arrays as well as maps. Schema 1 only
// admits JSON objects. Keep typed map deserialization (not Value) so duplicate
// keys and direct numeric deserialization remain observable.
fn deserialize_object<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Object<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Object<T> {
        type Value = T;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a room camera JSON object")
        }

        fn visit_map<M: serde::de::MapAccess<'de>>(self, map: M) -> Result<T, M::Error> {
            T::deserialize(serde::de::value::MapAccessDeserializer::new(map))
        }
    }
    deserializer.deserialize_map(Object(std::marker::PhantomData))
}

impl<'de> Deserialize<'de> for Document {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            schema: u32,
            target: [f32; 3],
            yaw: f32,
            pitch: f32,
            distance: f32,
            projection: Projection,
        }
        let wire: Wire = deserialize_object(deserializer)?;
        Ok(Self {
            schema: wire.schema,
            target: wire.target,
            yaw: wire.yaw,
            pitch: wire.pitch,
            distance: wire.distance,
            projection: wire.projection,
        })
    }
}

// Deserialize numeric fields directly rather than through serde's internally
// tagged enum Content buffer. With serde_json/arbitrary_precision (enabled by
// the editor), that buffer represents JSON decimal numbers as private maps,
// which cannot subsequently deserialize into f32.
impl<'de> Deserialize<'de> for Projection {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Missing is allowed here so the tag can select the required field.
        // Present null is never treated as missing, even for the other variant.
        fn present_float<'de, D: serde::Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<f32>, D::Error> {
            f32::deserialize(deserializer).map(Some)
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            #[serde(rename = "type")]
            // A derived unit enum would also accept {"orthographic": null}.
            // Schema 1 requires this tag to be a JSON string.
            kind: String,
            #[serde(default, deserialize_with = "present_float")]
            fov_y_degrees: Option<f32>,
            #[serde(default, deserialize_with = "present_float")]
            half_height: Option<f32>,
        }

        let wire: Wire = deserialize_object(deserializer)?;
        match (wire.kind.as_str(), wire.fov_y_degrees, wire.half_height) {
            ("perspective", Some(fov_y_degrees), None) => Ok(Self::Perspective { fov_y_degrees }),
            ("orthographic", None, Some(half_height)) => Ok(Self::Orthographic { half_height }),
            ("perspective", _, Some(_)) => Err(serde::de::Error::custom(
                "perspective projection forbids half_height",
            )),
            ("orthographic", Some(_), _) => Err(serde::de::Error::custom(
                "orthographic projection forbids fov_y_degrees",
            )),
            ("perspective", None, None) => Err(serde::de::Error::missing_field("fov_y_degrees")),
            ("orthographic", None, None) => Err(serde::de::Error::missing_field("half_height")),
            (kind, _, _) => Err(serde::de::Error::unknown_variant(
                kind,
                &["perspective", "orthographic"],
            )),
        }
    }
}

impl Document {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err(format!("room camera exceeds {MAX_BYTES} bytes"));
        }
        let document: Self =
            serde_json::from_slice(bytes).map_err(|e| format!("room camera JSON: {e}"))?;
        document.validate()?;
        Ok(document)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let mut bytes =
            serde_json::to_vec_pretty(self).map_err(|e| format!("room camera JSON: {e}"))?;
        bytes.push(b'\n');
        if bytes.len() > MAX_BYTES {
            return Err(format!("room camera exceeds {MAX_BYTES} bytes"));
        }
        Ok(bytes)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 1 {
            return Err("room camera schema must be 1".into());
        }
        validate_pose(self.target, self.yaw, self.pitch, self.distance)?;
        match self.projection {
            Projection::Perspective { fov_y_degrees } => {
                bounded("fov_y_degrees", fov_y_degrees, 20.0, 100.0)
            }
            Projection::Orthographic { half_height } => {
                bounded("half_height", half_height, 1.0, 64.0)
            }
        }
    }

    /// A high three-quarter view that keeps the room layout readable.
    pub fn readable_default() -> Self {
        Self {
            schema: 1,
            target: [0.0, 0.5, 0.0],
            yaw: 0.65,
            pitch: 1.02,
            distance: 24.0,
            projection: Projection::Orthographic { half_height: 8.0 },
        }
    }

    pub fn orbit(&self) -> OrbitCamera {
        OrbitCamera {
            target: self.target,
            yaw: self.yaw,
            pitch: self.pitch,
            distance: self.distance,
            fov_y_degrees: match self.projection {
                Projection::Perspective { fov_y_degrees } => fov_y_degrees,
                Projection::Orthographic { .. } => 50.0,
            },
        }
    }

    /// Reject invalid dimensions instead of silently changing the capture aspect.
    pub fn camera(&self, orbit: &OrbitCamera, size: (u32, u32)) -> Result<Camera3D, String> {
        self.validate()?;
        validate_pose(orbit.target, orbit.yaw, orbit.pitch, orbit.distance)?;
        bounded("session fov_y_degrees", orbit.fov_y_degrees, 20.0, 100.0)?;
        let aspect = aspect(size)?;
        let eye = orbit.eye();
        let camera = match self.projection {
            Projection::Perspective { fov_y_degrees } => {
                Camera3D::perspective(eye, orbit.target, fov_y_degrees)
            }
            Projection::Orthographic { half_height } => Camera3D::orthographic(
                eye,
                orbit.target,
                half_height * (orbit.distance / self.distance) / aspect.min(1.0),
            ),
        };
        if !camera
            .view_proj(aspect)
            .0
            .iter()
            .flatten()
            .all(|v| v.is_finite())
        {
            return Err("room camera produced a non-finite matrix".into());
        }
        for point in [
            [0.0, 0.0],
            [size.0 as f32, 0.0],
            [0.0, size.1 as f32],
            [size.0 as f32, size.1 as f32],
            [size.0 as f32 * 0.5, size.1 as f32 * 0.5],
        ] {
            let (origin, direction) = camera.screen_ray(point, size);
            if !origin.iter().chain(direction.iter()).all(|v| v.is_finite()) {
                return Err("room camera produced a non-finite ray".into());
            }
        }
        Ok(camera)
    }

    pub fn orbit_delta(&self, orbit: &mut OrbitCamera, delta: [f32; 2]) {
        if !delta.iter().all(|v| v.is_finite()) || self.camera(orbit, (1, 1)).is_err() {
            return;
        }
        let mut next = *orbit;
        // Widen before arithmetic so even finite f32::MAX gestures stay bounded.
        let pi = PI as f64;
        next.yaw =
            ((orbit.yaw as f64 - delta[0] as f64 * 0.006 + pi).rem_euclid(2.0 * pi) - pi) as f32;
        next.pitch = (orbit.pitch as f64 + delta[1] as f64 * 0.006).clamp(-1.4, 1.4) as f32;
        self.commit(orbit, next, (1, 1));
    }

    pub fn pan(&self, orbit: &mut OrbitCamera, delta: [f32; 2], size: (u32, u32)) {
        if !delta.iter().all(|v| v.is_finite()) {
            return;
        }
        let Ok(camera) = self.camera(orbit, size) else {
            return;
        };
        let half_height = match camera.projection {
            orr_render::camera3d::Projection::Perspective { fov_y, .. } => {
                orbit.distance * (fov_y * 0.5).tan()
            }
            orr_render::camera3d::Projection::Orthographic { half_height, .. } => half_height,
        };
        let per_pixel = 2.0 * half_height as f64 / size.1 as f64;
        let right = camera.right();
        let up = camera.camera_up();
        let mut next = *orbit;
        for axis in 0..3 {
            let shift = (-right[axis] as f64 * delta[0] as f64 + up[axis] as f64 * delta[1] as f64)
                * per_pixel;
            next.target[axis] = (orbit.target[axis] as f64 + shift).clamp(-64.0, 64.0) as f32;
        }
        self.commit(orbit, next, size);
    }

    pub fn zoom(&self, orbit: &mut OrbitCamera, steps: f32) {
        if !steps.is_finite() || self.camera(orbit, (1, 1)).is_err() {
            return;
        }
        let mut next = *orbit;
        // Bound the logarithm before exponentiation; extreme wheel values saturate.
        next.distance = ((orbit.distance as f64).ln() + steps as f64 * 0.9_f64.ln())
            .clamp(0.0, 128.0_f64.ln())
            .exp()
            .clamp(1.0, 128.0) as f32;
        self.commit(orbit, next, (1, 1));
    }

    fn commit(&self, orbit: &mut OrbitCamera, next: OrbitCamera, size: (u32, u32)) {
        if self.camera(&next, size).is_ok()
            && self.camera(&next, (1, MAX_VIEWPORT)).is_ok()
            && self.camera(&next, (MAX_VIEWPORT, 1)).is_ok()
        {
            *orbit = next;
        }
    }
}

fn bounded(field: &str, value: f32, min: f32, max: f32) -> Result<(), String> {
    if value.is_finite() && (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(format!(
            "room camera {field} must be finite and in {min}..={max}"
        ))
    }
}

fn validate_pose(target: [f32; 3], yaw: f32, pitch: f32, distance: f32) -> Result<(), String> {
    for value in target {
        bounded("target", value, -64.0, 64.0)?;
    }
    bounded("yaw", yaw, -PI, PI)?;
    bounded("pitch", pitch, -1.4, 1.4)?;
    bounded("distance", distance, 1.0, 128.0)
}

fn aspect(size: (u32, u32)) -> Result<f32, String> {
    if !(1..=MAX_VIEWPORT).contains(&size.0) || !(1..=MAX_VIEWPORT).contains(&size.1) {
        return Err(format!(
            "room camera viewport dimensions must be in 1..={MAX_VIEWPORT}"
        ));
    }
    Ok(size.0 as f32 / size.1 as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use orr_render::camera3d::Projection as RenderProjection;

    fn perspective() -> Document {
        Document {
            projection: Projection::Perspective {
                fov_y_degrees: 57.0,
            },
            ..Document::readable_default()
        }
    }

    #[test]
    fn strict_json_round_trip_and_size_bound() {
        for doc in [Document::readable_default(), perspective()] {
            assert_eq!(Document::parse(&doc.to_bytes().unwrap()).unwrap(), doc);
        }
        let bytes = Document::readable_default().to_bytes().unwrap();
        let mut padded = bytes.clone();
        padded.resize(MAX_BYTES, b' ');
        assert!(Document::parse(&padded).is_ok());
        padded.push(b' ');
        assert!(Document::parse(&padded).is_err());
        let base = String::from_utf8(bytes).unwrap();
        for malformed in [
            base.replacen("\"schema\": 1", "\"schema\": 2", 1),
            base.replacen("\"schema\": 1", "\"schema\": 1, \"schema\": 1", 1),
            base.replacen("\"schema\": 1,", "", 1),
            base.replacen("\"schema\": 1", "\"schema\": 1, \"extra\": 0", 1),
            base.replace(
                "\"half_height\": 8.0",
                "\"half_height\": 8.0, \"half_height\": 8.0",
            ),
            base.replace("\"half_height\": 8.0", "\"half_height\": 8.0, \"extra\": 0"),
            base.replace("\"half_height\": 8.0", "\"half_height\": null"),
            base.replace("\"half_height\": 8.0", "\"half_height\": 1e999"),
            base.replace("\"half_height\": 8.0", "\"half_height\": NaN"),
            base.replace(
                "\"type\": \"orthographic\"",
                "\"type\": \"orthographic\", \"type\": \"orthographic\"",
            ),
            base.replace("orthographic", "unknown"),
            format!("{base}{{}}"),
        ] {
            assert!(
                Document::parse(malformed.as_bytes()).is_err(),
                "accepted {malformed}"
            );
        }
    }

    #[test]
    fn every_field_is_required_and_projection_shapes_are_closed() {
        for doc in [Document::readable_default(), perspective()] {
            let value = serde_json::to_value(&doc).unwrap();
            for field in ["schema", "target", "yaw", "pitch", "distance", "projection"] {
                let mut missing = value.clone();
                missing.as_object_mut().unwrap().remove(field);
                assert!(
                    Document::parse(&serde_json::to_vec(&missing).unwrap()).is_err(),
                    "{field}"
                );
            }
            let fields: Vec<_> = value["projection"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
            for field in fields {
                let mut missing = value.clone();
                missing["projection"]
                    .as_object_mut()
                    .unwrap()
                    .remove(&field);
                assert!(
                    Document::parse(&serde_json::to_vec(&missing).unwrap()).is_err(),
                    "{field}"
                );
            }
            for replacement in [
                serde_json::json!([0, 0]),
                serde_json::json!([0, 0, 0, 0]),
                serde_json::json!("0,0,0"),
            ] {
                let mut invalid = value.clone();
                invalid["target"] = replacement;
                assert!(Document::parse(&serde_json::to_vec(&invalid).unwrap()).is_err());
            }
        }
    }

    // Run this suite with editor + sample in the same Cargo invocation to
    // exercise serde_json/arbitrary_precision feature unification as well.
    #[test]
    fn projection_decimal_roundtrip_and_strict_variant_fields() {
        for projection in [
            Projection::Perspective {
                fov_y_degrees: 57.125,
            },
            Projection::Orthographic { half_height: 8.375 },
        ] {
            let doc = Document {
                projection,
                ..Document::readable_default()
            };
            assert_eq!(Document::parse(&doc.to_bytes().unwrap()).unwrap(), doc);
        }
        for valid in [
            r#"{"type":"perspective","fov_y_degrees":5.7125e1}"#,
            r#"{"fov_y_degrees":57.125,"type":"perspective"}"#,
            r#"{"type":"orthographic","half_height":8.375}"#,
            r#"{"half_height":8.375,"type":"orthographic"}"#,
        ] {
            assert!(serde_json::from_str::<Projection>(valid).is_ok(), "{valid}");
        }
        for invalid in [
            r#"["perspective",57.0]"#,
            r#"["perspective",57.0,null]"#,
            r#"["orthographic",null,8.0]"#,
            r#"{"type":{"orthographic":null},"half_height":8}"#,
            r#"{"type":{"perspective":null},"fov_y_degrees":57}"#,
            r#"{"type":null,"half_height":8}"#,
            r#"{"type":["orthographic"],"half_height":8}"#,
            r#"{"type":true,"half_height":8}"#,
            r#"{"type":1,"half_height":8}"#,
            r#"{"type":"Orthographic","half_height":8}"#,
            r#"{"type":"unknown","half_height":8}"#,
            r#"{"type":"perspective","fov_y_degrees":57.125,"half_height":8}"#,
            r#"{"type":"perspective","fov_y_degrees":57.125,"half_height":null}"#,
            r#"{"type":"perspective","fov_y_degrees":null}"#,
            r#"{"type":"perspective","fov_y_degrees":57.125,"fov_y_degrees":57.125}"#,
            r#"{"type":"orthographic","half_height":8.375,"fov_y_degrees":57}"#,
            r#"{"type":"orthographic","half_height":8.375,"fov_y_degrees":null}"#,
            r#"{"type":"orthographic","half_height":null}"#,
            r#"{"type":"orthographic","half_height":8.375,"half_height":8.375}"#,
            r#"{"type":"orthographic","type":"orthographic","half_height":8.375}"#,
        ] {
            assert!(
                serde_json::from_str::<Projection>(invalid).is_err(),
                "{invalid}"
            );
        }
        for invalid in [
            r#"[1,[0,0.5,0],0.65,1.02,24,{"type":"orthographic","half_height":8}]"#,
            r#"{"schema":1,"target":[0,0.5,0],"yaw":0.65,"pitch":1.02,"distance":24,"projection":["perspective",57.0]}"#,
        ] {
            assert!(Document::parse(invalid.as_bytes()).is_err(), "{invalid}");
            assert!(
                serde_json::from_str::<Document>(invalid).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn all_numeric_fields_have_closed_finite_bounds() {
        // Exercise every target component and every authored scalar separately.
        for index in 0..8 {
            let (min, max) = match index {
                0..=2 => (-64.0, 64.0),
                3 => (-PI, PI),
                4 => (-1.4, 1.4),
                5 => (1.0, 128.0),
                6 => (20.0, 100.0),
                _ => (1.0, 64.0),
            };
            for value in [
                min,
                max,
                min - 0.01,
                max + 0.01,
                f32::NAN,
                f32::INFINITY,
                f32::NEG_INFINITY,
            ] {
                let mut doc = Document::readable_default();
                match index {
                    0..=2 => doc.target[index] = value,
                    3 => doc.yaw = value,
                    4 => doc.pitch = value,
                    5 => doc.distance = value,
                    6 => {
                        doc.projection = Projection::Perspective {
                            fov_y_degrees: value,
                        }
                    }
                    _ => doc.projection = Projection::Orthographic { half_height: value },
                }
                let valid = value == min || value == max;
                assert_eq!(
                    doc.validate().is_ok(),
                    valid,
                    "index {index}, value {value}"
                );
                assert_eq!(doc.to_bytes().is_ok(), valid);
                assert_eq!(doc.camera(&doc.orbit(), (800, 600)).is_ok(), valid);
            }
        }
    }

    #[test]
    fn capture_and_native_use_identical_centered_cameras() {
        for doc in [Document::readable_default(), perspective()] {
            let orbit = doc.orbit();
            for size in [
                (960, 540),
                (540, 960),
                (1, 1),
                (1, 16384),
                (16384, 1),
                (16384, 16384),
            ] {
                let native = doc.camera(&orbit, size).unwrap();
                let capture = Document::parse(&doc.to_bytes().unwrap())
                    .unwrap()
                    .camera(&orbit, size)
                    .unwrap();
                assert_eq!(native, capture);
                assert_eq!(
                    native.view_proj(aspect(size).unwrap()),
                    capture.view_proj(aspect(size).unwrap())
                );
                let center = native.world_to_screen(doc.target, size).unwrap();
                assert!(
                    (center[0] - size.0 as f32 * 0.5).abs() < 0.05,
                    "{size:?} {center:?}"
                );
                assert!(
                    (center[1] - size.1 as f32 * 0.5).abs() < 0.05,
                    "{size:?} {center:?}"
                );
            }
            for size in [(0, 1), (1, 0), (16385, 1), (1, 16385), (u32::MAX, u32::MAX)] {
                assert!(doc.camera(&orbit, size).is_err());
            }
        }
    }

    #[test]
    fn orthographic_portrait_fit_and_session_zoom_are_exact() {
        let doc = Document::readable_default();
        let mut orbit = doc.orbit();
        for (size, expected) in [
            ((800, 400), 8.0),
            ((400, 800), 16.0),
            ((1, 16384), 131072.0),
        ] {
            let camera = doc.camera(&orbit, size).unwrap();
            assert!(
                matches!(camera.projection, RenderProjection::Orthographic { half_height, .. } if half_height == expected)
            );
        }
        doc.zoom(&mut orbit, 1.0);
        assert!((orbit.distance - 21.6).abs() < 0.0001);
        let camera = doc.camera(&orbit, (400, 800)).unwrap();
        assert!(
            matches!(camera.projection, RenderProjection::Orthographic { half_height, .. } if (half_height - 14.4).abs() < 0.0001)
        );
        assert_eq!(doc, Document::readable_default());
    }

    #[test]
    fn pan_tracks_pixels_for_both_projections_after_resize_and_zoom() {
        for doc in [Document::readable_default(), perspective()] {
            let mut orbit = doc.orbit();
            doc.zoom(&mut orbit, 2.0);
            for size in [(800, 400), (400, 800), (1231, 719)] {
                let world = orbit.target;
                let before = doc
                    .camera(&orbit, size)
                    .unwrap()
                    .world_to_screen(world, size)
                    .unwrap();
                doc.pan(&mut orbit, [23.0, -17.0], size);
                let after = doc
                    .camera(&orbit, size)
                    .unwrap()
                    .world_to_screen(world, size)
                    .unwrap();
                assert!((after[0] - before[0] - 23.0).abs() < 0.01);
                assert!((after[1] - before[1] + 17.0).abs() < 0.01);
            }
        }
    }

    #[test]
    fn finite_extreme_gestures_saturate_and_wrap() {
        for doc in [Document::readable_default(), perspective()] {
            let mut orbit = doc.orbit();
            doc.orbit_delta(&mut orbit, [f32::MAX, f32::MAX]);
            assert_eq!(orbit.pitch, 1.4);
            assert!((-PI..=PI).contains(&orbit.yaw));
            doc.orbit_delta(&mut orbit, [-f32::MAX, -f32::MAX]);
            assert_eq!(orbit.pitch, -1.4);
            doc.zoom(&mut orbit, f32::MAX);
            assert_eq!(orbit.distance, 1.0);
            doc.zoom(&mut orbit, -f32::MAX);
            assert_eq!(orbit.distance, 128.0);
            doc.pan(&mut orbit, [f32::MAX, -f32::MAX], (1, 16384));
            assert!(orbit
                .target
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 64.0));
            assert!(doc.camera(&orbit, (16384, 1)).is_ok());
        }
    }

    #[test]
    fn invalid_inputs_and_invalid_sessions_are_noops() {
        let doc = Document::readable_default();
        let original = doc.orbit();
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut orbit = original;
            for delta in [[bad, 1.0], [1.0, bad]] {
                doc.orbit_delta(&mut orbit, delta);
                doc.pan(&mut orbit, delta, (800, 600));
                assert_eq!(orbit, original);
            }
            doc.zoom(&mut orbit, bad);
            assert_eq!(orbit, original);
        }
        for size in [(0, 600), (800, 0), (16385, 1)] {
            let mut orbit = original;
            doc.pan(&mut orbit, [2.0, 3.0], size);
            assert_eq!(orbit, original);
        }
        let mut invalid = original;
        invalid.distance = 0.0;
        let before = invalid;
        doc.orbit_delta(&mut invalid, [1.0, 1.0]);
        doc.pan(&mut invalid, [1.0, 1.0], (800, 600));
        doc.zoom(&mut invalid, 1.0);
        assert_eq!(invalid, before);
    }
}
