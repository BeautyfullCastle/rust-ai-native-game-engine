use super::*;
const ACTOR: &[u8] = include_bytes!("../../../../assets/sprite_demo/lantern_keeper.rgba");
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect()
}
fn runtime(root: &Path) -> PathBuf {
    let dir = root.join("supplied runtime");
    fs::create_dir(&dir).unwrap();
    let path = dir.join("arena");
    let source = std::env::var_os("ORR_CUSTOM_TILESET_ARENA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_arena")));
    assert!(source.is_absolute() && source.is_file());
    fs::copy(source, &path).unwrap();
    path
}
fn export(project: &Path, runtime: &Path, output: &Path) {
    let binary = std::env::var_os("ORR_CUSTOM_TILESET_EXPORTER")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_orr_export_arena")));
    success(
        Command::new(binary)
            .arg("--project")
            .arg(project)
            .arg("--runtime")
            .arg(runtime)
            .arg("--runtime-sha256")
            .arg(hash(&fs::read(runtime).unwrap()))
            .arg("--trusted-runtime")
            .arg("--output")
            .arg(output)
            .output()
            .unwrap(),
    );
}
fn run(bundle: &Path, cwd: &Path, capture: Option<&Path>) -> String {
    let mut command = Command::new(bundle.join("run-arena"));
    command
        .args(if capture.is_some() {
            ["--headless", "--ticks", "0", "--hold", "idle"]
        } else {
            ["--headless", "--ticks", "60", "--hold", "right"]
        })
        .current_dir(cwd)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY");
    if let Some(path) = capture {
        command.arg("--capture").arg(path);
    }
    success(command.output().unwrap())
}
fn state(output: &str) -> Vec<String> {
    output
        .lines()
        .filter(|line| {
            line.starts_with("project initial checksum:") || line.starts_with("project tick:")
        })
        .map(str::to_owned)
        .collect()
}
fn pixels(path: &Path) -> Vec<u8> {
    let mut reader = png17::Decoder::new(fs::File::open(path).unwrap())
        .read_info()
        .unwrap();
    let mut data = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut data).unwrap();
    assert_eq!((info.width, info.height), (512, 512));
    assert_eq!(info.color_type, png17::ColorType::Rgba);
    data.truncate(info.buffer_size());
    data
}
fn save_fixture_map(project: &Path) {
    // Deliberately a saved-file fixture, not a claim of Editor authoring.
    let path = project.join("arena.tilemap.json");
    let mut map = read_json(&path);
    map["width"] = json!(3);
    map["height"] = json!(2);
    map["origin"] = json!([-200, 160]);
    map["cells"] = json!([31, null, 47, 47, 31, null]);
    write_json(&path, &map);
}
#[test]
fn actual_export_preserves_custom_package_and_admits_after_sources_move() {
    let f = Fixture::new();
    f.create();
    save_fixture_map(&f.project);
    let initial = PreparedRuntime::open(&f.project)
        .unwrap()
        .initial_frame()
        .checksum();
    let runtime = runtime(f.temp.path());
    let output = f.temp.path().join("export");
    export(&f.project, &runtime, &output);
    let moved = f.temp.path().join("relocated export");
    fs::rename(&output, &moved).unwrap();
    assert_eq!(
        fs::read(moved.join("project/arena.tilemap.json")).unwrap(),
        fs::read(f.project.join("arena.tilemap.json")).unwrap()
    );
    let lock = read_json(&f.project.join("orr.packages.lock.json"));
    let object = PathBuf::from(".orr/packages/objects")
        .join(lock["packages"][PACKAGE]["digest"].as_str().unwrap());
    for name in ["sprites.json", "atlas.png", "LICENSE.txt"] {
        assert_eq!(
            fs::read(moved.join("project").join(&object).join(name)).unwrap(),
            fs::read(f.source.join(name)).unwrap()
        );
    }
    fs::rename(&f.source, f.temp.path().join("package hidden")).unwrap();
    fs::rename(&f.project, f.temp.path().join("project hidden")).unwrap();
    fs::rename(
        runtime.parent().unwrap(),
        f.temp.path().join("runtime hidden"),
    )
    .unwrap();
    assert!(!output.exists() && !f.source.exists() && !f.project.exists() && !runtime.exists());
    assert_eq!(
        PreparedRuntime::open(moved.join("project"))
            .unwrap()
            .initial_frame()
            .checksum(),
        initial
    );
    let text = run(&moved, f.temp.path(), None);
    assert_eq!(state(&text).len(), 2);
    assert!(text.contains(&format!("project initial checksum: 0x{initial:016x}")));
    assert!(text.contains("project tick: 60 checksum:"));
}
fn close(actual: &[u8], expected: &[u8]) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(a, b)| a.abs_diff(*b) <= 3)
}
// Independent literal copy of the Editor acceptance atlas design. No production
// extraction or UV helper participates in this pixel oracle.
fn editor_texel(id: u32, x: usize, y: usize) -> [u8; 4] {
    match (id, x, y) {
        (0, 0..=1, _) => [241, 37, 79, 255],
        (0, _, 0..=2) => [31, 211, 137, 255],
        (0, _, _) => [37, 73, 229, 255],
        (73, _, 9..=11) => [233, 191, 29, 255],
        (73, 5..=7, _) => [173, 41, 211, 255],
        (73, _, _) => [23, 193, 223, 255],
        _ => panic!("unknown Editor fixture region"),
    }
}
fn check_tiles(images: &[Vec<u8>], map: &orr_tilemap::Document) -> (usize, usize) {
    let mut checked = 0;
    let mut distinguished = 0;
    let width = map.tile_size[0];
    let height = map.tile_size[1];
    for y in 0..512 {
        for x in 0..512 {
            // Independent literal camera, tile orientation and fixture-atlas oracle.
            let wx = (x as f64 + 0.5 - 256.) * 480. / 512. - 60.;
            let wy = 240. - (y as f64 + 0.5) * 480. / 512.;
            let tx = wx - f64::from(map.origin[0]);
            let ty = f64::from(map.origin[1]) - wy;
            if tx < 0.
                || ty < 0.
                || tx >= f64::from(map.width * width)
                || ty >= f64::from(map.height * height)
            {
                continue;
            }
            // Exclude actor circles; tile and actor checks are independently counted.
            if [-60., 60.].iter().any(|cx| (wx - cx).hypot(wy) < 24.) {
                continue;
            }
            let px = tx % f64::from(width);
            let py = ty % f64::from(height);
            if !(1.0..f64::from(width - 1)).contains(&px)
                || !(1.0..f64::from(height - 1)).contains(&py)
            {
                continue;
            }
            let index =
                ty as usize / height as usize * map.width as usize + tx as usize / width as usize;
            let Some(region) = map.cells[index] else {
                continue;
            };
            let expected = if map.sprite.package == PACKAGE {
                texel(region, px.floor() as usize, py.floor() as usize)
            } else {
                editor_texel(region, px.floor() as usize, py.floor() as usize)
            };
            let p = (y * 512 + x) * 4;
            assert!(
                close(&images[0][p..p + 4], &expected),
                "custom tile ({x},{y}) {:?} != {expected:?}",
                &images[0][p..p + 4]
            );
            checked += 1;
            if images[0][p..p + 4] != images[1][p..p + 4]
                && images[0][p..p + 4] != images[2][p..p + 4]
            {
                distinguished += 1;
            }
        }
    }
    assert!(checked >= 40, "insufficient literal tile texels: {checked}");
    assert!(
        distinguished >= 40,
        "controls do not distinguish tiles: {distinguished}"
    );
    (checked, distinguished)
}
fn check_actors(images: &[Vec<u8>]) -> usize {
    let mut checked = [0; 2];
    let mut distinguished = 0;
    for y in 0..512 {
        for x in 0..512 {
            let wx = (x as f64 + 0.5 - 256.) * 480. / 512. - 60.;
            let wy = 240. - (y as f64 + 0.5) * 480. / 512.;
            for (actor, cx) in [-60., 60.].into_iter().enumerate() {
                let sx = (wx - cx + 16.) / 2.;
                let sy = (16. - wy) / 2.;
                if !(1.0..15.0).contains(&sx) || !(1.0..15.0).contains(&sy) {
                    continue;
                }
                let offset = (sy.floor() as usize * 64 + sx.floor() as usize) * 4;
                let expected = &ACTOR[offset..offset + 4];
                if expected[3] != 255 {
                    continue;
                }
                let p = (y * 512 + x) * 4;
                for image in &images[..3] {
                    assert!(
                        close(&image[p..p + 4], expected),
                        "actor {actor} ({x},{y}) {:?} != {expected:?}",
                        &image[p..p + 4]
                    );
                }
                if images[0][p..p + 4] != images[3][p..p + 4] {
                    distinguished += 1;
                }
                checked[actor] += 1;
            }
        }
    }
    assert!(
        checked.iter().all(|n| *n >= 10),
        "opaque actor coverage {checked:?}"
    );
    assert!(distinguished >= 10, "missing actor control ineffective");
    checked.iter().sum()
}
fn capture_delivery(project: &Path, work: &Path, evidence: &Path) {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    assert!(evidence.is_absolute() && evidence.is_dir());
    let map_bytes = fs::read(project.join("arena.tilemap.json")).unwrap();
    let map: orr_tilemap::Document = serde_json::from_slice(&map_bytes).unwrap();
    assert_eq!(map.units_per_pixel, 1.);
    if map.sprite.package == PACKAGE {
        assert_eq!(map.tile_size, [16, 12]);
    } else {
        assert_eq!(map.sprite.package, "acceptance-custom-tiles");
        assert_eq!(map.tile_size, [8, 12]);
        assert_eq!((map.width, map.height), (16, 12));
        assert_eq!(map.origin, [-64., 72.]);
        assert_eq!(&map.cells[..2], &[Some(0), Some(73)]);
        assert!(map.cells[2..].iter().all(Option::is_none));
    }

    let source_before = files(project);
    let runtime = runtime(work);
    let mut bundles = Vec::new();
    for name in ["saved", "empty-map", "wrong-region", "no-actors"] {
        let source = work.join(format!("{name}-source"));
        copy(project, &source);
        if name == "empty-map" || name == "wrong-region" {
            let mut changed = map.clone();
            for cell in &mut changed.cells {
                *cell = if name == "empty-map" {
                    None
                } else {
                    cell.map(|id| match id {
                        31 => 47,
                        47 => 31,
                        0 => 73,
                        73 => 0,
                        _ => panic!("fixture region"),
                    })
                };
            }
            fs::write(
                source.join("arena.tilemap.json"),
                serde_json::to_vec(&changed).unwrap(),
            )
            .unwrap();
        }
        if name == "no-actors" {
            let path = source.join("orr.project.json");
            let mut manifest = read_json(&path);
            manifest["entry"].as_object_mut().unwrap().remove("sprites");
            write_json(&path, &manifest);
        }
        let output = work.join(format!("{name}-export"));
        export(&source, &runtime, &output);
        let moved = work.join(format!("{name}-relocated"));
        fs::rename(&output, &moved).unwrap();
        fs::rename(&source, work.join(format!("{name}-source-hidden"))).unwrap();
        assert!(!source.exists() && !output.exists());
        if name == "saved" {
            assert_eq!(
                fs::read(moved.join("project/arena.tilemap.json")).unwrap(),
                map_bytes
            );
        }
        bundles.push(moved);
    }
    assert_eq!(files(project), source_before, "export changed saved source");
    fs::rename(runtime.parent().unwrap(), work.join("runtime-hidden")).unwrap();
    assert!(!runtime.exists());
    // Hide the owned working source. Caller owns it, including Editor handoff copies.
    fs::rename(project, work.join("owned-authoring-source-hidden")).unwrap();
    assert!(!project.exists());
    let cwd = work.join("empty cwd");
    fs::create_dir(&cwd).unwrap();
    let mut images = Vec::new();
    let mut states = Vec::new();
    let mut stepped = Vec::new();
    for (name, bundle) in ["saved", "empty-map", "wrong-region", "no-actors"]
        .into_iter()
        .zip(bundles)
    {
        let capture = evidence.join(format!("{name}.png"));
        let text = run(&bundle, &cwd, Some(&capture));
        assert!(text.contains("project capture adapter:"));
        states.push(state(&text));
        fs::write(evidence.join(format!("{name}.stdout")), text).unwrap();
        images.push(pixels(&capture));
        let text = run(&bundle, &cwd, None);
        assert!(text.contains("project tick: 60 checksum:"));
        stepped.push(state(&text));
        fs::write(evidence.join(format!("{name}-tick60.stdout")), text).unwrap();
    }
    assert_eq!(states[0].len(), 2);
    assert_eq!(stepped[0].len(), 2);
    for i in 1..4 {
        assert_eq!(states[0], states[i]);
        assert_eq!(stepped[0], stepped[i]);
    }
    let (tiles, controls) = check_tiles(&images, &map);
    let actors = check_actors(&images);
    fs::write(evidence.join("saved-tilemap.json"), map_bytes).unwrap();
    fs::write(evidence.join("oracle.txt"),format!("literal_custom_texels={tiles}\ncontrol_differences={controls}\nopaque_bundled_actor_texels={actors}\nall_initial_and_tick60_states_equal=true\nordinary_owned_source_relocation_not_sandbox_isolation\n")).unwrap();
}
#[test]
#[ignore = "explicit real GPU capture of custom tileset saved-file exports"]
fn relocated_custom_export_captures_literal_tiles_actors_and_controls() {
    let evidence = PathBuf::from(
        std::env::var_os("ORR_CUSTOM_TILESET_EVIDENCE")
            .expect("retained evidence directory required"),
    );
    let f = Fixture::new();
    f.create();
    save_fixture_map(&f.project);
    fs::rename(&f.source, f.temp.path().join("package hidden")).unwrap();
    capture_delivery(&f.project, f.temp.path(), &evidence);
}
// Renames only the explicitly supplied, canonical owned Editor fixture. A failed
// assertion restores it during unwind; restoration failure preserves the hidden
// directory rather than deleting the sole original evidence.
struct HiddenEditorProject {
    original: PathBuf,
    hidden: PathBuf,
    active: bool,
}
impl HiddenEditorProject {
    fn hide(original: &Path) -> Self {
        assert!(original.is_absolute() && original.is_dir());
        assert_eq!(
            original.file_name().and_then(|s| s.to_str()),
            Some("created-project"),
            "only the owned Editor acceptance fixture may be hidden"
        );
        assert_eq!(
            original.canonicalize().unwrap(),
            original,
            "symlinks and noncanonical authoring paths are not accepted"
        );
        let parent = original.parent().unwrap();
        assert!(parent.parent().is_some(), "refuse a root-level fixture");
        let hidden = parent.join(format!(
            ".created-project-export-hidden-{}",
            std::process::id()
        ));
        assert!(
            fs::symlink_metadata(&hidden).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
            "hidden destination must be absent"
        );
        fs::rename(original, &hidden).unwrap();
        let guard = Self {
            original: original.to_owned(),
            hidden,
            active: true,
        };
        assert!(!original.exists(), "original authoring path must be absent");
        guard
    }
    fn restore(mut self) {
        assert!(
            !self.original.exists(),
            "refuse to replace recreated authoring path"
        );
        fs::rename(&self.hidden, &self.original).expect("restore original Editor evidence");
        self.active = false;
    }
}
impl Drop for HiddenEditorProject {
    fn drop(&mut self) {
        if self.active {
            if self.original.exists() {
                eprintln!(
                    "Editor evidence retained at {:?}; original path unexpectedly exists",
                    self.hidden
                );
            } else if let Err(error) = fs::rename(&self.hidden, &self.original) {
                eprintln!(
                    "Editor evidence retained at {:?}; restoration failed: {error}",
                    self.hidden
                );
            }
        }
    }
}
#[test]
#[ignore = "actual Editor Save handoff then real relocated exported runtime capture"]
fn actual_editor_saved_project_exports_without_content_edits() {
    let original = PathBuf::from(
        std::env::var_os("ORR_CUSTOM_TILESET_EDITOR_PROJECT")
            .expect("actual Editor Save project required"),
    );
    let evidence = PathBuf::from(
        std::env::var_os("ORR_CUSTOM_TILESET_EDITOR_EXPORT_EVIDENCE")
            .expect("retained export evidence required"),
    );
    assert!(original.is_absolute() && original.is_dir());
    let before = files(&original);
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("exact Editor saved project");
    copy(&original, &project);
    assert_eq!(files(&project), before);
    assert!(
        !evidence.canonicalize().unwrap().starts_with(&original),
        "export evidence must be outside the hidden project"
    );
    let hidden = HiddenEditorProject::hide(&original);
    capture_delivery(&project, temp.path(), &evidence);
    assert!(
        !original.exists(),
        "authoring path reappeared during delivery"
    );
    hidden.restore();
    assert_eq!(files(&original), before);
    fs::write(evidence.join("handoff.txt"),"Exact byte copy of ORR_CUSTOM_TILESET_EDITOR_PROJECT; saved variant has no document edits. Original authoring path, working source and supplied runtime hidden during all captures; original restored byte-for-byte afterward. Ordinary owned-source relocation, not sandbox isolation.\n").unwrap();
}
