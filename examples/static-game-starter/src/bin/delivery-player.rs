//! Concrete native game entrypoint; no editor or game dispatch.
fn main() -> Result<(), String> {
    orr_static_starter::standalone::main_for::<orr_static_starter::delivery::DeliveryGame>()
}
