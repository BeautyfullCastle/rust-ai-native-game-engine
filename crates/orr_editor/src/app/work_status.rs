//! One typed retained-authoring inventory for passive UI and machine observation.
//! Passive observation never probes paths or serializes full documents.
//! Only an explicit close request retains the legacy disk-aware checks.
use super::{close_review::Item, EditorApp};
use orr_remote::work_status::{DomainId, DomainRecord, EditorSnapshot};

#[derive(Clone, Copy)]
pub(super) enum Inspection {
    Observation,
    CloseRequest,
}

#[cfg(any(
    feature = "sprites",
    feature = "models",
    feature = "animated-models",
    feature = "irradiance-probes",
    feature = "tilemap",
    test
))]
fn loaded_unknown(domain_id: DomainId) -> DomainRecord {
    DomainRecord {
        domain_id,
        presence: orr_remote::work_status::Presence::Loaded,
        work_state: orr_remote::work_status::WorkState::Unknown,
    }
}

impl Inspection {
    // The lazy branches ensure passive observation cannot run a disk-aware predicate.
    #[cfg(any(
        feature = "sprites",
        feature = "models",
        feature = "animated-models",
        feature = "irradiance-probes",
        test
    ))]
    pub(super) fn sidecar(
        self,
        id: DomainId,
        memory: impl FnOnce() -> bool,
        close: impl FnOnce() -> bool,
    ) -> DomainRecord {
        match self {
            Self::Observation => {
                if memory() {
                    DomainRecord::observed(id, true)
                } else {
                    loaded_unknown(id)
                }
            }
            Self::CloseRequest => DomainRecord::observed(id, close()),
        }
    }
}

impl EditorApp {
    pub(super) fn work_status_domains(&self, inspection: Inspection) -> Vec<DomainRecord> {
        // Builds without any disk-aware sidecar feature still share this signature.
        let _ = inspection;
        vec![
            {
                #[cfg(feature = "sprites")]
                {
                    match self.sprites.bindings.as_ref() {
                        Some(value) => inspection.sidecar(
                            DomainId::SpriteBindings,
                            || value.memory_dirty(),
                            || value.dirty(),
                        ),
                        None => DomainRecord::not_loaded(DomainId::SpriteBindings),
                    }
                }
                #[cfg(not(feature = "sprites"))]
                {
                    DomainRecord::disabled(DomainId::SpriteBindings)
                }
            },
            {
                #[cfg(feature = "tilemap")]
                {
                    match self.tilemap.as_ref() {
                        Some(value) => match inspection {
                            Inspection::Observation => loaded_unknown(DomainId::Tilemap),
                            Inspection::CloseRequest => {
                                DomainRecord::observed(DomainId::Tilemap, value.dirty())
                            }
                        },
                        None => DomainRecord::not_loaded(DomainId::Tilemap),
                    }
                }
                #[cfg(not(feature = "tilemap"))]
                {
                    DomainRecord::disabled(DomainId::Tilemap)
                }
            },
            {
                #[cfg(feature = "collect-ui")]
                {
                    match self.collect_ui.as_ref().map(|v| v.dirty()) {
                        Some(blocked) => {
                            DomainRecord::observed(DomainId::CollectUiDocument, blocked)
                        }
                        None => DomainRecord::not_loaded(DomainId::CollectUiDocument),
                    }
                }
                #[cfg(not(feature = "collect-ui"))]
                {
                    DomainRecord::disabled(DomainId::CollectUiDocument)
                }
            },
            {
                #[cfg(feature = "collect-ui")]
                {
                    match if self.collect_ui_staged_authoring() {
                        Some(true)
                    } else {
                        self.collect_ui.as_ref().map(|_| false)
                    } {
                        Some(blocked) => {
                            DomainRecord::observed(DomainId::CollectUiPreview, blocked)
                        }
                        None => DomainRecord::not_loaded(DomainId::CollectUiPreview),
                    }
                }
                #[cfg(not(feature = "collect-ui"))]
                {
                    DomainRecord::disabled(DomainId::CollectUiPreview)
                }
            },
            {
                #[cfg(feature = "models")]
                {
                    match self.models.bindings.as_ref() {
                        Some(value) => inspection.sidecar(
                            DomainId::ModelBindings,
                            || value.memory_dirty(),
                            || value.dirty(),
                        ),
                        None => DomainRecord::not_loaded(DomainId::ModelBindings),
                    }
                }
                #[cfg(not(feature = "models"))]
                {
                    DomainRecord::disabled(DomainId::ModelBindings)
                }
            },
            {
                #[cfg(feature = "animated-models")]
                {
                    match self.animated_models.bindings.as_ref() {
                        Some(value) => inspection.sidecar(
                            DomainId::AnimationBindings,
                            || value.memory_dirty(),
                            || value.dirty(),
                        ),
                        None => DomainRecord::not_loaded(DomainId::AnimationBindings),
                    }
                }
                #[cfg(not(feature = "animated-models"))]
                {
                    DomainRecord::disabled(DomainId::AnimationBindings)
                }
            },
            {
                #[cfg(feature = "terrain")]
                {
                    match self
                        .terrain
                        .document
                        .terrain()
                        .map(|_| self.terrain.document.dirty())
                    {
                        Some(blocked) => DomainRecord::observed(DomainId::Terrain, blocked),
                        None => DomainRecord::not_loaded(DomainId::Terrain),
                    }
                }
                #[cfg(not(feature = "terrain"))]
                {
                    DomainRecord::disabled(DomainId::Terrain)
                }
            },
            {
                #[cfg(feature = "irradiance-probes")]
                {
                    match self.irradiance.bindings.as_ref() {
                        Some(value) => inspection.sidecar(
                            DomainId::IrradianceBindings,
                            || value.memory_dirty(),
                            || value.dirty(),
                        ),
                        None => DomainRecord::not_loaded(DomainId::IrradianceBindings),
                    }
                }
                #[cfg(not(feature = "irradiance-probes"))]
                {
                    DomainRecord::disabled(DomainId::IrradianceBindings)
                }
            },
            {
                #[cfg(feature = "room-project")]
                {
                    match self.room_camera.as_ref().map(|v| v.dirty()) {
                        Some(blocked) => DomainRecord::observed(DomainId::RoomCamera, blocked),
                        None => DomainRecord::not_loaded(DomainId::RoomCamera),
                    }
                }
                #[cfg(not(feature = "room-project"))]
                {
                    DomainRecord::disabled(DomainId::RoomCamera)
                }
            },
            {
                #[cfg(feature = "mesh-navigation")]
                {
                    if self.mesh_navigation.has_unplaced_draft() {
                        DomainRecord::observed(DomainId::MeshGeometryArt, true)
                    } else {
                        DomainRecord::unknown(DomainId::MeshGeometryArt)
                    }
                }
                #[cfg(not(feature = "mesh-navigation"))]
                {
                    DomainRecord::disabled(DomainId::MeshGeometryArt)
                }
            },
            {
                #[cfg(feature = "mesh-navigation")]
                {
                    if self.mesh_camera.dirty() {
                        DomainRecord::observed(DomainId::MeshCamera, true)
                    } else {
                        DomainRecord::unknown(DomainId::MeshCamera)
                    }
                }
                #[cfg(not(feature = "mesh-navigation"))]
                {
                    DomainRecord::disabled(DomainId::MeshCamera)
                }
            },
            {
                #[cfg(feature = "mesh-navigation")]
                {
                    if self.mesh_lighting.dirty() {
                        DomainRecord::observed(DomainId::MeshLighting, true)
                    } else {
                        DomainRecord::unknown(DomainId::MeshLighting)
                    }
                }
                #[cfg(not(feature = "mesh-navigation"))]
                {
                    DomainRecord::disabled(DomainId::MeshLighting)
                }
            },
            {
                #[cfg(feature = "mesh-navigation")]
                {
                    if self.mesh_hud.dirty() {
                        DomainRecord::observed(DomainId::MeshHud, true)
                    } else {
                        DomainRecord::unknown(DomainId::MeshHud)
                    }
                }
                #[cfg(not(feature = "mesh-navigation"))]
                {
                    DomainRecord::disabled(DomainId::MeshHud)
                }
            },
            {
                #[cfg(feature = "mesh-mission")]
                {
                    if self.mesh_mission_dirty() {
                        DomainRecord::observed(DomainId::MeshMission, true)
                    } else {
                        DomainRecord::unknown(DomainId::MeshMission)
                    }
                }
                #[cfg(not(feature = "mesh-mission"))]
                {
                    DomainRecord::disabled(DomainId::MeshMission)
                }
            },
            {
                #[cfg(feature = "mesh-hazards")]
                {
                    if self.mesh_hazards_dirty() {
                        DomainRecord::observed(DomainId::MeshHazards, true)
                    } else {
                        DomainRecord::unknown(DomainId::MeshHazards)
                    }
                }
                #[cfg(not(feature = "mesh-hazards"))]
                {
                    DomainRecord::disabled(DomainId::MeshHazards)
                }
            },
            {
                #[cfg(feature = "mesh-animated-models")]
                {
                    if self.mesh_animation_dirty() {
                        DomainRecord::observed(DomainId::MeshAnimation, true)
                    } else {
                        DomainRecord::unknown(DomainId::MeshAnimation)
                    }
                }
                #[cfg(not(feature = "mesh-animated-models"))]
                {
                    DomainRecord::disabled(DomainId::MeshAnimation)
                }
            },
        ]
    }

    pub(super) fn publish_work_status(&mut self, domains: Vec<DomainRecord>) {
        self.editor.publish_work_status(EditorSnapshot {
            domains,
            operations: self.editor.work_status_operations(),
        });
    }
}

/// These labels are a projection of typed blockers, never a second classifier.
pub(super) fn close_item(id: DomainId) -> Item {
    let (name, action) = match id {
        DomainId::SpriteBindings => ("Sprite bindings · unsaved", "Use Save bindings in Sprite bindings (view only). Scene Save does not save this sidecar."),
        DomainId::Tilemap => ("Tilemap · unsaved", "Use Save tilemap or Undo tile. Scene Save does not save tiles."),
        DomainId::CollectUiDocument => ("Collect UI document · unsaved", "Use Save UI document in Collect UI document. Scene Save does not save this document."),
        DomainId::CollectUiPreview => ("Collect UI preview · unapplied or unfinished", "Finish or cancel the gesture in the authored-screen preview. Apply accepted changes there, then Save UI document."),
        DomainId::ModelBindings => ("Model bindings · unsaved", "Use Save model bindings in the model inspector."),
        DomainId::AnimationBindings => ("Animation bindings · unsaved", "Use Save animation bindings in Animation authoring."),
        DomainId::Terrain => ("Terrain · unsaved", "Use Save terrain or Terrain Undo in the terrain inspector."),
        DomainId::IrradianceBindings => ("Irradiance bindings · unsaved", "Use Save irradiance in the irradiance inspector."),
        DomainId::RoomCamera => ("Room camera · unsaved or unapplied", "Use Apply camera for staged fields, then Save camera; Undo camera edit is separate."),
        DomainId::MeshGeometryArt => ("Mesh geometry / art · unsaved, unapplied or unfinished", "Finish geometry/vertex work and apply or resolve drafts in Static-mesh navigation. Save accepted scene and art changes with the existing workspace controls."),
        DomainId::MeshCamera => ("Mesh camera · unsaved or unapplied", "Use Apply game camera for staged fields, then Save game camera."),
        DomainId::MeshLighting => ("Mesh lighting · unsaved or unapplied", "Use Apply game lighting for staged fields, then Save game lighting."),
        DomainId::MeshHud => ("Mesh HUD · unsaved or unapplied", "Use Apply game HUD for staged fields, then Save game HUD."),
        DomainId::MeshMission => ("Mesh mission · unapplied or unfinished", "Finish or cancel point picking and apply or resolve mission fields in Mesh mission, then save the scene."),
        DomainId::MeshHazards => ("Mesh hazards · unapplied or unfinished", "Finish or cancel hazard picking and apply or resolve hazard fields in Mesh hazards, then save the scene."),
        DomainId::MeshAnimation => ("Mesh animation · unsaved or unapplied", "Resolve the loaded candidate and staged fields with Apply mesh animation, then Save mesh animation."),
    };
    Item {
        name,
        action,
        last_close: false,
    }
}
