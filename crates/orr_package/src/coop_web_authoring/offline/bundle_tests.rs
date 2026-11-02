use super::*;
fn fixture() -> BTreeMap<String, Vec<u8>> {
    BTreeMap::from([
        (
            ASSETS[0].into(),
            b"export class CombatPresentation {}".to_vec(),
        ),
        (ASSETS[1].into(), b"export const adapter = 1;".to_vec()),
        (ASSETS[2].into(), b"export function save() {}".to_vec()),
        (
            ASSETS[3].into(),
            b"export const audioGain=1;\nexport async function loadAuthoredAssetBank() {}".to_vec(),
        ),
        (
            ASSETS[4].into(),
            format!("{AUDIO_IMPORT}export class CoopDefenseAudio {{}}").into_bytes(),
        ),
        (
            "game.js".into(),
            format!("{GAME_IMPORTS}const here=import.meta.url;").into_bytes(),
        ),
    ])
}
#[test]
fn exact_closed_graph_transform_and_order() {
    let expected = concat!(
        "export class CombatPresentation {}\nexport const adapter = 1;\nexport function save() {}\n",
        "const {audioGain, loadAuthoredAssetBank, loadSampleAssetBank, AuthoredAssetAudio}=(()=>{\n",
        "const audioGain=1;\nasync function loadAuthoredAssetBank() {}\n",
        "return {audioGain, loadAuthoredAssetBank, loadSampleAssetBank, AuthoredAssetAudio};\n})();\n",
        "const {CoopDefenseAudio, loadAuthoredAudio}=(()=>{\nclass CoopDefenseAudio {}\nreturn {CoopDefenseAudio, loadAuthoredAudio};\n})();\n",
        "const here=import.meta.url;"
    );
    assert_eq!(bundle(&fixture()).unwrap(), expected);
    assert_eq!(ASSETS.len(), 12);
}
#[test]
fn all_keyword_occurrences_and_script_closing_fail_closed() {
    for suffix in [
        "import('x')",
        "// import x",
        "'export x'",
        "export async function x(){}",
        "import.metaX",
        "</ScRiPt>",
        "export/*x*/const a=1",
    ] {
        let mut f = fixture();
        f.insert(
            "game.js".into(),
            format!("{GAME_IMPORTS}{suffix}").into_bytes(),
        );
        assert!(bundle(&f).is_err(), "{suffix}");
    }
    let mut f = fixture();
    f.insert(
        "game.js".into(),
        format!("{GAME_IMPORTS}const importer=1; export const x=import.meta.url;").into_bytes(),
    );
    assert!(bundle(&f).is_ok());
}
#[test]
fn exact_edges_utf8_and_line_start_exports() {
    let mut f = fixture();
    f.get_mut("game.js").unwrap().insert(0, b' ');
    assert!(bundle(&f).is_err());
    let mut f = fixture();
    f.insert(
        ASSETS[4].into(),
        format!("{AUDIO_IMPORT}{AUDIO_IMPORT}").into_bytes(),
    );
    assert!(bundle(&f).is_err());
    let mut f = fixture();
    f.insert(ASSETS[0].into(), vec![255]);
    assert!(bundle(&f).is_err());
    let mut f = fixture();
    f.remove(ASSETS[1]);
    assert!(bundle(&f).is_err());
    let mut f = fixture();
    f.insert(
        ASSETS[3].into(),
        b" export const retained=1;\nexport const removed=2;".to_vec(),
    );
    let result = bundle(&f).unwrap();
    assert!(result.contains(" export const retained=1;\nconst removed=2;"));
}
