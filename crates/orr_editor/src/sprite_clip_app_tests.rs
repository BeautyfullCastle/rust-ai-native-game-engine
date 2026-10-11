//! Real production App composition: controls only, not an atlas preview.
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]
use super::App;
use egui::accesskit::Role;
use egui_kittest::{Harness, kittest::Queryable};
use orr_editor::sprite_clip_panel::{ClipPanel, SaveOutcome};
use orr_package::{Project, Runtime};
use std::path::{Path, PathBuf};
fn settle(h: &mut Harness<'_, App>) { h.try_run().expect("bounded production App layout"); }
fn click(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_label(label).scroll_to_me(); settle(h);
    h.get_by_label(label).click(); settle(h);
}
fn text(h: &mut Harness<'_, App>, label: &str, value: &str) {
    h.get_by_role_and_label(Role::TextInput, label).scroll_to_me(); settle(h);
    h.get_by_role_and_label(Role::TextInput, label).focus(); settle(h);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A); settle(h);
    h.get_by_role_and_label(Role::TextInput, label).type_text(value); settle(h);
}
fn capture(h: &mut Harness<'_, App>, name: &str) -> Vec<u8> {
    let image = h.render().expect("actual standalone App WGPU composition");
    assert_eq!((image.width(), image.height()), (1000, 900));
    assert!(image.as_raw().as_chunks::<4>().0.windows(2).any(|pixels| pixels[0] != pixels[1]), "nonuniform controls framebuffer");
    let directory = PathBuf::from(std::env::var_os("ORR_SPRITE_CLIP_CAPTURE_DIR").expect("mandatory capture directory"));
    assert!(directory.is_dir());
    let file = std::fs::OpenOptions::new().create_new(true).write(true).open(directory.join(format!("{name}.png"))).unwrap();
    let mut encoder = png::Encoder::new(file, image.width(), image.height());
    encoder.set_color(png::ColorType::Rgba); encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(image.as_raw()).unwrap();
    image.into_raw()
}
#[test]
#[ignore = "mandatory explicit production App GPU control acceptance"]
fn production_app_controls_render_edit_and_save_admitted_package() {
    assert_eq!(std::env::var("ORR_REQUIRE_GPU").as_deref(), Ok("1"));
    let project_root = tempfile::tempdir().unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/sprite_demo").canonicalize().expect("trusted repository sprite fixture");
    let project = Project::open_for_install(project_root.path(), Runtime::content_only().engine_version).unwrap();
    project.install(&[source]).unwrap();
    let original_lock = std::fs::read(project_root.path().join("orr.packages.lock.json")).unwrap();
    let panel = ClipPanel::open_installed(project_root.path(), "sample-sprites", "sprites.json", "walk", "1.0.1").unwrap();
    let mut h = Harness::builder().with_size([1000.0, 900.0]).with_max_steps(120).wgpu()
        .build_eframe(move |cc| {
            eprintln!("actual standalone App adapter: {:?}", cc.wgpu_render_state.as_ref().expect("WGPU renderer").adapter.get_info());
            App { panel }
        });
    settle(&mut h);
    let initial = capture(&mut h, "standalone-01-controls");
    text(&mut h, "Duration ms", "0"); click(&mut h, "Apply frame draft");
    assert!(h.state().panel.message().contains("positive"));
    assert_eq!(h.state().panel.source().clips.iter().find(|c| c.id == "walk").unwrap().frames[0].duration_ms, 140);
    let rejected = capture(&mut h, "standalone-02-rejected-duration");
    assert_ne!(initial, rejected);
    text(&mut h, "Duration ms", "125"); click(&mut h, "Apply frame draft");
    assert_eq!(h.state().panel.source().clips.iter().find(|c| c.id == "walk").unwrap().frames[0].duration_ms, 125);
    let destination_root = tempfile::tempdir().unwrap();
    let destination = destination_root.path().canonicalize().unwrap().join("new-package");
    text(&mut h, "New package directory", destination.to_str().unwrap());
    click(&mut h, "Save As new package");
    assert!(matches!(h.state().panel.save_outcome(), SaveOutcome::Published { destination: path } if path == &destination));
    capture(&mut h, "standalone-03-published");
    assert_eq!(std::fs::read(project_root.path().join("orr.packages.lock.json")).unwrap(), original_lock);
    let saved: orr_sprite::SpriteSource = serde_json::from_slice(&std::fs::read(destination.join("sprites.json")).unwrap()).unwrap();
    assert_eq!(&saved, h.state().panel.source());
    click(&mut h, "Save As new package");
    assert!(matches!(h.state().panel.save_outcome(), SaveOutcome::Failed { published: false, .. }));
    capture(&mut h, "standalone-04-existing-destination-error");
}
