//! Scene settings: ranges, saving and loading.
#![allow(clippy::float_arithmetic)]

use orr_scenery::settings::{COLORS, GROUPS, SLIDERS, TOGGLES};
use orr_scenery::{Preset, Quality, Settings};

#[test]
fn defaults_are_inside_every_range() {
    let defaults = Settings::default();
    assert_eq!(defaults.sanitized(), defaults);
    for slider in SLIDERS {
        let value = (slider.get)(&defaults);
        assert!(slider.min < slider.max && (slider.min..=slider.max).contains(&value), "{} {}: {value}", slider.group, slider.name);
    }
    for color in COLORS {
        assert!((color.get)(&defaults).iter().all(|c| (0.0..=color.max).contains(c)), "{} {}", color.group, color.name);
    }
}

#[test]
fn every_control_has_a_known_group_a_unique_name_and_help() {
    let mut names: Vec<(&str, &str)> = SLIDERS.iter().map(|s| (s.group, s.name)).collect();
    names.extend(TOGGLES.iter().map(|t| (t.group, t.name)));
    names.extend(COLORS.iter().map(|c| (c.group, c.name)));
    assert!(names.iter().all(|(group, _)| GROUPS.contains(group)));
    assert!(GROUPS.iter().all(|group| names.iter().any(|(g, _)| g == group)), "a group has no control");
    let count = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), count, "two controls share a name");
    let help = SLIDERS.iter().map(|s| s.help).chain(TOGGLES.iter().map(|t| t.help)).chain(COLORS.iter().map(|c| c.help));
    assert!(help.into_iter().all(|text| text.len() > 10 && text.ends_with('.')));
}

#[test]
fn each_control_writes_the_value_it_reads() {
    for slider in SLIDERS {
        let mut settings = Settings::default();
        let value = slider.min + (slider.max - slider.min) * 0.25;
        (slider.set)(&mut settings, value);
        assert_eq!((slider.get)(&settings), value, "{} {}", slider.group, slider.name);
        // One slider changes one number.
        let changed = SLIDERS.iter().filter(|other| (other.get)(&settings) != (other.get)(&Settings::default())).count();
        assert!(changed <= 1, "{} {} changed {changed} values", slider.group, slider.name);
    }
    for toggle in TOGGLES {
        let mut settings = Settings::default();
        let flipped = !(toggle.get)(&settings);
        (toggle.set)(&mut settings, flipped);
        assert_eq!((toggle.get)(&settings), flipped, "{} {}", toggle.group, toggle.name);
    }
    for color in COLORS {
        let mut settings = Settings::default();
        (color.set)(&mut settings, [0.1, 0.2, 0.3]);
        assert_eq!((color.get)(&settings), [0.1, 0.2, 0.3], "{} {}", color.group, color.name);
    }
}

#[test]
fn saved_settings_load_back_unchanged() {
    let mut settings = Settings::default();
    for (index, slider) in SLIDERS.iter().enumerate() {
        let t = 0.1 + 0.8 * (index as f32 * 0.37).fract();
        (slider.set)(&mut settings, slider.min + (slider.max - slider.min) * t);
    }
    settings.shadows.enabled = false;
    settings.look.auto_exposure = false;
    settings.water.color = [0.2, 0.1, 0.05];
    let settings = settings.sanitized();
    assert_eq!(Settings::from_json(&settings.to_json()), Ok(settings));
}

#[test]
fn a_partial_file_takes_defaults_for_the_rest() {
    let loaded = Settings::from_json(r#"{ "sun": { "hour": 9.5 }, "look": { "bloom": 0.4 } }"#).unwrap();
    let mut expected = Settings::default();
    expected.sun.hour = 9.5;
    expected.look.bloom = 0.4;
    assert_eq!(loaded, expected);
    assert_eq!(Settings::from_json("{}"), Ok(Settings::default()));
}

#[test]
fn values_outside_their_range_are_clamped_and_bad_files_are_rejected() {
    let loaded = Settings::from_json(r#"{ "sun": { "hour": 99.0, "intensity": -4.0 }, "water": { "color": [9.0, -1.0, 0.1] } }"#).unwrap();
    assert_eq!((loaded.sun.hour, loaded.sun.intensity), (19.0, 0.0));
    assert_eq!(loaded.water.color, [0.5, 0.0, 0.1]);

    let mut broken = Settings::default();
    broken.fog.density = f32::NAN;
    broken.vegetation.tint = [f32::INFINITY, 1.0, f32::NAN];
    broken.shadows.near_range = 120.0;
    broken.shadows.far_range = 150.0;
    let fixed = broken.sanitized();
    assert_eq!(fixed.fog.density, Settings::default().fog.density);
    assert_eq!(fixed.vegetation.tint, [1.0, 1.0, 1.0]);
    assert!(fixed.shadows.far_range >= fixed.shadows.near_range * 2.0);

    assert!(Settings::from_json("not json").is_err());
    assert!(Settings::from_json(r#"{ "sun": { "hour": "noon" } }"#).is_err());
}

#[test]
fn the_sun_is_high_at_noon_and_low_in_the_evening() {
    let at = |hour: f32| {
        let mut settings = Settings::default();
        settings.sun.hour = hour;
        settings.sun_direction()
    };
    let length = |v: [f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    assert!(at(12.0)[1] > 0.85 && at(17.0)[1] < at(14.0)[1] && at(18.5)[1] < 0.0);
    assert!((5..=19).all(|hour| (length(at(hour as f32)) - 1.0).abs() < 1e-4));
}

#[test]
fn quality_rounds_to_supported_values() {
    assert_eq!(Quality::default().sanitized(), Quality::default());
    assert_eq!(Quality { antialiasing: 3, shadow_resolution: 3000 }.sanitized(), Quality { antialiasing: 2, shadow_resolution: 2048 });
    assert_eq!(Quality { antialiasing: 0, shadow_resolution: 0 }.sanitized(), Quality { antialiasing: 1, shadow_resolution: 1024 });
    assert_eq!(Quality { antialiasing: 64, shadow_resolution: 1 << 20 }.sanitized(), Quality { antialiasing: 4, shadow_resolution: 4096 });
}

#[test]
fn a_preset_keeps_seed_quality_and_settings_through_a_file() {
    let mut preset = Preset { seed: 31, quality: Quality { antialiasing: 2, shadow_resolution: 1024 }, settings: Settings::default() };
    preset.settings.sun.hour = 7.25;
    preset.settings.vegetation.tint = [1.2, 0.9, 0.8];
    preset.settings.shadows.enabled = false;
    assert_eq!(Preset::from_json(&preset.to_json()), Ok(preset));
    assert_eq!(Preset::from_json("{}"), Ok(Preset::default()));
    // Values outside their range are brought inside it.
    let loaded = Preset::from_json(r#"{ "seed": 4, "quality": { "antialiasing": 3 }, "settings": { "fog": { "density": 50.0 } } }"#).unwrap();
    assert_eq!((loaded.seed, loaded.quality.antialiasing, loaded.settings.fog.density), (4, 2, 5.0));
    assert!(Preset::from_json(r#"{ "seed": -1 }"#).is_err());
    // A file with other top-level entries is not a preset.
    assert!(Preset::from_json(r#"{ "sun": { "hour": 9.0 } }"#).is_err());
}
