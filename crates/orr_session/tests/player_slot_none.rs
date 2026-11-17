//! `PlayerSlot::NONE` must stay the value the relay protocol sends for "no slot".

use orr_sim::PlayerSlot;

#[test]
fn none_slot_matches_the_relay_protocol() {
    assert_eq!(PlayerSlot::NONE.0, orr_proto::NO_SLOT);
}
