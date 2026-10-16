"""Local protocol-only solver used for runner integration and accounting tests.

This deterministic driver is a harness fixture, not an LLM or a measured agent.
It deliberately issues the normal discovery, authoring, input, timeline,
undo/redo, save, host restart, and replay actions over the JSONL broker.
"""

from __future__ import annotations

import argparse
import json
import sys
from typing import Any


def send(value: dict[str, Any]) -> None:
    sys.stdout.write(json.dumps(value, separators=(",", ":"), sort_keys=True) + "\n")
    sys.stdout.flush()


def request(counter: int, op: str, **args: Any) -> tuple[int, dict[str, Any]]:
    counter += 1
    send({"type": "request", "id": f"mock-{counter}", "op": op, "args": args})
    line = sys.stdin.readline()
    if not line:
        raise RuntimeError("broker closed the protocol")
    reply = json.loads(line)
    if reply.get("ok") is not True:
        raise RuntimeError(reply.get("error") or "broker rejected request")
    return counter, reply.get("result") or {}


def act_task1(counter: int) -> int:
    counter, _ = request(counter, "read", what="status")
    counter, _ = request(counter, "read", what="schema")
    counter, _ = request(counter, "read", what="scene")
    counter, _ = request(counter, "apply", label="create two-player arena", operations=[
        {"op": "spawn_player", "name": "hero", "slot": 0, "pos": [-300, 0]},
        {"op": "spawn_player", "name": "target", "slot": 1, "pos": [300, 0]},
    ])
    counter, _ = request(counter, "read", what="history")
    counter, _ = request(counter, "task_done")
    return counter


def act_task2(counter: int, *, fail: bool = False, inject_infra: bool = False) -> int:
    counter, _ = request(counter, "sim_start")
    if inject_infra:
        # Exit 75 is reserved by this bundled mock for a runner-observed
        # infrastructure interruption. The task checkpoint remains pre-task.
        sys.exit(75)
    counter, _ = request(counter, "sim_input", player=0, value={"axis_x": 1, "axis_y": 0, "buttons": []})
    counter, _ = request(counter, "sim_step", ticks=9 if fail else 10)
    counter, _ = request(counter, "sim_input", player=0, value={"axis_x": 0, "axis_y": 0, "buttons": []})
    counter, _ = request(counter, "sim_step", ticks=5)
    counter, _ = request(counter, "sim_stop")
    counter, _ = request(counter, "task_done")
    return counter


def act_shot(counter: int, ticks: int, replay: str | None, every_tick: bool) -> int:
    counter, _ = request(counter, "sim_start")
    counter, _ = request(counter, "sim_input", player=1, value={"axis_x": 0, "axis_y": 0, "buttons": []})
    counter, _ = request(counter, "sim_input", player=0, value={"axis_x": 0, "axis_y": 0, "buttons": ["fire"]})
    counter, _ = request(counter, "sim_step", ticks=1)
    counter, _ = request(counter, "sim_input", player=0, value={"axis_x": 0, "axis_y": 0, "buttons": []})
    for _ in range(ticks - 1):
        counter, _ = request(counter, "sim_step", ticks=1 if every_tick else ticks - 1)
        if not every_tick:
            break
    counter, _ = request(counter, "sim_stop", replay=replay)
    return counter


def act_task4(counter: int) -> int:
    counter, scene = request(counter, "read", what="scene")
    # The broker API takes the entity GUID, discovered from the read result.
    target_id = next(entity["id"] for entity in scene["entities"]
                     if any(key.rsplit("::", 1)[-1] == "PlayerTag" and value.get("slot") == 1
                            for key, value in entity.get("values", {}).items()))
    counter, _ = request(counter, "apply", label="move target farther away", operations=[
        {"op": "set_position", "entity": target_id, "pos": [900, 0]},
    ])
    counter = act_shot(counter, 50, "modified50", True)
    counter, _ = request(counter, "verify_replay", replay="modified50")
    counter, _ = request(counter, "undo")
    counter, _ = request(counter, "redo")
    counter, _ = request(counter, "task_done")
    return counter


def act_task5(counter: int) -> int:
    counter, _ = request(counter, "save", write=True)
    counter, _ = request(counter, "restart_host")
    counter = act_shot(counter, 50, "fresh50", True)
    counter, _ = request(counter, "verify_replay", replay="modified50")
    counter, _ = request(counter, "task_done")
    return counter


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--scenario", choices=("success", "task2_failure", "infra_once"), default="success")
    args = parser.parse_args()
    first = sys.stdin.readline()
    if not first:
        return 2
    start = json.loads(first)
    if start.get("type") != "start":
        return 2
    task = start["task"]
    counter = 0
    if task == 1:
        act_task1(counter)
    elif task == 2:
        act_task2(counter, fail=args.scenario == "task2_failure",
                  inject_infra=args.scenario == "infra_once" and start.get("resume_count", 0) == 0)
    elif task == 3:
        counter = act_shot(counter, 20, "baseline20", False)
        counter, _ = request(counter, "verify_replay", replay="baseline20")
        request(counter, "task_done")
    elif task == 4:
        act_task4(counter)
    elif task == 5:
        act_task5(counter)
    else:
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
