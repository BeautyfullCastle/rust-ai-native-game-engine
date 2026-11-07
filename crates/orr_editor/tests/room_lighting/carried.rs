use super::*;
fn click(h:&mut Harness<'_,EditorApp>,label:&str) {h.get_by_label(label).scroll_to_me();h.run_steps(2);h.get_by_label(label).click();settle(h);}
fn wait(h:&mut Harness<'_,EditorApp>,label:&str,ready:impl Fn(&EditorApp)->bool) {let deadline=Instant::now()+Duration::from_secs(8);loop {settle(h);if ready(h.state()) {return;}assert!(Instant::now()<deadline,"{label}: {:?}",h.state().editor.log());std::thread::sleep(Duration::from_millis(5));}}
fn key(h:&mut Harness<'_,EditorApp>,key:egui::Key,pressed:bool) {h.event(egui::Event::Key {key,physical_key:Some(key),pressed,repeat:false,modifiers:egui::Modifiers::NONE});}
fn player_position(app:&EditorApp)->[f32;3] {
    use orr_sample::room_game::{RoomActor,PLAYER};
    let snapshot=app.editor.snapshot().unwrap();let frame=snapshot.predicted();let (e,_)=frame.iter::<RoomActor>().find(|(_,actor)|actor.kind==PLAYER).unwrap();let poses=orr_sample::yard3d_view::editor_body_poses(frame);poses.into_iter().find(|(entity,_)|*entity==e).unwrap().1.pos.to_array()
}
#[test]
fn carried_light_widgets_save_reopen_real_movement_seek_stop_and_disable() {
    let f=Fixture::new("room-escape-3d-v1");let mut h=f.app(false);settle(&mut h);let before=frame_bytes(&mut h.state_mut().editor);
    apply_enabled(&mut h);click(&mut h,"Carry light with player");type_number(&mut h,"Carried offset X","1");type_number(&mut h,"Carried offset Z","-1");click(&mut h,"Apply lighting");
    let saved=document(&h);assert_eq!(saved.follow_player,Some([1.0,2.0,-1.0]));assert_eq!(h.state().editor.room_point_light_settings_for_presentation().unwrap(),saved.settings());
    click(&mut h,"Undo lighting edit");assert_eq!(document(&h),Document::enabled_default());click(&mut h,"Redo lighting edit");click(&mut h,"Save lighting");assert_eq!(frame_bytes(&mut h.state_mut().editor),before);drop(h);
    let mut h=f.app(false);settle(&mut h);assert_eq!(document(&h),saved);click(&mut h,orr_editor::app::LBL_PLAY);wait(&mut h,"control available",|app|app.editor.can_take_control());click(&mut h,"Take control");wait(&mut h,"input active",|app|app.editor.input_phase()==orr_editor::editor::input::Phase::Active);
    key(&mut h,egui::Key::D,true);wait(&mut h,"player moves carried light",|app| {
        if !app.editor.yard_rows_coherent() {return false;} let pos=player_position(app);let resolved=app.editor.room_point_light_settings_for_presentation().unwrap().point_light.unwrap();pos[0]>0.1 && resolved.position==[pos[0]+1.0,pos[1]+2.0,pos[2]-1.0]
    });key(&mut h,egui::Key::D,false);h.state_mut().editor.pause();settle(&mut h);wait(&mut h,"paused coherent",|app|app.editor.yard_rows_coherent() && app.editor.timeline().is_some_and(|t|!t.playing));
    h.state_mut().editor.seek(0);wait(&mut h,"seekzero",|app|app.editor.yard_rows_coherent() && app.editor.timeline().is_some_and(|t|t.tick==0));assert_eq!(h.state().editor.room_point_light_settings_for_presentation().unwrap().point_light.unwrap().position,[1.0,2.5,-1.0]);
    click(&mut h,orr_editor::app::LBL_STOP);wait(&mut h,"stopedit",|app|app.editor.mode()==Mode::Edit && app.editor.yard_rows_coherent());assert_eq!(h.state().editor.room_point_light_settings_for_presentation().unwrap(),saved.settings());
    if h.query_by_label("Carry light with player").is_none() {click(&mut h,"Room lighting");}click(&mut h,"Carry light with player");click(&mut h,"Apply lighting");assert_eq!(document(&h).follow_player,None);click(&mut h,"Undo lighting edit");assert_eq!(document(&h),saved);
    click(&mut h,"Point light enabled");click(&mut h,"Apply lighting");assert_eq!(document(&h),Document::default());f.assert_preserved();
}
#[cfg(feature="windows-export")]
#[test]
#[ignore="explicit CPU source-hidden export requires ORR_CARRIED_LIGHT_RUNTIME"]
fn carried_light_source_hidden_export_and_native_headless_follow_actual_movement() {
    use orr_sim::{TickInputs,PlayerSlot};use sha2::{Digest,Sha256};
    let f=Fixture::new("room-escape-3d-v1");let mut h=f.app(false);settle(&mut h);let mut d=Document::enabled_default();d.follow_player=Some([1.0,2.0,-1.0]);
    {let app=h.state_mut();app.editor.install_room_lighting(d.clone()).unwrap();app.room_lighting.as_mut().unwrap().apply_document(d.clone()).unwrap();app.room_lighting.as_mut().unwrap().save().unwrap();}drop(h);
    let bytes=fs::read(f.root.join("room.lighting.json")).unwrap();let source=PathBuf::from(std::env::var_os("ORR_CARRIED_LIGHT_RUNTIME").expect("explicit runtime"));let runtime=f._temp.path().join("trusted carried light runtime");fs::copy(source,&runtime).unwrap();let hash:String=Sha256::digest(fs::read(&runtime).unwrap()).iter().map(|b|format!("{b:02x}")).collect();let bundle=f._temp.path().join("deployed carried light game");
    orr_sample::project_export::export_room(&orr_sample::project_export::ExportOptions {project:f.root.clone(),runtime:runtime.clone(),runtime_sha256:hash,output:bundle.clone(),trusted_runtime:true,source_revision:None}).unwrap();assert_eq!(fs::read(bundle.join("project/room.lighting.json")).unwrap(),bytes);
    fs::rename(&f.root,f.root.with_extension("hidden")).unwrap();fs::rename(&runtime,runtime.with_extension("hidden")).unwrap();let prepared=PreparedProject::open_with_lighting(&bundle.join("project"),true).unwrap();assert_eq!(prepared.lighting().unwrap().document,d);let mut sim=prepared.scene().simulation().unwrap();
    for _ in 0..8 {let mut input=TickInputs::new(sim.tick()+1,1);input.set_input(PlayerSlot(0),orr_sample::room_game::RoomInput {move_x:1,..Default::default()});sim.step(&input);}
    let placements=orr_sample::room_view::placements(orr_bridge::FrameView::of(sim.frame()),prepared.scene().index(),prepared.models()).unwrap();
    let entity=sim.frame().entities().find(|&e|sim.frame().get::<orr_sample::room_game::RoomActor>(e).is_some_and(|actor|actor.kind==orr_sample::room_game::PLAYER)).unwrap();let guid=prepared.scene().index().guid(entity).unwrap();let binding=prepared.models().document.bindings.get(guid).unwrap();assert_eq!(binding.transform,orr_model_bindings::model_bindings::LocalTransform::default());
    let placed=placements.iter().find(|placed|placed.entity==entity).unwrap();let expected=[placed.instance.translation[0]+1.0,placed.instance.translation[1]+2.0,placed.instance.translation[2]-1.0];assert!(expected[0]>1.0);
    let before=sim.frame().to_bytes();assert_eq!(prepared.point_light_settings_for_frame(orr_bridge::FrameView::of(sim.frame())).unwrap().point_light.unwrap().position,expected);assert_eq!(sim.frame().to_bytes(),before);
    let output=std::process::Command::new(bundle.join("run-room-escape")).current_dir(f._temp.path()).args(["--headless","--ticks","8","--hold","right","--check-model-poses"]).env_remove("DISPLAY").env_remove("WAYLAND_DISPLAY").output().unwrap();assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));let text=String::from_utf8(output.stdout).unwrap();assert!(text.contains("room model poses: 9 frames checked;"));let actual:[f32;3]=serde_json::from_str(text.lines().find_map(|line|line.strip_prefix("room carried light position: ")).expect("runtime consumes saved follow policy")).unwrap();assert_eq!(actual,expected);
}
