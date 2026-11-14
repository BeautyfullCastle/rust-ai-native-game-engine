//! The inspector section of the `orr_gfx::PostProcess` singleton: exposure,
//! tone mapping and the ordered effect list. Each effect shows its numbers by
//! name; effects can be added, switched on and off, moved up and down and
//! removed. Edits go through the ordinary field events, so they are saved with
//! the scene and undone like every other edit.
#![allow(clippy::float_arithmetic)]

use crate::inspector::{show_component, InspEvent};
use egui::{RichText, Ui};
use orr_reflect::{decimal, TypeDesc, Value};

/// Effect kind names in `orr_gfx` kind order; index 0 is an empty slot.
const KINDS: [&str; 13] = [
    "none", "bloom", "light_shafts", "auto_exposure", "color_grade", "vignette", "film_grain", "ssao", "toon", "outline", "fxaa", "upscale", "taa",
];

#[derive(Clone)]
struct Effect {
    kind: u32,
    enabled: bool,
    params: [f32; 4],
}

struct Stored {
    exposure: f32,
    tone_mapping: String,
    effects: Vec<Effect>,
}

fn number(value: &Value) -> f32 {
    if let Value::Fixed(f) = value { decimal::fp_to_decimal(*f).parse().unwrap_or(0.0) } else { 0.0 }
}

fn read(value: &Value) -> Option<Stored> {
    let Value::Struct(fields) = value else { return None };
    let field = |name: &str| fields.iter().find(|(n, _)| n == name).map(|(_, v)| v);
    let Some(Value::Enum(tone_mapping)) = field("tone_mapping") else { return None };
    let Some(Value::Array(items)) = field("effects") else { return None };
    let effects = items
        .iter()
        .map(|item| {
            let Value::Struct(f) = item else { return None };
            let get = |name: &str| f.iter().find(|(n, _)| n == name).map(|(_, v)| v);
            let Some(Value::Enum(kind)) = get("kind") else { return None };
            let Some(Value::Bool(enabled)) = get("enabled") else { return None };
            let Some(Value::Array(params)) = get("params") else { return None };
            let mut numbers = [0.0; 4];
            for (slot, p) in numbers.iter_mut().zip(params) {
                *slot = number(p);
            }
            Some(Effect { kind: KINDS.iter().position(|k| k == kind)? as u32, enabled: *enabled, params: numbers })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Stored { exposure: number(field("exposure")?), tone_mapping: tone_mapping.clone(), effects })
}

fn fixed(v: f32) -> Value {
    Value::Fixed(decimal::parse_fp(&format!("{v:.4}")).unwrap_or(orr_fp::FP::ZERO))
}

/// Events that write `effect` into slot `i`.
fn write(i: usize, effect: &Effect) -> Vec<InspEvent> {
    let mut events = vec![
        InspEvent::Set { path: format!("effects[{i}].kind"), value: Value::Enum(KINDS[effect.kind as usize].into()) },
        InspEvent::Set { path: format!("effects[{i}].enabled"), value: Value::Bool(effect.enabled) },
    ];
    events.extend((0..4).map(|k| InspEvent::Set { path: format!("effects[{i}].params[{k}]"), value: fixed(effect.params[k]) }));
    events
}

/// An effect of `kind` with its default numbers, switched on.
fn new_effect(kind: u32) -> Effect {
    let mut params = [0.0; 4];
    for (slot, (_, _, _, default)) in params.iter_mut().zip(orr_gfx::effect_params(kind)) {
        *slot = default.to_f32();
    }
    Effect { kind, enabled: true, params }
}

/// Adds the edits of one widget to the open undo step; the step ends when the pointer is released.
fn edit(response: &egui::Response, label: &str, edits: Vec<InspEvent>, open: &mut bool, out: &mut Vec<InspEvent>) {
    if response.changed() && !edits.is_empty() {
        if !*open {
            out.push(InspEvent::Begin(label.into()));
            *open = true;
        }
        out.extend(edits);
    }
}

/// One undo step of its own, for a button or a list choice.
fn single(label: &str, edits: Vec<InspEvent>, open: &mut bool, out: &mut Vec<InspEvent>) {
    if std::mem::take(open) {
        out.push(InspEvent::End);
    }
    out.push(InspEvent::Begin(label.into()));
    out.extend(edits);
    out.push(InspEvent::End);
}

/// Draws the post-process section. `open` is true while an edit of this section is open.
pub fn show(ui: &mut Ui, id: egui::Id, desc: &TypeDesc, value: &Value, open: &mut bool, out: &mut Vec<InspEvent>) {
    contents(ui, id, desc, value, open, out);
    if *open && !ui.input(|i| i.pointer.any_down()) {
        *open = false;
        out.push(InspEvent::End);
    }
}

fn contents(ui: &mut Ui, id: egui::Id, desc: &TypeDesc, value: &Value, open: &mut bool, out: &mut Vec<InspEvent>) {
    let Some(stored) = read(value) else {
        show_component(ui, id, desc, value, out);
        return;
    };
    egui::Grid::new(id.with("display")).num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        ui.label("exposure");
        let mut exposure = stored.exposure;
        let response = ui.add(egui::Slider::new(&mut exposure, 0.01..=16.0).logarithmic(true));
        edit(&response, "Edit exposure", vec![InspEvent::Set { path: "exposure".into(), value: fixed(exposure) }], open, out);
        ui.end_row();
        ui.label("tone mapping");
        egui::ComboBox::from_id_salt(id.with("tone")).selected_text(stored.tone_mapping.as_str()).show_ui(ui, |ui| {
            for name in ["clamp", "aces"] {
                if ui.selectable_label(stored.tone_mapping == name, name).clicked() && stored.tone_mapping != name {
                    single("Change tone mapping", vec![InspEvent::Set { path: "tone_mapping".into(), value: Value::Enum(name.into()) }], open, out);
                }
            }
        });
        ui.end_row();
    });
    ui.add_space(4.0);
    ui.label(RichText::new("Effects, in the order they run").strong());
    let used: Vec<usize> = (0..stored.effects.len()).filter(|i| stored.effects[*i].kind != 0).collect();
    if used.is_empty() {
        ui.weak("No effects. Exposure and tone mapping still apply.");
    }
    for (position, &i) in used.iter().enumerate() {
        let effect = &stored.effects[i];
        let row = id.with(("effect", i));
        ui.horizontal(|ui| {
            let mut enabled = effect.enabled;
            if ui.checkbox(&mut enabled, RichText::new(KINDS[effect.kind as usize]).strong()).changed() {
                single(if enabled { "Turn effect on" } else { "Turn effect off" }, vec![InspEvent::Set { path: format!("effects[{i}].enabled"), value: Value::Bool(enabled) }], open, out);
            }
            let swap = |other: usize, label: &str, open: &mut bool, out: &mut Vec<InspEvent>| {
                let mut edits = write(i, &stored.effects[other]);
                edits.extend(write(other, &stored.effects[i]));
                single(label, edits, open, out);
            };
            if ui.add_enabled(position > 0, egui::Button::new("Up").small()).clicked() {
                swap(used[position - 1], "Move effect up", open, out);
            }
            if ui.add_enabled(position + 1 < used.len(), egui::Button::new("Down").small()).clicked() {
                swap(used[position + 1], "Move effect down", open, out);
            }
            if ui.small_button("Remove").clicked() {
                single("Remove effect", write(i, &Effect { kind: 0, enabled: false, params: [0.0; 4] }), open, out);
            }
        });
        egui::Grid::new(row.with("params")).num_columns(2).spacing([8.0, 2.0]).show(ui, |ui| {
            for (k, (name, low, high, _)) in orr_gfx::effect_params(effect.kind).iter().enumerate() {
                ui.label(format!("  {name}"));
                let mut v = effect.params[k];
                let response = ui.add(egui::Slider::new(&mut v, low.to_f32()..=high.to_f32()));
                edit(&response, "Edit effect", vec![InspEvent::Set { path: format!("effects[{i}].params[{k}]"), value: fixed(v) }], open, out);
                ui.end_row();
            }
        });
    }
    let free = stored.effects.iter().position(|e| e.kind == 0);
    ui.add_enabled_ui(free.is_some(), |ui| {
        egui::ComboBox::from_id_salt(id.with("add")).selected_text("+ Add effect").show_ui(ui, |ui| {
            for (kind, name) in KINDS.iter().enumerate().skip(1) {
                if ui.selectable_label(false, *name).clicked() {
                    if let Some(slot) = free {
                        single("Add effect", write(slot, &new_effect(kind as u32)), open, out);
                    }
                }
            }
        });
    });
    crate::theme::header("Stored fields").id_salt(id.with("raw")).default_open(false).show(ui, |ui| show_component(ui, id.with("raw_fields"), desc, value, out));
}
