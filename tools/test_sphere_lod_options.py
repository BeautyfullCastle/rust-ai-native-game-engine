import json
import pathlib
import subprocess
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]


class SphereOptions(unittest.TestCase):
    def options(self, backend, env, platform="win32"):
        code = "const {sphereOptions}=require('./tools/webtransport/sphere_lod_options.cjs');console.log(JSON.stringify(sphereOptions(...JSON.parse(process.argv[1]))));"
        return subprocess.run(["node", "-e", code, json.dumps([backend, env, platform])], cwd=ROOT, capture_output=True, text=True)

    def test_default_swiftshader_contract(self):
        result = self.options("webgpu", {})
        self.assertEqual(result.returncode, 0, result.stderr)
        value = json.loads(result.stdout)
        self.assertEqual(value["adapter"], "swiftshader")
        self.assertTrue(value["headless"])
        self.assertIn("--use-webgpu-adapter=swiftshader", value["args"])

    def test_hardware_headed_and_headless(self):
        for headed in ["0", "1"]:
            value = json.loads(self.options("webgpu", {"ORR_SPHERE_WEBGPU_ADAPTER": "hardware", "ORR_SPHERE_HEADED": headed}).stdout)
            self.assertEqual(value["args"], ["--use-angle=d3d11"])
            self.assertEqual(value["headless"], headed == "0")
            self.assertEqual(value["adapter"], "hardware")

    def test_invalid_and_security_options_fail(self):
        for flag in ["--no-sandbox", "--disable-gpu-sandbox", "--disable-web-security", "--ignore-certificate-errors", "--disable-features=SitePerProcess", "--single-process", "--use-webgpu-adapter=swiftshader"]:
            self.assertNotEqual(self.options("webgpu", {"CHROMIUM_FLAGS": flag}).returncode, 0, flag)
        for env in [{"ORR_SPHERE_WEBGPU_ADAPTER": "other"}, {"ORR_SPHERE_HEADED": "yes"}]:
            self.assertNotEqual(self.options("webgpu", env).returncode, 0)

    def test_launch_sandbox_and_assertions_preserved(self):
        source = (ROOT / "tools/webtransport/sphere_lod.cjs").read_text()
        self.assertIn("options.chromiumSandbox = true", source)
        self.assertIn("options.headless = launch.headless", source)
        self.assertIn("adapterRequested: launch.adapter", source)
        self.assertIn('console.log("ADAPTER "', source)
        self.assertIn("hardware requested but fallback/software adapter selected", source)
        for assertion in ["navigator.gpu.requestAdapter()", "result.result.actual_backend !== expected", "!result.result.drawn", "required ? 1 : 2"]:
            self.assertIn(assertion, source)


if __name__ == "__main__":
    unittest.main()
