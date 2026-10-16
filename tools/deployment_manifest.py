#!/usr/bin/env python3
"""Create, validate, and export Orrery deployment manifests (stdlib only).

This tool describes launch configuration. Provenance fields are records supplied
by the release owner; they are not signatures or proof of a build relationship.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path
from typing import Any


FORMAT = "orr.deployment/1"
FRAME_FORMAT_VERSION = 2
MAX_U64 = (1 << 64) - 1
MASK64 = MAX_U64
_DECIMAL = re.compile(r"(?:0|[1-9][0-9]*)\Z")
_HEX = re.compile(r"0[xX][0-9a-fA-F]+\Z")
_SHA256 = re.compile(r"[0-9a-f]{64}\Z")
_TOP_FIELDS = {
    "format", "game", "game_code_identity", "game_code_id",
    "frame_format_version", "build_id", "provenance",
}


class ManifestError(ValueError):
    """Invalid or inconsistent deployment manifest."""


def _u64_text(value: Any, field: str, *, allow_hex: bool = False) -> int:
    if not isinstance(value, str):
        raise ManifestError(f"{field} must be a decimal u64 string")
    if _DECIMAL.fullmatch(value):
        number = int(value, 10)
    elif allow_hex and _HEX.fullmatch(value):
        number = int(value[2:], 16)
    else:
        suffix = " decimal or 0x-prefixed hexadecimal" if allow_hex else " decimal"
        raise ManifestError(f"{field} must be a canonical{suffix} u64 string")
    if number > MAX_U64:
        raise ManifestError(f"{field} is outside the u64 range")
    if number == 0:
        raise ManifestError(f"{field} must be nonzero")
    return number


def _mix64(value: int) -> int:
    value &= MASK64
    value ^= value >> 30
    value = (value * 0xBF58476D1CE4E5B9) & MASK64
    value ^= value >> 27
    value = (value * 0x94D049BB133111EB) & MASK64
    return (value ^ (value >> 31)) & MASK64


def frame_build_id(game_code_id: int, frame_format_version: int = FRAME_FORMAT_VERSION) -> int:
    """Mirror `orr_sim::frame_build_id` for the manifest's pinned frame format."""
    if not isinstance(game_code_id, int) or isinstance(game_code_id, bool) or not (1 <= game_code_id <= MAX_U64):
        raise ManifestError("game_code_id must be a nonzero u64")
    if frame_format_version != FRAME_FORMAT_VERSION:
        raise ManifestError(f"frame_format_version must be {FRAME_FORMAT_VERSION}")
    format_id = 0x4F52524600000000 | frame_format_version
    result = _mix64(game_code_id ^ _mix64(format_id))
    return result or 1


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _binary_provenance(items: list[str] | None) -> list[dict[str, str]]:
    records = []
    labels = set()
    for item in items or []:
        label, sep, file_name = item.partition("=")
        if not sep or not label.strip() or not file_name:
            raise ManifestError("--binary must be LABEL=PATH")
        label = label.strip()
        if label in labels:
            raise ManifestError(f"duplicate binary label: {label}")
        labels.add(label)
        path = Path(file_name)
        if not path.is_file():
            raise ManifestError(f"binary file does not exist: {path}")
        records.append({"label": label, "sha256": _sha256_file(path)})
    return records


def create_manifest(
    game: str,
    game_code_identity: str,
    game_code_id: str,
    source_revision: str | None = None,
    binaries: list[str] | None = None,
) -> dict[str, Any]:
    """Build a normalized manifest from release-owner supplied identity data."""
    if game not in ("arena", "physics"):
        raise ManifestError("game must be 'arena' or 'physics'")
    if not isinstance(game_code_identity, str) or not game_code_identity.strip():
        raise ManifestError("game_code_identity must be a nonempty string")
    if game_code_identity != game_code_identity.strip():
        raise ManifestError("game_code_identity must not have surrounding whitespace")
    raw_id = _u64_text(game_code_id, "game_code_id", allow_hex=True)
    if source_revision is not None and (
        not isinstance(source_revision, str)
        or not source_revision
        or source_revision != source_revision.strip()
    ):
        raise ManifestError("source_revision must be null or a nonempty trimmed string")
    build_id = frame_build_id(raw_id)
    manifest = {
        "format": FORMAT,
        "game": game,
        "game_code_identity": game_code_identity,
        "game_code_id": str(raw_id),
        "frame_format_version": FRAME_FORMAT_VERSION,
        "build_id": str(build_id),
        "provenance": {
            "source_revision": source_revision,
            "binaries": _binary_provenance(binaries),
        },
    }
    validate_manifest(manifest)
    return manifest


def validate_manifest(
    manifest: Any,
    *,
    expect_game: str | None = None,
    expect_game_code_identity: str | None = None,
) -> dict[str, Any]:
    """Validate strict v1 shape and the declared compatibility identity."""
    if not isinstance(manifest, dict):
        raise ManifestError("manifest root must be an object")
    unknown = set(manifest) - _TOP_FIELDS
    missing = _TOP_FIELDS - set(manifest)
    if unknown:
        raise ManifestError(f"unknown manifest field(s): {', '.join(sorted(unknown))}")
    if missing:
        raise ManifestError(f"missing manifest field(s): {', '.join(sorted(missing))}")
    if manifest["format"] != FORMAT:
        raise ManifestError(f"format must be {FORMAT!r}")
    game = manifest["game"]
    if game not in ("arena", "physics"):
        raise ManifestError("game must be 'arena' or 'physics'")
    identity = manifest["game_code_identity"]
    if not isinstance(identity, str) or not identity or identity != identity.strip():
        raise ManifestError("game_code_identity must be a nonempty trimmed string")
    version = manifest["frame_format_version"]
    if not isinstance(version, int) or isinstance(version, bool) or version != FRAME_FORMAT_VERSION:
        raise ManifestError(f"frame_format_version must be integer {FRAME_FORMAT_VERSION}")
    raw_id = _u64_text(manifest["game_code_id"], "game_code_id")
    build_id = _u64_text(manifest["build_id"], "build_id")
    expected_build_id = frame_build_id(raw_id, version)
    if build_id != expected_build_id:
        raise ManifestError("build_id does not match frame_build_id(game_code_id, frame_format_version)")
    provenance = manifest["provenance"]
    if not isinstance(provenance, dict) or set(provenance) != {"source_revision", "binaries"}:
        raise ManifestError("provenance must contain exactly source_revision and binaries")
    revision = provenance["source_revision"]
    if revision is not None and (not isinstance(revision, str) or not revision or revision != revision.strip()):
        raise ManifestError("provenance.source_revision must be null or a nonempty trimmed string")
    binaries = provenance["binaries"]
    if not isinstance(binaries, list):
        raise ManifestError("provenance.binaries must be an array")
    labels = set()
    for index, record in enumerate(binaries):
        if not isinstance(record, dict) or set(record) != {"label", "sha256"}:
            raise ManifestError(f"provenance.binaries[{index}] must contain exactly label and sha256")
        label, digest = record["label"], record["sha256"]
        if not isinstance(label, str) or not label or label != label.strip():
            raise ManifestError(f"provenance.binaries[{index}].label must be a nonempty trimmed string")
        if label in labels:
            raise ManifestError(f"duplicate binary label: {label}")
        labels.add(label)
        if not isinstance(digest, str) or not _SHA256.fullmatch(digest):
            raise ManifestError(f"provenance.binaries[{index}].sha256 must be lowercase SHA-256 hex")
    if expect_game is not None and game != expect_game:
        raise ManifestError(f"game identity mismatch: expected {expect_game!r}, manifest has {game!r}")
    if expect_game_code_identity is not None and identity != expect_game_code_identity:
        raise ManifestError(
            "game_code_identity mismatch: expected "
            f"{expect_game_code_identity!r}, manifest has {identity!r}"
        )
    return manifest


def _unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result = {}
    for key, value in pairs:
        if key in result:
            raise ManifestError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def load_manifest(path: str | Path, **expectations: Any) -> dict[str, Any]:
    try:
        with Path(path).open("r", encoding="utf-8") as stream:
            manifest = json.load(stream, object_pairs_hook=_unique_object)
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise ManifestError(f"cannot read manifest {path}: {exc}") from exc
    return validate_manifest(manifest, **expectations)


def _check_previous(manifest: dict[str, Any], previous_path: str | Path) -> None:
    previous = load_manifest(previous_path)
    if previous["game_code_id"] == manifest["game_code_id"] and (
        previous["game"] != manifest["game"]
        or previous["game_code_identity"] != manifest["game_code_identity"]
    ):
        raise ManifestError(
            "game_code_id was reused for a changed game or game_code_identity; "
            "release owner must assign a new game_code_id"
        )


def export_manifest(manifest: dict[str, Any]) -> dict[str, Any]:
    """Return JSON-compatible launch plans; this function never launches them."""
    validate_manifest(manifest)
    game = manifest["game"]
    browser_game = "arena" if game == "arena" else "phys"
    build_id = manifest["build_id"]
    host_server_args = ["--game", game, "--build-id", build_id]
    return {
        "format": FORMAT,
        "game": game,
        "game_code_id": manifest["game_code_id"],
        "frame_format_version": manifest["frame_format_version"],
        "build_id": build_id,
        "native_host": {"program": "orr_remote_host", "args": host_server_args.copy()},
        "relay": {"program": "orr_server", "args": host_server_args.copy()},
        "native_client": {
            "program": "arena" if game == "arena" else "physics",
            "args": ["--build-id", build_id],
        },
        "browser": {
            "opts": {"game": browser_game, "build_id": build_id},
            "query": {"game": browser_game, "build": build_id},
        },
    }


def _write_exclusive(path: str | Path, text: str) -> None:
    target = Path(path)
    try:
        with target.open("x", encoding="utf-8", newline="\n") as stream:
            stream.write(text)
    except FileExistsError as exc:
        raise ManifestError(f"refusing to overwrite existing output: {target}") from exc
    except OSError as exc:
        raise ManifestError(f"cannot write {target}: {exc}") from exc


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    create = commands.add_parser("create", help="create a new manifest without overwriting")
    create.add_argument("--game", required=True, choices=("arena", "physics"))
    create.add_argument("--game-code-identity", required=True)
    create.add_argument("--game-code-id", required=True, help="nonzero u64 as decimal or 0x hex text")
    create.add_argument("--source-revision")
    create.add_argument("--binary", action="append", metavar="LABEL=PATH", help="record a binary SHA-256")
    create.add_argument("--previous", help="reject reuse of the raw id across changed game identity")
    create.add_argument("--output", required=True)

    for command in ("validate", "export", "id"):
        sub = commands.add_parser(command, help=f"{command} a manifest")
        sub.add_argument("path")
        sub.add_argument("--expect-game", choices=("arena", "physics"))
        sub.add_argument("--expect-game-code-identity")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    try:
        if args.command == "create":
            manifest = create_manifest(
                args.game, args.game_code_identity, args.game_code_id,
                args.source_revision, args.binary,
            )
            if args.previous:
                _check_previous(manifest, args.previous)
            _write_exclusive(args.output, json.dumps(manifest, indent=2, sort_keys=True) + "\n")
            print(args.output)
        else:
            manifest = load_manifest(
                args.path,
                expect_game=args.expect_game,
                expect_game_code_identity=args.expect_game_code_identity,
            )
            if args.command == "validate":
                print("valid")
            elif args.command == "id":
                print(manifest["build_id"])
            else:
                print(json.dumps(export_manifest(manifest), indent=2, sort_keys=True))
        return 0
    except ManifestError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
