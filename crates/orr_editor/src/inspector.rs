//! The generic inspector: widgets generated from `orr_reflect` descriptors.
//!
//! [`show_component`] walks a `TypeDesc` next to the current `Value` and
//! draws one widget per field. It never changes anything itself: user
//! interaction becomes a list of [`InspEvent`]s that the caller applies
//! (edit mode: `EditorDoc::apply`; play mode: recorded debug commands).
//!
//! - Fixed-point fields are exact decimal text. The text is parsed by
//!   `orr_reflect::decimal`, never through `f64`. A small drag handle scrubs
//!   the value; the drag runs in `f64` (view layer only) and its result is
//!   printed as decimal text and parsed the same exact way, so what goes to
//!   the document is always a plain `FP`.
//! - One gesture is one undo step: a drag sends `Begin` at the press,
//!   `Set`s while moving and `End` at the release. A typed value is one `Set`
//!   when the field loses focus (Enter or click away); Escape cancels.
//! - Ints are exact text too. Bools are checkboxes, enums combo boxes,
//!   flags one checkbox per bit, vectors one field per axis, tagged values
//!   (a shape) a kind combo plus the fields of that kind, lists have
//!   add/remove buttons. Ranges are shown as hover text and respected by the
//!   drag; typed values outside them are refused by the document with a message.

use egui::{Id, Sense, Ui};
use orr_fp::{FPVec2, FPVec3, FP};
use orr_fp::FP32;
use orr_reflect::{decimal, range_text, Kind, Range, TypeDesc, Value};

use crate::editor::{fp_of_f64, fp_to_f64};

/// What the user did in the inspector this frame.
#[derive(Clone, Debug, PartialEq)]
pub enum InspEvent {
    /// A gesture (drag) starts; the label names the undo step.
    Begin(String),
    /// Set the field at `path` (relative to the component or singleton).
    Set {
        /// Reflect path (`"pos.x"`, `"shape.kind"`).
        path: String,
        /// The new value.
        value: Value,
    },
    /// The gesture ended.
    End,
    /// Typed text that is not a valid number; nothing was changed.
    Error(String),
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else if name.starts_with('[') {
        format!("{prefix}{name}")
    } else {
        format!("{prefix}.{name}")
    }
}

/// Draws the fields of a component (or singleton) value.
pub fn show_component(ui: &mut Ui, base: Id, desc: &TypeDesc, value: &Value, out: &mut Vec<InspEvent>) {
    match (&desc.kind, value) {
        (Kind::Struct { fields, .. }, Value::Struct(vals)) => {
            for f in fields {
                match vals.iter().find(|(n, _)| n == &f.name) {
                    Some((_, v)) => show_field(ui, base, &f.name, &f.ty, v, &f.name, &f.doc, out),
                    None => {
                        ui.label(format!("{}: missing", f.name));
                    }
                }
            }
        }
        _ => show_field(ui, base, "value", desc, value, "", &desc.doc, out),
    }
}

fn hover_text(doc: &str, ty: &TypeDesc) -> String {
    let range = match &ty.kind {
        Kind::Int { range, .. } => Some((range, false)),
        Kind::Fixed { range } | Kind::Fixed32 { range } | Kind::Vec2 { range } | Kind::Vec3 { range } => Some((range, true)),
        _ => None,
    };
    let mut s = doc.to_string();
    if let Some((r, fixed)) = range {
        if !r.is_open() {
            if !s.is_empty() {
                s.push('\n');
            }
            s.push_str(&format!("range {}", range_text(r, fixed)));
        }
    }
    s
}

#[allow(clippy::too_many_arguments)]
fn show_field(ui: &mut Ui, base: Id, label: &str, ty: &TypeDesc, value: &Value, path: &str, doc: &str, out: &mut Vec<InspEvent>) {
    let id = base.with(path);
    let hover = hover_text(doc, ty);
    match (&ty.kind, value) {
        (Kind::Struct { fields, .. }, Value::Struct(vals)) => {
            egui::CollapsingHeader::new(label).id_salt(id).default_open(true).show(ui, |ui| {
                for f in fields {
                    if let Some((_, v)) = vals.iter().find(|(n, _)| n == &f.name) {
                        show_field(ui, base, &f.name, &f.ty, v, &join(path, &f.name), &f.doc, out);
                    }
                }
            });
        }
        (Kind::Tagged(t), Value::Variant(kind, vals)) => {
            egui::CollapsingHeader::new(label).id_salt(id).default_open(true).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("kind");
                    egui::ComboBox::from_id_salt(id.with("kind")).selected_text(kind.as_str()).show_ui(ui, |ui| {
                        for v in &t.variants {
                            if ui.selectable_label(&v.name == kind, &v.name).on_hover_text(&v.doc).clicked() && &v.name != kind {
                                out.push(InspEvent::Set { path: join(path, "kind"), value: Value::Enum(v.name.clone()) });
                            }
                        }
                    });
                });
                if let Some(var) = t.variants.iter().find(|v| &v.name == kind) {
                    for f in &var.fields {
                        if let Some((_, v)) = vals.iter().find(|(n, _)| n == &f.name) {
                            show_field(ui, base, &f.name, &f.ty, v, &join(path, &f.name), &f.doc, out);
                        }
                    }
                }
            });
        }
        (Kind::Array { elem, .. }, Value::Array(items)) => {
            egui::CollapsingHeader::new(format!("{label} [{}]", items.len())).id_salt(id).default_open(true).show(ui, |ui| {
                for (i, item) in items.iter().enumerate() {
                    let name = format!("[{i}]");
                    show_field(ui, base, &name, elem, item, &join(path, &name), "", out);
                }
            });
        }
        (Kind::List { elem, min, max }, Value::Array(items)) => {
            egui::CollapsingHeader::new(format!("{label} [{}]", items.len())).id_salt(id).default_open(true).show(ui, |ui| {
                for (i, item) in items.iter().enumerate() {
                    let name = format!("[{i}]");
                    show_field(ui, base, &name, elem, item, &join(path, &name), "", out);
                }
                ui.horizontal(|ui| {
                    if ui.add_enabled(items.len() < *max, egui::Button::new("+ add")).clicked() {
                        let mut next = items.clone();
                        next.push(items.last().cloned().unwrap_or_else(|| zero_value(elem)));
                        out.push(InspEvent::Set { path: path.to_string(), value: Value::Array(next) });
                    }
                    if ui.add_enabled(items.len() > *min, egui::Button::new("- remove last")).clicked() {
                        let mut next = items.clone();
                        next.pop();
                        out.push(InspEvent::Set { path: path.to_string(), value: Value::Array(next) });
                    }
                });
            });
        }
        _ => {
            ui.horizontal(|ui| {
                ui.label(label).on_hover_text(&hover);
                scalar(ui, id, ty, value, path, label, &hover, out);
            });
        }
    }
}

/// A zero value of a type, for a new list element.
pub(crate) fn zero_value(ty: &TypeDesc) -> Value {
    match &ty.kind {
        Kind::Bool { .. } => Value::Bool(false),
        Kind::Int { range, .. } => Value::Int(range.min.map_or(0, |m| m.max(0))),
        Kind::Fixed { .. } => Value::Fixed(FP::ZERO),
        Kind::Fixed32 { .. } => Value::Fixed32(FP32::from_raw(0)),
        Kind::Vec2 { .. } => Value::Vec2(FPVec2::ZERO),
        Kind::Vec3 { .. } => Value::Vec3(FPVec3::ZERO),
        Kind::Entity => Value::EntityGuid(None),
        Kind::Enum { variants, .. } => Value::Enum(variants.first().map(|(n, _)| n.clone()).unwrap_or_default()),
        Kind::Flags { .. } => Value::Flags(Vec::new()),
        Kind::Array { elem, len } => Value::Array((0..*len).map(|_| zero_value(elem)).collect()),
        Kind::List { .. } => Value::Array(Vec::new()),
        Kind::Struct { fields, .. } => Value::Struct(fields.iter().map(|f| (f.name.clone(), zero_value(&f.ty))).collect()),
        Kind::Tagged(t) => match t.variants.first() {
            Some(v) => Value::Variant(v.name.clone(), v.default.clone()),
            None => Value::Variant(String::new(), Vec::new()),
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn scalar(ui: &mut Ui, id: Id, ty: &TypeDesc, value: &Value, path: &str, label: &str, hover: &str, out: &mut Vec<InspEvent>) {
    match (&ty.kind, value) {
        (Kind::Bool { .. }, Value::Bool(b)) => {
            let mut v = *b;
            if ui.checkbox(&mut v, "").on_hover_text(hover).changed() {
                out.push(InspEvent::Set { path: path.to_string(), value: Value::Bool(v) });
            }
        }
        (Kind::Int { .. }, Value::Int(i)) => {
            let canonical = decimal::int_to_decimal(*i);
            if let Some(text) = number_field(ui, id, &canonical, hover, 120.0) {
                match decimal::parse_int(&text) {
                    Ok(v) => out.push(InspEvent::Set { path: path.to_string(), value: Value::Int(v) }),
                    Err(_) => out.push(InspEvent::Error(format!("{label}: '{text}' is not a plain integer"))),
                }
            }
        }
        (Kind::Fixed { range }, Value::Fixed(f)) => {
            fixed_field(ui, id, path, label, Num::Fp, f.raw(), range, hover, out);
        }
        (Kind::Fixed32 { range }, Value::Fixed32(f)) => {
            fixed_field(ui, id, path, label, Num::Fp32, i64::from(f.raw()), range, hover, out);
        }
        (Kind::Vec2 { range }, Value::Vec2(v)) => {
            for (axis, raw) in [("x", v.x.raw()), ("y", v.y.raw())] {
                ui.label(axis);
                fixed_field(ui, id.with(axis), &join(path, axis), &format!("{label}.{axis}"), Num::Fp, raw, range, hover, out);
            }
        }
        (Kind::Vec3 { range }, Value::Vec3(v)) => {
            for (axis, raw) in [("x", v.x.raw()), ("y", v.y.raw()), ("z", v.z.raw())] {
                ui.label(axis);
                fixed_field(ui, id.with(axis), &join(path, axis), &format!("{label}.{axis}"), Num::Fp, raw, range, hover, out);
            }
        }
        (Kind::Enum { variants, .. }, Value::Enum(cur)) => {
            egui::ComboBox::from_id_salt(id).selected_text(cur.as_str()).show_ui(ui, |ui| {
                for (name, _) in variants {
                    if ui.selectable_label(name == cur, name).clicked() && name != cur {
                        out.push(InspEvent::Set { path: path.to_string(), value: Value::Enum(name.clone()) });
                    }
                }
            });
        }
        (Kind::Flags { bits, .. }, Value::Flags(set)) => {
            for (name, _) in bits {
                let mut on = set.contains(name);
                if ui.checkbox(&mut on, name).changed() {
                    let next: Vec<String> = bits.iter().map(|(n, _)| n).filter(|n| if *n == name { on } else { set.contains(*n) }).cloned().collect();
                    out.push(InspEvent::Set { path: path.to_string(), value: Value::Flags(next) });
                }
            }
            if bits.is_empty() {
                ui.weak("(no flags)");
            }
        }
        (Kind::Entity, Value::EntityGuid(g)) => {
            ui.weak(g.as_deref().unwrap_or("none"));
        }
        (Kind::Entity, Value::Entity(e)) => {
            ui.weak(format!("entity {}v{}", e.index, e.version));
        }
        _ => {
            ui.colored_label(ui.visuals().warn_fg_color, format!("cannot show a {}", value.kind_name()));
        }
    }
}

/// Which fixed-point width a field has.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Num {
    Fp,
    Fp32,
}

impl Num {
    fn text(self, raw: i64) -> String {
        match self {
            Num::Fp => decimal::fp_to_decimal(FP::from_raw(raw)),
            Num::Fp32 => decimal::fp32_raw_to_decimal(raw as i32),
        }
    }

    /// Exact text to value. `Err` carries a message.
    fn parse(self, text: &str) -> Result<Value, String> {
        let bad = |e: decimal::NumberError| match e {
            decimal::NumberError::NotDecimal => format!("'{text}' is not a plain decimal number like -12.5"),
            decimal::NumberError::Overflow => format!("'{text}' does not fit"),
        };
        match self {
            Num::Fp => decimal::parse_fp(text).map(Value::Fixed).map_err(bad),
            Num::Fp32 => decimal::parse_fp32_raw(text).map(|r| Value::Fixed32(FP32::from_raw(r))).map_err(bad),
        }
    }

    fn value_of_raw(self, raw: i64) -> Option<Value> {
        match self {
            Num::Fp => Some(Value::Fixed(FP::from_raw(raw))),
            Num::Fp32 => i32::try_from(raw).ok().map(|r| Value::Fixed32(FP32::from_raw(r))),
        }
    }
}

/// A text field that holds its text while it has focus and reports the text
/// once when it loses focus with a different value. Escape cancels.
fn number_field(ui: &mut Ui, id: Id, canonical: &str, hover: &str, width: f32) -> Option<String> {
    let edit_id = id.with("text");
    let focused = ui.memory(|m| m.has_focus(edit_id));
    let mut text: String = if focused { ui.data(|d| d.get_temp::<String>(edit_id)).unwrap_or_else(|| canonical.to_string()) } else { canonical.to_string() };
    let resp = ui.add(egui::TextEdit::singleline(&mut text).id(edit_id).desired_width(width).clip_text(true)).on_hover_text(hover);
    if resp.changed() {
        ui.data_mut(|d| d.insert_temp(edit_id, text.clone()));
    }
    if resp.lost_focus() {
        ui.data_mut(|d| d.remove_temp::<String>(edit_id));
        let cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if !cancelled && text != canonical {
            return Some(text);
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn fixed_field(ui: &mut Ui, id: Id, path: &str, label: &str, num: Num, raw: i64, range: &Range, hover: &str, out: &mut Vec<InspEvent>) {
    if let Some(text) = number_field(ui, id, &num.text(raw), hover, 84.0) {
        match num.parse(&text) {
            Ok(value) => out.push(InspEvent::Set { path: path.to_string(), value }),
            Err(e) => out.push(InspEvent::Error(format!("{label}: {e}"))),
        }
    }
    // Scrub handle: view-layer f64, turned into exact decimal text and parsed.
    let handle = ui.add(egui::Label::new("\u{2194}").sense(Sense::drag())).on_hover_cursor(egui::CursorIcon::ResizeHorizontal).on_hover_text("drag to change");
    let drag_id = id.with("scrub");
    if handle.drag_started() {
        ui.data_mut(|d| d.insert_temp(drag_id, (raw, 0.0f64)));
        out.push(InspEvent::Begin(format!("drag {label}")));
    }
    if handle.dragged() {
        let (base, acc): (i64, f64) = ui.data(|d| d.get_temp(drag_id)).unwrap_or((raw, 0.0));
        let acc = acc + f64::from(handle.drag_delta().x);
        ui.data_mut(|d| d.insert_temp(drag_id, (base, acc)));
        let base_f = fp_to_f64(FP::from_raw(base));
        let speed = (base_f.abs() * 0.01).max(0.05);
        if let Some(fp) = fp_of_f64(base_f + acc * speed) {
            let mut r = i128::from(fp.raw());
            if let Some(m) = range.min {
                r = r.max(m);
            }
            if let Some(m) = range.max {
                r = r.min(m);
            }
            if let Some(value) = i64::try_from(r).ok().and_then(|r| num.value_of_raw(r)) {
                out.push(InspEvent::Set { path: path.to_string(), value });
            }
        }
    }
    if handle.drag_stopped() {
        ui.data_mut(|d| d.remove_temp::<(i64, f64)>(drag_id));
        out.push(InspEvent::End);
    }
}
