//! Bounded linked CollectDodge authoring. No filesystem or simulation authority.
use crate::{EditError, EditorDoc, FragmentInstance, Op, Origin, SceneFragment};
use orr_fp::FPVec2;
use orr_reflect::{Guid, PrefabLink, Scene, SceneEntity, Value};
use std::collections::{BTreeMap, BTreeSet};

const ACTOR: &str = "CollectDodgeV1::Actor";
fn invalid(message: impl Into<String>) -> EditError {
    EditError::Invalid(message.into())
}
fn actor(entity: &SceneEntity) -> Result<&Value, EditError> {
    if entity.components.len() != 1 || entity.components[0].0 != ACTOR {
        return Err(invalid(
            "linked collect source requires exactly one Actor per entity",
        ));
    }
    Ok(&entity.components[0].1)
}
fn number(value: &Value, field: &str) -> Result<u32, EditError> {
    match value.field(field) {
        Some(Value::Int(n)) => u32::try_from(*n).map_err(|_| invalid("invalid Actor integer")),
        _ => Err(invalid("missing Actor integer")),
    }
}
fn replace(value: &mut Value, field: &str, new: Value) -> Result<(), EditError> {
    let Value::Struct(fields) = value else {
        return Err(invalid("Actor is not a struct"));
    };
    *fields
        .iter_mut()
        .find(|(name, _)| name == field)
        .map(|(_, value)| value)
        .ok_or_else(|| invalid("missing Actor field"))? = new;
    Ok(())
}
fn source(doc: &EditorDoc, text: &str) -> Result<SceneFragment, EditError> {
    if text.len() > 32 * 1024 {
        return Err(invalid("linked source exceeds 32 KiB"));
    }
    let parsed = Scene::parse(text, doc.types())?;
    if !parsed.prefab_links.is_empty() || !parsed.singletons.is_empty() {
        return Err(invalid(
            "linked sources cannot contain singleton state or nested links",
        ));
    }
    if parsed.entities.is_empty() || parsed.entities.len() > 8 {
        return Err(invalid("linked source requires 1..=8 entities"));
    }
    for entity in parsed.entities.values() {
        let value = actor(entity)?;
        if !matches!(number(value, "kind")?, 1 | 2) {
            return Err(invalid(
                "only collectible and hazard sources are supported; no player copies",
            ));
        }
    }
    SceneFragment::capture(
        &parsed,
        &parsed.entities.keys().cloned().collect::<Vec<_>>(),
    )
}
impl EditorDoc {
    /// Capture closed non-player Collect actors as a canonical schema-1 source.
    pub fn capture_linked_collect_source(&self, selected: &[Guid]) -> Result<String, EditError> {
        if selected.iter().any(|g| {
            self.scene
                .prefab_links
                .values()
                .any(|link| link.guids.values().any(|target| target == g))
        }) {
            return Err(invalid("nested linked source capture is unsupported"));
        }
        let fragment = SceneFragment::capture(self.scene(), selected)?;
        source(self, &fragment.to_yaml()).map(|f| f.to_yaml())
    }

    /// Instantiate a source using fresh GUIDs and contiguous per-kind ordinals.
    /// Source is an inert relative identity; callers supply already-read bytes.
    pub fn instantiate_linked_collect(
        &mut self,
        identity: &str,
        text: &str,
        origin: Origin,
    ) -> Result<FragmentInstance, EditError> {
        if identity.len() > 240 {
            return Err(invalid("source identity exceeds 240 bytes"));
        }
        self.scene
            .validate_prefab_links(&self.types)
            .map_err(invalid)?;
        if self.scene.prefab_links.len() >= 8 {
            return Err(invalid("at most eight linked instances are supported"));
        }
        let fragment = source(self, text)?;
        let baseline = fragment.to_yaml();
        let (instance, mut ops) = self.plan_fragment(&fragment, None)?;
        let mut next = BTreeMap::<u32, u32>::new();
        for entity in self.scene.entities.values() {
            let value = actor(entity)?;
            let kind = number(value, "kind")?;
            let ordinal = number(value, "ordinal")?;
            let count = next.entry(kind).or_default();
            *count = (*count).max(
                ordinal
                    .checked_add(1)
                    .ok_or_else(|| invalid("ordinal overflow"))?,
            );
        }
        let mut ordinals = BTreeMap::new();
        for (id, entity) in fragment.entities() {
            let kind = number(actor(entity)?, "kind")?;
            let value = next.entry(kind).or_default();
            ordinals.insert(id.clone(), *value);
            *value = value
                .checked_add(1)
                .ok_or_else(|| invalid("ordinal overflow"))?;
        }
        for op in &mut ops {
            if let Op::AddComponent {
                guid,
                value: Some(value),
                ..
            } = op
            {
                let source = instance
                    .guids
                    .iter()
                    .find(|(_, target)| *target == guid)
                    .map(|(source, _)| source)
                    .expect("planned fragment GUID");
                replace(value, "ordinal", Value::Int(i128::from(ordinals[source])))?;
            }
        }
        let key = instance
            .guids
            .values()
            .next()
            .expect("nonempty source")
            .clone();
        let link = PrefabLink {
            source: identity.into(),
            digest: PrefabLink::canonical_digest(&baseline),
            baseline,
            guids: instance.guids.clone(),
            ordinals,
            position_overrides: BTreeSet::new(),
        };
        let mut links = self.scene.prefab_links.clone();
        links.insert(key, link);
        self.apply_linked_batch("instantiate linked Collect group", ops, links, origin)?;
        Ok(instance)
    }

    /// Explicit position override; ordinary untracked edits remain rejected.
    pub fn override_linked_collect_position(
        &mut self,
        instance: &Guid,
        source_guid: &Guid,
        position: FPVec2,
        origin: Origin,
    ) -> Result<(), EditError> {
        self.scene
            .validate_prefab_links(&self.types)
            .map_err(invalid)?;
        let mut links = self.scene.prefab_links.clone();
        let link = links
            .get_mut(instance)
            .ok_or_else(|| invalid("unknown linked instance"))?;
        let target = link
            .guids
            .get(source_guid)
            .ok_or_else(|| invalid("unknown source GUID"))?
            .clone();
        link.position_overrides.insert(source_guid.clone());
        self.apply_linked_batch(
            "override linked position",
            vec![Op::SetField {
                guid: target,
                component: ACTOR.into(),
                path: "position".into(),
                value: Value::Vec2(position),
            }],
            links,
            origin,
        )
    }

    /// Restore the source position and clear its explicit override marker.
    pub fn revert_linked_collect_position(
        &mut self,
        instance: &Guid,
        source_guid: &Guid,
        origin: Origin,
    ) -> Result<(), EditError> {
        self.scene
            .validate_prefab_links(&self.types)
            .map_err(invalid)?;
        let mut links = self.scene.prefab_links.clone();
        let link = links
            .get_mut(instance)
            .ok_or_else(|| invalid("unknown linked instance"))?;
        let target = link
            .guids
            .get(source_guid)
            .ok_or_else(|| invalid("unknown source GUID"))?
            .clone();
        let baseline = source(self, &link.baseline)?;
        let position = actor(&baseline.entities()[source_guid])?
            .field("position")
            .ok_or_else(|| invalid("missing source position"))?
            .clone();
        link.position_overrides.remove(source_guid);
        self.apply_linked_batch(
            "revert linked position",
            vec![Op::SetField {
                guid: target,
                component: ACTOR.into(),
                path: "position".into(),
                value: position,
            }],
            links,
            origin,
        )
    }

    /// Explicitly update one instance from owned source bytes. Source identity
    /// and prior digest are optimistic guards; no filesystem is consulted.
    pub fn update_linked_collect(
        &mut self,
        instance: &Guid,
        identity: &str,
        expected_digest: &str,
        text: &str,
        origin: Origin,
    ) -> Result<(), EditError> {
        self.scene
            .validate_prefab_links(&self.types)
            .map_err(invalid)?;
        let old = self
            .scene
            .prefab_links
            .get(instance)
            .ok_or_else(|| invalid("unknown linked instance"))?;
        if old.source != identity || old.digest != expected_digest {
            return Err(invalid(
                "source identity or baseline digest changed; refresh before updating",
            ));
        }
        let previous = source(self, &old.baseline)?;
        let new = source(self, text)?;
        if previous.entities().keys().ne(new.entities().keys()) {
            return Err(invalid("source entity topology changed; update rejected"));
        }
        let mut ops = Vec::new();
        for (guid, entity) in new.entities() {
            let old_value = actor(&previous.entities()[guid])?;
            let mut value = actor(entity)?.clone();
            if number(old_value, "kind")? != number(&value, "kind")?
                || number(old_value, "ordinal")? != number(&value, "ordinal")?
            {
                return Err(invalid("source kind or ordinal changed; update rejected"));
            }
            replace(
                &mut value,
                "ordinal",
                Value::Int(i128::from(old.ordinals[guid])),
            )?;
            let target = &old.guids[guid];
            if old.position_overrides.contains(guid) {
                let current = actor(&self.scene.entities[target])?
                    .field("position")
                    .ok_or_else(|| invalid("missing overridden position"))?
                    .clone();
                replace(&mut value, "position", current)?;
            }
            ops.push(Op::SetField {
                guid: target.clone(),
                component: ACTOR.into(),
                path: String::new(),
                value,
            });
            ops.push(Op::Rename {
                guid: target.clone(),
                name: entity.name.clone(),
            });
        }
        let mut links = self.scene.prefab_links.clone();
        let link = links.get_mut(instance).expect("known instance");
        link.baseline = new.to_yaml();
        link.digest = PrefabLink::canonical_digest(&link.baseline);
        self.apply_linked_batch("update linked Collect source", ops, links, origin)
    }
}
