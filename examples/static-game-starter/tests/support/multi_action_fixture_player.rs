//! Acceptance-only player, not a shipped starter or registry entry.
mod multi_action_fixture;
fn main() {
    if let Err(error) = orr_static_starter::standalone::main_for::<multi_action_fixture::Fixture>()
    {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
