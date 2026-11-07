# Animated model package upgrades

Build `orr_room_package_upgrade` with `orr_editor/package-upgrade-animated` to adopt a replacement package containing animated Room models into a **new** closed Room project. This opt-in includes Room character validation. The existing `package-upgrade-copy` feature alone keeps its static-only contract.

```sh
cargo build -p orr_editor --features package-upgrade-animated --bin orr_room_package_upgrade
orr_room_package_upgrade --project /absolute/original-room --source /absolute/replacement-package --output /absolute/new-room --acknowledge-model-changes
```

Use a new output path outside both inputs. The command requires Linux x86_64 and a trusted, quiescent input ancestry. It supports the existing closed schema-2 Room copy format; saved progress is not copied.

The replacement must preserve the exact dependency graph and pass the existing package and complete Room admission checks. Each binding retains its kind, GUID, local transform and animation settings. The selected package version and verified content hashes change; the simulation Frame stays identical. Animated bindings are decoded as animated assets, separately from static assets even if asset paths match.

Clip indices and character mappings are validated against the replacement. This does not prove that two same-index clips have the same artistic meaning: inspect replacement animations before acknowledging changes. The command does not remap reordered clips or infer intent from clip names. Missing clips, invalid skinning or character mappings, and resource-budget violations reject the candidate before publication.

Original lock, Room documents and package objects remain unchanged. The existing bounded snapshot, input rechecks, full dependency closure reconstruction and no-replace publication remain in force. Existing output paths are not overwritten. If publication reports an uncertain durability result, inspect the reported output before retrying.

Focused acceptance covers the original static-only rejection, animated copy and clip admission, original-byte preservation and real CLI copy reopening after original sources are hidden. These are automated Linux filesystem and CPU checks; they do not establish Windows, hardware rendering or physical play coverage.