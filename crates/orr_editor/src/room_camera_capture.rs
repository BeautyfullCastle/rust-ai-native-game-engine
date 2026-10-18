//! Explicit transient capture of an existing Room camera's session framing.
//! No document, host, history or filesystem mutation occurs in this module.
use orr_render::OrbitCamera;
use orr_sample::room_camera::{Document, Projection};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Context {
    pub scene: PathBuf,
    pub epoch: u64,
    pub checksum: u64,
    pub applied: Document,
}

struct Staged {
    context: Context,
    document: Document,
}

#[derive(Default)]
pub(crate) struct State {
    staged: Option<Staged>,
}

impl State {
    pub(crate) fn busy(&self) -> bool {
        self.staged.is_some()
    }

    /// Call every frame, including while the inspector is closed. None means the
    /// caller's local/Edit/coherence/clean-document or action guard is not met.
    pub(crate) fn sync(&mut self, current: Option<&Context>) -> bool {
        if self
            .staged
            .as_ref()
            .is_some_and(|staged| Some(&staged.context) != current)
        {
            self.cancel();
            return true;
        }
        false
    }

    pub(crate) fn stage(&mut self, context: Context, live: &OrbitCamera) -> Result<(), String> {
        if self.busy() {
            return Err(
                "Apply or cancel the staged viewport camera before capturing another view".into(),
            );
        }
        let document = from_view(&context.applied, live)?;
        self.staged = Some(Staged { context, document });
        Ok(())
    }

    pub(crate) fn candidate(&self, current: &Context) -> Option<&Document> {
        self.staged
            .as_ref()
            .filter(|staged| &staged.context == current)
            .map(|staged| &staged.document)
    }

    pub(crate) fn cancel(&mut self) {
        self.staged = None;
    }
}

/// Preserve the effective projection while making the live orbit the authored
/// baseline. In particular orthographic zoom is encoded into half-height, since
/// the runtime multiplies it by session_distance / authored_distance.
pub(crate) fn from_view(applied: &Document, live: &OrbitCamera) -> Result<Document, String> {
    // Reuse the presentation consumer's complete pose/FOV/matrix validation.
    applied.camera(live, (1, 1))?;
    let projection = match applied.projection {
        Projection::Perspective { fov_y_degrees } => Projection::Perspective { fov_y_degrees },
        Projection::Orthographic { half_height } => Projection::Orthographic {
            half_height: half_height * (live.distance / applied.distance),
        },
    };
    let candidate = Document {
        schema: applied.schema,
        target: live.target,
        yaw: live.yaw,
        pitch: live.pitch,
        distance: live.distance,
        projection,
    };
    // A valid session zoom can exceed the authored half-height range. Refuse it
    // instead of clamping and silently changing the requested framing.
    candidate.to_bytes()?;
    candidate.camera(&candidate.orbit(), (1, 1))?;
    Ok(candidate)
}

#[cfg(test)]
mod tests;
