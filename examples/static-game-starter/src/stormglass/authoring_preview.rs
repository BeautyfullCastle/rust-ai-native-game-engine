//! Read-only authoring aid. Rebuilt from the displayed authoritative snapshot,
//! never cached from the project file and never used to dispatch commands.
use orr_bridge::FrameView;
use orr_editor::static_adapter::GameplayViewPhase;
use orr_games::coop_defense_game::{
    Campaign, EnemyHealth, Scenario, WaveComposition, WaveCounts, WavePacing, ARMORED, BOSS, FAST,
    HORDE, NORMAL,
};

#[derive(Debug, PartialEq, Eq)]
struct WavePreview {
    wave: u32,
    kinds: Vec<u32>,
}

impl WavePreview {
    fn label(&self) -> String {
        let names = self
            .kinds
            .iter()
            .map(|kind| match *kind {
                NORMAL => "normal",
                FAST => "fast",
                ARMORED => "armored",
                HORDE => "horde",
                BOSS => "boss",
                _ => "unknown",
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "Wave {}: {} per lane ({} total) | {}",
            self.wave,
            self.kinds.len(),
            self.kinds.len() * 2,
            names
        )
    }
}

fn waves(frame: FrameView<'_>) -> Vec<WavePreview> {
    let campaign = frame.singleton::<Campaign>();
    let counts = frame.singleton::<WaveCounts>();
    let composition = frame.singleton::<WaveComposition>();
    (0..campaign.last_wave.min(10))
        .map(|index| {
            let i = index as usize;
            WavePreview {
                wave: index + 1,
                kinds: (0..counts.pair_counts[i].min(28))
                    .map(|ordinal| {
                        if ordinal == 0 && composition.opening_boss[i] != 0 {
                            BOSS
                        } else {
                            composition.regular_kinds[i][(ordinal % 4) as usize]
                        }
                    })
                    .collect(),
            }
        })
        .collect()
}

fn first_spawn_tick(interval: u32) -> Option<u32> {
    // Same wave-local alignment as DefenseGame. Invalid snapshots cannot divide
    // by zero; normal admission restricts this value to 35..=350.
    (35..=350)
        .contains(&interval)
        .then(|| 180_u32.div_ceil(interval) * interval)
}

pub(super) fn show(ui: &mut egui::Ui, frame: FrameView<'_>, phase: GameplayViewPhase) {
    ui.collapsing("Authored waves", |ui| {
        ui.label(if phase == GameplayViewPhase::Edit {
            "Read-only current edit snapshot, including unsaved changes. Save to keep these settings."
        } else {
            "Read-only displayed session settings, not remaining enemies or a victory prediction."
        });
        let scenario = frame.singleton::<Scenario>();
        ui.label(format!("Starting gold per board: {} | Reactor HP: {} | Enemy HP multiplier: {}",
            scenario.starting_gold, scenario.reactor_hp, frame.singleton::<EnemyHealth>().multiplier));
        let interval = frame.singleton::<WavePacing>().spawn_interval_ticks;
        if let Some(first) = first_spawn_tick(interval) {
            ui.label(format!("First pair at wave tick {first}; then every {interval} ticks (60 ticks/second)."));
        }
        ui.label("Each pair sends one enemy per lane. Listed in spawn order; bosses replace the first pair without shifting later kinds.");
        egui::ScrollArea::vertical().id_salt("authored_wave_rows").max_height(180.0).show(ui, |ui| {
            for wave in waves(frame) {
                ui.label(wave.label());
            }
        });
    });
}

#[cfg(test)]
mod tests;
