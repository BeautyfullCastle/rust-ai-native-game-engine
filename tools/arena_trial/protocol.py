"""Small JSONL protocol and typed Arena request validation.

The broker accepts operations, never arbitrary CLI strings or ERP calls. This is
a protocol boundary for cooperative solvers, not an OS security boundary.
"""

from __future__ import annotations

import json
import math
import re
from typing import Any


class ProtocolError(ValueError):
    pass


TASKS = {
    1: "Create the two-player Arena scene through guarded apply. Discover the host first.",
    2: "Move player slot 0 right for ten ticks, release it, step five neutral ticks, and preserve the authoring document.",
    3: "Fire once, release, advance to tick 20, export and verify the baseline replay.",
    4: "Move the target to x=900 with guarded apply against the saved tick-20 replay; prove the changed outcome, play 50 ticks, then undo and redo exactly.",
    5: "Save the modified scene, restart the Arena host, replay 50 ticks, and verify the original tick-50 recording.",
}


def encode(message: dict[str, Any]) -> str:
    return json.dumps(message, separators=(",", ":"), sort_keys=True)


def decode(line: str) -> dict[str, Any]:
    try:
        value = json.loads(line)
    except (TypeError, json.JSONDecodeError) as exc:
        raise ProtocolError("message must be one JSON object per line") from exc
    if not isinstance(value, dict):
        raise ProtocolError("message must be an object")
    return value


def _fixed_vec2(value: Any, field: str) -> list[int | float]:
    if not isinstance(value, list) or len(value) != 2:
        raise ProtocolError(f"{field} must be a two-element array")
    for number in value:
        try:
            finite = math.isfinite(number)
        except (OverflowError, TypeError, ValueError):
            finite = False
        if isinstance(number, bool) or not isinstance(number, (int, float)) or not finite:
            raise ProtocolError(f"{field} must contain finite numbers")
    return value


def validate_request(message: dict[str, Any]) -> tuple[str, dict[str, Any]]:
    if message.get("type") != "request" or not isinstance(message.get("id"), str):
        raise ProtocolError("expected request with string id")
    op, args = message.get("op"), message.get("args", {})
    if not isinstance(op, str) or not isinstance(args, dict):
        raise ProtocolError("request op and args have invalid types")
    if op == "read":
        what = args.get("what")
        if not isinstance(what, str) or what not in {"status", "scene", "history", "schema", "state"}:
            raise ProtocolError("read supports status, scene, history, schema, or state")
    elif op == "apply":
        label, operations = args.get("label"), args.get("operations")
        if not isinstance(label, str) or not label.strip() or not isinstance(operations, list) or not operations:
            raise ProtocolError("apply requires label and a non-empty operations array")
        for item in operations:
            if not isinstance(item, dict):
                raise ProtocolError("each operation must be an object")
            kind = item.get("op")
            if kind == "spawn_player":
                name = item.get("name")
                slot = item.get("slot")
                if (not isinstance(name, str) or name not in {"hero", "target"} or isinstance(slot, bool) or
                        not isinstance(slot, int) or slot not in {0, 1}):
                    raise ProtocolError("spawn_player only accepts the two Arena player slots")
                _fixed_vec2(item.get("pos"), "pos")
            elif kind == "set_position":
                if not isinstance(item.get("entity"), str):
                    raise ProtocolError("set_position requires a discovered entity id")
                _fixed_vec2(item.get("pos"), "pos")
            else:
                raise ProtocolError("apply permits only spawn_player and set_position")
    elif op == "sim_start" or op == "sim_stop":
        if op == "sim_stop":
            replay = args.get("replay")
            if replay is not None and (not isinstance(replay, str) or
                                       replay not in {"baseline20", "modified50", "fresh50"}):
                raise ProtocolError("unknown runner-owned replay label")
    elif op == "sim_input":
        player = args.get("player")
        if isinstance(player, bool) or not isinstance(player, int) or player not in range(8):
            raise ProtocolError("player must be in [0, 7]")
        value = args.get("value")
        if not isinstance(value, dict) or set(value) != {"axis_x", "axis_y", "buttons"}:
            raise ProtocolError("input requires axis_x, axis_y, and buttons")
        axes = (value["axis_x"], value["axis_y"])
        if any(isinstance(axis, bool) or not isinstance(axis, int) or axis not in (-1, 0, 1)
               for axis in axes):
            raise ProtocolError("Arena mock contract accepts axes -1, 0, or 1")
        if value["buttons"] not in ([], ["fire"]):
            raise ProtocolError("buttons must be empty or [fire]")
    elif op == "sim_step":
        ticks = args.get("ticks")
        if isinstance(ticks, bool) or not isinstance(ticks, int) or not 1 <= ticks <= 6000:
            raise ProtocolError("ticks must be an integer in [1, 6000]")
    elif op == "verify_replay":
        replay = args.get("replay")
        if not isinstance(replay, str) or replay not in {"baseline20", "modified50"}:
            raise ProtocolError("verify_replay accepts only runner-owned baseline20 or modified50")
    elif op in {"undo", "redo"}:
        if args:
            raise ProtocolError(f"{op} takes no arguments")
    elif op == "save":
        if args.get("write") is not True:
            raise ProtocolError("save requires write=true")
    elif op == "restart_host":
        if args:
            raise ProtocolError("restart_host takes no arguments")
    elif op == "task_done":
        if args:
            raise ProtocolError("task_done takes no arguments")
    else:
        raise ProtocolError("unknown broker operation")
    return op, args


def validate_manifest(value: Any) -> dict[str, Any]:
    required = {
        "format", "experiment_id", "solver", "attempt_count", "seed", "players", "tick_rate",
        "task_timeout_seconds", "attempt_timeout_seconds", "command_timeout_seconds",
        "readiness_timeout_seconds", "max_requests", "max_interventions", "intervention_policy",
        "model", "token_budget", "cost_budget_usd", "usage_policy", "provenance",
    }
    if not isinstance(value, dict) or required - set(value):
        missing = sorted(required - set(value)) if isinstance(value, dict) else sorted(required)
        raise ProtocolError(f"manifest is missing fields: {', '.join(missing)}")
    if value["format"] != "orr.arena-agent-trial-manifest/1":
        raise ProtocolError("unsupported manifest format")
    if value["seed"] != 42 or value["players"] != 2 or value["tick_rate"] != 60:
        raise ProtocolError("Arena trial fixes seed=42, players=2, tick_rate=60")
    for field, maximum in (("attempt_count", 100), ("max_requests", 10000)):
        number = value[field]
        if isinstance(number, bool) or not isinstance(number, int) or not 1 <= number <= maximum:
            raise ProtocolError(f"{field} must be an integer in [1, {maximum}]")
    number = value["max_interventions"]
    if isinstance(number, bool) or not isinstance(number, int) or not 0 <= number <= 100:
        raise ProtocolError("max_interventions must be an integer in [0, 100]")
    for field in ("task_timeout_seconds", "attempt_timeout_seconds", "command_timeout_seconds", "readiness_timeout_seconds"):
        number = value[field]
        if isinstance(number, bool) or not isinstance(number, (int, float)) or not math.isfinite(number) or number <= 0:
            raise ProtocolError(f"{field} must be a finite positive number")
    if value["intervention_policy"] not in {"zero", "disclosed"}:
        raise ProtocolError("intervention_policy must be zero or disclosed")
    if value["intervention_policy"] == "zero" and value["max_interventions"] != 0:
        raise ProtocolError("zero intervention policy requires max_interventions=0")
    if value["usage_policy"] != "unavailable_is_unknown":
        raise ProtocolError("usage_policy must preserve unavailable usage as unknown")
    if not isinstance(value["experiment_id"], str) or not re.fullmatch(r"[A-Za-z0-9._-]{1,80}", value["experiment_id"]):
        raise ProtocolError("experiment_id contains unsupported characters")
    if not isinstance(value["solver"], dict) or not isinstance(value["solver"].get("name"), str) or not isinstance(value["solver"].get("version"), str):
        raise ProtocolError("solver must identify a name and version")
    if value["solver"].get("name") != "mock" or value["solver"].get("version") != "1":
        raise ProtocolError("this runner build executes only its bundled local mock solver")
    if value["solver"].get("argv") != ["{python}", "{repo}/tools/arena_trial/mock_solver.py"]:
        raise ProtocolError("solver argv must name the frozen bundled mock adapter")
    if not isinstance(value["solver"].get("script_sha256"), str) or len(value["solver"]["script_sha256"]) != 64:
        raise ProtocolError("solver script identity is missing")
    if not isinstance(value["solver"].get("interpreter"), str) or not isinstance(value["solver"].get("interpreter_path"), str):
        raise ProtocolError("solver interpreter identity is missing")
    if not isinstance(value["solver"].get("interpreter_sha256"), str) or len(value["solver"]["interpreter_sha256"]) != 64:
        raise ProtocolError("solver interpreter hash is missing")
    if not isinstance(value["model"], dict) or not isinstance(value["model"].get("provider"), str) or not isinstance(value["model"].get("version"), str):
        raise ProtocolError("model must identify provider and version (use mock for local tests)")
    if value["model"]["provider"] != "mock":
        raise ProtocolError("this runner build does not call model providers")
    if value["token_budget"] is not None and (isinstance(value["token_budget"], bool) or not isinstance(value["token_budget"], int) or value["token_budget"] <= 0):
        raise ProtocolError("token_budget must be a positive integer or null")
    if value["cost_budget_usd"] is not None and (isinstance(value["cost_budget_usd"], bool) or not isinstance(value["cost_budget_usd"], (int, float)) or not math.isfinite(value["cost_budget_usd"]) or value["cost_budget_usd"] <= 0):
        raise ProtocolError("cost_budget_usd must be positive or null")
    if value.get("mock_scenario", "success") not in {"success", "task2_failure", "infra_once"}:
        raise ProtocolError("mock_scenario must be success, task2_failure, or infra_once")
    provenance = value["provenance"]
    provenance_fields = {"commit", "tree", "dirty_status", "source_sha256", "source_file_count",
                         "fixture_sha256", "host_path", "host_sha256", "cli_path", "cli_sha256",
                         "engine_identity", "input_schema_sha256"}
    if not isinstance(provenance, dict) or provenance_fields - set(provenance):
        raise ProtocolError("provenance must freeze source, fixture, binaries, engine, and input schema")
    return value
