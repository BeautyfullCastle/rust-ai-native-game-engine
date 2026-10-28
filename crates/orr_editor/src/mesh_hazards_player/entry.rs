#[used]
#[unsafe(link_section = ".orrhaz")]
static MESH_HAZARDS_PLAYER_MARKER: [u8; 26] = *b"ORR_MESH_HAZARDS_PLAYER_V1";

#[used]
#[unsafe(link_section = ".orrhcam")]
static MESH_HAZARDS_CAMERA_MARKER: [u8; 26] = *b"ORR_MESH_HAZARDS_CAMERA_V1";

fn main() {
    std::hint::black_box(&MESH_HAZARDS_PLAYER_MARKER);
    std::hint::black_box(&MESH_HAZARDS_CAMERA_MARKER);
    if let Err(error) = orr_editor::mesh_hazards_player::run() {
        eprintln!("orr_mesh_hazards_player: {error}");
        std::process::exit(2);
    }
}
