//! JSON Schema (draft 2020-12) from the type registry.
//!
//! Layout of a scene schema: the root describes the file, `$defs` holds the
//! shared number and entity-reference types, one entry per registered type
//! (under its registry name), and the shape of one entity.

use super::json::Json;
use super::SCENE_SCHEMA;
use crate::decimal;
use crate::desc::{Kind, Range, TypeDesc};
use crate::registry::{TypeKind, TypeRegistry};
use orr_fp::FP;

const DRAFT: &str = "https://json-schema.org/draft/2020-12/schema";
const GUID_PATTERN: &str = "^e_[0-9a-f]{8,32}$";

const FIXED_NOTE: &str = "Fixed-point number, Q48.16 (1/65536 steps). Write a plain decimal such as 12, -0.5 or 3.25: no exponent, no '+', no quotes. The loader reads it with integer arithmetic (never as a float) and rounds to the nearest 1/65536, so 0.1 is stored as 0.1000061. Files are written with the shortest decimal that reads back to exactly the same value.";
const FIXED32_NOTE: &str = "Fixed-point number, Q16.16 (1/65536 steps, range -32768 to 32767.99998). Same text rules as the 64-bit fixed-point number: a plain decimal, rounded to the nearest 1/65536.";

fn fixed_bound(raw: i128) -> Json {
    Json::Num(decimal::fp_to_decimal(FP::from_raw(raw as i64)))
}

fn fixed_schema(range: &Range, reference: &str, note: &str) -> Json {
    if range.is_open() {
        return Json::obj(vec![("$ref", Json::str(reference))]);
    }
    let mut j = Json::obj(vec![("type", Json::str("number")), ("description", Json::str(note))]);
    if let Some(m) = range.min {
        j = j.with("minimum", fixed_bound(m));
    }
    if let Some(m) = range.max {
        j = j.with("maximum", fixed_bound(m));
    }
    j
}

fn with_doc(j: Json, doc: &str) -> Json {
    if doc.is_empty() {
        j
    } else {
        j.with("description", Json::str(doc))
    }
}

fn desc_schema(desc: &TypeDesc) -> Json {
    let j = match &desc.kind {
        Kind::Bool { .. } => Json::obj(vec![("type", Json::str("boolean"))]),
        Kind::Int { int, range } => {
            let lo = range.min.map_or(int.min(), |m| m.max(int.min()));
            let hi = range.max.map_or(int.max(), |m| m.min(int.max()));
            Json::obj(vec![("type", Json::str("integer")), ("minimum", Json::num(lo)), ("maximum", Json::num(hi))])
        }
        Kind::Fixed { range } => fixed_schema(range, "#/$defs/orr.Fixed", FIXED_NOTE),
        Kind::Fixed32 { range } => fixed_schema(range, "#/$defs/orr.Fixed32", FIXED32_NOTE),
        Kind::Vec2 { range } => vec_schema(range, 2),
        Kind::Vec3 { range } => vec_schema(range, 3),
        Kind::Entity => Json::obj(vec![("$ref", Json::str("#/$defs/orr.EntityRef"))]),
        Kind::Enum { variants, .. } => {
            Json::obj(vec![("enum", Json::Arr(variants.iter().map(|(n, _)| Json::str(n)).collect()))])
        }
        Kind::Flags { bits, .. } => Json::obj(vec![
            ("type", Json::str("array")),
            ("items", Json::obj(vec![("enum", Json::Arr(bits.iter().map(|(n, _)| Json::str(n)).collect()))])),
            ("uniqueItems", Json::Bool(true)),
        ]),
        Kind::Array { elem, len } => Json::obj(vec![
            ("type", Json::str("array")),
            ("items", desc_schema(elem)),
            ("minItems", Json::num(len)),
            ("maxItems", Json::num(len)),
        ]),
        Kind::List { elem, min, max } => Json::obj(vec![
            ("type", Json::str("array")),
            ("items", desc_schema(elem)),
            ("minItems", Json::num(min)),
            ("maxItems", Json::num(max)),
        ]),
        Kind::Struct { fields, .. } => {
            let props = fields.iter().map(|f| (f.name.clone(), with_doc(desc_schema(&f.ty), &f.doc))).collect();
            Json::obj(vec![
                ("type", Json::str("object")),
                ("properties", Json::Obj(props)),
                ("required", Json::Arr(fields.iter().map(|f| Json::str(&f.name)).collect())),
                ("additionalProperties", Json::Bool(false)),
            ])
        }
        Kind::Tagged(t) => {
            let variants = t
                .variants
                .iter()
                .map(|v| {
                    let mut props = vec![("kind".to_string(), Json::obj(vec![("const", Json::str(&v.name))]))];
                    props.extend(v.fields.iter().map(|f| (f.name.clone(), with_doc(desc_schema(&f.ty), &f.doc))));
                    let mut required = vec![Json::str("kind")];
                    required.extend(v.fields.iter().map(|f| Json::str(&f.name)));
                    with_doc(
                        Json::obj(vec![
                            ("type", Json::str("object")),
                            ("properties", Json::Obj(props)),
                            ("required", Json::Arr(required)),
                            ("additionalProperties", Json::Bool(false)),
                        ]),
                        &v.doc,
                    )
                })
                .collect();
            Json::obj(vec![("oneOf", Json::Arr(variants))])
        }
    };
    j
}

fn vec_schema(range: &Range, n: usize) -> Json {
    let item = fixed_schema(range, "#/$defs/orr.Fixed", FIXED_NOTE);
    Json::obj(vec![
        ("type", Json::str("array")),
        ("prefixItems", Json::Arr((0..n).map(|_| item.clone()).collect())),
        ("items", Json::Bool(false)),
        ("minItems", Json::num(n)),
        ("maxItems", Json::num(n)),
    ])
}

fn shared_defs() -> Vec<(String, Json)> {
    vec![
        (
            "orr.Fixed".to_string(),
            Json::obj(vec![("type", Json::str("number")), ("description", Json::str(FIXED_NOTE))]),
        ),
        (
            "orr.Fixed32".to_string(),
            Json::obj(vec![
                ("type", Json::str("number")),
                ("minimum", Json::num(-32768)),
                ("maximum", Json::Num("32767.99998".to_string())),
                ("description", Json::str(FIXED32_NOTE)),
            ]),
        ),
        (
            "orr.EntityRef".to_string(),
            Json::obj(vec![
                ("type", Json::Arr(vec![Json::str("string"), Json::str("null")])),
                ("pattern", Json::str(GUID_PATTERN)),
                ("description", Json::str("GUID of another entity of the scene (e_ and 8 to 32 lowercase hex digits), or null for no entity.")),
            ]),
        ),
    ]
}

pub(super) fn scene_schema(reg: &TypeRegistry) -> Json {
    let mut defs = shared_defs();
    for t in reg.types() {
        defs.push((t.name().to_string(), with_doc(desc_schema(t.desc()), t.doc())));
    }
    let mut entity_props: Vec<(String, Json)> = vec![(
        "name".to_string(),
        Json::obj(vec![("type", Json::str("string")), ("minLength", Json::num(1)), ("description", Json::str("Display name (editor only)."))]),
    )];
    for t in reg.components() {
        entity_props.push((t.name().to_string(), Json::obj(vec![("$ref", Json::Str(format!("#/$defs/{}", t.name())))])));
    }
    defs.push((
        "orr.SceneEntity".to_string(),
        Json::obj(vec![
            ("type", Json::str("object")),
            ("properties", Json::Obj(entity_props)),
            ("additionalProperties", Json::Bool(false)),
        ]),
    ));

    let mut props: Vec<(String, Json)> = vec![("schema".to_string(), Json::obj(vec![("const", Json::str(SCENE_SCHEMA))]))];
    let singles: Vec<(String, Json)> = reg
        .types()
        .filter(|t| t.kind() == TypeKind::Singleton)
        .map(|t| (t.name().to_string(), Json::obj(vec![("$ref", Json::Str(format!("#/$defs/{}", t.name())))])))
        .collect();
    if !singles.is_empty() {
        props.push((
            "singletons".to_string(),
            Json::obj(vec![
                ("type", Json::str("object")),
                ("description", Json::str("One value per singleton type of the frame.")),
                ("properties", Json::Obj(singles)),
                ("additionalProperties", Json::Bool(false)),
            ]),
        ));
    }
    props.push((
        "entities".to_string(),
        Json::obj(vec![
            ("type", Json::str("object")),
            ("description", Json::str("Entities by stable GUID. Keys are sorted on save.")),
            ("propertyNames", Json::obj(vec![("pattern", Json::str(GUID_PATTERN))])),
            ("additionalProperties", Json::obj(vec![("$ref", Json::str("#/$defs/orr.SceneEntity"))])),
        ]),
    ));

    Json::obj(vec![
        ("$schema", Json::str(DRAFT)),
        ("title", Json::Str(format!("Orrery scene ({SCENE_SCHEMA})"))),
        ("description", Json::str("Strict YAML scene file. No anchors, aliases or tags. Types come from this schema, not from the YAML text.")),
        ("type", Json::str("object")),
        ("properties", Json::Obj(props)),
        ("required", Json::Arr(vec![Json::str("schema"), Json::str("entities")])),
        ("additionalProperties", Json::Bool(false)),
        ("$defs", Json::Obj(defs)),
    ])
}

/// Optional closed metadata shape. Cross-document facts remain loader checks.
#[cfg(feature = "linked-prefabs")]
pub(super) fn linked_scene_schema(reg: &TypeRegistry) -> Json {
    let mut root = scene_schema(reg);
    let guid = || Json::obj(vec![("type", Json::str("string")), ("pattern", Json::str(GUID_PATTERN))]);
    let guid_map = |values: Json| Json::obj(vec![
        ("type", Json::str("object")),
        ("minProperties", Json::num(1)),
        ("maxProperties", Json::num(8)),
        ("propertyNames", guid()),
        ("additionalProperties", values),
    ]);
    let link = Json::obj(vec![
        ("type", Json::str("object")),
        ("properties", Json::obj(vec![
            ("profile", Json::obj(vec![("const", Json::str("collect-actors-v1"))])),
            ("source", Json::obj(vec![
                ("type", Json::str("string")),
                ("minLength", Json::num(1)),
                ("maxLength", Json::num(240)),
                ("pattern", Json::str("^[A-Za-z0-9_.-]+(/[A-Za-z0-9_.-]+)*$")),
            ])),
            ("digest", Json::obj(vec![("type", Json::str("string")), ("pattern", Json::str("^[0-9a-f]{64}$"))])),
            ("baseline", Json::obj(vec![("type", Json::str("string")), ("maxLength", Json::num(32768))])),
            ("guids", guid_map(guid())),
            ("ordinals", guid_map(Json::obj(vec![
                ("type", Json::str("integer")),
                ("minimum", Json::num(0)),
                ("maximum", Json::num(u32::MAX)),
            ]))),
            ("position_overrides", Json::obj(vec![
                ("type", Json::str("array")),
                ("items", guid()),
                ("maxItems", Json::num(8)),
                ("uniqueItems", Json::Bool(true)),
            ])),
        ])),
        ("required", Json::Arr(["profile", "source", "digest", "baseline", "guids", "ordinals", "position_overrides"].iter().map(|s| Json::str(s)).collect())),
        ("additionalProperties", Json::Bool(false)),
    ]);
    if let Json::Obj(fields) = &mut root {
        if let Some((_, Json::Obj(props))) = fields.iter_mut().find(|(key, _)| key == "properties") {
            if let Some((_, version)) = props.iter_mut().find(|(key, _)| key == "schema") {
                *version = Json::obj(vec![("enum", Json::Arr(vec![Json::str(SCENE_SCHEMA), Json::str("orr.scene/2")]))]);
            }
            props.push(("prefabs".into(), guid_map(link)));
        }
    }
    root.with("title", Json::str("Orrery scene (orr.scene/1 or orr.scene/2)"))
        .with("description", Json::str("Strict YAML scene file. No anchors, aliases or tags. Schema 2 is the optional collect-actors-v1 linked-prefab format. The loader additionally enforces the 64 KiB scene and aggregate 32 KiB baseline byte limits, quoted canonical comment-free entity-only scene/1 baselines, SHA-256 agreement, exact Actor profile, complete injective and disjoint GUID mappings, ordinal and override key relationships, root ownership, and flattened-value agreement. Source labels are inert: no filesystem access occurs; normalization and reserved device-name restrictions are checked by the loader. These semantic checks are not expressible by this JSON Schema."))
        .with("if", Json::obj(vec![("properties", Json::obj(vec![("schema", Json::obj(vec![("const", Json::str(SCENE_SCHEMA))]))]))]))
        .with("then", Json::obj(vec![("properties", Json::obj(vec![("prefabs", Json::Bool(false))]))]))
        .with("else", Json::obj(vec![("required", Json::Arr(vec![Json::str("prefabs")]))]))
}

pub(super) fn type_schema(reg: &TypeRegistry, name: &str) -> Option<Json> {
    let t = reg.get(name)?;
    let body = with_doc(desc_schema(t.desc()), t.doc());
    let mut fields = vec![
        ("$schema".to_string(), Json::str(DRAFT)),
        ("title".to_string(), Json::str(name)),
    ];
    if let Json::Obj(inner) = body {
        fields.extend(inner);
    }
    fields.push(("$defs".to_string(), Json::Obj(shared_defs())));
    Some(Json::Obj(fields))
}
