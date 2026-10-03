"""Host-free regression tests for the Arena trial protocol and accounting."""

from __future__ import annotations

import copy
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

from . import runner
from .protocol import ProtocolError, decode, validate_manifest, validate_request
from .runner import Broker, TrialError, TrialRunner, default_manifest
from tools.arena_agent_benchmark import BenchmarkFailure
from tools.arena_agent_benchmark import scene_facts


def manifest() -> dict:
    value = default_manifest()
    value["solver"].update({
        "script_sha256": "a" * 64,
        "interpreter": sys.version.splitlines()[0],
        "interpreter_path": str(Path(sys.executable).resolve()),
        "interpreter_sha256": "b" * 64,
    })
    value["provenance"] = {
        "commit": "commit", "tree": "tree", "dirty_status": "",
        "source_sha256": "c" * 64, "source_file_count": 1,
        "fixture_sha256": "d" * 64, "host_path": "host", "host_sha256": "e" * 64,
        "cli_path": "cli", "cli_sha256": "f" * 64,
        "engine_identity": {"game": "Arena", "build_id": "0x1"},
        "input_schema_sha256": "1" * 64,
    }
    return value


class FakeBenchmark:
    def __init__(self, output: Path, *, recording_checked: int = 51,
                 history_entries: list[dict] | None = None, doc_checksum: str = "modified",
                 scene_responses: list[dict] | None = None, mode: str = "edit"):
        self.output = output
        self.playing = False
        self.engine = {"game": "Arena", "build_id": "0x1"}
        self.recording_checked = recording_checked
        self.history_entries = history_entries or []
        self.doc_checksum = doc_checksum
        self.scene_responses = list(scene_responses or [])
        self.mode = mode

    def cli(self, *args: str):
        if args == ("status",):
            return {"state": {"doc_checksum": self.doc_checksum, "mode": self.mode}}
        if args == ("history",):
            return {"entries": self.history_entries}
        if args == ("scene", "--components"):
            if self.scene_responses:
                return self.scene_responses.pop(0)
            return scene_query(-300, 900, score=0)
        raise AssertionError(f"unexpected fake CLI call: {args!r}")

    def verify(self, replay: Path, ticks: int, name: str):
        return {
            "checks": {"passed": True}, "ticks": ticks, "debug_commands_replayed": 0,
            "metrics": [
                {"name": "players", "candidate": {"end": 2}},
                {"name": "bullets", "candidate": {"end": 0}},
                {"name": "score_0", "base": {"end": 1}, "candidate": {"end": 0}},
                {"name": "score_1", "candidate": {"end": 0}},
                {"name": "out_of_bounds", "candidate": {"end": 0}},
            ],
            "recording": {"checked": self.recording_checked, "mismatches": 0, "first_mismatch": None},
        }


def scene_query(hero_x: int, target_x: int, *, score: int = 1) -> dict:
    return {
        "truncated": False, "checksum": "scene",
        "entities": [
            {"id": "hero-id", "name": "hero", "values": {
                "Position": {"pos": [hero_x, 0]}, "PlayerTag": {"slot": 0}}},
            {"id": "target-id", "name": "target", "values": {
                "Position": {"pos": [target_x, 0]}, "PlayerTag": {"slot": 1}}},
        ],
        "singletons": {"Score": {"kills": [score, 0, 0, 0, 0, 0, 0, 0]}},
    }


def task5_actions() -> list[dict]:
    return ([{"op": "save", "write": True}, {"op": "restart_host", "generation": 2},
             {"op": "sim_start"},
             {"op": "sim_input", "player": 1, "value": {"axis_x": 0, "axis_y": 0, "buttons": []}},
             {"op": "sim_input", "player": 0, "value": {"axis_x": 0, "axis_y": 0, "buttons": ["fire"]}},
             {"op": "sim_step", "ticks": 1},
             {"op": "sim_input", "player": 0, "value": {"axis_x": 0, "axis_y": 0, "buttons": []}}]
            + [{"op": "sim_step", "ticks": 1} for _ in range(49)]
            + [{"op": "sim_stop"}, {"op": "verify_replay", "replay": "modified50"}])


def task4_actions(*, include_undo_redo: bool = True) -> list[dict]:
    actions = ([{"op": "read", "what": "scene"}, {"op": "apply"}, {"op": "sim_start"},
                {"op": "sim_input", "player": 1, "value": {"axis_x": 0, "axis_y": 0, "buttons": []}},
                {"op": "sim_input", "player": 0, "value": {"axis_x": 0, "axis_y": 0, "buttons": ["fire"]}},
                {"op": "sim_step", "ticks": 1},
                {"op": "sim_input", "player": 0, "value": {"axis_x": 0, "axis_y": 0, "buttons": []}}]
               + [{"op": "sim_step", "ticks": 1} for _ in range(49)]
               + [{"op": "sim_stop"}, {"op": "verify_replay", "replay": "modified50"}])
    if include_undo_redo:
        actions += [{"op": "undo"}, {"op": "redo"}]
    return actions


class ProtocolTests(unittest.TestCase):
    def test_decode_rejects_malformed_and_non_object_frames(self):
        for line in ("{", "[]", "null", '"text"'):
            with self.subTest(line=line), self.assertRaises(ProtocolError):
                decode(line)

    def test_broker_protocol_rejects_escape_hatches_paths_and_solver_verdicts(self):
        invalid = [
            {"type": "request", "id": "1", "op": "erp_call", "args": {"method": "x"}},
            {"type": "request", "id": "2", "op": "sim_stop", "args": {"replay": "../../verdict.json"}},
            {"type": "request", "id": "3", "op": "verify_replay", "args": {"replay": "fresh50"}},
            {"type": "request", "id": "4", "op": "task_done", "args": {"verdict": "passed"}},
            {"type": "request", "id": "5", "op": "apply", "args": {"label": "x", "operations": [
                {"op": "write_file", "path": "trial.json", "value": "passed"}]}},
            {"type": "request", "id": "6", "op": "read", "args": {"what": []}},
            {"type": "request", "id": "7", "op": "sim_stop", "args": {"replay": {}}},
            {"type": "request", "id": "8", "op": "verify_replay", "args": {"replay": []}},
            {"type": "request", "id": "9", "op": "apply", "args": {"label": "x", "operations": [
                {"op": "spawn_player", "name": [], "slot": 0, "pos": [0, 0]}]}},
            {"type": "request", "id": "10", "op": "sim_input", "args": {"player": [], "value": {
                "axis_x": 0, "axis_y": 0, "buttons": []}}},
            {"type": "request", "id": "11", "op": "sim_input", "args": {"player": 0, "value": {
                "axis_x": True, "axis_y": 0, "buttons": []}}},
            {"type": "request", "id": "12", "op": "apply", "args": {"label": "x", "operations": [
                {"op": "spawn_player", "name": "hero", "slot": 0, "pos": [10**400, 0]}]}},
        ]
        for message in invalid:
            with self.subTest(message=message), self.assertRaises(ProtocolError):
                validate_request(message)

    def test_manifest_freezes_supported_local_mock_identity_and_rejects_mutations(self):
        base = manifest()
        self.assertIs(validate_manifest(base), base)
        mutations = [
            ("attempt_count", 0), ("seed", 43), ("solver", {**base["solver"], "name": "provider"}),
            ("provenance", {k: v for k, v in base["provenance"].items() if k != "source_sha256"}),
        ]
        for key, value in mutations:
            changed = copy.deepcopy(base)
            changed[key] = value
            with self.subTest(key=key), self.assertRaises(ProtocolError):
                validate_manifest(changed)

    def test_solver_frame_reader_is_bounded_and_rejects_oversize(self):
        class Process:
            stdout = io.StringIO("x" * (runner.MAX_PROTOCOL_FRAME_BYTES + 1) + "\n")

        process = Process()
        instance = object.__new__(TrialRunner)
        instance._start_reader(process)
        self.assertEqual(process._trial_lines.maxsize, runner.PROTOCOL_QUEUE_DEPTH)
        with self.assertRaises(ProtocolError):
            instance._read_solver(process, 1)


class PersistenceTests(unittest.TestCase):
    def _runner(self, root: Path, value: dict | None = None, *, resume: bool = False) -> TrialRunner:
        return TrialRunner(root, value or manifest(), resume=resume, repo=root.parent,
                           host_bin=Path("host"), cli_bin=Path("cli"))

    def test_resume_rejects_transcript_tampering_and_manifest_changes(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "trial"
            value = manifest()
            trial = self._runner(root, value)
            trial.event({"kind": "checkpoint", "task": 1})
            trial.state["status"] = "interrupted"
            trial.persist()
            changed = copy.deepcopy(value)
            changed["attempt_count"] = 2
            with self.assertRaisesRegex(TrialError, "manifest"):
                self._runner(root, changed, resume=True)
            events = root / "events.jsonl"
            events.write_text(events.read_text(encoding="utf-8").replace("checkpoint", "tampered"), encoding="utf-8")
            with self.assertRaisesRegex(TrialError, "hash mismatch"):
                self._runner(root, value, resume=True)

    def test_source_fixture_and_binary_provenance_mutations_fail_before_host_start(self):
        for field in ("source_sha256", "dirty_status", "fixture_sha256", "host_sha256", "cli_sha256"):
            with self.subTest(field=field), tempfile.TemporaryDirectory() as temp:
                root = Path(temp) / "trial"
                value = manifest()
                trial = self._runner(root, value)
                current = dict(value["provenance"])
                current[field] = "changed"
                attempt = trial._create_attempt(1)
                with patch("tools.arena_trial.runner.frozen_provenance", return_value=current):
                    with self.assertRaisesRegex(TrialError, "frozen provenance changed"):
                        trial._prepare_attempt(attempt, fresh=True)

    def test_resume_continues_remaining_attempts_and_keeps_denominator_budget_and_time(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "trial"
            value = manifest()
            value["attempt_count"] = 2
            trial = self._runner(root, value)
            trial.state["counts"]["infrastructure_interrupted"] = 1
            trial.state["counts"]["solver_requests"] = 7
            trial.solver_request_count = 7

            def interrupt(_attempt, *, fresh):
                self.assertTrue(fresh)
                _attempt.update(next_task=2, active_seconds=10.0, wall_seconds=3.0,
                                tasks=[{"number": 1, "status": "passed", "active_seconds": 10.0}],
                                task_events=[{"task": 2, "status": "infrastructure_interrupted"}],
                                infra_intervals=[{"started_at": runner.utc_now(), "task": 2}])
                _attempt["status"] = "interrupted"

            trial.run_attempt = interrupt  # type: ignore[method-assign]
            trial.state["status"] = "running"
            trial.persist()
            self.assertEqual(trial.run(), 2)
            saved = json.loads((root / "trial.json").read_text(encoding="utf-8"))
            self.assertEqual(saved["status"], "interrupted")
            self.assertEqual(saved["attempts"][0]["number"], 1)
            self.assertEqual(saved["counts"]["solver_requests"], 7)
            initial_counts = saved["counts"]
            self.assertEqual(initial_counts["scheduled_task_cells"], 10)
            self.assertEqual(initial_counts["completed_task_cells"], 1)
            self.assertEqual(initial_counts["passed_task_cells"], 1)
            self.assertEqual(initial_counts["failed_task_cells"], 0)
            self.assertEqual(initial_counts["infrastructure_interrupted_task_cells"], 1)
            self.assertEqual(initial_counts["unstarted_task_cells"], 8)
            self.assertEqual(initial_counts["unresolved_task_cells"], 9)

            resumed = self._runner(root, value, resume=True)
            self.assertEqual(resumed.solver_request_count, 7)

            def finish(attempt, *, fresh):
                if fresh:
                    self.assertEqual(attempt["number"], 2)
                    start_task = 1
                else:
                    self.assertEqual(attempt["number"], 1)
                    start_task = 2
                    attempt["active_seconds"] += 5.0
                attempt["tasks"].extend({"number": n, "status": "passed", "active_seconds": 1.0}
                                        for n in range(start_task, 6))
                attempt["next_task"] = 6
                attempt["status"] = "passed"
                attempt["wall_seconds"] = attempt.get("wall_seconds", 0.0) + 2.0

            resumed.run_attempt = finish  # type: ignore[method-assign]
            self.assertEqual(resumed.run(), 0)
            state = json.loads((root / "trial.json").read_text(encoding="utf-8"))
            self.assertEqual([a["number"] for a in state["attempts"]], [1, 2])
            self.assertEqual(state["attempts"][0]["active_seconds"], 15.0)
            counts = state["counts"]
            self.assertEqual(counts["scheduled_task_cells"], 10)
            self.assertEqual(counts["completed_task_cells"], 10)
            self.assertEqual(counts["passed_task_cells"], 10)
            self.assertEqual(counts["failed_task_cells"], 0)
            self.assertEqual(counts["infrastructure_interrupted_task_cells"], 1)
            self.assertEqual(counts["resumed_task_cells"], 1)
            self.assertEqual(counts["unstarted_task_cells"], 0)

    def test_running_trial_is_not_claimed_resumable_after_unrecorded_crash(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "trial"
            trial = self._runner(root)
            trial.state["status"] = "running"
            trial.persist()
            with self.assertRaisesRegex(TrialError, "interrupted"):
                self._runner(root, resume=True)

    def test_downtime_starts_after_cleanup_and_does_not_double_count_tail(self):
        attempt = {
            "wall_seconds": 4.0,
            "infra_intervals": [{
                "started_at": "2026-10-03T00:00:01+00:00",  # interruption detected
                "segment_ended_at": "2026-10-03T00:00:11+00:00",  # cleanup completed
                "downtime_started_at": "2026-10-03T00:00:11+00:00",
            }],
        }
        seconds = TrialRunner._record_infrastructure_downtime(
            attempt, "2026-10-03T00:00:21+00:00")
        self.assertEqual(seconds, 10.0)
        self.assertEqual(attempt["wall_seconds"], 14.0)
        interval = attempt["infra_intervals"][0]
        self.assertEqual(interval["started_at"], "2026-10-03T00:00:01+00:00")
        self.assertEqual(interval["finished_at"], "2026-10-03T00:00:21+00:00")

    def test_ledger_rejects_duplicate_terminal_cells_instead_of_clamping(self):
        with tempfile.TemporaryDirectory() as temp:
            trial = self._runner(Path(temp) / "trial")
            trial.state["attempts"] = [{
                "number": 1, "status": "interrupted",
                "tasks": [{"number": 1, "status": "passed"},
                          {"number": 1, "status": "passed"}],
            }]
            with self.assertRaisesRegex(TrialError, "duplicate or out-of-range task cells"):
                trial._refresh_ledger()


class OracleTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.trial = TrialRunner(self.root / "trial", manifest(), repo=self.root,
                                 host_bin=Path("host"), cli_bin=Path("cli"))

    def broker(self, task: int, *, actions: list[dict], facts: dict, trace: list[dict] | None = None,
               recording_checked: int = 51, history_entries: list[dict] | None = None,
               doc_checksum: str = "modified", scene_responses: list[dict] | None = None,
               mode: str = "edit") -> Broker:
        attempt = {"number": 1, "next_task": task, "facts": facts,
                   "task_action_segments": {str(task): []}, "tasks": []}
        benchmark = FakeBenchmark(self.root, recording_checked=recording_checked,
                                  history_entries=history_entries, doc_checksum=doc_checksum,
                                  scene_responses=scene_responses, mode=mode)
        broker = Broker(self.trial, attempt, benchmark)
        broker.actions.extend(actions)
        broker.trace = trace or []
        broker.baseline_checksum = facts.get("baseline_checksum", "baseline")
        broker.modified_checksum = facts.get("modified_checksum", "modified")
        return broker

    def test_task1_requires_positive_integer_receipt_matching_only_history_entry(self):
        actions = [{"op": "read", "what": name} for name in ("status", "schema", "scene")]
        actions.extend([{"op": "apply", "task": 1, "accepted": True},
                        {"op": "read", "what": "history"}])
        verdict = self.trial.judge_task(
            self.broker(1, actions=actions, facts={"task1_apply_receipt": 17},
                        history_entries=[{"id": 17}], doc_checksum="baseline",
                        scene_responses=[scene_query(-300, 300, score=0)]), 1)
        self.assertEqual(verdict["status"], "passed")
        for receipt, history in (("17", [{"id": 17}]), (True, [{"id": True}]),
                                 (0, [{"id": 0}]), (18, [{"id": 17}]),
                                 (17, [{"id": 17}, {"id": 17}])):
            with self.subTest(receipt=receipt, history=history), self.assertRaises(BenchmarkFailure):
                self.trial.judge_task(
                    self.broker(1, actions=actions, facts={"task1_apply_receipt": receipt},
                                history_entries=history, doc_checksum="baseline",
                                scene_responses=[scene_query(-300, 300, score=0)]), 1)

    def test_task2_requires_live_snapshots_and_stopped_authoring_state(self):
        actions = [
            {"op": "sim_start"},
            {"op": "sim_input", "value": {"axis_x": 1, "axis_y": 0, "buttons": []}},
            {"op": "sim_step", "ticks": 10},
            {"op": "sim_input", "value": {"axis_x": 0, "axis_y": 0, "buttons": []}},
            {"op": "sim_step", "ticks": 5},
            {"op": "sim_stop"},
        ]
        live = scene_facts(scene_query(-240, 300, score=0))
        facts = {"baseline_checksum": "baseline", "task2_live_snapshots": [
            {"tick": 10, "scene": live}, {"tick": 15, "scene": live}]}
        verdict = self.trial.judge_task(
            self.broker(2, actions=actions, facts=copy.deepcopy(facts),
                        scene_responses=[scene_query(-300, 300, score=0)], doc_checksum="baseline", mode="edit"), 2)
        self.assertEqual(verdict["facts"]["movement_final"], [-240, 0])
        self.assertTrue(verdict["facts"]["document_unchanged"])

        invalid_facts = []
        missing = copy.deepcopy(facts)
        missing["task2_live_snapshots"] = []
        invalid_facts.append(missing)
        wrong_tick = copy.deepcopy(facts)
        wrong_tick["task2_live_snapshots"][1]["tick"] = 14
        invalid_facts.append(wrong_tick)
        wrong_live_position = copy.deepcopy(facts)
        wrong_live_position["task2_live_snapshots"][0]["scene"] = scene_facts(scene_query(-300, 300, score=0))
        invalid_facts.append(wrong_live_position)
        for wrong in invalid_facts:
            with self.subTest(snapshots=wrong["task2_live_snapshots"]), self.assertRaises(BenchmarkFailure):
                self.trial.judge_task(
                    self.broker(2, actions=actions, facts=wrong,
                                scene_responses=[scene_query(-300, 300, score=0)], doc_checksum="baseline", mode="edit"), 2)
        for checksum, mode, stopped_scene in (
            ("changed", "edit", scene_query(-300, 300, score=0)),
            ("baseline", "playing", scene_query(-300, 300, score=0)),
            ("baseline", "edit", scene_query(-240, 300, score=0)),
        ):
            with self.subTest(checksum=checksum, mode=mode), self.assertRaises(BenchmarkFailure):
                self.trial.judge_task(
                    self.broker(2, actions=actions, facts=copy.deepcopy(facts),
                                scene_responses=[stopped_scene], doc_checksum=checksum, mode=mode), 2)

    def test_shot_tasks_reject_movement_wrong_player_and_wrong_buttons(self):
        task3 = [
            {"op": "sim_start"},
            {"op": "sim_input", "player": 1, "value": {"axis_x": 0, "axis_y": 0, "buttons": []}},
            {"op": "sim_input", "player": 0, "value": {"axis_x": 0, "axis_y": 0, "buttons": ["fire"]}},
            {"op": "sim_step", "ticks": 1},
            {"op": "sim_input", "player": 0, "value": {"axis_x": 0, "axis_y": 0, "buttons": []}},
            {"op": "sim_step", "ticks": 19}, {"op": "sim_stop", "replay": "baseline20"},
            {"op": "verify_replay", "replay": "baseline20"},
        ]
        clean = {3: task3, 4: task4_actions(), 5: task5_actions()}
        mutations = (
            lambda items: items[0].update(value={"axis_x": 1, "axis_y": 0, "buttons": []}),
            lambda items: items[1].update(player=1),
            lambda items: items[1].update(value={"axis_x": 0, "axis_y": 0, "buttons": []}),
        )
        for task, original in clean.items():
            facts = {}
            trace = [{"tick": tick, "checksum": f"0x{tick:016x}"} for tick in range(51)]
            if task in (3, 5):
                replay = self.root / ("baseline20.orrp" if task == 3 else "modified50.orrp")
                replay.write_bytes(b"fixture")
                facts = {"modified_checksum": "modified", "task4_trace": trace,
                         f"task{task}_live_final": {"tick": 20 if task == 3 else 50,
                             "scene": scene_facts(scene_query(-300, 300 if task == 3 else 900, score=1))}}
                good = self.broker(task, actions=original, facts=copy.deepcopy(facts), trace=trace,
                                   recording_checked=21 if task == 3 else 51)
                self.assertEqual(self.trial.judge_task(good, task)["status"], "passed")
            for mutate in mutations:
                bad = copy.deepcopy(original)
                inputs = [item for item in bad if item["op"] == "sim_input"]
                mutate(inputs)
                with self.subTest(task=task, inputs=inputs), self.assertRaisesRegex(
                        BenchmarkFailure, f"task {task} must keep player 1 neutral"):
                    self.trial.judge_task(self.broker(task, actions=bad, facts=copy.deepcopy(facts), trace=trace), task)

    def test_task4_requires_exact_undo_redo_actions_and_intermediate_state(self):
        facts = {
            "baseline_checksum": "baseline", "modified_checksum": "modified",
            "task4_undo_seen": True, "task4_redo_seen": True,
            "task4_undo_checksum": "baseline", "task4_redo_checksum": "modified",
            "task4_undo_scene": {"players": [{"slot": 0, "pos": [-300, 0]}, {"slot": 1, "pos": [300, 0]}],
                                 "entity_count": 2, "kills": [0] * 8, "bullets": 0, "out_of_bounds": 0},
            "task4_redo_scene": {"players": [{"slot": 0, "pos": [-300, 0]}, {"slot": 1, "pos": [900, 0]}],
                                 "entity_count": 2, "kills": [0] * 8, "bullets": 0, "out_of_bounds": 0},
            "task4_apply_accepted": True, "task4_apply_verify": {},
            "task4_live_final": {"tick": 50, "scene": scene_facts(scene_query(-300, 900, score=1))},
        }
        with self.assertRaises(BenchmarkFailure):
            self.trial.judge_task(self.broker(4, actions=task4_actions(include_undo_redo=False), facts=facts), 4)

        replay = self.root / "modified50.orrp"
        replay.write_bytes(b"fixture")
        trace = [{"tick": tick, "checksum": f"0x{tick:016x}"} for tick in range(51)]
        passing_facts = {
            **facts,
            "task4_apply_verify": {
                "checks": {"passed": True}, "ticks": 20, "debug_commands_replayed": 0,
                "metrics": [
                    {"name": "score_0", "base": {"end": 1}, "candidate": {"end": 0}},
                    {"name": "players", "candidate": {"end": 2}},
                    {"name": "bullets", "candidate": {"end": 0}},
                    {"name": "score_1", "candidate": {"end": 0}},
                    {"name": "out_of_bounds", "candidate": {"end": 0}},
                ],
                "recording": {"checked": 21, "mismatches": 0, "first_mismatch": None},
            },
        }
        positive = self.broker(4, actions=task4_actions(), facts=passing_facts, trace=trace,
                               doc_checksum="modified")
        positive.baseline_checksum = "baseline"
        positive.modified_checksum = "modified"
        self.assertEqual(self.trial.judge_task(positive, 4)["status"], "passed")

        for mutation in (
            lambda items: items[0].update(value={"axis_x": 1, "axis_y": 0, "buttons": []}),
            lambda items: items[1].update(player=1),
            lambda items: items[1].update(value={"axis_x": 0, "axis_y": 0, "buttons": []}),
        ):
            bad_actions = task4_actions()
            input_actions = [item for item in bad_actions if item["op"] == "sim_input"]
            mutation(input_actions)
            with self.subTest(actions=input_actions), self.assertRaises(BenchmarkFailure):
                self.trial.judge_task(self.broker(4, actions=bad_actions, facts=passing_facts,
                                                  trace=trace, doc_checksum="modified"), 4)

        actions = task4_actions()
        for wrong in (
            {**facts, "task4_undo_checksum": "wrong"},
            {**facts, "task4_redo_checksum": "wrong"},
            {**facts, "task4_undo_scene": {**facts["task4_undo_scene"], "players": [
                {"slot": 0, "pos": [-300, 0]}, {"slot": 1, "pos": [900, 0]}]}},
        ):
            with self.subTest(facts=wrong), self.assertRaises(BenchmarkFailure):
                self.trial.judge_task(self.broker(4, actions=actions, facts=wrong), 4)

    def test_task5_rejects_incomplete_original_replay_checksum_coverage(self):
        replay = self.root / "modified50.orrp"
        replay.write_bytes(b"fixture")
        trace = [{"tick": tick, "checksum": f"0x{tick:016x}"} for tick in range(51)]
        facts = {"modified_checksum": "modified", "task4_trace": trace,
                 "task5_live_final": {"tick": 50, "scene": scene_facts(scene_query(-300, 900, score=1))}}
        broker = self.broker(5, actions=task5_actions(), facts=facts, trace=trace, recording_checked=50)
        with self.assertRaisesRegex(BenchmarkFailure, "original replay"):
            self.trial.judge_task(broker, 5)


class CleanupTests(unittest.TestCase):
    def test_live_process_is_distinguished_from_a_reused_windows_pid(self):
        from .integration import process_alive
        self.assertTrue(process_alive(os.getpid(), runner.utc_now()))
        if os.name == "nt":
            self.assertFalse(process_alive(os.getpid(), "1970-01-01T00:00:00+00:00"))

    def test_solver_timeout_cleanup_is_bounded_and_records_exit(self):
        kwargs = {"start_new_session": True} if os.name != "nt" else {
            "creationflags": subprocess.CREATE_NEW_PROCESS_GROUP}
        process = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"], **kwargs)
        process._trial_job = None
        started = time.monotonic()
        result = TrialRunner._stop_solver(process)
        self.assertLess(time.monotonic() - started, 5.0)
        self.assertEqual(result["pid"], process.pid)
        self.assertIsNotNone(result["exit_code"])
        self.assertIsNotNone(process.poll())


if __name__ == "__main__":
    unittest.main()
