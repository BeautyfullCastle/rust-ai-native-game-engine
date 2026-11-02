//! Closed reviewed module graph, not a JavaScript parser or syntax validator.
use regex::Regex;
use std::collections::BTreeMap;
pub(super) const ASSETS: [&str; 12] = [
    "combat-presentation.mjs",
    "strict-event-adapter.mjs",
    "run-save.mjs",
    "audio/authored-asset-audio.mjs",
    "audio/coop-defense-audio.mjs",
    "game.js",
    "index.html",
    "style.css",
    "game.wasm",
    "assets/characters.json",
    "assets/units.png",
    "assets/enemies.png",
];
const GAME_IMPORTS: &str = concat!(
    "import { CombatPresentation } from './combat-presentation.mjs';\n",
    "import { StrictEventAdapter, applyStrictSnapshot } from './strict-event-adapter.mjs';\n",
    "import { RUN_SAVE_LIMITS, parseRunSave, createRunRecorder, sceneInitial, sha256, replayRunSave } from './run-save.mjs';\n",
    "import { loadAuthoredAudio } from './audio/coop-defense-audio.mjs';\n",
);
const AUDIO_IMPORT: &str =
    "import { AuthoredAssetAudio, loadAuthoredAssetBank } from './authored-asset-audio.mjs';\n";
const AUDIO_EXPORTS: [&str; 2] = [
    "audioGain, loadAuthoredAssetBank, loadSampleAssetBank, AuthoredAssetAudio",
    "CoopDefenseAudio, loadAuthoredAudio",
];
pub(super) fn bundle(files: &BTreeMap<String, Vec<u8>>) -> Result<String, String> {
    let read = |name: &str| -> Result<&str, String> {
        let bytes = files
            .get(name)
            .ok_or_else(|| format!("Missing bundle module: {name}"))?;
        std::str::from_utf8(bytes).map_err(|_| format!("Module is not UTF-8: {name}"))
    };
    let game = read("game.js")?
        .strip_prefix(GAME_IMPORTS)
        .ok_or("Unexpected game import graph; update the explicit offline contract")?;
    let mut sources: Vec<String> = ASSETS[..3]
        .iter()
        .map(|name| read(name).map(str::to_owned))
        .collect::<Result<_, _>>()?;
    // Capture the declaration instead of lookahead (unsupported by regex).
    let declaration = Regex::new(r"(?m)^export ((?:async )?(?:const|class|function) )").unwrap();
    for (i, name) in ASSETS[3..5].iter().enumerate() {
        let mut source = read(name)?.to_owned();
        if i == 1 {
            if source.matches(AUDIO_IMPORT).count() != 1 {
                return Err("Unexpected authored-audio module graph".into());
            }
            source = source.replacen(AUDIO_IMPORT, "", 1);
        }
        source = declaration.replace_all(&source, "$1").into_owned();
        let exports = AUDIO_EXPORTS[i];
        sources.push(format!(
            "const {{{exports}}}=(()=>{{\n{source}\nreturn {{{exports}}};\n}})();"
        ));
    }
    sources.push(game.to_owned());
    let keywords = Regex::new(r"\b(?:import|export)\b").unwrap();
    let allowed_import = Regex::new(r"^import\.meta\b").unwrap();
    let allowed_export = Regex::new(r"^export (?:const|class|function)\b").unwrap();
    let closing = Regex::new(r"(?i)</script").unwrap();
    for source in &sources {
        for word in keywords.find_iter(source) {
            let tail = &source[word.start()..];
            if !allowed_import.is_match(tail) && !allowed_export.is_match(tail) {
                return Err("Unbundled JavaScript dependency or unsupported module keyword".into());
            }
        }
        if closing.is_match(source) {
            return Err("JavaScript would terminate its HTML script element".into());
        }
    }
    Ok(sources.join("\n"))
}
#[cfg(test)]
#[path = "bundle_tests.rs"]
mod tests;
