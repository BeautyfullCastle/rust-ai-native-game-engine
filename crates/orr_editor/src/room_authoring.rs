use super::{Editor, Mode, Pend, Target, J};
use orr_fp::{FPVec3, FP};
use orr_reflect::Value;
use orr_remote::json::value_to_json;
use serde_json::json;
use std::path::PathBuf;
#[cfg(any(test, all(windows, target_arch = "x86_64")))]
use std::collections::BTreeSet;

const ACTOR: &str = orr_sample::room_project::ACTOR;
const BODY: &str = "orr_physics3d::Body";
const COLLIDER: &str = "orr_physics3d::Collider";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RoomWall {
    pub guid: String,
    pub name: String,
    pub center: [FP; 2],
    pub half_extents: [FP; 2],
    pub center_y: FP,
    pub half_height: FP,
}

struct RoomWallRow {
    wall: RoomWall,
    target: Target,
    position: FPVec3,
    half_extents: FPVec3,
    components: J,
}

impl Editor {
    fn room_authoring_ready(&mut self) -> Result<(), String> {
        if !self.game().is_room()
            || !self.spec().is_local()
            || !self.can_mutate()
            || !self.scene_edit_allowed
            || self.mode() != Mode::Edit
            || self.previewing().is_some()
            || self.gesture.busy()
            || self.batch_uncertain
            || self.history.in_tx
            || self.sim.in_tx
            || self
                .pending
                .values()
                .any(|pending| matches!(pending.kind, Pend::Gesture(..) | Pend::Input(..)))
        {
            return Err("Room authoring requires an idle editable local Room with no transaction, preview or uncertain edit".into());
        }
        let state = self.call("sim.state", J::Null)?;
        self.apply_state(&state);
        if state.get("mode").and_then(J::as_str) != Some("edit")
            || state.get("in_tx").and_then(J::as_bool) != Some(false)
        {
            return Err("Room host is not an idle editable document".into());
        }
        Ok(())
    }

    fn room_layout_actors(&mut self) -> Result<[(Target, FPVec3); 3], String> {
        let query = self.call(
            "world.query",
            json!({"components":[ACTOR, BODY],"values":true,"limit":69}),
        )?;
        let entities = query
            .get("entities")
            .and_then(J::as_array)
            .ok_or("Room actor query returned no entities")?;
        if query.get("truncated").and_then(J::as_bool) != Some(false)
            || query.get("total").and_then(J::as_u64) != Some(entities.len() as u64)
        {
            return Err("Room actor query is incomplete".into());
        }
        let mut actors: [Option<(Target, FPVec3)>; 3] = std::array::from_fn(|_| None);
        for entity in entities {
            let values = entity
                .get("values")
                .ok_or("Room actor values are missing")?;
            let kind = values
                .get(ACTOR)
                .and_then(|actor| actor.get("kind"))
                .and_then(J::as_u64)
                .ok_or("Room actor kind is missing")?;
            if !(1..=3).contains(&kind) {
                continue;
            }
            let target = entity
                .get("guid")
                .and_then(J::as_str)
                .and_then(Target::parse)
                .filter(|target| matches!(target, Target::Guid(_)))
                .ok_or("Room layout actor has no persistent GUID")?;
            let position = values
                .get(BODY)
                .and_then(|body| body.get("pos"))
                .ok_or("Room actor position is missing")?;
            let Value::Vec3(position) = self.decode_field(BODY, "pos", position)? else {
                return Err("Room actor position is not an exact Vec3".into());
            };
            if actors[(kind - 1) as usize]
                .replace((target, position))
                .is_some()
            {
                return Err("Room layout has a duplicate PLAYER, KEY or EXIT".into());
            }
        }
        let [Some(player), Some(key), Some(exit)] = actors else {
            return Err("Room layout requires exactly one PLAYER, KEY and EXIT".into());
        };
        Ok([player, key, exit])
    }

    fn room_wall_rows(&mut self) -> Result<Vec<RoomWallRow>, String> {
        let query = self.call(
            "world.query",
            json!({"components":[ACTOR],"values":true,"limit":69}),
        )?;
        let entities = query
            .get("entities")
            .and_then(J::as_array)
            .ok_or("Room wall query returned no entities")?;
        if query.get("truncated").and_then(J::as_bool) != Some(false)
            || query.get("total").and_then(J::as_u64) != Some(entities.len() as u64)
            || entities.len() > 68
        {
            return Err("Room wall query is incomplete or exceeds the admitted actor limit".into());
        }
        let mut walls = Vec::new();
        for entity in entities {
            let values = entity
                .get("values")
                .and_then(J::as_object)
                .ok_or("Room actor component values are missing")?;
            let kind = values
                .get(ACTOR)
                .and_then(|actor| actor.get("kind"))
                .and_then(J::as_u64)
                .ok_or("Room actor kind is missing")?;
            if kind != u64::from(orr_sample::room_game::WALL) {
                continue;
            }
            let components = entity
                .get("components")
                .and_then(J::as_array)
                .ok_or("Room wall component list is missing")?;
            if components.len() != 3
                || !components
                    .iter()
                    .any(|component| component.as_str() == Some(BODY))
                || !components
                    .iter()
                    .any(|component| component.as_str() == Some(COLLIDER))
            {
                return Err("Room wall must contain exactly Actor, Body and Collider".into());
            }
            let target = entity
                .get("guid")
                .and_then(J::as_str)
                .and_then(Target::parse)
                .filter(|target| matches!(target, Target::Guid(_)))
                .ok_or("Room wall has no persistent GUID")?;
            let guid = match &target {
                Target::Guid(guid) => guid.to_string(),
                _ => return Err("Room wall GUID is invalid".into()),
            };
            let body = values.get(BODY).ok_or("Room wall Body is missing")?;
            let raw_position = body.get("pos").ok_or("Room wall Body.pos is missing")?;
            let Value::Vec3(position) = self.decode_field(BODY, "pos", raw_position)? else {
                return Err("Room wall Body.pos is not an exact Vec3".into());
            };
            let collider = values
                .get(COLLIDER)
                .ok_or("Room wall Collider is missing")?;
            let shape = collider
                .get("shape")
                .ok_or("Room wall Collider.shape is missing")?;
            if shape.get("kind").and_then(J::as_str) != Some("box") {
                return Err("Room wall Collider.shape must be a box".into());
            }
            let raw_half_extents = shape
                .get("half_extents")
                .ok_or("Room wall box half_extents are missing")?;
            let Value::Vec3(half_extents) =
                self.decode_field(COLLIDER, "shape.half_extents", raw_half_extents)?
            else {
                return Err("Room wall half_extents are not an exact Vec3".into());
            };
            let name = entity
                .get("name")
                .and_then(J::as_str)
                .unwrap_or_default()
                .to_owned();
            let wall = RoomWall {
                guid,
                name,
                center: [position.x, position.z],
                half_extents: [half_extents.x, half_extents.z],
                center_y: position.y,
                half_height: half_extents.y,
            };
            walls.push(RoomWallRow {
                wall,
                target,
                position,
                half_extents,
                components: J::Object(values.clone()),
            });
        }
        if walls.len() > orr_sample::room_game::MAX_WALLS {
            return Err("Room wall count exceeds the game limit".into());
        }
        walls.sort_by(|left, right| left.wall.guid.cmp(&right.wall.guid));
        Ok(walls)
    }

    fn room_wall_row(&mut self, guid: &str) -> Result<RoomWallRow, String> {
        let target = Target::parse(guid)
            .filter(|target| matches!(target, Target::Guid(_)))
            .ok_or("Room wall GUID must be persistent")?;
        let Target::Guid(guid) = target else {
            return Err("Room wall GUID must be persistent".into());
        };
        let guid = guid.to_string();
        self.room_wall_rows()?
            .into_iter()
            .find(|row| row.wall.guid == guid)
            .ok_or_else(|| "Room wall GUID was not found".into())
    }

    fn admit_saved_room_for_wall_removal(&mut self, guid: &str) -> Result<(), String> {
        let state = self.call("sim.state", J::Null)?;
        if state.get("mode").and_then(J::as_str) != Some("edit") {
            return Err("Wall removal requires Room Edit mode".into());
        }
        let current_path = state
            .get("scene_path")
            .and_then(J::as_str)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .ok_or("Wall removal requires a saved Room project")?;
        let canonical_current = current_path
            .canonicalize()
            .map_err(|error| format!("cannot resolve the current Room scene: {error}"))?;
        let mut root = canonical_current
            .parent()
            .ok_or("Current Room scene has no project parent")?;
        for _ in 0..64 {
            if root.join("orr.project.json").exists() {
                let prepared = orr_sample::room_project::PreparedProject::open(root)
                    .map_err(|error| format!("cannot admit the saved Room project: {error}"))?;
                let canonical_saved = prepared
                    .path()
                    .canonicalize()
                    .map_err(|error| format!("cannot resolve the admitted Room scene: {error}"))?;
                if canonical_saved != canonical_current {
                    return Err(
                        "Saved Room project entry does not match the current scene path".into(),
                    );
                }
                if prepared.models().document.bindings.contains_key(guid) {
                    return Err(
                        "Remove and save this wall's model binding before deleting the wall".into(),
                    );
                }
                return Ok(());
            }
            root = root
                .parent()
                .ok_or("Current Room scene is not inside an admitted Room project")?;
        }
        Err("Current Room scene is not inside a bounded Room project path".into())
    }

    pub(crate) fn read_room_layout(&mut self) -> Result<[[FP; 2]; 3], String> {
        self.room_authoring_ready()?;
        Ok(self
            .room_layout_actors()?
            .map(|(_, position)| [position.x, position.z]))
    }

    pub(crate) fn read_room_walls(&mut self) -> Result<Vec<RoomWall>, String> {
        self.room_authoring_ready()?;
        Ok(self
            .room_wall_rows()?
            .into_iter()
            .map(|row| row.wall)
            .collect())
    }

    fn room_authoring_transaction<Output>(
        &mut self,
        label: &str,
        operation: impl FnOnce(&mut Self) -> Result<Output, String>,
    ) -> Result<Output, String> {
        self.room_authoring_ready()?;
        match self.call("tx.begin", json!({"label":label})) {
            Ok(started) if started.get("ok").and_then(J::as_bool) == Some(true) => {}
            Ok(_) => {
                self.batch_uncertain = true;
                self.connection_lost("Room transaction acknowledgement was malformed; reconnect and inspect before editing; no mutation was submitted");
                return Err("Room transaction ownership could not be confirmed".into());
            }
            Err(error) => {
                if self.down.is_some() {
                    self.batch_uncertain = true;
                    return Err(format!("Room transaction outcome is uncertain; reconnect and inspect before editing: {error}"));
                }
                return Err(error);
            }
        }
        let result = operation(self).and_then(|output| {
            let document = self.call("scene.save", json!({"write":false}))?;
            let text = document.get("text").and_then(J::as_str)
                .ok_or("Room staged scene capture returned no text")?;
            let prepared = orr_sample::room_project::PreparedScene::parse(text)
                .map_err(|error|format!("Room authoring admission refused the staged edit: {error}"))?;
            if document.get("checksum").and_then(orr_remote::wire::parse_checksum)
                != Some(prepared.frame().checksum()) {
                return Err("Room staged document differs from its admitted frame".into());
            }
            let committed = self.call("tx.commit", J::Null)?;
            if committed.get("ok").and_then(J::as_bool) != Some(true) {
                self.batch_uncertain = true;
                self.connection_lost("Room commit acknowledgement was malformed; reconnect and inspect the document/history; no retry was submitted");
                return Err("Room commit outcome is uncertain".into());
            }
            Ok(output)
        });
        match result {
            Ok(output) => {
                self.mark_edited();
                Ok(output)
            }
            Err(error) => {
                if self.down.is_some() {
                    self.batch_uncertain = true;
                    return Err(format!("Room edit outcome is uncertain; reconnect and inspect before editing; no retry was submitted: {error}"));
                }
                match self.call("tx.rollback", J::Null) {
                    Ok(rolled_back) if rolled_back.get("ok").and_then(J::as_bool) == Some(true) => {
                        self.mark_changed();
                        Err(error)
                    }
                    _ => {
                        self.batch_uncertain = true;
                        self.connection_lost("Room rollback could not be confirmed; reconnect and inspect the document/history; no retry was submitted");
                        Err(format!("Room rollback outcome is uncertain: {error}"))
                    }
                }
            }
        }
    }

    // Retained for existing internal numeric callers; viewport drafts use the
    // checked variant with their explicit Read provenance.
    #[allow(dead_code)]
    pub(crate) fn apply_room_layout(&mut self, positions: [[FP; 2]; 3]) -> Result<(), String> {
        self.room_authoring_transaction("Room PLAYER KEY EXIT layout", |editor| {
            let actors = editor.room_layout_actors()?;
            for ((target, previous), [position_x, position_z]) in actors.into_iter().zip(positions) {
                let position = FPVec3::new(position_x, previous.y, position_z);
                editor.call(
                    "world.patch",
                    json!({"entity":target.param(),"component":BODY,"path":"pos","value":value_to_json(&Value::Vec3(position))}),
                )?;
            }
            Ok(())
        })
    }

    pub(crate) fn apply_room_layout_checked(
        &mut self,
        positions: [[FP; 2]; 3],
        expected_checksum: u64,
    ) -> Result<(), String> {
        self.room_authoring_transaction("Room PLAYER KEY EXIT layout", |editor| {
            let state = editor.call("sim.state", J::Null)?;
            if state.get("checksum").and_then(orr_remote::wire::parse_checksum) != Some(expected_checksum) {
                return Err("Room layout source changed; Read Room layout again before Apply".into());
            }
            let actors = editor.room_layout_actors()?;
            for ((target, previous), [position_x, position_z]) in actors.into_iter().zip(positions) {
                let position = FPVec3::new(position_x, previous.y, position_z);
                editor.call(
                    "world.patch",
                    json!({"entity":target.param(),"component":BODY,"path":"pos","value":value_to_json(&Value::Vec3(position))}),
                )?;
            }
            Ok(())
        })
    }

    pub(crate) fn spawn_room_wall(
        &mut self,
        name: &str,
        center: [FP; 2],
        half_extents: [FP; 2],
    ) -> Result<String, String> {
        if name.is_empty() || name.len() > 64 || name.chars().any(char::is_control) {
            return Err("Room wall name must contain 1..=64 bytes without controls".into());
        }
        let target = self.room_authoring_transaction("Create Room wall", |editor| {
            let position = value_to_json(&Value::Vec3(FPVec3::new(center[0], FP::HALF, center[1])));
            let half = value_to_json(&Value::Vec3(FPVec3::new(half_extents[0], FP::ONE, half_extents[1])));
            let created = editor.call("world.spawn", json!({"name":name,"components":{
                ACTOR:{"kind":orr_sample::room_game::WALL,"ordinal":0},
                BODY:{"pos":position,"rot":{"kind":"unit","x":0,"y":0,"z":0,"w":1},
                    "vel":[0,0,0],"omega":[0,0,0],"inv_mass":0,"inv_inertia":[0,0,0],
                    "linear_damping":0,"angular_damping":0,"kind":"static"},
                "orr_physics3d::Collider":{"shape":{"kind":"box","half_extents":half},
                    "restitution":0,"friction":value_to_json(&Value::Fixed(FP::HALF)),"layer":1,"mask":0}
            }}))?;
            created.get("guid").and_then(J::as_str).and_then(Target::parse)
                .filter(|target|matches!(target,Target::Guid(_)))
                .ok_or_else(||"Room wall creation returned no persistent GUID".to_owned())
        })?;
        let Target::Guid(guid) = &target else {
            return Err("Room wall identifier was not persistent".into());
        };
        let guid = guid.to_string();
        self.select(Some(target));
        self.inspect = None;
        self.dirty.inspect = true;
        Ok(guid)
    }

    // Retain the legacy numeric-edit entry point for existing callers and Windows regressions.
    #[allow(dead_code)]
    pub(crate) fn update_room_wall(
        &mut self,
        guid: &str,
        name: &str,
        center: [FP; 2],
        half_extents: [FP; 2],
    ) -> Result<(), String> {
        self.update_room_wall_impl(guid, name, center, half_extents, None)
    }

    pub(crate) fn update_room_wall_checked(
        &mut self,
        guid: &str,
        name: &str,
        center: [FP; 2],
        half_extents: [FP; 2],
        expected_checksum: u64,
        expected_wall: &RoomWall,
    ) -> Result<(), String> {
        self.update_room_wall_impl(guid, name, center, half_extents, Some((expected_checksum, expected_wall)))
    }

    fn update_room_wall_impl(
        &mut self,
        guid: &str,
        name: &str,
        center: [FP; 2],
        half_extents: [FP; 2],
        expected_source: Option<(u64, &RoomWall)>,
    ) -> Result<(), String> {
        validate_room_wall_name(name)?;
        validate_room_wall_extents(half_extents)?;
        self.room_authoring_transaction("Update Room wall", |editor| {
            if let Some((expected, _)) = expected_source {
                let state = editor.call("sim.state", J::Null)?;
                if state.get("checksum").and_then(orr_remote::wire::parse_checksum) != Some(expected) {
                    return Err("Room wall source changed; read the current wall before applying".into());
                }
            }
            let wall = editor.room_wall_row(guid)?;
            if expected_source.is_some_and(|(_, expected)| &wall.wall != expected) {
                return Err("Room wall source changed; read the current wall before applying".into());
            }
            if wall.wall.name == name
                && wall.wall.center == center
                && wall.wall.half_extents == half_extents
            {
                return Ok(());
            }
            let position = FPVec3::new(center[0], wall.position.y, center[1]);
            let half_extents = FPVec3::new(half_extents[0], wall.half_extents.y, half_extents[1]);
            editor.call("world.rename", json!({"entity":wall.target.param(),"name":name}))?;
            editor.call(
                "world.patch",
                json!({"entity":wall.target.param(),"component":BODY,"path":"pos","value":value_to_json(&Value::Vec3(position))}),
            )?;
            editor.call(
                "world.patch",
                json!({"entity":wall.target.param(),"component":COLLIDER,"path":"shape.half_extents","value":value_to_json(&Value::Vec3(half_extents))}),
            )?;
            Ok(())
        })
    }

    pub(crate) fn update_room_wall_vertical_checked(
        &mut self,
        guid: &str,
        center_y: FP,
        half_height: FP,
        expected_checksum: u64,
    ) -> Result<(), String> {
        self.room_authoring_transaction("Update Room wall height", |editor| {
            let state = editor.call("sim.state", J::Null)?;
            if state.get("checksum").and_then(orr_remote::wire::parse_checksum) != Some(expected_checksum) {
                return Err("Room wall source changed; read the current wall before applying height".into());
            }
            let wall = editor.room_wall_row(guid)?;
            let center_patch = json!({"entity":wall.target.param(),"component":BODY,"path":"pos.y","value":value_to_json(&Value::Fixed(center_y))});
            let height_patch = json!({"entity":wall.target.param(),"component":COLLIDER,"path":"shape.half_extents.y","value":value_to_json(&Value::Fixed(half_height))});
            // world.patch also admits each intermediate Room. Expand before
            // moving the center; when shrinking, move within the old larger
            // interval first. Both valid intervals cover the player span.
            let patches = if half_height >= wall.half_extents.y {
                [height_patch, center_patch]
            } else {
                [center_patch, height_patch]
            };
            for patch in patches { editor.call("world.patch", patch)?; }
            // The existing transaction re-admits the entire Room before commit,
            // including the numeric envelope and full player-height coverage.
            Ok(())
        })
    }

    pub(crate) fn duplicate_room_wall(
        &mut self,
        guid: &str,
        name: &str,
        center: [FP; 2],
    ) -> Result<String, String> {
        validate_room_wall_name(name)?;
        self.room_authoring_transaction("Duplicate Room wall", |editor| {
            let wall = editor.room_wall_row(guid)?;
            let position = FPVec3::new(center[0], wall.position.y, center[1]);
            let created = editor.call(
                "world.spawn",
                json!({"name":name,"components":wall.components}),
            )?;
            let target = created
                .get("guid")
                .and_then(J::as_str)
                .and_then(Target::parse)
                .filter(|target| matches!(target, Target::Guid(_)))
                .ok_or("Room wall duplicate returned no persistent GUID")?;
            editor.call(
                "world.patch",
                json!({"entity":target.param(),"component":BODY,"path":"pos","value":value_to_json(&Value::Vec3(position))}),
            )?;
            match target {
                Target::Guid(guid) => Ok(guid.to_string()),
                _ => Err("Room wall duplicate GUID is invalid".into()),
            }
        })
    }

    pub(crate) fn remove_room_wall(&mut self, guid: &str) -> Result<(), String> {
        let target = Target::parse(guid)
            .filter(|target| matches!(target, Target::Guid(_)))
            .ok_or("Room wall GUID must be persistent")?;
        let Target::Guid(guid_value) = target else {
            return Err("Room wall GUID must be persistent".into());
        };
        let guid = guid_value.to_string();
        self.room_authoring_transaction("Remove Room wall", |editor| {
            editor.admit_saved_room_for_wall_removal(&guid)?;
            let wall = editor.room_wall_row(&guid)?;
            editor.call("world.despawn", json!({"entity":wall.target.param()}))?;
            Ok(())
        })
    }

    #[cfg(any(test, all(windows, target_arch = "x86_64")))]
    pub(crate) fn capture_room_wall_stamp(
        &mut self,
        guids: &[String],
    ) -> Result<crate::room_wall_stamp::Template, String> {
        self.room_authoring_ready()?;
        if guids.is_empty() || guids.len() > orr_sample::room_game::MAX_WALLS {
            return Err("select 1..=64 Room walls to capture".into());
        }
        let mut selected = BTreeSet::new();
        for guid in guids {
            let target = Target::parse(guid)
                .filter(|target| matches!(target, Target::Guid(_)))
                .ok_or("Room wall stamp selection requires persistent GUIDs")?;
            let Target::Guid(parsed) = target else {
                return Err("Room wall stamp selection requires persistent GUIDs".into());
            };
            if !selected.insert(parsed.to_string()) {
                return Err("Room wall stamp selection contains a duplicate GUID".into());
            }
        }
        let rows = self.room_wall_rows()?;
        let by_guid = rows
            .into_iter()
            .map(|row| (row.wall.guid.clone(), row))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut captured = Vec::with_capacity(guids.len());
        for guid in guids {
            let target = Target::parse(guid)
                .filter(|target| matches!(target, Target::Guid(_)))
                .ok_or("Room wall stamp selection requires persistent GUIDs")?;
            let Target::Guid(parsed) = target else {
                return Err("Room wall stamp selection requires persistent GUIDs".into());
            };
            let normalized_guid = parsed.to_string();
            let row = by_guid
                .get(&normalized_guid)
                .ok_or("selected GUID is not a current Room wall")?;
            captured.push((row.wall.name.clone(), row.components.clone()));
        }
        crate::room_wall_stamp::Template::capture(captured)
    }

    #[cfg(any(test, all(windows, target_arch = "x86_64")))]
    pub(crate) fn instantiate_room_wall_stamp(
        &mut self,
        template: &crate::room_wall_stamp::Template,
        offset: [FP; 2],
    ) -> Result<Vec<String>, String> {
        template.encode()?;
        self.room_authoring_transaction("Instantiate Room wall stamp", |editor| {
            let current_walls = editor.room_wall_rows()?;
            if current_walls.len() + template.walls().len() > orr_sample::room_game::MAX_WALLS {
                return Err("wall stamp would exceed the Room wall limit".into());
            }
            let actor_query = editor.call(
                "world.query",
                json!({"components":[ACTOR],"values":false,"limit":69}),
            )?;
            let actor_entities = actor_query
                .get("entities")
                .and_then(J::as_array)
                .ok_or("Room actor query returned no entities")?;
            if actor_query.get("truncated").and_then(J::as_bool) != Some(false)
                || actor_query.get("total").and_then(J::as_u64) != Some(actor_entities.len() as u64)
                || actor_entities.len() > 68
            {
                return Err(
                    "Room actor query is incomplete or exceeds the admitted actor limit".into(),
                );
            }
            let mut actor_guids = BTreeSet::new();
            for entity in actor_entities {
                let guid = entity
                    .get("guid")
                    .and_then(J::as_str)
                    .and_then(Target::parse)
                    .filter(|target| matches!(target, Target::Guid(_)))
                    .ok_or("Room actor has no persistent GUID")?;
                let Target::Guid(guid) = guid else {
                    return Err("Room actor GUID is invalid".into());
                };
                actor_guids.insert(guid.to_string());
            }
            let mut created_guids = BTreeSet::new();
            let mut result = Vec::with_capacity(template.walls().len());
            for wall in template.walls() {
                let mut components = wall
                    .components
                    .as_object()
                    .cloned()
                    .map(J::Object)
                    .ok_or("wall stamp components must be an object")?;
                let raw_position = components
                    .get(BODY)
                    .and_then(|body| body.get("pos"))
                    .ok_or("wall stamp Body.pos is missing")?;
                let Value::Vec3(mut position) = editor.decode_field(BODY, "pos", raw_position)?
                else {
                    return Err("wall stamp Body.pos is not an exact Vec3".into());
                };
                position.x = position
                    .x
                    .checked_add(offset[0])
                    .ok_or("wall stamp X offset exceeds fixed-point bounds")?;
                position.z = position
                    .z
                    .checked_add(offset[1])
                    .ok_or("wall stamp Z offset exceeds fixed-point bounds")?;
                let body = components
                    .get_mut(BODY)
                    .and_then(J::as_object_mut)
                    .ok_or("wall stamp Body must be an object")?;
                body.insert("pos".into(), value_to_json(&Value::Vec3(position)));
                let spawned = editor.call(
                    "world.spawn",
                    json!({"name":&wall.name,"components":components}),
                )?;
                let target = spawned
                    .get("guid")
                    .and_then(J::as_str)
                    .and_then(Target::parse)
                    .filter(|target| matches!(target, Target::Guid(_)))
                    .ok_or("Room wall stamp spawn returned no persistent GUID")?;
                let Target::Guid(guid) = target else {
                    return Err("Room wall stamp spawn returned an invalid GUID".into());
                };
                let guid = guid.to_string();
                if actor_guids.contains(&guid) || !created_guids.insert(guid.clone()) {
                    return Err("Room wall stamp spawn returned a duplicate GUID".into());
                }
                result.push(guid);
            }
            Ok(result)
        })
    }
}

#[cfg(test)]
#[path = "room_wall_stamp_editor_tests.rs"]
mod room_wall_stamp_editor_tests;

fn validate_room_wall_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.len() > 64 || name.chars().any(char::is_control) {
        return Err("Room wall name must contain 1..=64 bytes without controls".into());
    }
    Ok(())
}

fn validate_room_wall_extents(half_extents: [FP; 2]) -> Result<(), String> {
    if half_extents.iter().any(|extent| *extent <= FP::ZERO) {
        return Err("Room wall half extents must be positive".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "room_wall_update_checked_tests.rs"]
mod room_wall_update_checked_tests;
