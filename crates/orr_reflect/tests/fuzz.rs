//! Malformed input must never panic: mutation fuzzing of valid scenes plus random text.

mod common;

use common::*;
use orr_reflect::Scene;

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

const TOKENS: &[&str] = &[
    "&a ", "*a", "!!str ", "!t ", "{", "}", "[", "]", ":", ": ", "-", "- ", "#", "\t", "\"", "'", "|", ">", "? ", "%YAML 1.2\n", "\0",
    "é", "\u{2028}", "\r\n", "<<: ", "---\n", "...\n", "null", "~", "true", "no", "1e999", "-0", "0x", "140737488355328", "e_", "\n  ", ",",
    "\\", "\\u", "\"\\x", "e_ffffffff", "kind", "name",
];

fn mutate(src: &str, rng: &mut Lcg) -> String {
    let mut chars: Vec<char> = src.chars().collect();
    for _ in 0..1 + rng.below(4) {
        if chars.is_empty() {
            chars.push('a');
        }
        let at = rng.below(chars.len());
        match rng.below(6) {
            0 => {
                let n = 1 + rng.below(8);
                let end = (at + n).min(chars.len());
                chars.drain(at..end);
            }
            1 => {
                let t: Vec<char> = TOKENS[rng.below(TOKENS.len())].chars().collect();
                for (i, c) in t.into_iter().enumerate() {
                    chars.insert(at + i, c);
                }
            }
            2 => {
                let n = 1 + rng.below(30);
                let end = (at + n).min(chars.len());
                let piece: Vec<char> = chars[at..end].to_vec();
                let to = rng.below(chars.len());
                for (i, c) in piece.into_iter().enumerate() {
                    chars.insert(to + i, c);
                }
            }
            3 => chars[at] = (32 + rng.below(95) as u8) as char,
            4 => chars.truncate(at),
            _ => {
                let j = rng.below(chars.len());
                chars.swap(at, j);
            }
        }
    }
    chars.into_iter().collect()
}

fn exercise(text: &str) {
    let result = std::panic::catch_unwind(|| {
        let reg = types();
        if let Ok(scene) = Scene::parse(text, &reg) {
            // Anything accepted must write, re-read identically and bake without error.
            let out = scene.to_yaml();
            let again = Scene::parse(&out, &reg).unwrap_or_else(|e| panic!("own output rejected: {e}\n{out}"));
            assert_eq!(again.to_yaml(), out, "writing is not stable");
            let mut frame = frame();
            scene.bake(&reg, &mut frame).unwrap_or_else(|e| panic!("bake of accepted scene failed: {e}"));
            let back = Scene::unbake(&reg, &frame, None).unwrap();
            let _ = back.to_yaml();
        }
    });
    if result.is_err() {
        panic!("panic on input:\n{text:?}");
    }
}

#[test]
fn mutated_scenes_never_panic() {
    let reg = types();
    let mut rng = Lcg(0xF00D);
    let seeds = [SAMPLE, "schema: orr.scene/1\nentities: {}\n", "schema: orr.scene/1\nentities:\n  e_00000001:\n    Blob: { kind: poly, pts: [[0, 0], [1, 0], [0, 1]] }\n"];
    let mut accepted = 0;
    for i in 0..100_000 {
        let text = mutate(seeds[i % seeds.len()], &mut rng);
        if Scene::parse(&text, &reg).is_ok() {
            accepted += 1;
        }
        exercise(&text);
    }
    assert!(accepted > 10, "the fuzzer should also reach accepted files ({accepted})");
}

#[test]
fn random_text_never_panics() {
    let reg = types();
    let mut rng = Lcg(0xBEEF);
    let alphabet: Vec<char> = "abcXYZ019 _:-#&*!{}[],.\"'|>?%~\n\t\\eEnul".chars().collect();
    for _ in 0..20_000 {
        let len = rng.below(120);
        let text: String = (0..len).map(|_| alphabet[rng.below(alphabet.len())]).collect();
        exercise(&text);
    }
}

#[test]
fn every_prefix_of_a_scene_is_handled() {
    for (i, _) in SAMPLE.char_indices() {
        exercise(&SAMPLE[..i]);
    }
}

#[test]
fn extreme_inputs_are_handled() {
    let reg = types();
    let long_key = "a".repeat(100_000);
    let inputs = [
        format!("schema: orr.scene/1\nentities:\n  {long_key}: {{}}\n"),
        format!("schema: orr.scene/1\nentities: {{}}\n{long_key}: 1\n"),
        format!("schema: orr.scene/1\nentities:\n  e_00000001:\n    name: {long_key}\n"),
        format!("schema: orr.scene/1\nentities:\n  e_00000001:\n    Transform: {{ pos: [{}, 1], rot: 0 }}\n", "9".repeat(5000)),
        format!("schema: orr.scene/1\nentities:\n  e_00000001:\n    Transform: {{ pos: [0.{}, 1], rot: 0 }}\n", "9".repeat(5000)),
        "\u{feff}schema: orr.scene/1\nentities: {}\n".to_string(),
        "schema: orr.scene/1\r\nentities: {}\r\n".to_string(),
        "\t\nschema: orr.scene/1\nentities: {}\n".to_string(),
    ];
    for text in &inputs {
        exercise(text);
    }
    // CRLF files are fine.
    assert!(Scene::parse("schema: orr.scene/1\r\nentities: {}\r\n", &reg).is_ok());
}
