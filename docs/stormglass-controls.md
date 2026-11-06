# Stormglass controls — implementation in progress

Owner: connected-PC worker, command `stormglass-controls-20261106`.
Base: development `7d526ff1eac1c72d0000f403187c6b92aea71fa1` (completed PR #455).
Latest #94 handoff and open PRs were checked; the older native-creator reservation
has completed. No overlapping active controls/settings reservation was found.

Reserved scope: `defense_ui.rs`, `defense_app.rs`, new `defense_controls*` modules,
audio settings backends for internal fixed storage contracts, sample feature/lib
gates, focused controls tests/check scripts/workflow and this guide. Core ActionMap,
simulation, distribution and other owners' files are outside the change.

Planned contract: reuse ActionMap for four gameplay keys, transactional session
Apply and explicit Save in a separate controls profile. Fixed Start/Continue and
pause/cancel safety keys remain available. Focus, per-event modifiers, repeats,
capture isolation, invalid files, cross-profile isolation and process restore need
focused acceptance. Audio files must not be read or written just by enabling
controls. Native-window/device tests are separate from CPU/filesystem checks.
