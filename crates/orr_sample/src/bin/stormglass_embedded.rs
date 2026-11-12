//! Opt-in prototype: the ordinary native game with exact authored YAML embedded.
const SCENE: &str = include_str!("../../assets/stormglass-embedded.scene.yaml");

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["--help"] || args == ["-h"] {
        println!("Stormglass embedded-scene prototype\nstormglass_embedded [--headless --ticks N [--demo] [--capture NEW.png]]\n--embedded-scene-sha256 reports the strictly parsed built-in source hash.\nNo --scene override. Changing the authored YAML requires rebuilding this executable.\nDefault keys: Space summon, U upgrade, I invest, P/Escape pause, Enter continue, R restart.");
        return;
    }
    if args == ["--embedded-scene-sha256"] {
        match orr_sample::defense_app::embedded_scene_sha256(SCENE) {
            Ok(hash) => println!("{hash}"),
            Err(error) => {
                eprintln!("Stormglass embedded prototype: {error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Err(error) = orr_sample::defense_app::Options::parse(args)
        .and_then(|options| orr_sample::defense_app::run_embedded(options, SCENE))
    {
        eprintln!("Stormglass embedded prototype: {error}");
        std::process::exit(1);
    }
}
