# Keep an installed dependency independently

An indirect content package can contain models or textures you still need after removing its parent selection. `orr_pkg keep` promotes that exact installed revision to a direct selection before the parent is removed. No original source directory, registry, download or package rebuild is needed.

```sh
orr_pkg keep /path/to/project dependency-name --dry-run
orr_pkg keep /path/to/project dependency-name
# Separately inspect saved usage/removal safety, then remove the parent.
orr_pkg remove /path/to/project parent-name --dry-run
```

The dry run emits the existing `PackagePreview` JSON with one changed direct flag and no writes. The apply command emits the resulting lock. Missing names, existing direct selections, invalid arguments and failed verification return failure without success JSON. An already-direct package is deliberately an error, preventing an accidental no-op from being presented as a promotion.

## Exact selection and publication

The dependency must already be active in the installed lock. Keep adds only its exact name/version to `direct`: all package digests, manifests, dependency edges, payload bytes and previous direct selections stay identical. This does not discover inactive cache entries, upgrade versions, install sources, repair corruption or reclaim files. The existing lock schema and runtime capability rules remain unchanged.

`Project::preview_keep_dependency` verifies the complete installed lock and bytes, then prepares the same one-flag observation. `keep_dependency_checked(name, expected, approved, before_publish)` takes the existing exclusive writer, verifies again, matches both complete locks, runs the consumer's rejection callback and publishes using the existing atomic lock replacement. Callback failures and pre-publication errors preserve the old lock. The callback must not mutate package authority. Existing writer guards are never stolen.

Project manifest pinning, tilemap restrictions, lock size limits, trusted/quiescent directory assumptions and existing publication durability limits still apply. This does not isolate hostile concurrent directory replacement or external consumer-file writers. An I/O error must be inspected; it is not permission to blindly retry.

## Use in games

Model bindings and saved scenes do not change. A consuming host must still reopen with its real compiled capabilities and admit the saved content. After a successful keep, separately checked removal of a parent can leave the promoted package available to those bindings.

The CLI retains its metadata-only installation-tool capability policy and stderr notice; it is not consuming-host approval or proof that parent removal is safe. Use saved package usage and the editor's checked removal flow for Room projects. No new editor button or automatic removal is introduced.

Focused acceptance distinguishes package/API and CLI behavior from actual saved Room model admission and source-hidden exported native headless execution after keeping the model dependency and removing its unused parent. CPU checks do not certify native-window or GPU output. Windows package fixture installation currently reports access error5 before keep execution; failures are preserved and not retried. Prior Windows model-publication errors also remain unresolved.
