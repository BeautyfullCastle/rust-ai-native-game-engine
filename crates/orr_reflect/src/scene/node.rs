//! Strict YAML reading: from text to a tree of [`Node`]s with positions.
//!
//! `saphyr-parser` turns the text into events. This module builds the tree
//! and rejects everything the scene format forbids, with a line and column:
//! anchors, aliases, tags, merge keys, complex keys, duplicate keys, more
//! than one document, and nesting deeper than [`MAX_DEPTH`].
//!
//! The tree keeps scalars as text and remembers whether they were quoted.
//! It never turns text into booleans, numbers or null. That is done later,
//! against the schema of the field (see `decode.rs`), so `no` stays a string
//! unless the field is a bool.

use saphyr_parser::{Event, Parser, ScalarStyle, Span};

/// Deepest allowed nesting of maps and lists.
pub const MAX_DEPTH: usize = 48;

/// Position in the text, 1-based.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pos {
    /// Line, from 1.
    pub line: usize,
    /// Column, from 1.
    pub col: usize,
}

impl Pos {
    fn of(span: &Span) -> Pos {
        Pos { line: span.start.line(), col: span.start.col() + 1 }
    }
}

/// A problem with a position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diag {
    /// Where.
    pub pos: Pos,
    /// What.
    pub message: String,
}

impl Diag {
    pub(crate) fn new(pos: Pos, message: impl Into<String>) -> Self {
        Self { pos, message: message.into() }
    }
}

/// One YAML node.
#[derive(Clone, Debug)]
pub struct Node {
    /// Where the node starts.
    pub pos: Pos,
    /// What it is.
    pub kind: NodeKind,
}

/// The shape of a node.
#[derive(Clone, Debug)]
pub enum NodeKind {
    /// A scalar: its text and whether it was quoted.
    Scalar {
        /// Text of the scalar (without quotes, escapes resolved).
        text: String,
        /// True for `'...'`, `"..."`, `|` and `>` scalars.
        quoted: bool,
    },
    /// A list.
    Seq(Vec<Node>),
    /// A map, in file order. Keys are scalars.
    Map(Vec<(Node, Node)>),
}

impl Node {
    /// What kind of node this is, for messages.
    pub fn describe(&self) -> String {
        match &self.kind {
            NodeKind::Scalar { text, quoted: true } => format!("the string \"{}\"", clip(text)),
            NodeKind::Scalar { text, quoted: false } if text.is_empty() => "an empty value".to_string(),
            NodeKind::Scalar { text, .. } => format!("'{}'", clip(text)),
            NodeKind::Seq(_) => "a list".to_string(),
            NodeKind::Map(_) => "a map".to_string(),
        }
    }

    /// The text of a scalar key.
    pub fn key_text(&self) -> Option<&str> {
        match &self.kind {
            NodeKind::Scalar { text, .. } => Some(text),
            _ => None,
        }
    }
}

fn clip(s: &str) -> String {
    let mut out: String = s.chars().take(40).collect();
    if s.chars().count() > 40 {
        out.push_str("...");
    }
    out
}

enum Frame {
    Seq { pos: Pos, items: Vec<Node> },
    Map { pos: Pos, entries: Vec<(Node, Node)>, key: Option<Node> },
}

/// Parses `src` into one node tree, or fails with the first problem.
pub fn parse(src: &str) -> Result<Node, Diag> {
    let mut parser = Parser::new_from_str(src);
    let mut stack: Vec<Frame> = Vec::new();
    let mut root: Option<Node> = None;
    let mut docs = 0usize;

    while let Some(ev) = parser.next() {
        let (ev, span) = ev.map_err(|e| {
            let m = e.marker();
            Diag::new(Pos { line: m.line(), col: m.col() + 1 }, format!("YAML syntax error: {}", e.info()))
        })?;
        let pos = Pos::of(&span);
        let node = match ev {
            Event::Nothing | Event::StreamStart | Event::StreamEnd | Event::DocumentEnd => continue,
            Event::DocumentStart(_) => {
                docs += 1;
                if docs > 1 {
                    return Err(Diag::new(pos, "a scene file holds exactly one YAML document"));
                }
                continue;
            }
            Event::Alias(_) => return Err(Diag::new(pos, "aliases (*name) are not allowed in scene files")),
            Event::Scalar(text, style, anchor, tag) => {
                if anchor != 0 {
                    return Err(Diag::new(pos, "anchors (&name) are not allowed in scene files"));
                }
                if let Some(t) = tag {
                    return Err(Diag::new(pos, format!("tags ({}{}) are not allowed in scene files", t.handle, t.suffix)));
                }
                Node {
                    pos,
                    kind: NodeKind::Scalar { text: text.into_owned(), quoted: !matches!(style, ScalarStyle::Plain) },
                }
            }
            Event::SequenceStart(anchor, tag) | Event::MappingStart(anchor, tag) if anchor != 0 || tag.is_some() => {
                return Err(match tag {
                    Some(t) => Diag::new(pos, format!("tags ({}{}) are not allowed in scene files", t.handle, t.suffix)),
                    None => Diag::new(pos, "anchors (&name) are not allowed in scene files"),
                });
            }
            Event::SequenceStart(..) => {
                if stack.len() >= MAX_DEPTH {
                    return Err(Diag::new(pos, format!("nesting is deeper than {MAX_DEPTH} levels")));
                }
                stack.push(Frame::Seq { pos, items: Vec::new() });
                continue;
            }
            Event::MappingStart(..) => {
                if stack.len() >= MAX_DEPTH {
                    return Err(Diag::new(pos, format!("nesting is deeper than {MAX_DEPTH} levels")));
                }
                stack.push(Frame::Map { pos, entries: Vec::new(), key: None });
                continue;
            }
            Event::SequenceEnd => match stack.pop() {
                Some(Frame::Seq { pos, items }) => Node { pos, kind: NodeKind::Seq(items) },
                _ => return Err(Diag::new(pos, "internal YAML error: unbalanced list end")),
            },
            Event::MappingEnd => match stack.pop() {
                Some(Frame::Map { pos, entries, key: None }) => Node { pos, kind: NodeKind::Map(entries) },
                Some(Frame::Map { key: Some(k), .. }) => return Err(Diag::new(k.pos, "map key without a value")),
                _ => return Err(Diag::new(pos, "internal YAML error: unbalanced map end")),
            },
        };
        // A finished node goes into its parent.
        match stack.last_mut() {
            None => {
                if root.is_some() {
                    return Err(Diag::new(pos, "more than one value at the top level"));
                }
                root = Some(node);
            }
            Some(Frame::Seq { items, .. }) => items.push(node),
            Some(Frame::Map { entries, key, .. }) => match key.take() {
                None => {
                    match &node.kind {
                        NodeKind::Scalar { text, .. } => {
                            if text == "<<" {
                                return Err(Diag::new(node.pos, "merge keys (<<) are not allowed in scene files"));
                            }
                            if let Some((first, _)) = entries.iter().find(|(k, _)| k.key_text() == Some(text.as_str())) {
                                return Err(Diag::new(
                                    node.pos,
                                    format!("duplicate key '{}' (first used at line {})", clip(text), first.pos.line),
                                ));
                            }
                        }
                        _ => return Err(Diag::new(node.pos, "map keys must be plain names, not lists or maps")),
                    }
                    *key = Some(node);
                }
                Some(k) => entries.push((k, node)),
            },
        }
    }

    root.ok_or_else(|| Diag::new(Pos { line: 1, col: 1 }, "the file is empty"))
}
