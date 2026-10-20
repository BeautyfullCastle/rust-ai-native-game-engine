# Find an export route

Select **Export guide** in the editor's bottom panel. It stays available in minimal
builds, and never opens itself, steals focus, saves files or starts an export.
The guide describes the current compiled game profile, not every engine feature.
**Open export settings** opens the profile's existing panel only when explicitly
clicked; that panel retains its original export checks and trust confirmation.

- **MeshNavigation:** enable `mesh-export`. The player target is Windows x86-64
  PE32+, including when the bundle is packaged on Linux. Saved supported
  presentation sidecars are included; incompatible players are refused.
- **MeshHazardsV1:** enable `mesh-hazards-export`. The separate scene-only
  exporter accepts Windows x86-64 PE32+ or Linux x86-64 ELF players. Hazards must
  be enabled. Adjacent presentation sidecars are refused, not silently omitted.
- **RoomEscapeV1:** the editor route requires Windows x86-64 and
  `windows-export`. Save camera/model sidecars separately from the scene.
  This exporter executes the explicitly trusted runtime for smoke verification.
- **Arena, Collect and Linux Room:** separate `orr_sample` CLI tools exist:
  `orr_export_arena`, `orr_export_collect` and `orr_export_room`. They require
  `project-export` plus the relevant profile feature, a Linux x86-64 host/player,
  a saved admitted project, explicit runtime trust and runtime SHA-256. Consult
  the matching tool's `--help`. These routes are not launched by the guide and
  their build features are not inferred to be compiled into this editor.
- Other profiles have no packaged export route in this guide. Editor Play is
  not proof of standalone distribution; no fallback exporter is selected.

Mesh and hazard panels show missing runtime, output and trust fields. Those
requirements are guidance, not a new admission policy. Opening settings does not
mean the project is ready: current source, pending work, path, capability and
no-overwrite checks still happen in the existing export flow. The guide does not
query a host or inspect paths on draw. In particular it does not treat the work
status observation as a save receipt or a coherent export-ready fence.

Only use known trusted, already-built players. Mesh/hazard exporters copy but
never execute the selected player. Linux project CLI exporters execute the
explicitly trusted player for bounded smoke verification. A hash, capability
marker or smoke result does not authenticate code. Nothing in the guide grants
trust, downloads/builds code, executes commands or expands permissions.

Tests of the actual EditorApp through CPU egui widgets are not native OS input,
GPU rendering, hosted CI or Windows-player execution. Those remain distinct
acceptance boundaries; constructing a PE bundle on Linux is not Windows runtime
acceptance.
