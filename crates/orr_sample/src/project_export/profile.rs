//! Closed export consumers. A profile never enables a game's optional capabilities.
use std::path::Path;
#[derive(Clone, Copy)]
pub(crate) enum Profile {
    Arena,
    #[cfg(feature = "room-project")]
    Room,
    #[cfg(feature = "collect-dodge")]
    Collect,
}
impl Profile {
    pub fn runtime(self) -> orr_package::Runtime {
        match self {
            #[cfg(feature = "room-project")] Self::Room => crate::room_project::compiled_runtime(),
            Self::Arena => crate::project_runtime::compiled_runtime(),
            #[cfg(feature = "collect-dodge")]
            Self::Collect => crate::collect_project::sprite_runtime(crate::collect_project::compiled_sprite_support()),
        }
    }
    pub fn binary(self) -> &'static str {
        match self {
            #[cfg(feature = "room-project")] Self::Room => "bin/room_escape",
            Self::Arena => "bin/arena",
            #[cfg(feature = "collect-dodge")]
            Self::Collect => "bin/collect_dodge",
        }
    }
    pub fn launcher_name(self) -> &'static str {
        match self {
            #[cfg(feature = "room-project")] Self::Room => "run-room-escape",
            Self::Arena => super::LAUNCHER_NAME,
            #[cfg(feature = "collect-dodge")]
            Self::Collect => "run-collect-dodge",
        }
    }
    pub fn launcher(self) -> &'static [u8] {
        match self { #[cfg(feature="room-project")] Self::Room=>b"#!/bin/sh\nset -eu\nroot=$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd -P)\nexec \"$root/bin/room_escape\" --project \"$root/project\" \"$@\"\n", Self::Arena=>super::LAUNCHER,#[cfg(feature="collect-dodge")]Self::Collect=>b"#!/bin/sh\nset -eu\nroot=$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd -P)\nexec \"$root/bin/collect_dodge\" --project \"$root/project\" \"$@\"\n"}
    }
    pub fn name(self) -> &'static str {
        match self {
            #[cfg(feature = "room-project")] Self::Room => "room-escape-authored-linux-x86_64-v1",
            Self::Arena => "arena-authored-linux-x86_64-v1",
            #[cfg(feature = "collect-dodge")]
            Self::Collect => "collect-dodge-authored-linux-x86_64-v1",
        }
    }
    pub fn domain(self) -> &'static [u8] {
        match self {
            #[cfg(feature = "room-project")] Self::Room => b"orrery.room-escape.export.content.v1\0",
            Self::Arena => super::DOMAIN,
            #[cfg(feature = "collect-dodge")]
            Self::Collect => b"orrery.collect-dodge.export.content.v1\0",
        }
    }
}
pub(crate) enum Prepared {
    Arena(Box<crate::project_runtime::PreparedRuntime>),
    #[cfg(feature="room-project")]
    Room(Box<crate::room_project::PreparedProject>),
    #[cfg(feature = "collect-dodge")]
    Collect(Box<crate::collect_project::PreparedProject>),
}
impl Prepared {
    pub fn open(root: &Path, profile: Profile) -> Result<Self, String> {
        match profile {
            #[cfg(feature="room-project")]
            Profile::Room => crate::room_project::PreparedProject::open_with_ui(root, cfg!(feature = "room-ui")).map(|value|Self::Room(Box::new(value))),
            Profile::Arena => crate::project_runtime::PreparedRuntime::open(root)
                .map(|value| Self::Arena(Box::new(value))),
            #[cfg(feature = "collect-dodge")]
            Profile::Collect => crate::collect_project::PreparedProject::open_with_ui(root, crate::collect_project::ProgressSupport::MetadataOnly, crate::collect_project::compiled_sprite_support(), cfg!(feature = "collect-ui"))
                .map(|value| Self::Collect(Box::new(value))),
        }
    }
    pub fn checksum(&self) -> u64 {
        match self {
            #[cfg(feature="room-project")] Self::Room(p) => p.scene().frame().checksum(),
            Self::Arena(p) => p.initial_frame().checksum(),
            #[cfg(feature = "collect-dodge")]
            Self::Collect(p) => p.scene().frame().checksum(),
        }
    }
    pub fn root(&self) -> &Path {
        match self {
            #[cfg(feature="room-project")] Self::Room(p) => p.root(),
            Self::Arena(p) => p.project().root(),
            #[cfg(feature = "collect-dodge")]
            Self::Collect(p) => p.root(),
        }
    }
    pub fn scene_path(&self) -> &Path {
        match self {
            #[cfg(feature="room-project")] Self::Room(p) => p.path(),
            Self::Arena(p) => p.project().scene().path(),
            #[cfg(feature = "collect-dodge")]
            Self::Collect(p) => p.path(),
        }
    }
    pub fn scene_text(&self) -> &str {
        match self {
            #[cfg(feature="room-project")] Self::Room(p) => p.scene().text(),
            Self::Arena(p) => p.project().scene().text(),
            #[cfg(feature = "collect-dodge")]
            Self::Collect(p) => p.scene().text(),
        }
    }
    pub fn sprites(&self) -> Option<&crate::project::PreparedSprites> {
        match self {
            #[cfg(feature="room-project")] Self::Room(_) => None,
            Self::Arena(p) => p.project().sprites(),
            #[cfg(feature = "collect-dodge")]
            Self::Collect(p) => p.sprites(),
        }
    }
    pub fn smoke(&self) -> Result<Vec<u8>, String> {
        match self {
            #[cfg(feature="room-project")] Self::Room(p) => { let checksum=p.scene().frame().checksum(); Ok(format!("room initial checksum: 0x{checksum:016x}\nroom tick: 0 checksum: 0x{checksum:016x}\nroom key: 0 won: 0\n").into_bytes()) },
            Self::Arena(p) => super::expected_smoke(p),
            #[cfg(feature = "collect-dodge")]
            Self::Collect(p) => {
                let checksum = self.checksum();
                let title =
                    crate::collect_view::title(orr_bridge::FrameView::of(p.scene().frame()));
                Ok(format!("collect initial checksum: 0x{checksum:016x}\ncollect tick: 0 checksum: 0x{checksum:016x}\n{title}\n").into_bytes())
            }
        }
    }
}
