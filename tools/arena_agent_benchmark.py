#!/usr/bin/env python3
"""Deterministic, scripted Arena workflow benchmark (Python standard library only).

This is an integration/reproducibility baseline, NOT an autonomous-agent benchmark.
Use --self-test to test the harness without Cargo, binaries, or a running host.
"""

from __future__ import annotations

import argparse
from contextlib import contextmanager
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import time
import traceback
import unittest


ROOT = Path(__file__).resolve().parents[1]
MASK64 = (1 << 64) - 1
NEUTRAL = {"axis_x": 0, "axis_y": 0, "buttons": []}
RIGHT = {"axis_x": 1, "axis_y": 0, "buttons": []}
FIRE = {"axis_x": 0, "axis_y": 0, "buttons": ["fire"]}


class BenchmarkFailure(RuntimeError):
    """A command, oracle, or provenance requirement failed."""


def require(condition, message):
    if not condition:
        raise BenchmarkFailure(message)


def utc_now():
    return datetime.now(timezone.utc).isoformat()


def sha256(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_json(path, data):
    Path(path).write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def mix64(value):
    value ^= value >> 30
    value = (value * 0xBF58476D1CE4E5B9) & MASK64
    value ^= value >> 27
    value = (value * 0x94D049BB133111EB) & MASK64
    return value ^ (value >> 31)


def build_hash_of(build_id, patch_generation=0):
    """Mirror orr_sim::build_hash_of; a build id is not a replay build hash."""
    if build_id == 0:
        return 0
    mixed = mix64(patch_generation)
    rotated = ((mixed << 17) | (mixed >> 47)) & MASK64
    return mix64(build_id ^ rotated)


def replay_header(path):
    """Read only the uncompressed ORRP prefix; engine verifies the body."""
    data = Path(path).read_bytes()
    require(data[:4] == b"ORRP", "replay has no ORRP magic")
    offset = 4

    def take(length):
        nonlocal offset
        require(offset + length <= len(data), "truncated replay header")
        result = data[offset:offset + length]
        offset += length
        return result

    def number(fmt):
        return struct.unpack(fmt, take(struct.calcsize(fmt)))[0]

    version = number("<I")
    require(version in (1, 2, 3), f"unsupported replay version {version}")
    game_id = take(number("<I")).decode("utf-8")
    header = {
        "format_version": version,
        "game_id": game_id,
        "build_hash": f"0x{number('<Q'):016x}",
        "seed": number("<Q"),
        "player_count": number("<B"),
        "tick_rate": number("<I"),
        "input_size": number("<I"),
        "compressed_bytes": number("<I"),
    }
    require(header["compressed_bytes"] > 0, "empty replay body")
    require(offset + header["compressed_bytes"] == len(data), "replay body length does not match file")
    return header


def validate_replay_identity(header, engine):
    require(header["game_id"] == engine["game"], "replay game identity differs from host")
    build_id = int(engine["build_id"], 0)
    require(build_id != 0, "host build identity is the untracked zero wildcard")
    require(int(header["build_hash"], 0) == build_hash_of(build_id), "replay build hash differs from host build id at generation 0")
    require(header["seed"] == 42, "replay seed differs from 42")
    require(header["player_count"] == 2, "replay player count differs from 2")
    require(header["tick_rate"] == 60, "replay tick rate differs from 60")
    require(header["input_size"] == 24, "ArenaInput ABI differs from the benchmark contract")


def component(values, name):
    found = [value for key, value in values.items() if key.rsplit("::", 1)[-1] == name]
    require(len(found) == 1, f"expected one {name}, found {len(found)}")
    return found[0]


def scene_facts(scene):
    require(scene.get("truncated") is False, "scene query was truncated")
    players = []
    bullets = 0
    out_of_bounds = 0
    for entity in scene["entities"]:
        values = entity["values"]
        names = {key.rsplit("::", 1)[-1] for key in values}
        if "Bullet" in names:
            bullets += 1
        if "Position" in names:
            pos = component(values, "Position")["pos"]
            if any(abs(value) > 2000 for value in pos):
                out_of_bounds += 1
        if "PlayerTag" in names:
            players.append({"id": entity["id"], "name": entity.get("name"),
                            "slot": component(values, "PlayerTag")["slot"],
                            "pos": component(values, "Position")["pos"]})
    return {"players": sorted(players, key=lambda player: player["slot"]),
            "bullets": bullets, "kills": component(scene["singletons"], "Score")["kills"],
            "out_of_bounds": out_of_bounds, "checksum": scene["checksum"], "entity_count": len(scene["entities"])}


def assert_scene(facts, hero_x, target_x, score=0, bullets=0):
    require(len(facts["players"]) == 2, "expected exactly two players")
    require(facts["entity_count"] == 2 + bullets, "unexpected extra entities")
    require([p["slot"] for p in facts["players"]] == [0, 1], "player slots must be unique 0 and 1")
    require([p["pos"] for p in facts["players"]] == [[hero_x, 0], [target_x, 0]], "unexpected player positions")
    require(facts["kills"] == [score, 0, 0, 0, 0, 0, 0, 0], "unexpected Score singleton")
    require(facts["bullets"] == bullets, f"expected {bullets} bullets")
    require(facts["out_of_bounds"] == 0, "entities escaped Arena bounds")


def check_report(report, ticks, metrics, recording=False):
    require(report["checks"]["passed"] is True, "verification checks did not explicitly pass")
    require(report["ticks"] == ticks, "verifier did not execute the expected tick count")
    require(report["debug_commands_replayed"] == 0, "replay contains debug scene edits")
    by_name = {entry["name"]: entry for entry in report["metrics"]}
    for key, expected in metrics.items():
        side, name = key.split(":", 1) if ":" in key else ("candidate", key)
        require(by_name[name][side]["end"] == expected, f"unexpected final metric {key}")
    if recording:
        record = report["recording"]
        require(record["checked"] == ticks + 1, "recording must check every tick, including tick 0")
        require(record["mismatches"] == 0 and record["first_mismatch"] is None, "recording checksum mismatch")


class Benchmark:
    def __init__(self, args):
        self.args = args
        self.repo = args.repo.resolve()
        self.output = args.output.resolve()
        self.output.mkdir(parents=True, exist_ok=False)
        self.log_path = self.output / "commands.jsonl"
        self.stage = "setup"
        self.playing = False
        self.process = None
        self.host_log = None
        self.host_record = None
        self.host_generation = 0
        self.started = time.perf_counter()
        self.report = {
            "format": "orr.arena-scripted-workflow/1", "kind": "scripted_integration_baseline",
            "started_at": utc_now(), "status": "running", "tasks": [], "hosts": [], "errors": [],
            "counts": {"commands": 0, "command_failures": 0, "readiness_probe_failures": 0,
                       "command_retries": 0, "unexpected_host_restarts": 0, "planned_host_restarts": 0,
                       "proposal_rejections": 0, "proposal_check_failures": 0, "human_interventions": args.human_interventions},
            "intervention_note": args.intervention_note,
            "llm": {"used": False, "model": None, "tokens": None, "cost": None},
            "settings": {"seed": 42, "players": 2, "tick_rate": 60, "manual_ticks": True},
            "limitations": ["Not an autonomous-agent trial or agent superiority result",
                            "Default engine build id does not uniquely identify source commits",
                            "Oracle checks identity locally; it is not a claim of global engine enforcement",
                            "Warm/fresh-target timing is separate from task timing and downloads may be cached"],
        }
        self.environment = os.environ.copy()
        for name in ("ORR_ERP", "ORR_ERP_URL", "ORR_ERP_TOKEN"):
            self.environment.pop(name, None)

    def log(self, entry):
        with self.log_path.open("a", encoding="utf-8") as stream:
            stream.write(json.dumps(entry, sort_keys=True) + "\n")

    def command(self, argv, *, probe=False, timeout=None, env=None, allow_failure=False):
        argv = [str(arg) for arg in argv]
        start = time.perf_counter()
        entry = {"kind": "command", "stage": self.stage, "argv": argv, "cwd": str(self.repo),
                 "started_at": utc_now(), "readiness_probe": probe}
        self.report["counts"]["commands"] += 1
        try:
            result = subprocess.run(argv, cwd=self.repo, env=env or self.environment,
                                    capture_output=True, text=True, encoding="utf-8", errors="replace",
                                    timeout=timeout or self.args.command_timeout, check=False)
            entry.update(returncode=result.returncode, stdout=result.stdout, stderr=result.stderr)
        except (OSError, subprocess.TimeoutExpired) as exc:
            entry.update(returncode=None, error=str(exc))
            result = None
        entry["elapsed_seconds"] = time.perf_counter() - start
        self.log(entry)
        if result is None or result.returncode != 0:
            self.report["counts"]["command_failures"] += 1
            if probe:
                self.report["counts"]["readiness_probe_failures"] += 1
            if not allow_failure:
                raise BenchmarkFailure(f"command failed: {argv!r}; see commands.jsonl")
        return result

    def cli(self, *args, probe=False):
        # No raw ERP escape hatch. In play mode only timeline/input/read calls are allowed.
        allowed = {"status", "scene", "get", "schema", "agents-md", "history", "verify", "apply", "save", "undo", "redo", "sim"}
        require(args and args[0] in allowed, "command is outside benchmark policy")
        if self.playing:
            require(args[0] not in {"apply", "save", "undo", "redo"}, "authoring is forbidden during gameplay")
        if args[0] == "sim":
            require(args[1] in {"start", "stop", "step", "state", "input"}, "only manual timeline and ordinary input are permitted")
        result = self.command([self.cli_bin, "--erp", self.url, "--json", *args], probe=probe, allow_failure=True)
        response = None
        if result is not None:
            try:
                response = json.loads(result.stdout)
            except json.JSONDecodeError as exc:
                if result.returncode == 0:
                    raise BenchmarkFailure(f"CLI did not return JSON: {args!r}") from exc
        if args[0] == "apply" and isinstance(response, dict):
            if response.get("outcome") == "rejected":
                self.report["counts"]["proposal_rejections"] += 1
            if response.get("verify", {}).get("checks", {}).get("passed") is False:
                self.report["counts"]["proposal_check_failures"] += 1
        if result is None or result.returncode:
            if probe:
                return None
            raise BenchmarkFailure(f"CLI command failed: {args!r}; see commands.jsonl")
        return response

    def artifact(self, name, value):
        write_json(self.output / name, value)
        return value

    @contextmanager
    def task(self, number, name):
        self.stage = f"task_{number}"
        record = {"number": number, "name": name, "started_at": utc_now(), "status": "running"}
        self.report["tasks"].append(record)
        start = time.perf_counter()
        try:
            yield record
        except BaseException:
            record["status"] = "failed"
            raise
        else:
            record["status"] = "passed"
        finally:
            record["elapsed_seconds"] = time.perf_counter() - start
            write_json(self.output / "run.json", self.report)

    def provenance(self):
        metadata = {}
        for name, argv in {
            "commit": ["git", "rev-parse", "HEAD"], "head_tree": ["git", "rev-parse", "HEAD^{tree}"],
            "status": ["git", "status", "--porcelain=v1"],
            "tracked_and_untracked_paths": ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        }.items():
            result = self.command(argv)
            metadata[name] = result.stdout.strip() if name != "tracked_and_untracked_paths" else result.stdout
        digest = hashlib.sha256()
        file_count = 0
        for name in sorted(set(metadata.pop("tracked_and_untracked_paths").split("\0")) - {""}):
            path = self.repo / name
            # Artifacts and build outputs never contribute to the source fingerprint.
            if path == self.output or self.output in path.parents:
                continue
            digest.update(name.encode("utf-8") + b"\0")
            if path.is_file():
                digest.update(bytes.fromhex(sha256(path)))
            else:
                digest.update(b"<missing>")
            file_count += 1
        metadata.update(worktree_sha256=digest.hexdigest(), files=file_count)
        return metadata

    def prepare(self):
        self.report["source"] = self.provenance()
        diff = self.command(["git", "diff", "--binary", "HEAD"]).stdout
        (self.output / "source.diff").write_text(diff, encoding="utf-8")
        self.report["source"]["diff_sha256"] = sha256(self.output / "source.diff")
        toolchain = {}
        for tool in ("rustc", "cargo"):
            result = self.command([tool, "--version"], allow_failure=True)
            toolchain[tool] = result.stdout.strip() if result and result.returncode == 0 else None
        toolchain["python"] = sys.version
        self.report["toolchain"] = toolchain
        self.report["platform"] = {"system": platform.system(), "release": platform.release(),
                                   "machine": platform.machine(), "python_implementation": platform.python_implementation()}
        build = {"mode": self.args.build_mode, "profile": "release", "status": "prebuilt_unverified",
                 "elapsed_seconds": None, "downloads_included_if_any": True}
        self.report["build"] = build
        target = Path(self.environment.get("CARGO_TARGET_DIR", self.repo / "target"))
        if not target.is_absolute():
            target = self.repo / target
        if self.args.build_mode != "prebuilt":
            require(not self.args.host_bin and not self.args.cli_bin, "explicit binaries cannot be combined with a build")
            env = self.environment.copy()
            if self.args.build_mode == "fresh-target":
                target = self.output / "build-target"
                env["CARGO_TARGET_DIR"] = str(target)
            build["target_dir"] = str(target)
            build["target_existed_before"] = target.exists()
            start = time.perf_counter()
            result = self.command(["cargo", "build", "--locked", "--release", "-p", "orr_remote", "--bin", "orr_remote_host",
                                   "-p", "orr_cli", "--bin", "orr"], timeout=self.args.build_timeout, env=env, allow_failure=True)
            build["elapsed_seconds"] = time.perf_counter() - start
            build["status"] = "passed" if result and result.returncode == 0 else "failed"
            require(build["status"] == "passed", "build failed; see commands.jsonl")
        self.host_bin = (self.args.host_bin or target / "release/orr_remote_host").resolve()
        self.cli_bin = (self.args.cli_bin or target / "release/orr").resolve()
        for path in (self.host_bin, self.cli_bin):
            require(path.is_file() and os.access(path, os.X_OK), f"missing executable: {path}")
        self.report["binaries"] = {"host": {"path": str(self.host_bin), "sha256": sha256(self.host_bin)},
                                   "cli": {"path": str(self.cli_bin), "sha256": sha256(self.cli_bin)}}
        fixture = self.repo / "scenes/arena_blank.scene.yaml"
        require(fixture.is_file(), f"missing blank fixture: {fixture}")
        self.scene_path = self.output / "working.scene.yaml"
        shutil.copyfile(fixture, self.scene_path)
        shutil.copyfile(fixture, self.output / "blank.scene.yaml")
        self.report["fixture"] = {"path": str(fixture), "sha256": sha256(fixture)}
        self.artifact("input-manifest.json", {
            "movement": [{"player": 0, "value": RIGHT, "ticks": 10}, {"player": 0, "value": NEUTRAL, "ticks": 5}],
            "shot20": [{"player": 0, "value": FIRE, "ticks": 1}, {"player": 0, "value": NEUTRAL, "ticks": 19}],
            "shot50": [{"player": 0, "value": FIRE, "ticks": 1}, {"player": 0, "value": NEUTRAL, "ticks": 49}],
            "player1": "neutral for every tick", "input_semantics": "whole object held until replaced"})

    def start_host(self):
        require(self.process is None, "host already started")
        self.host_generation += 1
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        self.url = f"ws://127.0.0.1:{port}"
        argv = [str(self.host_bin), "--game", "arena", "--scene", str(self.scene_path),
                "--bind", f"127.0.0.1:{port}", "--seed", "42", "--players", "2", "--tick-rate", "60", "--dev-no-auth"]
        start = time.perf_counter()
        self.host_log = (self.output / f"host-{self.host_generation}.log").open("w", encoding="utf-8")
        self.process = subprocess.Popen(argv, cwd=self.repo, env=self.environment, stdout=self.host_log, stderr=subprocess.STDOUT)
        self.host_record = {"generation": self.host_generation, "pid": self.process.pid, "argv": argv,
                            "started_at": utc_now(), "status": "starting", "readiness_seconds": None}
        self.report["hosts"].append(self.host_record)
        self.log({"kind": "host_spawn", "stage": self.stage, **self.host_record})
        deadline = start + self.args.readiness_timeout
        while time.perf_counter() < deadline:
            require(self.process.poll() is None, f"host exited before readiness; see host-{self.host_generation}.log")
            status = self.cli("status", probe=True)
            if status is not None:
                require(status["engine"]["game"] == "Arena", "host is not running Arena")
                require(status["state"]["mode"] == "edit", "host did not start in edit mode")
                require(status["state"]["player_count"] == 2 and status["state"]["tick_rate"] == 60, "host settings differ")
                schema = self.cli("schema", "--input")
                require(isinstance(schema.get("schema"), dict) and schema.get("value_format"), "input schema discovery failed")
                require(status["engine"]["verify"]["bot_available"] is False, "Arena must not silently use a bot")
                self.host_record.update(status="ready", readiness_seconds=time.perf_counter() - start,
                                        ready_status=status, input_schema=schema)
                self.artifact(f"host-{self.host_generation}-status.json", status)
                self.artifact(f"host-{self.host_generation}-input-schema.json", schema)
                return status
            time.sleep(0.1)
        raise BenchmarkFailure("host readiness timed out")

    def stop_host(self):
        if self.process is None:
            return
        process = self.process
        unexpected_exit = process.poll() is not None
        if not unexpected_exit:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
                self.host_record["forced_kill"] = True
        self.host_record.update(exit_code=process.returncode, stopped_at=utc_now(), unexpected_exit=unexpected_exit)
        self.log({"kind": "host_stop", "stage": self.stage, "pid": process.pid, "exit_code": process.returncode,
                  "unexpected_exit": unexpected_exit})
        self.host_log.close()
        self.process = None
        self.playing = False

    def snapshot(self, filename):
        return scene_facts(self.artifact(filename, self.cli("scene", "--components")))

    def state(self, expected_tick=None):
        result = self.cli("sim", "state")
        if expected_tick is not None:
            require(result["head_tick"] == expected_tick and result["playing"] is False, "manual tick state differs")
        return result

    def start_play(self, document_checksum):
        result = self.cli("sim", "start")
        self.playing = True
        require(result["mode"] == "play" and result["head_tick"] == 0 and result["playing"] is False, "play did not start paused at tick 0")
        require(result["doc_checksum"] == document_checksum, "play start changed authoring document")
        return result

    def input(self, value, player=0):
        result = self.cli("sim", "input", "--player", str(player), json.dumps(value, separators=(",", ":")))
        require(result.get("ok") is True, "input did not explicitly succeed")

    def step(self, ticks, expected_tick):
        result = self.cli("sim", "step", str(ticks))
        require(result["head_tick"] == expected_tick and result["playing"] is False, "manual step returned unexpected tick")
        return result

    def stop_play(self, replay=None):
        args = ["sim", "stop"]
        if replay:
            args += ["--replay-out", str(replay)]
        result = self.cli(*args)
        self.playing = False
        if replay:
            require(replay.is_file() and replay.stat().st_size > 0, "replay export is absent or empty")
            header = replay_header(replay)
            validate_replay_identity(header, self.engine)
            self.artifact(replay.name + ".header.json", header)
        return result

    def shot(self, ticks, document_checksum, target_x, label, every_tick=False):
        initial = self.start_play(document_checksum)
        assert_scene(self.snapshot(label + "-initial.json"), -300, target_x)
        trace = [{"tick": 0, "checksum": initial["checksum"]}]
        self.input(NEUTRAL, player=1)
        self.input(FIRE)
        first = self.step(1, 1)
        trace.append({"tick": 1, "checksum": first["checksum"]})
        self.input(NEUTRAL)
        for tick in range(2, ticks + 1) if every_tick else [ticks]:
            state = self.step(1 if every_tick else ticks - 1, tick)
            require(state["doc_checksum"] == document_checksum, "input changed authoring document")
            trace.append({"tick": tick, "checksum": state["checksum"]})
        assert_scene(self.snapshot(label + "-final.json"), -300, target_x, score=1)
        replay = self.output / (label + ".orrp")
        self.stop_play(replay)
        require(self.state()["doc_checksum"] == document_checksum, "stop changed authoring document")
        self.artifact(label + "-checksums.json", trace)
        return replay, trace

    def verify(self, replay, ticks, name):
        args = ["verify", "--replay", str(replay)]
        for rule in ("recording_matches", "no_divergence", "players.final == 2", "bullets.final == 0",
                     "score_0.final == 1", "score_1.final == 0", "out_of_bounds.final == 0"):
            args += ["--check", rule]
        report = self.artifact(name, self.cli(*args))
        check_report(report, ticks, {"players": 2, "bullets": 0, "score_0": 1, "score_1": 0, "out_of_bounds": 0}, recording=True)
        require(report["identical"] is True, "standalone verifier reruns diverged")
        return report

    def run(self):
        self.prepare()
        initial = self.start_host()
        self.engine = initial["engine"]
        self.artifact("schema-types.json", self.cli("schema", "--types"))
        guide = self.cli("agents-md")
        (self.output / "AGENTS.arena.md").write_text(guide["text"], encoding="utf-8")
        with self.task(1, "Create a guarded two-player scene") as task:
            blank = self.snapshot("task1-blank.json")
            require(blank["entity_count"] == 0 and blank["players"] == [] and blank["bullets"] == 0 and blank["kills"] == [0] * 8, "fixture is not blank with zero Score")
            require(self.cli("history")["entries"] == [], "blank host has pre-existing edit history")
            apply = self.cli("apply", "create two-player arena", "spawn", "--name", "hero", 'Position={"pos":[-300,0]}', 'PlayerTag={"slot":0}',
                             "spawn", "--name", "target", 'Position={"pos":[300,0]}', 'PlayerTag={"slot":1}', "--idle", "1",
                             "--check", "players.final == 2", "--check", "bullets.final == 0", "--check", "score_0.final == 0",
                             "--check", "score_1.final == 0", "--check", "out_of_bounds.final == 0")
            self.artifact("task1-apply.json", apply)
            require(apply["accepted"] is True and apply["outcome"] == "accepted", "guarded apply was not accepted")
            check_report(apply["verify"], 1, {"players": 2, "bullets": 0, "score_0": 0, "score_1": 0, "out_of_bounds": 0})
            baseline_facts = self.snapshot("task1-authored.json")
            assert_scene(baseline_facts, -300, 300)
            history = self.cli("history")["entries"]
            require(len(history) == 1 and history[0]["id"] == apply["history_id"], "creation must produce exactly one history entry")
            baseline_checksum = self.state()["doc_checksum"]
            saved = self.cli("save")
            (self.output / "baseline.scene.yaml").write_text(saved["text"], encoding="utf-8")
            target_id = baseline_facts["players"][1]["id"]
            task.update(document_checksum=baseline_checksum, player_ids=[p["id"] for p in baseline_facts["players"]])
        with self.task(2, "Move and release ordinary input") as task:
            self.start_play(baseline_checksum)
            self.input(RIGHT)
            self.step(10, 10)
            assert_scene(self.snapshot("task2-right10.json"), -240, 300)
            self.input(NEUTRAL)
            self.step(5, 15)
            assert_scene(self.snapshot("task2-neutral5.json"), -240, 300)
            self.stop_play()
            require(self.state()["doc_checksum"] == baseline_checksum, "movement/stop changed authoring document")
            task.update(final_position=[-240, 0], document_unchanged=True)
        with self.task(3, "Fire once and verify a replay") as task:
            replay20, _ = self.shot(20, baseline_checksum, 300, "baseline20")
            verified = self.verify(replay20, 20, "task3-verify.json")
            task.update(recording=verified["recording"], debug_commands_replayed=verified["debug_commands_replayed"])
        with self.task(4, "Verify an edit and undo exactly") as task:
            apply = self.cli("apply", "move target farther away", "set", target_id, "Position.pos=[900,0]", "--replay", str(replay20),
                             "--check", "base:score_0.final == 1", "--check", "score_0.final == 0", "--check", "players.final == 2",
                             "--check", "score_1.final == 0", "--check", "out_of_bounds.final == 0")
            self.artifact("task4-apply.json", apply)
            require(apply["accepted"] is True and apply["outcome"] == "accepted", "modified proposal was not guarded-accepted")
            check_report(apply["verify"], 20, {"base:score_0": 1, "score_0": 0, "players": 2, "score_1": 0, "out_of_bounds": 0}, recording=True)
            modified_checksum = self.state()["doc_checksum"]
            require(modified_checksum != baseline_checksum, "moving the target did not change document checksum")
            replay50, trace50 = self.shot(50, modified_checksum, 900, "modified50", every_tick=True)
            self.verify(replay50, 50, "task4-verify50.json")
            self.cli("undo")
            require(self.state()["doc_checksum"] == baseline_checksum, "undo did not restore baseline document checksum")
            assert_scene(self.snapshot("task4-undone.json"), -300, 300)
            self.cli("redo")
            require(self.state()["doc_checksum"] == modified_checksum, "redo did not restore modified document checksum")
            assert_scene(self.snapshot("task4-redone.json"), -300, 900)
            task.update(baseline_document_checksum=baseline_checksum, modified_document_checksum=modified_checksum,
                        undo_exact=True, redo_exact=True, replay20_sha256=sha256(replay20))
        with self.task(5, "Restart the host and reproduce every tick") as task:
            self.cli("save", "--write")
            shutil.copyfile(self.scene_path, self.output / "modified.scene.yaml")
            first_pid = self.process.pid
            self.stop_host()
            self.report["counts"]["planned_host_restarts"] += 1
            require(sha256(self.host_bin) == self.report["binaries"]["host"]["sha256"], "host binary changed during benchmark")
            fresh = self.start_host()
            require(fresh["engine"] == self.engine, "fresh host discovery identity differs")
            require(fresh["state"]["doc_checksum"] == modified_checksum, "saved scene failed exact fresh-process reload")
            require(self.cli("history")["entries"] == [], "fresh host retained undo history")
            assert_scene(self.snapshot("task5-reloaded.json"), -300, 900)
            _, fresh_trace = self.shot(50, modified_checksum, 900, "fresh50", every_tick=True)
            require(len(trace50) == 51 and [v["tick"] for v in trace50] == list(range(51)), "reference trace does not cover tick 0..50")
            require(fresh_trace == trace50, "fresh process differs at one or more ticks")
            # Verify the ORIGINAL recording, not just the fresh process's own recording.
            verified = self.verify(replay50, 50, "task5-original-replay-verify.json")
            task.update(original_pid=first_pid, fresh_pid=self.process.pid, checksum_ticks_matched=51,
                        recording=verified["recording"], debug_commands_replayed=verified["debug_commands_replayed"])
        self.stage = "audit"
        self.artifact("final-history.json", self.cli("history"))
        self.report["source_after"] = self.provenance()
        require(self.report["source_after"]["worktree_sha256"] == self.report["source"]["worktree_sha256"], "source tree changed during benchmark")
        for role, path in (("host", self.host_bin), ("cli", self.cli_bin)):
            require(sha256(path) == self.report["binaries"][role]["sha256"], f"{role} binary changed during benchmark")
        self.report["status"] = "passed"

    def finish(self):
        self.stop_host()
        self.report.update(finished_at=utc_now(), elapsed_seconds=time.perf_counter() - self.started)
        self.report["artifacts"] = []
        for directory, children, files in os.walk(self.output):
            children[:] = [name for name in children if name != "build-target"]
            for name in sorted(files):
                path = Path(directory) / name
                if path != self.output / "run.json":
                    self.report["artifacts"].append({"path": str(path.relative_to(self.output)),
                                                     "bytes": path.stat().st_size, "sha256": sha256(path)})
        write_json(self.output / "run.json", self.report)
        return 0 if self.report["status"] == "passed" else 1


class HarnessTests(unittest.TestCase):
    def test_hash_vectors_and_zero_wildcard(self):
        self.assertEqual(build_hash_of(0), 0)
        self.assertEqual(mix64(0), 0)
        self.assertEqual(mix64(1), 0x5692161D100B05E5)
        self.assertEqual(build_hash_of(1), mix64(1))
        self.assertNotEqual(build_hash_of(1, 1), build_hash_of(1))

    def test_header_parser_and_identity(self):
        game = b"Arena"
        blob = b"ORRP" + struct.pack("<II", 3, len(game)) + game
        blob += struct.pack("<QQBIII", build_hash_of(123), 42, 2, 60, 24, 1) + b"x"
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "test.orrp"
            path.write_bytes(blob)
            header = replay_header(path)
            validate_replay_identity(header, {"game": "Arena", "build_id": "0x7b"})
            with self.assertRaises(BenchmarkFailure):
                validate_replay_identity(header, {"game": "Arena", "build_id": "0x7c"})
            for invalid in (b"", blob[:-1], blob + b"x", b"NOPE" + blob[4:]):
                path.write_bytes(invalid)
                with self.assertRaises(BenchmarkFailure):
                    replay_header(path)

    def test_nonzero_apply_counts_actual_rejection_before_failure(self):
        for outcome, rejected in (("rejected", 1), ("kept", 0)):
            benchmark = Benchmark.__new__(Benchmark)
            benchmark.playing = False
            benchmark.cli_bin = Path("orr")
            benchmark.url = "ws://127.0.0.1:7777"
            benchmark.report = {"counts": {"proposal_rejections": 0, "proposal_check_failures": 0}}
            payload = {"accepted": False, "outcome": outcome, "verify": {"checks": {"passed": False}}}
            benchmark.command = lambda *a, **k: subprocess.CompletedProcess([], 4, json.dumps(payload), "")
            with self.assertRaises(BenchmarkFailure):
                benchmark.cli("apply", "test")
            self.assertEqual(benchmark.report["counts"]["proposal_rejections"], rejected)
            self.assertEqual(benchmark.report["counts"]["proposal_check_failures"], 1)

    def test_gameplay_policy_blocks_authoring_and_direct_commands(self):
        benchmark = Benchmark.__new__(Benchmark)
        benchmark.playing = True
        for args in (("apply", "cheat"), ("undo",), ("sim", "command"), ("set", "hero", "Score=1")):
            with self.assertRaises(BenchmarkFailure):
                benchmark.cli(*args)

    def test_scene_oracle_rejects_shortcuts(self):
        facts = {"players": [{"slot": 0, "pos": [-300, 0]}, {"slot": 1, "pos": [300, 0]}],
                 "kills": [0] * 8, "bullets": 0, "out_of_bounds": 0, "entity_count": 2}
        assert_scene(facts, -300, 300)
        for key, value in (("kills", [1] + [0] * 7), ("bullets", 1), ("out_of_bounds", 1), ("players", facts["players"] * 2)):
            with self.assertRaises(BenchmarkFailure):
                assert_scene({**facts, key: value}, -300, 300)

    def test_recording_oracle_requires_every_tick_and_no_debug(self):
        report = {"checks": {"passed": True}, "ticks": 50, "debug_commands_replayed": 0,
                  "metrics": [{"name": "score_0", "candidate": {"end": 1}}],
                  "recording": {"checked": 51, "mismatches": 0, "first_mismatch": None}}
        check_report(report, 50, {"score_0": 1}, recording=True)
        for bad in ({**report, "debug_commands_replayed": 1},
                    {**report, "recording": {"checked": 50, "mismatches": 0, "first_mismatch": None}},
                    {**report, "checks": {"passed": None}},
                    {**report, "recording": {"checked": 51, "mismatches": 1, "first_mismatch": 20}}):
            with self.assertRaises(BenchmarkFailure):
                check_report(bad, 50, {"score_0": 1}, recording=True)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true", help="run Python oracle tests only; do not build or start a host")
    parser.add_argument("--repo", type=Path, default=ROOT)
    parser.add_argument("--output", type=Path, help="new artifact directory (must not already exist)")
    parser.add_argument("--build-mode", choices=("prebuilt", "warm", "fresh-target"), default="prebuilt")
    parser.add_argument("--host-bin", type=Path, help="prebuilt host executable")
    parser.add_argument("--cli-bin", type=Path, help="prebuilt orr executable")
    parser.add_argument("--readiness-timeout", type=float, default=20)
    parser.add_argument("--command-timeout", type=float, default=30)
    parser.add_argument("--build-timeout", type=float, default=1800)
    parser.add_argument("--human-interventions", type=int, default=0, help="disclosed interventions before this attempt")
    parser.add_argument("--intervention-note", default="none reported")
    args = parser.parse_args(argv)
    if args.self_test:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(HarnessTests)
        return 0 if unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful() else 1
    if args.output is None:
        stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
        args.output = args.repo / "target/arena-agent-benchmark" / stamp
    if args.human_interventions < 0 or min(args.readiness_timeout, args.command_timeout, args.build_timeout) <= 0:
        parser.error("timeouts must be positive and human intervention count non-negative")
    try:
        benchmark = Benchmark(args)
    except OSError as exc:
        print(f"benchmark setup failed: {exc}", file=sys.stderr)
        return 1
    try:
        benchmark.run()
    except (Exception, KeyboardInterrupt) as exc:
        benchmark.report["status"] = "failed"
        benchmark.report["errors"].append({"stage": benchmark.stage, "type": type(exc).__name__, "message": str(exc)})
        (benchmark.output / "failure.txt").write_text(traceback.format_exc(), encoding="utf-8")
    finally:
        code = benchmark.finish()
    print(f"{benchmark.report['status'].upper()}: {benchmark.output / 'run.json'}")
    return code


if __name__ == "__main__":
    sys.exit(main())
