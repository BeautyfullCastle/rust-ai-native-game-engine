//! Two application-owned games sharing the public static registration route.
pub mod delivery;
pub mod integration;
pub mod switchboard;
pub const SEED: u64 = 42;
pub const TICK_RATE: u32 = 60;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct NoCommand(pub u32);
impl orr_sim::SimCommand for NoCommand {
    fn encode(&self, out: &mut Vec<u8>) {
        orr_sim::encode_pod(self, out);
    }
    fn decode(bytes: &[u8]) -> Option<Self> {
        orr_sim::decode_pod(bytes)
    }
}
