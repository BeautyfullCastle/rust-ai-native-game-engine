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

impl Document {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err("Collect UI exceeds 64 KiB".into());
        }
        // Decode straight into strict structs, not Value: Value would silently
        // discard duplicate JSON keys before schema validation could see them.
        let document: Self = serde_json::from_slice(bytes)
            .map_err(|error| format!("invalid Collect UI JSON: {error}"))?;
        document.validate()?;
        Ok(document)
    }

    pub fn validate(&self) -> Result<(), String> {
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
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|error| error.to_string())?;
        if bytes.len() > MAX_BYTES {
            return Err("Collect UI exceeds 64 KiB".into());
        }
        Ok(bytes)
    }

    /// Complete authored text plus characters used by the fixed runtime values.
    /// Consumers can construct a font atlas without interpreting authored text.
    pub fn corpus(&self) -> String {
        let mut result = String::from("0123456789 /:- PLAYING WON LOST: hazard LOST: time INVALID Best Score Phase unavailable");
        for node in &self.nodes {
            if let Kind::Label { text, .. } | Kind::Button { text, .. } = &node.kind {
                result.push(' ');
                result.push_str(text);
            }
        }
        result
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
