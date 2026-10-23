//! Actual offline CLI -> declared package -> hidden inputs -> cooked GPU consumer.
//! Dedicated, bounded acceptance; never silently skips a missing software adapter.
#![cfg(all(feature = "animation", feature = "imported-scene"))]
#![allow(clippy::float_arithmetic)]
#[allow(dead_code)]
#[path = "../../orr_model/tests/common/animation_clip_sequence_oracle.rs"]
mod oracle;
use orr_model::animation::AnimatedModel;
use orr_package::{Project, Runtime};
use orr_render::orr_rhi::{Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions};
use orr_render::{Camera3D, Lighting, SkinnedInstance, SkinnedModelRenderer};
use serde_json::json;
use std::{fs, path::PathBuf, process::Command};
const SIZE: u32 = 192;
const PACKAGE: &str = "sequence-acceptance";
const ASSET: &str = "action.animated.json";
fn write(root: &std::path::Path, name: &str, bytes: &[u8]) {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(name))
        .unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
}
fn capture(root: &std::path::Path, name: &str, pixels: &[u8]) {
    write(root, &format!("{name}.rgba"), pixels);
    let mut ppm = format!("P6\n{SIZE} {SIZE}\n255\n").into_bytes();
    for p in pixels.chunks_exact(4) {
        ppm.extend_from_slice(&p[..3]);
    }
    write(root, &format!("{name}.ppm"), &ppm);
}
fn bary(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> [f64; 3] {
    let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    let u = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / d;
    let v = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / d;
    [u, v, 1.0 - u - v]
}
// Uses literal original fixture geometry, f64 independent deformation, and a
// closed-form camera. No renderer bounds, production projection or pose helper.
fn expected(positions: &[[f64; 3]], x: usize, y: usize) -> Option<[u8; 4]> {
    let screen: Vec<_> = positions
        .iter()
        .map(|p| [96.0 + 32.0 * p[0], 96.0 - 32.0 * (p[1] - 1.5)])
        .collect();
    let mut near = false;
    let mut color = None;
    for ids in oracle::TRIANGLES {
        let b = bary(
            [x as f64 + 0.5, y as f64 + 0.5],
            screen[ids[0]],
            screen[ids[1]],
            screen[ids[2]],
        );
        near |= b.iter().all(|&v| v >= -0.08);
        if b.iter().all(|&v| v > 0.06) {
            let u: f64 = (0..3).map(|i| b[i] * (ids[i] % 2) as f64).sum();
            let v: f64 = (0..3)
                .map(|i| b[i] * (1.0 - (ids[i] / 2) as f64 / 3.0))
                .sum();
            let t = [u.clamp(0.0, 1.0) * 4.0, v.clamp(0.0, 1.0) * 4.0];
            if t.iter().any(|v| v.fract() < 0.025 || v.fract() > 0.975) {
                continue;
            }
            color = Some(
                oracle::COLORS[oracle::CHECKER[(t[1].floor() as usize).min(3)]
                    [(t[0].floor() as usize).min(3)]],
            );
        }
    }
    color.or_else(|| (!near).then_some([0, 0, 0, 255]))
}
fn check(pixels: &[u8], positions: &[[f64; 3]]) -> (usize, usize) {
    let (mut interior, mut clear) = (0, 0);
    for y in 0..SIZE as usize {
        for x in 0..SIZE as usize {
            if let Some(color) = expected(positions, x, y) {
                let actual = &pixels[(y * SIZE as usize + x) * 4..][..4];
                assert!(
                    actual.iter().zip(color).all(|(a, b)| a.abs_diff(b) <= 3),
                    "pixel {x},{y}: {actual:?} != {color:?}"
                );
                if color == [0, 0, 0, 255] {
                    clear += 1;
                } else {
                    interior += 1;
                }
            }
        }
    }
    assert!(
        interior > 100 && clear > 20000,
        "nonvacuous geometry coverage {interior}/{clear}"
    );
    (interior, clear)
}
#[test]
#[ignore = "mandatory explicit CLI, evidence directory and software Vulkan acceptance"]
fn actual_cli_installed_sequence_has_independent_visible_transition() {
    assert_eq!(
        std::env::var("ORR_SEQUENCE_REQUIRE_SOFTWARE").as_deref(),
        Ok("1")
    );
    let cli = PathBuf::from(
        std::env::var_os("ORR_ANIMATION_SEQUENCE_CLI").expect("absolute real CLI required"),
    );
    assert!(cli.is_absolute() && cli.is_file());
    let root = PathBuf::from(
        std::env::var_os("ORR_SEQUENCE_RENDER_EVIDENCE")
            .expect("fresh absolute evidence directory required"),
    );
    assert!(root.is_absolute());
    fs::create_dir(&root).unwrap();
    let source = root.join("inputs");
    fs::create_dir(&source).unwrap();
    write(
        &source,
        "source.animated.json",
        &oracle::fixture().to_bytes().unwrap(),
    );
    write(&source,"recipe.sequence.json",&serde_json::to_vec(&json!({"format":"orr_animation_clip_sequence","version":1,"source":"source.animated.json","first_clip":0,"second_clip":1,"transition_seconds":1,"name":"A then B"})).unwrap());
    let output = Command::new(&cli)
        .args(["--recipe"])
        .arg(source.join("recipe.sequence.json"))
        .args(["--asset-id", "sequences/action.sequence.json", "--output"])
        .arg(source.join(ASSET))
        .output()
        .unwrap();
    write(&root, "cli.stdout", &output.stdout);
    write(&root, "cli.stderr", &output.stderr);
    write(&root,"started.json",&serde_json::to_vec_pretty(&json!({"status":"partial","cli":cli,"exit":output.status.code(),"samples":[1.0,2.25,2.5,2.75,4.0],"oracle":"independent literal fixture f64 deformation","color_tolerance":3})).unwrap());
    assert!(output.status.success());
    for (label, first, second, transition) in [
        ("wrong-order", 1, 0, 1.0),
        ("wrong-transition-duration", 0, 1, 0.5),
    ] {
        let recipe = format!("{label}.sequence.json");
        write(&source,&recipe,&serde_json::to_vec(&json!({"format":"orr_animation_clip_sequence","version":1,"source":"source.animated.json","first_clip":first,"second_clip":second,"transition_seconds":transition,"name":label})).unwrap());
        let result = Command::new(&cli)
            .arg("--recipe")
            .arg(source.join(&recipe))
            .args(["--asset-id", "sequences/control.sequence.json", "--output"])
            .arg(source.join(format!("{label}.animated.json")))
            .output()
            .unwrap();
        write(&root, &format!("{label}-cli.stdout"), &result.stdout);
        write(&root, &format!("{label}-cli.stderr"), &result.stderr);
        assert!(result.status.success());
    }
    write(&source,"orr.package.json",&serde_json::to_vec(&json!({"schema":1,"name":PACKAGE,"version":"1.0.0","engine":"*","capabilities":["animation"],"dependencies":{},"files":[ASSET,"wrong-order.animated.json","wrong-transition-duration.animated.json"]})).unwrap());
    let project_root = root.join("project");
    fs::create_dir(&project_root).unwrap();
    Project::open_for_install(&project_root, Runtime::content_only().engine_version)
        .unwrap()
        .install(std::slice::from_ref(&source))
        .unwrap();
    fs::rename(&source, root.join("inputs-hidden")).unwrap();
    let mut runtime = Runtime::content_only();
    runtime.capabilities.insert("animation".into());
    let project = Project::open(&project_root, runtime).unwrap();
    project.verify().unwrap();
    let bytes = project.read_asset(PACKAGE, ASSET).unwrap();
    write(&root, ASSET, &bytes);
    let model = AnimatedModel::from_bytes(&bytes).unwrap();
    assert_eq!(model.source().dependencies.len(), 2);
    let gpu = Wgpu::headless(WgpuOptions {
        force_software: true,
        ..Default::default()
    })
    .expect("software Vulkan required");
    let info = gpu.adapter().get_info();
    let adapter = json!({"name":info.name,"backend":format!("{:?}",info.backend),"device_type":format!("{:?}",info.device_type)});
    write(
        &root,
        "adapter.json",
        &serde_json::to_vec_pretty(&adapter).unwrap(),
    );
    assert_eq!(adapter["backend"], "Vulkan");
    assert_eq!(adapter["device_type"], "Cpu");
    assert!(gpu.is_software());
    let format = TextureFormat::Rgba8UnormSrgb;
    let target = gpu.create_texture(&TextureDesc {
        label: "sequence acceptance",
        width: SIZE,
        height: SIZE,
        format,
        usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
        sample_count: 1,
        view_formats: &[],
    });
    let view = gpu.create_texture_view(&target, None);
    let camera = Camera3D::orthographic([0.0, 1.5, 5.0], [0.0, 1.5, 0.0], 3.0);
    let light = Lighting {
        sky: [1.0; 3],
        ground: [1.0; 3],
        ambient: 1.0,
        intensity: 0.0,
        shadows: false,
        tonemap: false,
        exposure: 1.0,
        ..Default::default()
    };
    let mut renderer = SkinnedModelRenderer::new(gpu.clone(), format, model.clone()).unwrap();
    renderer.clear = [0.0, 0.0, 0.0, 1.0];
    let clip = (model.source().clips.len() - 1) as u32;
    // Check discriminator texels BEFORE any draws. There are no procedural
    // geometry markers in this direct renderer; all six fixture triangles are
    // considered above, so background controls cannot hide behind another mesh.
    let correct = oracle::sequence_positions(2.25);
    let wrong_order = oracle::ordered_positions(1, 0, 2.25, 1.0);
    let wrong_duration = oracle::ordered_positions(0, 1, 2.25, 0.5);
    let discriminators = |other: &[[f64; 3]]| {
        (0..SIZE as usize).flat_map(|y|(0..SIZE as usize).map(move|x|(x,y))).filter(|&(x,y)|matches!((expected(&correct,x,y),expected(other,x,y)),(Some(a),Some(b)) if a!=b)).count()
    };
    let controls = json!({"wrong_order_confident_texels":discriminators(&wrong_order),"wrong_transition_confident_texels":discriminators(&wrong_duration),"occlusion":"all fixture triangles included; no other geometry submitted"});
    write(
        &root,
        "control-plan.json",
        &serde_json::to_vec_pretty(&controls).unwrap(),
    );
    assert!(controls["wrong_order_confident_texels"].as_u64().unwrap() > 50);
    assert!(
        controls["wrong_transition_confident_texels"]
            .as_u64()
            .unwrap()
            > 50
    );
    let mut frames = Vec::new();
    let mut correct_pixels = Vec::new();
    for (index, time) in [1.0, 2.25, 2.5, 2.75, 4.0].into_iter().enumerate() {
        let pose = model.sample_clip(clip, time).unwrap();
        renderer
            .draw(
                &view,
                (SIZE, SIZE),
                &camera,
                &light,
                &[SkinnedInstance::new(&pose)],
            )
            .unwrap();
        let pixels = gpu.read_texture(&target);
        capture(&root, &format!("sample-{index}"), &pixels);
        let positions = oracle::sequence_positions(f64::from(time));
        let actual = model.deform(&pose).unwrap();
        for (v, p) in actual[0].vertices.iter().zip(&positions) {
            for (actual, expected) in v.position.iter().zip(p) {
                assert!((f64::from(*actual) - expected).abs() < 4e-5);
            }
        }
        if index == 1 {
            correct_pixels = pixels.clone();
        }
        let coverage = check(&pixels, &positions);
        frames.push(json!({"time":time,"coverage":coverage}));
    }
    for (label, positions) in [
        ("wrong-order", wrong_order),
        ("wrong-transition-duration", wrong_duration),
    ] {
        let control = AnimatedModel::from_bytes(
            &project
                .read_asset(PACKAGE, &format!("{label}.animated.json"))
                .unwrap(),
        )
        .unwrap();
        let mut control_renderer =
            SkinnedModelRenderer::new(gpu.clone(), format, control.clone()).unwrap();
        control_renderer.clear = renderer.clear;
        let pose = control.sample_clip(clip, 2.25).unwrap();
        control_renderer
            .draw(
                &view,
                (SIZE, SIZE),
                &camera,
                &light,
                &[SkinnedInstance::new(&pose)],
            )
            .unwrap();
        let pixels = gpu.read_texture(&target);
        capture(&root, label, &pixels);
        check(&pixels, &positions);
        let changed = correct_pixels
            .chunks_exact(4)
            .zip(pixels.chunks_exact(4))
            .filter(|(a, b)| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > 3))
            .count();
        assert!(
            changed > 50,
            "actual independently authored {label} must visibly differ at identical t=2.25: {changed}"
        );
    }
    renderer
        .draw(&view, (SIZE, SIZE), &camera, &light, &[])
        .unwrap();
    let empty = gpu.read_texture(&target);
    capture(&root, "empty", &empty);
    assert!(empty.chunks_exact(4).all(|p| p == [0, 0, 0, 255]));
    write(&root,"acceptance.json",&serde_json::to_vec_pretty(&json!({"status":"passed","actual_cli":true,"inputs_hidden":!source.exists(),"adapter":adapter,"samples":frames,"controls":controls})).unwrap());
}
