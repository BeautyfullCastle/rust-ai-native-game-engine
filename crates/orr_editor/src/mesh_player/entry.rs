#[used]
#[unsafe(link_section = ".orrmesh")]
static MESH_PLAYER_MARKER: [u8; 18] = *b"ORR_MESH_PLAYER_V1";

#[used]
#[unsafe(link_section = ".orrprop")]
static MESH_PROPS_MARKER: [u8; 17] = *b"ORR_MESH_PROPS_V1";

fn main() {
    std::hint::black_box(&MESH_PLAYER_MARKER);
    std::hint::black_box(&MESH_PROPS_MARKER);
    if let Err(error) = orr_editor::mesh_player::run() {
        eprintln!("orr_mesh_player: {error}");
        std::process::exit(2);
    }
}
