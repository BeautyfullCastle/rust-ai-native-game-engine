//! The arrangement of the editor panels, and where a user keeps it.
//!
//! This module has no window code. It holds the layout as plain data, checks it, turns it
//! into JSON and back, and reads and writes the user's layout files. A layout file is a
//! local user setting. It never touches a scene file, the undo history or a checksum.
//!
//! Every read is bounded: file size, node count, nesting depth, name length and preset
//! count. A file that breaks a bound, or that names an unknown panel or a newer version,
//! is refused as a whole. The caller then shows the default layout and a notice.

use std::collections::BTreeSet;
use std::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Value};

/// Version written into every layout file. A file with another version is refused.
pub const VERSION: u64 = 1;
/// Largest layout file that is read, in bytes.
pub const MAX_FILE_BYTES: u64 = 64 * 1024;
/// Most nodes (panes, splits and tab groups together) in one layout.
pub const MAX_NODES: usize = 64;
/// Deepest nesting of splits and tab groups.
pub const MAX_DEPTH: usize = 8;
/// Most children of one split or tab group.
pub const MAX_CHILDREN: usize = 8;
/// Most named presets a user can keep.
pub const MAX_PRESETS: usize = 16;
/// Longest preset name, in characters.
pub const MAX_NAME_CHARS: usize = 40;
/// Total of the share numbers is not limited, but one share stays inside this range.
pub const MAX_SHARE: u32 = 1000;

const FORMAT: &str = "orrery-editor-layout";
const LAST_FILE: &str = "last.json";
/// How often a save tries again when the replace is refused for a moment.
const RENAME_RETRIES: u32 = 8;
const PRESET_PREFIX: &str = "preset-";

/// One panel of the editor window.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum Panel {
    /// The entity list.
    Hierarchy,
    /// The scene view. It can move but it can never be hidden.
    Viewport,
    /// The component fields of the selection.
    Inspector,
    /// The play controls, history and the other tabs at the bottom.
    Timeline,
}

impl Panel {
    /// Every panel of this build.
    pub const ALL: [Panel; 4] = [Panel::Hierarchy, Panel::Viewport, Panel::Inspector, Panel::Timeline];

    /// The name used in layout files.
    pub fn key(self) -> &'static str {
        match self {
            Panel::Hierarchy => "hierarchy",
            Panel::Viewport => "viewport",
            Panel::Inspector => "inspector",
            Panel::Timeline => "timeline",
        }
    }

    /// The name shown to the user.
    pub fn title(self) -> &'static str {
        match self {
            Panel::Hierarchy => "Hierarchy",
            Panel::Viewport => "Viewport",
            Panel::Inspector => "Inspector",
            Panel::Timeline => "Timeline & tools",
        }
    }

    /// The panel with this file name, if this build has it.
    pub fn from_key(key: &str) -> Option<Panel> {
        Panel::ALL.into_iter().find(|panel| panel.key() == key)
    }

    /// True for a panel that the user may hide.
    pub fn can_hide(self) -> bool {
        self != Panel::Viewport
    }
}

/// The direction of a split.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    /// Children side by side.
    Horizontal,
    /// Children one above the other.
    Vertical,
}

/// A window edge that a panel can dock to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edge {
    /// The left edge.
    Left,
    /// The right edge.
    Right,
    /// The top edge.
    Top,
    /// The bottom edge.
    Bottom,
}

/// One child of a split, with its share of the split.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Child {
    /// Relative size in 1 to [`MAX_SHARE`]. Only the ratios between the children matter.
    pub share: u32,
    /// The child.
    pub node: Node,
}

/// A part of the layout tree.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Node {
    /// One panel. A hidden panel keeps its place for when it is shown again.
    Pane {
        /// Which panel.
        panel: Panel,
        /// False while the user has hidden it.
        visible: bool,
    },
    /// Children that share the space.
    Split {
        /// How the children line up.
        dir: Dir,
        /// The children in order.
        children: Vec<Child>,
    },
    /// Children that share one place and show one at a time.
    Tabs {
        /// The tab contents in order.
        children: Vec<Node>,
        /// Index of the shown tab.
        active: usize,
    },
}

/// Why a layout or a layout file was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum LayoutError {
    /// The file is larger than [`MAX_FILE_BYTES`].
    TooLarge,
    /// The text is not valid JSON, or a field has the wrong type.
    Corrupt(String),
    /// The file has a version this editor does not know.
    Version(u64),
    /// A panel name is not a panel of this build.
    UnknownPanel(String),
    /// A panel of this build is not in the layout.
    MissingPanel(Panel),
    /// A panel is in the layout twice.
    DuplicatePanel(Panel),
    /// The viewport is hidden.
    HiddenViewport,
    /// The layout breaks a node, depth, child or share bound.
    Limit(&'static str),
    /// The layout cannot be written or read.
    Io(String),
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LayoutError::TooLarge => write!(f, "the file is larger than {} KiB", MAX_FILE_BYTES / 1024),
            LayoutError::Corrupt(why) => write!(f, "the file is damaged ({why})"),
            LayoutError::Version(version) => write!(f, "the file has version {version}, and this editor reads version {VERSION}"),
            LayoutError::UnknownPanel(name) => write!(f, "the file names a panel that this editor does not have ({name})"),
            LayoutError::MissingPanel(panel) => write!(f, "the file has no place for the {} panel", panel.title()),
            LayoutError::DuplicatePanel(panel) => write!(f, "the file lists the {} panel twice", panel.title()),
            LayoutError::HiddenViewport => write!(f, "the file hides the viewport"),
            LayoutError::Limit(what) => write!(f, "the file breaks a limit ({what})"),
            LayoutError::Io(why) => write!(f, "{why}"),
        }
    }
}

/// The arrangement of all panels.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Layout {
    /// The top of the tree.
    pub root: Node,
}

impl Default for Layout {
    /// Hierarchy at the left, Inspector at the right, the Timeline across the bottom.
    fn default() -> Self {
        let pane = |panel| Node::Pane { panel, visible: true };
        let child = |share, node| Child { share, node };
        Layout {
            root: Node::Split {
                dir: Dir::Vertical,
                children: vec![
                    child(840, Node::Split {
                        dir: Dir::Horizontal,
                        children: vec![
                            child(180, pane(Panel::Hierarchy)),
                            child(554, pane(Panel::Viewport)),
                            child(266, pane(Panel::Inspector)),
                        ],
                    }),
                    child(160, pane(Panel::Timeline)),
                ],
            },
        }
    }
}

impl Layout {
    /// Checks the rules every layout must keep.
    pub fn validate(&self) -> Result<(), LayoutError> {
        let mut seen = BTreeSet::new();
        let mut nodes = 0;
        let mut viewport_visible = false;
        check_node(&self.root, 1, &mut nodes, &mut seen, &mut viewport_visible)?;
        for panel in Panel::ALL {
            if !seen.contains(&panel) {
                return Err(LayoutError::MissingPanel(panel));
            }
        }
        if !viewport_visible {
            return Err(LayoutError::HiddenViewport);
        }
        Ok(())
    }

    /// True while `panel` is shown.
    pub fn is_visible(&self, panel: Panel) -> bool {
        fn find(node: &Node, panel: Panel) -> Option<bool> {
            match node {
                Node::Pane { panel: p, visible } => (*p == panel).then_some(*visible),
                Node::Split { children, .. } => children.iter().find_map(|child| find(&child.node, panel)),
                Node::Tabs { children, .. } => children.iter().find_map(|child| find(child, panel)),
            }
        }
        find(&self.root, panel).unwrap_or(false)
    }

    /// Shows or hides `panel`. The viewport stays visible.
    pub fn set_visible(&mut self, panel: Panel, show: bool) {
        fn set(node: &mut Node, panel: Panel, show: bool) {
            match node {
                Node::Pane { panel: p, visible } => {
                    if *p == panel {
                        *visible = show;
                    }
                }
                Node::Split { children, .. } => children.iter_mut().for_each(|child| set(&mut child.node, panel, show)),
                Node::Tabs { children, .. } => children.iter_mut().for_each(|child| set(child, panel, show)),
            }
        }
        if panel.can_hide() || show {
            set(&mut self.root, panel, show);
        }
    }

    /// Moves `panel` to a window edge. The panel is shown afterwards.
    pub fn dock(&mut self, panel: Panel, edge: Edge) {
        let Some(rest) = self.take(panel) else { return };
        let moved = Child { share: 220, node: Node::Pane { panel, visible: true } };
        let rest = Child { share: 780, node: rest };
        let (dir, children) = match edge {
            Edge::Left => (Dir::Horizontal, vec![moved, rest]),
            Edge::Right => (Dir::Horizontal, vec![rest, moved]),
            Edge::Top => (Dir::Vertical, vec![moved, rest]),
            Edge::Bottom => (Dir::Vertical, vec![rest, moved]),
        };
        self.root = Node::Split { dir, children };
    }

    /// Moves `panel` into the tab group of `other`. The panel is shown afterwards.
    pub fn tab_with(&mut self, panel: Panel, other: Panel) {
        if panel == other {
            return;
        }
        let Some(rest) = self.take(panel) else { return };
        let mut root = rest;
        if !put_beside(&mut root, other, panel) {
            // `other` was not found. Keep the layout whole instead of losing the panel.
            root = Node::Split {
                dir: Dir::Horizontal,
                children: vec![
                    Child { share: 780, node: root },
                    Child { share: 220, node: Node::Pane { panel, visible: true } },
                ],
            };
        }
        self.root = root;
    }

    /// Removes `panel` and tidies up. Returns what is left, or `None` when nothing is.
    fn take(&mut self, panel: Panel) -> Option<Node> {
        let placeholder = Node::Tabs { children: Vec::new(), active: 0 };
        let root = std::mem::replace(&mut self.root, placeholder);
        let (rest, found) = remove_pane(root.clone(), panel);
        if !found || rest.is_none() {
            self.root = root;
            return None;
        }
        rest
    }

    /// The JSON value of the layout tree.
    pub fn to_value(&self) -> Value {
        node_value(&self.root)
    }

    /// Reads a layout tree from JSON and checks it.
    pub fn from_value(value: &Value) -> Result<Layout, LayoutError> {
        let mut nodes = 0;
        let root = parse_node(value, 1, &mut nodes)?;
        let layout = Layout { root };
        layout.validate()?;
        Ok(layout)
    }
}

fn check_node(node: &Node, depth: usize, nodes: &mut usize, seen: &mut BTreeSet<Panel>, viewport_visible: &mut bool) -> Result<(), LayoutError> {
    *nodes += 1;
    if *nodes > MAX_NODES {
        return Err(LayoutError::Limit("too many nodes"));
    }
    if depth > MAX_DEPTH {
        return Err(LayoutError::Limit("nested too deep"));
    }
    match node {
        Node::Pane { panel, visible } => {
            if !seen.insert(*panel) {
                return Err(LayoutError::DuplicatePanel(*panel));
            }
            if *panel == Panel::Viewport {
                *viewport_visible = *visible;
            }
        }
        Node::Split { children, .. } => {
            if children.is_empty() || children.len() > MAX_CHILDREN {
                return Err(LayoutError::Limit("a split has no children or too many"));
            }
            for child in children {
                if child.share == 0 || child.share > MAX_SHARE {
                    return Err(LayoutError::Limit("a share is out of range"));
                }
                check_node(&child.node, depth + 1, nodes, seen, viewport_visible)?;
            }
        }
        Node::Tabs { children, active } => {
            if children.is_empty() || children.len() > MAX_CHILDREN {
                return Err(LayoutError::Limit("a tab group has no tabs or too many"));
            }
            if *active >= children.len() {
                return Err(LayoutError::Limit("the active tab does not exist"));
            }
            for child in children {
                check_node(child, depth + 1, nodes, seen, viewport_visible)?;
            }
        }
    }
    Ok(())
}

/// Removes `panel` from `node`. Empty groups go away and a split of one child becomes the child.
fn remove_pane(node: Node, panel: Panel) -> (Option<Node>, bool) {
    match node {
        Node::Pane { panel: p, .. } if p == panel => (None, true),
        Node::Pane { .. } => (Some(node), false),
        Node::Split { dir, children } => {
            let mut found = false;
            let mut kept = Vec::new();
            for child in children {
                let (rest, hit) = remove_pane(child.node, panel);
                found |= hit;
                if let Some(node) = rest {
                    kept.push(Child { share: child.share, node });
                }
            }
            match kept.len() {
                0 => (None, found),
                1 => (kept.pop().map(|child| child.node), found),
                _ => (Some(Node::Split { dir, children: kept }), found),
            }
        }
        Node::Tabs { children, active } => {
            let mut found = false;
            let mut kept = Vec::new();
            let mut new_active = 0;
            for (index, child) in children.into_iter().enumerate() {
                let (rest, hit) = remove_pane(child, panel);
                found |= hit;
                if let Some(node) = rest {
                    if index <= active {
                        new_active = kept.len();
                    }
                    kept.push(node);
                }
            }
            match kept.len() {
                0 => (None, found),
                1 => (kept.pop(), found),
                n => (Some(Node::Tabs { children: kept, active: new_active.min(n - 1) }), found),
            }
        }
    }
}

/// Puts `panel` into the tab group of `other`. Returns false when `other` is not in `node`.
fn put_beside(node: &mut Node, other: Panel, panel: Panel) -> bool {
    let moved = Node::Pane { panel, visible: true };
    match node {
        Node::Pane { panel: p, visible } if *p == other => {
            let first = Node::Pane { panel: other, visible: *visible };
            *node = Node::Tabs { children: vec![first, moved], active: 1 };
            true
        }
        Node::Pane { .. } => false,
        Node::Split { children, .. } => {
            for child in children.iter_mut() {
                if put_beside(&mut child.node, other, panel) {
                    return true;
                }
            }
            false
        }
        Node::Tabs { children, active } => {
            if children.iter().any(|child| matches!(child, Node::Pane { panel: p, .. } if *p == other)) {
                if children.len() < MAX_CHILDREN {
                    children.push(moved);
                    *active = children.len() - 1;
                    return true;
                }
                return false;
            }
            for child in children.iter_mut() {
                if put_beside(child, other, panel) {
                    return true;
                }
            }
            false
        }
    }
}

fn node_value(node: &Node) -> Value {
    match node {
        Node::Pane { panel, visible } => json!({ "pane": panel.key(), "visible": visible }),
        Node::Split { dir, children } => json!({
            "split": match dir { Dir::Horizontal => "horizontal", Dir::Vertical => "vertical" },
            "children": children.iter().map(|child| json!({ "share": child.share, "node": node_value(&child.node) })).collect::<Vec<_>>(),
        }),
        Node::Tabs { children, active } => json!({
            "tabs": children.iter().map(node_value).collect::<Vec<_>>(),
            "active": active,
        }),
    }
}

fn corrupt(why: &str) -> LayoutError {
    LayoutError::Corrupt(why.to_owned())
}

fn parse_node(value: &Value, depth: usize, nodes: &mut usize) -> Result<Node, LayoutError> {
    *nodes += 1;
    if *nodes > MAX_NODES {
        return Err(LayoutError::Limit("too many nodes"));
    }
    if depth > MAX_DEPTH {
        return Err(LayoutError::Limit("nested too deep"));
    }
    let object = value.as_object().ok_or_else(|| corrupt("a node is not an object"))?;
    if let Some(key) = object.get("pane") {
        let key = key.as_str().ok_or_else(|| corrupt("a pane name is not text"))?;
        let panel = Panel::from_key(key).ok_or_else(|| LayoutError::UnknownPanel(key.chars().take(32).collect()))?;
        let visible = object.get("visible").and_then(Value::as_bool).ok_or_else(|| corrupt("a pane has no visible flag"))?;
        return Ok(Node::Pane { panel, visible });
    }
    if let Some(dir) = object.get("split") {
        let dir = match dir.as_str() {
            Some("horizontal") => Dir::Horizontal,
            Some("vertical") => Dir::Vertical,
            _ => return Err(corrupt("a split has an unknown direction")),
        };
        let list = object.get("children").and_then(Value::as_array).ok_or_else(|| corrupt("a split has no children list"))?;
        if list.is_empty() || list.len() > MAX_CHILDREN {
            return Err(LayoutError::Limit("a split has no children or too many"));
        }
        let mut children = Vec::new();
        for entry in list {
            let share = entry.get("share").and_then(Value::as_u64).ok_or_else(|| corrupt("a share is not a whole number"))?;
            let share = u32::try_from(share).ok().filter(|share| (1..=MAX_SHARE).contains(share)).ok_or(LayoutError::Limit("a share is out of range"))?;
            let node = parse_node(entry.get("node").ok_or_else(|| corrupt("a split child has no node"))?, depth + 1, nodes)?;
            children.push(Child { share, node });
        }
        return Ok(Node::Split { dir, children });
    }
    if let Some(list) = object.get("tabs") {
        let list = list.as_array().ok_or_else(|| corrupt("a tab group is not a list"))?;
        if list.is_empty() || list.len() > MAX_CHILDREN {
            return Err(LayoutError::Limit("a tab group has no tabs or too many"));
        }
        let active = object.get("active").and_then(Value::as_u64).ok_or_else(|| corrupt("a tab group has no active tab"))?;
        let active = usize::try_from(active).ok().filter(|active| *active < list.len()).ok_or(LayoutError::Limit("the active tab does not exist"))?;
        let children = list.iter().map(|entry| parse_node(entry, depth + 1, nodes)).collect::<Result<Vec<_>, _>>()?;
        return Ok(Node::Tabs { children, active });
    }
    Err(corrupt("a node is neither a pane, a split nor a tab group"))
}

/// A preset name as the user may keep it: not empty, short, and free of control characters.
pub fn clean_name(name: &str) -> Option<String> {
    let name = name.trim();
    let valid = !name.is_empty() && name.chars().count() <= MAX_NAME_CHARS && !name.chars().any(char::is_control);
    valid.then(|| name.to_owned())
}

/// The file name of a preset. It comes from the name, never from user-given path text.
fn preset_file(name: &str) -> String {
    let slug: String = name
        .chars()
        .filter_map(|c| if c.is_ascii_alphanumeric() { Some(c.to_ascii_lowercase()) } else if c == ' ' || c == '-' || c == '_' { Some('-') } else { None })
        .take(24)
        .collect();
    // FNV-1a keeps two names with the same slug apart.
    let hash = name.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3));
    format!("{PRESET_PREFIX}{slug}-{:08x}.json", hash & 0xffff_ffff)
}

/// Where the layout files of one user live.
#[derive(Clone, Debug)]
pub struct LayoutStore {
    dir: PathBuf,
}

impl LayoutStore {
    /// A store in `dir`. The folder is created on the first write.
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        LayoutStore { dir: dir.into() }
    }

    /// The per-user folder of this platform: `%LOCALAPPDATA%\orrery\editor\layouts` on Windows,
    /// `~/Library/Application Support/orrery/editor/layouts` on macOS and
    /// `$XDG_CONFIG_HOME/orrery/editor/layouts` (or `~/.config/...`) elsewhere.
    /// Returns `None` when the platform gives no absolute base folder.
    pub fn user_default() -> Option<Self> {
        let var = |name: &str| std::env::var_os(name).map(PathBuf::from).filter(|path| path.is_absolute());
        let base = if cfg!(windows) {
            var("LOCALAPPDATA")?
        } else if cfg!(target_os = "macos") {
            var("HOME")?.join("Library").join("Application Support")
        } else {
            var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|home| home.join(".config")))?
        };
        Some(Self::at(base.join("orrery").join("editor").join("layouts")))
    }

    /// The folder of the store.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Reads the layout of the last session. `Ok(None)` means there is none yet.
    pub fn read_last(&self) -> Result<Option<Layout>, LayoutError> {
        Ok(self.read_file(&self.dir.join(LAST_FILE))?.map(|(_, layout)| layout))
    }

    /// Writes the layout of this session. A failed write leaves the old file as it was.
    pub fn write_last(&self, layout: &Layout) -> Result<(), LayoutError> {
        self.write_file(LAST_FILE, None, layout)
    }

    /// The names of the valid presets, sorted. Files that cannot be read are skipped.
    pub fn list_presets(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        let mut names = BTreeSet::new();
        for entry in entries.flatten().take(MAX_PRESETS * 4) {
            let file = entry.file_name();
            let Some(file) = file.to_str() else { continue };
            if !file.starts_with(PRESET_PREFIX) || !file.ends_with(".json") {
                continue;
            }
            if let Ok(Some((Some(name), _))) = self.read_file(&entry.path()) {
                if preset_file(&name) == file {
                    names.insert(name);
                }
            }
        }
        names.into_iter().take(MAX_PRESETS).collect()
    }

    /// Reads one preset by name.
    pub fn read_preset(&self, name: &str) -> Result<Layout, LayoutError> {
        let name = clean_name(name).ok_or_else(|| LayoutError::Io("the preset name is not valid".into()))?;
        match self.read_file(&self.dir.join(preset_file(&name)))? {
            Some((_, layout)) => Ok(layout),
            None => Err(LayoutError::Io(format!("the preset \"{name}\" does not exist"))),
        }
    }

    /// Writes a preset. A name that exists is replaced. The count of presets is bounded.
    pub fn write_preset(&self, name: &str, layout: &Layout) -> Result<(), LayoutError> {
        let name = clean_name(name).ok_or_else(|| LayoutError::Io(format!("a preset name has 1 to {MAX_NAME_CHARS} characters without control characters")))?;
        let existing = self.list_presets();
        if !existing.contains(&name) && existing.len() >= MAX_PRESETS {
            return Err(LayoutError::Io(format!("at most {MAX_PRESETS} presets can be kept; delete one first")));
        }
        self.write_file(&preset_file(&name), Some(&name), layout)
    }

    /// Deletes a preset. A missing preset is not an error.
    pub fn delete_preset(&self, name: &str) -> Result<(), LayoutError> {
        let name = clean_name(name).ok_or_else(|| LayoutError::Io("the preset name is not valid".into()))?;
        match std::fs::remove_file(self.dir.join(preset_file(&name))) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(LayoutError::Io(format!("cannot delete the preset: {error}"))),
        }
    }

    fn read_file(&self, path: &Path) -> Result<Option<(Option<String>, Layout)>, LayoutError> {
        let file = match orr_fsid::open_file(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(LayoutError::Io(format!("cannot open the layout file: {error}"))),
        };
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes).map_err(|error| LayoutError::Io(format!("cannot read the layout file: {error}")))?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(LayoutError::TooLarge);
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|error| LayoutError::Corrupt(error.to_string().chars().take(80).collect()))?;
        if value.get("format").and_then(Value::as_str) != Some(FORMAT) {
            return Err(corrupt("this is not an editor layout file"));
        }
        let version = value.get("version").and_then(Value::as_u64).ok_or_else(|| corrupt("no version"))?;
        if version != VERSION {
            return Err(LayoutError::Version(version));
        }
        let name = match value.get("name") {
            None | Some(Value::Null) => None,
            Some(name) => Some(name.as_str().and_then(clean_name).ok_or_else(|| corrupt("the preset name is not valid"))?),
        };
        let layout = Layout::from_value(value.get("layout").ok_or_else(|| corrupt("no layout"))?)?;
        Ok(Some((name, layout)))
    }

    /// Writes `file` through a temporary file in the same folder, then renames it over the
    /// old file. When any step fails, the old file stays and the temporary file is removed.
    fn write_file(&self, file: &str, name: Option<&str>, layout: &Layout) -> Result<(), LayoutError> {
        let io = |what: &str| {
            let what = what.to_owned();
            move |error: std::io::Error| LayoutError::Io(format!("cannot {what}: {error}"))
        };
        layout.validate()?;
        let mut document = json!({ "format": FORMAT, "version": VERSION, "layout": layout.to_value() });
        if let (Some(name), Some(object)) = (name, document.as_object_mut()) {
            object.insert("name".into(), json!(name));
        }
        let text = serde_json::to_vec_pretty(&document).map_err(|error| LayoutError::Io(error.to_string()))?;
        if text.len() as u64 > MAX_FILE_BYTES {
            return Err(LayoutError::TooLarge);
        }
        std::fs::create_dir_all(&self.dir).map_err(io("create the layout folder"))?;
        orr_fsid::check_real_folders(&self.dir).map_err(io("use the layout folder"))?;
        let target = self.dir.join(file);
        // Each write has its own temporary name, so windows that save at the same time never share one.
        static WRITES: AtomicU64 = AtomicU64::new(0);
        let temp = self.dir.join(format!("{file}.{}-{}.tmp", std::process::id(), WRITES.fetch_add(1, Ordering::Relaxed)));
        let result = (|| {
            let mut handle = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp).map_err(io("create the temporary layout file"))?;
            handle.write_all(&text).map_err(io("write the temporary layout file"))?;
            handle.sync_all().map_err(io("flush the temporary layout file"))?;
            drop(handle);
            // On Windows, a replace is refused while another window opens or replaces the same file.
            // That state ends soon, so try again a few times. The last writer then wins with a whole file.
            let mut attempt = 0_u32;
            loop {
                match std::fs::rename(&temp, &target) {
                    Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied && attempt < RENAME_RETRIES => {
                        attempt += 1;
                        std::thread::sleep(std::time::Duration::from_millis(u64::from(attempt) * 5));
                    }
                    other => break other.map_err(io("replace the layout file")),
                }
            }
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layout_is_valid_and_round_trips() {
        let layout = Layout::default();
        layout.validate().unwrap();
        assert_eq!(Layout::from_value(&layout.to_value()).unwrap(), layout);
    }

    #[test]
    fn docking_keeps_every_panel_once() {
        for panel in Panel::ALL {
            for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
                let mut layout = Layout::default();
                layout.dock(panel, edge);
                layout.validate().unwrap_or_else(|error| panic!("{panel:?} {edge:?}: {error}"));
            }
            for other in Panel::ALL {
                let mut layout = Layout::default();
                layout.tab_with(panel, other);
                layout.validate().unwrap_or_else(|error| panic!("{panel:?} with {other:?}: {error}"));
            }
        }
    }

    #[test]
    fn tab_with_groups_two_panels() {
        let mut layout = Layout::default();
        layout.tab_with(Panel::Inspector, Panel::Hierarchy);
        let text = layout.to_value().to_string();
        assert!(text.contains("\"tabs\""), "{text}");
        assert!(layout.is_visible(Panel::Inspector));
    }

    #[test]
    fn viewport_cannot_be_hidden() {
        let mut layout = Layout::default();
        layout.set_visible(Panel::Viewport, false);
        assert!(layout.is_visible(Panel::Viewport));
        layout.set_visible(Panel::Inspector, false);
        assert!(!layout.is_visible(Panel::Inspector));
        layout.validate().unwrap();
    }

    #[test]
    fn bounds_are_enforced() {
        let mut value = Layout::default().to_value();
        // Too deep: wrap the layout in tab groups.
        for _ in 0..MAX_DEPTH {
            value = json!({ "tabs": [value], "active": 0 });
        }
        assert!(matches!(Layout::from_value(&value), Err(LayoutError::Limit(_))));
        // A pane name that this build does not have.
        let value = json!({ "tabs": [{ "pane": "terminal", "visible": true }], "active": 0 });
        assert!(matches!(Layout::from_value(&value), Err(LayoutError::UnknownPanel(_))));
        // A missing panel.
        let value = json!({ "pane": "viewport", "visible": true });
        assert!(matches!(Layout::from_value(&value), Err(LayoutError::MissingPanel(_))));
        // A share of zero.
        let mut value = Layout::default().to_value();
        value["children"][0]["share"] = json!(0);
        assert!(matches!(Layout::from_value(&value), Err(LayoutError::Limit(_))));
    }

    #[test]
    fn store_round_trips_and_refuses_bad_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = LayoutStore::at(dir.path().join("layouts"));
        assert_eq!(store.read_last(), Ok(None));
        let mut layout = Layout::default();
        layout.dock(Panel::Inspector, Edge::Left);
        store.write_last(&layout).unwrap();
        assert_eq!(store.read_last(), Ok(Some(layout.clone())));
        store.write_preset("Wide view", &layout).unwrap();
        assert_eq!(store.list_presets(), vec!["Wide view".to_owned()]);
        assert_eq!(store.read_preset("Wide view"), Ok(layout.clone()));
        // A newer version, a damaged file and an oversized file are refused.
        let path = store.dir().join(LAST_FILE);
        std::fs::write(&path, r#"{"format":"orrery-editor-layout","version":2,"layout":{}}"#).unwrap();
        assert_eq!(store.read_last(), Err(LayoutError::Version(2)));
        std::fs::write(&path, "{ not json").unwrap();
        assert!(matches!(store.read_last(), Err(LayoutError::Corrupt(_))));
        std::fs::write(&path, vec![b' '; (MAX_FILE_BYTES + 1) as usize]).unwrap();
        assert_eq!(store.read_last(), Err(LayoutError::TooLarge));
        store.delete_preset("Wide view").unwrap();
        assert!(store.list_presets().is_empty());
    }

    #[test]
    fn preset_names_are_bounded() {
        assert_eq!(clean_name("  Debug  "), Some("Debug".to_owned()));
        assert_eq!(clean_name(""), None);
        assert_eq!(clean_name("a\nb"), None);
        assert_eq!(clean_name(&"x".repeat(MAX_NAME_CHARS + 1)), None);
        let file = preset_file("../../evil");
        assert!(!file.contains('/') && !file.contains('\\') && !file.contains(".."), "{file}");
    }

    #[test]
    fn preset_count_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let store = LayoutStore::at(dir.path());
        for index in 0..MAX_PRESETS {
            store.write_preset(&format!("p{index}"), &Layout::default()).unwrap();
        }
        assert!(store.write_preset("one more", &Layout::default()).is_err());
        store.write_preset("p0", &Layout::default()).unwrap();
    }
}
