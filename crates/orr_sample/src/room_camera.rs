//! Authored, presentation-only room camera. Session gestures never edit this document.
//!
//! Schemas 1 and 2 are strict JSON: all fields required by the selected schema
//! are required, and unknown or duplicate fields are errors. Angles are radians;
//! the perspective field of view is degrees.
//! Orthographic `half_height` fits the shorter viewport axis and zooms in proportion
//! to the session distance. The same camera construction serves capture and play.

use orr_bridge::FrameView;
use orr_ecs::Entity;
use orr_fp::FP;
use orr_games::room_escape_game::{RoomActor, PLAYER};
use orr_physics3d::Body;
use orr_reflect::{Guid, SceneIndex};
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
    /// Optional live PLAYER anchor. Omitted documents retain the schema-1
    /// wire bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub follow: Option<Follow>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Follow {
    pub player: String,
    pub offset: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Projection {
    Perspective { fov_y_degrees: f32 },
    Orthographic { half_height: f32 },
}

// Derived structs accept positional arrays as well as maps. Camera schemas only
// admit JSON objects. Keep typed map deserialization (not Value) so duplicate
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
            // A custom deserializer distinguishes a missing field from explicit null.
            #[serde(default, deserialize_with = "deserialize_present_follow")]
            follow: Option<Follow>,
        }
        let wire: Wire = deserialize_object(deserializer)?;
        Ok(Self {
            schema: wire.schema,
            target: wire.target,
            yaw: wire.yaw,
            pitch: wire.pitch,
            distance: wire.distance,
            projection: wire.projection,
            follow: wire.follow,
        })
    }
}

fn deserialize_present_follow<'de, D>(deserializer: D) -> Result<Option<Follow>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Follow::deserialize(deserializer).map(Some)
}

impl<'de> Deserialize<'de> for Follow {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            player: String,
            offset: [f32; 3],
        }
        let wire: Wire = deserialize_object(deserializer)?;
        Ok(Self {
            player: wire.player,
            offset: wire.offset,
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
            // Camera schemas require this tag to be a JSON string.
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
        match (self.schema, self.follow.as_ref()) {
            (1, None) => {}
            (1, Some(_)) => return Err("room camera schema 1 forbids follow".into()),
            (2, None) => return Err("room camera schema 2 requires follow".into()),
            (2, Some(follow)) => {
                Guid::parse(&follow.player)
                    .map_err(|e| format!("room camera follow player: {e}"))?;
                for value in follow.offset {
                    bounded("follow offset", value, -16.0, 16.0)?;
                }
            }
            _ => return Err("room camera schema must be 1 or 2".into()),
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
            follow: None,
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

    /// Construct from an explicit session orbit. Frame-aware consumers should
    /// use `camera_for_frame` so follow mode uses the current authoritative pose.
    /// Reject invalid dimensions instead of silently changing the capture aspect.
    pub fn camera(&self, orbit: &OrbitCamera, size: (u32, u32)) -> Result<Camera3D, String> {
        self.validate()?;
        validate_orbit(orbit)?;
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

    /// Validate a schema-2 live binding against the exact current frame and
    /// scene index.
    /// Fixed schema-1 cameras do not depend on a frame and always admit.
    pub fn validate_frame(&self, frame: FrameView<'_>, index: &SceneIndex) -> Result<(), String> {
        self.validate()?;
        if let Some(follow) = &self.follow {
            follow_body(follow, frame, index).map(|_| ())
        } else {
            Ok(())
        }
    }

    /// Construct the presentation camera for this authoritative frame.
    /// Follow mode replaces only the target; session yaw, pitch, distance, and
    /// projection stay intact.
    pub fn camera_for_frame(
        &self,
        orbit: &OrbitCamera,
        frame: FrameView<'_>,
        index: &SceneIndex,
        size: (u32, u32),
    ) -> Result<Camera3D, String> {
        self.validate()?;
        // Match the fixed-camera contract before rebasing the target; malformed
        // session state must not be masked by a valid followed PLAYER pose.
        validate_orbit(orbit)?;
        let Some(follow) = &self.follow else {
            return self.camera(orbit, size);
        };
        let (_, body) = follow_body(follow, frame, index)?;
        let pos = orr_view::fp_to_vec3(body.pos);
        let mut live_orbit = *orbit;
        live_orbit.target = [
            pos.x + follow.offset[0],
            pos.y + follow.offset[1],
            pos.z + follow.offset[2],
        ];
        self.camera(&live_orbit, size)
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
        if self.follow.is_some() || !delta.iter().all(|v| v.is_finite()) {
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

fn follow_body<'a>(
    follow: &Follow,
    frame: FrameView<'a>,
    index: &SceneIndex,
) -> Result<(Entity, &'a Body), String> {
    let guid =
        Guid::parse(&follow.player).map_err(|e| format!("room camera follow player: {e}"))?;
    let entity = index
        .entity(&guid)
        .ok_or_else(|| "room camera follow PLAYER GUID is not in the scene index".to_string())?;
    if index.guid(entity) != Some(&guid) {
        return Err("room camera follow PLAYER scene-index reverse mapping does not match".into());
    }
    if !frame.exists(entity) {
        return Err("room camera follow PLAYER entity is not alive in this frame".into());
    }

    let mut players = frame
        .iter::<RoomActor>()
        .filter_map(|(candidate, actor)| (actor.kind == PLAYER).then_some(candidate));
    if players.next() != Some(entity) || players.next().is_some() {
        return Err("room camera follow requires the exact sole PLAYER entity".into());
    }

    let body = frame
        .get::<Body>(entity)
        .ok_or_else(|| "room camera follow PLAYER body is missing".to_string())?;
    let min_xz = FP::from_int(-16);
    let max_xz = FP::from_int(16);
    let min_y = FP::from_int(-4);
    let max_y = FP::from_int(4);
    if !(min_xz..=max_xz).contains(&body.pos.x)
        || !(min_y..=max_y).contains(&body.pos.y)
        || !(min_xz..=max_xz).contains(&body.pos.z)
    {
        return Err("room camera follow PLAYER body position is outside room bounds".into());
    }
    Ok((entity, body))
}

fn validate_orbit(orbit: &OrbitCamera) -> Result<(), String> {
    validate_pose(orbit.target, orbit.yaw, orbit.pitch, orbit.distance)?;
    bounded("session fov_y_degrees", orbit.fov_y_degrees, 20.0, 100.0)
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
    use orr_ecs::Frame;
    use orr_fp::{fp, FPVec3};
    use orr_games::room_escape_game::{RoomConfig, RoomEscapeV1};
    use orr_render::camera3d::Projection as RenderProjection;
    use orr_sim::{Game, Simulation};

    fn perspective() -> Document {
        Document {
            projection: Projection::Perspective {
                fov_y_degrees: 57.0,
            },
            ..Document::readable_default()
        }
    }

    fn room_frame() -> (Frame, SceneIndex, Entity, Guid) {
        let mut frame = Frame::new(Simulation::<RoomEscapeV1>::build_registry());
        RoomEscapeV1::setup(&mut frame, &RoomConfig);
        let player = FrameView::of(&frame)
            .iter::<RoomActor>()
            .find(|(_, actor)| actor.kind == PLAYER)
            .unwrap()
            .0;
        let guid = Guid::from_u32(1);
        let mut index = SceneIndex::default();
        index.insert(guid.clone(), player);
        (frame, index, player, guid)
    }

    fn following(guid: &Guid) -> Document {
        Document {
            schema: 2,
            follow: Some(Follow {
                player: guid.to_string(),
                offset: [0.25, 1.0, -0.5],
            }),
            ..Document::readable_default()
        }
    }

    #[test]
    fn schema_one_bytes_and_fixed_camera_behavior_are_unchanged() {
        let document = Document::readable_default();
        let expected = concat!(
            "{\n",
            "  \"schema\": 1,\n",
            "  \"target\": [\n",
            "    0.0,\n",
            "    0.5,\n",
            "    0.0\n",
            "  ],\n",
            "  \"yaw\": 0.65,\n",
            "  \"pitch\": 1.02,\n",
            "  \"distance\": 24.0,\n",
            "  \"projection\": {\n",
            "    \"type\": \"orthographic\",\n",
            "    \"half_height\": 8.0\n",
            "  }\n",
            "}\n"
        );
        assert_eq!(document.to_bytes().unwrap(), expected.as_bytes());

        let (frame, index, _, _) = room_frame();
        let orbit = document.orbit();
        assert_eq!(
            document.camera_for_frame(&orbit, FrameView::of(&frame), &index, (800, 600)),
            document.camera(&orbit, (800, 600))
        );
    }

    #[test]
    fn schema_two_follow_roundtrips_and_rejects_non_object_shapes() {
        let doc = following(&Guid::from_u32(0xfeed));
        let bytes = doc.to_bytes().unwrap();
        assert_eq!(Document::parse(&bytes).unwrap(), doc);
        let decimal = format!(
            r#"{{"schema":2,"target":[0,0.5,0],"yaw":0.65,"pitch":1.02,"distance":24,"projection":{{"type":"orthographic","half_height":8.375}},"follow":{{"player":"{}","offset":[3.75e-1,-1.125,1.5875e1]}}}}"#,
            doc.follow.as_ref().unwrap().player
        );
        let parsed_decimal = Document::parse(decimal.as_bytes()).unwrap();
        assert_eq!(
            parsed_decimal.follow.unwrap().offset,
            [0.375, -1.125, 15.875]
        );
        let text = String::from_utf8(bytes).unwrap();
        for malformed in [
            text.replacen("\"schema\": 2", "\"schema\": 1", 1),
            text.replacen("\"schema\": 2,", "\"schema\": 2, \"follow\": null,", 1),
            text.replace("\"follow\": {", "\"follow\": ["),
            text.replace("\"offset\": [", "\"offset\": null, \"bad\": ["),
            text.replace(
                "\"offset\": [",
                "\"offset\": [0.25, 1.0, -0.5, 0.0], \"bad\": [",
            ),
            text.replace("\"player\": \"e_0000feed\"", "\"player\": null"),
            text.replace(
                "\"player\": \"e_0000feed\"",
                "\"player\": \"e_0000feed\", \"player\": \"e_0000feed\"",
            ),
            text.replace(
                "\"player\": \"e_0000feed\"",
                "\"player\": \"e_0000feed\", \"extra\": 0",
            ),
            text.replace("\"offset\": [", "\"offset\": [null, 1.0, -0.5], \"bad\": ["),
            text.replace(
                "\"offset\": [",
                "\"offset\": [0.25, 1.0, -0.5], \"offset\": [",
            ),
            text.replace("e_0000feed", "E_0000FEED"),
            text.replace("e_0000feed", "e_0000fee"),
            text.replace("\"schema\": 2", "\"schema\": 3"),
            text.replace("\"schema\": 2,", "\"schema\": 2, \"extra\": 0,"),
            text.replace("\"follow\": {", "\"follow\": {\"follow\": {} ,"),
        ] {
            assert!(
                Document::parse(malformed.as_bytes()).is_err(),
                "accepted {malformed}"
            );
        }
        let mut invalid_offset = doc.clone();
        for axis in 0..3 {
            for edge in [
                -16.0,
                16.0,
                -16.01,
                16.01,
                f32::NAN,
                f32::INFINITY,
                f32::NEG_INFINITY,
            ] {
                invalid_offset.follow.as_mut().unwrap().offset[axis] = edge;
                let valid = edge == -16.0 || edge == 16.0;
                assert_eq!(
                    invalid_offset.validate().is_ok(),
                    valid,
                    "axis {axis}, edge {edge}"
                );
                assert_eq!(
                    invalid_offset.to_bytes().is_ok(),
                    valid,
                    "axis {axis}, edge {edge}"
                );
            }
            invalid_offset.follow.as_mut().unwrap().offset[axis] = 0.0;
        }
        for invalid_follow in [
            r#"["e_0000feed",[0,0,0]]"#,
            r#"{"player":"e_0000feed","offset":[0,0]}"#,
            r#"{"player":"e_0000feed","offset":[0,0,0,0]}"#,
            r#"{"player":"e_0000feed","offset":null}"#,
            r#"{"player":"e_0000feed","offset":[0,0,0],"extra":0}"#,
            r#"{"player":"e_0000feed","player":"e_0000feed","offset":[0,0,0]}"#,
            r#"{"player":"e_0000feed","offset":[0,0,0],"offset":[0,0,0]}"#,
            r#"null"#,
        ] {
            assert!(
                serde_json::from_str::<Follow>(invalid_follow).is_err(),
                "{invalid_follow}"
            );
        }
        let no_follow = serde_json::json!({
            "schema": 2,
            "target": [0.0, 0.5, 0.0],
            "yaw": 0.65,
            "pitch": 1.02,
            "distance": 24.0,
            "projection": {"type":"orthographic", "half_height":8.0}
        });
        assert!(Document::parse(&serde_json::to_vec(&no_follow).unwrap()).is_err());
        let (room_frame, empty_index, _, _) = room_frame();
        let unknown_target = following(&Guid::from_u32(0xdeadbeef));
        assert!(unknown_target
            .validate_frame(FrameView::of(&room_frame), &empty_index)
            .is_err());
        let mut at_limit = doc.to_bytes().unwrap();
        at_limit.resize(MAX_BYTES, b' ');
        assert!(Document::parse(&at_limit).is_ok());
        at_limit.push(b' ');
        assert!(Document::parse(&at_limit).is_err());
        let schema_one_follow = serde_json::json!({
            "schema": 1,
            "target": [0.0, 0.5, 0.0],
            "yaw": 0.65,
            "pitch": 1.02,
            "distance": 24.0,
            "projection": {"type":"orthographic", "half_height":8.0},
            "follow": {"player":"e_0000feed", "offset":[0, 0, 0]}
        });
        assert!(Document::parse(&serde_json::to_vec(&schema_one_follow).unwrap()).is_err());
    }

    #[test]
    fn follow_binding_checks_exact_entity_generation_player_body_and_raw_bounds() {
        let (mut frame, index, player, guid) = room_frame();
        let doc = following(&guid);
        doc.validate_frame(FrameView::of(&frame), &index).unwrap();

        // Bounds are checked in fixed-point before conversion, including the exact edges.
        frame.get_mut::<Body>(player).unwrap().pos =
            FPVec3::new(FP::from_int(16), FP::from_int(-4), FP::from_int(-16));
        doc.validate_frame(FrameView::of(&frame), &index).unwrap();
        for outside in [
            FPVec3::new(FP::from_raw(FP::from_int(16).raw() + 1), FP::ZERO, FP::ZERO),
            FPVec3::new(FP::ZERO, FP::from_raw(FP::from_int(-4).raw() - 1), FP::ZERO),
            FPVec3::new(FP::ZERO, FP::ZERO, FP::from_raw(FP::from_int(16).raw() + 1)),
        ] {
            frame.get_mut::<Body>(player).unwrap().pos = outside;
            assert!(doc.validate_frame(FrameView::of(&frame), &index).is_err());
        }

        let (mut stale_frame, stale_index, stale_player, stale_guid) = room_frame();
        assert!(stale_frame.despawn(stale_player));
        let replacement = stale_frame.spawn();
        assert_eq!(replacement.index, stale_player.index);
        assert_ne!(replacement.version, stale_player.version);
        stale_frame.add(
            replacement,
            RoomActor {
                kind: PLAYER,
                ordinal: 0,
            },
        );
        stale_frame.add(replacement, Body::new_kinematic(FPVec3::ZERO));
        assert!(following(&stale_guid)
            .validate_frame(FrameView::of(&stale_frame), &stale_index)
            .is_err());

        let (mut missing_body_frame, missing_body_index, missing_body_player, missing_body_guid) =
            room_frame();
        missing_body_frame.remove::<Body>(missing_body_player);
        assert!(following(&missing_body_guid)
            .validate_frame(FrameView::of(&missing_body_frame), &missing_body_index)
            .is_err());

        let (mut duplicate_frame, duplicate_index, _, duplicate_guid) = room_frame();
        let extra = duplicate_frame.spawn();
        duplicate_frame.add(
            extra,
            RoomActor {
                kind: PLAYER,
                ordinal: 1,
            },
        );
        duplicate_frame.add(extra, Body::new_kinematic(FPVec3::ZERO));
        assert!(following(&duplicate_guid)
            .validate_frame(FrameView::of(&duplicate_frame), &duplicate_index)
            .is_err());

        let (
            wrong_reverse_frame,
            mut wrong_reverse_index,
            wrong_reverse_player,
            wrong_reverse_guid,
        ) = room_frame();
        wrong_reverse_index.insert(Guid::from_u32(2), wrong_reverse_player);
        assert!(following(&wrong_reverse_guid)
            .validate_frame(FrameView::of(&wrong_reverse_frame), &wrong_reverse_index)
            .is_err());

        let (wrong_target_frame, mut wrong_target_index, _, wrong_target_guid) = room_frame();
        let not_player = FrameView::of(&wrong_target_frame)
            .iter::<RoomActor>()
            .find(|(_, actor)| actor.kind != PLAYER)
            .unwrap()
            .0;
        wrong_target_index.insert(wrong_target_guid.clone(), not_player);
        assert!(following(&wrong_target_guid)
            .validate_frame(FrameView::of(&wrong_target_frame), &wrong_target_index)
            .is_err());
    }

    #[test]
    fn follow_camera_tracks_authoritative_frames_without_mutation_or_history() {
        let (frame, index, player, guid) = room_frame();
        let doc = following(&guid);
        let mut orbit = doc.orbit();
        doc.orbit_delta(&mut orbit, [17.0, -11.0]);
        doc.zoom(&mut orbit, 1.0);
        let session = orbit;
        let mut first = frame.clone();
        first.set_tick(4);
        first.get_mut::<Body>(player).unwrap().pos = FPVec3::new(fp!(2.25), fp!(1.5), fp!(-3.0));
        let mut later = frame.clone();
        later.set_tick(9);
        later.get_mut::<Body>(player).unwrap().pos = FPVec3::new(fp!(-5.0), fp!(0.25), fp!(6.5));
        let first_bytes = first.to_bytes();
        let first_checksum = FrameView::of(&first).checksum();
        let later_bytes = later.to_bytes();
        let later_checksum = FrameView::of(&later).checksum();

        let first_camera = doc
            .camera_for_frame(&orbit, FrameView::of(&first), &index, (800, 600))
            .unwrap();
        assert_eq!(first_camera.target, [2.5, 2.5, -3.5]);
        let later_camera = doc
            .camera_for_frame(&orbit, FrameView::of(&later), &index, (800, 600))
            .unwrap();
        assert_eq!(later_camera.target, [-4.75, 1.25, 6.0]);
        let mut invalid_session = orbit;
        invalid_session.target[0] = f32::NAN;
        assert!(doc
            .camera_for_frame(&invalid_session, FrameView::of(&first), &index, (800, 600))
            .is_err());
        let replayed_first = doc
            .camera_for_frame(&orbit, FrameView::of(&first), &index, (800, 600))
            .unwrap();
        let sought_back = doc
            .camera_for_frame(&orbit, FrameView::of(&first), &index, (800, 600))
            .unwrap();
        assert_eq!(replayed_first, first_camera);
        assert_eq!(sought_back, first_camera);
        assert_eq!(orbit, session);
        let restored_first = Frame::from_bytes(first.registry().clone(), &first_bytes).unwrap();
        assert_eq!(
            doc.camera_for_frame(&orbit, FrameView::of(&restored_first), &index, (800, 600))
                .unwrap(),
            first_camera
        );
        assert_eq!(first.to_bytes(), first_bytes);
        assert_eq!(FrameView::of(&first).checksum(), first_checksum);
        assert_eq!(later.to_bytes(), later_bytes);
        assert_eq!(FrameView::of(&later).checksum(), later_checksum);

        let mut pan = orbit;
        doc.pan(&mut pan, [200.0, -100.0], (800, 600));
        assert_eq!(pan, orbit);
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
