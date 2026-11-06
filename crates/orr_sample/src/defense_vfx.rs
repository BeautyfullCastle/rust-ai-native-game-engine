//! Bounded, reversible presentation of real combat events using baked pixelfx.
//! No external code, runtime file lookup, wall clock, or simulation writes.
use crate::defense_view::Layout;
use egui::{vec2, Color32, ColorImage, Painter, Rect, TextureHandle, TextureOptions};
use orr_games::coop_defense_game::{DefenseEvent, DefenseEventKind as Kind};
use orr_session::{EventBatch, EventKey, EventStatus};
use orr_sprite::{PlaybackMode, SpriteDocument};
use std::collections::BTreeMap;

const MAX_ACTIVE: usize = 64;
const MAX_SEEN: usize = 2048;
// Local sessions may announce both prediction and verification for one tick.
// Never turn a valid producer burst into a fatal presentation error.
const MAX_BATCH: usize = 2 * orr_games::coop_defense_game::MAX_EVENTS_PER_TICK;
const SPECS: [(&str, &str, &[u8], [f32; 2]); 2] = [
    (
        "hit-spark",
        include_str!("../assets/stormglass-pixelfx/hit-spark/sprites.json"),
        include_bytes!("../assets/stormglass-pixelfx/hit-spark/hit-spark.png"),
        [44.0, 64.0],
    ),
    (
        "zap-hit",
        include_str!("../assets/stormglass-pixelfx/zap-hit/sprites.json"),
        include_bytes!("../assets/stormglass-pixelfx/zap-hit/zap-hit.png"),
        [76.0, 56.0],
    ),
];

struct Asset {
    document: SpriteDocument,
    texture: TextureHandle,
    pivot: [f32; 2],
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Active {
    event: DefenseEvent,
    asset: usize,
    born: u64,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct State {
    generation: u32,
    baseline: u64,
    tick: u64,
    restart: Option<EventKey>,
    floor: Option<EventKey>,
    seen: BTreeMap<EventKey, Option<DefenseEvent>>,
    active: BTreeMap<EventKey, Active>,
    started: u64,
    dropped: u64,
    canceled: u64,
}
#[derive(Default)]
pub(crate) struct DefenseVfx {
    assets: Vec<Asset>,
    state: State,
}
impl DefenseVfx {
    /// Decode only compile-time assets through the existing strict PNG contract.
    /// Called before opening a window or accepting gameplay input.
    pub(crate) fn prepare(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if !self.assets.is_empty() {
            return Ok(());
        }
        let mut decoded = Vec::new();
        for (name, json, png, pivot) in SPECS {
            let document = SpriteDocument::from_json(json).map_err(|e| e.to_string())?;
            if document.clips().len() != 1
                || document.clips()[0].id() != name
                || document.clips()[0].mode() != PlaybackMode::Once
            {
                return Err("embedded pixelfx clip contract mismatch".into());
            }
            let a = document.atlas();
            let pixels = orr_sprite::png_import::decode_atlas(png, a.width, a.height)?;
            let image =
                ColorImage::from_rgba_unmultiplied([a.width as usize, a.height as usize], &pixels);
            decoded.push((name, document, image, pivot));
        }
        self.assets = decoded
            .into_iter()
            .map(|(name, document, image, pivot)| Asset {
                texture: ctx.load_texture(
                    format!("stormglass-pixelfx-{name}"),
                    image,
                    TextureOptions::NEAREST,
                ),
                document,
                pivot,
            })
            .collect();
        Ok(())
    }
    /// A loaded/replaced session cannot replay historical effects.
    #[cfg(any(test, all(feature = "coop-defense-run-save", target_os = "linux")))]
    pub(crate) fn replace(&mut self, tick: u64, generation: u32) {
        self.state = State {
            tick,
            baseline: tick,
            generation,
            ..State::default()
        };
    }
    pub(crate) fn consume(&mut self, batch: &EventBatch<DefenseEvent>) -> Result<(), String> {
        if batch.len() > MAX_BATCH {
            return Err("pixelfx event batch exceeds bound".into());
        }
        self.consume_items(batch.iter().cloned())
    }
    fn consume_items(
        &mut self,
        items: impl IntoIterator<Item = (EventKey, EventStatus<DefenseEvent>)>,
    ) -> Result<(), String> {
        // A malformed later record cannot partially publish effects or a new epoch.
        let mut next = self.state.clone();
        for (key, status) in items {
            next.apply(key, status)?;
        }
        self.state = next;
        Ok(())
    }
    /// Advance on the existing session clock only. Pause/focus loss do not advance it.
    pub(crate) fn advance_to(&mut self, tick: u64) {
        self.state.tick = tick;
        self.state.active.retain(|_, effect| {
            let duration = self
                .assets
                .get(effect.asset)
                .map_or(if effect.asset == 0 { 528 } else { 924 }, |a| {
                    a.document.clips()[0].duration_ms()
                });
            tick.saturating_sub(effect.born).saturating_mul(1000) / 60 < duration
        });
    }
    pub(crate) fn paint(&self, painter: &Painter, layout: Layout) {
        for effect in self.state.active.values() {
            let Some(asset) = self.assets.get(effect.asset) else {
                continue;
            };
            let elapsed = self
                .state
                .tick
                .saturating_sub(effect.born)
                .saturating_mul(1000)
                / 60;
            let sample = asset.document.clips()[0].sample(elapsed);
            if sample.finished {
                continue;
            }
            let Some(region) = asset.document.region(sample.region) else {
                continue;
            };
            let scale = layout.scale
                * if effect.event.event_kind() == Some(Kind::Release) {
                    0.55
                } else {
                    0.75
                };
            let position = layout.world(effect.event.x, effect.event.y);
            let rect = Rect::from_min_size(
                position - vec2(asset.pivot[0], asset.pivot[1]) * scale,
                vec2(region.width as f32, region.height as f32) * scale,
            );
            let a = asset.document.atlas();
            let uv = Rect::from_min_max(
                egui::pos2(
                    region.x as f32 / a.width as f32,
                    region.y as f32 / a.height as f32,
                ),
                egui::pos2(
                    (region.x + region.width) as f32 / a.width as f32,
                    (region.y + region.height) as f32 / a.height as f32,
                ),
            );
            painter.image(asset.texture.id(), rect, uv, Color32::WHITE);
        }
    }
    /// Recovery from a real shot by this exact actor. The existing bounded
    /// event ledger owns deduplication, cancellation and epoch changes. This
    /// view offset neither anticipates release nor changes projectile timing.
    #[cfg(feature = "coop-defense-pixel-motion")]
    pub(crate) fn unit_recoil(
        &self,
        unit: &orr_games::coop_defense_game::DefenseUnit,
        run: &orr_games::coop_defense_game::DefenseRun,
    ) -> f32 {
        if unit.active == 0
            || unit.id == 0
            || run.phase != orr_games::coop_defense_game::PLAYING
            || run.generation != self.state.generation
        {
            return 0.0;
        }
        self.state
            .active
            .values()
            .filter(|effect| {
                effect.event.event_kind() == Some(Kind::Release)
                    && effect.event.unit_id == unit.id
                    && effect.event.board == unit.board
                    && effect.event.role == unit.role
                    && effect.event.generation == run.generation
            })
            .filter_map(|effect| self.state.tick.checked_sub(effect.born))
            .min()
            .filter(|age| *age < 8)
            .map_or(0.0, |age| (8 - age) as f32 * 0.75)
    }
    pub(crate) fn summary(&self) -> (usize, u64, u64, u64) {
        (
            self.state.active.len(),
            self.state.started,
            self.state.dropped,
            self.state.canceled,
        )
    }
    #[cfg(test)]
    pub(crate) fn test_stats(&self) -> (u64, u64, usize, u32) {
        (
            self.state.tick,
            self.state.started,
            self.state.active.len(),
            self.state.generation,
        )
    }
}
impl State {
    fn apply(&mut self, key: EventKey, status: EventStatus<DefenseEvent>) -> Result<(), String> {
        if key.system_index != 0 || key._pad != 0 {
            return Err("unsupported pixelfx event producer".into());
        }
        if key.tick <= self.baseline || self.floor.is_some_and(|floor| key <= floor) {
            return Ok(());
        }
        let event = match status {
            EventStatus::Predicted(e) | EventStatus::Verified(e) => e,
            EventStatus::Canceled => {
                if self.restart == Some(key) {
                    return Err(
                        "speculative restart rollback is unsupported by local pixelfx".into(),
                    );
                }
                if self.active.remove(&key).is_some() {
                    self.canceled += 1;
                }
                self.seen.insert(key, None);
                self.bound();
                return Ok(());
            }
        };
        let kind = event.event_kind().ok_or("unsupported pixelfx event kind")?;
        if kind == Kind::Restart && self.restart != Some(key) {
            if self.generation.checked_add(1) != Some(event.generation) {
                return Err("invalid pixelfx restart generation".into());
            }
            self.active.clear();
            self.seen.clear();
            self.generation = event.generation;
            self.restart = Some(key);
            self.baseline = key.tick.saturating_sub(1);
            self.floor = None;
        }
        if event.generation != self.generation {
            return Err("stale pixelfx event generation".into());
        }
        if self.seen.get(&key) == Some(&Some(event)) {
            return Ok(());
        }
        self.active.remove(&key);
        self.seen.insert(key, Some(event));
        if matches!(kind, Kind::Hit | Kind::Release) {
            if event.board > 1
                || event.role > 3
                || event.unit_id == 0
                || event.projectile_id == 0
                || event.enemy_id == 0
            {
                return Err("invalid pixelfx combat identity".into());
            }
            if self.active.len() < MAX_ACTIVE {
                let asset = usize::from(kind == Kind::Hit && event.role >= 2);
                self.active.insert(
                    key,
                    Active {
                        event,
                        asset,
                        born: key.tick,
                    },
                );
                self.started += 1;
            } else {
                self.dropped += 1;
            }
        }
        self.bound();
        Ok(())
    }
    fn bound(&mut self) {
        while self.seen.len() > MAX_SEEN {
            let (key, _) = self.seen.pop_first().expect("nonempty bounded history");
            self.floor = Some(key);
            self.active.remove(&key);
        }
    }
}

#[cfg(test)]
#[path = "defense_vfx_tests.rs"]
mod tests;

#[cfg(all(test, feature = "coop-defense-pixel-motion"))]
#[path = "defense_release_recoil_tests.rs"]
mod release_recoil_tests;
