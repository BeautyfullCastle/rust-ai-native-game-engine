//! Optional generated Rivet skin. The accepted charge motion is a visual loop,
//! not anticipation, attack synchronization or an event producer.
use crate::defense_view::{BRONZE, INK, PAPER};
use egui::{
    pos2, vec2, Align2, Color32, ColorImage, FontId, Painter, Pos2, Rect, TextureHandle,
    TextureOptions,
};
use orr_games::coop_defense_game as game;
use orr_sprite::{PlaybackMode, SpriteDocument};

const JSON: &str = include_str!("../assets/stormglass-pixel-motion/sprites.json");
const PNG: &[u8] = include_bytes!("../assets/stormglass-pixel-motion/atlas.png");
const PIVOT: [f32; 2] = [48.0, 85.0];
const SCALE: f32 = 0.85;
const CYCLE_MS: u64 = 1400;

struct Asset {
    document: SpriteDocument,
    texture: TextureHandle,
}
#[derive(Default)]
pub(crate) struct PixelMotion {
    asset: Option<Asset>,
}
impl PixelMotion {
    #[cfg(test)]
    pub(crate) fn test_texture_id(&self) -> egui::TextureId {
        self.asset
            .as_ref()
            .expect("prepared test skin")
            .texture
            .id()
    }

    /// The package's immutable Once clip stays untouched. Looping is exclusively
    /// a presentation policy, sampled from the session's run-relative tick.
    pub(crate) fn prepare(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.asset.is_some() {
            return Ok(());
        }
        let document = SpriteDocument::from_json(JSON).map_err(|e| e.to_string())?;
        let atlas = document.atlas();
        if (atlas.width, atlas.height, atlas.image.as_str()) != (672, 224, "atlas.png")
            || document.regions().len() != 14
            || document.clips().len() != 1
        {
            return Err("embedded pixel motion atlas contract mismatch".into());
        }
        let clip = &document.clips()[0];
        if clip.id() != "stormglass-sentry-charge"
            || clip.mode() != PlaybackMode::Once
            || clip.duration_ms() != CYCLE_MS
            || clip.frames().len() != 14
            || clip
                .frames()
                .iter()
                .enumerate()
                .any(|(i, frame)| frame.region != i as u32 || frame.duration_ms != 100)
            || (0..14).any(|i| {
                document.region(i).is_none_or(|r| {
                    (r.x, r.y, r.width, r.height) != (i % 7 * 96, i / 7 * 112, 96, 112)
                })
            })
        {
            return Err("embedded pixel motion frame contract mismatch".into());
        }
        let rgba = orr_sprite::png_import::decode_atlas(PNG, atlas.width, atlas.height)?;
        let image = ColorImage::from_rgba_unmultiplied(
            [atlas.width as usize, atlas.height as usize],
            &rgba,
        );
        let texture = ctx.load_texture("stormglass-pixel-motion", image, TextureOptions::NEAREST);
        self.asset = Some(Asset { document, texture });
        Ok(())
    }

    /// No wall clock, accumulated state or historical events: paused redraw,
    /// restart, restored saves and rollback all sample the actual snapshot.
    fn elapsed(run: &game::DefenseRun, started: bool) -> u64 {
        if started && run.phase == game::PLAYING {
            (u64::from(run.tick) * 1000 / 60) % CYCLE_MS
        } else {
            0 // Prestart/terminal presentation uses the accepted neutral frame.
        }
    }

    pub(crate) fn paint_unit(
        &self,
        p: &Painter,
        center: Pos2,
        scale: f32,
        unit: &game::DefenseUnit,
        run: &game::DefenseRun,
        started: bool,
    ) -> bool {
        if unit.active == 0 || unit.role != game::RIVET {
            return false;
        }
        let Some(asset) = &self.asset else {
            return false; // A caller without prepared assets retains vector art.
        };
        let sample = asset.document.clips()[0].sample(Self::elapsed(run, started));
        let Some(region) = asset.document.region(sample.region) else {
            return false;
        };
        let anchor = center + vec2(0.0, 20.0) * scale;
        let sprite_scale = scale * SCALE;
        let rect = Rect::from_min_size(
            anchor - vec2(PIVOT[0], PIVOT[1]) * sprite_scale,
            vec2(region.width as f32, region.height as f32) * sprite_scale,
        );
        let [u0, v0, u1, v1] = asset
            .document
            .uv_rect(sample.region)
            .expect("validated region");
        p.add(egui::Shape::ellipse_filled(
            center + vec2(0.0, 19.0) * scale,
            vec2(21.0, 5.0) * scale,
            INK,
        ));
        p.image(
            asset.texture.id(),
            rect,
            Rect::from_min_max(pos2(u0, v0), pos2(u1, v1)),
            Color32::WHITE,
        );
        // Explicit role/tier feedback replaces the vector turret's tier bracing.
        // The enclosing view still draws selection, range and real firing cues.
        let badge =
            Rect::from_center_size(center + vec2(0.0, 28.0) * scale, vec2(54.0, 11.0) * scale);
        p.rect_filled(badge, 2.0 * scale, INK);
        p.rect_stroke(
            badge,
            2.0 * scale,
            egui::Stroke::new(scale, BRONZE),
            egui::StrokeKind::Inside,
        );
        p.text(
            badge.center(),
            Align2::CENTER_CENTER,
            format!("Rivet T{}", unit.tier),
            FontId::proportional(9.0 * scale),
            PAPER,
        );
        true
    }
}

#[cfg(test)]
#[path = "defense_pixel_motion_tests.rs"]
mod tests;
