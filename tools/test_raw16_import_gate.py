#!/usr/bin/env python3
"""CPU-only negative controls for the RAW16 acceptance gate. No Cargo or GPU."""
import errno
import json
import os
from pathlib import Path
import signal
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
import zlib

sys.dont_write_bytecode = True
import raw16_import_gate as gate

ROOT = Path(__file__).resolve().parents[1]
NATIVE_HEADER = '''  native:
    name: native (${{ matrix.name }})
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: ubuntu-latest
            name: ubuntu-latest-x86_64
          - os: windows-latest
            name: windows-latest-x86_64
          - os: macos-latest
            name: macos-latest-arm64
          - os: ubuntu-24.04-arm
            name: ubuntu-24.04-arm64
'''
# Exact aggregate preserves the nine-result and eight-row authoring steps,
# and adds the independent four-row Room wall result. Pin all three steps and
# dependency edges, including generated user-game and private action lanes in needs, result
# binding and the fail-closed loop; actual Bash failure states are tested separately.
AGGREGATE_SHA256 = 'e5bcdb49bc50ef54b20f907ea16069d5537cf87eb3d0d302b49e82dfe58339b8'


def listing(names):
    return ''.join(name + ': test\n' for name in names) + f'\n{len(names)} tests, 0 benchmarks\n'


def passed(names):
    return ''.join('test ' + name + ' ... ok\n' for name in names) + (
        f'\ntest result: ok. {len(names)} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n')


def encode_png(raw, size=(320, 256)):
    width, height = size
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data) & 0xffffffff)
    rows = b''.join(b'\0' + raw[y*width*4:(y+1)*width*4] for y in range(height))
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b'')


def capture_fixture(root):
    black = bytes((0, 0, 0, 255)) * (320 * 256)
    base = bytes((0, 200, 0, 255)) * 301 + black[301*4:]
    changed = bytes((200, 0, 0, 255)) * 102 + base[102*4:]
    raws = {name: base for name in gate.FRAMES}
    for view in ('side', 'oblique'):
        for variant in ('flip-z', 'transpose'):
            raws[view + '-' + variant] = changed
    raws['top-baseline'] = black
    raws['top-markers'] = bytes((255, 80, 0, 255)) * 31 + black[31*4:]
    raws['side-wrong-height'] = changed
    for name, raw in raws.items():
        (root / (name + '.rgba')).write_bytes(raw)
        (root / (name + '.png')).write_bytes(encode_png(raw))
    return raws


def section(text, header, indent):
    lines = text.splitlines()
    starts = [i for i, line in enumerate(lines) if line == header]
    gate.require(len(starts) == 1, 'unique section')
    start, end = starts[0], starts[0] + 1
    while end < len(lines):
        line = lines[end]
        if line.strip() and not line.lstrip().startswith('#') and len(line) - len(line.lstrip()) <= indent:
            break
        end += 1
    while end > start + 1 and (not lines[end-1].strip() or lines[end-1].lstrip().startswith('#')):
        end -= 1
    return '\n'.join(lines[start:end]) + '\n'


def expected_steps(phase):
    title = {'native': 'native inventories and strict feature boundary',
             'editor': 'editor inventory and strict feature combinations',
             'device': 'explicit software-device readback'}[phase]
    path = '${{ runner.temp }}/raw16-' + phase + '-${{ github.run_id }}-${{ github.run_attempt }}'
    name = 'Required RAW16 ' + title
    body = '      - name: ' + name + '\n'
    if phase == 'device':
        body += ('        if: runner.os == \'Linux\'\n        env:\n'
                 '          ORR_REQUIRE_GPU: "1"\n          ORR_RAW16_REQUIRE_SOFTWARE: "1"\n'
                 '          WGPU_BACKEND: vulkan\n')
    body += '        run: python -B tools/raw16_import_gate.py ' + phase + ' --evidence "' + path + '"\n'
    upload_name = 'Retain RAW16 ' + phase + ' and partial failure evidence'
    upload = ('      - name: ' + upload_name + '\n        if: always()' +
              (" && runner.os == 'Linux'" if phase == 'device' else '') + '\n'
              '        uses: actions/upload-artifact@v4\n        with:\n'
              '          name: raw16-' + phase + '-${{ matrix.' + ('name' if phase == 'native' else 'os') + ' }}\n'
              '          path: ' + path + '\n          if-no-files-found: error\n          retention-days: 7\n')
    return name, body, upload_name, upload


def workflow_contract(text):
    # Reuse the already-present test-only Room shape checker, never a runtime
    # helper from a newer development branch. It preserves four original gates.
    from test_room_project_gate import check_room_workflow
    check_room_workflow(text)
    jobs = section(text, 'jobs:', 0)
    native = section(jobs, '  native:', 2)
    sample = section(jobs, '  sample-build:', 2)
    for phase in ('native', 'editor', 'device'):
        job = native if phase == 'native' else sample
        name, body, upload_name, upload = expected_steps(phase)
        gate.require(section(job, '      - name: ' + name, 6) == body, 'mandatory RAW16 step')
        gate.require(section(job, '      - name: ' + upload_name, 6) == upload, 'failure evidence upload')
        gate.require(job.index(body) < job.index(upload), 'gate precedes upload')
    gate.require(native.index('      - name: cargo clippy --workspace --all-targets\n') < native.index(expected_steps('native')[1]) < native.index('      - name: cargo build -p orr_sim --features hotpatch\n'), 'native additive position')
    gate.require(sample.index('      - uses: Swatinem/rust-cache@v2\n') < sample.index(expected_steps('editor')[1]) < sample.index('      - name: View CPU library default tests\n'), 'editor additive position')
    mesa = '        run: sudo apt-get update -q && sudo apt-get install -y -q mesa-vulkan-drivers\n'
    gate.require(sample.index(mesa) < sample.index(expected_steps('device')[1]) < sample.index('        run: cargo test -p orr_render --release --test gpu --test gpu3d\n'), 'device provisioning position')
    header = native.split('    steps:\n', 1)[0]
    gate.require(header == NATIVE_HEADER, 'exact native matrix and execution header')
    aggregate = section(jobs, '  cross_platform_checksum_compare:', 2)
    gate.require(gate.digest(aggregate.encode('utf-8')) == AGGREGATE_SHA256, 'exact mandatory aggregate')


class Raw16GateTests(unittest.TestCase):
    def test_platform_and_command_inventory(self):
        self.assertEqual([sum(len(row[2]) for row in gate.native_lanes(p)) for p in ('linux', 'darwin', 'win32')], [32, 32, 30])
        self.assertEqual(set(gate.native_lanes('linux')[2][2]) - set(gate.native_lanes('win32')[2][2]), set(gate.UNIX_CLI))
        self.assertEqual(gate.native_lanes('win32')[1][2], [])
        for phase, count in [('native', 21), ('editor', 8), ('device', 4)]:
            labels = gate.command_inventory(phase)
            self.assertEqual(len(labels), count)
            self.assertEqual(len(labels), len(set(labels)))
        with self.assertRaises(AssertionError):
            gate.native_lanes('unknown')
        gate.dependency_tree('orr_terrain_import v0.0.1\norr_terrain v0.0.1\n')
        for name in ('orr_editor', 'orr_render', 'orr_rhi', 'wgpu', 'egui', 'eframe', 'winit'):
            with self.subTest(dependency=name), self.assertRaises(AssertionError):
                gate.dependency_tree('orr_terrain_import v0.0.1\n' + name + ' v1.0.0\n')
        with self.assertRaises(AssertionError):
            gate.dependency_tree('')

    def test_exact_names_and_duplicates(self):
        gate.inventory(listing(gate.EDITOR), gate.EDITOR)
        gate.inventory(listing([]), [])
        for names in (gate.EDITOR[:-1], gate.EDITOR + gate.EDITOR[:1], ['wrong'] * 3):
            with self.subTest(names=names), self.assertRaises(AssertionError):
                gate.inventory(listing(names), gate.EDITOR)
        with self.assertRaises(AssertionError):
            gate.inventory(listing(gate.EDITOR).replace('3 tests', '0 tests'), gate.EDITOR)

    def test_zero_ignored_filtered_and_failed_results(self):
        text = passed(gate.EDITOR)
        gate.result(text, gate.EDITOR)
        gate.result(passed([]), [])
        variants = [passed([]), text + text, text.replace('0 ignored', '1 ignored'),
                    text.replace('0 filtered', '1 filtered'), text.replace('0 failed', '1 failed'),
                    text.replace(' ... ok', ' ... ignored', 1), text.replace(gate.EDITOR[0], 'wrong', 1),
                    text + 'SKIP: no device\n', text + 'skipping device\n']
        for changed in variants:
            with self.subTest(text=changed), self.assertRaises(AssertionError):
                gate.result(changed, gate.EDITOR)
        helper = ''.join(f'{name} (__main__.Raw16GateTests.{name}) ... ok\n' for name in gate.HELPER_TESTS)
        helper += '\nRan 12 tests in 0.01s\n\nOK\n'
        gate.helper_result(helper)
        for changed in (helper.replace(' ... ok', ' ... skipped', 1), helper.replace(' ... ok', ' ... FAIL', 1),
                        helper.replace('Ran 12 tests', 'test_extra (__main__.Raw16GateTests) ... skipped\nRan 12 tests'),
                        helper.replace('Ran 12 tests', 'Ran 0 tests'), helper + helper):
            with self.assertRaises(AssertionError):
                gate.helper_result(changed)

    def test_artifact_selection(self):
        root = ROOT
        def artifact(kind, name, test):
            return {'reason': 'compiler-artifact', 'manifest_path': str(root / 'crates/orr_terrain_import/Cargo.toml'),
                    'target': {'kind': [kind], 'name': name}, 'profile': {'test': test}, 'executable': '/not-executed'}
        rows = [artifact('test', 'cli', True), artifact('bin', gate.PACKAGE, False)]
        encode = lambda records: '\n'.join(map(json.dumps, records))
        selected = gate.artifact_records(encode(rows), root, [('test', 'cli')], True)
        self.assertEqual(set(selected), {'test:cli', 'actual-cli'})
        for changed in (rows[:1], rows + rows[:1], rows + [artifact('test', 'editor', True)],
                        [artifact('bin', 'cli', True), rows[1]]):
            with self.subTest(rows=changed), self.assertRaises(AssertionError):
                gate.artifact_records(encode(changed), root, [('test', 'cli')], True)
        with tempfile.TemporaryDirectory() as name:
            temporary = Path(name)
            original = temporary / 'producer'; original.write_bytes(b'fixture, never executed')
            instance = gate.Gate.__new__(gate.Gate)
            instance.root, instance.evidence, instance.producers = root, temporary, {}
            calls = []
            entry = artifact('test', 'render', True); entry['executable'] = str(original)
            instance.run = lambda label, argv, **kw: (calls.append((label, argv, kw)) or json.dumps(entry))
            binaries = instance.compile('device-compile', 'gpu-acceptance', [('test', 'render')])
            self.assertEqual(calls[-1][1], ['cargo', 'test', '--locked', '--release', '-p', gate.PACKAGE,
                '--no-default-features', '--features', 'gpu-acceptance', '--test', 'render', '--no-run', '--message-format=json'])
            def response(label, argv, **kw):
                calls.append((label, argv, kw))
                return listing([gate.DEVICE]) if label.endswith('-list') else passed([gate.DEVICE])
            instance.run = response
            instance.test('render', binaries['test:render'], [gate.DEVICE], [gate.DEVICE], device=True)
            self.assertEqual(calls[-1][1][1:], ['--exact', gate.DEVICE, '--ignored', '--test-threads=1', '--show-output'])
            self.assertEqual(calls[-1][2], {'timeout': 180})
            instance.strict('combined-strict', 'editor-acceptance,gpu-acceptance')
            self.assertEqual(calls[-1][1], ['cargo', 'clippy', '--locked', '--release', '-p', gate.PACKAGE,
                '--no-default-features', '--features', 'editor-acceptance,gpu-acceptance', '--all-targets', '--', '-D', 'warnings'])
            instance.run = mock.Mock(side_effect=RuntimeError('preserved caller failure'))
            for action in (lambda: instance.strict('strict'),
                           lambda: instance.compile('compile', 'gpu-acceptance', [('test', 'render')]),
                           lambda: instance.test('render', original, [gate.DEVICE])):
                with self.assertRaisesRegex(RuntimeError, 'preserved caller failure'):
                    action()

    def test_actual_cli_producer_fence(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            cli, harness = root / 'actual-cli', root / 'saved-harness'
            cli.write_bytes(b'actual'); harness.write_bytes(b'harness')
            records = {label: {'path': str(path), 'sha256': gate.digest(path.read_bytes())}
                       for label, path in [('actual-cli:original', cli), ('test:cli:saved', harness)]}
            gate.verify_producers(records)
            for path in (cli, harness):
                original = path.read_bytes(); path.write_bytes(b'changed')
                with self.assertRaises(AssertionError):
                    gate.verify_producers(records)
                path.write_bytes(original)
            cli.unlink()
            with self.assertRaises(FileNotFoundError):
                gate.verify_producers(records)

    def test_source_and_mode_fence(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            instance = gate.Gate.__new__(gate.Gate)
            instance.source = {'a': {'mode': '100644', 'index_blob': 'blob', 'sha256': 'hash'}}
            actual = json.loads(json.dumps(instance.source))
            instance.snapshot = lambda: actual
            instance.identity = {'head': 'head'}
            instance.git = lambda _: 'head\n'
            instance.producers = {}
            instance.captures = root / 'captures'; instance.captures.mkdir()
            instance.cache = root / 'cache'; instance.cache.mkdir()
            instance.fence()
            for field in ('mode', 'index_blob', 'sha256'):
                old = actual['a'][field]; actual['a'][field] = 'changed'
                with self.assertRaises(AssertionError):
                    instance.fence()
                actual['a'][field] = old
            instance.git = lambda _: 'other\n'
            with self.assertRaises(AssertionError):
                instance.fence()

    def test_software_adapter_required(self):
        good = 'RAW16 acceptance adapter: llvmpipe (software: true)\n'
        self.assertTrue(gate.adapter(good)['software'])
        for text in ('', good + good, good.replace('true', 'false'), good.replace('llvmpipe', '')):
            with self.subTest(text=text), self.assertRaises(AssertionError):
                gate.adapter(text)

    def test_png_integrity_and_bounds(self):
        raw = bytes((1, 2, 3, 255)) * (320 * 256)
        png = encode_png(raw)
        self.assertEqual(gate.png_rgba(png, (320, 256)), raw)
        for data in (png + b'x', png[:-1], png[:12] + b'FAIL' + png[16:], b'x' * (2*1024**2+1)):
            with self.subTest(size=len(data)), self.assertRaises(AssertionError):
                gate.png_rgba(data, (320, 256))
        with self.assertRaises(AssertionError):
            gate.png_rgba(png, (319, 256))

    def test_capture_census_and_controls(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            raws = capture_fixture(root)
            report = gate.audit_captures(root)
            self.assertEqual(len(report['files']), 20)
            extra = root / 'extra'; extra.write_bytes(b'x')
            with self.assertRaises(AssertionError):
                gate.audit_captures(root)
            extra.unlink()
            path = root / 'oblique-flip-z.rgba'
            original = path.read_bytes(); path.write_bytes(original[:-1])
            with self.assertRaises(AssertionError):
                gate.audit_captures(root)
            path.write_bytes(raws['oblique-baseline'])
            with self.assertRaises(AssertionError):
                gate.audit_captures(root)
            (root / 'oblique-flip-z.png').write_bytes(encode_png(raws['oblique-baseline']))
            with self.assertRaises(AssertionError):
                gate.audit_captures(root)
            path.write_bytes(original)
            (root / 'oblique-flip-z.png').write_bytes(encode_png(original))
            path.unlink()
            with self.assertRaises(AssertionError):
                gate.audit_captures(root)

    def test_exclusive_evidence_and_budgets(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name).resolve()
            env = {'GITHUB_ACTIONS': 'true', 'GITHUB_RUN_ID': '123', 'GITHUB_RUN_ATTEMPT': '1', 'RUNNER_TEMP': str(root)}
            path = root / 'raw16-native-123-1'
            self.assertEqual(gate.evidence_path(path, 'native', env), path)
            for key, value in [('GITHUB_ACTIONS', 'false'), ('GITHUB_RUN_ID', '../123'), ('GITHUB_RUN_ATTEMPT', '0')]:
                with self.subTest(key=key), self.assertRaises(AssertionError):
                    gate.evidence_path(path, 'native', dict(env, **{key: value}))
            gate.compiler_environment({})
            for key in ('RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
                with self.subTest(override=key), self.assertRaises(AssertionError):
                    gate.compiler_environment({key: '-A warnings'})
            path.mkdir()
            with self.assertRaises(AssertionError):
                gate.evidence_path(path, 'native', env)
            (path / 'bytes').write_bytes(b'1234')
            self.assertEqual(gate.directory_bytes(path), 4)
            if os.name != 'nt':
                (path / 'link').symlink_to(path / 'bytes')
                with self.assertRaises(AssertionError):
                    gate.directory_bytes(path)

    def test_workflow_each_inherited_and_private_action_dependency_is_mandatory(self):
        workflow = (ROOT / '.github/workflows/determinism.yml').read_text()
        workflow_contract(workflow)
        dependencies = ('native', 'wasm32', 'wasm32-browser', 'android-arm64',
                        'sample-build', 'generated-user-game-native',
                        'static-game-actions-native', 'sample-arena-cpu',
                        'sample-fixture-preflight', 'sample-linked-prefabs',
                        'sample-room', 'static-model-authoring', 'room-wall-authoring')
        line = '    needs: [' + ', '.join(dependencies) + ']'
        self.assertEqual(workflow.count(line), 1)
        for missing in dependencies:
            changed = workflow.replace(line, '    needs: [' + ', '.join(
                name for name in dependencies if name != missing) + ']', 1)
            with self.subTest(missing=missing), self.assertRaises(AssertionError):
                workflow_contract(changed)

    def test_mandatory_workflow_and_preserved_gates(self):
        workflow = (ROOT / '.github/workflows/determinism.yml').read_text()
        workflow_contract(workflow)
        for phase in ('native', 'editor', 'device'):
            name, body, upload_name, upload = expected_steps(phase)
            for changed in ('', body + body, body.replace('        run:', '        if: false\n        run:'),
                            body.replace('        run:', '        continue-on-error: true\n        run:'),
                            body.replace('raw16_import_gate.py', 'other.py'), body.replace(phase + ' --evidence', 'wrong --evidence')):
                with self.subTest(phase=phase, body=changed), self.assertRaises(AssertionError):
                    workflow_contract(workflow.replace(body, changed, 1))
            for changed in ('', upload.replace('always()', 'success()'), upload.replace('if-no-files-found: error', 'if-no-files-found: ignore')):
                with self.subTest(phase=phase, upload=changed), self.assertRaises(AssertionError):
                    workflow_contract(workflow.replace(upload, changed, 1))
        for lane, result in [('sample-arena-cpu', 'SAMPLE_ARENA_CPU_RESULT'),
                             ('sample-fixture-preflight', 'SAMPLE_PREFLIGHT_RESULT')]:
            for old, new in [(', ' + lane, ''),
                             ("          " + result + ": ${{ needs['" + lane + "'].result }}\n", ''),
                             (' ' + result + ' ', ' ')]:
                self.assertIn(old, workflow)
                with self.subTest(lane=lane, removed=old), self.assertRaises(AssertionError):
                    workflow_contract(workflow.replace(old, new, 1))
        for old, new in [('ORR_RAW16_REQUIRE_SOFTWARE: "1"', 'ORR_RAW16_REQUIRE_SOFTWARE: "0"'),
                         ('needs: [native, wasm32', 'needs: [wasm32'),
                         (', sample-room, static-model-authoring, room-wall-authoring]', ', static-model-authoring, room-wall-authoring]'),
                         (', static-model-authoring, room-wall-authoring]', ', room-wall-authoring]'),
                         ("          AUTHORING_RESULT: ${{ needs['static-model-authoring'].result }}\n", ''),
                         ('      - name: Require every independent static authoring matrix row to pass\n', '      - name: Require every independent static authoring matrix row to pass\n        if: false\n'),
                         ('[[ \"$AUTHORING_RESULT\" != \"success\" ]]', '[[ \"$AUTHORING_RESULT\" == \"success\" ]]'),
                         ("          ROOM_RESULT: ${{ needs['sample-room'].result }}\n", ''),
                         ('LINKED_PREFABS_RESULT ROOM_RESULT GENERATED_USER_NATIVE_RESULT PRIVATE_ACTION_NATIVE_RESULT; do', 'LINKED_PREFABS_RESULT GENERATED_USER_NATIVE_RESULT PRIVATE_ACTION_NATIVE_RESULT; do'),
                         (', generated-user-game-native', ''),
                         ("          GENERATED_USER_NATIVE_RESULT: ${{ needs['generated-user-game-native'].result }}\n", ''),
                         ('ROOM_RESULT GENERATED_USER_NATIVE_RESULT PRIVATE_ACTION_NATIVE_RESULT; do', 'ROOM_RESULT PRIVATE_ACTION_NATIVE_RESULT; do'),
                         (', static-game-actions-native', ''),
                         ("          PRIVATE_ACTION_NATIVE_RESULT: ${{ needs['static-game-actions-native'].result }}\n", ''),
                         (' GENERATED_USER_NATIVE_RESULT PRIVATE_ACTION_NATIVE_RESULT; do', ' GENERATED_USER_NATIVE_RESULT; do'),
                         ('  native:\n', '  native:\n    if: false\n'),
                         ('          - os: macos-latest\n', ''),
                         ('    runs-on: ${{ matrix.os }}\n', '    runs-on: ubuntu-latest\n'),
                         ('        include:\n', '        exclude:\n          - os: windows-latest\n        include:\n'),
                         ('            name: ubuntu-24.04-arm64\n', '            name: ubuntu-24.04-arm64\n          - os: ubuntu-22.04\n            name: extra\n')]:
            with self.subTest(old=old):
                self.assertIn(old, workflow, 'negative-control target must exist')
                changed = workflow.replace(old, new, 1)
                self.assertNotEqual(changed, workflow, 'negative control must change input')
                with self.assertRaises(AssertionError):
                    workflow_contract(changed)

        aggregate = section(workflow, '  cross_platform_checksum_compare:', 2)
        for changed in (aggregate.replace('          failed=0', '          exit 0\n          failed=0', 1),
                        aggregate.replace('  cross_platform_checksum_compare:\n',
                                          '  cross_platform_checksum_compare:\n    continue-on-error: true\n', 1)):
            with self.assertRaises(AssertionError):
                workflow_contract(workflow.replace(aggregate, changed, 1))

    def test_bounded_process_receipts(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            command = lambda code: [sys.executable, '-c', code]
            path = root / 'ok.log'
            self.assertEqual(gate.bounded_run(command("print('ok')"), root, os.environ.copy(), path, 10, disk_floor=0), 'ok\n')
            receipt = json.loads(path.with_suffix('.json').read_text())
            self.assertEqual(receipt['exit'], 0)
            self.assertTrue(receipt['owned_children_reaped'])
            cases = [('exit', 'import sys;sys.exit(7)', 10), ('timeout', 'import time;time.sleep(30)', .1)]
            for label, code, timeout in cases:
                path = root / (label + '.log')
                with self.assertRaises(AssertionError):
                    gate.bounded_run(command(code), root, os.environ.copy(), path, timeout, disk_floor=0)
                self.assertTrue(json.loads(path.with_suffix('.json').read_text())['owned_children_reaped'])
            with mock.patch.object(gate, 'LOG_CAP', 64):
                path = root / 'cap.log'
                with self.assertRaises(AssertionError):
                    gate.bounded_run(command("print('x'*1000)"), root, os.environ.copy(), path, 10, disk_floor=0)
                self.assertLessEqual(path.stat().st_size, 64)
                receipt = json.loads(path.with_suffix('.json').read_text())
                self.assertEqual(receipt['stop_reason'], 'log cap')
                self.assertTrue(receipt['owned_children_reaped'])
                self.assertIsNotNone(receipt['exit'])
            self._posix_cleanup_controls(root)
            if os.name != 'nt':
                self._real_posix_permission_cleanup(root)
                self._exited_leader_cleanup(root)

    def _posix_cleanup_controls(self, root):
        # Deterministic Darwin syscall outcomes, also run on other platforms.
        # No real PIDs are signalled by these controls.
        denied = PermissionError(1, 'Operation not permitted')
        missing = ProcessLookupError(3, 'No such process')
        sigkill = getattr(signal, 'SIGKILL', 9)
        cases = [
            ('zombie-only', 'darwin', denied, missing, None, False, True),
            ('live-group', 'darwin', denied, None, None, False, False),
            ('denied-group', 'darwin', denied, denied, None, False, False),
            ('probe-error', 'darwin', denied, OSError(5, 'probe failed'), None, False, False),
            ('wait-timeout', 'darwin', denied, missing,
             subprocess.TimeoutExpired(['fixture'], 10), False, False),
            ('other-platform-denial', 'linux', denied, missing, None, False, False),
            ('other-permission-error', 'darwin', PermissionError(errno.EACCES, 'Permission denied'),
             missing, None, False, False),
            ('signal-error', 'darwin', OSError(5, 'signal failed'), missing, None, False, False),
            ('missing-group', 'darwin', missing, missing, None, False, True),
            ('reader-live', 'darwin', None, missing, None, True, False),
        ]
        for label, platform, signal_error, probe_error, wait_error, reader_live, complete in cases:
            with self.subTest(cleanup=label):
                path = root / (label + '.log'); calls = []
                process = mock.Mock(pid=12345, returncode=None, stdin=None)
                def wait(timeout):
                    calls.append(('wait', timeout))
                    if wait_error is not None: raise wait_error
                    process.returncode = 0
                    return 0
                process.wait.side_effect = wait
                def killpg(pid, sig):
                    calls.append(('killpg', pid, sig))
                    error = signal_error if sig == sigkill else probe_error
                    if error is not None: raise error
                reader = mock.Mock()
                reader.is_alive.return_value = reader_live
                reader.join.side_effect = lambda timeout: calls.append(('join', timeout))
                events = mock.Mock()
                events.get.return_value = b'x' * 1000
                stopped = gate.threading.Event()
                with mock.patch.object(gate.os, 'name', 'posix'), \
                     mock.patch.object(gate.sys, 'platform', platform), \
                     mock.patch.object(gate.signal, 'SIGKILL', sigkill, create=True), \
                     mock.patch.object(gate.os, 'killpg', side_effect=killpg, create=True), \
                     mock.patch.object(gate.subprocess, 'Popen', return_value=process) as popen, \
                     mock.patch.object(gate.threading, 'Thread', return_value=reader), \
                     mock.patch.object(gate.threading, 'Event', return_value=stopped), \
                     mock.patch.object(gate.queue, 'Queue', return_value=events), \
                     mock.patch.object(gate, 'LOG_CAP', 64):
                    with self.assertRaises(AssertionError):
                        gate.bounded_run(['fixture'], root, {}, path, 10, disk_floor=0)
                receipt = json.loads(path.with_suffix('.json').read_text())
                self.assertEqual(path.stat().st_size, 64)
                self.assertEqual(receipt['owned_children_reaped'], complete)
                self.assertEqual(popen.call_args.kwargs['start_new_session'], True)
                self.assertEqual(calls[:2], [('killpg', process.pid, sigkill), ('wait', 10)])
                self.assertEqual(calls[-1], ('join', 10))
                reader.start.assert_called_once_with()
                self.assertTrue(stopped.is_set())
                signals = [call for call in calls if call[0] == 'killpg']
                self.assertEqual([call for call in signals if call[2] != 0],
                                 [('killpg', process.pid, sigkill)])
                if complete:
                    self.assertEqual(receipt['stop_reason'], 'log cap')
                else:
                    self.assertIn('cleanup:', receipt['stop_reason'])
                if isinstance(signal_error, PermissionError):
                    proof = receipt['posix_group_cleanup']
                    self.assertEqual(proof['signal_error'], repr(signal_error))
                    self.assertEqual(proof['child_reaped'], wait_error is None)
                    self.assertEqual(proof['group_absent_after_reap'], label == 'zombie-only')
                    if not complete: self.assertIn(repr(signal_error), receipt['stop_reason'])
                    should_probe = platform == 'darwin' and signal_error.errno == errno.EPERM and wait_error is None
                    self.assertEqual(signals[1:], [('killpg', process.pid, 0)] if should_probe else [])
                    if should_probe and probe_error is not None and probe_error is not missing:
                        self.assertEqual(proof['probe_error'], repr(probe_error))
                else:
                    self.assertNotIn('posix_group_cleanup', receipt)

    def _real_posix_permission_cleanup(self, root):
        # Inject Darwin's signal error around a real child and real absence probe.
        path = root / 'real-zombie.log'; children = []; signals = []
        real_popen, real_killpg = subprocess.Popen, os.killpg
        def launch(*args, **kwargs):
            process = real_popen(*args, **kwargs); children.append(process); return process
        def killpg(pid, sig):
            self.assertEqual(pid, children[0].pid)
            signals.append(sig)
            if sig == signal.SIGKILL: raise PermissionError(1, 'Operation not permitted')
            self.assertEqual(sig, 0)
            self.assertIsNotNone(children[0].returncode)
            return real_killpg(pid, sig)
        with mock.patch.object(gate.subprocess, 'Popen', side_effect=launch), \
             mock.patch.object(gate.os, 'killpg', side_effect=killpg), \
             mock.patch.object(gate.sys, 'platform', 'darwin'), \
             mock.patch.object(gate, 'LOG_CAP', 64):
            with self.assertRaises(AssertionError):
                gate.bounded_run([sys.executable, '-c', "print('x'*1000)"],
                                 root, os.environ.copy(), path, 10, disk_floor=0)
        receipt = json.loads(path.with_suffix('.json').read_text())
        self.assertEqual(signals, [signal.SIGKILL, 0])
        self.assertEqual(receipt['stop_reason'], 'log cap')
        self.assertEqual(receipt['exit'], 0)
        self.assertTrue(receipt['owned_children_reaped'])
        self.assertEqual(receipt['posix_group_cleanup'], {
            'signal_error': repr(PermissionError(1, 'Operation not permitted')),
            'child_reaped': True, 'group_absent_after_reap': True})
        self.assertTrue(children[0].stdout.closed)

    def _exited_leader_cleanup(self, root):
        # The child emits excess output only after its group leader exits.
        # It holds stdout until killed, or records failure before a bounded exit.
        path = root / 'descendant.log'
        survived = root / 'descendant-survived'
        child_code = ('import os,sys,time\nfrom pathlib import Path\n'
                      'while os.getppid() == int(sys.argv[1]): time.sleep(.01)\n'
                      'print("x"*1000,flush=True)\ntime.sleep(5)\n'
                      'Path(sys.argv[2]).write_text("group cleanup missed live child")\n')
        code = ('import os,subprocess,sys; '
                'subprocess.Popen([sys.executable,"-c",sys.argv[1],str(os.getpid()),sys.argv[2]]); '
                'os._exit(0)')
        children = []
        real_popen = subprocess.Popen
        def launch(*args, **kwargs):
            process = real_popen(*args, **kwargs); children.append(process); return process
        with mock.patch.object(gate.subprocess, 'Popen', side_effect=launch), \
             mock.patch.object(gate, 'LOG_CAP', 64):
            with self.assertRaises(AssertionError):
                gate.bounded_run([sys.executable, '-c', code, child_code, str(survived)],
                                 root, os.environ.copy(), path, 10, disk_floor=0)
        receipt = json.loads(path.with_suffix('.json').read_text())
        self.assertEqual(receipt['stop_reason'], 'log cap')
        self.assertEqual(receipt['exit'], 0)
        self.assertTrue(receipt['owned_children_reaped'])
        self.assertTrue(children[0].stdout.closed)
        self.assertEqual(path.read_bytes(), b'x' * 64)
        self.assertFalse(survived.exists())


if __name__ == '__main__':
    unittest.main()
