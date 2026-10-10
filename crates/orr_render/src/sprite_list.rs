//! GPU-free submission list for a single sprite atlas.
pub use orr_sprite::SpriteInstance;

/// Instances use world-space centers/full sizes and straight linear RGBA tints.
/// Higher `order` draws later; equal orders preserve insertion order. A list is
/// resolved against the document owned by the renderer at draw time.
#[derive(Clone, Debug, Default)]
pub struct SpriteDrawList {
    pub sprites: Vec<SpriteInstance>,
}

impl SpriteDrawList {
    pub fn clear(&mut self) {
        self.sprites.clear();
    }

    pub fn push(&mut self, sprite: SpriteInstance) {
        self.sprites.push(sprite);
    }
}
