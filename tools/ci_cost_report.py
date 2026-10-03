#!/usr/bin/env python3
"""Offline comparison of declared CI timing captures; never runs a benchmark."""

from __future__ import annotations

import argparse
import copy
from decimal import Decimal, InvalidOperation, localcontext
import html
import json
import math
from pathlib import Path
import re
import sys

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
    for key, fields in (("platform", ("os", "arch", "image")),
                        ("runner", ("class", "hardware", "isolation")),
                        ("cache", ("state", "key"))):
        nested = _object(conditions.get(key), f"conditions.{key}", errors)
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


def main(argv=None):
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", help="UTF-8 JSON path, or - for stdin")
    parser.add_argument("--format", choices=("json", "markdown"), default="json")
    args = parser.parse_args(argv)
    try:
        if args.input == "-":
            data = sys.stdin.buffer.read(MAX_INPUT_BYTES + 1)
        else:
            with Path(args.input).open("rb") as stream:
                data = stream.read(MAX_INPUT_BYTES + 1)
        report = build_report(loads_document(data.decode("utf-8")))
        if args.format == "markdown":
            sys.stdout.write(render_markdown(report))
        else:
            sys.stdout.write(json.dumps(report, indent=2, ensure_ascii=False,
                                        default=lambda value: str(value),
                                        allow_nan=False) + "\n")
        return 0
    except (OSError, UnicodeError, ValueError, RecursionError) as exc:
        print(f"ci-cost-report: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
