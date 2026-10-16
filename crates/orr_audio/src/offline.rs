use kira::backend::{Backend, Renderer};

use crate::AudioError;

/// A device-free backend that captures Kira's actual mixed PCM, not mock commands.
/// Rendering is caller-paced and stores no sample history.
pub struct OfflineBackend {
    renderer: Option<Renderer>,
}

impl Backend for OfflineBackend {
    type Settings = u32;
    type Error = AudioError;

    fn setup(sample_rate: u32, _: usize) -> Result<(Self, u32), Self::Error> {
        if !(8_000..=192_000).contains(&sample_rate) {
            return Err(AudioError("sample rate must be 8000..=192000 Hz".into()));
        }
        Ok((Self { renderer: None }, sample_rate))
    }

    fn start(&mut self, renderer: Renderer) -> Result<(), Self::Error> {
        self.renderer = Some(renderer);
        Ok(())
    }
}

impl OfflineBackend {
    pub(crate) fn render(&mut self, stereo: &mut [f32]) -> Result<(), AudioError> {
        if stereo.len() % 2 != 0 {
            return Err(AudioError(
                "interleaved stereo output needs an even sample count".into(),
            ));
        }
        let renderer = self
            .renderer
            .as_mut()
            .expect("AudioManager starts the backend");
        // Like a device callback: apply pending commands, then process actual mixer output.
        renderer.on_start_processing();
        renderer.process(stereo, 2);
        Ok(())
    }
}
