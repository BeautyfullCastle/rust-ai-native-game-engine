//! Passive route guidance, not export admission or a saved-source receipt.
//! No paths, host queries, source stamps or authoring blockers are inspected here.
use super::{EditorApp, Mode};
use crate::game::EditorGame;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Settings {
    #[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "arena-project-export"))]
    Arena,
    #[cfg(feature = "mesh-export")]
    Mesh,
    #[cfg(feature = "mesh-hazards-export")]
    Hazards,
    #[cfg(all(windows, target_arch = "x86_64", feature = "windows-export"))]
    Room,
}

#[derive(Debug)]
pub(super) struct Route {
    pub title: &'static str,
    pub target: &'static str,
    pub instructions: &'static str,
    pub limits: &'static str,
    pub settings: Option<Settings>,
}

pub(super) fn route(game: EditorGame) -> Route {
    match game {
        #[cfg(feature = "mesh-navigation")]
        EditorGame::MeshNavigation => Route {
            title: "MeshNavigation export",
            target: "Target: Windows x86-64 player (PE32+), including when packaged on Linux.",
            instructions: if cfg!(feature = "mesh-export") {
                "The mesh-export editor route is compiled. Open the existing export settings below."
            } else {
                "This editor was built without mesh-export. Rebuild with that optional feature to use its export settings."
            },
            limits: "Packaging requires a Windows or Linux host. Saved scene and supported presentation sidecars only. Supply a trusted matching player; the editor validates and copies it, never executes it.",
            settings: {
                #[cfg(feature = "mesh-export")]
                { Some(Settings::Mesh) }
                #[cfg(not(feature = "mesh-export"))]
                { None }
            },
        },
        #[cfg(feature = "mesh-hazards")]
        EditorGame::MeshHazardsV1 => Route {
            title: "MeshHazardsV1 export",
            target: "Target: Windows x86-64 PE32+ or Linux x86-64 ELF player; match the player to its destination OS.",
            instructions: if cfg!(feature = "mesh-hazards-export") {
                "The mesh-hazards-export editor route is compiled. Open the separate hazard export settings below."
            } else {
                "This editor was built without mesh-hazards-export. Legacy mesh-export alone does not export this profile."
            },
            limits: "Packaging requires a Windows or Linux host. Enabled hazards with an optional separately saved camera. Art, props, lighting, HUD and animation sidecars are refused. Camera bundles require a camera-capable player. The editor never executes the supplied player.",
            settings: {
                #[cfg(feature = "mesh-hazards-export")]
                { Some(Settings::Hazards) }
                #[cfg(not(feature = "mesh-hazards-export"))]
                { None }
            },
        },
        #[cfg(feature = "room-project")]
        EditorGame::RoomEscape => room_route(),
        EditorGame::Arena => arena_route(),
        #[cfg(feature = "collect-dodge")]
        EditorGame::CollectDodge => cli_route("orr_export_collect"),
        _ => Route {
            title: "No packaged export route for this profile",
            target: "Editor Play is not a standalone exported game.",
            instructions: "This profile has no packaged export route in this guide. There is no fallback to a different game's exporter.",
            limits: "Use an explicitly supported project/profile for distribution. This guide does not build or convert games.",
            settings: None,
        },
    }
}

fn arena_route() -> Route {
    #[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "arena-project-export"))]
    { Route {
        title: "Arena export", target: "Editor route: Linux x86-64 host and player only.",
        instructions: "Open the inline Arena export tab to export the explicitly saved project.",
        limits: "Scenes and sprites only. Verification executes that runtime; its hash does not establish trust. No automatic save, launch, focus or popup.",
        settings: Some(Settings::Arena),
    } }
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64", feature = "arena-project-export")))]
    { cli_route("orr_export_arena") }
}

// These are separate CLI tools, not features inferred to be linked into this editor.
#[cfg(any(not(all(target_os = "linux", target_arch = "x86_64", feature = "arena-project-export")), feature = "collect-dodge", feature = "room-project"))]
fn cli_route(binary: &'static str) -> Route {
    Route {
        title: binary,
        target: "Separate CLI route: Linux x86-64 host and player only; no editor export action.",
        instructions: match binary {
            "orr_export_arena" => "Use the orr_sample orr_export_arena tool built with project-export; pass --help for its required paths and SHA-256.",
            "orr_export_collect" => "Use the orr_sample orr_export_collect tool built with project-export and collect-dodge; pass --help for required paths and SHA-256.",
            _ => "Use the orr_sample orr_export_room tool built with project-export and room-project; pass --help for required paths and SHA-256.",
        },
        limits: "Requires a saved admitted project and an explicitly trusted, already-built runtime. The CLI executes that runtime for smoke verification; a hash does not establish trust. Nothing is run from this guide.",
        settings: None,
    }
}

#[cfg(feature = "room-project")]
fn room_route() -> Route {
    #[cfg(all(windows, target_arch = "x86_64", feature = "windows-export"))]
    {
        Route {
            title: "RoomEscapeV1 export",
            target: "Editor route: Windows x86-64 host and player.",
            instructions: "The windows-export editor route is compiled. Open the existing Room export settings below.",
            limits: "Save the scene and camera/model sidecars separately. This exporter executes the explicitly trusted runtime for smoke verification; a hash does not establish trust. Authoritative export checks remain required.",
            settings: Some(Settings::Room),
        }
    }
    #[cfg(not(all(windows, target_arch = "x86_64", feature = "windows-export")))]
    {
        let mut route = cli_route("orr_export_room");
        route.title = "RoomEscapeV1: Windows editor route unavailable";
        route.target = "Windows editor export requires a Windows x86-64 host and windows-export. Separate CLI route: Linux x86-64 host and player.";
        route
    }
}

impl EditorApp {
    pub(super) fn show_export_guide(&mut self, ui: &mut egui::Ui) {
        let route = route(self.editor.game());
        egui::ScrollArea::vertical().id_salt("export_guide").show(ui, |ui| {
            ui.strong(format!("{} — {}", self.editor.game().name(), route.title));
            if let Some(settings) = route.settings {
                // These are only the existing menu's settings-opening conditions,
                // not an export permit. The panel retains every dispatch check.
                let can_open = self.editor.mode() == Mode::Edit && self.editor.spec().is_local();
                #[cfg(all(windows, target_arch = "x86_64", feature = "windows-export"))]
                let can_open = can_open && (settings != Settings::Room || !self.package_work_pending());
                if ui.add_enabled(can_open, egui::Button::new("Open export settings")).clicked() {
                    match settings {
                        #[cfg(all(target_os = "linux", target_arch = "x86_64", feature = "arena-project-export"))]
                        Settings::Arena => self.ui.bottom_tab = super::BottomTab::ArenaExport,
                        #[cfg(feature = "mesh-export")]
                        Settings::Mesh => self.mesh_export.open = true,
                        #[cfg(feature = "mesh-hazards-export")]
                        Settings::Hazards => self.mesh_hazards_export.open = true,
                        #[cfg(all(windows, target_arch = "x86_64", feature = "windows-export"))]
                        Settings::Room => {
                            // Preserve existing menu behavior only after explicit click.
                            self.windows_export.use_current_scene(self.editor.path().as_deref());
                            self.windows_export.open = true;
                        }
                    }
                }
                if !can_open {
                    ui.label("Open settings from a local editor in Edit mode; finish Room package operations first.");
                }
            }
            ui.label(route.target);
            ui.label(route.instructions);
            ui.label(route.limits);
            ui.weak("Guidance only, not export readiness. Settings do not save, build or export; the exporter checks current source, pending work, paths and player compatibility when you explicitly export.");
        });
    }
}
