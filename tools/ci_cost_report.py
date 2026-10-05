#!/usr/bin/env python3
"""Compare declared CI timings, or explicitly collect bounded local Cargo wall time."""

from __future__ import annotations

import argparse
import copy
from datetime import datetime, timezone
from decimal import Decimal, InvalidOperation, localcontext
import hashlib
import html
import json
import math
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import threading
import time
import uuid

INPUT_SCHEMA = "orr.ci-cost-input/1"
REPORT_SCHEMA = "orr.ci-cost-report/1"
MAX_INPUT_BYTES = 2 * 1024 * 1024
METRICS = ("compile", "runtime", "total")
SCOPES = {
    "compile": {"cargo-build-wall"},
    "runtime": {"test-execution-wall"},
    "total": {"command-wall", "job-wall"},
}
STATUSES = {"success", "failure", "cancelled", "timed_out", "unknown"}
CONDITION_KEYS = {
    "": {"platform", "runner", "cache", "toolchain", "profile", "target", "features", "command"},
    "platform": {"os", "arch", "image"},
    "runner": {"class", "hardware", "isolation"},
    "cache": {"state", "key"},
}


def _unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def loads_document(text):
    if not isinstance(text, str) or len(text.encode("utf-8")) > MAX_INPUT_BYTES:
        raise ValueError("input must be UTF-8 JSON of at most 2 MiB")
    try:
        return json.loads(
            text, object_pairs_hook=_unique_object, parse_float=Decimal,
            parse_constant=lambda token: (_ for _ in ()).throw(
                ValueError(f"non-finite JSON number: {token}")),
        )
    except (json.JSONDecodeError, RecursionError, InvalidOperation) as exc:
        raise ValueError(f"invalid JSON: {exc}") from exc


def _text(value):
    return (isinstance(value, str) and bool(value.strip())
            and value.strip().casefold() not in {"unknown", "unavailable", "n/a"})


def _decimal_text(value):
    result = format(value, "f")
    if "." in result:
        result = result.rstrip("0").rstrip(".")
    return result if result not in {"", "-0"} else "0"


def _seconds(value):
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, (str, int, float, Decimal)):
        raise ValueError("seconds must be a finite nonnegative decimal or null")
    if isinstance(value, float) and not math.isfinite(value):
        raise ValueError("seconds must be finite")
    if len(str(value)) > 40:
        raise ValueError("seconds exceeds decimal precision limit")
    try:
        result = Decimal(str(value))
    except InvalidOperation as exc:
        raise ValueError("invalid decimal seconds") from exc
    if not result.is_finite() or result < 0 or result > Decimal("1000000000000"):
        raise ValueError("seconds outside finite range 0..1e12")
    if result.as_tuple().exponent < -9:
        raise ValueError("seconds supports at most nanosecond precision")
    return result


def _object(value, path, errors):
    if value is None:
        return {}
    if not isinstance(value, dict):
        errors.append(f"{path}: expected object or null")
        return {}
    return value


def _optional_text(value, path, errors):
    if value is not None and not isinstance(value, str):
        errors.append(f"{path}: expected string or null")


def _condition_keys(value, group, errors):
    path = "conditions" + ("." + group if group else "")
    for key in value:
        if key not in CONDITION_KEYS[group]:
            errors.append(f"{path}.{key}: unsupported condition field; not validated")


def _validate_record(record):
    errors = []
    provenance = _object(record.get("provenance"), "provenance", errors)
    for key in ("source_sha", "checkout_sha", "source_tree_sha", "checkout_tree_sha"):
        value = provenance.get(key)
        if value is not None and (not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{40}", value)):
            errors.append(f"provenance.{key}: expected exact lowercase SHA or null")
    for key in ("run_id", "run_attempt", "job_id"):
        value = provenance.get(key)
        if value is not None and (type(value) is not int or value <= 0):
            errors.append(f"provenance.{key}: expected positive integer or null")
    artifact = _object(provenance.get("artifact"), "provenance.artifact", errors)
    for key in ("name", "locator"):
        _optional_text(artifact.get(key), f"provenance.artifact.{key}", errors)
    digest = artifact.get("sha256")
    if digest is not None and (not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest)):
        errors.append("provenance.artifact.sha256: expected exact lowercase SHA256 or null")
    status = record.get("status", "unknown")
    if not isinstance(status, str) or status not in STATUSES:
        errors.append("status: invalid outcome")
    code = record.get("exit_code")
    if code is not None and type(code) is not int:
        errors.append("exit_code: expected integer or null")
    conditions = _object(record.get("conditions"), "conditions", errors)
    _condition_keys(conditions, "", errors)
    for key, fields in (("platform", ("os", "arch", "image")),
                        ("runner", ("class", "hardware", "isolation")),
                        ("cache", ("state", "key"))):
        nested = _object(conditions.get(key), f"conditions.{key}", errors)
        _condition_keys(nested, key, errors)
        for field in fields:
            _optional_text(nested.get(field), f"conditions.{key}.{field}", errors)
    for key in ("toolchain", "profile", "target"):
        _optional_text(conditions.get(key), f"conditions.{key}", errors)
    for key in ("features", "command"):
        value = conditions.get(key)
        if value is not None and (not isinstance(value, list) or len(value) > 128
                                  or any(not _text(item) for item in value)):
            errors.append(f"conditions.{key}: expected string array or null")
    measures = _object(record.get("measurements"), "measurements", errors)
    for key in METRICS:
        measure = _object(measures.get(key), f"measurements.{key}", errors)
        for field in ("scope", "source"):
            _optional_text(measure.get(field), f"measurements.{key}.{field}", errors)
        try:
            _seconds(measure.get("seconds"))
        except ValueError as exc:
            errors.append(f"measurements.{key}: {exc}")
    return errors


def _get(record, path):
    value = record
    for key in path.split("."):
        if not isinstance(value, dict):
            return None
        value = value.get(key)
    return value


PROVENANCE_FIELDS = (
    "source_sha", "checkout_sha", "source_tree_sha", "checkout_tree_sha",
    "run_id", "run_attempt", "job_id", "artifact.name", "artifact.locator", "artifact.sha256",
)
CONDITION_FIELDS = (
    "platform.os", "platform.arch", "platform.image", "runner.class", "runner.hardware",
    "runner.isolation", "toolchain", "profile", "target", "features", "command",
    "cache.state", "cache.key",
)


def _common_reasons(base, current, errors):
    reasons = []
    for label, record in (("baseline", base), ("current", current)):
        if errors[record["id"]]:
            reasons.append(f"{label}: invalid_record")
        if record.get("status") != "success" or type(record.get("exit_code")) is not int or record.get("exit_code") != 0:
            reasons.append(f"{label}: not_successful_exit0")
        for field in PROVENANCE_FIELDS:
            value = _get(record, "provenance." + field)
            known = (type(value) is int and value > 0) if field in {"run_id", "run_attempt", "job_id"} else _text(value)
            if not known:
                reasons.append(f"{label}: unknown_provenance.{field}")
        if _get(record, "provenance.source_tree_sha") != _get(record, "provenance.checkout_tree_sha"):
            reasons.append(f"{label}: source_checkout_tree_mismatch")
        if _get(record, "conditions.runner.isolation") != "exclusive":
            reasons.append(f"{label}: runner_not_exclusive")
        cache_state = _get(record, "conditions.cache.state")
        if not isinstance(cache_state, str) or cache_state not in {"cold", "warm"}:
            reasons.append(f"{label}: unknown_cache_state")
    for field in CONDITION_FIELDS:
        left, right = _get(base, "conditions." + field), _get(current, "conditions." + field)
        def known(value):
            if field == "features":
                return isinstance(value, list) and all(_text(item) for item in value)
            if field == "command":
                return isinstance(value, list) and bool(value) and all(_text(item) for item in value)
            return _text(value)
        if not known(left) or not known(right):
            reasons.append(f"unknown_condition.{field}")
        elif left != right:
            reasons.append(f"condition_mismatch.{field}")
    identity = ("run_id", "run_attempt", "job_id")
    if all(_get(base, "provenance." + key) == _get(current, "provenance." + key) for key in identity):
        reasons.append("same_capture")
    return list(dict.fromkeys(reasons))


def _compare(base, current, errors):
    common = _common_reasons(base, current, errors)
    metrics = {}
    for name in METRICS:
        reasons = list(common)
        values = []
        for label, record in (("baseline", base), ("current", current)):
            measure = _get(record, "measurements." + name)
            measure = measure if isinstance(measure, dict) else {}
            try:
                value = _seconds(measure.get("seconds"))
            except ValueError:
                value = None
            values.append(value)
            if value is None:
                reasons.append(f"{label}: unknown_seconds")
            elif value == 0:
                reasons.append(f"{label}: zero_seconds_no_ratio")
            if not _text(measure.get("source")):
                reasons.append(f"{label}: unknown_measurement_source")
            scope = measure.get("scope")
            if not isinstance(scope, str) or scope not in SCOPES[name]:
                reasons.append(f"{label}: unknown_or_mixed_scope")
        if _get(base, f"measurements.{name}.scope") != _get(current, f"measurements.{name}.scope"):
            reasons.append("measurement_scope_mismatch")
        metric = {
            "baseline_seconds": _decimal_text(values[0]) if values[0] is not None else None,
            "current_seconds": _decimal_text(values[1]) if values[1] is not None else None,
            "comparable": not reasons, "reasons": reasons,
        }
        if not reasons:
            with localcontext() as context:
                context.prec = 28
                change = values[1] - values[0]
                metric.update(ratio_baseline_over_current=_decimal_text(values[0] / values[1]),
                              change_seconds=_decimal_text(change),
                              direction="slower" if change > 0 else "faster" if change < 0 else "unchanged")
        metrics[name] = metric
    comparable_count = sum(metric["comparable"] for metric in metrics.values())
    return {
        "baseline": base["id"], "current": current["id"],
        "status": "comparable" if comparable_count == 3 else "partially_comparable" if comparable_count else "incomparable",
        "common_reasons": common, "metrics": metrics,
        "interpretation": "paired observations; no causal or statistical speedup claim",
    }


def build_report(document):
    if not isinstance(document, dict) or document.get("schema") != INPUT_SCHEMA:
        raise ValueError(f"expected schema {INPUT_SCHEMA}")
    if set(document) - {"schema", "records", "comparisons"}:
        raise ValueError("unknown top-level fields")
    records, pairs = document.get("records"), document.get("comparisons", [])
    if not isinstance(records, list) or not 1 <= len(records) <= 1000:
        raise ValueError("records must contain 1..1000 entries")
    if not isinstance(pairs, list) or len(pairs) > 1000:
        raise ValueError("comparisons must be an array of at most 1000 pairs")
    by_id, errors = {}, {}
    for record in records:
        if not isinstance(record, dict) or not _text(record.get("id")):
            raise ValueError("each record needs a nonempty id")
        if record["id"] in by_id:
            raise ValueError(f"duplicate record id: {record['id']}")
        by_id[record["id"]] = record
        errors[record["id"]] = _validate_record(record)
    comparisons = []
    for pair in pairs:
        if not isinstance(pair, dict) or set(pair) != {"baseline", "current"}:
            raise ValueError("comparison needs exactly baseline and current ids")
        if any(not isinstance(pair[key], str) or pair[key] not in by_id for key in pair):
            raise ValueError("comparison references an unknown record")
        comparisons.append(_compare(by_id[pair["baseline"]], by_id[pair["current"]], errors))
    return {
        "schema": REPORT_SCHEMA,
        "provenance_verification": "caller-declared; artifacts are not fetched or attested",
        "records": [{"input": copy.deepcopy(record), "validation_errors": errors[record["id"]]} for record in records],
        "comparisons": comparisons,
    }


def _markdown_text(value):
    text = html.escape(" ".join(str(value).splitlines()), quote=False)
    return re.sub(r"([\\`*_{}\[\]()#+.!|>\-])", r"\\\1", text)


def render_markdown(report):
    lines = ["CI timing observations", "", "Provenance is caller-declared; this report is not a benchmark or an attestation.", ""]
    for pair in report["comparisons"]:
        lines.extend([f"{_markdown_text(pair['baseline'])} → {_markdown_text(pair['current'])}: {pair['status']}", "",
                      "| Metric | Baseline seconds | Current seconds | Baseline/current | Result |",
                      "| --- | --- | --- | --- | --- |"])
        for name, metric in pair["metrics"].items():
            reason = metric.get("direction") or "; ".join(metric["reasons"])
            cells = [name, metric["baseline_seconds"], metric["current_seconds"], metric.get("ratio_baseline_over_current"), reason]
            lines.append("| " + " | ".join(_markdown_text(cell) if cell is not None else "unknown" for cell in cells) + " |")
        lines.append("")
    lines.append("Per-run inputs and validation details are retained in the JSON report.")
    return "\n".join(lines) + "\n"


LOCAL_PLAN_SCHEMA = "orr.ci-cost-local-plan/1"
LOCAL_REPORT_SCHEMA = "orr.ci-cost-local-report/1"
LOCAL_RESOURCE_SCHEMA = "orr.ci-cost-independent-resource/1"
LOCAL_ADMISSION_SCHEMA = "orr.ci-cost-local-admission/1"
LOCAL_COMMANDS = {
    "native": ["cargo", "test", "--workspace", "--release", "--timings",
               "--exclude", "orr_sample", "--exclude", "orr_editor",
               "--exclude", "orr_web_gpu"],
    "sample": ["cargo", "test", "-p", "orr_sample", "-p", "orr_view", "-p",
               "orr_bridge", "-p", "orr_rhi", "-p", "orr_render", "-p",
               "orr_editor", "-p", "orr_web_gpu", "--release", "--timings"],
}
# Only these values are saved. PATH and the complete inherited environment are
# deliberately not dumped; credentials are neither accepted nor recorded.
LOCAL_ENV_KEYS = {
    "CARGO_HOME", "RUSTUP_HOME", "RUSTUP_TOOLCHAIN", "CARGO_TARGET_DIR",
    "CARGO_BUILD_JOBS", "CARGO_BUILD_TARGET", "CARGO_INCREMENTAL",
    "CARGO_NET_OFFLINE", "RUSTC", "RUSTDOC", "RUSTFLAGS", "RUSTDOCFLAGS",
    "CARGO_ENCODED_RUSTFLAGS", "CARGO_ENCODED_RUSTDOCFLAGS",
    "CARGO_PROFILE_RELEASE_OPT_LEVEL", "CARGO_PROFILE_RELEASE_DEBUG",
    "CARGO_PROFILE_RELEASE_LTO", "CARGO_PROFILE_RELEASE_CODEGEN_UNITS",
    "CARGO_PROFILE_RELEASE_INCREMENTAL", "ORR_REQUIRE_C_COMPILER",
    "ORR_REQUIRE_GPU", "ORR_REQUIRE_NATIVE_EDITOR",
}


def _local_fields(value, keys, path):
    if not isinstance(value, dict) or set(value) != set(keys):
        raise ValueError(f"{path}: needs exactly {', '.join(sorted(keys))}")


def _local_text(value, path):
    if not _text(value) or len(value) > 4096 or "\x00" in value:
        raise ValueError(f"{path}: expected nonempty bounded text")


def _local_path(value, path):
    _local_text(value, path)
    if not Path(value).is_absolute():
        raise ValueError(f"{path}: expected absolute path")


def _local_digest(value, width, path):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{" + str(width) + "}", value):
        raise ValueError(f"{path}: expected exact lowercase digest")


def _local_receipt(value, path):
    _local_fields(value, {"path", "sha256"}, path)
    _local_path(value["path"], path + ".path")
    _local_digest(value["sha256"], 64, path + ".sha256")


def validate_local_plan(document):
    """Validate declarations only: no files, programs or workloads are opened."""
    _local_fields(document, {"schema", "plan_id", "once_dir", "baseline", "current",
                            "runner", "coordination", "watchdog", "captures"}, "plan")
    if document["schema"] != LOCAL_PLAN_SCHEMA:
        raise ValueError(f"expected schema {LOCAL_PLAN_SCHEMA}")
    if not isinstance(document["plan_id"], str) or not re.fullmatch(r"[A-Za-z0-9_-]{1,64}", document["plan_id"]):
        raise ValueError("plan_id: expected 1..64 safe identifier characters")
    _local_path(document["once_dir"], "once_dir")
    for side in ("baseline", "current"):
        _local_fields(document[side], {"sha", "tree_sha"}, side)
        for key in ("sha", "tree_sha"):
            _local_digest(document[side][key], 40, side + "." + key)
    if document["baseline"]["sha"] == document["current"]["sha"]:
        raise ValueError("baseline and current must identify distinct exact commits")
    runner = document["runner"]
    _local_fields(runner, {"os", "arch", "class", "hardware", "isolation", "toolchain"}, "runner")
    for key in ("os", "arch", "class", "hardware", "isolation"):
        _local_text(runner[key], "runner." + key)
    if runner["isolation"] not in {"exclusive", "coordinated", "shared", "unverified"}:
        raise ValueError("runner.isolation: expected exclusive, coordinated, shared or unverified")
    toolchain = runner["toolchain"]
    _local_fields(toolchain, {name + suffix for name in ("cargo", "rustc", "rustdoc")
                             for suffix in ("_path", "_version")}, "runner.toolchain")
    for name in ("cargo", "rustc", "rustdoc"):
        _local_path(toolchain[name + "_path"], "runner.toolchain." + name + "_path")
        _local_text(toolchain[name + "_version"], "runner.toolchain." + name + "_version")
    coordination = document["coordination"]
    independent = isinstance(coordination, dict) and "mode" in coordination
    if independent:
        _local_fields(coordination, {"mode", "lane_receipt", "resource_receipt",
                                     "n6_receipt", "n6_completed"}, "coordination")
        if (coordination["mode"] != "independent-linux" or coordination["n6_completed"] is not False
                or coordination["n6_receipt"] is not None):
            raise ValueError("independent-linux requires literal n6_completed=false and n6_receipt=null")
        if (runner["os"] != "Linux" or runner["class"] != "coordinated-local"
                or runner["isolation"] not in {"coordinated", "exclusive"}):
            raise ValueError("independent-linux requires a coordinated local Linux runner")
        receipt_keys = ("lane_receipt", "resource_receipt")
    else:
        _local_fields(coordination, {"lane_receipt", "n6_receipt", "n6_completed"}, "coordination")
        if coordination["n6_completed"] is not True:
            raise ValueError("coordination.n6_completed must be true before collection")
        receipt_keys = ("lane_receipt", "n6_receipt")
    for key in receipt_keys:
        _local_receipt(coordination[key], "coordination." + key)
    watchdog = document["watchdog"]
    _local_fields(watchdog, {"seconds", "max_log_bytes", "cleanup"}, "watchdog")
    if type(watchdog["seconds"]) is not int or not 1 <= watchdog["seconds"] <= 86400:
        raise ValueError("watchdog.seconds: expected integer 1..86400")
    if type(watchdog["max_log_bytes"]) is not int or not 1 <= watchdog["max_log_bytes"] <= 64 * 1024 * 1024:
        raise ValueError("watchdog.max_log_bytes: expected integer 1..64 MiB combined")
    if not isinstance(watchdog["cleanup"], str) or watchdog["cleanup"] not in {"windows-job", "posix-process-group"}:
        raise ValueError("watchdog.cleanup: unsupported owned process-tree cleanup")
    if independent and watchdog["cleanup"] != "posix-process-group":
        raise ValueError("independent-linux requires posix-process-group cleanup")
    captures = document["captures"]
    if not isinstance(captures, list) or not 1 <= len(captures) <= 6:
        raise ValueError("captures: expected 1..6 entries")
    ids, lanes, targets = set(), {}, set()
    for capture in captures:
        _local_fields(capture, {"id", "side", "lane", "workload", "checkout", "command",
                               "environment", "cache"}, "capture")
        for key in ("id", "lane"):
            if not isinstance(capture[key], str) or not re.fullmatch(r"[A-Za-z0-9_-]{1,64}", capture[key]):
                raise ValueError(f"capture.{key}: expected safe bounded identifier")
        if capture["id"] in ids:
            raise ValueError("duplicate capture id")
        ids.add(capture["id"])
        if not isinstance(capture["side"], str) or capture["side"] not in {"baseline", "current"}:
            raise ValueError("capture.side: expected baseline or current")
        workload = capture["workload"]
        if not isinstance(workload, str) or workload not in LOCAL_COMMANDS or capture["command"] != LOCAL_COMMANDS[workload]:
            raise ValueError("capture.command: must exactly retain the native or sample workflow Cargo test recipe")
        _local_path(capture["checkout"], "capture.checkout")
        environment = capture["environment"]
        if not isinstance(environment, dict) or set(environment) - LOCAL_ENV_KEYS:
            raise ValueError("capture.environment: unsupported build environment variable")
        for key, value in environment.items():
            if value is not None and (not isinstance(value, str) or len(value) > 4096 or "\x00" in value):
                raise ValueError(f"capture.environment.{key}: expected bounded string or null")
        cache = capture["cache"]
        _local_fields(cache, {"state", "key", "target_dir", "receipt", "prepared"}, "capture.cache")
        if not isinstance(cache["state"], str) or cache["state"] not in {"cold", "warm", "unverified"} or cache["prepared"] is not True:
            raise ValueError("capture.cache: declare cold/warm/unverified and prepared=true")
        _local_text(cache["key"], "capture.cache.key")
        _local_path(cache["target_dir"], "capture.cache.target_dir")
        target_identity = os.path.normcase(os.path.normpath(cache["target_dir"]))
        if target_identity in targets:
            raise ValueError("captures must use distinct target directories")
        targets.add(target_identity)
        _local_receipt(cache["receipt"], "capture.cache.receipt")
        required_environment = {"CARGO_TARGET_DIR": cache["target_dir"], "CARGO_NET_OFFLINE": "true",
                                "RUSTC": toolchain["rustc_path"], "RUSTDOC": toolchain["rustdoc_path"]}
        if any(environment.get(key) != value for key, value in required_environment.items()):
            raise ValueError("capture.environment: declare exact target, real rustc/rustdoc and offline=true")
        # Distinct target directories are recorded, not interpreted as equal
        # cache contents. The report never emits a local ratio.
        binding = {"workload": workload, "command": capture["command"],
                   "environment": {key: environment.get(key) for key in LOCAL_ENV_KEYS if key != "CARGO_TARGET_DIR"},
                   "cache_state": cache["state"], "cache_key": cache["key"]}
        previous = lanes.setdefault(capture["lane"], binding)
        if previous != binding:
            raise ValueError("same lane has mismatched workload, command, environment or cache declarations")
    if independent:
        pairs = {(capture["side"], capture["workload"]) for capture in captures}
        expected_pairs = {(side, workload) for side in ("baseline", "current")
                          for workload in ("native", "sample")}
        native_lanes = {capture["lane"] for capture in captures if capture["workload"] == "native"}
        sample_lanes = {capture["lane"] for capture in captures if capture["workload"] == "sample"}
        if (len(captures) != 4 or pairs != expected_pairs or len(native_lanes) != 1
                or len(sample_lanes) != 1 or native_lanes == sample_lanes):
            raise ValueError("independent-linux requires exactly native and sample baseline/current pairs on two distinct lanes")
    return copy.deepcopy(document)


def _local_utc():
    return datetime.now(timezone.utc).isoformat()


def _local_sha(path):
    digest, size = hashlib.sha256(), 0
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(65536), b""):
            digest.update(chunk)
            size += len(chunk)
    return {"path": str(Path(path).resolve()), "bytes": size, "sha256": digest.hexdigest()}


def _local_probe(command, cwd=None, environment=None):
    process = subprocess.Popen(command, cwd=cwd, env=environment, stdin=subprocess.DEVNULL,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, shell=False)
    try:
        stdout, stderr = process.communicate()
    except KeyboardInterrupt:
        # Metadata probes also drain and reap naturally, without termination.
        process.communicate()
        raise
    if process.returncode != 0 or len(stdout) > 65536 or len(stderr) > 65536:
        raise ValueError("bounded local metadata probe failed")
    return stdout.decode("utf-8").strip()


def _local_source(checkout, expected):
    actual = {"sha": _local_probe(["git", "rev-parse", "HEAD"], checkout),
              "tree_sha": _local_probe(["git", "rev-parse", "HEAD^{tree}"], checkout),
              "clean": not _local_probe(["git", "status", "--porcelain=v1", "--untracked-files=normal"], checkout)}
    if actual["sha"] != expected["sha"] or actual["tree_sha"] != expected["tree_sha"] or not actual["clean"]:
        raise ValueError("checkout SHA/tree/clean state does not match declared side")
    return actual


def _local_environment(capture, toolchain):
    declared = capture["environment"]
    # Refuse hidden build flags, wrappers and target-specific overrides. Values
    # outside the safe allowlist never appear in the report or error message.
    for key in os.environ:
        if (key in LOCAL_ENV_KEYS or key.startswith("CARGO_TARGET_")
                or key.startswith("CARGO_PROFILE_") or key in {"RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_RUSTC"}):
            if key not in declared:
                raise ValueError("inherited build environment contains an undeclared or unsupported override")
    environment = dict(os.environ)
    for key in LOCAL_ENV_KEYS:
        value = declared.get(key)
        if value is None:
            environment.pop(key, None)
        else:
            environment[key] = value
    # Test subprocesses discover installed sibling tools, not rustup shims.
    environment["PATH"] = str(Path(toolchain["cargo_path"]).parent) + os.pathsep + environment.get("PATH", "")
    return environment


def _local_receipt_file(receipt):
    actual = _local_sha(receipt["path"])
    if actual["sha256"] != receipt["sha256"]:
        raise ValueError("coordination/cache receipt SHA256 mismatch")
    return actual


def _local_receipt_document(receipt):
    """Snapshot bounded receipt bytes once; the hash is not issuer authentication."""
    started = _local_utc()
    path = Path(receipt["path"])
    with path.open("rb") as stream:
        raw = stream.read(65537)
    ended = _local_utc()
    if len(raw) > 65536 or hashlib.sha256(raw).hexdigest() != receipt["sha256"]:
        raise ValueError("bounded coordination receipt SHA256 mismatch")
    try:
        def unique_object(items):
            value = {}
            for key, item in items:
                if key in value:
                    raise ValueError("duplicate coordination receipt JSON key")
                value[key] = item
            return value
        def reject_constant(value):
            raise ValueError("nonfinite receipt JSON")
        document = json.loads(raw.decode("utf-8"), object_pairs_hook=unique_object,
                              parse_constant=reject_constant)
    except (UnicodeDecodeError, json.JSONDecodeError, RecursionError) as exc:
        raise ValueError("invalid bounded coordination receipt JSON") from exc
    return {"path": str(path.resolve()), "bytes": len(raw), "sha256": receipt["sha256"],
            "read_started_utc": started, "read_completed_utc": ended, "document": document}


def _local_resource_id(value, path):
    if (not isinstance(value, str) or not re.fullmatch(r"[A-Za-z0-9_-]{1,64}", value)
            or set(re.split(r"[_-]", value.casefold())) & {"unknown", "unverified", "unset", "none", "null", "placeholder", "tbd"}):
        raise ValueError(path + ": expected known safe resource identifier")


def _local_linux_machine_identity():
    started = _local_utc()
    with Path("/etc/machine-id").open("rb") as stream:
        raw = stream.read(129)
    ended = _local_utc()
    identity = raw.strip()
    if (len(raw) > 128 or not re.fullmatch(rb"[0-9a-f]{32}", identity)
            or identity in {b"0" * 32, b"f" * 32}):
        raise ValueError("Linux machine-id is absent, malformed or a placeholder")
    return {"path": "/etc/machine-id", "bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest(),
            "read_started_utc": started, "read_completed_utc": ended,
            "scope": "observed OS installation identifier; not physical resource isolation"}


def _local_independent_admission(coordination, runner):
    if platform.system() != "Linux" or runner["os"] != "Linux" or runner["arch"] != platform.machine():
        raise ValueError("independent-linux actual runner OS/arch mismatch")
    receipts = {key: _local_receipt_document(coordination[key])
                for key in ("resource_receipt", "lane_receipt")}
    resource, lane = (receipts[key]["document"] for key in ("resource_receipt", "lane_receipt"))
    _local_fields(resource, {"schema", "owner", "runner_id", "n6_runner_id", "runner_machine_id_sha256",
                             "runner_os", "runner_arch", "separate_resources"}, "resource receipt")
    if (resource["schema"] != LOCAL_RESOURCE_SCHEMA or resource["owner"] != "su"
            or resource["separate_resources"] is not True):
        raise ValueError("resource receipt requires explicit su owner-attested separate resources")
    for key in ("runner_id", "n6_runner_id"):
        _local_resource_id(resource[key], "resource receipt." + key)
    if resource["runner_id"].casefold() == resource["n6_runner_id"].casefold():
        raise ValueError("independent-linux and N6 resource identifiers must differ")
    _local_digest(resource["runner_machine_id_sha256"], 64, "resource receipt.runner_machine_id_sha256")
    if resource["runner_os"] != runner["os"] or resource["runner_arch"] != runner["arch"]:
        raise ValueError("resource receipt runner OS/arch mismatch")
    _local_fields(lane, {"schema", "runner_id", "observed_at_utc", "quiet", "active_related_pids"}, "lane receipt")
    if (lane["schema"] != LOCAL_ADMISSION_SCHEMA or lane["runner_id"] != resource["runner_id"]
            or lane["quiet"] is not True or lane["active_related_pids"] != []):
        raise ValueError("lane receipt must identify the same quiet runner with no related PID")
    _local_text(lane["observed_at_utc"], "lane receipt.observed_at_utc")
    try:
        observed = datetime.fromisoformat(lane["observed_at_utc"].replace("Z", "+00:00"))
        admitted = datetime.fromisoformat(_local_utc())
        if observed.tzinfo is None or observed.utcoffset().total_seconds() != 0:
            raise ValueError("lane receipt needs an aware UTC observation")
        age = (admitted - observed).total_seconds()
    except (ValueError, OverflowError) as exc:
        raise ValueError("lane receipt needs an aware UTC observation") from exc
    if not 0 <= age <= 120:
        raise ValueError("lane receipt must be observed within the previous 120 seconds; future clocks refused")
    identity = _local_linux_machine_identity()
    if identity["sha256"] != resource["runner_machine_id_sha256"]:
        raise ValueError("actual Linux machine-id differs from resource receipt")
    return {"receipts": receipts, "admitted_at_utc": admitted.isoformat(), "lane_age_seconds": age,
            "actual_machine_identity": identity,
            "verification": "receipt hashes bind observed bytes, not authenticated issuer; distinct N6 association and resources are owner-attested; no physical isolation or future quiet guarantee"}


def _local_linux_census():
    started = _local_utc()
    text = _local_probe(["ps", "-eo", "pid=,comm="])
    ended = _local_utc()
    seen, related = set(), []
    build_names = {"cargo", "rustc", "rustdoc", "rustup", "cc", "c++", "gcc", "g++", "clang", "clang++",
                   "ld", "ld.lld", "lld", "link", "cl", "cc1", "cc1plus", "collect2", "lto-wrapper", "lto1",
                   "physics", "physics3d", "arena", "ccache", "sccache", "ninja", "make", "cmake"}
    for line in text.splitlines():
        fields = line.strip().split(maxsplit=1)
        if (len(fields) != 2 or not re.fullmatch(r"[0-9]+", fields[0]) or int(fields[0]) <= 0
                or int(fields[0]) in seen or len(fields[1]) > 255
                or any(ord(char) < 32 for char in fields[1])):
            raise ValueError("malformed or duplicate PID/name census")
        pid, name = int(fields[0]), fields[1]
        seen.add(pid)
        lowered = name.casefold()
        compiler_name = re.fullmatch(r"(?:.*-)?(?:gcc|g\+\+|clang(?:\+\+)?|cc|c\+\+|ld)(?:-[0-9.]+)?", lowered)
        if compiler_name or lowered in build_names or lowered.startswith(("orr_", "rustc", "rustdoc", "cargo", "cc1", "gcc-", "g++-", "clang-", "ld.")):
            related.append({"pid": pid, "name": name})
    if not seen:
        raise ValueError("empty PID/name census cannot establish current admission evidence")
    raw = text.encode("utf-8")
    return {"observed_started_utc": started, "observed_completed_utc": ended,
            "collector_pid": os.getpid(), "process_count": len(seen), "related_processes": related,
            "observed_text_bytes": len(raw), "observed_text_sha256": hashlib.sha256(raw).hexdigest(),
            "scope": "point-in-time PID/comm names; normalized metadata probe text; comm is mutable/truncated; no argv/environment, isolation or future quiet guarantee"}


def _local_cache(capture):
    target = Path(capture["cache"]["target_dir"])
    exists = target.exists()
    if exists and (not target.is_dir() or target.is_symlink()):
        raise ValueError("cache target must be a directory without a symlink")
    populated = exists and next(target.iterdir(), None) is not None
    state = capture["cache"]["state"]
    if state == "cold" and populated or state == "warm" and not populated:
        raise ValueError("target-directory occupancy does not match declared cache state")
    return {"target_exists": exists, "target_populated": populated,
            "receipt": _local_receipt_file(capture["cache"]["receipt"]),
            "state_verification": "occupancy only; cache provenance remains caller-declared"}


class _WindowsOwnedProcess:
    """Assign a non-terminating accounting Job before resuming Cargo."""
    def __init__(self, command, cwd, environment):
        import ctypes
        from ctypes import wintypes
        import msvcrt
        self.ctypes = ctypes
        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel = self.kernel
        class StartupInfo(ctypes.Structure):
            _fields_ = [("cb", wintypes.DWORD), ("reserved", wintypes.LPWSTR),
                        ("desktop", wintypes.LPWSTR), ("title", wintypes.LPWSTR),
                        ("x", wintypes.DWORD), ("y", wintypes.DWORD),
                        ("xsize", wintypes.DWORD), ("ysize", wintypes.DWORD),
                        ("xchars", wintypes.DWORD), ("ychars", wintypes.DWORD),
                        ("fill", wintypes.DWORD), ("flags", wintypes.DWORD),
                        ("show", wintypes.WORD), ("reserved2size", wintypes.WORD),
                        ("reserved2", ctypes.c_void_p), ("stdin", wintypes.HANDLE),
                        ("stdout", wintypes.HANDLE), ("stderr", wintypes.HANDLE)]
        class ProcessInfo(ctypes.Structure):
            _fields_ = [("process", wintypes.HANDLE), ("thread", wintypes.HANDLE),
                        ("pid", wintypes.DWORD), ("tid", wintypes.DWORD)]
        class BasicLimit(ctypes.Structure):
            _fields_ = [("process_time", ctypes.c_longlong), ("job_time", ctypes.c_longlong),
                        ("flags", wintypes.DWORD), ("minimum", ctypes.c_size_t),
                        ("maximum", ctypes.c_size_t), ("active", wintypes.DWORD),
                        ("affinity", ctypes.c_size_t), ("priority", wintypes.DWORD),
                        ("scheduling", wintypes.DWORD)]
        class ExtendedLimit(ctypes.Structure):
            _fields_ = [("basic", BasicLimit), ("io", ctypes.c_ulonglong * 6),
                        ("process_memory", ctypes.c_size_t), ("job_memory", ctypes.c_size_t),
                        ("peak_process", ctypes.c_size_t), ("peak_job", ctypes.c_size_t)]
        declarations = {
            "CreateJobObjectW": ([ctypes.c_void_p, wintypes.LPCWSTR], wintypes.HANDLE),
            "SetInformationJobObject": ([wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD], wintypes.BOOL),
            "AssignProcessToJobObject": ([wintypes.HANDLE, wintypes.HANDLE], wintypes.BOOL),
            "CreateProcessW": ([wintypes.LPCWSTR, wintypes.LPWSTR, ctypes.c_void_p, ctypes.c_void_p,
                                wintypes.BOOL, wintypes.DWORD, ctypes.c_void_p, wintypes.LPCWSTR,
                                ctypes.POINTER(StartupInfo), ctypes.POINTER(ProcessInfo)], wintypes.BOOL),
            "ResumeThread": ([wintypes.HANDLE], wintypes.DWORD),
            "QueryInformationJobObject": ([wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD, ctypes.c_void_p], wintypes.BOOL),
            "WaitForSingleObject": ([wintypes.HANDLE, wintypes.DWORD], wintypes.DWORD),
            "GetExitCodeProcess": ([wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)], wintypes.BOOL),
            "CloseHandle": ([wintypes.HANDLE], wintypes.BOOL),
        }
        for name, (argtypes, restype) in declarations.items():
            getattr(kernel, name).argtypes, getattr(kernel, name).restype = argtypes, restype
        self.job = kernel.CreateJobObjectW(None, None)
        self.handle = None
        self.thread = None
        self.stdout = self.stderr = None
        self.containment_verified = False
        self.setup_error = None
        self.resume_pending = False
        if not self.job:
            raise OSError("cannot create owned Windows Job")
        limits = ExtendedLimit()
        limits.basic.flags = 0  # Closing the Job never terminates a live process.
        descriptors, thread = [], None
        try:
            if not kernel.SetInformationJobObject(self.job, 9, ctypes.byref(limits), ctypes.sizeof(limits)):
                raise OSError("cannot configure owned Windows Job")
            out_read, out_write = os.pipe()
            descriptors.extend([out_read, out_write])
            err_read, err_write = os.pipe()
            descriptors.extend([err_read, err_write])
            null_fd = os.open(os.devnull, os.O_RDONLY)
            descriptors.append(null_fd)
            for descriptor in (out_write, err_write, null_fd):
                os.set_inheritable(descriptor, True)
            startup = StartupInfo()
            startup.cb, startup.flags = ctypes.sizeof(startup), 0x100
            startup.stdin = msvcrt.get_osfhandle(null_fd)
            startup.stdout, startup.stderr = msvcrt.get_osfhandle(out_write), msvcrt.get_osfhandle(err_write)
            self.stdout = os.fdopen(out_read, "rb", buffering=0)
            descriptors.remove(out_read)
            self.stderr = os.fdopen(err_read, "rb", buffering=0)
            descriptors.remove(err_read)
            info = ProcessInfo()
            block = ctypes.create_unicode_buffer("\0".join(f"{key}={value}" for key, value in sorted(environment.items(), key=lambda item: item[0].casefold())) + "\0\0")
            line = ctypes.create_unicode_buffer(subprocess.list2cmdline(command))
            if not kernel.CreateProcessW(command[0], line, None, None, True, 0x4 | 0x400 | 0x200 | 0x08000000,
                                         block, str(cwd), ctypes.byref(startup), ctypes.byref(info)):
                raise OSError("cannot create suspended owned Cargo process")
            self.handle, thread, self.pid = info.process, info.thread, info.pid
            self.containment_verified = bool(kernel.AssignProcessToJobObject(self.job, self.handle))
            if not self.containment_verified:
                self.setup_error = "Windows Job assignment failed; descendant containment unverified"
            self._resume_owned_child(thread)
        except BaseException as exc:
            if self.handle:
                # A created child must run its approved command and drain
                # naturally even if accounting setup failed. No termination.
                self.setup_error = type(exc).__name__ + ": launch setup failed; natural drain required"
                self.containment_verified = False
                self._resume_owned_child(thread)
            else:
                self.close()
                raise
        finally:
            if thread and not self.resume_pending:
                kernel.CloseHandle(thread)
            for descriptor in descriptors:
                os.close(descriptor)

    def _resume_owned_child(self, thread):
        # If both attempts fail, retain every owned handle and keep the
        # collector alive until external resolution/natural exit. A PID-only
        # error followed by closing these handles would lose ownership.
        if thread and self.kernel.ResumeThread(thread) != 0xffffffff:
            self.resume_pending = False
            return
        if thread and self.kernel.ResumeThread(thread) != 0xffffffff:
            self.resume_pending = False
            self.setup_error = "Windows first resume failed; second resumed; natural drain required"
            return
        self.thread = thread
        self.resume_pending = True
        self.setup_error = "Windows resume failed; possibly suspended owned child; lane remains held"

    def poll(self):
        wait_status = self.kernel.WaitForSingleObject(self.handle, 0)
        if wait_status == 0x102:
            return None
        if wait_status != 0:
            raise OSError("owned Cargo wait failed; lane remains held")
        code = self.ctypes.c_ulong()
        if not self.kernel.GetExitCodeProcess(self.handle, self.ctypes.byref(code)):
            raise OSError("cannot read owned Cargo exit status")
        return code.value

    def wait(self, timeout=None):
        milliseconds = 0xffffffff if timeout is None else int(timeout * 1000)
        wait_status = self.kernel.WaitForSingleObject(self.handle, milliseconds)
        if wait_status == 0x102:
            raise TimeoutError("owned Cargo process has not naturally exited")
        if wait_status != 0:
            raise OSError("owned Cargo wait failed; lane remains held")
        return self.poll()

    def active_descendants(self):
        accounting = (self.ctypes.c_ubyte * 48)()
        if not self.kernel.QueryInformationJobObject(self.job, 1, accounting, 48, None):
            raise OSError("owned Windows Job census unavailable; lane remains held")
        return int.from_bytes(bytes(accounting)[40:44], "little")

    def close(self):
        for attribute in ("thread", "job", "handle"):
            handle = getattr(self, attribute, None)
            if handle:
                self.kernel.CloseHandle(handle)
                setattr(self, attribute, None)
        for stream in (getattr(self, "stdout", None), getattr(self, "stderr", None)):
            if stream:
                stream.close()


def _local_group_active(process):
    if isinstance(process, _WindowsOwnedProcess):
        return process.active_descendants()
    listing = _local_probe(["ps", "-eo", "pid=,pgid="])
    return sum(len(fields) == 2 and fields[1] == str(process.pid)
               for fields in (line.split() for line in listing.splitlines()))


def _local_run(command, checkout, environment, directory, watchdog):
    """Bound saved bytes; failures stop future captures, never a live process."""
    state = {"saved": 0, "observed": {"stdout": 0, "stderr": 0}, "errors": [],
             "eof": set(), "write_failed": set()}
    lock, overflow = threading.RLock(), threading.Event()
    launched = threading.Event()
    process, readers = None, []
    result = {"pid": None, "exit_code": None, "status": "failure", "cleanup": watchdog["cleanup"],
              "automatic_termination": False, "lane_held": True}
    files = {}
    try:
        for name in ("stdout", "stderr"):
            files[name] = (directory / (name + ".raw")).open("xb", buffering=0)
    except OSError:
        for stream in files.values():
            stream.close()
        raise
    def drain(name):
        launched.wait()
        if process is None:
            return
        pipe = getattr(process, name)
        try:
            while True:
                chunk = pipe.read(65536)
                if not chunk:
                    with lock:
                        state["eof"].add(name)
                    break
                with lock:
                    state["observed"][name] += len(chunk)
                    remaining = max(0, watchdog["max_log_bytes"] - state["saved"])
                    accepted = chunk[:remaining]
                    if name not in state["write_failed"]:
                        # Reserve the entire requested prefix before writing;
                        # partial/failed storage cannot overspend the bound.
                        state["saved"] += len(accepted)
                        try:
                            written = files[name].write(accepted)
                            if written != len(accepted):
                                raise OSError("short raw write")
                            files[name].flush()
                        except (OSError, ValueError) as exc:
                            # Continue consuming the pipe after storage fails.
                            # Only the actual on-disk prefix is later hashed.
                            state["write_failed"].add(name)
                            state["errors"].append(name + ": raw write " + type(exc).__name__)
                            overflow.set()
                            mark_failure("failure", "raw_storage_write_error; continuing discard drain")
                    if len(accepted) != len(chunk):
                        overflow.set()
                        mark_failure("failure", "log_limit_or_capture_error")
        except (OSError, ValueError) as exc:
            with lock:
                state["errors"].append(type(exc).__name__)
            overflow.set()
    def mark_failure(status, reason):
        if "failure_reason" not in result:
            result["status"], result["failure_reason"] = status, reason
            try:
                _local_write(directory / "incomplete.json", {"status": status, "reason": reason,
                          "pid": result["pid"], "utc": _local_utc(), "lane_held": True,
                          "collector_pid": os.getpid(),
                          "owned_handles_retained": process is not None,
                          "automatic_termination": False,
                          "natural_exit_pending": result["pid"] is not None and result["exit_code"] is None,
                          "descendant_or_eof_verification_pending": True})
            except OSError as exc:
                # A full disk must not turn the observer into a blocked pipe.
                result["incomplete_receipt_error"] = type(exc).__name__
    try:
        result["utc_started"], result["monotonic_started_ns"] = _local_utc(), time.monotonic_ns()
        deadline = result["monotonic_started_ns"] + watchdog["seconds"] * 1_000_000_000
        # Start both waiting readers before creating any owned child. A
        # Thread.start failure therefore cannot strand a chatty process.
        for name in ("stdout", "stderr"):
            reader = threading.Thread(target=drain, args=(name,), daemon=True)
            readers.append(reader)
            reader.start()
        if os.name == "nt":
            process = _WindowsOwnedProcess(command, checkout, environment)
        else:
            process = subprocess.Popen(command, cwd=checkout, env=environment, stdin=subprocess.DEVNULL,
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE, shell=False,
                                       start_new_session=True)
        result["pid"] = process.pid
        launched.set()
        contained = not isinstance(process, _WindowsOwnedProcess) or process.containment_verified
        result["containment_verified"] = contained
        if not contained:
            mark_failure("failure", "containment_failed")
        if getattr(process, "setup_error", None):
            mark_failure("failure", process.setup_error)
        if getattr(process, "resume_pending", False):
            result["residual_possibly_suspended_pid"] = process.pid
            result["owned_resume_thread_handle_retained"] = True
            mark_failure("failure", "resume_failed; owned child and handles retained; awaiting external resolution/natural exit")
        while process.poll() is None:
            if overflow.is_set():
                mark_failure("failure", "log_limit_or_capture_error")
            if time.monotonic_ns() >= deadline:
                mark_failure("timed_out", "owned_process_watchdog_exceeded; awaiting natural exit")
            time.sleep(0.02)
        result["exit_code"] = process.poll()
        if "failure_reason" not in result:
            if result["exit_code"] == 0:
                result["status"] = "success"
            else:
                mark_failure("failure", "owned_process_exit_nonzero")
        result["monotonic_completed_ns"], result["utc_completed"] = time.monotonic_ns(), _local_utc()
    except KeyboardInterrupt:
        mark_failure("cancelled", "collector_interrupted; awaiting natural exit")
    except (OSError, ValueError, RuntimeError) as exc:
        if hasattr(exc, "owned_pid"):
            result["pid"] = exc.owned_pid
            result["residual_possibly_suspended_pid"] = exc.owned_pid
        mark_failure("failure", type(exc).__name__ + ": " + str(exc))
    finally:
        launched.set()
        # Retain the lane while the original owned command and descendants
        # naturally exit. A watchdog is an incomplete observation, not kill
        # authority. Closing the accounting Job has no termination flag.
        if process is not None:
            try:
                result["exit_code"] = process.wait()
                result["owned_process_reaped"] = True
            except (OSError, TimeoutError, subprocess.TimeoutExpired) as exc:
                mark_failure("failure", "natural_reap_unverified")
                result["cleanup_error"] = type(exc).__name__
            if "monotonic_completed_ns" not in result:
                result["monotonic_completed_ns"], result["utc_completed"] = time.monotonic_ns(), _local_utc()
            if result.get("containment_verified") and result.get("owned_process_reaped"):
                try:
                    while _local_group_active(process):
                        if time.monotonic_ns() >= deadline:
                            mark_failure("timed_out", "owned_descendants_watchdog_exceeded; awaiting natural exit")
                        time.sleep(0.05)
                    result["active_owned_descendants"] = 0
                except (OSError, ValueError) as exc:
                    mark_failure("failure", "owned_descendant_census_unverified")
                    result["census_error"] = type(exc).__name__
        for reader in readers:
            while reader.is_alive():
                if time.monotonic_ns() >= deadline:
                    mark_failure("timed_out", "pipe_eof_watchdog_exceeded; awaiting natural EOF")
                reader.join(timeout=0.05)
        if process is not None:
            result["drain_complete"] = state["eof"] == {"stdout", "stderr"}
            result["raw_storage_errors"] = state["errors"]
            result["lane_held"] = not (result.get("owned_process_reaped")
                                       and result.get("active_owned_descendants") == 0
                                       and result["drain_complete"])
            if isinstance(process, _WindowsOwnedProcess):
                process.close()
            else:
                process.stdout.close()
                process.stderr.close()
        else:
            result["lane_held"] = False  # Reader/launch preflight created no child.
        for stream in files.values():
            try:
                stream.close()
            except OSError as exc:
                state["errors"].append("raw close " + type(exc).__name__)
                mark_failure("failure", "raw_storage_close_error")
        if "monotonic_completed_ns" not in result:
            result["monotonic_completed_ns"], result["utc_completed"] = time.monotonic_ns(), _local_utc()
        result["capture_finalized_utc"] = _local_utc()
    result["total_seconds"] = _decimal_text(Decimal(result["monotonic_completed_ns"] - result["monotonic_started_ns"]) / Decimal(1_000_000_000))
    result["streams"] = {name: dict(_local_sha(directory / (name + ".raw")),
                                    observed_bytes=state["observed"][name]) for name in ("stdout", "stderr")}
    for stream in result["streams"].values():
        stream["discarded_bytes"] = stream["observed_bytes"] - stream["bytes"]
        stream["sha256_scope"] = "full_drained_stream" if stream["discarded_bytes"] == 0 and result.get("drain_complete") and not state["errors"] else "saved_binary_prefix"
        stream["original_full_sha256"] = stream["sha256"] if stream["sha256_scope"] == "full_drained_stream" else None
    result["discarded_bytes"] = sum(stream["discarded_bytes"] for stream in result["streams"].values())
    if overflow.is_set() or state["errors"]:
        mark_failure("failure", "log_limit_or_capture_error")
    return result


def _local_write(path, value):
    with Path(path).open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(value, stream, indent=2, ensure_ascii=False, allow_nan=False)
        stream.write("\n")


def _local_cleanup_mode():
    return "windows-job" if os.name == "nt" else "posix-process-group" if os.name == "posix" else None


def collect_local(document, output_dir):
    """Collect an explicit plan once; do not manufacture stage-1 CI provenance."""
    plan = validate_local_plan(document)
    _local_path(str(output_dir), "output_dir")
    directory, once_dir = Path(output_dir), Path(plan["once_dir"])
    if directory.exists() or any((once_dir / (plan["plan_id"] + suffix)).exists()
                                 for suffix in (".started.json", ".completed.json")):
        raise ValueError("completed/partial plan or output exists; capture rerun refused before metadata probes")
    expected_cleanup = _local_cleanup_mode()
    if plan["watchdog"]["cleanup"] != expected_cleanup:
        raise ValueError("owned cleanup unsupported on this platform")
    runner = plan["runner"]
    if runner["os"] != platform.system() or runner["arch"] != platform.machine():
        raise ValueError("actual platform differs from declared runner")
    independent = plan["coordination"].get("mode") == "independent-linux"
    admission = _local_independent_admission(plan["coordination"], runner) if independent else None
    toolchain = runner["toolchain"]
    actual_tools = {}
    parents = set()
    for name in ("cargo", "rustc", "rustdoc"):
        path = Path(toolchain[name + "_path"]).resolve(strict=True)
        if path.name.casefold() not in {name, name + ".exe"} or not path.is_file():
            raise ValueError("toolchain must reference installed real Cargo/Rust tools")
        parents.add(path.parent)
        # rustup proxies can have the right basename but the same bytes.
        proxy = path.parent / ("rustup.exe" if os.name == "nt" else "rustup")
        binary = _local_sha(path)
        if proxy.is_file() and _local_sha(proxy)["sha256"] == binary["sha256"]:
            raise ValueError("rustup proxy refused; choose installed toolchain bin paths")
        version = _local_probe([str(path), "--version"])
        if version != toolchain[name + "_version"]:
            raise ValueError("actual installed toolchain version differs from plan")
        actual_tools[name] = dict(binary, version=version)
    if len(parents) != 1:
        raise ValueError("Cargo, rustc and rustdoc must share an installed toolchain bin directory")
    receipts = admission["receipts"] if independent else {
        key: _local_receipt_file(plan["coordination"][key]) for key in ("lane_receipt", "n6_receipt")}
    for capture in plan["captures"]:
        _local_source(capture["checkout"], plan[capture["side"]])
        _local_cache(capture)
        _local_environment(capture, toolchain)
    resolved_targets = [Path(capture["cache"]["target_dir"]).resolve() for capture in plan["captures"]]
    if len(set(resolved_targets)) != len(resolved_targets):
        raise ValueError("distinct target paths resolve to a reused cache directory")
    if os.name == "posix":
        _local_probe(["ps", "-eo", "pid=,pgid="])
    once_dir.mkdir(parents=True, exist_ok=True)
    if once_dir.is_symlink():
        raise ValueError("once_dir must not be a symlink")
    local_id = "local-" + str(uuid.uuid4())
    marker = once_dir / (plan["plan_id"] + ".started.json")
    _local_write(marker, {"plan_id": plan["plan_id"], "local_collection_id": local_id,
                          "utc": _local_utc(), "output_dir": str(directory)})
    directory.mkdir(parents=True, exist_ok=False)
    _local_write(directory / "plan.json", plan)
    report = {"schema": LOCAL_REPORT_SCHEMA, "local_collection_id": local_id,
              "plan_id": plan["plan_id"], "plan_sha256": hashlib.sha256(json.dumps(plan, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")).hexdigest(),
              "runner": runner, "actual_platform": {"os": platform.system(), "arch": platform.machine(), "release": platform.release()},
              "actual_toolchain": actual_tools, "coordination_receipts": receipts,
              "coordination_verification": admission["verification"] if independent else "receipt bytes verified; N6 completion, lane, cache and isolation are caller-declared",
              "resource_admission": admission,
              "capture_bound_scope": "per-plan maximum only; cross-machine capture allocation requires external owner coordination",
              "captures": [], "status": "completed", "comparisons": [],
              "comparison_reason": "local measurements have no CI identities; cache/isolation equivalence and separate compile/runtime walls are not attested"}
    for capture in plan["captures"]:
        capture_dir = directory / capture["id"]
        capture_dir.mkdir(exist_ok=False)
        record = {"local_capture_id": local_id + ":" + capture["id"], "declaration": capture,
                  "source_before": None, "source_after": None}
        try:
            record["source_before"] = _local_source(capture["checkout"], plan[capture["side"]])
            record["cache_before"] = _local_cache(capture)
            if not independent:
                for key in ("lane_receipt", "n6_receipt"):
                    _local_receipt_file(plan["coordination"][key])
            environment = _local_environment(capture, toolchain)
            if independent:
                record["launch_census"] = _local_linux_census()
                if record["launch_census"]["related_processes"]:
                    raise ValueError("current Linux PID/name census contains related processes; launch refused")
            _local_write(capture_dir / "started.json", {"local_capture_id": record["local_capture_id"], "utc": _local_utc()})
            command = [toolchain["cargo_path"], *capture["command"][1:]]
            record["execution"] = _local_run(command, capture["checkout"], environment, capture_dir, plan["watchdog"])
            record["status"] = record["execution"]["status"]
            record["measurements"] = {
                "total": {"seconds": record["execution"]["total_seconds"], "scope": "command-wall", "source": "independent monotonic_ns around owned Cargo invocation; pipe drain/finalization excluded"},
                "compile": {"seconds": None, "reason": "Cargo test does not expose an independently measured compile-only wall"},
                "runtime": {"seconds": None, "reason": "not independently measured; no subtraction of Cargo HTML or Finished timing"}}
            record["source_after"] = _local_source(capture["checkout"], plan[capture["side"]])
        except (OSError, ValueError, subprocess.TimeoutExpired) as exc:
            record["status"], record["failure_reason"] = "failure", type(exc).__name__ + ": " + str(exc)
        _local_write(capture_dir / "receipt.json", record)
        report["captures"].append(record)
        if record["status"] != "success":
            report["status"] = "failed_stop"
            break
    report["planned_captures"], report["attempted_captures"] = len(plan["captures"]), len(report["captures"])
    _local_write(directory / "report.json", report)
    _local_write(once_dir / (plan["plan_id"] + ".completed.json"), {"local_collection_id": local_id,
                                                                  "status": report["status"], "utc": _local_utc(),
                                                                  "report": _local_sha(directory / "report.json")})
    return report


def main(argv=None):
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", nargs="?", help="stage-1 UTF-8 JSON path, or - for stdin")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--collect", metavar="PLAN", help="explicitly execute a bounded local Cargo plan once")
    mode.add_argument("--validate-plan", metavar="PLAN", help="validate local declarations without invoking any program")
    parser.add_argument("--output", metavar="DIR", help="new absolute output directory for --collect")
    parser.add_argument("--format", choices=("json", "markdown"), default="json")
    args = parser.parse_args(argv)
    if bool(args.collect or args.validate_plan) == bool(args.input):
        parser.error("choose a stage-1 input, --collect PLAN, or --validate-plan PLAN")
    if bool(args.output) != bool(args.collect):
        parser.error("--output is required only with --collect")
    if (args.collect or args.validate_plan) and args.format != "json":
        parser.error("local modes emit JSON only")
    try:
        input_path = args.collect or args.validate_plan or args.input
        if input_path == "-":
            data = sys.stdin.buffer.read(MAX_INPUT_BYTES + 1)
        else:
            with Path(input_path).open("rb") as stream:
                data = stream.read(MAX_INPUT_BYTES + 1)
        document = loads_document(data.decode("utf-8"))
        if args.validate_plan:
            validated = validate_local_plan(document)
            report = {"schema": LOCAL_PLAN_SCHEMA, "plan_id": validated["plan_id"],
                      "valid": True, "validation_scope": "declarations only; no programs or files opened",
                      "planned_captures": len(validated["captures"])}
        elif args.collect:
            report = collect_local(document, args.output)
        else:
            report = build_report(document)
        if args.format == "markdown":
            sys.stdout.write(render_markdown(report))
        else:
            sys.stdout.write(json.dumps(report, indent=2, ensure_ascii=False,
                                        default=lambda value: str(value),
                                        allow_nan=False) + "\n")
        return 1 if args.collect and report["status"] != "completed" else 0
    except (OSError, UnicodeError, ValueError, RecursionError, subprocess.TimeoutExpired) as exc:
        print(f"ci-cost-report: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
