#!/usr/bin/env python3
"""Materialize the reviewed starter without running Cargo, Git, or game code (Linux)."""
from __future__ import annotations

import argparse
import ctypes
import errno
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import sys
import tomllib
import uuid

PINNED_COMMIT = "27e53b356b25aa505995f3e8b322093948ef9db0"
PINNED_TREE = "59158bede83603d7cb272ce7a101b4178956c598"
STARTER = "examples/static-game-starter/"
OMITTED = {"tests/standalone_x11.rs", "tests/support/standalone_x11.rs"}
MAX_FILE = 32 * 1024 * 1024
ACTION_PREVIEW_COMMIT = "6f97ac85a80aabda26679b482d182c9365bff92e"
ACTION_PREVIEW_TREE = "cd7cc2a03990a8778a99e60a82c3cfbb425c4d87"
PROFILES = ("legacy", "multi-action-preview")
ACTION_NATIVE_OMITTED = {
    "tests/multi_action_standalone_x11.rs", "tests/multi_action_editor_x11.rs",
    "tests/support/multi_action_fixture_player.rs", "tests/support/multi_action_fixture.rs",
}


def profile_contract(profile: str) -> tuple[str, str, set[str]]:
    # Selection is explicit and closed: inventory data cannot choose its own trust root.
    if profile == "legacy":
        return PINNED_COMMIT, PINNED_TREE, OMITTED
    if profile == "multi-action-preview":
        return ACTION_PREVIEW_COMMIT, ACTION_PREVIEW_TREE, OMITTED | ACTION_NATIVE_OMITTED
    raise ValueError("unknown bootstrap profile")



def git_hash(kind: str, data: bytes) -> str:
    return hashlib.sha1(kind.encode() + b" " + str(len(data)).encode() + b"\0" + data).hexdigest()


def inventory_tree(entries: list[dict]) -> str:
    root: dict = {}
    seen = set()
    for item in entries:
        path = item["path"]
        parts = PurePosixPath(path).parts
        if (not parts or path != "/".join(parts) or any(p in (".", "..", ".git", "target") for p in parts)
                or path.startswith("/") or "\\" in path or "\0" in path or path in seen):
            raise ValueError("invalid or duplicate inventory path")
        if item["mode"] not in ("100644", "100755") or not re.fullmatch(r"[0-9a-f]{40}", item["blob"]):
            raise ValueError("unsupported inventory mode or blob")
        seen.add(path)
        node = root
        for part in parts[:-1]:
            node = node.setdefault(part, {})
            if not isinstance(node, dict):
                raise ValueError("inventory path collision")
        if parts[-1] in node:
            raise ValueError("inventory path collision")
        node[parts[-1]] = (item["mode"], item["blob"])

    def encode(node):
        payload = b""
        for name, value in sorted(node.items(), key=lambda kv: (kv[0] + ("/" if isinstance(kv[1], dict) else "")).encode()):
            mode, digest = ("40000", encode(value)) if isinstance(value, dict) else value
            payload += mode.encode() + b" " + name.encode() + b"\0" + bytes.fromhex(digest)
        return git_hash("tree", payload)

    return encode(root)


def absolute_path(value: str) -> Path:
    if ".." in value.split("/") or "\0" in value or "\\" in value:
        raise ValueError("traversal or ambiguous path rejected")
    return Path(os.path.abspath(value))


def open_directory(path: Path) -> int:
    """Walk every ancestor without following symlinks; caller owns returned fd."""
    fd = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
    try:
        for part in path.parts[1:]:
            next_fd = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            os.close(fd)
            fd = next_fd
        return fd
    except BaseException:
        os.close(fd)
        raise


def read_file(root_fd: int, path: str) -> tuple[bytes, int]:
    fd = os.dup(root_fd)
    try:
        parts = path.split("/")
        for part in parts[:-1]:
            next_fd = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            os.close(fd)
            fd = next_fd
        source = os.open(parts[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=fd)
        try:
            info = os.fstat(source)
            if not stat.S_ISREG(info.st_mode) or info.st_size > MAX_FILE:
                raise ValueError(f"not a bounded regular file: {path}")
            with os.fdopen(os.dup(source), "rb") as stream:
                data = stream.read(MAX_FILE + 1)
            if len(data) > MAX_FILE:
                raise ValueError(f"file too large: {path}")
            return data, info.st_mode
        finally:
            os.close(source)
    finally:
        os.close(fd)


def validate_engine(engine: Path, inventory: dict, *, profile: str = "legacy") -> dict[str, bytes]:
    commit, tree, _ = profile_contract(profile)
    if (inventory.get("schema") != 1 or inventory.get("commit") != commit
            or inventory.get("tree") != tree or inventory_tree(inventory["files"]) != tree):
        raise ValueError("inventory is not the pinned reviewed Git tree")
    expected = {item["path"] for item in inventory["files"]}
    root_fd = open_directory(engine)
    snapshots = {}
    try:
        # Reject untracked code/configuration that could affect Cargo discovery. Only
        # top-level Git metadata and build outputs are outside this source contract.
        def scan(fd, prefix=""):
            for name in os.listdir(fd):
                if not prefix and name in (".git", "target"):
                    continue
                path = prefix + name
                info = os.stat(name, dir_fd=fd, follow_symlinks=False)
                if stat.S_ISDIR(info.st_mode):
                    child = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
                    try:
                        scan(child, path + "/")
                    finally:
                        os.close(child)
                elif not stat.S_ISREG(info.st_mode) or path not in expected:
                    raise ValueError(f"untracked or nonregular engine input: {path}")
        scan(root_fd)
        for item in inventory["files"]:
            data, mode = read_file(root_fd, item["path"])
            if git_hash("blob", data) != item["blob"] or bool(mode & 0o111) != (item["mode"] == "100755"):
                raise ValueError(f"engine input differs from pinned tree: {item['path']}")
            if item["path"].startswith(STARTER):
                snapshots[item["path"][len(STARTER):]] = data
        return snapshots
    finally:
        os.close(root_fd)


def project_manifest(original: bytes, engine: Path, name: str, *, profile: str = "legacy") -> bytes:
    profile_contract(profile)
    text = original.decode()
    text = text.replace('name = "orr_static_starter"', f'name = {json.dumps(name)}', 1)
    text = text.replace('version.workspace = true', 'version = "0.1.0"', 1)
    text = text.replace('edition.workspace = true', 'edition = "2021"', 1)
    text = text.replace('license.workspace = true', 'license = "MIT OR Apache-2.0"', 1)
    text = text.replace('[lints]\nworkspace = true', '[lints.clippy]\nfloat_arithmetic = "warn"', 1)
    text = re.sub(r'path = "../../crates/([a-z0-9_]+)"',
                  lambda match: 'path = ' + json.dumps(str(engine / "crates" / match[1])), text)
    text = re.sub(r'\nstandalone-native-acceptance = \[[^\n]+\]\n', '\n', text)
    text = text.replace('[[test]]\nname = "standalone_x11"\npath = "tests/standalone_x11.rs"\nrequired-features = ["standalone-native-acceptance"]\n', '')
    if profile == "multi-action-preview":
        # These repository-only harnesses require the omitted source-fence support.
        for feature in ("multi-action-standalone-acceptance", "multi-action-editor-acceptance"):
            text = re.sub(r"(?m)^" + re.escape(feature) + r" = \[[^\n]+\]\n", "", text)
        text = re.sub(r'(?ms)^\[\[(?:bin|test)\]\]\nname = "(?:multi-action-fixture-player|multi_action_standalone_x11|multi_action_editor_x11)"\n.*?(?=^\[|\Z)', "", text)
        parsed_preview = tomllib.loads(text)
        if (parsed_preview["features"].get("static-game-actions") != ["dep:orr_input", "orr_editor?/static-game-actions"]
                or "static-game-actions" in parsed_preview["features"].get("default", [])):
            raise ValueError("preview optional action feature contract changed")
    text += '\n[lib]\nname = "orr_static_starter"\n\n[workspace]\nresolver = "2"\n\n[profile.release]\ndebug = 1\ncodegen-units = 1\nlto = "thin"\n'
    parsed = tomllib.loads(text)
    if any(isinstance(v, dict) and v.get("workspace") for v in parsed["package"].values()):
        raise ValueError("remaining package workspace inheritance")
    return text.encode()


def project_readme(name: str) -> bytes:
    return f'''# {name}: independent authoring workspace

This is the reviewed two-game starter, not a new runtime. You own Cargo.toml,
src/, scenes/, tests/, and the Cargo.lock you generate. The library name remains
orr_static_starter so copied Rust sources are unchanged. Engine dependencies use
absolute paths to the separately pinned local engine checkout in provenance.json.
Keep that checkout immutable and at the same location. This folder alone is NOT
an engine-source-free authoring SDK or a delivered player bundle.

From THIS folder, with a supported installed Rust toolchain and cached dependencies:

    cargo generate-lockfile --offline
    cargo test --offline --locked --no-default-features --features standalone,bundle --lib --test standalone_contract --test bundle
    cargo test --offline --locked --test acceptance
    cargo run --offline --locked --bin editor -- delivery scenes/delivery.scene.yaml
    cargo run --offline --locked --bin player -- delivery scenes/delivery.scene.yaml

The bootstrap never invokes Cargo. Lock generation can fail if dependencies are
not cached; there is no claim of offline build readiness until that separate gate
succeeds. Review and commit your generated Cargo.lock with your project sources.
The template's license is MIT OR Apache-2.0; review your project's licensing.

For native builds, first commit your user project (including lock and provenance)
to your own Git repository, then use that clean user commit's lowercase 40-hex
SHA as ORR_STATIC_STANDALONE_BUILD_SHA during compilation. Do NOT use the engine
SHA as a substitute for user-code provenance. Use the same user SHA for bundle
--source-sha, and explicitly review the runtime binary/hash before using the
existing bundle tool's --trusted-runtime flag. Nothing here trusts or executes a
runtime. After reviewing and committing the user project, the explicit Linux
native build steps (run from this folder, not the engine) are:

    test -z "$(git status --porcelain)" || exit 1
    USER_SOURCE_SHA="$(git rev-parse --verify HEAD)"
    export ORR_STATIC_STANDALONE_BUILD_SHA="$USER_SOURCE_SHA"
    cargo build --offline --locked --release --no-default-features --features standalone,bundle --bin delivery-player --bin switchboard-player --bin bundle
    ./target/release/delivery-player --scene "$PWD/scenes/delivery.scene.yaml"
    ./target/release/switchboard-player --scene "$PWD/scenes/switchboard.scene.yaml"

For a Delivery bundle, first review the built runtime before executing the next
commands, which explicitly trust it. The staging copy avoids Cargo hard links;
replace the output path with your own NEW absolute bundle destination:

    RUNTIME_STAGE="$(mktemp -d)"
    cp -- ./target/release/delivery-player "$RUNTIME_STAGE/delivery-player"
    RUNTIME_SHA256="$(sha256sum "$RUNTIME_STAGE/delivery-player" | cut -d ' ' -f 1)"
    ./target/release/bundle delivery --runtime "$RUNTIME_STAGE/delivery-player" --runtime-sha256 "$RUNTIME_SHA256" --scene "$PWD/scenes/delivery.scene.yaml" --output /absolute/new-delivery-bundle --source-sha "$USER_SOURCE_SHA" --trusted-runtime
    /absolute/new-delivery-bundle/run-game

These commands assume the default target directory, an installed supported
Linux x86-64 toolchain, and the documented display/graphics dependencies. If you
set CARGO_TARGET_DIR, adjust binary paths. Both native players start paused;
Enter starts/resumes, Escape or focus loss pauses. Actual window/source-hidden
bundle acceptance is still required. UPSTREAM_README.md is historical context:
its -p orr_static_starter selector is WRONG for this renamed package; omit -p
inside this workspace or use -p {name}, and change repo-root scene paths to scenes/.

The engine's native standalone_x11 test and its support module are deliberately
omitted: that source-fence harness assumes the engine repository layout. Native
window/trace tests are retained but require their documented environment and
separate renderer validation. Portable tests and successful bootstrap creation do
NOT prove native GUI behavior, source-hidden delivery, or bundle reproducibility.
Build and validate actual delivered bundles separately using the existing player
and bundle implementation. No engine runtime or registry source was changed.

provenance.json records the engine pin and initial generated-file SHA256 values;
it is a creation receipt, not ongoing attestation after you edit files. The old
engine README is preserved verbatim as UPSTREAM_README.md, not as runnable
instructions for this relocated workspace.
'''.encode()


def generated_files(snapshot: dict[str, bytes], engine: Path, name: str, *, profile: str = "legacy") -> dict[str, bytes]:
    commit, tree, omitted = profile_contract(profile)
    files = {p: data for p, data in snapshot.items() if p not in omitted and p not in ("Cargo.toml", "README.md")}
    files["Cargo.toml"] = project_manifest(snapshot["Cargo.toml"], engine, name, profile=profile)
    files["UPSTREAM_README.md"] = snapshot["README.md"]
    files["README.md"] = project_readme(name)
    if profile == "multi-action-preview":
        if "tests/multi_action_contract.rs" not in files:
            raise ValueError("preview portable action contract missing")
        files["README.md"] += preview_readme()
    files[".gitignore"] = b"/target/\n"
    receipt = {"schema": 1, "engine_commit": commit, "engine_tree": tree,
               "engine_path": str(engine), "project_name": name, "user_source_commit": None,
               "lockfile": "not generated; owned by user project", "omitted_engine_only_tests": sorted(omitted),
               "files_sha256": {p: hashlib.sha256(data).hexdigest() for p, data in sorted(files.items())}}
    if profile == "multi-action-preview":
        receipt.update({"template_profile": profile, "stability": "preview",
                        "hosted_native_acceptance": "unverified",
                        "action_feature": "static-game-actions", "action_feature_default": False})
    files["provenance.json"] = (json.dumps(receipt, indent=2) + "\n").encode()
    return files


def preview_readme() -> bytes:
    return b"""
## Explicit multi-action preview (not a stable or recommended release)

This profile uses a different, exact reviewed engine commit recorded in
provenance.json. It does not upgrade an existing project or the legacy profile.
Hosted native CI and actual generated-project action execution are UNVERIFIED.
No creation receipt certifies runtime, hardware, source-hidden or merge acceptance.

The static-game-actions feature is opt-in, never a default. The included
portable tests/multi_action_contract.rs demonstrates a typed four-action mapper
and exercises admission, neutral input, and lifecycle handling in the standalone
controller. It is a test fixture, not a third playable game. From this workspace:

    cargo test --offline --locked --no-default-features --features standalone,static-game-actions --test multi_action_contract
    cargo test --offline --locked --features static-game-actions --test acceptance

These are commands to run, not results already obtained for your project.
Implement your game's StandaloneGame action_profile and held_actions_input,
and its editor registration with StaticGameRegistry::with_action_profiles and
StaticActionDescriptor.
The original Delivery/Switchboard remain legacy one-action samples. Enabling
this feature does not automatically redesign those games' controls.

Repository-only multi-action X11 harnesses and their private player/support
fixtures are omitted along with the legacy source-fence test. Native validation
of a generated user game requires a separately provenance-bound harness; do not
use the engine's native receipts as proof for your modified user code.
"""


def rename_noreplace(parent_fd: int, source: str, destination: str) -> None:
    libc = ctypes.CDLL(None, use_errno=True)
    rename = getattr(libc, "renameat2", None)
    if rename is None:
        raise OSError(errno.ENOSYS, "atomic no-replace rename unavailable")
    rename.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
    rename.restype = ctypes.c_int
    if rename(parent_fd, os.fsencode(source), parent_fd, os.fsencode(destination), 1) != 0:
        code = ctypes.get_errno()
        raise OSError(code, os.strerror(code), destination)


def bootstrap(engine: Path, destination: Path, name: str, inventory: dict, *, profile: str = "legacy") -> dict:
    _, tree, _ = profile_contract(profile)
    if sys.platform != "linux":
        raise ValueError("Linux with renameat2 is required; no unsafe publication fallback")
    if not re.fullmatch(r"[a-z][a-z0-9_-]{0,63}", name) or name in ("test", "build", "deps", "examples"):
        raise ValueError("project name must be a lowercase Cargo-safe name, at most 64 characters")
    engine, destination = absolute_path(str(engine)), absolute_path(str(destination))
    if destination == engine or engine in destination.parents:
        raise ValueError("destination must be outside the engine checkout")
    parent_fd = open_directory(destination.parent)
    stage = ".orr-bootstrap-" + uuid.uuid4().hex
    created = False
    try:
        try:
            os.stat(destination.name, dir_fd=parent_fd, follow_symlinks=False)
        except FileNotFoundError:
            pass
        else:
            raise ValueError("destination already exists; no files were changed")
        snapshot = validate_engine(engine, inventory, profile=profile)
        files = generated_files(snapshot, engine, name, profile=profile)
        os.mkdir(stage, 0o700, dir_fd=parent_fd)
        created = True
        stage_fd = os.open(stage, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent_fd)
        try:
            for path, data in sorted(files.items()):
                fd = os.dup(stage_fd)
                try:
                    parts = path.split("/")
                    for part in parts[:-1]:
                        try:
                            os.mkdir(part, 0o755, dir_fd=fd)
                        except FileExistsError:
                            pass
                        child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
                        os.close(fd)
                        fd = child
                    output = os.open(parts[-1], os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o644, dir_fd=fd)
                    with os.fdopen(output, "wb") as stream:
                        stream.write(data)
                finally:
                    os.close(fd)
        finally:
            os.close(stage_fd)
        rename_noreplace(parent_fd, stage, destination.name)
        created = False
        return {"destination": str(destination), "engine_tree": tree, "files": len(files), "cargo_ran": False}
    finally:
        if created:
            shutil.rmtree(stage, dir_fd=parent_fd)
        os.close(parent_fd)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", required=True, type=Path)
    parser.add_argument("--destination", required=True, type=Path, help="new directory; parent must already exist")
    parser.add_argument("--name", required=True)
    parser.add_argument("--profile", choices=PROFILES, default="legacy",
                        help="explicit template pin; multi-action-preview has no native acceptance")
    args = parser.parse_args()
    try:
        inventory_name = "user_game_bootstrap_source.json" if args.profile == "legacy" else "user_game_action_preview_source.json"
        inventory = json.loads(Path(__file__).with_name(inventory_name).read_text())
        print(json.dumps(bootstrap(args.engine, args.destination, args.name, inventory, profile=args.profile)))
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"bootstrap refused: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
