# Rust fixed-source user-game bootstrap

Build the optional Linux CLI with `cargo build --release -p orr_package --bin orr_pkg --features bootstrap-authoring`.

```
orr_pkg bootstrap-user-game --engine /absolute/reviewed-engine --destination /absolute/new-game --name my-game
orr_pkg bootstrap-user-game --engine /absolute/preview-engine --destination /absolute/new-preview --name my-preview --profile multi-action-preview
```

This replaces Python for materializing the existing reviewed starter. It does not
run Python, Git, Cargo, a shell, the generated game, network requests or downloads.
It cannot choose a custom inventory, commit or executable. The original Python
tool remains for compatibility and differential tests. The final HTML packer has
a separate remaining Python dependency.

The closed `legacy` profile pins commit `27e53b356b25aa505995f3e8b322093948ef9db0`,
tree `59158bede83603d7cb272ce7a101b4178956c598`; `multi-action-preview` pins commit
`6f97ac85a80aabda26679b482d182c9365bff92e`, tree
`cd7cc2a03990a8778a99e60a82c3cfbb425c4d87`. Source files must match every embedded
Git blob, executable mode and reconstructed tree. SHA1 here identifies Git content;
it is not a new authentication or cryptographic provenance claim. Only top-level
`.git` and `target` are outside source admission. Untracked files, symlinks and
special files elsewhere are refused. Preview remains opt-in and not a stable
release or native acceptance certificate.

Generated Cargo/README/source/provenance bytes preserve the original contract:
engine dependencies point to that absolute immutable engine checkout; repository-only
native test harnesses are omitted; copied files are mode0644, directories0755 and
private project root0700 subject to umask. `provenance.json` records initial file
hashes and a null user commit. No Cargo.lock is generated. You still own and must
review, build, commit and validate the resulting project; materialization is not
an engine-source-free SDK, GUI test, delivered player or reproducible bundle proof.

Admission completes before a private same-parent stage is claimed. Output is
published using Linux `renameat2(NOREPLACE)` only: existing files/directories/links
are never overwritten and an unsupported operation has no fallback. Held parent
and stage identity, exact inventory and file bytes are rechecked before publication;
files and directories are synced before rename, parent afterwards. A post-rename
failure reports a published output with uncertain durability, not no output.

Unlike the old recursive failure cleanup, Rust deliberately **leaves an unpublished
stage on any post-creation error**, reporting its path for manual inspection. It
never recursively deletes foreign or replaced objects. Do not blindly retry or
remove a retained stage. This safer failure-policy difference is not byte parity
for error side effects. These checks assume trusted, quiescent directory ancestry,
not a sandbox against concurrent hostile filesystem changes.

Additional explicit bounds: paths4096 UTF-8 bytes/128 components, argv64KiB/16
arguments, engine scan10000 entries, files32MiB each, retained/generated starter
128 files/64MiB. Non-Unicode and ambiguous backslash/traversal paths are refused.
Generated names retain the original lowercase Cargo-safe64-character policy and
reserved names. Existing default features are unchanged; authoring is opt-in.

Focused differential verification uses both exact pinned source trees, all generated
bytes/modes and existing Python TOML/provenance consumers. It does not automatically
run generated Cargo projects or native window/source-hidden bundle acceptance.
