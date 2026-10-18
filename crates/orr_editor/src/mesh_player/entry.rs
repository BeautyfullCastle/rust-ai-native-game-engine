#[used]
#[unsafe(link_section = ".orrmesh")]
static MESH_PLAYER_MARKER: [u8; 18] = *b"ORR_MESH_PLAYER_V1";

#[used]
#[unsafe(link_section = ".orrprop")]
static MESH_PROPS_MARKER: [u8; 17] = *b"ORR_MESH_PROPS_V1";

#[used]
#[unsafe(link_section = ".orrrot")]
static MESH_ROTATION_MARKER: [u8; 20] = *b"ORR_MESH_ROTATION_V1";

#[used]
#[unsafe(link_section = ".orrcam")]
static MESH_CAMERA_MARKER: [u8; 18] = *b"ORR_MESH_CAMERA_V1";

#[used]
#[unsafe(link_section = ".orrmat")]
static MESH_MATERIAL_MARKER: [u8; 20] = *b"ORR_MESH_MATERIAL_V1";

#[cfg(feature = "mesh-animated-models")]
#[used]
#[unsafe(link_section = ".orranim")]
static MESH_ANIMATION_MARKER: [u8; 21] = *b"ORR_MESH_ANIMATION_V1";

#[used]
#[unsafe(link_section = ".orrsurf")]
static MESH_SURFACE_MARKER: [u8; 19] = *b"ORR_MESH_SURFACE_V1";

#[used]
#[unsafe(link_section = ".orrlit")]
static MESH_LIGHTING_MARKER: [u8; 20] = *b"ORR_MESH_LIGHTING_V1";

#[used]
#[unsafe(link_section = ".orrctrl")]
static MESH_INPUT_MARKER: [u8; 17] = *b"ORR_MESH_INPUT_V1";

#[used]
#[unsafe(link_section = ".orrface")]
static MESH_FACING_MARKER: [u8; 18] = *b"ORR_MESH_FACING_V1";

#[used]
#[unsafe(link_section = ".orrchar")]
static MESH_CHARACTER_MARKER: [u8; 21] = *b"ORR_MESH_CHARACTER_V1";

#[used]
#[unsafe(link_section = ".orrhud")]
static MESH_HUD_MARKER: [u8; 15] = *b"ORR_MESH_HUD_V1";

fn main() {
    std::hint::black_box(&MESH_PLAYER_MARKER);
    std::hint::black_box(&MESH_PROPS_MARKER);
    std::hint::black_box(&MESH_ROTATION_MARKER);
    std::hint::black_box(&MESH_CAMERA_MARKER);
    std::hint::black_box(&MESH_MATERIAL_MARKER);
    #[cfg(feature = "mesh-animated-models")]
    std::hint::black_box(&MESH_ANIMATION_MARKER);
    std::hint::black_box(&MESH_SURFACE_MARKER);
    std::hint::black_box(&MESH_LIGHTING_MARKER);
    std::hint::black_box(&MESH_INPUT_MARKER);
    std::hint::black_box(&MESH_FACING_MARKER);
    std::hint::black_box(&MESH_CHARACTER_MARKER);
    std::hint::black_box(&MESH_HUD_MARKER);
    if let Err(error) = orr_editor::mesh_player::run() {
        eprintln!("orr_mesh_player: {error}");
        std::process::exit(2);
    }
}
