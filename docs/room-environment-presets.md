# Reuse a Room environment look

With the Room lighting panel open, save the applied environment to a new `.room-environment-preset.json` file. Enter an absolute local path. Saving a preset never replaces an existing file and does not save the Room itself.

Load a preset to stage its environment lighting in the current Room draft. Review it, then use the existing Apply, Undo/Redo and Save controls. Only environment lighting is transferred: point-light position, following and activation settings remain those of the current draft. The saved Room and normal game runtime continue using the existing lighting document format.

Presets contain a strict version-1 environment document with a 4096-byte limit. Unknown fields and invalid environment values reject. Reads require a stable bounded regular file; writes publish a synced temporary file without replacement after ancestry checks. Use trusted, quiescent local paths. Preset files do not establish publisher authenticity or redistribution rights.

Read-only, unfocused and stale-source panels cannot publish or stage changes. Loading a preset does not Apply or Save automatically. A Room without environment lighting cannot export an environment preset.

Focused CPU acceptance covers the codec, invalid input, no-replace storage, preservation of scene-specific point lighting, and the actual panel Apply/Undo/Save/reopen flow. Windows filesystem execution, GPU output, a physical game window and playback video remain unverified unless separately recorded.
