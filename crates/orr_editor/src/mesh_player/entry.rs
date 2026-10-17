#[used]
#[unsafe(link_section = ".orrmesh")]
static MESH_PLAYER_MARKER: [u8; 18] = *b"ORR_MESH_PLAYER_V1";

fn main() {
    std::hint::black_box(&MESH_PLAYER_MARKER);
    if let Err(error) = orr_editor::mesh_player::run() {
        eprintln!("orr_mesh_player: {error}");
        std::process::exit(2);
    }
}
