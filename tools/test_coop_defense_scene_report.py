"""Actual bounded CLI acceptance; supply the freshly built example via env."""
import json
import os
from pathlib import Path
import subprocess
import unittest

EMPTY = b"schema: orr.scene/1\nentities: {}\n"
LIMIT = 64 * 1024


class SceneReportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        value = os.environ.get("ORR_SCENE_REPORT_BIN")
        if not value or not Path(value).is_file():
            raise RuntimeError("ORR_SCENE_REPORT_BIN must name the freshly built example")
        cls.binary = str(Path(value).resolve())

    def invoke(self, data=EMPTY, *args):
        return subprocess.run([self.binary, *(args or ("--stdin",))], input=data,
                              capture_output=True, timeout=10, check=False)

    def result(self, data):
        run = self.invoke(data)
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertEqual(run.stderr, b"")
        return json.loads(run.stdout)

    def test_bounded_authored_simulation(self):
        args = ("--stdin", "--simulate", "1")
        first = self.invoke(EMPTY, *args)
        self.assertEqual(first.returncode, 0, first.stderr)
        self.assertEqual(first.stdout, self.invoke(EMPTY, *args).stdout)
        value = json.loads(first.stdout)
        smoke = value["simulation"]
        self.assertEqual((smoke["ticks_executed"], smoke["tick_budget"]), (1, 1))
        self.assertEqual(smoke["outcome"], "playing")
        self.assertTrue(smoke["budget_exhausted"])
        self.assertEqual(smoke["same_input_replay"], "exact")
        self.assertNotEqual(value["initial_checksum"], smoke["final_checksum"])
        custom = EMPTY + b"singletons: {CoopDefense::Scenario: {starting_gold: 0, reactor_hp: 1}}\n"
        changed = self.invoke(custom, *args)
        self.assertEqual(changed.returncode, 0, changed.stderr)
        self.assertNotEqual(smoke["final_checksum"], json.loads(changed.stdout)["simulation"]["final_checksum"])

    def test_authored_short_campaign_victory_stops_exactly(self):
        path = Path(__file__).resolve().parents[1] / "examples/static-game-starter/scenes/stormglass-short.scene.yaml"
        source = path.read_bytes()
        result = self.invoke(source, "--stdin", "--simulate", "36000")
        self.assertEqual(result.returncode, 0, result.stderr)
        smoke = json.loads(result.stdout)["simulation"]
        self.assertEqual(smoke["outcome"], "victory")
        self.assertEqual(smoke["wave"], 3)
        self.assertFalse(smoke["budget_exhausted"])
        self.assertLess(smoke["ticks_executed"], 36000)
        exact = self.invoke(source, "--stdin", "--simulate", str(smoke["ticks_executed"]))
        self.assertEqual(exact.returncode, 0, exact.stderr)
        exact_smoke = json.loads(exact.stdout)["simulation"]
        self.assertEqual(exact_smoke["outcome"], "victory")
        self.assertFalse(exact_smoke["budget_exhausted"])
        self.assertEqual(exact_smoke["final_checksum"], smoke["final_checksum"])

    def test_terminal_defeat_stops_and_is_not_a_tool_error(self):
        source = EMPTY + b"singletons: {CoopDefense::Scenario: {starting_gold: 0, reactor_hp: 1}}\n"
        result = self.invoke(source, "--stdin", "--simulate", "36000")
        self.assertEqual(result.returncode, 0, result.stderr)
        smoke = json.loads(result.stdout)["simulation"]
        self.assertEqual(smoke["outcome"], "defeat")
        self.assertFalse(smoke["budget_exhausted"])
        self.assertGreater(smoke["ticks_executed"], 1)
        self.assertLess(smoke["ticks_executed"], 36000)
        self.assertEqual(smoke["reactor_hp"], 0)
        self.assertEqual(smoke["same_input_replay"], "exact")

    def test_simulation_usage_is_bounded_and_private(self):
        for value in ("0", "36001", "9999999999999", "-1", "+1", "secret"):
            result = self.invoke(b"private invalid source", "--stdin", "--simulate", value)
            self.assertEqual(result.returncode, 2)
            self.assertEqual(result.stdout, b"")
            self.assertEqual(json.loads(result.stderr), {"error": {"kind": "usage", "exit_code": 2}})
        result = self.invoke(EMPTY, "--stdin", "--simulate", "1", "extra")
        self.assertEqual(result.returncode, 2)

    def test_defaults_and_repeatability(self):
        first = self.invoke()
        self.assertEqual(first.returncode, 0, first.stderr)
        self.assertEqual(first.stdout, self.invoke().stdout)
        value = json.loads(first.stdout)
        self.assertEqual(value["schema"], "orr.coop.scene-report/1")
        self.assertIs(value["admitted"], True)
        self.assertEqual((value["initial_tick"], value["tick_rate"]), (0, 60))
        self.assertRegex(value["initial_checksum"], r"^0x[0-9a-f]{16}$")
        self.assertRegex(value["build_id"], r"^0x[0-9a-f]{16}$")
        cfg = value["config"]
        self.assertEqual((cfg["starting_gold"], cfg["reactor_hp"], cfg["last_wave"]), (150, 30, 10))
        self.assertEqual(cfg["pair_counts"], [10, 12, 14, 16, 18, 20, 22, 24, 26, 28])
        self.assertEqual(cfg["opening_boss"], [0, 0, 0, 0, 1, 0, 0, 0, 0, 1])
        self.assertEqual(value["source_bytes"], len(EMPTY))

    def test_authored_fields_and_inactive_waves_are_retained(self):
        source = EMPTY + b"""singletons:
  CoopDefense::Scenario: {starting_gold: 250, reactor_hp: 75}
  CoopDefense::Campaign: {last_wave: 3}
  CoopDefense::WavePacing: {spawn_interval_ticks: 70}
  CoopDefense::WaveCounts: {pair_counts: [5,6,7,8,9,10,11,12,13,14]}
  CoopDefense::EnemyHealth: {multiplier: 2}
  CoopDefense::WaveComposition:
    regular_kinds: [[3,0,2,1],[2,3,1,0],[0,2,3,1],[0,1,2,3],[1,2,3,0],[2,3,0,1],[3,0,1,2],[0,1,2,3],[1,2,3,0],[2,3,0,1]]
    opening_boss: [1,0,1,0,0,0,0,0,0,0]
"""
        value = self.result(source)
        cfg = value["config"]
        self.assertEqual((cfg["starting_gold"], cfg["reactor_hp"], cfg["last_wave"], cfg["spawn_interval_ticks"], cfg["enemy_health_multiplier"]), (250, 75, 3, 70, 2))
        self.assertEqual(cfg["pair_counts"], list(range(5, 15)))
        self.assertEqual(cfg["regular_kinds"][0], [3, 0, 2, 1])
        self.assertEqual(cfg["regular_kinds"][9], [2, 3, 0, 1])
        self.assertEqual(cfg["opening_boss"], [1, 0, 1, 0, 0, 0, 0, 0, 0, 0])
        self.assertNotEqual(value["initial_checksum"], self.result(EMPTY)["initial_checksum"])

    def test_all_shipped_authored_fixtures_are_admitted(self):
        root = Path(__file__).resolve().parents[1]
        fixtures = list((root / "examples/static-game-starter/scenes").glob("stormglass-*.scene.yaml"))
        self.assertGreaterEqual(len(fixtures), 5)
        for path in fixtures:
            with self.subTest(fixture=path.name):
                self.assertIs(self.result(path.read_bytes())["admitted"], True)

    def test_strict_admission_and_private_diagnostics(self):
        invalid = [b"", b"private-token", EMPTY + b"entities: {}\n", EMPTY + b"---\n" + EMPTY,
                   EMPTY + b"singletons: {CoopDefense::Scenario: {reactor_hp: 75}}\n",
                   EMPTY + b"singletons: {CoopDefense::Campaign: {last_wave: 11}}\n",
                   EMPTY + b"singletons: {CoopDefense::Run: {private_token: 1}}\n",
                   b"schema: orr.scene/1\nentities: {e_00000001: {name: private-token}}\n"]
        for data in invalid:
            with self.subTest(data_length=len(data)):
                run = self.invoke(data)
                self.assertEqual(run.returncode, 1)
                self.assertEqual(run.stdout, b"")
                self.assertEqual(json.loads(run.stderr), {"error": {"kind": "invalid_scene", "exit_code": 1}})
                self.assertNotIn(b"private", run.stderr)

    def test_utf8_and_exact_byte_limit(self):
        prefix = EMPTY + "#한글".encode()
        maximum = prefix + b"x" * (LIMIT - len(prefix))
        self.assertEqual(self.result(maximum)["source_bytes"], LIMIT)
        for data, kind in [(maximum + b"x", "source_too_large"), (b"\xff", "invalid_utf8")]:
            run = self.invoke(data)
            self.assertEqual(run.returncode, 1)
            self.assertEqual(run.stdout, b"")
            self.assertEqual(json.loads(run.stderr)["error"]["kind"], kind)

    def test_help_and_usage(self):
        help_run = self.invoke(b"private-token", "--help")
        self.assertEqual(help_run.returncode, 0)
        self.assertIn(b"64 KiB", help_run.stdout)
        self.assertNotIn(b"private-token", help_run.stdout)
        for args in [[], ["--file", "scene.yaml"], ["--stdin", "--stdin"], ["--help", "--stdin"]]:
            run = subprocess.run([self.binary, *args], input=b"", capture_output=True, timeout=10)
            self.assertEqual(run.returncode, 2)
            self.assertEqual(run.stdout, b"")
            self.assertEqual(json.loads(run.stderr), {"error": {"kind": "usage", "exit_code": 2}})


if __name__ == "__main__":
    unittest.main()
