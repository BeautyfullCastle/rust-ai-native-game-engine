//! Runtime admission never rewrites the authored document.
use orr_render::{ImportedSceneError, PostProcessSettings};
use std::path::PathBuf;

#[derive(Default)]
pub(crate) struct RuntimePolicy {
    key: Option<(u64, Option<PathBuf>, PostProcessSettings)>,
    rejected: Option<String>,
}
impl RuntimePolicy {
    pub(crate) fn select(
        &mut self,
        epoch: u64,
        scene: Option<PathBuf>,
        requested: PostProcessSettings,
    ) -> PostProcessSettings {
        let key = (epoch, scene, requested);
        if self.key.as_ref() != Some(&key) {
            self.key = Some(key);
            self.rejected = None;
        }
        if self.rejected.is_some() {
            PostProcessSettings {
                enabled: false,
                ..requested
            }
        } else {
            requested
        }
    }
    pub(crate) fn reject_render_error(&mut self, error: &str) {
        if !self
            .key
            .as_ref()
            .is_some_and(|(_, _, settings)| settings.enabled)
        {
            return;
        }
        if [
            ImportedSceneError::UnsupportedHdr,
            ImportedSceneError::InvalidPostProcess,
            ImportedSceneError::PostProcessBudget,
        ]
        .iter()
        .any(|kind| kind.to_string() == error)
        {
            self.rejected = Some(error.into());
        }
    }
    pub(crate) fn warning(&self) -> Option<&str> {
        self.rejected.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejected_policy_is_disabled_until_scene_load_or_policy_changes() {
        let requested = PostProcessSettings {
            enabled: true,
            ..Default::default()
        };
        let mut runtime = RuntimePolicy::default();
        assert_eq!(runtime.select(1, None, requested), requested);
        runtime.reject_render_error(&ImportedSceneError::UnsupportedHdr.to_string());
        assert!(!runtime.select(1, None, requested).enabled);
        assert!(runtime.warning().is_some());
        assert!(runtime.select(2, None, requested).enabled);
        runtime.reject_render_error(&ImportedSceneError::PostProcessBudget.to_string());
        assert!(
            runtime
                .select(2, Some("different.scene".into()), requested)
                .enabled
        );
        runtime.reject_render_error(&ImportedSceneError::InvalidPostProcess.to_string());
        assert!(
            runtime
                .select(
                    2,
                    Some("different.scene".into()),
                    PostProcessSettings {
                        strength: 0.2,
                        ..requested
                    }
                )
                .enabled
        );
    }
    #[test]
    fn unrelated_render_errors_and_disabled_requests_do_not_mask_failures() {
        let mut runtime = RuntimePolicy::default();
        let requested = PostProcessSettings {
            enabled: true,
            ..Default::default()
        };
        runtime.select(1, None, requested);
        runtime.reject_render_error("model upload failed");
        assert_eq!(runtime.select(1, None, requested), requested);
        assert!(runtime.warning().is_none());
        runtime.select(1, None, PostProcessSettings::default());
        runtime.reject_render_error(&ImportedSceneError::UnsupportedHdr.to_string());
        assert!(runtime.warning().is_none());
    }
}
