use orr_sprite::{
    grid::{generate, GridClip, GridSpec},
    Atlas, PlaybackMode, SpriteDocument,
};
use std::{
    ffi::OsString,
    process::{Command, Output},
};

fn args() -> Vec<OsString> {
    [
        "--image",
        "missing/hero.png",
        "--atlas-width",
        "37",
        "--atlas-height",
        "23",
        "--columns",
        "3",
        "--rows",
        "2",
        "--cell-width",
        "7",
        "--cell-height",
        "5",
        "--clip",
        "walk",
        "--frame-count",
        "4",
        "--duration-ms",
        "13",
        "--mode",
        "loop",
        "--offset-x",
        "4",
        "--offset-y",
        "3",
        "--spacing-x",
        "2",
        "--spacing-y",
        "4",
        "--first-region-id",
        "91",
        "--first-cell",
        "1",
    ]
    .map(OsString::from)
    .into()
}
fn run(args: &[OsString]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_orr_sprite_grid"))
        .args(args)
        .output()
        .unwrap()
}
fn invalid(args: &[OsString]) {
    let output = run(args);
    assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
    assert!(output.stdout.is_empty(), "error emitted stdout: {args:?}");
    assert!(!output.stderr.is_empty());
}
fn replace(args: &mut [OsString], flag: &str, value: impl Into<OsString>) {
    let i = args.iter().position(|a| a == flag).unwrap();
    args[i + 1] = value.into();
}

#[test]
fn cli_is_deterministic_exact_api_json_with_one_newline() {
    for mode in [PlaybackMode::Loop, PlaybackMode::Once] {
        let mut a = args();
        replace(
            &mut a,
            "--mode",
            if mode == PlaybackMode::Loop {
                "loop"
            } else {
                "once"
            },
        );
        let expected = generate(
            Atlas {
                image: "missing/hero.png".into(),
                width: 37,
                height: 23,
            },
            GridSpec {
                columns: 3,
                rows: 2,
                cell_width: 7,
                cell_height: 5,
                offset_x: 4,
                offset_y: 3,
                spacing_x: 2,
                spacing_y: 4,
                first_region_id: 91,
            },
            GridClip {
                name: "walk".into(),
                first_cell: 1,
                frame_count: 4,
                duration_ms: 13,
                mode,
            },
        )
        .unwrap()
        .to_json()
        .unwrap()
            + "\n";
        let out = run(&a);
        assert!(out.status.success());
        assert!(out.stderr.is_empty());
        assert_eq!(out.stdout, expected.as_bytes());
        assert_eq!(run(&a).stdout, out.stdout);
        assert_eq!(
            SpriteDocument::from_json(&expected).unwrap().clips()[0].mode(),
            mode
        );
    }
}

#[test]
fn unicode_cli_names_preserve_exact_byte_boundaries() {
    let mut a = args();
    let image = "é".repeat(512);
    let clip = "é".repeat(64);
    replace(&mut a, "--image", image.as_str());
    replace(&mut a, "--clip", clip.as_str());
    let out = run(&a);
    assert!(out.status.success());
    assert!(out.stderr.is_empty());
    let doc = SpriteDocument::from_json(std::str::from_utf8(&out.stdout).unwrap()).unwrap();
    assert_eq!(doc.atlas().image, image);
    assert_eq!(doc.clips()[0].id(), clip);
    replace(&mut a, "--image", image.clone() + "x");
    invalid(&a);
    replace(&mut a, "--image", image);
    replace(&mut a, "--clip", clip + "x");
    invalid(&a);
}

#[test]
fn optional_defaults_and_argument_order_are_explicit() {
    let mut a = args();
    a.truncate(22);
    let out = run(&a);
    assert!(out.status.success());
    let doc = SpriteDocument::from_json(std::str::from_utf8(&out.stdout).unwrap()).unwrap();
    assert_eq!(doc.regions()[0].id, 0);
    assert_eq!(doc.regions()[0].x, 0);
    assert_eq!(doc.regions()[0].y, 0);
    assert_eq!(doc.regions()[3].y, 5);
    assert_eq!(doc.clips()[0].frames()[0].region, 0);
    let reversed: Vec<_> = a
        .chunks_exact(2)
        .rev()
        .flat_map(|p| p.iter().cloned())
        .collect();
    assert_eq!(run(&reversed).stdout, out.stdout);
}

#[test]
fn help_is_standalone_and_never_hides_bad_arguments() {
    let out = run(&["--help".into()]);
    assert!(out.status.success());
    assert!(out.stderr.is_empty());
    assert!(std::str::from_utf8(&out.stdout)
        .unwrap()
        .starts_with("Usage: orr_sprite_grid"));
    invalid(&["--help".into(), "--unknown".into()]);
    let mut a = args();
    a.push("--help".into());
    invalid(&a);
}

#[test]
fn missing_duplicate_unknown_and_positional_arguments_reject() {
    invalid(&[]);
    let original = args();
    for i in (0..22).step_by(2) {
        let mut a = original.clone();
        a.drain(i..i + 2);
        invalid(&a);
    }
    for i in (0..original.len()).step_by(2) {
        let mut a = original.clone();
        a.extend_from_slice(&original[i..i + 2]);
        invalid(&a);
    }
    let mut a = original.clone();
    a.pop();
    invalid(&a);
    for bad in ["--output", "--atlas-width=37", "--", "file.json", "-h"] {
        let mut a = original.clone();
        a.push(bad.into());
        invalid(&a);
    }
}

#[test]
fn malformed_numbers_modes_and_schema_invalid_inputs_reject() {
    for flag in [
        "--atlas-width",
        "--atlas-height",
        "--columns",
        "--rows",
        "--cell-width",
        "--cell-height",
        "--frame-count",
        "--duration-ms",
        "--offset-x",
        "--offset-y",
        "--spacing-x",
        "--spacing-y",
        "--first-region-id",
        "--first-cell",
    ] {
        for bad in [
            "",
            "-1",
            "+1",
            "1.0",
            " 1",
            "1 ",
            "0x10",
            "١",
            "18446744073709551616",
        ] {
            let mut a = args();
            replace(&mut a, flag, bad);
            invalid(&a);
        }
    }
    for (flag, bad) in [
        ("--atlas-width", "2049"),
        ("--atlas-height", "0"),
        ("--columns", "4294967296"),
        ("--rows", "4294967295"),
        ("--cell-width", "0"),
        ("--spacing-x", "4294967295"),
        ("--first-region-id", "4294967295"),
        ("--first-cell", "4294967295"),
        ("--frame-count", "0"),
        ("--frame-count", "4097"),
        ("--duration-ms", "0"),
        ("--duration-ms", "18446744073709551615"),
        ("--mode", "Loop"),
        ("--mode", "ping-pong"),
        ("--clip", " "),
        ("--image", ""),
    ] {
        let mut a = args();
        replace(&mut a, flag, bad);
        invalid(&a);
    }
    for (flag, length) in [("--clip", 129), ("--image", 1025), ("--duration-ms", 1025)] {
        let mut a = args();
        replace(&mut a, flag, "x".repeat(length));
        invalid(&a);
    }
}

#[cfg(any(unix, windows))]
#[test]
fn non_utf8_flags_and_values_reject_without_panicking() {
    #[cfg(unix)]
    let bad = {
        use std::os::unix::ffi::OsStringExt;
        OsString::from_vec(vec![0xff])
    };
    #[cfg(windows)]
    let bad = {
        use std::os::windows::ffi::OsStringExt;
        OsString::from_wide(&[0xd800])
    };
    let mut a = args();
    a[0] = bad.clone();
    invalid(&a);
    let mut a = args();
    a[1] = bad;
    invalid(&a);
}

#[test]
fn image_metadata_never_reads_or_mutates_files() {
    // Exclusive per-process directory; no dependence on clocks or external crates.
    let dir = std::env::temp_dir().join(format!("orr-sprite-grid-cli-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let image = dir.join("hero.png");
    let sentinel = dir.join("existing.json");
    std::fs::write(&image, b"not an image").unwrap();
    std::fs::write(&sentinel, b"leave untouched").unwrap();
    let mut a = args();
    replace(&mut a, "--image", image.as_os_str());
    let out = Command::new(env!("CARGO_BIN_EXE_orr_sprite_grid"))
        .args(&a)
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(out.stderr.is_empty());
    assert_eq!(std::fs::read(&image).unwrap(), b"not an image");
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"leave untouched");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
    let missing = dir.join("absent.png");
    replace(&mut a, "--image", missing.as_os_str());
    assert!(run(&a).status.success());
    assert!(!missing.exists());
    replace(&mut a, "--rows", "0");
    invalid(&a);
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
    std::fs::remove_file(image).unwrap();
    std::fs::remove_file(sentinel).unwrap();
    std::fs::remove_dir(dir).unwrap();
}
