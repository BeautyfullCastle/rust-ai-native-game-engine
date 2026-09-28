/// A one-off, gameplay-affecting command (as opposed to a per-tick
/// [`crate::SimInput`] sample): purchases, builds, skill picks, chat,
/// debug/editor edits. Commands are sent reliably, not predicted, and
/// applied only once the tick they were submitted for is confirmed.
///
/// Implement `encode`/`decode` by hand for variable-size payloads, or use
/// [`encode_pod`]/[`decode_pod`] as the body of both when `Self: Pod`.
pub trait SimCommand: Sized + Send + Sync + 'static {
    /// Appends this command's wire representation to `out`.
    fn encode(&self, out: &mut Vec<u8>);
    /// Parses a command previously written by [`SimCommand::encode`].
    /// Returns `None` on malformed input (a corrupt replay file, a hostile
    /// peer) rather than panicking.
    fn decode(bytes: &[u8]) -> Option<Self>;
}

/// Helper body for [`SimCommand::encode`] when `T: bytemuck::Pod`: writes
/// the value's raw bytes verbatim.
///
/// ```
/// use bytemuck::{Pod, Zeroable};
/// use orr_sim::{decode_pod, encode_pod, SimCommand};
///
/// #[repr(C)]
/// #[derive(Clone, Copy, Pod, Zeroable)]
/// struct BuildTower { cell_x: i32, cell_y: i32, kind: u32 }
///
/// impl SimCommand for BuildTower {
///     fn encode(&self, out: &mut Vec<u8>) { encode_pod(self, out) }
///     fn decode(bytes: &[u8]) -> Option<Self> { decode_pod(bytes) }
/// }
/// ```
pub fn encode_pod<T: bytemuck::Pod>(v: &T, out: &mut Vec<u8>) {
    out.extend_from_slice(bytemuck::bytes_of(v));
}

/// Helper body for [`SimCommand::decode`] when `T: bytemuck::Pod`: reads the
/// value back from raw bytes (allowing unaligned input), returning `None` if
/// `bytes` is the wrong length.
pub fn decode_pod<T: bytemuck::Pod>(bytes: &[u8]) -> Option<T> {
    bytemuck::try_pod_read_unaligned(bytes).ok()
}
