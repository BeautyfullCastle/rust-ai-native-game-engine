//! Optional dedicated post-release frames. Artwork is sampled, never simulated.
use super::*;

const ATTACK_JSON: &str = include_str!("../assets/stormglass-attack-clip/sprites.json");
const ATTACK_PNG: &[u8] = include_bytes!("../assets/stormglass-attack-clip/attack.png");
const ATTACK_PIVOT: [f32; 2] = [222.0, 362.0];
const ATTACK_SCALE: f32 = 0.184;
const ATTACK_DURATION_MS: u64 = 320;

impl PixelMotion {
    pub(super) fn prepare_attack(ctx: &egui::Context) -> Result<Asset, String> {
        let document = SpriteDocument::from_json(ATTACK_JSON).map_err(|e| e.to_string())?;
        let atlas = document.atlas();
        if (atlas.width, atlas.height, atlas.image.as_str()) != (1774, 887, "attack.png")
            || document.clips().len() != 1
            || document.regions().len() != 8
        {
            return Err("embedded attack atlas contract mismatch".into());
        }
        let clip = &document.clips()[0];
        if clip.id() != "rivet-shot"
            || clip.mode() != PlaybackMode::Once
            || clip.duration_ms() != ATTACK_DURATION_MS
            || clip.frames().len() != 8
            || clip
                .frames()
                .iter()
                .enumerate()
                .any(|(i, frame)| frame.region != i as u32 || frame.duration_ms != 40)
        {
            return Err("embedded attack clip contract mismatch".into());
        }
        let xs = [0, 444, 887, 1330, 1774];
        let ys = [0, 444, 887];
        if (0..8).any(|i| {
            document.region(i as u32).is_none_or(|r| {
                (r.x, r.y, r.width, r.height)
                    != (
                        xs[i % 4],
                        ys[i / 4],
                        xs[i % 4 + 1] - xs[i % 4],
                        ys[i / 4 + 1] - ys[i / 4],
                    )
            })
        }) {
            return Err("embedded attack region contract mismatch".into());
        }
        let rgba = orr_sprite::png_import::decode_atlas(ATTACK_PNG, atlas.width, atlas.height)?;
        let image = ColorImage::from_rgba_unmultiplied(
            [atlas.width as usize, atlas.height as usize],
            &rgba,
        );
        Ok(Asset {
            document,
            texture: ctx.load_texture("stormglass-rivet-shot", image, TextureOptions::NEAREST),
            pivot: ATTACK_PIVOT,
            scale: ATTACK_SCALE,
        })
    }

    /// False after the one-shot duration so the caller resumes the idle skin.
    pub(crate) fn paint_release(
        &self,
        p: &Painter,
        center: Pos2,
        scale: f32,
        unit: &game::DefenseUnit,
        age: u64,
        flip_x: bool,
    ) -> bool {
        if unit.active == 0 || unit.role != game::RIVET {
            return false;
        }
        let Some(asset) = &self.attack else {
            return false;
        };
        let elapsed = age.saturating_mul(1000) / 60;
        if elapsed >= ATTACK_DURATION_MS {
            return false;
        }
        Self::paint_asset(p, center, scale, unit, asset, elapsed, flip_x)
    }

    #[cfg(test)]
    pub(crate) fn test_attack_texture_id(&self) -> egui::TextureId {
        self.attack.as_ref().expect("prepared attack").texture.id()
    }
}

#[cfg(test)]
#[path = "defense_attack_clip_tests.rs"]
mod tests;
