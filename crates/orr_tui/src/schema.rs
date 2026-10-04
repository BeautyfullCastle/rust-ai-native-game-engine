//! The schema message (JSON, `docs/view-stream.md`), read as plain JSON: this viewer learns
//! what a game is (kinds, input layout, event names) from text, never from a Rust type.

use serde_json::Value as J;

/// One entity kind of the game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KindInfo {
    pub id: u16,
    pub name: String,
    /// Number of 32-bit property words, and their `(name, type)`.
    pub props: Vec<(String, String)>,
}

/// One field of the input layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputField {
    pub name: String,
    pub offset: usize,
    pub size: usize,
    /// `fixed`, `fixed32`, `i32`, `u32`, `flags`, `bool`, ...
    pub ty: String,
    /// For `flags`: `(name, mask)`.
    pub bits: Vec<(String, u64)>,
}

/// What the viewer needs of the schema.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewSchema {
    pub game: String,
    /// Spatial dimension of the frame records: 2 for schema v1, 3 for schema v2.
    pub dimensions: u8,
    pub tick_rate: u32,
    pub player_count: u8,
    pub kinds: Vec<KindInfo>,
    pub input_size: usize,
    pub input_fields: Vec<InputField>,
    /// `(id, name)`.
    pub events: Vec<(u16, String)>,
}

fn uint(v: &J, key: &str) -> Option<u64> {
    v.get(key).and_then(J::as_u64)
}

impl ViewSchema {
    /// Reads the schema text. Checks the format name and that the version is one we know.
    pub fn parse(text: &str) -> Result<ViewSchema, String> {
        let j: J = serde_json::from_str(text).map_err(|e| format!("the schema is not JSON: {e}"))?;
        if j.get("format").and_then(J::as_str) != Some("orrery.viewstream") {
            return Err("not an orrery.viewstream schema".into());
        }
        let version = uint(&j, "version").unwrap_or(0);
        let dimensions = match version {
            v if v == u64::from(orr_viewstream::VERSION) => {
                if j.get("frame3d").is_some() {
                    return Err("view stream version 1 cannot contain frame3d".into());
                }
                if j.get("dimensions").is_some_and(|d| d.as_u64() != Some(2)) {
                    return Err("view stream version 1 requires dimensions 2".into());
                }
                2
            }
            v if v == u64::from(orr_viewstream::VERSION_3D) => {
                let frame3d = j.get("frame3d");
                if !frame3d.is_some_and(J::is_object) || j.get("frame").is_some() {
                    return Err("view stream version 2 requires frame3d and cannot contain frame".into());
                }
                let frame3d = frame3d.expect("validated frame3d object");
                if uint(frame3d, "message_type") != Some(u64::from(orr_viewstream::MSG_FRAME3D))
                    || uint(frame3d, "record_len") != Some(u64::from(orr_viewstream::RECORD3D_LEN as u32))
                {
                    return Err("view stream version 2 requires frame3d message_type 3 and record_len 88".into());
                }
                if j.get("dimensions").is_some_and(|d| d.as_u64() != Some(3)) {
                    return Err("view stream version 2 requires dimensions 3".into());
                }
                3
            }
            _ => return Err(format!("unsupported view stream version {version}")),
        };
        let kinds = j["kinds"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|k| {
                        let props = k["props"]
                            .as_array()
                            .map(|p| {
                                p.iter()
                                    .map(|p| (p["name"].as_str().unwrap_or("").to_string(), p["type"].as_str().unwrap_or("u32").to_string()))
                                    .collect()
                            })
                            .unwrap_or_default();
                        Some(KindInfo { id: uint(k, "id")? as u16, name: k["name"].as_str()?.to_string(), props })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let input = &j["input"];
        let input_fields = input["fields"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|f| {
                        let bits = f["bits"]
                            .as_object()
                            .map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_u64()?))).collect())
                            .unwrap_or_default();
                        Some(InputField {
                            name: f["name"].as_str()?.to_string(),
                            offset: uint(f, "offset")? as usize,
                            size: uint(f, "size")? as usize,
                            ty: f["type"].as_str().unwrap_or("opaque").to_string(),
                            bits,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let events = j["events"]
            .as_array()
            .map(|a| a.iter().filter_map(|e| Some((uint(e, "id")? as u16, e["name"].as_str()?.to_string()))).collect())
            .unwrap_or_default();
        Ok(ViewSchema {
            game: j["game"].as_str().unwrap_or("?").to_string(),
            dimensions,
            tick_rate: uint(&j, "tick_rate").unwrap_or(60).clamp(1, 1000) as u32,
            player_count: uint(&j, "player_count").unwrap_or(1).min(255) as u8,
            kinds,
            input_size: uint(input, "size").unwrap_or(0) as usize,
            input_fields,
            events,
        })
    }

    /// True when this schema carries 3D frame records.
    pub fn is_3d(&self) -> bool {
        self.dimensions == 3
    }

    /// Name of a kind id (`"?"` if unknown).
    pub fn kind_name(&self, id: u16) -> &str {
        self.kinds.iter().find(|k| k.id == id).map_or("?", |k| k.name.as_str())
    }

    /// Number of property words of a kind.
    pub fn props_words(&self, id: u16) -> usize {
        self.kinds.iter().find(|k| k.id == id).map_or(0, |k| k.props.len())
    }

    /// Index of the property `name` among the words of `kind`.
    pub fn prop_index(&self, kind: u16, name: &str) -> Option<usize> {
        self.kinds.iter().find(|k| k.id == kind)?.props.iter().position(|(n, _)| n == name)
    }

    /// Name of an event type id.
    pub fn event_name(&self, id: u16) -> String {
        self.events.iter().find(|(i, _)| *i == id).map_or_else(|| format!("event#{id}"), |(_, n)| n.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::ViewSchema;

    const V1: &str = r#"{"format":"orrery.viewstream","version":1,"game":"other"}"#;
    const V2: &str = r#"{"format":"orrery.viewstream","version":2,"game":"Yard3D","frame3d":{"message_type":3,"record_len":88}}"#;

    #[test]
    fn schema_versions_select_their_frame_dimensions() {
        assert_eq!(ViewSchema::parse(V1).unwrap().dimensions, 2);
        assert_eq!(ViewSchema::parse(&V1.replace("\"game\":\"other\"", "\"game\":\"other\",\"dimensions\":2")).unwrap().dimensions, 2);
        assert_eq!(ViewSchema::parse(V2).unwrap().dimensions, 3);
        assert_eq!(ViewSchema::parse(&V2.replace("\"game\":\"Yard3D\"", "\"game\":\"Yard3D\",\"dimensions\":3")).unwrap().dimensions, 3);
    }

    #[test]
    fn schema_version_and_frame_shape_must_agree() {
        for bad in [
            V1.replace("\"game\":\"other\"", "\"game\":\"other\",\"dimensions\":3"),
            V1.replace("\"game\":\"other\"", "\"game\":\"other\",\"frame3d\":{}"),
            V2.replace("\"frame3d\":{\"message_type\":3,\"record_len\":88}", "\"frame\":{}"),
            V2.replace("\"game\":\"Yard3D\"", "\"game\":\"Yard3D\",\"dimensions\":2"),
            V2.replace("\"frame3d\":{\"message_type\":3,\"record_len\":88}", "\"frame3d\":null"),
            V2.replace("\"message_type\":3", "\"message_type\":2"),
            V2.replace("\"record_len\":88", "\"record_len\":80"),
            V1.replace("\"version\":1", "\"version\":3"),
        ] {
            assert!(ViewSchema::parse(&bad).is_err(), "accepted {bad}");
        }
    }
}
