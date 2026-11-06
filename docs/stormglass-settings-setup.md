# Prepare a Windows settings directory

The optional `orr_prepare_stormglass_settings` deployment helper removes the manual
Explorer ACL step for a **new** Stormglass controls/audio directory. Build it with
`coop-defense-controls` and copy the executable beside your trusted game launcher.
See the [Windows launcher guide](stormglass-windows-launcher.md) for commands.

The caller supplies exactly `--directory ABSOLUTE_NEW_DIRECTORY`, or `--help`.
Parents must exist; ordinary startup never invokes this tool automatically. The
launcher can invoke it only with explicit `-PrepareUserSettings` together with
`-UserSettingsDirectory`; helper failure stops the game. Without the preparation
switch, a shared directory is only forwarded to both stores. The helper creates one
empty directory, with current-user owner and protected private ACL from the first
creation operation. It does not choose defaults, read/write preferences, repair
permissions, replace an existing object, or migrate old settings. Pass the result
explicitly as `-ControlsDirectory` and optionally `-AudioSettingsDirectory`.

The implementation reuses native strict path and security helpers, checks fixed
local NTFS, opens each ancestor without reparse traversal and denies delete
sharing, then creates through the stable opened-volume GUID. Source/executable
root identity exclusions cover casing and aliases. The native player additionally
checks its scene and bank roots when admitting the directory. This does not defend
against arbitrary actions by the same user after the helper exits. It does not
promise directory-fsync/power-loss durability. If creation succeeds but verification
fails, the error says so and leaves the directory intact for manual inspection.

Focused Windows acceptance uses actual NTFS handles/ACLs and both existing
settings stores: empty creation, controls/audio save and reopen, existing private
and broad ACL/data preservation, missing-parent refusal, strict path refusal,
forbidden-root identity and junction refusal. The standalone binary has strict
argument tests and a copied-executable CLI smoke from another working directory.

```powershell
cargo test --locked -p orr_sample --features coop-defense-controls --lib defense_audio_settings:: -- --test-threads=1
cargo test --locked -p orr_sample --features coop-defense-controls --bin orr_prepare_stormglass_settings
cargo build --locked -p orr_sample --features coop-defense-controls --bin orr_prepare_stormglass_settings
& ./tools/test_stormglass_settings_setup_windows.ps1 -Executable "$PWD/target/debug/orr_prepare_stormglass_settings.exe" -EvidenceDirectory C:\Evidence\fresh-setup
```

Run in a normal current-user session: a restricted sandbox token can be denied by
the intentionally user-private ACL. Preserve failures and choose fresh evidence
directories. These are CPU/filesystem/CLI checks, not window gameplay, a video,
hardware audio, or settings persistence through the interactive UI. Windows
checkpoint storage remains unsupported; existing native Run Save is Linux-only.
