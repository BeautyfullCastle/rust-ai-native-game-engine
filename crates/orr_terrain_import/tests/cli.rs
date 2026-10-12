mod common;
use common::*;
use orr_terrain::Terrain;
use orr_terrain_import::{publish::import_file, Endian};
use std::{
    ffi::OsString,
    fs,
    path::Path,
    process::{Command, Output},
};
fn args(input: &Path, output: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "--width",
        "3",
        "--depth",
        "2",
        "--endian",
        "le",
        "--height-offset",
        "-1",
        "--height-scale",
        "1",
        "--origin-x",
        "-1",
        "--origin-z",
        "-1",
        "--spacing",
        "1",
        "--asset-id",
        "terrain/imported",
        "--input",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    args.push(input.into());
    args.push("--output".into());
    args.push(output.into());
    args
}
fn cli(args: &[OsString]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_orr_terrain_import"))
        .args(args)
        .output()
        .unwrap()
}
fn published(output: &Output) {
    #[cfg(unix)]
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    #[cfg(not(unix))]
    assert_eq!(
        output.status.code(),
        Some(3),
        "unsupported parent sync must be reported"
    );
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("published "));
}
fn fail(output: &Output) {
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}
#[test]
fn actual_cli_imports_canonical_bytes_and_keeps_source_unchanged() {
    let root = temp();
    let input = root.path().join("map.raw");
    let output = root.path().join("map.orrt");
    let bytes = raw(&SAMPLES, Endian::Little);
    fs::write(&input, &bytes).unwrap();
    published(&cli(&args(&input, &output)));
    assert_eq!(fs::read(&input).unwrap(), bytes);
    assert_eq!(
        Terrain::load(&fs::read(&output).unwrap()).unwrap(),
        terrain()
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    let other = temp();
    let second = other.path().join("map.orrt");
    published(&cli(&args(&input, &second)));
    assert_eq!(fs::read(output).unwrap(), fs::read(second).unwrap());
}
#[test]
fn help_and_usage_statuses_are_explicit() {
    let output = cli(&["--help".into()]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("--height-scale"));
    for a in [
        vec![],
        vec!["--what".into()],
        vec!["--help".into(), "--force".into()],
        vec!["--input".into()],
    ] {
        fail(&cli(&a));
    }
}
#[test]
fn duplicate_missing_unknown_and_malformed_parameters_do_not_create_output() {
    let root = temp();
    let input = root.path().join("map.raw");
    fs::write(&input, raw(&SAMPLES, Endian::Little)).unwrap();
    let output = root.path().join("out.orrt");
    let normal = args(&input, &output);
    for (index, value) in [
        (1, "0"),
        (1, "130"),
        (1, "-2"),
        (1, "4294967296"),
        (5, "native"),
        (7, "NaN"),
        (7, "inf"),
        (7, "1e2"),
        (7, "1.2.3"),
        (7, "140737488355328"),
        (9, "-1"),
        (15, "0"),
        (17, "../bad"),
    ] {
        let mut a = normal.clone();
        a[index] = value.into();
        fail(&cli(&a));
        assert!(!output.exists());
    }
    let mut duplicate = normal.clone();
    duplicate.drain(2..4); // Keep the width pair, omit depth, stay below argument cap.
    duplicate.extend(["--width".into(), "3".into()]);
    let duplicate_result = cli(&duplicate);
    fail(&duplicate_result);
    assert!(String::from_utf8_lossy(&duplicate_result.stderr).contains("duplicate option --width"));
    let mut missing = normal.clone();
    missing.drain(0..2);
    fail(&cli(&missing));
    let mut unknown = normal.clone();
    unknown.push("--force".into());
    fail(&cli(&unknown));
    let mut flip = normal;
    flip.drain(0..2); // Parsing must reach the second flag before required-field checks.
    flip.extend(["--flip-z".into(), "--flip-z".into()]);
    let flip_result = cli(&flip);
    fail(&flip_result);
    assert!(String::from_utf8_lossy(&flip_result.stderr).contains("duplicate --flip-z"));
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}
#[test]
fn explicit_big_endian_and_flip_z_are_used_by_actual_binary() {
    let root = temp();
    let input = root.path().join("map.raw");
    fs::write(&input, raw(&SAMPLES, Endian::Big)).unwrap();
    let output = root.path().join("out.orrt");
    let mut a = args(&input, &output);
    a[5] = "be".into();
    a.push("--flip-z".into());
    published(&cli(&a));
    let mut config = recipe();
    config.flip_z = true;
    config.endian = Endian::Big;
    assert_eq!(
        fs::read(output).unwrap(),
        orr_terrain_import::import_raw16(&raw(&SAMPLES, Endian::Big), &config)
            .unwrap()
            .cook()
    );
}
#[test]
fn input_lengths_directories_and_missing_files_are_rejected_without_staging() {
    let root = temp();
    let input = root.path().join("map.raw");
    let output = root.path().join("out.orrt");
    fail(&cli(&args(&input, &output)));
    for len in [0, 11, 13, 33_283] {
        fs::write(&input, vec![0; len]).unwrap();
        fail(&cli(&args(&input, &output)));
        assert!(!output.exists());
    }
    fail(&cli(&args(root.path(), &output)));
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}
#[test]
fn no_clobber_aliases_traversal_package_store_and_missing_parent_are_rejected() {
    let root = temp();
    let input = root.path().join("map.orrt");
    let raw = raw(&SAMPLES, Endian::Little);
    fs::write(&input, &raw).unwrap();
    let output = root.path().join("out.orrt");
    fs::write(&output, b"existing output").unwrap();
    fail(&cli(&args(&input, &output)));
    assert_eq!(fs::read(&output).unwrap(), b"existing output");
    fail(&cli(&args(&input, &input)));
    assert_eq!(fs::read(&input).unwrap(), raw);
    let alias = root.path().join("alias.orrt");
    fs::hard_link(&input, &alias).unwrap();
    fail(&cli(&args(&input, &alias)));
    assert_eq!(fs::read(alias).unwrap(), raw);
    let store = root.path().join(".orr");
    fs::create_dir(&store).unwrap();
    fs::write(store.join("keep"), b"package bytes").unwrap();
    fail(&cli(&args(&input, &store.join("new.orrt"))));
    assert_eq!(fs::read(store.join("keep")).unwrap(), b"package bytes");
    assert_eq!(fs::read_dir(&store).unwrap().count(), 1);
    fail(&cli(&args(&input, &root.path().join("missing/new.orrt"))));
    assert!(!root.path().join("missing").exists());
    fs::create_dir(root.path().join("sub")).unwrap();
    // Path::join normalizes away `..` on Windows verbatim temp paths.
    // Preserve the untrusted spelling with native separators until CLI parsing.
    let mut traversal = root.path().as_os_str().to_os_string();
    for part in ["sub", "..", "new.orrt"] {
        traversal.push(std::path::MAIN_SEPARATOR_STR);
        traversal.push(part);
    }
    let traversal_args = args(&input, Path::new(&traversal));
    assert!(
        Path::new(traversal_args.last().unwrap())
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir)),
        "the output argument must retain parent traversal at the CLI boundary"
    );
    let result = cli(&traversal_args);
    fail(&result);
    assert!(String::from_utf8_lossy(&result.stderr)
        .contains("empty paths and parent traversal are not allowed"));
    assert!(!root.path().join("new.orrt").exists());
    assert_eq!(fs::read_dir(root.path().join("sub")).unwrap().count(), 0);
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 5);
    assert_eq!(fs::read(&input).unwrap(), raw);
}
#[test]
fn output_basename_is_portable_for_existing_editor_open() {
    let root = temp();
    let input = root.path().join("map.raw");
    fs::write(&input, raw(&SAMPLES, Endian::Little)).unwrap();
    for name in [
        ".orrt",
        "not_raw.bin",
        "my terrain.orrt",
        "CON.orrt",
        "COM0.orrt",
        "LPT9.orrt",
        "has:colon.orrt",
        "雪.orrt",
    ] {
        fail(&cli(&args(&input, &root.path().join(name))));
    }
    let spaced = root.path().join("scene directory");
    fs::create_dir(&spaced).unwrap();
    published(&cli(&args(&input, &spaced.join("good.orrt"))));
}
#[test]
fn independent_cli_processes_race_one_destination_without_overwriting() {
    let root = temp();
    let input = root.path().join("map.raw");
    fs::write(&input, raw(&SAMPLES, Endian::Little)).unwrap();
    let output = root.path().join("out.orrt");
    let mut children = Vec::new();
    for _ in 0..4 {
        children.push(
            Command::new(env!("CARGO_BIN_EXE_orr_terrain_import"))
                .args(args(&input, &output))
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    let statuses: Vec<_> = children
        .into_iter()
        .map(|child| child.wait_with_output().unwrap())
        .collect();
    assert_eq!(
        statuses
            .iter()
            .filter(|s| matches!(s.status.code(), Some(0) | Some(3)))
            .count(),
        1
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|s| s.status.code() == Some(2))
            .count(),
        3
    );
    assert_eq!(fs::read(output).unwrap(), terrain().cook());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
}
#[cfg(unix)]
#[test]
fn symlinks_dangling_links_and_fifo_are_rejected_before_open() {
    use std::os::unix::fs::symlink;
    let root = temp();
    let input = root.path().join("map.raw");
    fs::write(&input, raw(&SAMPLES, Endian::Little)).unwrap();
    let output = root.path().join("out.orrt");
    let link = root.path().join("link.raw");
    symlink(&input, &link).unwrap();
    fail(&cli(&args(&link, &output)));
    symlink(root.path().join("absent"), &output).unwrap();
    fail(&cli(&args(&input, &output)));
    assert!(fs::symlink_metadata(&output)
        .unwrap()
        .file_type()
        .is_symlink());
    fs::remove_file(&output).unwrap();
    let dir_link = root.path().join("directory-link");
    symlink(root.path(), &dir_link).unwrap();
    fail(&cli(&args(&input, &dir_link.join("out.orrt"))));
    fail(&cli(&args(&dir_link.join("map.raw"), &output)));
    let fifo = root.path().join("pipe.raw");
    assert!(Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap()
        .success());
    fail(&cli(&args(&fifo, &output)));
    assert!(!output.exists());
    assert_eq!(fs::read(input).unwrap(), raw(&SAMPLES, Endian::Little));
}
#[test]
fn public_file_api_rejects_display_failure_before_staging() {
    let root = temp();
    let input = root.path().join("map.raw");
    fs::write(&input, raw(&SAMPLES, Endian::Little)).unwrap();
    let output = root.path().join("out.orrt");
    let mut config = recipe();
    config.height_offset = orr_fp::FP::from_raw(16_777_217);
    config.height_scale = orr_fp::FP::ZERO;
    assert!(import_file(&input, &output, &config).is_err());
    assert!(!output.exists());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

// Separate default-on hosted-native coverage. A sandbox unable to create Unix
// sockets must report this case blocked, not silently turn it into a success.
#[cfg(unix)]
#[test]
fn socket_input_is_rejected_before_open() {
    use std::os::unix::net::UnixListener;
    let root = temp();
    let socket = root.path().join("socket");
    let output = root.path().join("out.orrt");
    let _listener = UnixListener::bind(&socket).expect("socket fixture creation unavailable");
    fail(&cli(&args(&socket, &output)));
    assert!(!output.exists());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}
