//! Text and structural diffs of two scene documents.
//!
//! [`unified_diff`] is a small deterministic line diff (common prefix and
//! suffix trimmed, LCS on the middle, fixed tie-breaking), printed in the
//! unified format. [`ProposalSummary`] is the structural view of the same
//! change: entities added, removed and renamed, components added and removed,
//! and every changed field with its old and new value.

use core::fmt::Write as _;

use orr_reflect::decimal::{fp32_raw_to_decimal, fp_to_decimal};
use orr_reflect::{Guid, Scene, Value};

// ---- unified text diff ----

#[derive(Clone, Copy, PartialEq, Eq)]
enum Edit {
    Keep,
    Del,
    Ins,
}

/// Longest middle (`a_len * b_len` cells) the LCS table is built for; beyond
/// it the middle is reported as one delete and one insert (still a valid diff).
const MAX_LCS_CELLS: usize = 16_000_000;

fn edit_script(a: &[&str], b: &[&str]) -> Vec<Edit> {
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let (a_rest, b_rest) = (&a[prefix..], &b[prefix..]);
    let suffix = a_rest.iter().rev().zip(b_rest.iter().rev()).take_while(|(x, y)| x == y).count();
    let (am, bm) = (&a_rest[..a_rest.len() - suffix], &b_rest[..b_rest.len() - suffix]);

    let mut out = vec![Edit::Keep; prefix];
    let (n, m) = (am.len(), bm.len());
    if n == 0 || m == 0 || (n + 1).saturating_mul(m + 1) > MAX_LCS_CELLS {
        out.extend(std::iter::repeat_n(Edit::Del, n));
        out.extend(std::iter::repeat_n(Edit::Ins, m));
    } else {
        // lcs[i][j] = LCS length of am[i..] and bm[j..].
        let w = m + 1;
        let mut lcs = vec![0u32; (n + 1) * w];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                lcs[i * w + j] =
                    if am[i] == bm[j] { lcs[(i + 1) * w + j + 1] + 1 } else { lcs[(i + 1) * w + j].max(lcs[i * w + j + 1]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n && j < m {
            if am[i] == bm[j] {
                out.push(Edit::Keep);
                i += 1;
                j += 1;
            } else if lcs[(i + 1) * w + j] >= lcs[i * w + j + 1] {
                out.push(Edit::Del);
                i += 1;
            } else {
                out.push(Edit::Ins);
                j += 1;
            }
        }
        out.extend(std::iter::repeat_n(Edit::Del, n - i));
        out.extend(std::iter::repeat_n(Edit::Ins, m - j));
    }
    out.extend(std::iter::repeat_n(Edit::Keep, suffix));
    out
}

/// A unified diff (`--- a_name`, `+++ b_name`, `@@ -l,n +l,n @@` hunks with
/// `context` lines around each change; a count of 1 is written without
/// `,1`). Empty when `a` and `b` have the same lines. Deterministic: the
/// same two texts always give the same diff.
pub fn unified_diff(a: &str, b: &str, a_name: &str, b_name: &str, context: usize) -> String {
    let (al, bl): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    let script = edit_script(&al, &bl);
    if script.iter().all(|e| *e == Edit::Keep) {
        return String::new();
    }
    // Line index into a and b before each script entry.
    let mut pos = Vec::with_capacity(script.len() + 1);
    let (mut ai, mut bi) = (0usize, 0usize);
    for e in &script {
        pos.push((ai, bi));
        match e {
            Edit::Keep => {
                ai += 1;
                bi += 1;
            }
            Edit::Del => ai += 1,
            Edit::Ins => bi += 1,
        }
    }
    pos.push((ai, bi));

    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for (i, e) in script.iter().enumerate() {
        if *e == Edit::Keep {
            continue;
        }
        let (lo, hi) = (i.saturating_sub(context), (i + context + 1).min(script.len()));
        match ranges.last_mut() {
            Some(last) if lo <= last.1 => last.1 = last.1.max(hi),
            _ => ranges.push((lo, hi)),
        }
    }

    let mut out = String::new();
    let _ = writeln!(out, "--- {a_name}");
    let _ = writeln!(out, "+++ {b_name}");
    for (lo, hi) in ranges {
        let ((a0, b0), (a1, b1)) = (pos[lo], pos[hi]);
        let range = |start: usize, count: usize| {
            let first = if count == 0 { start } else { start + 1 };
            if count == 1 {
                format!("{first}")
            } else {
                format!("{first},{count}")
            }
        };
        let _ = writeln!(out, "@@ -{} +{} @@", range(a0, a1 - a0), range(b0, b1 - b0));
        for k in lo..hi {
            let (ai, bi) = pos[k];
            match script[k] {
                Edit::Keep => {
                    let _ = writeln!(out, " {}", al[ai]);
                }
                Edit::Del => {
                    let _ = writeln!(out, "-{}", al[ai]);
                }
                Edit::Ins => {
                    let _ = writeln!(out, "+{}", bl[bi]);
                }
            }
        }
    }
    out
}

// ---- values as text ----

/// A compact one-line text of a value: numbers as decimals, vectors as
/// `[x, y]`, structs as `{a: 1, b: 2}`, entity references as their GUID.
pub fn format_value(v: &Value) -> String {
    match v {
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Fixed(f) => fp_to_decimal(*f),
        Value::Fixed32(f) => fp32_raw_to_decimal(f.0),
        Value::Vec2(p) => format!("[{}, {}]", fp_to_decimal(p.x), fp_to_decimal(p.y)),
        Value::Vec3(p) => format!("[{}, {}, {}]", fp_to_decimal(p.x), fp_to_decimal(p.y), fp_to_decimal(p.z)),
        Value::Entity(e) => format!("entity {}v{}", e.index, e.version),
        Value::EntityGuid(None) => "none".to_string(),
        Value::EntityGuid(Some(g)) => g.clone(),
        Value::Enum(n) => n.clone(),
        Value::Flags(f) if f.is_empty() => "[]".to_string(),
        Value::Flags(f) => f.join("|"),
        Value::Array(a) => format!("[{}]", a.iter().map(format_value).collect::<Vec<_>>().join(", ")),
        Value::Struct(f) => format!("{{{}}}", fields_text(f)),
        Value::Variant(n, f) if f.is_empty() => n.clone(),
        Value::Variant(n, f) => format!("{n}{{{}}}", fields_text(f)),
    }
}

fn fields_text(f: &[(String, Value)]) -> String {
    f.iter().map(|(n, v)| format!("{n}: {}", format_value(v))).collect::<Vec<_>>().join(", ")
}

// ---- structural summary ----

/// An entity in a summary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityRef {
    /// Scene GUID.
    pub guid: Guid,
    /// Display name.
    pub name: Option<String>,
}

impl EntityRef {
    fn label(&self) -> String {
        match &self.name {
            Some(n) => format!("{n} ({})", self.guid),
            None => self.guid.to_string(),
        }
    }
}

/// An entity whose display name changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Renamed {
    /// Scene GUID.
    pub guid: Guid,
    /// Name before.
    pub old: Option<String>,
    /// Name after.
    pub new: Option<String>,
}

/// One changed field.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldChange {
    /// The entity, or `None` for a singleton.
    pub entity: Option<Guid>,
    /// Component or singleton type name.
    pub component: String,
    /// Field path (`pos.x`); empty when the whole value changed shape.
    pub path: String,
    /// Value before (scene form).
    pub old: Value,
    /// Value after (scene form).
    pub new: Value,
}

impl FieldChange {
    /// `e_00000005 orr_physics::Body.pos: [0, 5] -> [0, 9]`.
    pub fn describe(&self) -> String {
        let who = self.entity.as_ref().map_or_else(|| "singleton".to_string(), |g| g.to_string());
        let dot = if self.path.is_empty() { String::new() } else { format!(".{}", self.path) };
        format!("{who} {}{dot}: {} -> {}", self.component, format_value(&self.old), format_value(&self.new))
    }
}

/// What a proposal changes, structurally. Every list is sorted (entities by
/// GUID, then component name, then path), so the summary is deterministic.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProposalSummary {
    /// Entities that exist only in the staged scene.
    pub entities_added: Vec<EntityRef>,
    /// Entities that exist only in the base scene.
    pub entities_removed: Vec<EntityRef>,
    /// Entities in both whose display name differs.
    pub entities_renamed: Vec<Renamed>,
    /// `(entity, component type)` added to entities that exist in both.
    pub components_added: Vec<(Guid, String)>,
    /// `(entity, component type)` removed from entities that exist in both.
    pub components_removed: Vec<(Guid, String)>,
    /// Changed fields of components and singletons that exist in both.
    pub fields_changed: Vec<FieldChange>,
    /// Singleton types the staged scene has a value for and the base has not.
    pub singletons_added: Vec<String>,
    /// Singleton types the base scene has a value for and the staged has not.
    pub singletons_removed: Vec<String>,
}

impl ProposalSummary {
    /// True if base and staged are the same document.
    pub fn is_empty(&self) -> bool {
        *self == ProposalSummary::default()
    }

    /// One human-readable line per change, in a fixed order.
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        for e in &self.entities_added {
            out.push(format!("+ entity {}", e.label()));
        }
        for e in &self.entities_removed {
            out.push(format!("- entity {}", e.label()));
        }
        for r in &self.entities_renamed {
            let n = |o: &Option<String>| o.clone().unwrap_or_else(|| "(none)".into());
            out.push(format!("~ rename {}: {} -> {}", r.guid, n(&r.old), n(&r.new)));
        }
        for (g, c) in &self.components_added {
            out.push(format!("+ component {c} on {g}"));
        }
        for (g, c) in &self.components_removed {
            out.push(format!("- component {c} on {g}"));
        }
        for s in &self.singletons_added {
            out.push(format!("+ singleton {s}"));
        }
        for s in &self.singletons_removed {
            out.push(format!("- singleton {s}"));
        }
        for f in &self.fields_changed {
            out.push(format!("~ {}", f.describe()));
        }
        out
    }
}

/// Compares two scenes (the base and the staged one of a proposal).
pub fn summarize(base: &Scene, staged: &Scene) -> ProposalSummary {
    let mut s = ProposalSummary::default();
    for (guid, ent) in &staged.entities {
        if !base.entities.contains_key(guid) {
            s.entities_added.push(EntityRef { guid: guid.clone(), name: ent.name.clone() });
        }
    }
    for (guid, old) in &base.entities {
        let Some(new) = staged.entities.get(guid) else {
            s.entities_removed.push(EntityRef { guid: guid.clone(), name: old.name.clone() });
            continue;
        };
        if old.name != new.name {
            s.entities_renamed.push(Renamed { guid: guid.clone(), old: old.name.clone(), new: new.name.clone() });
        }
        for (name, ov) in &old.components {
            match new.components.iter().find(|(n, _)| n == name) {
                None => s.components_removed.push((guid.clone(), name.clone())),
                Some((_, nv)) => value_changes(Some(guid), name, "", ov, nv, &mut s.fields_changed),
            }
        }
        for (name, _) in &new.components {
            if !old.components.iter().any(|(n, _)| n == name) {
                s.components_added.push((guid.clone(), name.clone()));
            }
        }
    }
    for (name, ov) in &base.singletons {
        match staged.singletons.iter().find(|(n, _)| n == name) {
            None => s.singletons_removed.push(name.clone()),
            Some((_, nv)) => value_changes(None, name, "", ov, nv, &mut s.fields_changed),
        }
    }
    for (name, _) in &staged.singletons {
        if !base.singletons.iter().any(|(n, _)| n == name) {
            s.singletons_added.push(name.clone());
        }
    }
    s.components_added.sort();
    s.components_removed.sort();
    s.singletons_added.sort();
    s.singletons_removed.sort();
    s
}

/// Pushes the leaf differences of two values of one component or singleton.
fn value_changes(entity: Option<&Guid>, component: &str, path: &str, old: &Value, new: &Value, out: &mut Vec<FieldChange>) {
    if old == new {
        return;
    }
    let join = |seg: &str| if path.is_empty() { seg.to_string() } else { format!("{path}.{seg}") };
    let same_names = |a: &[(String, Value)], b: &[(String, Value)]| a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.0 == y.0);
    match (old, new) {
        (Value::Struct(a), Value::Struct(b)) if same_names(a, b) => {
            for ((n, x), (_, y)) in a.iter().zip(b) {
                value_changes(entity, component, &join(n), x, y, out);
            }
        }
        (Value::Variant(na, a), Value::Variant(nb, b)) if na == nb && same_names(a, b) => {
            for ((n, x), (_, y)) in a.iter().zip(b) {
                value_changes(entity, component, &join(n), x, y, out);
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                let p = if path.is_empty() { format!("[{i}]") } else { format!("{path}[{i}]") };
                value_changes(entity, component, &p, x, y, out);
            }
        }
        _ => out.push(FieldChange {
            entity: entity.cloned(),
            component: component.to_string(),
            path: path.to_string(),
            old: old.clone(),
            new: new.clone(),
        }),
    }
}
