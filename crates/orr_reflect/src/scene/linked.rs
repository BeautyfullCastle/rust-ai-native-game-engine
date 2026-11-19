//! Closed, bounded editor metadata. Source paths are inert labels, never opened.
use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use super::node::{Diag, Node, NodeKind};
use super::{decode, Guid, Scene, SceneEntity, SCENE_SCHEMA};
use crate::{TypeRegistry, Value};

const PROFILE: &str = "collect-actors-v1";
const GENERIC: &str = "generic-v1";
/// Generic prefab limits. Collect links keep their own, smaller, bounds.
const MAX_GENERIC_SCENE: usize = 4 * 1024 * 1024;
const MAX_GENERIC_BASELINES: usize = 2 * 1024 * 1024;
const MAX_GENERIC_LINKS: usize = 1024;
const MAX_GENERIC_ENTITIES: usize = 256;
const ACTOR: &str = "CollectDodgeV1::Actor";
const MAX_SCENE: usize = 64 * 1024;
const MAX_BASELINES: usize = 32 * 1024;

/// How a link relates its flattened instance entities to the source snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkProfile {
    /// Fixed-topology CollectDodge actors with ordinals and a position override.
    CollectActorsV1,
    /// Any registered components. Overrides are not stored: they are the leaf
    /// differences between the live entities and the baseline snapshot.
    GenericV1,
}

/// A bounded source snapshot and its explicit instance allocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrefabLink {
    /// Which validation and update rules this link follows.
    pub profile: LinkProfile,
    /// Normalized, portable, relative source label; never used for filesystem access.
    pub source: String,
    /// Lowercase SHA-256 of the exact canonical baseline bytes.
    pub digest: String,
    /// Canonical, comment-free `orr.scene/1` entity-only source fragment.
    pub baseline: String,
    /// Source GUID to live scene GUID; complete and injective.
    pub guids: BTreeMap<Guid, Guid>,
    /// Source GUID to allocated actor ordinal (empty for generic links).
    pub ordinals: BTreeMap<Guid, u32>,
    /// Source GUIDs whose live positions intentionally override the baseline
    /// (empty for generic links).
    pub position_overrides: BTreeSet<Guid>,
}

impl PrefabLink {
    /// SHA-256 of canonical scene text, encoded as lowercase hexadecimal.
    pub fn canonical_digest(text: &str) -> String {
        Sha256::digest(text.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

fn map(n: &Node) -> Result<&[(Node, Node)], Diag> {
    match &n.kind {
        NodeKind::Map(entries) => Ok(entries),
        _ => Err(Diag::new(n.pos, "prefab metadata must be a map")),
    }
}

fn scalar(n: &Node) -> Result<&str, Diag> {
    n.key_text()
        .ok_or_else(|| Diag::new(n.pos, "prefab metadata must be text"))
}

fn guid(n: &Node) -> Result<Guid, Diag> {
    Guid::parse(scalar(n)?).map_err(|e| Diag::new(n.pos, e))
}

pub(super) fn decode_scene(
    root: &Node,
    text_len: usize,
    reg: &TypeRegistry,
) -> Result<Scene, Vec<Diag>> {
    let version2 = matches!(&root.kind, NodeKind::Map(entries) if entries.iter().any(|(k, v)| k.key_text() == Some("schema") && v.key_text() == Some("orr.scene/2")));
    // Schema 2 is admitted only for the closed Actor profile, matching the
    // optional JSON Schema export. Unrelated registries retain legacy version
    // diagnostics (including their positions and full diagnostic list).
    if !version2 || !(supports_profile(reg) || declares_only_generic(root)) {
        return decode::decode_scene(root, reg);
    }
    decode_linked(root, text_len, reg).map_err(|d| vec![d])
}

fn profile_of(n: &Node) -> Option<LinkProfile> {
    let NodeKind::Map(fields) = &n.kind else { return None };
    fields.iter().find(|(k, _)| k.key_text() == Some("profile")).and_then(|(_, v)| match v.key_text() {
        Some(PROFILE) => Some(LinkProfile::CollectActorsV1),
        Some(GENERIC) => Some(LinkProfile::GenericV1),
        _ => None,
    })
}

/// True if the document has links and every one declares the generic profile.
fn declares_only_generic(root: &Node) -> bool {
    let NodeKind::Map(entries) = &root.kind else { return false };
    let Some((_, prefabs)) = entries.iter().find(|(k, _)| k.key_text() == Some("prefabs")) else { return false };
    let NodeKind::Map(links) = &prefabs.kind else { return false };
    !links.is_empty() && links.iter().all(|(_, v)| profile_of(v) == Some(LinkProfile::GenericV1))
}

fn decode_linked(root: &Node, text_len: usize, reg: &TypeRegistry) -> Result<Scene, Diag> {
    let entries = map(root)?;
    let metadata = entries
        .iter()
        .find(|(k, _)| k.key_text() == Some("prefabs"))
        .map(|(_, v)| v)
        .ok_or_else(|| Diag::new(root.pos, "orr.scene/2 requires nonempty prefabs"))?;
    let links = map(metadata)?;
    let generic = declares_only_generic(root);
    if text_len > if generic { MAX_GENERIC_SCENE } else { MAX_SCENE } {
        return Err(Diag::new(root.pos, if generic { "orr.scene/2 exceeds 4 MiB" } else { "orr.scene/2 exceeds 64 KiB" }));
    }
    if links.is_empty() || links.len() > if generic { MAX_GENERIC_LINKS } else { 8 } {
        return Err(Diag::new(metadata.pos, "prefabs requires 1 to 8 links (1 to 1024 for generic-v1)"));
    }
    // Bound all embedded text before parsing any baseline or decoding entities.
    let mut baseline_bytes = 0usize;
    for (_, n) in links {
        for (k, v) in map(n)? {
            if k.key_text() == Some("baseline") {
                baseline_bytes = baseline_bytes.saturating_add(scalar(v)?.len());
            }
        }
    }
    if baseline_bytes > if generic { MAX_GENERIC_BASELINES } else { MAX_BASELINES } {
        return Err(Diag::new(
            metadata.pos,
            "aggregate prefab baselines exceed 32 KiB",
        ));
    }
    let mut flattened = root.clone();
    if let NodeKind::Map(entries) = &mut flattened.kind {
        entries.retain(|(k, _)| k.key_text() != Some("prefabs"));
        for (k, v) in entries {
            if k.key_text() == Some("schema") {
                v.kind = NodeKind::Scalar {
                    text: SCENE_SCHEMA.into(),
                    quoted: false,
                };
            }
        }
    }
    let mut scene = decode::decode_scene(&flattened, reg).map_err(|d| d[0].clone())?;
    for (key, n) in links {
        let mut fields = BTreeMap::new();
        for (k, v) in map(n)? {
            let name = scalar(k)?;
            if !matches!(
                name,
                "profile"
                    | "source"
                    | "digest"
                    | "baseline"
                    | "guids"
                    | "ordinals"
                    | "position_overrides"
            ) {
                return Err(Diag::new(k.pos, format!("unknown prefab field '{name}'")));
            }
            fields.insert(name, v);
        }
        let get = |key: &str| {
            fields
                .get(key)
                .copied()
                .ok_or_else(|| Diag::new(n.pos, format!("missing prefab field '{key}'")))
        };
        let profile = profile_of(n).ok_or_else(|| {
            Diag::new(n.pos, "unsupported prefab profile (expected collect-actors-v1 or generic-v1)")
        })?;
        if (profile == LinkProfile::GenericV1) != generic {
            return Err(Diag::new(n.pos, "a scene cannot mix prefab link profiles"));
        }
        let baseline = get("baseline")?;
        if !matches!(&baseline.kind, NodeKind::Scalar { quoted: true, .. }) {
            return Err(Diag::new(
                baseline.pos,
                "prefab baseline must be quoted canonical scene text",
            ));
        }
        let mut link = PrefabLink {
            profile,
            source: scalar(get("source")?)?.into(),
            digest: scalar(get("digest")?)?.into(),
            baseline: scalar(baseline)?.into(),
            guids: BTreeMap::new(),
            ordinals: BTreeMap::new(),
            position_overrides: BTreeSet::new(),
        };
        let mappings = map(get("guids")?)?;
        let max_entities = if generic { MAX_GENERIC_ENTITIES } else { 8 };
        if mappings.is_empty() || mappings.len() > max_entities {
            return Err(Diag::new(n.pos, "prefab guids exceeds its entity limit"));
        }
        for (k, v) in mappings {
            link.guids.insert(guid(k)?, guid(v)?);
        }
        if generic {
            if fields.contains_key("ordinals") || fields.contains_key("position_overrides") {
                return Err(Diag::new(n.pos, "generic-v1 links have no ordinals or position_overrides"));
            }
            scene.prefab_links.insert(guid(key)?, link);
            continue;
        }
        for (k, v) in map(get("ordinals")?)? {
            let text = scalar(v)?;
            if !matches!(&v.kind, NodeKind::Scalar { quoted: false, .. })
                || text.is_empty()
                || !text.bytes().all(|b| b.is_ascii_digit())
            {
                return Err(Diag::new(
                    v.pos,
                    "prefab ordinal must be a plain u32 decimal",
                ));
            }
            let ordinal = text
                .parse::<u32>()
                .map_err(|_| Diag::new(v.pos, "prefab ordinal exceeds u32"))?;
            link.ordinals.insert(guid(k)?, ordinal);
        }
        let overrides = get("position_overrides")?;
        let NodeKind::Seq(overrides) = &overrides.kind else {
            return Err(Diag::new(
                overrides.pos,
                "position_overrides must be a GUID list",
            ));
        };
        for n in overrides {
            if !link.position_overrides.insert(guid(n)?) {
                return Err(Diag::new(n.pos, "duplicate position override"));
            }
        }
        scene.prefab_links.insert(guid(key)?, link);
    }
    scene
        .validate_prefab_links(reg)
        .map_err(|e| Diag::new(metadata.pos, e))?;
    Ok(scene)
}

fn validate_source(source: &str) -> Result<(), String> {
    if source.is_empty()
        || source.len() > 240
        || !source
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'-' | b'.'))
    {
        return Err(
            "prefab source must be a portable relative ASCII path of 1 to 240 bytes".into(),
        );
    }
    for part in source.split('/') {
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.len() == 4
                && matches!(stem.as_bytes()[3], b'1'..=b'9'));
        if part.is_empty() || part == "." || part == ".." || part.ends_with('.') || reserved {
            return Err("prefab source contains an empty, dot, or reserved path segment".into());
        }
    }
    Ok(())
}

fn actor(entity: &SceneEntity) -> Result<&Value, String> {
    if entity.components.len() != 1 || entity.components[0].0 != ACTOR {
        return Err(
            "collect-actors-v1 requires exactly CollectDodgeV1::Actor on each entity".into(),
        );
    }
    let value = &entity.components[0].1;
    let Value::Struct(fields) = value else {
        return Err("Actor must be a struct".into());
    };
    if fields.len() != 4
        || fields[0].0 != "position"
        || fields[1].0 != "velocity"
        || fields[2].0 != "kind"
        || fields[3].0 != "ordinal"
        || !matches!(fields[0].1, Value::Vec2(_))
        || !matches!(fields[1].1, Value::Vec2(_))
        || !matches!(fields[2].1, Value::Int(1 | 2))
        || !matches!(fields[3].1, Value::Int(n) if n >= 0 && n <= i128::from(u32::MAX))
    {
        return Err("collect-actors-v1 requires position/velocity Vec2, collectible or hazard kind, and u32 ordinal".into());
    }
    Ok(value)
}

/// Shared capability gate for loader validation and optional schema export.
pub(super) fn supports_profile(reg: &TypeRegistry) -> bool {
    use crate::{IntKind, Kind, TypeKind};
    let Some(info) = reg.get(ACTOR) else {
        return false;
    };
    let fields = info.fields();
    info.kind() == TypeKind::Component
        && fields.len() == 4
        && fields[0].name == "position"
        && fields[1].name == "velocity"
        && fields[2].name == "kind"
        && fields[3].name == "ordinal"
        && matches!(fields[0].ty.kind, Kind::Vec2 { .. })
        && matches!(fields[1].ty.kind, Kind::Vec2 { .. })
        && matches!(
            fields[2].ty.kind,
            Kind::Int {
                int: IntKind::U32,
                ..
            }
        )
        && matches!(
            fields[3].ty.kind,
            Kind::Int {
                int: IntKind::U32,
                ..
            }
        )
}

impl Scene {
    /// Validates bounded metadata and proves that the flattened scene agrees with each link.
    /// No source filesystem access occurs. Call after an entire document operation, not
    /// between its intermediate edits.
    pub fn validate_prefab_links(&self, reg: &TypeRegistry) -> Result<(), String> {
        if self.prefab_links.is_empty() {
            return Ok(());
        }
        let generic = self.prefab_links.values().filter(|l| l.profile == LinkProfile::GenericV1).count();
        if generic != 0 {
            if generic != self.prefab_links.len() {
                return Err("a scene cannot mix prefab link profiles".into());
            }
            return self.validate_generic_links(reg);
        }
        if self.prefab_links.len() > 8 {
            return Err("prefabs exceeds 8 links".into());
        }
        let baseline_bytes = self
            .prefab_links
            .values()
            .fold(0usize, |n, l| n.saturating_add(l.baseline.len()));
        if baseline_bytes > MAX_BASELINES {
            return Err("aggregate prefab baselines exceed 32 KiB".into());
        }
        // Check bounded public metadata fields before allocating any encoded
        // output. Callers may construct links directly rather than parse YAML.
        for link in self.prefab_links.values() {
            validate_source(&link.source)?;
            if link.guids.is_empty() || link.guids.len() > 8 {
                return Err("prefab requires 1 to 8 entities".into());
            }
            if link.ordinals.len() != link.guids.len()
                || link.position_overrides.len() > link.guids.len()
            {
                return Err("prefab ordinal/override mappings exceed their source key set".into());
            }
            if link.digest.len() != 64 {
                return Err(
                    "prefab baseline SHA-256 digest must contain 64 lowercase hex characters"
                        .into(),
                );
            }
        }
        // Includes escaping overhead and live entities when admitting hand-built scenes.
        if self.to_yaml().len() > MAX_SCENE {
            return Err("orr.scene/2 exceeds 64 KiB".into());
        }
        if !supports_profile(reg) {
            return Err("collect-actors-v1 requires the exact Actor reflection profile".into());
        }
        let mut owned = BTreeSet::new();
        for (root, link) in &self.prefab_links {
            validate_source(&link.source)?;
            if link.guids.is_empty() || link.guids.len() > 8 {
                return Err("prefab requires 1 to 8 entities".into());
            }
            if !link.guids.values().any(|g| g == root) {
                return Err("prefab root must be one of its mapped live targets".into());
            }
            if link.digest.len() != 64
                || !link
                    .digest
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                || link.digest != PrefabLink::canonical_digest(&link.baseline)
            {
                return Err("prefab baseline SHA-256 digest mismatch".into());
            }
            // Explicit non-recursive baseline mode: legacy decoder rejects schema/2
            // and all metadata. Never call Scene::parse on embedded text.
            let node = super::node::parse(&link.baseline)
                .map_err(|e| format!("invalid prefab baseline: {}", e.message))?;
            let mut baseline = decode::decode_scene(&node, reg)
                .map_err(|e| format!("invalid prefab baseline: {}", e[0].message))?;
            baseline.read_comments(&link.baseline, &node);
            if !baseline.singletons.is_empty()
                || baseline.entities.is_empty()
                || baseline.entities.len() > 8
                || !baseline.header_comments.is_empty()
                || !baseline.comments.is_empty()
            {
                return Err("prefab baseline must be a comment-free entity-only fragment of 1 to 8 entities".into());
            }
            if baseline.to_yaml() != link.baseline {
                return Err("prefab baseline is not canonical scene/1 text".into());
            }
            if !baseline.entities.keys().eq(link.guids.keys())
                || !link.guids.keys().eq(link.ordinals.keys())
                || !link
                    .position_overrides
                    .iter()
                    .all(|g| link.guids.contains_key(g))
            {
                return Err(
                    "prefab mapping/ordinals must cover baseline keys; overrides must be a subset"
                        .into(),
                );
            }
            for (source, target) in &link.guids {
                if !owned.insert(target.clone()) {
                    return Err(
                        "prefab live targets must be injective and disjoint across links".into(),
                    );
                }
                let live = self
                    .entities
                    .get(target)
                    .ok_or_else(|| format!("prefab live target {target} is missing"))?;
                actor(live)?;
                let mut expected = baseline.entities[source].clone();
                actor(&expected)?;
                let Value::Struct(fields) = &mut expected.components[0].1 else {
                    unreachable!()
                };
                fields[3].1 = Value::Int(i128::from(link.ordinals[source]));
                if link.position_overrides.contains(source) {
                    fields[0].1 = live.components[0]
                        .1
                        .field("position")
                        .expect("validated Actor")
                        .clone();
                }
                if &expected != live {
                    return Err(format!("prefab live target {target} differs from baseline outside allocated ordinal/position override"));
                }
            }
        }
        Ok(())
    }
}

pub(super) fn write_links(scene: &Scene, out: &mut String) {
    use super::encode::quote;
    if scene.prefab_links.is_empty() {
        return;
    }
    out.push_str("prefabs:\n");
    for (root, link) in &scene.prefab_links {
        if link.profile == LinkProfile::GenericV1 {
            out.push_str(&format!("  {root}:\n    profile: {GENERIC}\n    source: {}\n    digest: {}\n    baseline: {}\n    guids:\n", quote(&link.source), quote(&link.digest), quote(&link.baseline)));
            for (source, target) in &link.guids {
                out.push_str(&format!("      {source}: {target}\n"));
            }
            continue;
        }
        out.push_str(&format!("  {root}:\n    profile: {PROFILE}\n    source: {}\n    digest: {}\n    baseline: {}\n    guids:\n", quote(&link.source), quote(&link.digest), quote(&link.baseline)));
        for (source, target) in &link.guids {
            out.push_str(&format!("      {source}: {target}\n"));
        }
        out.push_str("    ordinals:\n");
        for (source, ordinal) in &link.ordinals {
            out.push_str(&format!("      {source}: {ordinal}\n"));
        }
        out.push_str("    position_overrides: [");
        for (i, source) in link.position_overrides.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(source.as_str());
        }
        out.push_str("]\n");
    }
}

impl Scene {
    /// Structural validation of generic links. The live entities are not
    /// compared with the baseline: their differences are the instance overrides.
    /// A mapped live entity may be missing (the instance deleted that member).
    fn validate_generic_links(&self, reg: &TypeRegistry) -> Result<(), String> {
        if self.prefab_links.len() > MAX_GENERIC_LINKS {
            return Err("prefabs exceeds 1024 generic links".into());
        }
        let baseline_bytes = self.prefab_links.values().fold(0usize, |n, l| n.saturating_add(l.baseline.len()));
        if baseline_bytes > MAX_GENERIC_BASELINES {
            return Err("aggregate prefab baselines exceed 2 MiB".into());
        }
        let mut owned = BTreeSet::new();
        let mut checked: BTreeMap<String, Scene> = BTreeMap::new();
        for (root, link) in &self.prefab_links {
            validate_source(&link.source)?;
            if link.guids.is_empty() || link.guids.len() > MAX_GENERIC_ENTITIES {
                return Err("generic prefab requires 1 to 256 entities".into());
            }
            if !link.ordinals.is_empty() || !link.position_overrides.is_empty() {
                return Err("generic prefab links have no ordinals or position overrides".into());
            }
            if !link.guids.values().any(|g| g == root) {
                return Err("prefab root must be one of its mapped live targets".into());
            }
            if link.digest != PrefabLink::canonical_digest(&link.baseline) {
                return Err("prefab baseline SHA-256 digest mismatch".into());
            }
            // Equal baselines are checked once: many instances share one source.
            if !checked.contains_key(&link.digest) {
                let node = super::node::parse(&link.baseline)
                    .map_err(|e| format!("invalid prefab baseline: {}", e.message))?;
                let mut baseline = decode::decode_scene(&node, reg)
                    .map_err(|e| format!("invalid prefab baseline: {}", e[0].message))?;
                baseline.read_comments(&link.baseline, &node);
                if !baseline.singletons.is_empty()
                    || baseline.entities.is_empty()
                    || baseline.entities.len() > MAX_GENERIC_ENTITIES
                    || !baseline.header_comments.is_empty()
                    || !baseline.comments.is_empty()
                {
                    return Err("prefab baseline must be a comment-free entity-only fragment of 1 to 256 entities".into());
                }
                if baseline.to_yaml() != link.baseline {
                    return Err("prefab baseline is not canonical scene/1 text".into());
                }
                checked.insert(link.digest.clone(), baseline);
            }
            if !checked[&link.digest].entities.keys().eq(link.guids.keys()) {
                return Err("prefab mapping must cover exactly the baseline entities".into());
            }
            for target in link.guids.values() {
                if !owned.insert(target.clone()) {
                    return Err("prefab live targets must be injective and disjoint across links".into());
                }
            }
        }
        Ok(())
    }
}
