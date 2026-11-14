//! The inspector section of `orr_gfx::Material`: pick a surface shader, see
//! whether it compiled and why not, and set its parameters by the names the
//! shader declares. Edits go through the ordinary field events, so they are
//! saved with the scene and undone like every other edit.
#![allow(clippy::float_arithmetic)]

use crate::inspector::{show_component, InspEvent};
use crate::surface_library::SurfaceLibrary;
use egui::{Color32, RichText, Ui};
use orr_reflect::{decimal, TypeDesc, Value};
use orr_render::ParamKind;

/// The material's stored fields, read from its reflected value.
struct Stored {
    shader: String,
    values: Vec<f32>,
    overrides: u32,
}

fn read(value: &Value) -> Option<Stored> {
    let Value::Struct(fields) = value else { return None };
    let field = |name: &str| fields.iter().find(|(n, _)| n == name).map(|(_, v)| v);
    let Some(Value::Text(shader)) = field("shader") else { return None };
    let Some(Value::Array(items)) = field("values") else { return None };
    let Some(Value::Int(overrides)) = field("overrides") else { return None };
    let values = items.iter().map(|v| if let Value::Fixed(f) = v { decimal::fp_to_decimal(*f).parse().unwrap_or(0.0) } else { 0.0 }).collect();
    Some(Stored { shader: shader.clone(), values, overrides: *overrides as u32 })
}

fn fixed(v: f32) -> Value {
    // Four decimals keep scene files short and readable.
    Value::Fixed(decimal::parse_fp(&format!("{v:.4}")).unwrap_or(orr_fp::FP::ZERO))
}

/// Events that set parameter numbers `offset..` to `numbers` and mark them as overridden.
fn set_numbers(offset: usize, numbers: &[f32], overrides: u32) -> Vec<InspEvent> {
    let mut events: Vec<InspEvent> = numbers.iter().enumerate().map(|(k, v)| InspEvent::Set { path: format!("values[{}]", offset + k), value: fixed(*v) }).collect();
    let mask = numbers.iter().enumerate().fold(overrides, |mask, (k, _)| mask | (1 << (offset + k)));
    events.push(InspEvent::Set { path: "overrides".into(), value: Value::Int(i128::from(mask)) });
    events
}

/// Adds the edits of one widget to the open undo step, opening one if needed.
/// The step stays open while a pointer button is held (a drag, the color
/// picker) and ends when it is released, so a whole drag is one undo step.
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

/// Draws the material section. `desc` and `value` are the reflected component.
/// `open` is true while an edit of this section is open; the caller keeps it between frames.
pub fn show(ui: &mut Ui, id: egui::Id, desc: &TypeDesc, value: &Value, library: &mut SurfaceLibrary, open: &mut bool, out: &mut Vec<InspEvent>) {
    contents(ui, id, desc, value, library, open, out);
    // Every way out of the section ends an open edit once the pointer is released.
    if *open && !ui.input(|i| i.pointer.any_down()) {
        *open = false;
        out.push(InspEvent::End);
    }
}

fn contents(ui: &mut Ui, id: egui::Id, desc: &TypeDesc, value: &Value, library: &mut SurfaceLibrary, open: &mut bool, out: &mut Vec<InspEvent>) {
    let Some(stored) = read(value) else {
        show_component(ui, id, desc, value, out);
        return;
    };
    let available = library.available().to_vec();
    ui.horizontal(|ui| {
        ui.label("Shader");
        let shown = if stored.shader.is_empty() { "(none)" } else { stored.shader.as_str() };
        egui::ComboBox::from_id_salt(id.with("shader")).selected_text(shown).show_ui(ui, |ui| {
            for name in &available {
                if ui.selectable_label(*name == stored.shader, name).clicked() && *name != stored.shader {
                    // A new shader starts from its own defaults.
                    let edits = vec![
                        InspEvent::Set { path: "shader".into(), value: Value::Text(name.clone()) },
                        InspEvent::Set { path: "overrides".into(), value: Value::Int(0) },
                    ];
                    single("Change surface shader", edits, open, out);
                }
            }
        });
    });
    if available.is_empty() {
        let place = library.root().map_or_else(|| "a shaders folder next to the saved scene file".to_string(), |root| root.display().to_string());
        ui.weak(format!("No shaders yet. Put .wgsl files in {place}."));
    }
    if stored.shader.is_empty() {
        return;
    }
    let shader = match library.get(&stored.shader).map(|entry| &entry.result) {
        Some(Ok(shader)) => shader,
        Some(Err(error)) => {
            ui.label(RichText::new(error).color(Color32::from_rgb(255, 110, 110)));
            ui.weak("The shape is drawn magenta until the shader compiles. Saving the file reloads it.");
            return;
        }
        None => {
            ui.weak("Loading the shader…");
            return;
        }
    };
    let count = shader.params().len();
    ui.weak(format!("{count} parameter{} · reloads when the file is saved", if count == 1 { "" } else { "s" }));
    let current = |offset: usize, k: usize, default: f32| {
        if stored.overrides & (1 << (offset + k)) != 0 {
            stored.values.get(offset + k).copied().unwrap_or(default)
        } else {
            default
        }
    };
    egui::Grid::new(id.with("params")).num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        for param in shader.params() {
            let n = param.kind.numbers();
            let mut numbers: Vec<f32> = (0..n).map(|k| current(param.offset, k, param.default[k]).clamp(param.min, param.max)).collect();
            ui.label(&param.name);
            ui.horizontal(|ui| match param.kind {
                ParamKind::Color => {
                    let mut rgb = [numbers[0], numbers[1], numbers[2]];
                    let response = ui.color_edit_button_rgb(&mut rgb);
                    numbers.copy_from_slice(&rgb);
                    edit(&response, "Edit material color", set_numbers(param.offset, &numbers, stored.overrides), open, out);
                }
                ParamKind::Float if param.max - param.min <= 1.0e4 => {
                    let response = ui.add(egui::Slider::new(&mut numbers[0], param.min..=param.max));
                    edit(&response, "Edit material value", set_numbers(param.offset, &numbers, stored.overrides), open, out);
                }
                _ => {
                    for (k, number) in numbers.clone().iter().enumerate() {
                        let mut value = *number;
                        let response = ui.add(egui::DragValue::new(&mut value).speed(0.01).range(param.min..=param.max));
                        if response.changed() {
                            numbers[k] = value;
                        }
                        edit(&response, "Edit material value", set_numbers(param.offset, &numbers, stored.overrides), open, out);
                    }
                }
            });
            ui.end_row();
        }
    });
    if stored.overrides != 0 && ui.small_button("Use the shader's defaults").clicked() {
        single("Reset material", vec![InspEvent::Set { path: "overrides".into(), value: Value::Int(0) }], open, out);
    }
    crate::theme::header("Stored fields").id_salt(id.with("raw")).default_open(false).show(ui, |ui| show_component(ui, id.with("raw_fields"), desc, value, out));
}
