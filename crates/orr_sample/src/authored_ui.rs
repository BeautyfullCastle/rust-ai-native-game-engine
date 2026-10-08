//! Bounded, presentation-only Collect UI v1. Text is literal; bindings and
//! actions are closed enums, never expressions, paths, or executable callbacks.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_BYTES: usize = 64 * 1024;
pub const MAX_NODES: usize = 32;
pub const MAX_DEPTH: usize = 4;
pub const SCHEMA: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub schema: u32,
    /// Draw order; a parent must appear before all its children.
    pub nodes: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: String,
    pub parent: Option<String>,
    pub kind: Kind,
    pub screen: Screen,
    /// Thousandths of the parent's extent (the viewport for a root).
    pub anchor: [u16; 2],
    /// Logical-pixel displacement from the anchored top-left position.
    pub offset: [i16; 2],
    pub size: [u16; 2],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Kind {
    Label {
        text: String,
        binding: Option<Binding>,
    },
    Button {
        text: String,
        action: Action,
    },
    Container,
}

// An empty struct variant deliberately enforces unknown-field rejection for
// containers too. Serde's internally tagged unit variant may ignore content.
impl<'de> Deserialize<'de> for Kind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
        enum Wire {
            Label {
                text: String,
                binding: Option<Binding>,
            },
            Button {
                text: String,
                action: Action,
            },
            Container {},
        }
        Ok(match Wire::deserialize(deserializer)? {
            Wire::Label { text, binding } => Self::Label { text, binding },
            Wire::Button { text, action } => Self::Button { text, action },
            Wire::Container {} => Self::Container,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Binding {
    Score,
    Phase,
    Best,
    KeyAcquired,
    ExitState,
    RoomPhase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Play,
    Menu,
    Continue,
    Restart,
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Screen {
    Title,
    Playing,
    Menu,
    Terminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Collect,
    Room,
}

// Room's new wire profile is object-only. Deserializing through MapAccess
// preserves the typed duplicate/unknown-field checks; no lossy Value round trip.
// Keep the historical Collect decoder unchanged.
struct MapOnly<T>(T);
impl<'de, T: Deserialize<'de>> Deserialize<'de> for MapOnly<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Visitor<T> {
            type Value = MapOnly<T>;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a Room UI JSON object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                map: A,
            ) -> Result<Self::Value, A::Error> {
                T::deserialize(serde::de::value::MapAccessDeserializer::new(map)).map(MapOnly)
            }
        }
        deserializer.deserialize_map(Visitor(std::marker::PhantomData))
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RoomDocumentWire {
    schema: u32,
    nodes: Vec<MapOnly<RoomNodeWire>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RoomNodeWire {
    id: String,
    parent: Option<String>,
    kind: MapOnly<Kind>,
    screen: Screen,
    anchor: [u16; 2],
    offset: [i16; 2],
    size: [u16; 2],
}
fn parse_room_object(bytes: &[u8]) -> Result<Document, serde_json::Error> {
    let MapOnly(wire): MapOnly<RoomDocumentWire> = serde_json::from_slice(bytes)?;
    Ok(Document {
        schema: wire.schema,
        nodes: wire
            .nodes
            .into_iter()
            .map(|MapOnly(node)| Node {
                id: node.id,
                parent: node.parent,
                kind: node.kind.0,
                screen: node.screen,
                anchor: node.anchor,
                offset: node.offset,
                size: node.size,
            })
            .collect(),
    })
}

impl Document {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        Self::parse_for(bytes, Profile::Collect)
    }

    pub fn parse_for(bytes: &[u8], profile: Profile) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err("Collect UI exceeds 64 KiB".into());
        }
        // Decode straight into strict structs, not Value: Value would silently
        // discard duplicate JSON keys before schema validation could see them.
        let document: Self = match profile {
            Profile::Collect => serde_json::from_slice(bytes),
            Profile::Room => parse_room_object(bytes),
        }
        .map_err(|error| format!("invalid authored UI JSON: {error}"))?;
        document.validate_for(profile)?;
        Ok(document)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.validate_for(Profile::Collect)
    }

    pub fn validate_for(&self, profile: Profile) -> Result<(), String> {
        if self.schema != SCHEMA {
            return Err(format!("unsupported Collect UI schema {}", self.schema));
        }
        if self.nodes.len() > MAX_NODES {
            return Err("Collect UI exceeds 32 nodes".into());
        }
        let mut previous: BTreeMap<&str, (&Node, usize)> = BTreeMap::new();
        for node in &self.nodes {
            if !valid_id(&node.id) {
                return Err(
                    "UI id must be 1..32 ASCII letters, digits, underscores or hyphens".into(),
                );
            }
            if previous.contains_key(node.id.as_str()) {
                return Err(format!("duplicate UI id {}", node.id));
            }
            if node.anchor.iter().any(|v| *v > 1000)
                || node.offset.iter().any(|v| !(-4096..=4096).contains(v))
                || node.size.iter().any(|v| !(1..=4096).contains(v))
            {
                return Err(format!("UI node {} has out-of-range geometry", node.id));
            }
            if let Kind::Label {
                binding: Some(binding),
                ..
            } = &node.kind
            {
                let room = matches!(
                    binding,
                    Binding::KeyAcquired | Binding::ExitState | Binding::RoomPhase
                );
                if room != (profile == Profile::Room) {
                    return Err("UI binding does not match consuming profile".into());
                }
            }
            match &node.kind {
                Kind::Label { text, .. } | Kind::Button { text, .. } => {
                    if text.chars().count() > 128 || text.chars().any(char::is_control) {
                        return Err(format!(
                            "UI node {} text exceeds 128 characters or has controls",
                            node.id
                        ));
                    }
                }
                Kind::Container => {}
            }
            let depth = if let Some(id) = &node.parent {
                let (parent, depth) = previous
                    .get(id.as_str())
                    .ok_or_else(|| format!("UI parent {id} must precede child {}", node.id))?;
                if !matches!(parent.kind, Kind::Container) || parent.screen != node.screen {
                    return Err(format!(
                        "UI parent {id} must be a container on the same screen"
                    ));
                }
                depth + 1
            } else {
                1
            };
            if depth > MAX_DEPTH {
                return Err("Collect UI exceeds hierarchy depth 4 (root depth 1)".into());
            }
            previous.insert(&node.id, (node, depth));
        }
        Ok(())
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        self.to_bytes_for(Profile::Collect)
    }

    pub fn to_bytes_for(&self, profile: Profile) -> Result<Vec<u8>, String> {
        self.validate_for(profile)?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|error| error.to_string())?;
        if bytes.len() > MAX_BYTES {
            return Err("Collect UI exceeds 64 KiB".into());
        }
        Ok(bytes)
    }

    /// Complete authored text plus characters used by the fixed runtime values.
    /// Consumers can construct a font atlas without interpreting authored text.
    pub fn corpus(&self) -> String {
        self.corpus_for(Profile::Collect)
    }
    pub fn corpus_for(&self, profile: Profile) -> String {
        let mut result = String::from(match profile {
            Profile::Collect => "0123456789 /:- PLAYING WON LOST: hazard LOST: time INVALID Best Score Phase unavailable",
            Profile::Room => "ACQUIRED MISSING LOCKED UNLOCKED WON PLAYING INVALID",
        });
        for node in &self.nodes {
            if let Kind::Label { text, .. } | Kind::Button { text, .. } = &node.kind {
                result.push(' ');
                result.push_str(text);
            }
        }
        result
    }

    pub fn default_room() -> Self {
        let mut document = Self::default_collect();
        for node in &mut document.nodes {
            if let Kind::Label { text, binding } = &mut node.kind {
                match binding {
                    Some(Binding::Score) => {
                        *text = "Key:".into();
                        *binding = Some(Binding::KeyAcquired);
                    }
                    Some(Binding::Best) => {
                        *text = "Exit:".into();
                        *binding = Some(Binding::ExitState);
                    }
                    Some(Binding::Phase) => {
                        *text = "Room:".into();
                        *binding = Some(Binding::RoomPhase);
                    }
                    None if node.id == "title" => *text = "Room Escape".into(),
                    _ => {}
                }
            }
        }
        for node in &mut document.nodes {
            node.id = match node.id.as_str() {
                "title_best" => "title_exit",
                "score" => "key",
                "phase" => "room_phase",
                "result_score" => "result_key",
                "result_best" => "result_exit",
                other => other,
            }
            .into();
        }
        if let Some(menu) = document.nodes.iter_mut().find(|node| node.id == "menu") {
            menu.offset[1] = 140;
        }
        document.nodes.push(Node {
            id: "exit".into(),
            parent: None,
            kind: Kind::Label {
                text: "Exit:".into(),
                binding: Some(Binding::ExitState),
            },
            screen: Screen::Playing,
            anchor: [500, 0],
            offset: [-160, 96],
            size: [320, 36],
        });
        let mut backgrounds = Vec::new();
        for (id, screen, y, height) in [
            ("title_background", Screen::Title, 40, 230),
            ("playing_background", Screen::Playing, 0, 190),
            ("menu_background", Screen::Menu, 40, 216),
            ("terminal_background", Screen::Terminal, 40, 254),
        ] {
            backgrounds.push(Node {
                id: id.into(),
                parent: None,
                kind: Kind::Container,
                screen,
                anchor: [500, 0],
                offset: [-174, y],
                size: [348, height],
            });
        }
        backgrounds.append(&mut document.nodes);
        document.nodes = backgrounds;
        document
    }

    pub fn default_collect() -> Self {
        let mut nodes = Vec::new();
        let mut add = |id: &str, screen, y, kind| {
            nodes.push(Node {
                id: id.into(),
                parent: None,
                screen,
                kind,
                anchor: [500, 0],
                offset: [-160, y],
                size: [320, 36],
            })
        };
        let label = |text: &str, binding| Kind::Label {
            text: text.into(),
            binding,
        };
        let button = |text: &str, action| Kind::Button {
            text: text.into(),
            action,
        };
        add(
            "title",
            Screen::Title,
            56,
            label("Collect and Dodge · 플레이", None),
        );
        add(
            "title_best",
            Screen::Title,
            104,
            label("Best: ", Some(Binding::Best)),
        );
        add("play", Screen::Title, 164, button("플레이", Action::Play));
        add(
            "title_quit",
            Screen::Title,
            208,
            button("종료", Action::Quit),
        );
        add(
            "score",
            Screen::Playing,
            16,
            label("Score: ", Some(Binding::Score)),
        );
        add(
            "phase",
            Screen::Playing,
            56,
            label("Phase: ", Some(Binding::Phase)),
        );
        add("menu", Screen::Playing, 100, button("메뉴", Action::Menu));
        add(
            "paused",
            Screen::Menu,
            56,
            label("로컬 조작만 멈춥니다.", None),
        );
        add(
            "continue",
            Screen::Menu,
            112,
            button("계속하기", Action::Continue),
        );
        add(
            "menu_restart",
            Screen::Menu,
            156,
            button("다시 시작", Action::Restart),
        );
        add("menu_quit", Screen::Menu, 200, button("종료", Action::Quit));
        add(
            "result",
            Screen::Terminal,
            56,
            label("Result: ", Some(Binding::Phase)),
        );
        add(
            "result_score",
            Screen::Terminal,
            100,
            label("Score: ", Some(Binding::Score)),
        );
        add(
            "result_best",
            Screen::Terminal,
            140,
            label("Best: ", Some(Binding::Best)),
        );
        add(
            "restart",
            Screen::Terminal,
            196,
            button("다시 시작", Action::Restart),
        );
        add(
            "result_quit",
            Screen::Terminal,
            240,
            button("종료", Action::Quit),
        );
        Self {
            schema: SCHEMA,
            nodes,
        }
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn node() -> Value {
        json!({"id":"root", "parent":null, "kind":{"type":"container"},
               "screen":"title", "anchor":[0,1000], "offset":[-4096,4096], "size":[1,4096]})
    }
    fn parse_node(node: Value) -> Result<Document, String> {
        Document::parse(&serde_json::to_vec(&json!({"schema":1,"nodes":[node]})).unwrap())
    }
    fn chain(count: usize) -> Document {
        Document {
            schema: 1,
            nodes: (0..count)
                .map(|i| Node {
                    id: format!("n{i}"),
                    parent: (i > 0).then(|| format!("n{}", i - 1)),
                    kind: Kind::Container,
                    screen: Screen::Title,
                    anchor: [0, 0],
                    offset: [0, 0],
                    size: [1, 1],
                })
                .collect(),
        }
    }

    #[test]
    fn default_round_trip_and_complete_vocabulary() {
        let doc = Document::default_collect();
        assert_eq!(Document::parse(&doc.to_bytes().unwrap()).unwrap(), doc);
        for screen in [
            Screen::Title,
            Screen::Playing,
            Screen::Menu,
            Screen::Terminal,
        ] {
            assert!(doc.nodes.iter().any(|node| node.screen == screen));
        }
        for action in [
            Action::Play,
            Action::Menu,
            Action::Continue,
            Action::Restart,
            Action::Quit,
        ] {
            assert!(doc
                .nodes
                .iter()
                .any(|node| matches!(node.kind, Kind::Button { action: a, .. } if a == action)));
        }
        for binding in [Binding::Score, Binding::Phase, Binding::Best] {
            assert!(doc.nodes.iter().any(
                |node| matches!(node.kind, Kind::Label { binding: Some(b), .. } if b == binding)
            ));
        }
        assert!(doc.corpus().contains("Collect and Dodge"));
        assert!(doc.corpus().contains("0123456789"));
    }

    #[test]
    fn boundaries_are_inclusive() {
        let mut n = node();
        n["id"] = json!("a".repeat(32));
        n["kind"] = json!({"type":"label","text":"界".repeat(128),"binding":null});
        assert!(parse_node(n).is_ok());
        assert!(chain(4).validate().is_ok());
        let mut doc = chain(1);
        doc.nodes = (0..32)
            .map(|i| {
                let mut n = doc.nodes[0].clone();
                n.id = format!("n{i}");
                n
            })
            .collect();
        assert!(doc.validate().is_ok());
        assert!(Document::parse(br#"{"schema":1,"nodes":[]}"#).is_ok());
    }

    #[test]
    fn rejects_size_version_syntax_and_trailing_data() {
        for bytes in [
            vec![b' '; MAX_BYTES + 1],
            b"{}".to_vec(),
            b"null".to_vec(),
            br#"{"schema":0,"nodes":[]}"#.to_vec(),
            br#"{"schema":2,"nodes":[]}"#.to_vec(),
            br#"{"schema":1.0,"nodes":[]}"#.to_vec(),
            br#"{"schema":1,"nodes":[]}{}"#.to_vec(),
            br#"{"schema":1,"nodes":[],"script":"run()"}"#.to_vec(),
            vec![0xff],
        ] {
            assert!(Document::parse(&bytes).is_err(), "accepted {bytes:?}");
        }
        let mut bytes = br#"{"schema":1,"nodes":[]}"#.to_vec();
        bytes.resize(MAX_BYTES, b' ');
        assert!(Document::parse(&bytes).is_ok());
        let mut doc = chain(1);
        doc.nodes = vec![doc.nodes[0].clone(); 33];
        assert!(doc.validate().is_err());
        assert!(doc.to_bytes().is_err());
    }

    #[test]
    fn rejects_invalid_ids_and_text() {
        for id in [
            "".into(),
            "a".repeat(33),
            "é".into(),
            "a.b".into(),
            "a/b".into(),
            "has space".into(),
            "\n".into(),
        ] {
            let mut n = node();
            n["id"] = json!(id);
            assert!(parse_node(n).is_err());
        }
        for text in [
            "x".repeat(129),
            "new\nline".into(),
            "tab\t".into(),
            "\0".into(),
            "\u{7f}".into(),
            "\u{85}".into(),
        ] {
            for kind in [
                json!({"type":"label","text":text}),
                json!({"type":"button","text":text,"action":"play"}),
            ] {
                let mut n = node();
                n["kind"] = kind;
                assert!(parse_node(n).is_err());
            }
        }
    }

    #[test]
    fn rejects_geometry_outside_bounds_and_wrong_types() {
        for (field, values) in [
            (
                "anchor",
                vec![
                    json!([-1, 0]),
                    json!([1001, 0]),
                    json!([0, 1001]),
                    json!([0.0, 0]),
                    json!([0]),
                    json!([0, 0, 0]),
                ],
            ),
            (
                "offset",
                vec![
                    json!([-4097, 0]),
                    json!([0, 4097]),
                    json!([32768, 0]),
                    json!([0, 0.5]),
                ],
            ),
            (
                "size",
                vec![
                    json!([0, 1]),
                    json!([1, 0]),
                    json!([4097, 1]),
                    json!([1, 4097]),
                    json!([65536, 1]),
                    json!(["1", 1]),
                ],
            ),
        ] {
            for value in values {
                let mut n = node();
                n[field] = value;
                assert!(parse_node(n).is_err());
            }
        }
    }

    #[test]
    fn rejects_hierarchy_errors() {
        assert!(chain(5).validate().is_err());
        let mut doc = chain(2);
        doc.nodes[1].id = "n0".into();
        assert!(doc.validate().is_err());
        let mut doc = chain(2);
        doc.nodes[0].parent = Some("n1".into()); // cycle and forward reference
        assert!(doc.validate().is_err());
        let mut doc = chain(1);
        doc.nodes[0].parent = Some("n0".into());
        assert!(doc.validate().is_err());
        doc.nodes[0].parent = Some("missing".into());
        assert!(doc.validate().is_err());
        let mut doc = chain(2);
        doc.nodes[1].screen = Screen::Menu;
        assert!(doc.validate().is_err());
        for kind in [
            Kind::Label {
                text: String::new(),
                binding: None,
            },
            Kind::Button {
                text: String::new(),
                action: Action::Quit,
            },
        ] {
            let mut doc = chain(2);
            doc.nodes[0].kind = kind;
            assert!(doc.validate().is_err());
        }
    }

    #[test]
    fn rejects_unknown_fields_variants_and_expressions() {
        let mut n = node();
        n["callback"] = json!("eval()");
        assert!(parse_node(n).is_err());
        for kind in [
            json!({"type":"container","text":"unexpected"}),
            json!({"type":"label","text":"hello","binding":"score + 1"}),
            json!({"type":"label","text":"hello","binding":{"score":1}}),
            json!({"type":"label","text":"hello","action":"play"}),
            json!({"type":"button","text":"hello","action":"load_file"}),
            json!({"type":"button","text":"hello","action":"play","binding":"score"}),
            json!({"type":"button","text":"hello"}),
            json!({"type":"script"}),
        ] {
            let mut n = node();
            n["kind"] = kind;
            assert!(parse_node(n).is_err());
        }
        let mut n = node();
        n["screen"] = json!("global");
        assert!(parse_node(n).is_err());
        for field in ["id", "kind", "screen", "anchor", "offset", "size"] {
            let mut n = node();
            n.as_object_mut().unwrap().remove(field);
            assert!(parse_node(n).is_err());
        }
    }

    #[test]
    fn rejects_duplicate_fields_including_null_and_tags() {
        for json in [
            r#"{"schema":1,"schema":1,"nodes":[]}"#,
            r#"{"schema":1,"nodes":[],"nodes":[]}"#,
        ] {
            assert!(Document::parse(json.as_bytes()).is_err());
        }
        let base = serde_json::to_string(&node()).unwrap();
        for (field, value) in [
            ("id", r#""root""#),
            ("parent", "null"),
            ("screen", r#""title""#),
            ("kind", r#"{"type":"container"}"#),
            ("anchor", "[0,1000]"),
            ("offset", "[-4096,4096]"),
            ("size", "[1,4096]"),
        ] {
            let n = format!("{{\"{field}\":{value},{}", &base[1..]);
            assert!(
                Document::parse(format!("{{\"schema\":1,\"nodes\":[{n}]}}").as_bytes()).is_err()
            );
        }
        for kind in [
            r#"{"type":"container","type":"container"}"#,
            r#"{"type":"label","text":"a","text":"b"}"#,
            r#"{"type":"label","text":"a","binding":null,"binding":null}"#,
            r#"{"type":"button","text":"a","action":"play","action":"quit"}"#,
        ] {
            let n = base.replace(r#"{"type":"container"}"#, kind);
            assert!(
                Document::parse(format!("{{\"schema\":1,\"nodes\":[{n}]}}").as_bytes()).is_err()
            );
        }
    }
}

#[cfg(test)]
mod room_profile_tests {
    use super::*;

    #[test]
    fn room_profile_is_explicit_and_collect_default_stays_closed() {
        let room = Document::default_room();
        assert!(room.validate().is_err());
        assert!(room.to_bytes().is_err());
        let bytes = room.to_bytes_for(Profile::Room).unwrap();
        assert!(Document::parse(&bytes).is_err());
        assert_eq!(Document::parse_for(&bytes, Profile::Room).unwrap(), room);
        assert!(Document::default_collect()
            .validate_for(Profile::Room)
            .is_err());
    }

    #[test]
    fn room_schema_rejects_cross_bindings_unknown_duplicate_and_bad_geometry() {
        let base = Document::default_room();
        for binding in [Binding::Score, Binding::Phase, Binding::Best] {
            let mut doc = base.clone();
            doc.nodes[0].kind = Kind::Label {
                text: "bad".into(),
                binding: Some(binding),
            };
            assert!(doc.validate_for(Profile::Room).is_err());
        }
        for bad in [
            br#"[1,[]]"#.as_slice(),
            br#"{"schema":1,"nodes":[["x",null,{"type":"label","text":"Room","binding":null},"title",[0,0],[0,0],[100,30]]]}"#.as_slice(),
            br#"{"schema":1,"nodes":[{"id":"x","parent":null,"kind":["container"],"screen":"title","anchor":[0,0],"offset":[0,0],"size":[100,30]}]}"#.as_slice(),
            br#"{"schema":1,"nodes":[{"id":"x","parent":null,"kind":{"type":"container"},"screen":"title","anchor":[0,0],"offset":[0,0],"size":[100,30]},{"id":"child","parent":"x","kind":["label","x",null],"screen":"title","anchor":[0,0],"offset":[0,0],"size":[50,20]}]}"#.as_slice(),
            br#"{"schema":1,"schema":1,"nodes":[]}"#.as_slice(),
            br#"{"schema":1,"nodes":[],"callback":"run"}"#.as_slice(),
            br#"{"schema":1,"nodes":[{"id":"x","parent":null,"kind":{"type":"container","extra":1},"screen":"playing","anchor":[0,0],"offset":[0,0],"size":[10,10]}]}"#.as_slice(),
        ] { assert!(Document::parse_for(bad, Profile::Room).is_err()); }
        let mut doc = base;
        doc.nodes[0].size = [0, 1];
        assert!(doc.validate_for(Profile::Room).is_err());
    }
}
