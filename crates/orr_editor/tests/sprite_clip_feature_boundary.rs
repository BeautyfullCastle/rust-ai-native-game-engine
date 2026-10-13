//! Source registration contract complements later compiled feature-off inventories.
#[test]
fn sprite_clip_registration_is_default_off_and_bin_is_gated() {
    let manifest = include_str!("../Cargo.toml");
    assert!(manifest.lines().any(|line| line == "default = []"));
    assert!(manifest.lines().any(|line| line == "sprite-clip-editor = [\"sprites\", \"dep:sha2\", \"dep:rustix\"]"));
    let bin = manifest.split("[[bin]]").find(|section| section.contains("name = \"orr_sprite_clip\"")).unwrap();
    assert!(bin.contains("required-features = [\"sprite-clip-editor\"]"));
    assert!(!bin.contains("test = false"));
    let library = include_str!("../src/lib.rs");
    for module in ["sprite_clip_authoring", "sprite_clip_panel"] {
        assert!(library.contains(&format!("#[cfg(feature = \"sprite-clip-editor\")]\npub mod {module};")));
    }
}
#[cfg(not(feature = "sprite-clip-editor"))]
#[test]
fn default_profile_does_not_enable_sprite_clip_feature() {
    const { assert!(!cfg!(feature = "sprite-clip-editor")); }
}
