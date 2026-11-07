//! Ephemeral selected-body model draft. No binding/history/frame/file writes.
use super::*;
use orr_ecs::Entity;

pub(crate) enum DraftPlacement {
    Static(ModelPlacement),
    #[cfg(feature = "animated-models")]
    Animated(AnimatedPlacement),
}
impl DraftPlacement {
    pub(crate) fn entity(&self) -> Entity {
        match self {
            Self::Static(p) => p.entity,
            #[cfg(feature = "animated-models")]
            Self::Animated(p) => p.entity,
        }
    }
    fn duplicate(&self) -> Self {
        match self {
            Self::Static(p) => Self::Static(ModelPlacement {
                entity: p.entity,
                model: p.model.clone(),
                instance: p.instance,
            }),
            #[cfg(feature = "animated-models")]
            Self::Animated(p) => Self::Animated(AnimatedPlacement {
                entity: p.entity,
                model: p.model.clone(),
                instance: p.instance,
                pose: p.pose.clone(),
            }),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Context {
    scene: PathBuf,
    epoch: u64,
    checksum: u64,
    guid: String,
    entity: Entity,
    project: String,
    package: String,
    asset: String,
    kind: ModelKind,
    package_digest: String,
    source_hash: String,
    body_bits: Vec<u32>,
    transform_bits: Vec<u32>,
    document: Vec<u8>,
    binding_path: PathBuf,
    reserved_scene_models: usize,
    #[cfg(feature = "animated-models")]
    animation: (u32, String, String),
}
#[derive(Default)]
pub(super) struct State {
    accepted: Option<(Context, DraftPlacement)>,
}
impl State {
    fn sync(&mut self, current: Option<&Context>) {
        if self
            .accepted
            .as_ref()
            .is_some_and(|(old, _)| Some(old) != current)
        {
            self.accepted = None;
        }
    }
}
impl ModelPanel {
    fn draft_context(&self, editor: &Editor) -> Result<(Context, orr_view::Transform3), String> {
        if !self.editable(editor) || editor.screenshot_gesture_busy() {
            return Err("Draft preview requires coherent local Edit mode".into());
        }
        self.transform.validate()?;
        let guid = editor
            .selected_guid()
            .ok_or("Select a persistent body GUID")?;
        let row = editor
            .rows()
            .iter()
            .find(|r| r.guid.as_ref() == Some(&guid))
            .ok_or("Selected GUID no longer exists")?;
        let body = editor
            .yard_frame()
            .poses
            .iter()
            .find(|(e, _)| *e == row.entity)
            .map(|(_, p)| *p)
            .ok_or("Selected body pose unavailable")?;
        let (_, _, _, asset) = self
            .candidate
            .as_ref()
            .filter(|(p, a, k, _)| p == &self.package && a == &self.asset && *k == self.kind)
            .ok_or("Load the current model asset before previewing")?;
        let mut body_bits = body.pos.to_array().map(f32::to_bits).to_vec();
        body_bits.extend(body.rot.to_array().map(f32::to_bits));
        let mut transform_bits = self.transform.translation.map(f32::to_bits).to_vec();
        transform_bits.extend(self.transform.rotation.map(f32::to_bits));
        transform_bits.extend(self.transform.scale.map(f32::to_bits));
        Ok((
            Context {
                scene: Self::local_scene(editor)?,
                epoch: editor.scene_load_epoch(),
                checksum: editor.checksum(),
                guid: guid.to_string(),
                entity: row.entity,
                project: self.project.clone(),
                package: self.package.clone(),
                asset: self.asset.clone(),
                kind: self.kind,
                package_digest: asset.package_digest().into(),
                source_hash: asset.source_hash().into(),
                body_bits,
                transform_bits,
                binding_path: self
                    .bindings
                    .as_ref()
                    .ok_or("Open model bindings first")?
                    .path
                    .clone(),
                reserved_scene_models: self.reserved_scene_models,
                document: serde_json::to_vec(
                    self.bindings
                        .as_ref()
                        .ok_or("Open model bindings first")?
                        .document(),
                )
                .map_err(|e| e.to_string())?,
                #[cfg(feature = "animated-models")]
                animation: (
                    self.animation.clip_index,
                    format!("{:?}", self.animation.playback),
                    self.sample_seconds.clone(),
                ),
            },
            body,
        ))
    }
    pub(crate) fn draft_preview_active(&self) -> bool {
        self.draft_preview.accepted.is_some()
    }
    pub(crate) fn cancel_draft_preview(&mut self) {
        self.draft_preview.accepted = None;
    }
    pub(crate) fn begin_draft_preview(&mut self, editor: &Editor) -> Result<(), String> {
        // A failed explicit restart must never leave the previous preview visible.
        self.cancel_draft_preview();
        let (context, body) = self.draft_context(editor)?;
        let loaded = self.load_requested()?;
        if loaded.package_digest() != context.package_digest
            || loaded.source_hash() != context.source_hash
            || loaded.kind() != context.kind
        {
            return Err("Installed model changed; load it again before previewing".into());
        }
        let binding = match self.kind {
            ModelKind::Static => Binding::from_asset(
                self.package.clone(),
                self.asset.clone(),
                &loaded,
                self.transform,
            )?,
            ModelKind::Animated => {
                #[cfg(feature = "animated-models")]
                {
                    Binding::from_animated_asset(
                        self.package.clone(),
                        self.asset.clone(),
                        &loaded,
                        self.animation,
                        self.transform,
                    )?
                }
                #[cfg(not(feature = "animated-models"))]
                {
                    return Err("Animated preview requires animated-models".into());
                }
            }
        };
        let mut document = self
            .bindings
            .as_ref()
            .ok_or("Open model bindings first")?
            .document()
            .clone();
        let mut assets = self.loaded.clone();
        document.bindings.insert(context.guid.clone(), binding);
        assets.insert(context.guid.clone(), loaded.clone());
        assets.retain(|g, _| document.bindings.contains_key(g));
        Self::validate_candidate(editor, &document, &assets, self.reserved_scene_models)?;
        let placement = match self.kind {
            ModelKind::Static => DraftPlacement::Static(ModelPlacement {
                entity: context.entity,
                model: loaded
                    .static_model()
                    .ok_or("Expected static model")?
                    .clone(),
                instance: Self::instance(body, self.transform),
            }),
            ModelKind::Animated => {
                #[cfg(feature = "animated-models")]
                {
                    let model = loaded.animated_model().ok_or("Expected animated model")?;
                    let clip = model
                        .source()
                        .clips
                        .get(self.animation.clip_index as usize)
                        .ok_or("Preview clip is out of range")?;
                    let seconds = parse_pose_seconds(&self.sample_seconds, clip.duration())?;
                    let pose = model
                        .sample_clip(self.animation.clip_index, seconds)
                        .map_err(|e| e.to_string())?;
                    model.bounds(&pose).map_err(|e| e.to_string())?;
                    DraftPlacement::Animated(AnimatedPlacement {
                        entity: context.entity,
                        model: model.clone(),
                        instance: Self::animated_instance(body, self.transform),
                        pose,
                    })
                }
                #[cfg(not(feature = "animated-models"))]
                {
                    return Err("Animated preview requires animated-models".into());
                }
            }
        };
        self.draft_preview.accepted = Some((context, placement));
        Ok(())
    }
    /// Every draw rechecks context, even with the inspector closed. It performs no disk I/O.
    pub(crate) fn draft_placement(&mut self, editor: &Editor) -> Option<DraftPlacement> {
        if self.draft_preview.accepted.is_none() {
            return None;
        }
        let current = self.draft_context(editor).ok().map(|(c, _)| c);
        self.draft_preview.sync(current.as_ref());
        self.draft_preview
            .accepted
            .as_ref()
            .map(|(_, p)| p.duplicate())
    }
}
#[cfg(test)]
#[path = "draft_preview_tests.rs"]
mod tests;
