# Windows folder launcher — in progress

Owner: connected-PC worker, command `stormglass-windows-launcher-20261106`.
Base: development `ef4e5b5a9c919a30feacd907dd9a1bd581338d31`.
Latest #94 and open PR handoffs checked. Su's Arena sprite generator reservation
(`project_create`, `orr_new_arena`) is excluded.

Investigation: existing `pack-coop-native` publishes Linux ELF/shell bundles only.
Windows native runtime, audio bank admission and private settings already work,
but require manually assembled absolute CLI paths.

Reserved scope: new Windows launcher template, isolated launcher-probe fixture,
focused PowerShell contract/real-runtime tests and gate, workflow and this guide.
No runtime, package publisher, source generator or simulation changes planned.
The launcher will use a fixed trusted folder and explicit external settings paths;
no implicit settings creation or new packaging trust claim. Stub argument tests
and actual copied-runtime headless evidence will be reported separately. Existing
window-activation failures remain preserved; no window interaction is attempted.
