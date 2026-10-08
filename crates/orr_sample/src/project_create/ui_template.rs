//! Closed optional font source; no filesystem discovery or downloads.
use sha2::{Digest, Sha256};
pub(super) const PACKAGE: &str = "korean-game-ui";
pub(super) const FONT_BYTES: usize = 1_891_888;
pub(super) const SOURCE: &[(&str, &[u8])] = &[
    (
        "orr.package.json",
        include_bytes!("../../../../assets/game_ui_font/orr.package.json"),
    ),
    (
        "OrreryKoreanUI.otf",
        include_bytes!("../../../../assets/game_ui_font/OrreryKoreanUI.otf"),
    ),
    (
        "font-manifest.json",
        include_bytes!("../../../../assets/game_ui_font/font-manifest.json"),
    ),
    (
        "corpus.txt",
        include_bytes!("../../../../assets/game_ui_font/corpus.txt"),
    ),
    (
        "OFL.txt",
        include_bytes!("../../../../assets/game_ui_font/OFL.txt"),
    ),
    (
        "COPYRIGHT.txt",
        include_bytes!("../../../../assets/game_ui_font/COPYRIGHT.txt"),
    ),
];
pub(super) fn is_bundled_font(bytes: &[u8]) -> bool {
    bytes.len() == FONT_BYTES
        && bytes == SOURCE[1].1
        && Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
            == "91c7e75ac1b54a3571a305d259a2f486b88289853baef90753506855f5c5dd08"
}
#[cfg(feature = "collect-ui")]
pub(super) fn descriptor() -> serde_json::Value {
    serde_json::json!({"profile":"collect-authored-v1","document":"level.ui.json",
        "font":{"package":PACKAGE,"asset":"OrreryKoreanUI.otf"}})
}
