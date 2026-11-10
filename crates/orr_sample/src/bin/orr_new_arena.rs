//! Create one offline, versioned Arena starter with an explicit authoring seed.
use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};
const HELP: &str = "Usage: orr_new_arena --output ABSOLUTE_NEW_DIR --template arena-2d-v1 --seed KEY\n\nCreates one sprite-only two-player Arena starter on Linux. The parent must exist;\nall existing destinations are rejected. KEY is 1..128 ASCII letters, digits,\ndots, underscores or hyphens. Same seed/template/tool reproduces project bytes;\nchoose a different seed for fresh authored GUIDs. Runtime identity and shared\nplayer preferences do not change. No downloads, scripts or builds are executed.\n\nWith collect-dodge enabled: --template collect-dodge-2d-v1 --seed KEY --game-id UUID\nrequires an explicit canonical lowercase UUIDv4 for the new game.\nWith collect-ui enabled, collect-dodge-ui-2d-v1 also installs the authored UI and Korean font.\nA reused UUID shares high-score identity; the seed does not create or change game identity.\n\nWith room-project enabled: --template room-escape-3d-v1|room-escape-character-3d-v1 --seed KEY creates a closed static-model RoomEscapeV1 starter. No --game-id or progress/save identity is accepted. Plain room-escape-3d-v1 accepts optional --room-layout FILE.json for schema-1 actor positions and replacement walls.\n\nWith tilemap enabled: --template arena-tilemap-2d-v1 creates a closed installed-atlas Arena background map for tilemap-enabled editor paint/erase/Undo/Save/reopen/Play. Linux creation only. Existing-project Attach, export and package changes are unsupported.\n\nWith tilemap enabled: --template arena-custom-tileset-2d-v1 --tileset-package ABS_DIR --tile-width N --tile-height N creates a new empty map from a local four-file sprite package. The package requires orr.package.json, sprites.json, a PNG atlas and LICENSE.txt. Actor art is retained; source files are unchanged.\n\nWith project-create enabled: --template arena-custom-sprites-2d-v1 --sprite-package ABS_DIR --idle-clip ID --walk-clip ID --units-per-pixel N replaces only the camera-follow hero art. The local four-file package requires a PNG atlas, sprites.json, LICENSE.txt and manifest. Clip IDs are explicit and may be equal; scale must be finite in 0.0001..=100. Target art and Arena gameplay are retained.";
fn main() {
    if let Err((code, error)) = run() {
        eprintln!("error: {error}");
        std::process::exit(code);
    }
}
fn run() -> Result<(), (i32, String)> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{HELP}");
        return Ok(());
    }
    let args = parse(args).map_err(|e| (2, e))?;
    #[cfg(target_os = "linux")]
    {
        let options = orr_sample::project_create::CreateOptions {
            output: PathBuf::from(&args["--output"]),
            template: args["--template"]
                .clone()
                .into_string()
                .map_err(|_| (2, "template must be UTF-8".into()))?,
            seed: args["--seed"]
                .clone()
                .into_string()
                .map_err(|_| (2, "seed must be ASCII".into()))?,
        };
        if args.contains_key("--room-layout") && options.template != orr_sample::project_create::ROOM_TEMPLATE {
            return Err((2, "--room-layout requires room-escape-3d-v1".into()));
        }
        let custom_sprites = options.template == orr_sample::project_create::CUSTOM_SPRITE_TEMPLATE;
        if !custom_sprites
            && [
                "--sprite-package",
                "--idle-clip",
                "--walk-clip",
                "--units-per-pixel",
            ]
            .iter()
            .any(|key| args.contains_key(*key))
        {
            return Err((
                2,
                "custom sprite options require arena-custom-sprites-2d-v1".into(),
            ));
        }
        let custom = options.template == orr_sample::project_create::CUSTOM_TILEMAP_TEMPLATE;
        if !custom && ["--tileset-package", "--tile-width", "--tile-height"].iter().any(|key| args.contains_key(*key)) {
            return Err((2, "custom tileset options require arena-custom-tileset-2d-v1".into()));
        }
        let report = if custom_sprites {
            if args.contains_key("--game-id") {
                return Err((2, "--game-id is only valid for CollectDodge".into()));
            }
            let required = |key: &str| -> Result<&OsString, (i32, String)> {
                args.get(key).ok_or_else(|| (2, format!("custom sprites require {key}")))
            };
            let clip = |key: &str| -> Result<String, (i32, String)> {
                required(key)?
                    .to_str()
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
                    .ok_or_else(|| (2, format!("{key} requires a nonempty UTF-8 clip ID")))
            };
            let package = PathBuf::from(required("--sprite-package")?);
            let idle = clip("--idle-clip")?;
            let walk = clip("--walk-clip")?;
            let units_per_pixel = required("--units-per-pixel")?
                .to_str()
                .and_then(|text| text.parse::<f32>().ok())
                .filter(|value| value.is_finite() && (0.0001..=100.0).contains(value))
                .ok_or_else(|| (2, "--units-per-pixel must be finite in 0.0001..=100".into()))?;
            orr_sample::project_create::create_custom_sprites(
                &orr_sample::project_create::CustomSpriteOptions {
                    base: options,
                    package,
                    idle,
                    walk,
                    units_per_pixel,
                },
            )
        } else if custom {
            if args.contains_key("--game-id") { return Err((2, "--game-id is only valid for CollectDodge".into())); }
            let package = args.get("--tileset-package").ok_or((2, "custom tileset requires --tileset-package".into()))?;
            let dimension = |key: &str| -> Result<u32, (i32, String)> {
                let text = args.get(key).and_then(|v| v.to_str()).ok_or((2, format!("custom tileset requires {key}")))?;
                if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) { return Err((2, format!("{key} must be a positive integer"))); }
                text.parse::<u32>().ok().filter(|n| *n > 0).ok_or((2, format!("{key} must be a positive u32 integer")))
            };
            let tile_size = [dimension("--tile-width")?, dimension("--tile-height")?];
            #[cfg(feature = "tilemap")]
            { orr_sample::project_create::create_custom_tilemap(&orr_sample::project_create::CustomTilemapOptions { base: options, package: PathBuf::from(package), tile_size }) }
            #[cfg(not(feature = "tilemap"))]
            { let _ = (package, tile_size); Err("Arena custom tileset template requires tilemap feature".into()) }
        } else if options.template == orr_sample::project_create::COLLECT_TEMPLATE
            || options.template == orr_sample::project_create::COLLECT_UI_TEMPLATE
        {
            let game_id = args
                .get("--game-id")
                .and_then(|v| v.to_str())
                .ok_or((2, "CollectDodge requires --game-id canonical UUIDv4".into()))?;
            #[cfg(feature = "collect-dodge")]
            {
                orr_sample::project_create::create_collect(&options, game_id)
            }
            #[cfg(not(feature = "collect-dodge"))]
            {
                let _ = game_id;
                Err("CollectDodge template requires collect-dodge feature".into())
            }
        } else if options.template == orr_sample::project_create::TILEMAP_TEMPLATE {
            if args.contains_key("--game-id") { return Err((2, "--game-id is only valid for CollectDodge".into())); }
            #[cfg(feature = "tilemap")]
            { orr_sample::project_create::create_tilemap(&options) }
            #[cfg(not(feature = "tilemap"))]
            { Err("Arena tilemap template requires tilemap feature".into()) }
        } else {
            if args.contains_key("--game-id") {
                return Err((2, "--game-id is only valid for CollectDodge".into()));
            }
            if let Some(recipe) = args.get("--room-layout") {
                #[cfg(feature = "room-project")]
                { orr_sample::project_create::create_room_layout(&options, &PathBuf::from(recipe)) }
                #[cfg(not(feature = "room-project"))]
                { let _ = recipe; Err("Room layout requires room-project feature".into()) }
            } else { orr_sample::project_create::create(&options) }
        }
        .map_err(|e| (1, e))?;
        println!("created project: {}", report.output.display());
        println!("template: {}", report.template);
        println!("authoring seed: {}", report.seed);
        println!(
            "project initial checksum: 0x{:016x}",
            report.initial_checksum
        );
        for guid in report.entity_guids {
            println!("project entity: {guid}");
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (args, PathBuf::new());
        Err((1, "project-create supports only Linux hosts".into()))
    }
}
fn parse(args: Vec<OsString>) -> Result<BTreeMap<String, OsString>, String> {
    let mut options = BTreeMap::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let option = arg.to_str().ok_or("option names must be UTF-8")?;
        if !matches!(option, "--output" | "--template" | "--seed" | "--game-id" | "--tileset-package" | "--tile-width" | "--tile-height" | "--sprite-package" | "--idle-clip" | "--walk-clip" | "--units-per-pixel" | "--room-layout") {
            return Err(format!("unknown option {option}; use --help for usage"));
        }
        if options.contains_key(option) {
            return Err(format!("{option} may be supplied only once"));
        }
        let value = args
            .next()
            .ok_or_else(|| format!("{option} requires a value"))?;
        if value.to_str().is_some_and(|v| v.starts_with("--")) {
            return Err(format!("{option} requires a value"));
        }
        options.insert(option.into(), value);
    }
    for key in ["--output", "--template", "--seed"] {
        if !options.contains_key(key) {
            return Err(format!("{key} is required"));
        }
    }
    Ok(options)
}
