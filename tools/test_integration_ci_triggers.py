"""Fast trigger-contract checks. No compiler or hosted CI is invoked."""
from pathlib import Path
import ast
import textwrap
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]
TRIGGER = ('on:\n  pull_request:\n'
           '    branches: [main, su/engine-correctness-hardening]\n'
           '    types: [ready_for_review]\n'
           '  workflow_dispatch:\n')


def check(text):
    match = re.search(r'^on:\n(?:[ \t].*\n|\n)*', text, re.M)
    assert match and match.group() == TRIGGER
    assert 'pull_request_target:' not in text


def check_room_usage_selectors(text):
    # Keep the new checks in the existing batch, with no separate job or workflow.
    required = (
        'for room_features in ("room-project", "room-animated-models"):',
        '"--features", room_features, "--example", "room_package_usage", "--",',
        'helpers["validate_tests"](output, "tests", 12 if os.name == "nt" else 13)',
        '"--test", "room_package_usage_consumer", "--",',
        'helpers["CASE"].findall(output) != [(consumer, "ok")]',
        '"room-usage-static-lint"', '"room-usage-animated-lint"',
        '"room-usage-default-off-lint"', '"room-usage-consumer-lint"',
        '"--test", "checked_remove"', '"--test", "restore_removed"',
        '"package_manager::removal::tests"', '"package_manager_panel::tests"',
        '"--test", "package_removal_ui"',
    )
    for selector in required:
        assert selector in text, selector
    assert text.count('  package-removal:\n') == 1
    assert 'os: [ubuntu-22.04, windows-latest]' in text
    # Parse each inline Python block without executing Rust or hosted actions.
    for block in re.findall(r'        shell: python \{0\}\n        run: \|\n((?:          .*\n|\n)+)', text):
        ast.parse(textwrap.dedent(block))


def check_grid_selectors(text):
    required = (
        "examples = ('png_sequence_package', 'png_grid_package')",
        "expected = {'png_sequence_package': 20, 'png_sprite_package': 16, 'png_grid_package': 11}",
        "'--features', 'project-create', *test_selectors",
        "os.environ['ORR_PNG_GRID_PACKAGE_BIN']",
        "'--features', 'sprites', '--test', 'png_grid_package_consumer'",
        "'--ignored', '--exact', consumer",
        "helpers['CASE'].findall(output) != [(consumer, 'ok')]",
        "'--test', 'png_sequence_package_gpu'",
        "'--test', 'png_sequence_canvas_gpu'",
    )
    # The refusal message is dynamically assembled for both examples.
    for selector in required:
        assert selector in text, selector
    assert "example + ' requires Linux and --features project-create'" in text
    assert text.count('  png-sequence-package:\n') == 1
    for block in re.findall(r'        shell: python \{0\}\n        run: \|\n((?:          .*\n|\n)+)', text):
        ast.parse(textwrap.dedent(block))


def grid_validator(text):
    block = re.search(r'        shell: python \{0\}\n        run: \|\n((?:          .*\n|\n)+)', text).group(1)
    nodes = ast.parse(textwrap.dedent(block)).body
    start = next(i for i, node in enumerate(nodes) if isinstance(node, ast.Assign)
                 and isinstance(node.targets[0], ast.Name) and node.targets[0].id == 'expected')
    end = next(i for i in range(start, len(nodes)) if isinstance(nodes[i], ast.For))
    # Run only the real batch census, never Cargo or other workflow side effects.
    kept = [node for node in nodes[start:end + 1] if not (isinstance(node, ast.Assign)
            and isinstance(node.targets[0], ast.Name) and node.targets[0].id in ('test_selectors', 'output'))]
    return compile(ast.Module(body=kept, type_ignores=[]), '<grid-census>', 'exec')


class IntegrationTriggers(unittest.TestCase):
    def test_all_build_test_workflows(self):
        paths = list((ROOT / '.github/workflows').glob('*.yml'))
        self.assertEqual(len(paths), 47)
        for path in paths:
            with self.subTest(workflow=path.name):
                check(path.read_text())

    def test_package_removal_keeps_saved_usage_in_existing_batch(self):
        text = (ROOT / ".github/workflows/package-removal.yml").read_text()
        check_room_usage_selectors(text)

    def test_reject_missing_room_usage_route_or_wrong_platform_count(self):
        text = (ROOT / ".github/workflows/package-removal.yml").read_text()
        for before, after in (
            ('for room_features in ("room-project", "room-animated-models"):',
             'for room_features in ("room-project",):'),
            ('"--test", "room_package_usage_consumer", "--",',
             '"--test", "unrelated_consumer", "--",'),
            ('12 if os.name == "nt" else 13', '13'),
            ('"--test", "restore_removed"', '"--test", "unrelated"'),
        ):
            with self.subTest(selector=before), self.assertRaises(AssertionError):
                check_room_usage_selectors(text.replace(before, after))

    def test_grid_keeps_existing_batch_and_mandatory_consumer(self):
        text = (ROOT / '.github/workflows/png-sequence-package.yml').read_text()
        check_grid_selectors(text)
        for before, after in (
            ("'png_grid_package': 11", "'png_grid_package': 0"),
            ("'--ignored', '--exact', consumer", "'--exact', consumer"),
            ("os.environ['ORR_PNG_GRID_PACKAGE_BIN']", "os.environ['WRONG_BIN']"),
            ("'--test', 'png_sequence_canvas_gpu'", "'--test', 'unrelated'"),
        ):
            with self.subTest(selector=before), self.assertRaises(AssertionError):
                check_grid_selectors(text.replace(before, after))

    def test_grid_batch_census_rejects_missing_failed_or_zero_tests(self):
        import runpy
        helpers = runpy.run_path(str(ROOT / 'tools/check-authoring-controls.py'))
        text = (ROOT / '.github/workflows/png-sequence-package.yml').read_text()
        program = grid_validator(text)
        def harness(name, count):
            return (f'     Running unittests examples/{name}.rs (target/debug/examples/{name}-hash)\n'
                    f'\nrunning {count} tests\n' + ''.join(f'test case_{i} ... ok\n' for i in range(count))
                    + f'test result: ok. {count} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n')
        parts = [harness(name, count) for name, count in (
            ('png_grid_package', 11), ('png_sequence_package', 20), ('png_sprite_package', 16))]
        good = ''.join(parts)
        exec(program, {'output': good, 'helpers': helpers, 're': re})
        for bad in (''.join(parts[1:]), good.replace('running 11 tests', 'running 0 tests'),
                    good.replace('test case_0 ... ok', 'test case_0 ... FAILED', 1),
                    good + parts[0], good.replace('examples/png_grid_package.rs', 'examples/unrelated.rs')):
            with self.subTest(output=bad[:80]), self.assertRaises(RuntimeError):
                exec(program, {'output': bad, 'helpers': helpers, 're': re})

    def grid_gpu_validator(self):
        import hashlib
        import json
        import runpy
        text = (ROOT / '.github/workflows/png-sequence-package.yml').read_text()
        step = text.split('      - name: Required grid installed pixels and authoritative parity\n')[1].split('      - name: Retain logs')[0]
        self.assertIn("ORR_REQUIRE_GPU: '1'", step)
        self.assertIn("WGPU_BACKEND: vulkan", step)
        self.assertIn("'--test', 'png_grid_package_gpu', '--', '--ignored', '--exact', test", step)
        self.assertIn("os.environ['ORR_GRID_PIXEL_EVIDENCE']", step)
        self.assertIn("os.environ['ORR_PNG_GRID_PACKAGE_BIN']", step)
        self.assertIn("require(digest(binary) == producer_hash", step)
        block = step.split('        run: |\n')[1]
        nodes = ast.parse(textwrap.dedent(block)).body
        selected = [node for node in nodes if isinstance(node, ast.FunctionDef) or
                    (isinstance(node, ast.Assign) and isinstance(node.targets[0], ast.Name)
                     and node.targets[0].id in ('test', 'marker'))]
        env = {'hashlib': hashlib, 'json': json, 're': re,
               'helpers': runpy.run_path(str(ROOT / 'tools/check-authoring-controls.py'))}
        exec(compile(ast.Module(body=selected, type_ignores=[]), '<grid-gpu-receipt>', 'exec'), env)
        return env

    def test_grid_gpu_receipt_and_log_fail_closed(self):
        import copy
        import hashlib
        import json
        import tempfile
        env = self.grid_gpu_validator()
        test, marker, validate = env['test'], env['marker'], env['validate_grid']
        output = (f'running 1 test\ntest {test} ... {marker}\nok\n'
                  'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n')
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            captures = []
            for tick, cell, wrong in [(0,0,False),(5,0,False),(6,1,False),(11,1,False),(12,0,False),(13,2,False),(18,2,False),(19,3,False),(24,3,False),(25,3,False),(26,0,False),(19,2,True)]:
                name = f"grid-{'wrong-order' if wrong else 'good'}-tick-{tick}.png"
                # Only a digest/header validator fixture, not a GPU readback.
                data = b'\x89PNG\r\n\x1a\n' + b'\0\0\0\rIHDR' + (64).to_bytes(4, 'big') * 2 + bytes([tick])
                (root / name).write_bytes(data)
                captures.append(dict(file=name, tick=tick, cell=cell, wrong_order=wrong, sha256=hashlib.sha256(data).hexdigest()))
            receipt = dict(schema=1, test=test, capture_count=12, parity_checks=54, source_hidden=True,
                           installed_bytes_preserved=True, backend='Vulkan', adapter='fixture software adapter',
                           device_type='Cpu', is_software=True, producer_sha256='a'*64, source_png_sha256='b'*64, captures=captures)
            (root / 'partial-receipt.json').write_text('{}')
            for name in ('grid-good-cli.log', 'grid-wrong-order-cli.log'):
                (root / name).write_text('synthetic CLI log fixture\n')
            def write(value):
                (root / 'receipt.json').write_text(json.dumps(value))
            write(receipt)
            accepted = validate(output, root, 'a'*64)
            self.assertEqual(len(accepted['captures']), 12)
            self.assertEqual(accepted['receipt_sha256'], hashlib.sha256((root/'receipt.json').read_bytes()).hexdigest())
            for key, value in [('capture_count',11),('parity_checks',53),('source_hidden',False),('installed_bytes_preserved',False),('backend','Gl'),('adapter',''),('device_type',None),('is_software',1),('producer_sha256','c'*64),('source_png_sha256','bad')]:
                bad = copy.deepcopy(receipt); bad[key] = value; write(bad)
                with self.subTest(field=key), self.assertRaises(RuntimeError): validate(output, root, 'a'*64)
            for field, value in [('file','../escape.png'),('tick',True),('cell',3),('wrong_order',1),('sha256','d'*64)]:
                bad=copy.deepcopy(receipt);bad['captures'][0][field]=value;write(bad)
                with self.subTest(capture=field), self.assertRaises(RuntimeError): validate(output, root, 'a'*64)
            write(receipt)
            for badlog in (output.replace('running 1 test','running 0 tests'), output.replace(marker,''), output+'\nSKIP\n', output.replace(test,'other'), output.replace('1 passed','0 passed'), output+marker):
                with self.assertRaises(RuntimeError): validate(badlog, root, 'a'*64)
            extra=root/'extra.png';extra.write_bytes(b'bad')
            with self.assertRaises(RuntimeError): validate(output, root, 'a'*64)
            extra.unlink()
            first=root/captures[0]['file'];original=first.read_bytes();first.write_bytes(b'changed')
            with self.assertRaises(RuntimeError): validate(output, root, 'a'*64)
            first.write_bytes(original)
            (root/'receipt.json').write_text('{"schema":1,"schema":1}')
            with self.assertRaises(RuntimeError): validate(output, root, 'a'*64)

    def reparent_validator(self):
        import hashlib
        import json
        import runpy
        import struct
        text = (ROOT / '.github/workflows/collect-ui-screen-preview.yml').read_text()
        check(text)
        for selector in ('("orr_sample", "collect_ui::tests", 14)', '"--test", "collect_ui_reparent"',
                         '"--skip", "app::collect_ui_preview::"', '"--features", "collect-ui,project-create"',
                         '"--lib", reparent_test, "--", "--ignored", "--exact"',
                         'os.environ["ORR_COLLECT_UI_REPARENT_EVIDENCE"]',
                         'json.loads(manifest.read_text()) != validate_reparent(output, captures)'):
            self.assertIn(selector, text)
        blocks = re.findall(r'        shell: python \{0\}\n        run: \|\n((?:          .*\n|\n)+)', text)
        parsed = [ast.parse(textwrap.dedent(block)) for block in blocks]
        selected = [node for tree in parsed for node in tree.body if
                    isinstance(node, ast.FunctionDef) and node.name == 'validate_reparent' or
                    isinstance(node, ast.Assign) and isinstance(node.targets[0], ast.Name)
                    and node.targets[0].id == 'reparent_test']
        env = dict(hashlib=hashlib, json=json, struct=struct,
                   h=runpy.run_path(str(ROOT / 'tools/check-authoring-controls.py')))
        exec(compile(ast.Module(body=selected, type_ignores=[]), '<reparent-receipt>', 'exec'), env)
        return env

    def test_reparent_gpu_receipt_fails_closed(self):
        import copy
        import json
        import tempfile
        env = self.reparent_validator()
        test, validate = env['reparent_test'], env['validate_reparent']
        output = (f'running 1 test\ntest {test} ... ok\n'
                  'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n')
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for name in ('reparent-initial','reparent-staged-clipping','reparent-cancelled','reparent-saved-reopened'):
                for suffix, size in (('canvas',(640,360)),('editor',(1800,1200))):
                    # Synthetic header fixture only, not GPU success evidence.
                    (root/f'{name}-{suffix}.png').write_bytes(b'\x89PNG\r\n\x1a\n'+b'\0\0\0\rIHDR'+b''.join(v.to_bytes(4,'big') for v in size))
            receipt = dict(schema=1, production_preview_pixel_parity_cases=4, window_pixels=[1800,1200],
                           native_os_window=False, separate_runtime_process=False,
                           redo='not integrated; PR182 prerequisite', adapter='llvmpipe Cpu Vulkan')
            for flag in ('actual_editor_app','software_vulkan','stage_cancel_apply_undo_save_reopen',
                         'saved_production_collect_ui_play','same_parent_no_history','new_parent_clipping_visible',
                         'selection_document_focus_escape_viewport_screen_mode_close_title_x_epoch_play_panel_cancel',
                         'saved_authored_play_button_click'):
                receipt[flag] = True
            path = root/'reparent-acceptance.json'
            path.write_text(json.dumps(receipt))
            hashes = validate(output,root)
            self.assertEqual(len(hashes),9)
            for key, value in [('schema',True),('production_preview_pixel_parity_cases',3),('native_os_window',True),
                               ('separate_runtime_process',True),('adapter','SwiftShader Cpu Vulkan'),('window_pixels',[1,1]),
                               ('saved_authored_play_button_click',False),('redo','implemented')]:
                bad = copy.deepcopy(receipt); bad[key]=value; path.write_text(json.dumps(bad))
                with self.subTest(key=key), self.assertRaises(RuntimeError): validate(output,root)
            missing = copy.deepcopy(receipt); del missing['software_vulkan']; path.write_text(json.dumps(missing))
            with self.assertRaises(RuntimeError): validate(output,root)
            path.write_text(json.dumps(receipt))
            for bad in (output.replace('1 passed','0 passed'),output.replace(test,'other'),output+'SKIP\n',output+output):
                with self.assertRaises(RuntimeError): validate(bad,root)
            extra=root/'extra.png';extra.write_bytes(b'bad')
            with self.assertRaises(RuntimeError): validate(output,root)
            extra.unlink()
            capture=root/'reparent-initial-canvas.png';raw=capture.read_bytes();capture.write_bytes(b'bad')
            with self.assertRaises(RuntimeError): validate(output,root)
            capture.unlink()
            with self.assertRaises(RuntimeError): validate(output,root)
            capture.write_bytes(raw[:16]+(1).to_bytes(4,'big')+raw[20:])
            with self.assertRaises(RuntimeError): validate(output,root)
            capture.write_bytes(raw+b'changed')
            self.assertNotEqual(hashes,validate(output,root))
            path.write_text('{"schema":1,"schema":1}')
            with self.assertRaises(RuntimeError): validate(output,root)

    def test_core_preview_batch_exact_census(self):
        import runpy
        text = (ROOT / '.github/workflows/collect-ui-screen-preview.yml').read_text()
        block = re.findall(r'        shell: python \{0\}\n        run: \|\n((?:          .*\n|\n)+)', text)[0]
        nodes = ast.parse(textwrap.dedent(block)).body
        core = next(n for n in nodes if isinstance(n,ast.Assign) and isinstance(n.targets[0],ast.Name) and n.targets[0].id=='core_names')
        start = next(i for i,n in enumerate(nodes) if isinstance(n,ast.Assign) and isinstance(n.targets[0],ast.Name) and n.targets[0].id=='summaries')
        program=compile(ast.Module(body=[core]+nodes[start:start+3],type_ignores=[]),'<core-census>','exec')
        env={'h':runpy.run_path(str(ROOT/'tools/check-authoring-controls.py'))}
        names=ast.literal_eval(core.value)
        good=''.join(f'test {name} ... ok\n' for name in sorted(names))+'test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n'
        exec(program,dict(env,output=good))
        for bad in (good.replace('18 passed','17 passed'),good.replace(' ... ok',' ... ignored',1),good.replace(sorted(names)[0],sorted(names)[1]),good+good):
            with self.assertRaises(RuntimeError): exec(program,dict(env,output=bad))

    def test_reject_automatic_or_incomplete_triggers(self):
        for bad in [
            TRIGGER.replace('ready_for_review', 'opened, synchronize'),
            TRIGGER.replace('    types: [ready_for_review]\n', ''),
            TRIGGER.replace('branches: [main, su/engine-correctness-hardening]', "branches: ['**']"),
            TRIGGER.replace('  workflow_dispatch:\n', ''),
            TRIGGER + '  push:\n',
            TRIGGER + '  schedule:\n    - cron: "0 * * * *"\n',
        ]:
            with self.subTest(trigger=bad), self.assertRaises(AssertionError):
                check(bad)


if __name__ == '__main__':
    unittest.main()
