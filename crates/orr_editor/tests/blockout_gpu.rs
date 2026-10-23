//! Explicit bounded-box acceptance through real editor widgets and the production
//! viewport, exporter and runtime. Run this ignored test deliberately with the
//! exact production tools; an ignored inventory entry is never GPU evidence.
#![cfg(all(feature = "room-project", target_os = "linux", target_arch = "x86_64"))]
#![allow(clippy::disallowed_types, clippy::float_arithmetic)]

use egui::{accesskit::Role, Key, Modifiers};
use egui_kittest::{kittest::{NodeT, Queryable}, Harness};
use orr_bridge::PlayerSlot;
use orr_editor::{blockout::{Face, Recipe}, editor::input::Phase, Editor, EditorApp, HostSpec, Mode, Target};
use orr_fp::{FPVec3, FP};
use orr_model::{Dependency, Image, Material, ModelSource, Primitive, StaticModel, Vertex, Wrap, IDENTITY};
use orr_model_bindings::model_bindings::{self, Binding, Document, LocalTransform};
use orr_package::{Project, Runtime};
use orr_reflect::{Guid, Scene, Value};
use orr_sample::{room_game::{RoomActor, RoomConfig, RoomEscapeV1, RoomInput, RoomRun, EXIT, INTERACT, KEY, PLAYER}, room_project::{self, PreparedProject, PreparedScene, SEED}};
use orr_sim::{Simulation, TickInputs};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, io::{Read, Write}, path::{Path, PathBuf}, process::{Command, Output}, time::{Duration, Instant}};

const TEST: &str = "authored_box_widgets_six_face_gpu_and_source_hidden_room_export";
const RECIPE: &str = "authored.blockout.json";
const ASSET: &str = "box.model.json";
const LOCAL_OFFSET: [f32; 3] = [0.0, 5.0, 0.0];
const CAPTURE_SIZE: (u32,u32) = (892,962);
// The integrated editor adds a visible Frame selected collider row before the
// viewport. Reserve its expected 21 points without changing the pixel oracle.
// This source-derived allowance must be confirmed by the retained layout probe.
const FRAME_SELECTION_ROW_ALLOWANCE: f32 = 21.0;
const WINDOW_SIZE: [f32; 2] = [1600.0, 1100.0 + FRAME_SELECTION_ROW_ALLOWANCE];
const VIEWS: [(&str, f32, f32); 6] = [
    ("positive-x", std::f32::consts::FRAC_PI_2, 0.0),
    ("negative-x", -std::f32::consts::FRAC_PI_2, 0.0),
    ("positive-y", 0.0, 1.5),
    ("negative-y", 0.0, -1.5),
    ("positive-z", 0.0, 0.0),
    ("negative-z", std::f32::consts::PI, 0.0),
];

fn digest(bytes: &[u8]) -> String { Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect() }
fn file_hash(path: &Path) -> String {
    let mut reader = fs::File::open(path).unwrap();
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop { let n = reader.read(&mut buffer).unwrap(); if n == 0 { break; } hash.update(&buffer[..n]); }
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}
fn tree(root: &Path) -> BTreeMap<String, (u64, String)> {
    fn visit(root: &Path, path: &Path, out: &mut BTreeMap<String, (u64, String)>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap(); let kind = entry.file_type().unwrap();
            assert!(!kind.is_symlink(), "unexpected symlink {}", entry.path().display());
            if kind.is_dir() { visit(root, &entry.path(), out); }
            else { assert!(kind.is_file()); out.insert(entry.path().strip_prefix(root).unwrap().to_str().unwrap().into(), (entry.metadata().unwrap().len(), file_hash(&entry.path()))); }
        }
    }
    let mut out = BTreeMap::new(); visit(root, root, &mut out); out
}
fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap(); let kind = entry.file_type().unwrap(); assert!(!kind.is_symlink());
        if kind.is_dir() { copy_tree(&entry.path(), &to.join(entry.file_name())); }
        else { assert!(kind.is_file()); fs::copy(entry.path(), to.join(entry.file_name())).unwrap(); }
    }
}
fn good(output: Output) -> String {
    assert!(output.status.success(), "{}\n{}\n{}", output.status, String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

struct Fixture { temp: tempfile::TempDir, root: PathBuf, source: PathBuf, guids: BTreeMap<u32, Guid> }
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap(); let root = temp.path().join("authored room"); fs::create_dir(&root).unwrap();
        let source = temp.path().join("original source package");
        copy_tree(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/imported_scene_demo"), &source);
        let sim = Simulation::<RoomEscapeV1>::new(RoomConfig, 60, SEED);
        let mut scene = Scene::unbake(&room_project::types(), sim.frame(), None).unwrap();
        for entity in scene.entities.values_mut() {
            let kind = entity.components.iter().find(|(name,_)| name == room_project::ACTOR).unwrap().1.field("kind").unwrap();
            let x = match kind { Value::Int(n) if *n == i128::from(PLAYER) => FP::ZERO, Value::Int(n) if *n == i128::from(KEY) => FP::HALF, Value::Int(n) if *n == i128::from(EXIT) => FP::from_int(2), _ => continue };
            let body = &mut entity.components.iter_mut().find(|(name,_)| name == "orr_physics3d::Body").unwrap().1;
            let Value::Struct(fields) = body else { panic!("reflected body") };
            fields.iter_mut().find(|(name,_)| name == "pos").unwrap().1 = Value::Vec3(FPVec3::new(x, FP::HALF, FP::ZERO));
        }
        let text = scene.to_yaml(); fs::write(root.join("room.scene.yaml"), &text).unwrap();
        fs::write(root.join("orr.project.json"), serde_json::to_vec_pretty(&serde_json::json!({"schema":2,"engine":"*","entry":{"game":"room-escape-v1","scene":"room.scene.yaml","models":"room.models.json"}})).unwrap()).unwrap();
        Project::open_for_install(&root, Runtime::content_only().engine_version).unwrap().install(std::slice::from_ref(&source)).unwrap();
        let prepared = PreparedScene::parse(&text).unwrap();
        let guids = prepared.frame().entities().filter_map(|entity| { let kind = prepared.frame().get::<RoomActor>(entity).unwrap().kind; [PLAYER, KEY, EXIT].contains(&kind).then(|| (kind, prepared.index().guid(entity).unwrap().clone())) }).collect::<BTreeMap<_,_>>();
        let fixture = Self { temp, root, source, guids };
        fixture.bind("sample-imported-scene", "foreground.glb"); fixture
    }
    fn bind(&self, package: &str, asset: &str) {
        let loaded = model_bindings::load_asset(&self.root, package, asset).unwrap();
        let binding = Binding::from_asset(package.into(), asset.into(), &loaded, LocalTransform { translation: LOCAL_OFFSET, ..LocalTransform::default() }).unwrap();
        let doc = Document { version:2, scene:"room.scene.yaml".into(), project:".".into(), bindings: [(self.guids[&PLAYER].to_string(), binding)].into() };
        fs::write(self.root.join("room.models.json"), serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
    }
    fn app(&self) -> Harness<'static, EditorApp> {
        let prepared = PreparedProject::open(&self.root).unwrap(); let (_,path,scene,models) = prepared.into_parts();
        let mut editor = Editor::start(&HostSpec::PreparedRoom { scene:path, text:scene.text().into(), listen:None, debug_hooks:false }).unwrap(); editor.sync();
        editor.select(Some(Target::Guid(self.guids[&PLAYER].clone())));
        Harness::builder().with_size(WINDOW_SIZE).with_step_dt(1.0/60.0).wgpu().build_eframe(move |cc| { let mut app = EditorApp::new(editor,cc.wgpu_render_state.clone()); app.models.install_room(models).unwrap(); app })
    }
    fn install_oracle(&self, model: &StaticModel) {
        let source = self.temp.path().join("independent model oracle"); fs::create_dir_all(&source).unwrap();
        fs::write(source.join(ASSET),model.to_bytes().unwrap()).unwrap();
        fs::write(source.join("orr.package.json"),serde_json::to_vec_pretty(&serde_json::json!({"schema":1,"name":"independent-box-oracle","version":"1.0.0","engine":"*","capabilities":["models"],"dependencies":{},"files":[ASSET]})).unwrap()).unwrap();
        Project::open_for_install(&self.root,Runtime::content_only().engine_version).unwrap().install(&[source]).unwrap(); self.bind("independent-box-oracle",ASSET);
    }
}
fn settle(h: &mut Harness<'_,EditorApp>) { for _ in 0..3 { h.state_mut().editor.sync(); h.run_steps(2); } assert!(h.state().editor.down().is_none()); }
fn wait(h: &mut Harness<'_,EditorApp>, reason: &str, ready: impl Fn(&EditorApp)->bool) {
    let deadline = Instant::now()+Duration::from_secs(8);
    loop { settle(h); if ready(h.state()) { return; } assert!(Instant::now()<deadline,"{reason}: {:?}",h.state().editor.log()); std::thread::sleep(Duration::from_millis(5)); }
}
fn click(h: &mut Harness<'_,EditorApp>, label: &str) { h.get_by_label(label).scroll_to_me(); h.run_steps(3); h.get_by_label(label).click(); settle(h); }
fn type_into(h: &mut Harness<'_,EditorApp>,label:&str,text:&str) {
    h.get_by_role_and_label(Role::TextInput,label).scroll_to_me(); h.run_steps(3); h.get_by_role_and_label(Role::TextInput,label).focus(); h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND,Key::A); h.run_steps(2); h.get_by_role_and_label(Role::TextInput,label).type_text(text); h.run_steps(2); h.key_press(Key::Enter); settle(h);
}
fn key(h:&mut Harness<'_,EditorApp>, key:Key, pressed:bool) { h.event(egui::Event::Key { key,physical_key:Some(key),pressed,repeat:false,modifiers:Modifiers::NONE }); }
fn recipe(h:&Harness<'_,EditorApp>)->Recipe { h.state().models.blockout.document.as_ref().unwrap().recipe().clone() }
fn frame_bytes(editor:&mut Editor)->Vec<u8> {
    editor.sync(); let snapshot=editor.snapshot().unwrap(); let tick=snapshot.predicted().tick(); let checksum=snapshot.predicted().checksum();
    let client=editor.agent_client("box-acceptance-frame-probe").unwrap(); client.local_frames.clear(); client.call("watch.subscribe",serde_json::json!({"topics":["frames"],"source":"view","max_fps":60})).unwrap();
    let deadline=Instant::now()+Duration::from_secs(3);
    loop { client.poll().unwrap(); while let Some(frame)=client.local_frames.pop_front() { if frame.frame.tick()==tick && frame.frame.checksum()==checksum {return frame.frame.to_bytes();} } assert!(Instant::now()<deadline,"matching host Frame bytes missing"); std::thread::sleep(Duration::from_millis(1)); }
}
fn save_png(path:&Path,size:(u32,u32),pixels:&[u8]) { let out=fs::File::create(path).unwrap(); let mut encoder=png::Encoder::new(out,size.0,size.1); encoder.set_color(png::ColorType::Rgba); encoder.set_depth(png::BitDepth::Eight); let mut writer=encoder.write_header().unwrap(); writer.write_image_data(pixels).unwrap(); writer.finish().unwrap(); }
fn capture(h:&mut Harness<'_,EditorApp>,path:&Path)->Vec<u8> {
    settle(h); let pixels=h.state().viewport3d_gpu().expect("production main viewport GPU").gpu().read_rgba8(); let size=h.state().ui.viewport_px;
    assert_eq!(size,CAPTURE_SIZE,"capture geometry must remain fixed across camera changes");
    assert_eq!(pixels.len(),(size.0*size.1*4) as usize); assert!(pixels.as_chunks::<4>().0.iter().filter(|p| *p!=&pixels[..4]).count()>100,"blank viewport"); save_png(path,size,&pixels); pixels
}
// Inspector content legitimately changes its resizable width. Compare the
// same real editor layout, never resized/cropped readbacks: close both authoring
// sections through their actual widgets, and restore their prior state afterward.
fn capture_layout(h:&mut Harness<'_,EditorApp>, evidence:&Path)->[bool;2] {
    let open=[h.query_by_label("New box recipe").is_some(),h.query_by_label("Load model asset").is_some()];
    for (label,was_open) in ["Box blockout","Static model binding"].into_iter().zip(open) {if was_open {click(h,label);}}
    h.run_steps(20);settle(h);
    assert!(h.query_by_label("New box recipe").is_none()&&h.query_by_label("Load model asset").is_none());
    // Collapsing preserves the panels' previous resized widths. Seed the same
    // public egui remembered geometry on both apps before comparing readbacks.
    for (name,width) in [("inspector",478.0),("hierarchy",230.0)] {
        let id=egui::Id::new(name);
        let mut state=egui::containers::panel::PanelState::load(&h.ctx,id).expect("rendered panel state");
        if name=="inspector" {state.outer_rect.min.x=state.outer_rect.max.x-width;}
        else {state.outer_rect.max.x=state.outer_rect.min.x+width;}
        h.ctx.data_mut(|data|data.insert_persisted(id,state));
    }
    h.run_steps(20);settle(h);
    // Retain actual geometry before the fixed-size assertions, even on failure.
    let rect = |r: egui::Rect| [r.min.x, r.min.y, r.max.x, r.max.y];
    let toolbar = h.get_by_label("Frame selected collider").rect();
    let panels = ["top", "status", "bottom", "inspector", "hierarchy"].map(|name| {
        (name, egui::containers::panel::PanelState::load(&h.ctx,egui::Id::new(name)).map(|state| rect(state.outer_rect)))
    });
    let diagnostic = serde_json::json!({
        "outer_rect": rect(h.ctx.viewport_rect()), "pixels_per_point": h.ctx.pixels_per_point(),
        "toolbar_rect": rect(toolbar), "panels": panels,
        "viewport_rect": h.state().ui.viewport_rect.map(rect), "viewport_px": h.state().ui.viewport_px,
        "required_viewport_px": CAPTURE_SIZE, "source_hypothesis_toolbar_allowance": FRAME_SELECTION_ROW_ALLOWANCE,
    });
    let mut log = fs::OpenOptions::new().create(true).append(true).open(evidence.join("capture-layout.jsonl")).unwrap();
    writeln!(log,"{diagnostic}").unwrap();
    assert_eq!(h.ctx.viewport_rect().size(),egui::vec2(WINDOW_SIZE[0],WINDOW_SIZE[1]));
    for (name,width) in [("inspector",478.0),("hierarchy",230.0)] {
        assert_eq!(egui::containers::panel::PanelState::load(&h.ctx,egui::Id::new(name)).unwrap().outer_rect.width(),width,"canonical {name} width must remain fixed");
    }
    assert_eq!(h.state().ui.viewport_px,CAPTURE_SIZE,"identical collapsed inspector/window geometry is a required capture precondition");
    open
}
fn restore_capture_layout(h:&mut Harness<'_,EditorApp>,open:[bool;2]) {
    for (label,was_open) in ["Box blockout","Static model binding"].into_iter().zip(open) {if was_open {click(h,label);}}
    h.run_steps(20);settle(h);
}
fn six_views(h:&mut Harness<'_,EditorApp>,directory:&Path,prefix:&str)->Vec<Vec<u8>> {
    fs::create_dir_all(directory).unwrap();
    let open=capture_layout(h,directory);
    let pixels=VIEWS.into_iter().map(|(name,yaw,pitch)| { h.state_mut().editor.camera3d=orr_render::OrbitCamera::new([0.125,5.875,-0.1875],yaw,pitch,6.0); capture(h,&directory.join(format!("{prefix}-{name}.png"))) }).collect();
    restore_capture_layout(h,open);pixels
}
fn composed_positive_x(h:&mut Harness<'_,EditorApp>,path:&Path)->(Vec<u8>,(u32,u32),egui::Rect) {
    let open=capture_layout(h,path.parent().expect("capture directory"));
    h.state_mut().editor.camera3d=orr_render::OrbitCamera::new([0.125,5.875,-0.1875],VIEWS[0].1,VIEWS[0].2,6.0); settle(h);
    let image=h.render().expect("native viewport texture must compose into real EditorApp output"); let size=(image.width(),image.height()); let rgba=image.into_raw(); save_png(path,size,&rgba); let rect=h.state().ui.viewport_rect.unwrap();
    restore_capture_layout(h,open);(rgba,size,rect)
}
fn composed_change(a:&(Vec<u8>,(u32,u32),egui::Rect),b:&(Vec<u8>,(u32,u32),egui::Rect))->usize {
    assert_eq!(a.1,b.1);assert_eq!(a.2,b.2);
    a.0.as_chunks::<4>().0.iter().zip(b.0.as_chunks::<4>().0).enumerate().filter(|(i,(left,right))| {let p=egui::pos2((*i as u32%a.1.0) as f32,(*i as u32/a.1.0) as f32);a.2.shrink(5.0).contains(p)&&p.y>a.2.min.y+40.0&&left!=right}).count()
}
fn changed(a:&[u8],b:&[u8])->usize { assert_eq!(a.len(),b.len()); a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0).filter(|(a,b)|a!=b).count() }

// A literal independent oracle: no Recipe::face_uvs, bake_model, face normals,
// generated corner tables or baked source cloning participates in this model.
fn expected_model(recipe_bytes:&[u8], revised_uv:bool, revised_material:bool)->StaticModel {
    let positions=[
        [[1.25,-0.75,-1.25],[1.25,1.5,-1.25],[1.25,1.5,0.875],[1.25,-0.75,0.875]],
        [[-1.0,-0.75,0.875],[-1.0,1.5,0.875],[-1.0,1.5,-1.25],[-1.0,-0.75,-1.25]],
        [[-1.0,1.5,0.875],[1.25,1.5,0.875],[1.25,1.5,-1.25],[-1.0,1.5,-1.25]],
        [[-1.0,-0.75,-1.25],[1.25,-0.75,-1.25],[1.25,-0.75,0.875],[-1.0,-0.75,0.875]],
        [[-1.0,-0.75,0.875],[1.25,-0.75,0.875],[1.25,1.5,0.875],[-1.0,1.5,0.875]],
        [[1.25,-0.75,-1.25],[-1.0,-0.75,-1.25],[-1.0,1.5,-1.25],[1.25,1.5,-1.25]],
    ];
    let normals=[[1.0,0.0,0.0],[-1.0,0.0,0.0],[0.0,1.0,0.0],[0.0,-1.0,0.0],[0.0,0.0,1.0],[0.0,0.0,-1.0]];
    let uv=[
        if revised_uv { [[0.375,0.25],[0.875,0.25],[0.875,1.0],[0.375,1.0]] } else { [[0.125,0.25],[0.625,0.25],[0.625,1.0],[0.125,1.0]] },
        [[0.25,0.375],[0.25,1.125],[-0.25,1.125],[-0.25,0.375]],
        [[0.375,0.125],[-0.875,0.125],[-0.875,-0.375],[0.375,-0.375]],
        [[0.5,0.25],[0.5,-0.25],[1.75,-0.25],[1.75,0.25]],
        [[0.125,0.375],[0.125,1.125],[-1.375,1.125],[-1.375,0.375]],
        [[0.375,0.5],[0.375,-1.0],[1.125,-1.0],[1.125,0.5]],
    ];
    let ids=["box.blockout.json#node=positive_x","box.blockout.json#node=negative_x","box.blockout.json#node=positive_y","box.blockout.json#node=negative_y","box.blockout.json#node=positive_z","box.blockout.json#node=negative_z"];
    let slots=[0,1,0,1,u32::from(revised_material),1];
    StaticModel::new(ModelSource {format:"orr_static_model".into(),version:1,asset_id:"box.blockout.json".into(),dependencies:vec![Dependency {uri:"$source".into(),sha256:digest(recipe_bytes)}],primitives:(0..6).map(|i| Primitive{id:ids[i].into(),vertices:(0..4).map(|v|Vertex{position:positions[i][v],normal:normals[i],uv:uv[i][v]}).collect(),indices:vec![0,1,2,0,2,3],material:slots[i],transform:IDENTITY}).collect(),materials:vec![
        Material{base_color:[1.0,1.0,1.0,1.0],image:0,linear_filter:false,wrap_s:Wrap::Repeat,wrap_t:Wrap::Repeat},
        Material{base_color:[0.3,0.65,1.0,1.0],image:0,linear_filter:false,wrap_s:Wrap::Repeat,wrap_t:Wrap::Repeat},
    ],images:vec![Image{width:4,height:4,rgba8:vec![
        255,32,16,255,224,208,32,255,24,176,64,255,16,96,240,255,
        144,24,192,255,240,120,24,255,32,216,200,255,192,64,96,255,
        48,64,112,255,176,240,80,255,104,32,40,255,80,144,224,255,
        248,192,160,255,64,112,32,255,208,96,232,255,24,40,56,255,
    ]}]}).unwrap()
}

fn simulate(scene:&PreparedScene,ticks:u32,input:RoomInput)->Simulation<RoomEscapeV1> {
    let mut sim=scene.simulation().unwrap();
    for tick in 1..=ticks { let mut samples=TickInputs::new(u64::from(tick),1); samples.set_input(PlayerSlot(0),input); sim.step(&samples); }
    sim
}
// This branch is a fresh-process production-admission probe, not a claim that
// room_escape exposes Frame bytes. The real runtime's observations are its PNG
// and checksum output. Both processes consume the same read-only exported bytes.
fn child_probe() {
    assert_eq!(std::env::var("ORR_BLOCKOUT_CHILD").as_deref(),Ok("source-hidden-v1"));
    assert_eq!(std::env::var("ORR_BLOCKOUT_NAMESPACE").as_deref(),Ok("verified"));
    let root=PathBuf::from(std::env::var_os("ORR_BLOCKOUT_CHILD_PROJECT").unwrap());
    let hidden:Vec<PathBuf>=serde_json::from_str(&std::env::var("ORR_BLOCKOUT_CHILD_HIDDEN").unwrap()).unwrap();
    assert!(hidden.len()>=3);
    for file in hidden { assert!(!file.is_file(),"authoring source visible in Frame probe: {}",file.display()); }
    let forbidden=root.join("probe-must-not-write");
    let error=fs::write(&forbidden,b"forbidden").expect_err("bundle must reject writes");
    assert_eq!(error.raw_os_error(),Some(30),"expected a read-only filesystem");
    let ticks=std::env::var("ORR_BLOCKOUT_CHILD_TICKS").unwrap().parse::<u32>().unwrap(); assert!(ticks<=180);
    let held=std::env::var("ORR_BLOCKOUT_CHILD_INPUT").unwrap();
    let input=match held.as_str() { "idle"=>RoomInput::default(),"right"=>RoomInput{move_x:1,..RoomInput::default()},"right,interact"=>RoomInput{move_x:1,buttons:INTERACT,..RoomInput::default()}, _=>panic!("unrecognized probe input") };
    let admitted=PreparedProject::open(root).unwrap();
    let sim=simulate(admitted.scene(),ticks,input);
    let output=PathBuf::from(std::env::var_os("ORR_BLOCKOUT_CHILD_OUTPUT").unwrap());
    assert!(!output.exists()); fs::write(output,sim.frame().to_bytes()).unwrap();
    println!("PreparedProject probe tick={} checksum=0x{:016x}",sim.tick(),sim.frame().checksum());
}
struct Production<'a> { fixture:&'a Fixture, workspace:PathBuf, runtime:PathBuf, exporter:PathBuf, probe:PathBuf, originals:Vec<PathBuf> }
impl<'a> Production<'a> {
    fn new(fixture:&'a Fixture)->Self {
        let base=fixture.temp.path();
        let workspace=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().unwrap(); assert!(!base.starts_with(&workspace));
        for name in ["tools","probe","empty","captures","home","xdg-data","xdg-config","xdg-cache","xdg-runtime"] { fs::create_dir_all(base.join(name)).unwrap(); }
        let mut originals=Vec::new(); let mut copied=Vec::new();
        for (key,name) in [("ORR_ROOM_RUNTIME","room_escape"),("ORR_ROOM_EXPORTER","orr_export_room")] {
            let original=PathBuf::from(std::env::var_os(key).unwrap_or_else(||panic!("mandatory acceptance needs {key}"))).canonicalize().unwrap(); assert!(original.is_absolute()&&original.is_file());
            let copy=base.join("tools").join(name); fs::copy(&original,&copy).unwrap(); assert_eq!(file_hash(&original),file_hash(&copy)); originals.push(original); copied.push(copy);
        }
        let original=std::env::current_exe().unwrap().canonicalize().unwrap(); let probe=base.join("probe/acceptance"); fs::copy(&original,&probe).unwrap(); assert_eq!(file_hash(&original),file_hash(&probe)); originals.push(original);
        Self {fixture,workspace,runtime:copied[0].clone(),exporter:copied[1].clone(),probe,originals}
    }
    fn isolated(&self,exe:&Path,bundle:Option<&Path>)->Command {
        let base=self.fixture.temp.path(); let mut command=Command::new("bwrap");
        command.args(["--die-with-parent","--ro-bind","/","/","--dev-bind","/dev/null","/dev/null","--bind"]).arg(base).arg(base).arg("--tmpfs").arg(&self.workspace).arg("--tmpfs").arg(&self.fixture.source);
        for original in &self.originals { if !original.starts_with(&self.workspace) {command.args(["--ro-bind","/dev/null"]).arg(original);} }
        if let Some(bundle)=bundle { command.arg("--tmpfs").arg(&self.fixture.root).arg("--tmpfs").arg(base.join("tools")).arg("--ro-bind").arg(bundle).arg(bundle); }
        else { command.arg("--ro-bind").arg(&self.fixture.root).arg(&self.fixture.root); }
        command.arg("--");
        if bundle.is_some() {
            command.args(["/bin/sh","-c","test ! -e \"$1\" && test ! -e \"$2\" && test ! -e \"$3\" && test ! -e \"$4\" || exit 91; shift 4; export ORR_BLOCKOUT_NAMESPACE=verified; exec \"$@\"","sh"])
                .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml"))
                .arg(self.fixture.root.join("orr.project.json")).arg(self.fixture.source.join("orr.package.json")).arg(&self.runtime);
        }
        command.arg(exe).current_dir(base.join("empty")).env("HOME",base.join("home")).env("XDG_DATA_HOME",base.join("xdg-data")).env("XDG_CONFIG_HOME",base.join("xdg-config")).env("XDG_CACHE_HOME",base.join("xdg-cache")).env("XDG_RUNTIME_DIR",base.join("xdg-runtime")).env_remove("DISPLAY").env_remove("WAYLAND_DISPLAY"); command
    }
    fn runtime(&self,bundle:Option<&Path>,ticks:u32,held:&str,png:&Path)->Command {
        let exe=bundle.map(|b|b.join("run-room-escape")).unwrap_or_else(||self.runtime.clone()); let mut command=self.isolated(&exe,bundle);
        if bundle.is_none() {command.arg("--project").arg(&self.fixture.root);}
        command.args(["--headless","--ticks",&ticks.to_string(),"--capture"]).arg(png); if !held.is_empty() {command.args(["--hold",held]);} command
    }
    fn export(&self,path:&Path) {
        let mut command=self.isolated(&self.exporter,None); command.arg("--project").arg(&self.fixture.root).arg("--runtime").arg(&self.runtime).args(["--runtime-sha256",&file_hash(&self.runtime),"--trusted-runtime","--output"]).arg(path);
        good(command.output().unwrap());
    }
    fn probe(&self,bundle:&Path,ticks:u32,held:&str,out:&Path)->String {
        let mut command=self.isolated(&self.probe,Some(bundle));
        let hidden=[Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml"),self.fixture.root.join("orr.project.json"),self.fixture.source.join("orr.package.json")];
        command.args(["--ignored","--exact",TEST,"--nocapture"]).env("ORR_BLOCKOUT_CHILD","source-hidden-v1").env("ORR_BLOCKOUT_CHILD_PROJECT",bundle.join("project")).env("ORR_BLOCKOUT_CHILD_HIDDEN",serde_json::to_string(&hidden).unwrap()).env("ORR_BLOCKOUT_CHILD_TICKS",ticks.to_string()).env("ORR_BLOCKOUT_CHILD_INPUT",if held.is_empty(){"idle"}else{held}).env("ORR_BLOCKOUT_CHILD_OUTPUT",out);
        good(command.output().unwrap())
    }
}
fn production_export(fixture:&Fixture,baseline:&PreparedScene,evidence:&Path) {
    let production=Production::new(fixture); let before_source=tree(&fixture.root);
    let staged=fixture.temp.path().join("staged export"); production.export(&staged);
    let second=fixture.temp.path().join("second export"); production.export(&second);
    assert_eq!(tree(&staged),tree(&second),"export bytes must be reproducible"); fs::remove_dir_all(second).unwrap();
    let bundle=fixture.temp.path().join("relocated read-only room"); fs::rename(staged,&bundle).unwrap(); let before=tree(&bundle);
    for (path,bytes) in tree(&bundle.join("project")) { assert_eq!(before_source.get(&path),Some(&bytes),"export changed admitted bytes {path}"); }
    assert!(!bundle.join("project").join(RECIPE).exists(),"working recipe is not a runtime entry");
    assert!(before.keys().any(|p|p.ends_with(ASSET))); assert!(before.keys().any(|p|p.ends_with("CHECKER-LICENSE.txt"))); assert!(before.keys().any(|p|p.ends_with("NOTICE.txt")));
    let mut first_pixels=Vec::new(); let mut reports=BTreeMap::new();
    for (name,ticks,held,input) in [
        ("initial",0,"",RoomInput::default()),
        ("moved",18,"right",RoomInput{move_x:1,..RoomInput::default()}),
        ("collision",180,"right",RoomInput{move_x:1,..RoomInput::default()}),
        ("interacted",18,"right,interact",RoomInput{move_x:1,buttons:INTERACT,..RoomInput::default()}),
    ] {
        let source=fixture.temp.path().join("captures").join(format!("runtime-{name}-source.png")); let exported=fixture.temp.path().join("captures").join(format!("runtime-{name}-export.png"));
        let a=good(production.runtime(None,ticks,held,&source).output().unwrap()); let b=good(production.runtime(Some(&bundle),ticks,held,&exported).output().unwrap());
        assert!(a.contains("room capture adapter:"),"actual production GPU required"); assert_eq!(a,b,"fresh production runtime stdout parity {name}"); assert!(fs::read(&source).unwrap()==fs::read(&exported).unwrap(),"fresh production PNG byte parity {name}; source={} export={}",file_hash(&source),file_hash(&exported));
        let expected=simulate(baseline,ticks,input); assert!(a.contains(&format!("room initial checksum: 0x{:016x}",baseline.frame().checksum()))); assert!(a.contains(&format!("room tick: {ticks} checksum: 0x{:016x}",expected.frame().checksum())));
        let probe=fixture.temp.path().join("captures").join(format!("prepared-project-probe-{name}.frame")); let probe_report=production.probe(&bundle,ticks,held,&probe);
        assert_eq!(fs::read(&probe).unwrap(),expected.frame().to_bytes(),"fresh exported PreparedProject Frame bytes preserve pre-authoring simulation and collision {name}");
        if name=="interacted" { assert!(a.contains("room key: 1")); }
        let mut reader=png::Decoder::new(std::io::BufReader::new(fs::File::open(&source).unwrap())).read_info().unwrap(); let mut pixels=vec![0;reader.output_buffer_size()]; let info=reader.next_frame(&mut pixels).unwrap(); pixels.truncate(info.buffer_size()); assert_eq!((info.width,info.height),(1024,768));
        if ticks==0 {first_pixels=pixels;} else {assert!(changed(&first_pixels,&pixels)>20,"input must change actual production presentation");}
        reports.insert(name,serde_json::json!({"production_runtime_exit":0,"production_runtime_stdout":a,"prepared_project_probe_exit":0,"prepared_project_probe_stdout":probe_report,"frame_sha256":file_hash(&probe)}));
    }
    let rejected=production.runtime(Some(&bundle),0,"",&bundle.join("must-not-write.png")).output().unwrap(); assert!(!rejected.status.success()); assert!(String::from_utf8_lossy(&rejected.stderr).contains("Read-only file system"));
    assert_eq!(tree(&bundle),before,"all exported bundle bytes unchanged"); assert_eq!(tree(&fixture.root),before_source,"all authoring/package bytes unchanged");
    copy_tree(&fixture.temp.path().join("captures"),evidence); copy_tree(&fixture.root,&evidence.join("authored-project")); fs::copy(bundle.join("orr.export.json"),evidence.join("orr.export.json")).unwrap();
    fs::write(evidence.join("production-receipt.json"),serde_json::to_vec_pretty(&serde_json::json!({"runtime_sha256":file_hash(&production.runtime),"exporter_sha256":file_hash(&production.exporter),"acceptance_executable_sha256":file_hash(&production.probe),"observations":reports,"exported_file_inventory":before,"frame_evidence_boundary":"Frame bytes are from a separate fresh PreparedProject probe consuming the source-hidden read-only export, not bytes emitted by room_escape"})).unwrap()).unwrap();
}

fn number(h:&mut Harness<'_,EditorApp>,label:&str,text:&str) {
    h.get_by_role_and_label(Role::SpinButton,label).scroll_to_me(); h.run_steps(3); h.get_by_role_and_label(Role::SpinButton,label).focus(); h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND,Key::A); h.run_steps(2); h.get_by_role_and_label(Role::SpinButton,label).type_text(text); h.run_steps(2); h.key_press(Key::Enter); settle(h);
}
fn bake_assign(h:&mut Harness<'_,EditorApp>)->String {
    click(h,"Save box recipe"); assert!(h.state().models.blockout.error().is_none(),"{:?}",h.state().models.blockout.error());
    click(h,"Bake immutable box"); assert!(h.state().models.blockout.error().is_none(),"{:?}",h.state().models.blockout.error());
    let package=h.state().models.blockout.last_bake().unwrap().package.clone(); assert_eq!(h.state().models.package,package); assert_eq!(h.state().models.asset,ASSET);
    h.state_mut().models.transform.translation=LOCAL_OFFSET;
    click(h,"Load model asset"); assert!(h.state().models.error().is_none(),"{:?}",h.state().models.error());
    click(h,"Assign static model"); assert!(h.state().models.error().is_none(),"{:?}",h.state().models.error()); package
}
fn assert_oracle(fixture:&Fixture,package:&str,bytes:&[u8],uv:bool,material:bool) {
    let loaded=model_bindings::load_asset(&fixture.root,package,ASSET).unwrap();
    assert_eq!(loaded.static_model().unwrap().source(),expected_model(bytes,uv,material).source(),"all six literal faces, UVs, material slots, image texels and source receipt");
    assert_eq!(package,format!("blockout-{}",&digest(bytes)[..48]));
}
fn assert_views_equal(actual:&[Vec<u8>],expected:&[Vec<u8>],context:&str) {
    assert_eq!(actual.len(),6);assert_eq!(expected.len(),6);
    for (i,(a,b)) in actual.iter().zip(expected).enumerate() {assert_eq!(changed(a,b),0,"{context}: {} pixel mismatch (retained captures contain full images)",VIEWS[i].0);}
}
fn assert_isolated_difference(before:&[Vec<u8>],after:&[Vec<u8>],face:usize) {
    for (i,(a,b)) in before.iter().zip(after).enumerate() {
        if i==face {assert!(changed(a,b)>100,"selected {} must visibly change",VIEWS[i].0);}
        else {assert_eq!(changed(a,b),0,"editing one face leaked into the {} frontal view",VIEWS[i].0);}
    }
}

#[test]
#[ignore = "mandatory explicit real editor GPU, production Room tools and source-hidden read-only Linux export acceptance"]
fn authored_box_widgets_six_face_gpu_and_source_hidden_room_export() {
    // The private child branch must precede every fixture/source read. The child
    // is selected with this same exact singleton test name, never a loose filter.
    if std::env::var_os("ORR_BLOCKOUT_CHILD").is_some() { child_probe(); return; }
    for key in ["ORR_REQUIRE_GPU","ORR_REQUIRE_PROJECT_ISOLATION"] { assert_eq!(std::env::var(key).as_deref(),Ok("1"),"explicit acceptance requires {key}=1"); }
    let fixture=Fixture::new();
    let temporary_evidence=tempfile::tempdir().unwrap();
    let evidence=std::env::var_os("ORR_BLOCKOUT_CAPTURE_DIR").map(PathBuf::from).unwrap_or_else(||temporary_evidence.path().to_path_buf());
    if evidence.exists() {assert_eq!(fs::read_dir(&evidence).unwrap().count(),0,"acceptance capture directory must be empty; stale captures are not evidence");}
    fs::create_dir_all(&evidence).unwrap();
    let original_scene=fs::read(fixture.root.join("room.scene.yaml")).unwrap();
    let baseline=PreparedScene::parse(std::str::from_utf8(&original_scene).unwrap()).unwrap();
    let mut h=fixture.app(); settle(&mut h); assert!(h.state().has_gpu());
    let authored_frame=frame_bytes(&mut h.state_mut().editor); let checksum=h.state().editor.checksum(); let host_history=h.state().editor.history().entries.len();
    assert_eq!(authored_frame,baseline.frame().to_bytes());
    click(&mut h,"Static model binding"); click(&mut h,"Box blockout");
    type_into(&mut h,"Box recipe path",RECIPE); click(&mut h,"New box recipe");
    assert_eq!(recipe(&h),Recipe::default());
    for (i,(face,distance)) in Face::ALL.into_iter().zip(["0.75","0.5","1","0.25","0.375","0.75"]).enumerate() {
        click(&mut h,face.name()); number(&mut h,"Move face delta",distance); let before=recipe(&h); click(&mut h,"Apply face move"); let moved=recipe(&h); assert_ne!(before,moved);
        if i==0 { click(&mut h,"Undo blockout"); assert_eq!(recipe(&h),before); click(&mut h,"Redo blockout"); assert_eq!(recipe(&h),moved); }
    }
    assert_eq!(recipe(&h).bounds.min,[-1.0,-0.75,-1.25]); assert_eq!(recipe(&h).bounds.max,[1.25,1.5,0.875]);
    let settings=[
        (["0.125","0.25","0.5","0.75"],"0 degrees"),
        (["0.25","0.375","0.75","0.5"],"90 degrees"),
        (["0.375","0.125","1.25","0.5"],"180 degrees"),
        (["0.5","0.25","0.5","1.25"],"270 degrees"),
        (["0.125","0.375","0.75","1.5"],"90 degrees"),
        (["0.375","0.5","1.5","0.75"],"270 degrees"),
    ];
    for (i,(values,turn)) in settings.into_iter().enumerate() {
        click(&mut h,Face::ALL[i].name());
        for (label,value) in ["Face U offset","Face V offset","Face U scale","Face V scale"].into_iter().zip(values) { number(&mut h,label,value); }
        click(&mut h,turn); let before=recipe(&h); click(&mut h,"Apply face UV"); let after=recipe(&h);
        assert_ne!(before.faces[i].uv,after.faces[i].uv); for other in 0..6 {if other!=i {assert_eq!(before.faces[other],after.faces[other]);}}
        if i==0 {click(&mut h,"Undo blockout");assert_eq!(recipe(&h),before);click(&mut h,"Redo blockout");assert_eq!(recipe(&h),after);}
        click(&mut h,if i%2==0 {"Checker white"} else {"Checker blue"}); click(&mut h,"Apply face material"); assert_eq!(recipe(&h).faces[i].material,(i%2) as u8);
    }
    let original_recipe=recipe(&h);
    for label in ["New box recipe","Open box recipe"] {click(&mut h,label);assert!(h.state().models.blockout.error().unwrap().contains("Save or explicitly discard"));assert_eq!(recipe(&h),original_recipe);}
    click(&mut h,"Discard box recipe");click(&mut h,"Keep box recipe");assert_eq!(recipe(&h),original_recipe);
    click(&mut h,"Save box recipe"); let bytes1=fs::read(fixture.root.join(RECIPE)).unwrap();
    assert!(!h.state().models.blockout.document.as_ref().unwrap().dirty());
    click(&mut h,"Open box recipe"); assert_eq!(recipe(&h),original_recipe); assert_eq!(fs::read(fixture.root.join(RECIPE)).unwrap(),bytes1);
    let package1=bake_assign(&mut h); assert_oracle(&fixture,&package1,&bytes1,false,false);
    let first=six_views(&mut h,&evidence,"editor-original");
    let composed_before=composed_positive_x(&mut h,&evidence.join("editor-original-composed.png"));
    let oracle_fixture=Fixture::new(); oracle_fixture.install_oracle(&expected_model(&bytes1,false,false));
    let mut oracle=oracle_fixture.app(); settle(&mut oracle);
    let independent=six_views(&mut oracle,&evidence,"independent-original"); assert_views_equal(&first,&independent,"actual production viewport must match hand-assembled model on all six sides"); drop(oracle); drop(oracle_fixture);
    // A real UV field and apply/history cycle changes only +X. The nonselected
    // frontal views cannot pass merely because some unspecified pixel changed.
    click(&mut h,"+X"); number(&mut h,"Face U offset","0.375"); click(&mut h,"Apply face UV"); let uv_recipe=recipe(&h);
    click(&mut h,"Undo blockout"); assert_eq!(recipe(&h),original_recipe); click(&mut h,"Redo blockout"); assert_eq!(recipe(&h),uv_recipe);
    let package2=bake_assign(&mut h); let bytes2=fs::read(fixture.root.join(RECIPE)).unwrap(); assert_ne!(package1,package2); assert_oracle(&fixture,&package2,&bytes2,true,false);
    let uv_pixels=six_views(&mut h,&evidence,"editor-uv-only"); assert_isolated_difference(&first,&uv_pixels,0);
    let composed_after=composed_positive_x(&mut h,&evidence.join("editor-uv-only-composed.png")); assert!(composed_change(&composed_before,&composed_after)>100,"UV pixels must reach the composed main viewport, excluding controls and status text");
    click(&mut h,"Undo model"); assert_views_equal(&six_views(&mut h,&evidence,"editor-model-undo"),&first,"editor-model-undo"); click(&mut h,"Redo model"); assert_views_equal(&six_views(&mut h,&evidence,"editor-model-redo"),&uv_pixels,"editor-model-redo");
    click(&mut h,"+Z"); click(&mut h,"Checker blue"); click(&mut h,"Apply face material");
    let package3=bake_assign(&mut h); let bytes3=fs::read(fixture.root.join(RECIPE)).unwrap(); assert_ne!(package2,package3); assert_oracle(&fixture,&package3,&bytes3,true,true);
    let final_pixels=six_views(&mut h,&evidence,"editor-material-only"); assert_isolated_difference(&uv_pixels,&final_pixels,4);
    // Replace the oracle in a fresh project, avoiding a mutable model cache or
    // any baked geometry being used as expected data.
    let final_oracle_fixture=Fixture::new(); final_oracle_fixture.install_oracle(&expected_model(&bytes3,true,true)); let mut oracle=final_oracle_fixture.app(); settle(&mut oracle);
    assert_views_equal(&final_pixels,&six_views(&mut oracle,&evidence,"independent-final"),"final literal oracle"); drop(oracle); drop(final_oracle_fixture);
    for (package,bytes,uv,material) in [(&package1,&bytes1,false,false),(&package2,&bytes2,true,false),(&package3,&bytes3,true,true)] {assert_oracle(&fixture,package,bytes,uv,material);}
    click(&mut h,"Undo model"); click(&mut h,"Undo model"); assert_views_equal(&six_views(&mut h,&evidence,"editor-two-revision-undo"),&first,"editor-two-revision-undo");
    click(&mut h,"Redo model"); click(&mut h,"Redo model"); assert_views_equal(&six_views(&mut h,&evidence,"editor-two-revision-redo"),&final_pixels,"editor-two-revision-redo");
    click(&mut h,"Save model bindings"); let model_bytes=fs::read(fixture.root.join("room.models.json")).unwrap();
    click(&mut h,"Open model bindings"); assert_views_equal(&six_views(&mut h,&evidence,"editor-bindings-reopened"),&final_pixels,"editor-bindings-reopened");
    assert_eq!(frame_bytes(&mut h.state_mut().editor),authored_frame); assert_eq!(h.state().editor.checksum(),checksum); assert_eq!(h.state().editor.history().entries.len(),host_history); assert_eq!(fs::read(fixture.root.join("room.scene.yaml")).unwrap(),original_scene);
    // Reopen the production project, then exercise Play and physical keyboard
    // control. Replay exactly the admitted samples rather than guessed timing.
    drop(h); let mut h=fixture.app(); settle(&mut h);
    assert_eq!(frame_bytes(&mut h.state_mut().editor),authored_frame); assert_views_equal(&six_views(&mut h,&evidence,"editor-project-reopened"),&final_pixels,"editor-project-reopened");
    click(&mut h,"Box blockout");type_into(&mut h,"Box recipe path",RECIPE);click(&mut h,"Open box recipe");let before_play_recipe=recipe(&h);
    h.get_by_label(orr_editor::app::LBL_PLAY).click(); wait(&mut h,"Play",|app|app.editor.can_take_control());
    for label in ["New box recipe","Open box recipe","Apply face move","Apply face UV","Apply face material","Undo blockout","Redo blockout","Save box recipe","Bake immutable box"] {assert!(h.get_by_label(label).accesskit_node().is_disabled(),"authoring control must be disabled during Play: {label}");}
    assert_eq!(recipe(&h),before_play_recipe);click(&mut h,"Take control"); wait(&mut h,"managed input",|app|app.editor.input_phase()==Phase::Active);
    key(&mut h,Key::E,true); wait(&mut h,"key interaction",|app|app.editor.snapshot().unwrap().predicted().singleton::<RoomRun>().key_collected==1); key(&mut h,Key::E,false);
    key(&mut h,Key::D,true); wait(&mut h,"physical movement",|app|app.models.placements(&app.editor)[0].instance.translation[0]>0.25); key(&mut h,Key::D,false); h.state_mut().editor.pause(); settle(&mut h);
    let live=frame_bytes(&mut h.state_mut().editor); let stopped=h.state_mut().editor.stop().unwrap().clone(); settle(&mut h); assert_eq!(h.state().editor.mode(),Mode::Edit); assert_eq!(frame_bytes(&mut h.state_mut().editor),authored_frame);
    let reader=orr_session::ReplayReader::<RoomEscapeV1>::parse(&stopped.replay).unwrap(); let mut sim=baseline.simulation().unwrap(); let mut saw_move=false; let mut saw_interact=false;
    for tick in 1..=reader.last_tick() {let (inputs,commands)=reader.tick(tick).unwrap();assert!(commands.is_empty());let input=inputs[0];saw_move|=input.move_x==1;saw_interact|=input.buttons&INTERACT!=0;let mut samples=TickInputs::new(tick,1);samples.set_input(PlayerSlot(0),input);sim.step(&samples);}
    assert!(saw_move&&saw_interact); assert_eq!(sim.frame().to_bytes(),live); assert_eq!(sim.frame().checksum(),stopped.checksum); assert_eq!(fs::read(fixture.root.join("room.models.json")).unwrap(),model_bytes); drop(h);
    production_export(&fixture,&baseline,&evidence);
    fs::write(evidence.join("acceptance-source.json"),serde_json::to_vec_pretty(&serde_json::json!({"test":TEST,"source_sha256":digest(include_bytes!("blockout_gpu.rs")),"recipe_sha256":digest(&bytes3),"retained_revisions":[package1,package2,package3],"gpu_views":VIEWS.map(|(name,_,_)|name),"oracle":"independently literal 24 positions/normals/UVs, both materials and 16 unique image colors","host_initial_frame_sha256":digest(&authored_frame),"host_play_frame_sha256":digest(&live)})).unwrap()).unwrap();
    fs::write(evidence.join("capture-inventory.json"),serde_json::to_vec_pretty(&tree(&evidence)).unwrap()).unwrap();
}
