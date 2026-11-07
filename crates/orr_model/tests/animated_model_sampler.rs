#![cfg(all(feature = "animation", feature = "import"))]
#![allow(clippy::float_arithmetic)]
#[allow(dead_code)]
#[path = "common/animation_clip_sequence_oracle.rs"]
mod oracle;
use orr_model::{animated_sampler, animation::AnimatedModel, Dependency, Image, Wrap};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
const SOURCE: &str = "inputs/hero.animated.json";
const ID: &str = "variants/blue.sampler.json";
fn recipe() -> Value {
    json!({"format":"orr_animated_material_sampler","version":1,"source":SOURCE,"material":0,"linear_filter":false,"wrap_s":"clamp","wrap_t":"mirror"})
}
fn bytes() -> Vec<u8> {
    serde_json::to_vec(&recipe()).unwrap()
}
fn digest(raw: &[u8]) -> String {
    Sha256::digest(raw)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn fixture() -> AnimatedModel {
    let mut s = oracle::fixture().source().clone();
    let material = s.materials[0].clone();
    s.materials = vec![material.clone(), material.clone(), material];
    s.materials[0].base_color = [0.125, 0.5, 0.75, -0.0];
    s.materials[0].linear_filter = true;
    s.materials[0].wrap_s = Wrap::Mirror;
    s.materials[0].wrap_t = Wrap::Repeat;
    s.materials[1].base_color = [-0.0, 0.75, 0.5, 0.375];
    for p in &mut s.primitives {
        p.material = 0;
        for v in &mut p.vertices {
            v.vertex.uv[0] = -0.0;
        }
    }
    let mut other = s.primitives[0].clone();
    other.material = 1;
    other.id = format!("{}#node={}/mesh=17/primitive=3", s.asset_id, other.node);
    s.primitives.push(other);
    s.nodes[0].rest.translation[2] = -0.0;
    s.images.push(Image {
        width: 1,
        height: 1,
        rgba8: vec![19, 37, 71, 103],
    });
    s.dependencies.push(Dependency {
        uri: "old-history/texture.png".into(),
        sha256: digest(b"history must not be resolved"),
    });
    AnimatedModel::new(s).unwrap()
}
fn apply(r: &[u8], id: &str, raw: &[u8]) -> Result<AnimatedModel, String> {
    animated_sampler::author_sampler(r, id, |path| {
        assert_eq!(path, SOURCE);
        Ok(raw.to_vec())
    })
}
fn reject(r: &[u8], id: &str) {
    assert!(animated_sampler::author_sampler(r, id, |path| panic!(
        "invalid recipe resolved {path}"
    ))
    .is_err());
}
#[test]
fn chosen_sampler_only_preserves_unowned_bits_textures_skin_clips_and_sampled_geometry() {
    let original = fixture();
    let raw = original.to_bytes().unwrap();
    let result = apply(&bytes(), ID, &raw).unwrap();
    let old = original.source();
    let new = result.source();
    assert_eq!(new.asset_id, ID);
    assert!(!new.materials[0].linear_filter);
    assert_eq!(new.materials[0].wrap_s, Wrap::Clamp);
    assert_eq!(new.materials[0].wrap_t, Wrap::Mirror);
    assert_eq!(
        new.materials[0].base_color[3].to_bits(),
        old.materials[0].base_color[3].to_bits()
    );
    let mut restored = new.clone();
    restored.asset_id = old.asset_id.clone();
    restored.dependencies = old.dependencies.clone();
    restored.materials[0].linear_filter = old.materials[0].linear_filter;
    restored.materials[0].wrap_s = old.materials[0].wrap_s;
    restored.materials[0].wrap_t = old.materials[0].wrap_t;
    for (a, b) in old.primitives.iter().zip(&mut restored.primitives) {
        assert_eq!(b.id, format!("{ID}#{}", a.id.split_once('#').unwrap().1));
        b.id = a.id.clone();
    }
    assert_eq!(
        serde_json::to_vec(&restored).unwrap(),
        serde_json::to_vec(old).unwrap(),
        "all unowned data, including signed zero, must survive exactly"
    );
    for clip in 0..old.clips.len() {
        for time in [-1.0, 0.0, 0.125, 0.75, 1.375, 2.0, 3.0] {
            let a = original.sample_clip(clip as u32, time).unwrap();
            let b = result.sample_clip(clip as u32, time).unwrap();
            assert_eq!(a.local(), b.local());
            assert_eq!(a.global(), b.global());
            assert_eq!(a.skin_matrices(), b.skin_matrices());
            let da = original.deform(&a).unwrap();
            let db = result.deform(&b).unwrap();
            assert_eq!(da.len(), db.len());
            for (a, b) in da.iter().zip(db) {
                assert_eq!(a.vertices, b.vertices);
                assert_eq!(a.indices, b.indices);
                assert_eq!(a.material, b.material);
            }
        }
    }
    assert_eq!(original.to_bytes().unwrap(), raw);
    assert_eq!(
        AnimatedModel::from_bytes(&result.to_bytes().unwrap())
            .unwrap()
            .source(),
        new
    );
}
#[test]
fn deterministic_direct_provenance_binds_raw_inputs_without_resolving_history() {
    let raw = fixture().to_bytes().unwrap();
    let r = bytes();
    let mut calls = Vec::new();
    let model = animated_sampler::author_sampler(&r, ID, |path| {
        calls.push(path.to_owned());
        Ok(raw.clone())
    })
    .unwrap();
    assert_eq!(calls, vec![SOURCE]);
    assert_eq!(
        model.to_bytes().unwrap(),
        apply(&r, ID, &raw).unwrap().to_bytes().unwrap()
    );
    assert_eq!(
        model.source().dependencies,
        vec![
            Dependency {
                uri: "$source".into(),
                sha256: digest(&r)
            },
            Dependency {
                uri: SOURCE.into(),
                sha256: digest(&raw)
            }
        ]
    );
    let mut padded = r.clone();
    padded.push(b' ');
    let other = apply(&padded, ID, &raw).unwrap();
    assert_ne!(
        model.source().dependencies[0].sha256,
        other.source().dependencies[0].sha256
    );
    assert_eq!(
        model.source().dependencies[1],
        other.source().dependencies[1]
    );
}
#[test]
fn strict_seven_fields_duplicates_null_unknown_types_and_wraps_reject_before_resolution() {
    let good = recipe();
    for key in [
        "format",
        "version",
        "source",
        "material",
        "linear_filter",
        "wrap_s",
        "wrap_t",
    ] {
        let mut v = good.clone();
        v.as_object_mut().unwrap().remove(key);
        reject(&serde_json::to_vec(&v).unwrap(), ID);
        let mut v = good.clone();
        v[key] = Value::Null;
        reject(&serde_json::to_vec(&v).unwrap(), ID);
        let text = serde_json::to_string(&good).unwrap();
        reject(
            format!("{{\"{key}\":{},{}", good[key], &text[1..]).as_bytes(),
            ID,
        );
    }
    for (key, values) in [
        (
            "format",
            vec![json!("orr_static_material_sampler"), json!(1)],
        ),
        ("version", vec![json!(0), json!(2), json!(1.5), json!(true)]),
        (
            "material",
            vec![json!(-1), json!(4294967296u64), json!(0.5), json!("0")],
        ),
        (
            "linear_filter",
            vec![json!(0), json!(1), json!("false"), json!([])],
        ),
        (
            "wrap_s",
            vec![
                json!("Clamp"),
                json!("clamp_to_edge"),
                json!({"clamp":null}),
                json!("repeat "),
                json!(""),
                json!(0),
                json!([]),
            ],
        ),
        (
            "wrap_t",
            vec![
                json!("MIRROR"),
                json!(" mirrored"),
                json!("unknown"),
                json!(false),
            ],
        ),
    ] {
        for value in values {
            let mut v = good.clone();
            v[key] = value;
            reject(&serde_json::to_vec(&v).unwrap(), ID);
        }
    }
    let mut v = good.clone();
    v["extra"] = json!(true);
    reject(&serde_json::to_vec(&v).unwrap(), ID);
    for raw in [b"[]".as_slice(),b"null",b"true",b"{}",b"{",b"[] true",b"[\"orr_animated_material_sampler\",1,\"inputs/hero.animated.json\",0,false,\"clamp\",\"repeat\"]"] {reject(raw,ID);}
    reject(&vec![b' '; 16 * 1024 + 1], ID);
}
#[test]
fn portable_source_and_recipe_id_reject_traversal_aliases_before_resolution() {
    for path in [
        "",
        "/hero.animated.json",
        "../hero.animated.json",
        "a/../hero.animated.json",
        "a//hero.animated.json",
        "./hero.animated.json",
        "a\\hero.animated.json",
        "a b.animated.json",
        "a%20b.animated.json",
        "https://host/a.animated.json",
        "CON.animated.json",
        "aux.animated.json",
        "a./hero.animated.json",
        ".hidden/hero.animated.json",
        "hero.ANIMATED.JSON",
    ] {
        let mut r = recipe();
        r["source"] = json!(path);
        reject(&serde_json::to_vec(&r).unwrap(), ID);
    }
    for id in [
        "",
        "../blue.sampler.json",
        "blue.animated.json",
        "a//blue.sampler.json",
        "NUL.sampler.json",
        "blue.RECOLOR.JSON",
    ] {
        reject(&bytes(), id);
    }
    let mut r = recipe();
    r["source"] = json!(format!("{}.animated.json", "a".repeat(241)));
    reject(&serde_json::to_vec(&r).unwrap(), ID);
}
#[test]
fn full_animated_admission_requires_clips_and_a_used_selected_material() {
    let original = fixture();
    let raw = original.to_bytes().unwrap();
    for slot in [2, 3, u32::MAX] {
        let mut r = recipe();
        r["material"] = json!(slot);
        assert!(apply(&serde_json::to_vec(&r).unwrap(), ID, &raw).is_err());
    }
    let mut no_clips = original.source().clone();
    no_clips.clips.clear();
    let raw = AnimatedModel::new(no_clips).unwrap().to_bytes().unwrap();
    assert!(apply(&bytes(), ID, &raw).is_err());
    let valid: Value = serde_json::from_slice(&original.to_bytes().unwrap()).unwrap();
    for (pointer, value) in [
        ("/materials/0/base_color/3", json!(2)),
        ("/materials/1/image", json!(999)),
        ("/primitives/0/node", json!(999)),
        ("/primitives/0/skin", json!(999)),
        ("/primitives/0/indices/0", json!(999999)),
        ("/skins/0/joints/0", json!(999)),
        ("/clips/0/channels/0/node", json!(999)),
        ("/images/0/width", json!(0)),
        ("/dependencies/0/sha256", json!("bad")),
    ] {
        let mut bad = valid.clone();
        *bad.pointer_mut(pointer).unwrap() = value;
        assert!(
            apply(&bytes(), ID, &serde_json::to_vec(&bad).unwrap()).is_err(),
            "{pointer}"
        );
    }
    for raw in [b"null".to_vec(), vec![b' '; orr_model::MAX_FILE_BYTES + 1]] {
        assert!(apply(&bytes(), ID, &raw).is_err());
    }
}
#[test]
fn all_eighteen_sampler_combinations_and_identity_failures() {
    let original = fixture();
    let raw = original.to_bytes().unwrap();
    for filter in [false, true] {
        for (s, ws) in [
            ("clamp", Wrap::Clamp),
            ("repeat", Wrap::Repeat),
            ("mirror", Wrap::Mirror),
        ] {
            for (t, wt) in [
                ("clamp", Wrap::Clamp),
                ("repeat", Wrap::Repeat),
                ("mirror", Wrap::Mirror),
            ] {
                let mut r = recipe();
                r["linear_filter"] = json!(filter);
                r["wrap_s"] = json!(s);
                r["wrap_t"] = json!(t);
                let result = apply(&serde_json::to_vec(&r).unwrap(), ID, &raw).unwrap();
                let m = &result.source().materials[0];
                assert_eq!((m.linear_filter, m.wrap_s, m.wrap_t), (filter, ws, wt));
                assert_eq!(
                    m.base_color.map(f32::to_bits),
                    original.source().materials[0].base_color.map(f32::to_bits)
                );
                assert_eq!(m.image, original.source().materials[0].image);
            }
        }
    }
    assert!(
        animated_sampler::author_sampler(&bytes(), ID, |_| Err("resolver failed".into())).is_err()
    );
    let mut alias = original.source().clone();
    let prior = alias.asset_id.clone();
    alias.asset_id = ID.into();
    for p in &mut alias.primitives {
        p.id = p.id.replacen(&prior, ID, 1);
    }
    assert!(apply(
        &bytes(),
        ID,
        &AnimatedModel::new(alias).unwrap().to_bytes().unwrap()
    )
    .is_err());
}
#[test]
fn composing_recolor_retexture_then_sampler_preserves_images_colors_and_animation_without_history_io(
) {
    let original = fixture();
    let raw = original.to_bytes().unwrap();
    let color_recipe=serde_json::to_vec(&json!({"format":"orr_animated_material_color","version":1,"source":SOURCE,"material":0,"linear_rgb":[0.25,0.75,1]})).unwrap();
    let colored = orr_model::animated_recolor::recolor_model(
        &color_recipe,
        "variants/color.recolor.json",
        |p| {
            assert_eq!(p, SOURCE);
            Ok(raw.clone())
        },
    )
    .unwrap();
    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[9, 27, 81, 243])
            .unwrap();
    }
    let texture_recipe=serde_json::to_vec(&json!({"format":"orr_animated_material_texture","version":1,"source":SOURCE,"material":0,"texture":"textures/new.png"})).unwrap();
    let colored = orr_model::animated_retexture::retexture_model(
        &texture_recipe,
        "variants/texture.retexture.json",
        |p| match p {
            SOURCE => Ok(colored.to_bytes().unwrap()),
            "textures/new.png" => Ok(png_bytes.clone()),
            _ => panic!("historical dependency resolved"),
        },
    )
    .unwrap();
    let result = apply(&bytes(), ID, &colored.to_bytes().unwrap()).unwrap();
    assert_eq!(
        result.source().materials[0].base_color.map(f32::to_bits),
        colored.source().materials[0].base_color.map(f32::to_bits)
    );
    assert_eq!(result.source().images, colored.source().images);
    for clip in 0..colored.source().clips.len() {
        for t in [0.0, 0.375, 1.25, 3.0] {
            let a = colored.sample_clip(clip as u32, t).unwrap();
            let b = result.sample_clip(clip as u32, t).unwrap();
            assert_eq!(a.local(), b.local());
            assert_eq!(a.global(), b.global());
            assert_eq!(a.skin_matrices(), b.skin_matrices());
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod filesystem {
    use super::*;
    use std::{
        ffi::OsString,
        fs,
        path::{Path, PathBuf},
        process::Command,
        sync::atomic::{AtomicUsize, Ordering},
    };
    static SERIAL: AtomicUsize = AtomicUsize::new(0);
    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let root = std::env::temp_dir().canonicalize().unwrap();
            for ancestor in root.ancestors() {
                let m = fs::symlink_metadata(ancestor).unwrap();
                assert!(m.is_dir() && !m.file_type().is_symlink());
            }
            let p = root.join(format!(
                "orr-animated-sampler-test-{}-{}",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
        fn join(&self, p: &str) -> PathBuf {
            self.0.join(p)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    struct Fixture {
        temp: Scratch,
        recipe: PathBuf,
        source: Vec<u8>,
        raw: Vec<u8>,
        output: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let temp = Scratch::new();
            fs::create_dir(temp.join("inputs")).unwrap();
            let source = fixture().to_bytes().unwrap();
            let raw = bytes();
            let recipe = temp.join("blue.sampler.json");
            fs::write(&recipe, &raw).unwrap();
            fs::write(temp.join(SOURCE), &source).unwrap();
            let output = temp.join("blue.animated.json");
            Self {
                temp,
                recipe,
                source,
                raw,
                output,
            }
        }
        fn args(&self, output: &Path) -> Vec<OsString> {
            vec![
                "--recipe".into(),
                self.recipe.as_os_str().into(),
                "--asset-id".into(),
                ID.into(),
                "--output".into(),
                output.as_os_str().into(),
            ]
        }
        fn unchanged(&self) {
            assert_eq!(fs::read(&self.recipe).unwrap(), self.raw);
            assert_eq!(fs::read(self.temp.join(SOURCE)).unwrap(), self.source);
        }
    }
    fn tree(root: &Path) -> Vec<PathBuf> {
        fn visit(root: &Path, at: &Path, out: &mut Vec<PathBuf>) {
            for e in fs::read_dir(at).unwrap() {
                let p = e.unwrap().path();
                out.push(p.strip_prefix(root).unwrap().to_path_buf());
                let m = fs::symlink_metadata(&p).unwrap();
                if m.is_dir() && !m.file_type().is_symlink() {
                    visit(root, &p, out);
                }
            }
        }
        let mut out = Vec::new();
        visit(root, root, &mut out);
        out.sort();
        out
    }
    #[test]
    fn recipe_file_api_resolves_relative_direct_source_and_returns_self_contained_bytes() {
        let f = Fixture::new();
        let before = tree(&f.temp.0);
        let result = animated_sampler::sampler_recipe(&f.recipe, ID).unwrap();
        assert_eq!(
            result,
            apply(&f.raw, ID, &f.source).unwrap().to_bytes().unwrap()
        );
        f.unchanged();
        assert_eq!(tree(&f.temp.0), before);
        fs::rename(&f.recipe, f.temp.join("recipe.hidden")).unwrap();
        fs::rename(f.temp.join("inputs"), f.temp.join("inputs.hidden")).unwrap();
        assert_eq!(
            AnimatedModel::from_bytes(&result)
                .unwrap()
                .source()
                .asset_id,
            ID
        );
    }
    #[test]
    fn run_publishes_once_and_preserves_inputs_existing_files_and_hardlink_aliases() {
        let f = Fixture::new();
        animated_sampler::run(f.args(&f.output)).unwrap();
        let result = fs::read(&f.output).unwrap();
        assert_eq!(
            result,
            apply(&f.raw, ID, &f.source).unwrap().to_bytes().unwrap()
        );
        f.unchanged();
        let before = tree(&f.temp.0);
        assert!(animated_sampler::run(f.args(&f.output)).is_err());
        assert_eq!(fs::read(&f.output).unwrap(), result);
        assert_eq!(tree(&f.temp.0), before);
        for (name, target) in [
            ("source-alias.animated.json", f.temp.join(SOURCE)),
            ("recipe-alias.animated.json", f.recipe.clone()),
        ] {
            let alias = f.temp.join(name);
            fs::hard_link(target, &alias).unwrap();
            let bytes = fs::read(&alias).unwrap();
            let before = tree(&f.temp.0);
            assert!(animated_sampler::run(f.args(&alias)).is_err());
            assert_eq!(fs::read(alias).unwrap(), bytes);
            assert_eq!(tree(&f.temp.0), before);
        }
        f.unchanged();
    }
    #[test]
    fn strict_args_links_special_inputs_and_bounds_fail_without_publication_or_staging() {
        let f = Fixture::new();
        let original = tree(&f.temp.0);
        for args in [
            vec![],
            vec!["--recipe".into(), f.recipe.as_os_str().into()],
            vec![
                "--bogus".into(),
                "x".into(),
                "--asset-id".into(),
                ID.into(),
                "--output".into(),
                f.output.as_os_str().into(),
            ],
            vec![
                "--recipe".into(),
                f.recipe.as_os_str().into(),
                "--recipe".into(),
                f.recipe.as_os_str().into(),
                "--output".into(),
                f.output.as_os_str().into(),
            ],
        ] {
            assert!(animated_sampler::run(args).is_err());
            assert!(!f.output.exists());
            assert_eq!(tree(&f.temp.0), original);
        }
        let mut consumed = 0;
        assert!(animated_sampler::run(std::iter::from_fn(|| {
            consumed += 1;
            assert!(consumed <= 8);
            Some(OsString::from("--extra"))
        }))
        .is_err());
        assert!(consumed > 6);
        let alias = f.temp.join("alias.sampler.json");
        std::os::unix::fs::symlink(&f.recipe, &alias).unwrap();
        assert!(animated_sampler::sampler_recipe(&alias, ID).is_err());
        let input_alias = f.temp.join("inputs/alias.animated.json");
        std::os::unix::fs::symlink(f.temp.join(SOURCE), &input_alias).unwrap();
        let mut r = recipe();
        r["source"] = json!("inputs/alias.animated.json");
        fs::write(&f.recipe, serde_json::to_vec(&r).unwrap()).unwrap();
        assert!(animated_sampler::run(f.args(&f.output)).is_err());
        assert!(!f.output.exists());
        let fifo = f.temp.join("inputs/blocked.animated.json");
        assert!(Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success());
        r["source"] = json!("inputs/blocked.animated.json");
        fs::write(&f.recipe, serde_json::to_vec(&r).unwrap()).unwrap();
        assert!(animated_sampler::sampler_recipe(&f.recipe, ID).is_err());
        fs::write(&f.recipe, vec![b' '; 16 * 1024 + 1]).unwrap();
        assert!(animated_sampler::run(f.args(&f.output)).is_err());
        assert!(!f.output.exists());
        assert_eq!(fs::read(f.temp.join(SOURCE)).unwrap(), f.source);
        assert!(!fs::read_dir(&f.temp.0).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".orr-")));
    }
    #[test]
    #[ignore = "requires explicit freshly built ORR_ANIMATED_SAMPLER_TOOL"]
    fn actual_example_cli_publishes_source_hidden_reusable_animation_and_refuses_collisions() {
        let tool = PathBuf::from(
            std::env::var_os("ORR_ANIMATED_SAMPLER_TOOL")
                .expect("build the real example and pass its absolute path"),
        );
        assert!(tool.is_absolute());
        let f = Fixture::new();
        let run = |output: &Path| Command::new(&tool).args(f.args(output)).output().unwrap();
        let output = run(&f.output);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result = fs::read(&f.output).unwrap();
        assert_eq!(
            result,
            apply(&f.raw, ID, &f.source).unwrap().to_bytes().unwrap()
        );
        f.unchanged();
        assert!(!run(&f.output).status.success());
        assert_eq!(fs::read(&f.output).unwrap(), result);
        fs::rename(&f.recipe, f.temp.join("recipe.hidden")).unwrap();
        fs::rename(f.temp.join("inputs"), f.temp.join("inputs.hidden")).unwrap();
        let model = AnimatedModel::from_bytes(&fs::read(&f.output).unwrap()).unwrap();
        assert_eq!(model.source().asset_id, ID);
        assert!(!model.source().materials[0].linear_filter);
        assert_eq!(model.source().materials[0].wrap_s, Wrap::Clamp);
        assert_eq!(model.source().materials[0].wrap_t, Wrap::Mirror);
        for clip in 0..model.source().clips.len() {
            let pose = model.sample_clip(clip as u32, 0.75).unwrap();
            assert_eq!(model.deform(&pose).unwrap().len(), 2);
        }
    }
}
