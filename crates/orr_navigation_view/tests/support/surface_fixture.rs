use std::{
    fs,
    path::{Path, PathBuf},
};
pub const RELATIVE: &str = "navigation/point_demo.orrt";
pub const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 2, 8, 6, 0,
    0, 0, 114, 182, 13, 36, 0, 0, 0, 27, 73, 68, 65, 84, 120, 156, 99, 248, 122, 73, 227, 191, 134,
    219, 157, 255, 12, 119, 52, 166, 253, 55, 186, 227, 246, 31, 0, 91, 217, 10, 36, 133, 10, 8,
    251, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];
pub fn surface(relative: &str, asset: &str) -> orr_terrain_view::surface::Surface {
    orr_terrain_view::surface::Surface::from_png(relative, asset, PNG).unwrap()
}
pub fn fixture(root: &Path) -> PathBuf {
    fs::create_dir_all(root.join("navigation")).unwrap();
    let scene = root.join("point.scene.yaml");
    fs::write(
        &scene,
        include_bytes!("../../../../scenes/navigation_point.scene.yaml"),
    )
    .unwrap();
    fs::write(
        root.join(RELATIVE),
        include_bytes!("../../../../scenes/navigation/point_demo.orrt"),
    )
    .unwrap();
    fs::write(root.join("source appearance.png"), PNG).unwrap();
    fs::write(
        root.join(format!("{RELATIVE}.surface.json")),
        surface(RELATIVE, RELATIVE).to_bytes().unwrap(),
    )
    .unwrap();
    scene
}
