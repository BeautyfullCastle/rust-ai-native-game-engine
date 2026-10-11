//! Fixed-topology CPU, strict recipe, immutable package and failure-atomic contracts.
#![cfg(feature = "models")]
#![allow(clippy::float_arithmetic)]
use orr_editor::blockout::{Blockout, Face, FaceUv, Recipe, MAX_HISTORY, MAX_RECIPE_BYTES, MODEL_ASSET, RECIPE_ASSET};
use orr_package::{Project, Runtime};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

fn project(root: &Path) -> Project { let mut runtime = Runtime::content_only(); runtime.capabilities.insert("models".into()); Project::open(root, runtime).unwrap() }
fn bake(doc: &Blockout, p: &Project) -> orr_editor::blockout::BakeResult { doc.prepare_bake(p).unwrap().commit(p, || Ok(())).unwrap() }
fn hash(bytes: &[u8]) -> String { Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect() }

#[test]
fn six_named_faces_have_independent_outward_mesh_and_uv_oracle() {
    let mut recipe = Recipe::default();
    recipe.bounds.min = [-1.0,-0.75,-1.25]; recipe.bounds.max = [1.25,1.5,0.875];
    for (i, face) in recipe.faces.iter_mut().enumerate() { face.material = (i % 2) as u8; face.uv = FaceUv { offset: [0.125,-0.25], scale: [2.0,0.5], quarter_turn: (i % 4) as u8 }; }
    let model = recipe.bake_model(&recipe.to_bytes().unwrap()).unwrap();
    let expected_corners = [
        [[1.25,-0.75,-1.25],[1.25,1.5,-1.25],[1.25,1.5,0.875],[1.25,-0.75,0.875]],
        [[-1.0,-0.75,0.875],[-1.0,1.5,0.875],[-1.0,1.5,-1.25],[-1.0,-0.75,-1.25]],
        [[-1.0,1.5,0.875],[1.25,1.5,0.875],[1.25,1.5,-1.25],[-1.0,1.5,-1.25]],
        [[-1.0,-0.75,-1.25],[1.25,-0.75,-1.25],[1.25,-0.75,0.875],[-1.0,-0.75,0.875]],
        [[-1.0,-0.75,0.875],[1.25,-0.75,0.875],[1.25,1.5,0.875],[-1.0,1.5,0.875]],
        [[1.25,-0.75,-1.25],[-1.0,-0.75,-1.25],[-1.0,1.5,-1.25],[1.25,1.5,-1.25]],
    ];
    let expected_uv = [
        [[0.125,-0.25],[2.125,-0.25],[2.125,0.25],[0.125,0.25]],
        [[0.125,-0.25],[0.125,1.75],[-0.375,1.75],[-0.375,-0.25]],
        [[0.125,-0.25],[-1.875,-0.25],[-1.875,-0.75],[0.125,-0.75]],
        [[0.125,-0.25],[0.125,-2.25],[0.625,-2.25],[0.625,-0.25]],
    ];
    let normals = [[1.0,0.0,0.0],[-1.0,0.0,0.0],[0.0,1.0,0.0],[0.0,-1.0,0.0],[0.0,0.0,1.0],[0.0,0.0,-1.0]];
    let names = ["positive_x","negative_x","positive_y","negative_y","positive_z","negative_z"];
    assert_eq!(model.source().primitives.len(), 6);
    assert_eq!(model.source().primitives.iter().map(|p| p.vertices.len()).sum::<usize>(), 24);
    assert_eq!(model.source().primitives.iter().map(|p| p.indices.len()).sum::<usize>(), 36);
    for (i,p) in model.source().primitives.iter().enumerate() {
        assert_eq!(p.id, format!("box.blockout.json#node={}", names[i])); assert_eq!(p.indices, [0,1,2,0,2,3]); assert_eq!(p.material, (i % 2) as u32);
        for (j,v) in p.vertices.iter().enumerate() { assert_eq!(v.position, expected_corners[i][j]); assert_eq!(v.normal, normals[i]); assert_eq!(v.uv, expected_uv[i%4][j]); }
        for triangle in p.indices.as_chunks::<3>().0 {
            let [a,b,c] = [p.vertices[triangle[0] as usize].position,p.vertices[triangle[1] as usize].position,p.vertices[triangle[2] as usize].position];
            let u = [b[0]-a[0],b[1]-a[1],b[2]-a[2]]; let v = [c[0]-a[0],c[1]-a[1],c[2]-a[2]];
            let cross = [u[1]*v[2]-u[2]*v[1],u[2]*v[0]-u[0]*v[2],u[0]*v[1]-u[1]*v[0]];
            assert!(cross.iter().zip(normals[i]).map(|(a,b)| a*b).sum::<f32>() > 0.0);
        }
    }
    assert_eq!(model.source().materials.len(), 2);
    let pixels = &model.source().images[0].rgba8;
    assert_eq!(pixels.len(), 64);
    assert_eq!(&pixels[0..4], &[255,32,16,255]); assert_eq!(&pixels[60..64], &[24,40,56,255]);
    assert_eq!(pixels.as_chunks::<4>().0.iter().collect::<std::collections::BTreeSet<_>>().len(),16);
}
#[test]
fn malformed_versions_duplicates_records_and_numeric_inputs_are_rejected() {
    let text = String::from_utf8(Recipe::default().to_bytes().unwrap()).unwrap();
    let valid: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut bad = vec![text.replacen("\"version\": 1", "\"version\": 1, \"version\": 1",1),text.replacen("\"material\": 0", "\"material\": 0, \"material\": 1",1),text.replacen("\"scale\": [", "\"mystery\": 0, \"scale\": [",1),text.replacen("0.5", "1e1000",1),text.replacen("0.5", "NaN",1)];
    for (path,value) in [("/version",serde_json::json!(2)),("/format",serde_json::json!("other")),("/bounds/min/0",serde_json::json!(1)),("/bounds/min/1",serde_json::json!(0.5)),("/faces/0/material",serde_json::json!(2)),("/faces/0/uv/scale/0",serde_json::json!(0)),("/faces/0/uv/quarter_turn",serde_json::json!(4)),("/bounds",serde_json::json!([[-0.5,-0.5,-0.5],[0.5,0.5,0.5]])),("/faces/0",serde_json::json!([[ [0,0],[1,1],0 ],0])),("/faces/0/uv",serde_json::json!([[0,0],[1,1],0]))] {
        let mut v=valid.clone(); *v.pointer_mut(path).unwrap()=value; bad.push(v.to_string());
    }
    bad.push(serde_json::json!(["orr_box_blockout",1,valid["bounds"],valid["faces"]]).to_string());
    for bytes in bad { assert!(Recipe::from_bytes(bytes.as_bytes()).is_err(), "accepted {bytes}"); }
    assert!(Recipe::from_bytes(&vec![b' ';MAX_RECIPE_BYTES+1]).is_err());
}
#[test]
fn invalid_edits_preserve_document_history_and_redo() {
    let dir=tempfile::tempdir().unwrap(); let root=dir.path().canonicalize().unwrap(); let mut doc=Blockout::create(root.as_path(),"box.blockout.json").unwrap();
    doc.move_face(Face::PosX,0.5).unwrap(); doc.move_face(Face::NegZ,0.25).unwrap(); doc.undo();
    let original=doc.recipe().clone(); let history=doc.history_len();
    for value in [f32::NAN,f32::INFINITY,f32::MAX,-1000.0,-1.5] {
        assert!(doc.move_face(Face::PosX,value).is_err()); assert_eq!(doc.recipe(),&original); assert_eq!(doc.history_len(),history);
    }
    for scale in [-1.0,0.0,f32::MIN_POSITIVE,f32::NAN,f32::INFINITY] {
        assert!(doc.set_uv(Face::PosX,FaceUv { scale:[scale,1.0],..FaceUv::default() }).is_err()); assert_eq!(doc.recipe(),&original); assert_eq!(doc.history_len(),history);
    }
    assert!(doc.set_material(Face::PosY,2).is_err()); assert_eq!(doc.history_len(),history);
    doc.redo(); assert_eq!(doc.recipe().bounds.min[2],-0.75);
}
#[test]
fn history_is_bounded_and_new_success_clears_redo() {
    let dir=tempfile::tempdir().unwrap(); let root=dir.path().canonicalize().unwrap(); let mut doc=Blockout::create(root.as_path(),"box.blockout.json").unwrap();
    for _ in 0..100 { doc.move_face(Face::PosX,0.125).unwrap(); }
    assert_eq!(doc.history_len(),(MAX_HISTORY,0));
    for _ in 0..100 { doc.undo(); } assert_eq!(doc.history_len(),(0,MAX_HISTORY));
    doc.set_material(Face::PosZ,1).unwrap(); assert_eq!(doc.history_len(),(1,0));
}
#[test]
fn save_reopen_retains_exact_bytes_and_recipe_hash() {
    let dir=tempfile::tempdir().unwrap(); let root=dir.path().canonicalize().unwrap(); let path=root.as_path().join("box.blockout.json");
    let bytes=format!(" {} \n",serde_json::to_string(&Recipe::default()).unwrap()).into_bytes(); fs::write(&path,&bytes).unwrap();
    let mut doc=Blockout::open(root.as_path(),"box.blockout.json").unwrap(); doc.save().unwrap(); assert_eq!(fs::read(&path).unwrap(),bytes);
    let p=project(root.as_path()); let result=bake(&doc,&p); assert_eq!(result.recipe_hash,hash(&bytes));
    assert_eq!(p.read_asset(&result.package,RECIPE_ASSET).unwrap(),bytes);
    let model=orr_model::StaticModel::from_bytes(&p.read_asset(&result.package,MODEL_ASSET).unwrap()).unwrap();
    assert_eq!(model.source().dependencies[0].sha256,hash(&bytes));
    let assets=orr_editor::model_bindings::list_assets(root.as_path()).unwrap(); assert_eq!(assets.len(),1); assert_eq!(assets[0].asset,MODEL_ASSET);
}
#[test]
fn immutable_revisions_are_idempotent_and_remain_loadable_for_binding_undo() {
    let dir=tempfile::tempdir().unwrap(); let root=dir.path().canonicalize().unwrap(); let mut doc=Blockout::create(root.as_path(),"box.blockout.json").unwrap(); doc.save().unwrap();
    let p=project(root.as_path()); let first=bake(&doc,&p); let lock=p.verify().unwrap(); let lock_bytes=fs::read(root.as_path().join("orr.packages.lock.json")).unwrap();
    assert_eq!(bake(&doc,&p),first); assert_eq!(p.verify().unwrap(),lock); assert_eq!(fs::read(root.as_path().join("orr.packages.lock.json")).unwrap(),lock_bytes);
    let first_bytes=p.read_asset(&first.package,MODEL_ASSET).unwrap();
    doc.move_face(Face::NegY,0.25).unwrap(); assert!(doc.prepare_bake(&p).is_err()); doc.save().unwrap(); let second=bake(&doc,&p);
    assert_ne!(first.package,second.package); assert_eq!(p.verify().unwrap().packages.len(),2); assert_eq!(p.read_asset(&first.package,MODEL_ASSET).unwrap(),first_bytes);
    assert_ne!(p.read_asset(&second.package,MODEL_ASSET).unwrap(),first_bytes);
    doc.undo(); doc.save().unwrap(); assert_eq!(bake(&doc,&p),first);
}
#[test]
fn failed_save_changed_file_and_failed_bake_preserve_state_and_active_lock() {
    let dir=tempfile::tempdir().unwrap(); let root=dir.path().canonicalize().unwrap(); let mut doc=Blockout::create(root.as_path(),"box.blockout.json").unwrap(); doc.save().unwrap();
    let p=project(root.as_path()); bake(&doc,&p); let before=p.verify().unwrap();
    doc.move_face(Face::PosX,0.25).unwrap(); doc.save().unwrap();
    let prepared=doc.prepare_bake(&p).unwrap(); let recipe=doc.recipe().clone(); let history=doc.history_len();
    assert!(prepared.commit(&p,||Err("injected before publication".into())).is_err()); assert_eq!(p.verify().unwrap(),before); assert_eq!(doc.recipe(),&recipe); assert_eq!(doc.history_len(),history);
    let prepared=doc.prepare_bake(&p).unwrap(); fs::write(doc.path(),b"changed externally").unwrap();
    assert!(prepared.commit(&p,||Ok(())).is_err()); assert_eq!(p.verify().unwrap(),before);
    doc.move_face(Face::NegX,0.25).unwrap(); let history=doc.history_len(); let recipe=doc.recipe().clone();
    assert!(doc.save().is_err()); assert!(doc.dirty()); assert_eq!(doc.recipe(),&recipe); assert_eq!(doc.history_len(),history); assert_eq!(fs::read(doc.path()).unwrap(),b"changed externally");
}
#[test]
fn changed_lock_rejects_stale_bake_without_publishing_its_revision() {
    let dir=tempfile::tempdir().unwrap(); let root=dir.path().canonicalize().unwrap(); let mut a=Blockout::create(root.as_path(),"a.blockout.json").unwrap(); a.save().unwrap(); let p=project(root.as_path());
    let stale=a.prepare_bake(&p).unwrap(); let mut b=Blockout::create(root.as_path(),"b.blockout.json").unwrap(); b.set_material(Face::NegZ,1).unwrap(); b.save().unwrap(); let newer=bake(&b,&p); let before=p.verify().unwrap();
    assert!(stale.commit(&p,||panic!("stale callback must not run")).is_err()); assert_eq!(p.verify().unwrap(),before); assert!(before.packages.contains_key(&newer.package));
}
#[test]
fn contained_regular_file_contract_rejects_escape_oversize_and_special_paths() {
    let dir=tempfile::tempdir().unwrap(); let root=dir.path().canonicalize().unwrap();
    for path in ["../box.blockout.json","/tmp/box.blockout.json","a/../box.blockout.json","./box.blockout.json",".orr/box.blockout.json","box.json","box\\x.blockout.json"] { assert!(Blockout::create(root.as_path(),path).is_err(),"{path}"); }
    fs::create_dir(root.as_path().join("directory.blockout.json")).unwrap(); assert!(Blockout::open(root.as_path(),"directory.blockout.json").is_err());
    fs::write(root.as_path().join("large.blockout.json"),vec![b' ';MAX_RECIPE_BYTES+1]).unwrap(); assert!(Blockout::open(root.as_path(),"large.blockout.json").is_err());
    let mut doc=Blockout::create(root.as_path(),"new.blockout.json").unwrap(); fs::write(doc.path(),b"somebody else's bytes").unwrap(); assert!(doc.save().is_err()); assert_eq!(fs::read(doc.path()).unwrap(),b"somebody else's bytes");
    #[cfg(unix)] {
        std::os::unix::fs::symlink(root.as_path().join("large.blockout.json"),root.as_path().join("link.blockout.json")).unwrap(); assert!(Blockout::open(root.as_path(),"link.blockout.json").is_err());
        std::os::unix::fs::symlink(root.as_path(),root.as_path().join("linked-root")).unwrap(); assert!(Blockout::create(&root.as_path().join("linked-root"),"box.blockout.json").is_err());
        let status=std::process::Command::new("mkfifo").arg(root.as_path().join("fifo.blockout.json")).status().unwrap(); assert!(status.success()); assert!(Blockout::open(root.as_path(),"fifo.blockout.json").is_err());
    }
}
#[test]
fn package_budget_rejects_revision_129_without_pruning_old_models() {
    let dir=tempfile::tempdir().unwrap(); let root=dir.path().canonicalize().unwrap(); let p=project(root.as_path()); let mut doc=Blockout::create(root.as_path(),"box.blockout.json").unwrap();
    for i in 0..128 { doc.move_face(Face::PosX,0.01).unwrap(); doc.save().unwrap(); bake(&doc,&p); assert_eq!(p.list().unwrap().packages.len(),i+1); }
    let before=p.verify().unwrap(); doc.move_face(Face::PosX,0.01).unwrap(); doc.save().unwrap(); assert!(doc.prepare_bake(&p).unwrap_err_string().contains("128-package")); assert_eq!(p.verify().unwrap(),before);
}
trait ErrorString { fn unwrap_err_string(self) -> String; }
impl<T> ErrorString for Result<T,String> { fn unwrap_err_string(self) -> String { match self { Ok(_) => panic!("expected error"),Err(e)=>e } } }
#[test]
fn recipe_mutation_inside_publish_callback_retains_active_lock() {
    let dir=tempfile::tempdir().unwrap(); let root=dir.path().canonicalize().unwrap(); let p=project(&root);
    let mut doc=Blockout::create(&root,"box.blockout.json").unwrap(); doc.save().unwrap(); bake(&doc,&p);
    let before=p.verify().unwrap(); doc.move_face(Face::PosY,0.5).unwrap(); doc.save().unwrap();
    let prepared=doc.prepare_bake(&p).unwrap(); let path=doc.path().to_owned();
    assert!(prepared.commit(&p,||{fs::write(path,b"changed during callback").map_err(|e|e.to_string())?;Ok(())}).is_err());
    assert_eq!(p.verify().unwrap(),before);
}
#[test]
fn preexisting_name_collision_never_overwrites_installed_content() {
    let dir=tempfile::tempdir().unwrap(); let root=dir.path().canonicalize().unwrap(); let p=project(&root);
    let mut doc=Blockout::create(&root,"box.blockout.json").unwrap(); doc.save().unwrap();
    let name=format!("blockout-{}",&hash(&fs::read(doc.path()).unwrap())[..48]);
    let source=tempfile::tempdir().unwrap(); fs::write(source.path().join("NOTICE.txt"),b"different preexisting package").unwrap();
    fs::write(source.path().join("orr.package.json"),serde_json::to_vec(&serde_json::json!({"schema":1,"name":name,"version":"1.0.0","engine":"*","capabilities":["models"],"dependencies":{},"files":["NOTICE.txt"]})).unwrap()).unwrap();
    p.install(&[source.path().to_path_buf()]).unwrap(); let before=p.verify().unwrap();
    assert!(doc.prepare_bake(&p).unwrap_err_string().contains("collision")); assert_eq!(p.verify().unwrap(),before);
    assert_eq!(p.read_asset(&name,"NOTICE.txt").unwrap(),b"different preexisting package");
}
