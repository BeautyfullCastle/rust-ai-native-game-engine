use super::{Editor, Mode, Pend, Target, J};
use orr_fp::{FPVec3, FP};
use orr_reflect::Value;
use orr_remote::json::value_to_json;
use serde_json::json;

const ACTOR: &str = orr_sample::room_project::ACTOR;
const BODY: &str = "orr_physics3d::Body";

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

    pub(crate) fn read_room_layout(&mut self) -> Result<[[FP; 2]; 3], String> {
        self.room_authoring_ready()?;
        Ok(self
            .room_layout_actors()?
            .map(|(_, position)| [position.x, position.z]))
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
}
