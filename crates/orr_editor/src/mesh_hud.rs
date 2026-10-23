use orr_navigation::NavigationStatus;
use orr_remote::mesh_navigation::MeshView;
use serde::{Deserialize, Deserializer, Serialize};

pub(crate) const MAX_BYTES: usize = 16 * 1024;
const IDENTITY: &str = "ORR_MESH_HUD_V1";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Document {
    pub(crate) enabled: bool,
    pub(crate) title: String,
    pub(crate) idle_text: String,
    pub(crate) moving_text: String,
    pub(crate) arrived_text: String,
    pub(crate) show_tick: bool,
    pub(crate) font_size: f32,
    pub(crate) text_color: [u8; 4],
    pub(crate) background_color: [u8; 4],
}

impl Default for Document {
    fn default() -> Self {
        Self {
            enabled: true,
            title: "Mesh adventure".into(),
            idle_text: "Choose a destination".into(),
            moving_text: "Moving to destination".into(),
            arrived_text: "Destination reached".into(),
            show_tick: true,
            font_size: 18.0,
            text_color: [255, 255, 255, 255],
            background_color: [12, 16, 24, 220],
        }
    }
}

fn deserialize_object<'de, DeserializerType, ValueType>(
    deserializer: DeserializerType,
) -> Result<ValueType, DeserializerType::Error>
where
    DeserializerType: Deserializer<'de>,
    ValueType: Deserialize<'de>,
{
    struct Object<ValueType>(std::marker::PhantomData<ValueType>);

    impl<'de, ValueType: Deserialize<'de>> serde::de::Visitor<'de> for Object<ValueType> {
        type Value = ValueType;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a mesh HUD JSON object")
        }

        fn visit_map<MapAccessType: serde::de::MapAccess<'de>>(
            self,
            map: MapAccessType,
        ) -> Result<ValueType, MapAccessType::Error> {
            ValueType::deserialize(serde::de::value::MapAccessDeserializer::new(map))
        }
    }

    deserializer.deserialize_map(Object(std::marker::PhantomData))
}

impl<'de> Deserialize<'de> for Document {
    fn deserialize<DeserializerType: Deserializer<'de>>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            enabled: bool,
            title: String,
            idle_text: String,
            moving_text: String,
            arrived_text: String,
            show_tick: bool,
            font_size: f32,
            text_color: [u8; 4],
            background_color: [u8; 4],
        }

        let wire: Wire = deserialize_object(deserializer)?;
        Ok(Self {
            enabled: wire.enabled,
            title: wire.title,
            idle_text: wire.idle_text,
            moving_text: wire.moving_text,
            arrived_text: wire.arrived_text,
            show_tick: wire.show_tick,
            font_size: wire.font_size,
            text_color: wire.text_color,
            background_color: wire.background_color,
        })
    }
}

#[derive(Serialize)]
struct BoundDocument<'a> {
    schema: u32,
    marker: &'static str,
    scene_filename: &'a str,
    graph_revision: [u8; 32],
    hud: &'a Document,
}

impl Document {
    pub(crate) fn validate(&self) -> Result<(), String> {
        validate_text("title", &self.title, 80)?;
        validate_text("idle_text", &self.idle_text, 160)?;
        validate_text("moving_text", &self.moving_text, 160)?;
        validate_text("arrived_text", &self.arrived_text, 160)?;
        if !self.font_size.is_finite() || !(12.0..=48.0).contains(&self.font_size) {
            return Err("mesh HUD font size must be finite and in 12..=48".into());
        }
        Ok(())
    }

    pub(crate) fn encode(
        &self,
        scene_filename: &str,
        graph_revision: [u8; 32],
    ) -> Result<Vec<u8>, String> {
        self.validate()?;
        validate_scene_filename(scene_filename)?;
        let mut bytes = serde_json::to_vec_pretty(&BoundDocument {
            schema: 1,
            marker: IDENTITY,
            scene_filename,
            graph_revision,
            hud: self,
        })
        .map_err(|error| format!("serialize mesh HUD: {error}"))?;
        bytes.push(b'\n');
        if bytes.len() > MAX_BYTES {
            return Err(format!("mesh HUD exceeds {MAX_BYTES} bytes"));
        }
        Ok(bytes)
    }

    pub(crate) fn decode(
        bytes: &[u8],
        scene_filename: &str,
        graph_revision: [u8; 32],
    ) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err(format!("mesh HUD exceeds {MAX_BYTES} bytes"));
        }
        validate_scene_filename(scene_filename)?;
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            schema: u32,
            marker: String,
            scene_filename: String,
            graph_revision: [u8; 32],
            hud: Document,
        }

        let mut deserializer = serde_json::Deserializer::from_slice(bytes);
        let bound: Wire = deserialize_object(&mut deserializer)
            .map_err(|error| format!("mesh HUD JSON: {error}"))?;
        deserializer
            .end()
            .map_err(|error| format!("mesh HUD JSON: {error}"))?;
        if bound.schema != 1
            || bound.marker != IDENTITY
            || bound.scene_filename != scene_filename
            || bound.graph_revision != graph_revision
        {
            return Err("mesh HUD belongs to another scene or graph revision".into());
        }
        bound.hud.validate()?;
        Ok(bound.hud)
    }

    pub(crate) fn status_text(&self, view: &MeshView) -> &str {
        match view.agent.status {
            NavigationStatus::Stopped => &self.idle_text,
            NavigationStatus::Moving if view.agent.spec.distance_per_tick > orr_fp::FP::ZERO => {
                &self.moving_text
            }
            NavigationStatus::Moving => &self.idle_text,
            NavigationStatus::Arrived => &self.arrived_text,
        }
    }

    pub(crate) fn draw(&self, ui: &mut egui::Ui, viewport: egui::Rect, view: &MeshView, tick: u64) {
        if !self.enabled || viewport.width() <= 0.0 || viewport.height() <= 0.0 {
            return;
        }
        let status = self.status_text(view);
        let text = if self.show_tick {
            format!("{}\n{}\nTick {tick}", self.title, status)
        } else {
            format!("{}\n{}", self.title, status)
        };
        let font_id = egui::FontId::proportional(self.font_size);
        let text_color = egui::Color32::from_rgba_unmultiplied(
            self.text_color[0],
            self.text_color[1],
            self.text_color[2],
            self.text_color[3],
        );
        let available_width = (viewport.width() - HUD_MARGIN * 2.0).min(MAX_HUD_WIDTH);
        let wrap_width = available_width - HUD_PADDING * 2.0;
        if wrap_width <= 0.0 {
            return;
        }
        let galley = ui.painter().layout(text, font_id, text_color, wrap_width);
        let Some(rectangle) = hud_rect(viewport, galley.size()) else {
            return;
        };
        let clip_rect = rectangle.intersect(viewport);
        let painter = ui.painter().with_clip_rect(clip_rect);
        painter.rect_filled(
            rectangle,
            4.0,
            egui::Color32::from_rgba_unmultiplied(
                self.background_color[0],
                self.background_color[1],
                self.background_color[2],
                self.background_color[3],
            ),
        );
        painter.galley(
            rectangle.min + egui::vec2(HUD_PADDING, HUD_PADDING),
            galley,
            text_color,
        );
        let mut hud_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rectangle)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        hud_ui.set_clip_rect(clip_rect);
        hud_ui.add(egui::Label::new(
            egui::RichText::new(&self.title).color(egui::Color32::TRANSPARENT),
        ));
        hud_ui.add(egui::Label::new(
            egui::RichText::new(status).color(egui::Color32::TRANSPARENT),
        ));
        if self.show_tick {
            hud_ui.add(egui::Label::new(
                egui::RichText::new(format!("Tick {tick}")).color(egui::Color32::TRANSPARENT),
            ));
        }
    }
}

const MAX_HUD_WIDTH: f32 = 420.0;
const HUD_MARGIN: f32 = 8.0;
const HUD_PADDING: f32 = 10.0;

fn hud_rect(viewport: egui::Rect, content_size: egui::Vec2) -> Option<egui::Rect> {
    let min = viewport.min + egui::vec2(HUD_MARGIN, HUD_MARGIN);
    let max = viewport.max - egui::vec2(HUD_MARGIN, HUD_MARGIN);
    let available = egui::Rect::from_min_max(min, max).intersect(viewport);
    let width = available.width().min(MAX_HUD_WIDTH);
    let height = (content_size.y + HUD_PADDING * 2.0).min(available.height());
    (width > 0.0 && height > 0.0)
        .then(|| egui::Rect::from_min_size(available.min, egui::vec2(width, height)))
}

fn validate_text(field: &str, text: &str, maximum_bytes: usize) -> Result<(), String> {
    if text.len() > maximum_bytes || text.trim().is_empty() || text.chars().any(char::is_control) {
        return Err(format!(
            "mesh HUD {field} must be nonempty, control-free and at most {maximum_bytes} UTF-8 bytes"
        ));
    }
    Ok(())
}

fn validate_scene_filename(filename: &str) -> Result<(), String> {
    if filename.is_empty()
        || filename.len() > 255
        || filename == "."
        || filename == ".."
        || filename
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\' | ':'))
    {
        return Err("mesh HUD requires a bounded scene filename".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "mesh_hud/tests.rs"]
mod tests;
