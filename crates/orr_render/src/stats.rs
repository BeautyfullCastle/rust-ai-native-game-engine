//! Per-frame work recorded at the renderer's RHI calls, independent of the backend.

use std::time::Duration;

use orr_rhi::Rhi;

/// Commands actually encoded in one category of render passes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PassStats {
    /// Includes clear-only passes, even when there are no draw calls.
    pub passes: u32,
    /// Number of `Draw` and `DrawIndexed` commands (not pipeline switches).
    pub draw_calls: u32,
    /// Sum of the instance ranges of those commands. An instance drawn in
    /// both the main and shadow passes is counted once in each pass.
    pub instances: u64,
}

impl PassStats {
    pub(crate) fn draw(&mut self, instances: u32) {
        self.draw_calls += 1;
        self.instances += u64::from(instances);
    }
}

/// Work performed by the most recent completed renderer `draw` call.
///
/// Counts reset every frame, and exclude renderer construction, target resizing,
/// presentation, readback and other users of the RHI. They describe submitted
/// work, not visible pixels: clipping, occlusion and GPU execution are not measured.
/// Before the first draw all counts are zero. Timings are unavailable (`None`)
/// before that draw and on wasm32, where the standard library clock is unsupported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
    /// Submitted input instances, before repeating any in the shadow pass.
    pub shape_instances: u64,
    pub mesh_instances: u64,
    pub line_instances: u64,
    pub main: PassStats,
    pub shadow: PassStats,
    /// Calls to `Rhi::write_buffer`, including the frame uniforms.
    pub upload_calls: u32,
    /// Bytes passed to those calls, not allocated capacity or driver traffic.
    pub upload_bytes: u64,
    /// Instance buffers grown during this draw; initial constructor buffers are excluded.
    pub buffer_reallocations: u32,
    /// Size-dependent textures created during this draw, including the first frame.
    /// The fixed shadow map and caller-owned target are excluded.
    pub attachment_allocations: u32,
    /// Of those textures, how many replace an existing size-dependent attachment.
    pub attachment_reallocations: u32,
    /// Actual color/depth sample count, after backend capability fallback (2D: 1).
    pub msaa_samples: u32,
    /// CPU time for attachment preparation, globals and buffer uploads.
    pub cpu_prepare_time: Option<Duration>,
    /// CPU time for command construction and RHI encoding, excluding submission.
    pub cpu_encode_time: Option<Duration>,
    /// CPU time inside `Rhi::submit`; no wait for the GPU. This is not GPU time.
    pub cpu_submit_time: Option<Duration>,
}

impl FrameStats {
    pub(crate) fn upload<B: Rhi>(
        &mut self,
        rhi: &B,
        buffer: &B::Buffer,
        offset: u64,
        bytes: &[u8],
    ) {
        rhi.write_buffer(buffer, offset, bytes);
        self.upload_calls += 1;
        self.upload_bytes += bytes.len() as u64;
    }
}

// Do not introduce a std::time::Instant panic into the browser renderer, or add
// a browser dependency solely for diagnostics. Counts work on every backend.
pub(crate) struct CpuClock {
    #[cfg(not(target_arch = "wasm32"))]
    start: std::time::Instant,
}

impl CpuClock {
    pub(crate) fn start() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            start: std::time::Instant::now(),
        }
    }

    pub(crate) fn elapsed(&self) -> Option<Duration> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Some(self.start.elapsed())
        }
        #[cfg(target_arch = "wasm32")]
        {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_stats_are_empty_and_have_no_timing_sample() {
        let stats = FrameStats::default();
        assert_eq!(stats.main, PassStats::default());
        assert_eq!(stats.shadow, PassStats::default());
        assert_eq!(stats.upload_bytes, 0);
        assert_eq!(stats.buffer_reallocations, 0);
        assert_eq!(stats.attachment_allocations, 0);
        assert_eq!(stats.attachment_reallocations, 0);
        assert_eq!(stats.cpu_prepare_time, None);
        assert_eq!(stats.cpu_encode_time, None);
        assert_eq!(stats.cpu_submit_time, None);
    }

    #[test]
    fn draw_calls_and_instances_are_distinct() {
        let mut pass = PassStats {
            passes: 1,
            ..PassStats::default()
        };
        pass.draw(10_000);
        pass.draw(2);
        assert_eq!(
            pass,
            PassStats {
                passes: 1,
                draw_calls: 2,
                instances: 10_002
            }
        );
    }
}
