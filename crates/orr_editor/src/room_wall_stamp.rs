use orr_fp::{fp, FP};
use serde::Deserialize;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeSet;

const ACTOR: &str = "RoomEscapeV1::Actor";
const BODY: &str = "orr_physics3d::Body";
const COLLIDER: &str = "orr_physics3d::Collider";
const MAX_WALLS: usize = 64;
const MAX_BYTES: usize = 64 * 1024;
const MAX_NAME_BYTES: usize = 64;
const MAX_CENTER: FP = fp!(16);
const MIN_EXTENT: FP = fp!(0.1);
const MAX_EXTENT: FP = fp!(8);
const PLAYER_Y: FP = FP::HALF;
const PLAYER_RADIUS: FP = fp!(0.4);
const SKIN: FP = FP(64);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Wall {
    pub name: String,
    pub components: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Template {
    schema: u32,
    walls: Vec<Wall>,
}

#[cfg(test)]
mod tests;

impl Template {
    pub(crate) fn capture(rows: Vec<(String, Value)>) -> Result<Self, String> {
        if rows.is_empty() || rows.len() > MAX_WALLS {
            return Err("wall stamp requires 1..=64 walls".into());
        }
        let mut walls = Vec::with_capacity(rows.len());
        for (name, components) in rows {
            validate_name(&name)?;
            validate_components(&components)?;
            walls.push(Wall { name, components });
        }
        let template = Self { schema: 1, walls };
        template.encode()?;
        Ok(template)
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let bytes =
            serde_json::to_vec(self).map_err(|error| format!("wall stamp encode: {error}"))?;
        if bytes.len() > MAX_BYTES {
            return Err("wall stamp exceeds 64 KiB".into());
        }
        Ok(bytes)
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.is_empty() || bytes.len() > MAX_BYTES {
            return Err("wall stamp must be 1..=64 KiB".into());
        }
        scan_unique_json(bytes)?;
        let value: Value =
            serde_json::from_slice(bytes).map_err(|error| format!("wall stamp JSON: {error}"))?;
        let Value::Object(object) = value else {
            return Err("wall stamp must be a JSON object".into());
        };
        if object.keys().map(String::as_str).collect::<BTreeSet<_>>()
            != BTreeSet::from(["schema", "walls"])
        {
            return Err("wall stamp requires exactly schema and walls".into());
        }
        let schema = object
            .get("schema")
            .and_then(Value::as_u64)
            .ok_or("wall stamp schema is invalid")?;
        if schema != 1 {
            return Err("unsupported wall stamp schema".into());
        }
        let template: Self = serde_json::from_value(Value::Object(object))
            .map_err(|error| format!("wall stamp fields: {error}"))?;
        template.validate()?;
        Ok(template)
    }

    pub(crate) fn walls(&self) -> &[Wall] {
        &self.walls
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema != 1 {
            return Err("unsupported wall stamp schema".into());
        }
        if self.walls.is_empty() || self.walls.len() > MAX_WALLS {
            return Err("wall stamp requires 1..=64 walls".into());
        }
        for wall in &self.walls {
            validate_name(&wall.name)?;
            validate_components(&wall.components)?;
        }
        Ok(())
    }
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.len() > MAX_NAME_BYTES || name.chars().any(char::is_control) {
        return Err("wall stamp names must contain at most 64 bytes without controls".into());
    }
    Ok(())
}

fn validate_components(components: &Value) -> Result<(), String> {
    let map = components
        .as_object()
        .ok_or("wall components must be an object")?;
    let keys = map.keys().map(String::as_str).collect::<BTreeSet<_>>();
    if keys != BTreeSet::from([ACTOR, BODY, COLLIDER]) {
        return Err("wall stamp components must contain exactly Actor, Body and Collider".into());
    }
    let actor = map[ACTOR]
        .as_object()
        .ok_or("wall Actor must be an object")?;
    require_keys(actor, &["kind", "ordinal"], "wall Actor")?;
    if actor.get("kind").and_then(Value::as_u64) != Some(4)
        || actor.get("ordinal").and_then(Value::as_u64) != Some(0)
    {
        return Err("wall Actor must be the canonical WALL role".into());
    }

    let body = map[BODY].as_object().ok_or("wall Body must be an object")?;
    require_keys(
        body,
        &[
            "pos",
            "rot",
            "vel",
            "omega",
            "inv_mass",
            "inv_inertia",
            "linear_damping",
            "angular_damping",
            "kind",
        ],
        "wall Body",
    )?;
    let position = vector3(body.get("pos").ok_or("wall Body.pos is missing")?)?;
    if position
        .iter()
        .any(|value| *value < -MAX_CENTER || *value > MAX_CENTER)
        || position[1] < fp!(-4)
        || position[1] > fp!(4)
    {
        return Err("wall center exceeds the Room bounds".into());
    }
    let rotation = body
        .get("rot")
        .and_then(Value::as_object)
        .ok_or("wall Body.rot must be an object")?;
    require_keys(rotation, &["kind", "x", "y", "z", "w"], "wall Body.rot")?;
    if rotation.get("kind").and_then(Value::as_str) != Some("unit")
        || fixed(rotation.get("x").ok_or("wall rotation x is missing")?)? != FP::ZERO
        || fixed(rotation.get("y").ok_or("wall rotation y is missing")?)? != FP::ZERO
        || fixed(rotation.get("z").ok_or("wall rotation z is missing")?)? != FP::ZERO
        || fixed(rotation.get("w").ok_or("wall rotation w is missing")?)? != FP::ONE
        || vector3(body.get("vel").ok_or("wall Body.vel is missing")?)? != [FP::ZERO; 3]
        || vector3(body.get("omega").ok_or("wall Body.omega is missing")?)? != [FP::ZERO; 3]
        || fixed(
            body.get("inv_mass")
                .ok_or("wall Body.inv_mass is missing")?,
        )? != FP::ZERO
        || vector3(
            body.get("inv_inertia")
                .ok_or("wall Body.inv_inertia is missing")?,
        )? != [FP::ZERO; 3]
        || fixed(
            body.get("linear_damping")
                .ok_or("wall Body.linear_damping is missing")?,
        )? != FP::ZERO
        || fixed(
            body.get("angular_damping")
                .ok_or("wall Body.angular_damping is missing")?,
        )? != FP::ZERO
        || body.get("kind").and_then(Value::as_str) != Some("static")
    {
        return Err("wall Body must contain canonical static fields".into());
    }

    let collider = map[COLLIDER]
        .as_object()
        .ok_or("wall Collider must be an object")?;
    require_keys(
        collider,
        &["shape", "restitution", "friction", "layer", "mask"],
        "wall Collider",
    )?;
    let shape = collider
        .get("shape")
        .and_then(Value::as_object)
        .ok_or("wall Collider.shape must be an object")?;
    require_keys(shape, &["kind", "half_extents"], "wall Collider.shape")?;
    if shape.get("kind").and_then(Value::as_str) != Some("box")
        || fixed(
            collider
                .get("restitution")
                .ok_or("wall restitution is missing")?,
        )? != FP::ZERO
        || fixed(collider.get("friction").ok_or("wall friction is missing")?)? != fp!(0.5)
        || collider.get("layer").and_then(Value::as_u64) != Some(1)
        || collider.get("mask").and_then(Value::as_u64) != Some(0)
    {
        return Err("wall Collider must contain canonical Room wall settings".into());
    }
    let half = vector3(
        shape
            .get("half_extents")
            .ok_or("wall box half_extents are missing")?,
    )?;
    if half
        .iter()
        .any(|value| *value < MIN_EXTENT || *value > MAX_EXTENT)
    {
        return Err("wall half extents must be in 0.1..=8".into());
    }
    if position[1] - half[1] > PLAYER_Y - PLAYER_RADIUS - SKIN
        || position[1] + half[1] < PLAYER_Y + PLAYER_RADIUS + SKIN
    {
        return Err("wall must span the full player height plus skin".into());
    }
    Ok(())
}

fn require_keys(map: &Map<String, Value>, expected: &[&str], label: &str) -> Result<(), String> {
    if map.keys().map(String::as_str).collect::<BTreeSet<_>>() != expected.iter().copied().collect()
    {
        return Err(format!("{label} has missing or unsupported fields"));
    }
    Ok(())
}

fn fixed(value: &Value) -> Result<FP, String> {
    let number = value
        .as_number()
        .ok_or("expected exact fixed-point number")?;
    orr_reflect::decimal::parse_fp(&number.to_string())
        .map_err(|error| format!("fixed-point value: {error:?}"))
}

fn vector3(value: &Value) -> Result<[FP; 3], String> {
    let values = value
        .as_array()
        .ok_or("expected three-component fixed-point vector")?;
    if values.len() != 3 {
        return Err("expected three-component fixed-point vector".into());
    }
    Ok([fixed(&values[0])?, fixed(&values[1])?, fixed(&values[2])?])
}

fn scan_unique_json(bytes: &[u8]) -> Result<(), String> {
    let mut offset = 0;
    scan_value(bytes, &mut offset, 0)?;
    skip_whitespace(bytes, &mut offset);
    if offset != bytes.len() {
        return Err("wall stamp JSON has trailing data".into());
    }
    Ok(())
}

fn scan_value(bytes: &[u8], offset: &mut usize, depth: usize) -> Result<(), String> {
    if depth > 128 {
        return Err("wall stamp JSON nesting exceeds 128 levels".into());
    }
    skip_whitespace(bytes, offset);
    match bytes.get(*offset).copied() {
        Some(b'{') => scan_object(bytes, offset, depth + 1),
        Some(b'[') => scan_array(bytes, offset, depth + 1),
        Some(b'"') => scan_string(bytes, offset).map(|_| ()),
        Some(_) => {
            let start = *offset;
            while let Some(byte) = bytes.get(*offset) {
                if matches!(*byte, b',' | b']' | b'}' | b' ' | b'\t' | b'\r' | b'\n') {
                    break;
                }
                *offset += 1;
            }
            if *offset == start {
                return Err("wall stamp JSON has an invalid value".into());
            }
            Ok(())
        }
        None => Err("wall stamp JSON is truncated".into()),
    }
}

fn scan_object(bytes: &[u8], offset: &mut usize, depth: usize) -> Result<(), String> {
    *offset += 1;
    skip_whitespace(bytes, offset);
    let mut keys = BTreeSet::new();
    if bytes.get(*offset) == Some(&b'}') {
        *offset += 1;
        return Ok(());
    }
    loop {
        skip_whitespace(bytes, offset);
        if bytes.get(*offset) != Some(&b'"') {
            return Err("wall stamp JSON object key is invalid".into());
        }
        let key = scan_string(bytes, offset)?;
        if !keys.insert(key) {
            return Err("wall stamp JSON contains a duplicate object key".into());
        }
        skip_whitespace(bytes, offset);
        if bytes.get(*offset) != Some(&b':') {
            return Err("wall stamp JSON object is missing a colon".into());
        }
        *offset += 1;
        scan_value(bytes, offset, depth)?;
        skip_whitespace(bytes, offset);
        match bytes.get(*offset) {
            Some(b',') => *offset += 1,
            Some(b'}') => {
                *offset += 1;
                return Ok(());
            }
            _ => return Err("wall stamp JSON object separator is invalid".into()),
        }
    }
}

fn scan_array(bytes: &[u8], offset: &mut usize, depth: usize) -> Result<(), String> {
    *offset += 1;
    skip_whitespace(bytes, offset);
    if bytes.get(*offset) == Some(&b']') {
        *offset += 1;
        return Ok(());
    }
    loop {
        scan_value(bytes, offset, depth)?;
        skip_whitespace(bytes, offset);
        match bytes.get(*offset) {
            Some(b',') => *offset += 1,
            Some(b']') => {
                *offset += 1;
                return Ok(());
            }
            _ => return Err("wall stamp JSON array separator is invalid".into()),
        }
    }
}

fn scan_string(bytes: &[u8], offset: &mut usize) -> Result<String, String> {
    let start = *offset;
    *offset += 1;
    let mut escaped = false;
    while let Some(byte) = bytes.get(*offset).copied() {
        *offset += 1;
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == b'"' {
            return serde_json::from_slice(&bytes[start..*offset])
                .map_err(|error| format!("wall stamp JSON string: {error}"));
        }
    }
    Err("wall stamp JSON string is truncated".into())
}

fn skip_whitespace(bytes: &[u8], offset: &mut usize) {
    while bytes
        .get(*offset)
        .is_some_and(|byte| matches!(*byte, b' ' | b'\t' | b'\r' | b'\n'))
    {
        *offset += 1;
    }
}
