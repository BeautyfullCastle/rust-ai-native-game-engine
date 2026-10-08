//! Original closed pickup sound package.
pub(super) const PACKAGE: &str = "collect-audio-v1";
pub(super) const SOURCE: &[(&str,&[u8])] = &[
    ("orr.package.json", include_bytes!("../../../../assets/packages/collect-audio-v1/orr.package.json")),
    ("LICENSE.txt", include_bytes!("../../../../assets/packages/collect-audio-v1/LICENSE.txt")),
    ("cooked/objects/46a3ffa90212574047f6ad40c51c5e81d5644bb4d1a232a73c42d246a78f3eb2.bin", include_bytes!("../../../../assets/packages/collect-audio-v1/cooked/objects/46a3ffa90212574047f6ad40c51c5e81d5644bb4d1a232a73c42d246a78f3eb2.bin")),
    ("cooked/objects/ca829e3288092ebbe47cacc91a5ff6187e0a16711e380cad2a779e50f5b0318a.bin", include_bytes!("../../../../assets/packages/collect-audio-v1/cooked/objects/ca829e3288092ebbe47cacc91a5ff6187e0a16711e380cad2a779e50f5b0318a.bin")),
    ("cooked/view.manifest.bin", include_bytes!("../../../../assets/packages/collect-audio-v1/cooked/view.manifest.bin")),
    ("generate.py", include_bytes!("../../../../assets/packages/collect-audio-v1/generate.py")),
    ("index.json", include_bytes!("../../../../assets/packages/collect-audio-v1/index.json")),
    ("source/pickup.json", include_bytes!("../../../../assets/packages/collect-audio-v1/source/pickup.json")),
    ("source/pickup_alt.json", include_bytes!("../../../../assets/packages/collect-audio-v1/source/pickup_alt.json")),
];
