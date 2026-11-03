"""Legacy Scene parity and Rust wave/health options through trusted WASM/offline delivery.

Requires ORR_PKG_BIN and ORR_COOP_WEB_BUILD_DIR containing the reviewed locally
built CoopDefense game.wasm, plus Node on PATH. No mock WASM/scene validator.
Python is the internal legacy oracle; Node remains an explicit product dependency.
Packaging/checksum validation is not browser, gameplay-session or GPU acceptance.
"""
import hashlib
from html.parser import HTMLParser
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

import create_coop_defense_scene as legacy
import pack_coop_defense_offline as packer

TOOLS = Path(__file__).resolve().parent


def wasm_uint(value):
    """Unsigned LEB128 for fixed trusted test-module lengths/indices."""
    result = bytearray()
    while True:
        byte = value & 127
        value >>= 7
        result.append(byte | (128 if value else 0))
        if not value:
            return bytes(result)


def wasm_section(kind, payload):
    return bytes([kind]) + wasm_uint(len(payload)) + payload


def inert_wasm_admission_failures():
    """No host effects: import-only module and const-returning wrong-init ABI."""
    header = b'\0asm\x01\0\0\0'
    # A valid module requesting env.blocked: () -> (), never instantiated by gate.
    imports = (header + wasm_section(1, b'\x01\x60\x00\x00')
               + wasm_section(2, b'\x01\x03env\x07blocked\x00\x00'))
    # All required exports exist. The only initialization behavior is returning 1;
    # no start function, imports, loops, memory writes or nonconstant computation.
    types = wasm_section(1, b'\x02\x60\x00\x01\x7f\x60\x01\x7f\x01\x7f')
    functions = wasm_section(3, b'\x05\x00\x00\x01\x00\x00')
    memory = wasm_section(5, b'\x01\x00\x01')
    names = ['defense_scene_buffer', 'defense_scene_capacity', 'defense_init_scene',
             'defense_snapshot', 'defense_snapshot_len']
    exports = b'\x06\x06memory\x02\x00'
    for index, name in enumerate(names):
        encoded = name.encode('ascii')
        exports += wasm_uint(len(encoded)) + encoded + b'\x00' + wasm_uint(index)
    bodies = b'\x05'
    for value in (0, 65536, 1, 0, 0):
        # These positive constants have a clear signed-LEB sign bit, so unsigned
        # encoding is also their valid signed encoding for i32.const.
        body = b'\x00\x41' + wasm_uint(value) + b'\x0b'
        bodies += wasm_uint(len(body)) + body
    bad_init = header + types + functions + memory + wasm_section(7, exports) + wasm_section(10, bodies)
    return [('imports', imports, 'engine must not require imports'),
            ('init-refused', bad_init, 'engine rejected authored scene')]


class EmbeddedScripts(HTMLParser):
    def __init__(self):
        super().__init__()
        self.current = None
        self.scripts = {}

    def handle_starttag(self, tag, attributes):
        if tag == 'script':
            self.current = dict(attributes).get('id')
            if self.current:
                if self.current in self.scripts:
                    raise AssertionError('duplicate embedded script')
                self.scripts[self.current] = ''

    def handle_endtag(self, tag):
        if tag == 'script':
            self.current = None

    def handle_data(self, data):
        if self.current:
            self.scripts[self.current] += data


class RustCoopWebScene(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        binary, build = os.environ.get('ORR_PKG_BIN'), os.environ.get('ORR_COOP_WEB_BUILD_DIR')
        if not binary or not Path(binary).is_absolute() or not Path(binary).is_file() or not os.access(binary, os.X_OK):
            raise RuntimeError('ORR_PKG_BIN must identify the actual trusted native orr_pkg executable')
        if not build or not Path(build).is_absolute() or not Path(build).is_dir():
            raise RuntimeError('ORR_COOP_WEB_BUILD_DIR must identify the reviewed actual clean build')
        if not shutil.which('node'):
            raise RuntimeError('Node must be available on PATH for actual trusted-WASM admission')
        cls.binary, cls.build = binary, Path(build)
        cls.wasm_bytes = (cls.build / 'game.wasm').read_bytes()
        if cls.wasm_bytes[:8] != b'\0asm\x01\0\0\0':
            raise RuntimeError('Actual WASM fixture required, not a placeholder')

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.wasm = self.root / 'trusted.wasm'
        self.wasm.write_bytes(self.wasm_bytes)
        self.sentinel = self.root / 'unrelated-user-file'
        self.sentinel.write_bytes(b'keep')

    def command(self, native, output, extra=(), wasm=None):
        prefix = [self.binary, 'create-coop-scene'] if native else [sys.executable, '-B', str(TOOLS / 'create_coop_defense_scene.py')]
        return prefix + ['--wasm', str(self.wasm if wasm is None else wasm), '--output', str(output), *map(str, extra)]

    def execute(self, command):
        return subprocess.run(command, capture_output=True, text=True, timeout=30,
                              env={**os.environ, 'PYTHONDONTWRITEBYTECODE': '1'})

    def success(self, native, output, extra=()):
        result = self.execute(self.command(native, output, extra))
        self.assertEqual(result.returncode, 0, result.stderr)
        receipt = json.loads(result.stdout)
        self.assertEqual(set(receipt), {'version', 'sceneSha256', 'sceneBytes', 'wasmSha256', 'initialChecksum'})
        self.assertEqual(receipt['version'], 1)
        self.assertEqual(receipt['sceneBytes'], output.stat().st_size)
        self.assertEqual(receipt['sceneSha256'], hashlib.sha256(output.read_bytes()).hexdigest())
        self.assertEqual(receipt['wasmSha256'], hashlib.sha256(self.wasm_bytes).hexdigest())
        self.assertRegex(receipt['initialChecksum'], r'^[0-9a-f]{16}$')
        self.assertEqual(self.wasm.read_bytes(), self.wasm_bytes)
        self.assertEqual(self.sentinel.read_bytes(), b'keep')
        return receipt

    def rejected_both(self, extra=(), wasm=None):
        source = self.wasm if wasm is None else wasm
        before = source.read_bytes() if source.is_file() and not source.is_symlink() else None
        for native in (False, True):
            output = self.root / f'refused-{native}.yaml'
            result = self.execute(self.command(native, output, extra, wasm))
            self.assertNotEqual(result.returncode, 0, result.stdout)
            self.assertFalse(os.path.lexists(output))
            self.assertEqual(self.sentinel.read_bytes(), b'keep')
        if before is not None:
            self.assertEqual(source.read_bytes(), before)

    def test_exact_yaml_receipts_initial_checksums_defaults_and_all_boundaries(self):
        cases = [(150, 30, 10, None), (0, 1, 1, None), (10000, 1000, 10, None),
                 (250, 75, 3, 35), (250, 75, 3, 70), (250, 75, 3, 350)]
        for index, (gold, hp, wave, interval) in enumerate(cases):
            args = [] if index == 0 else ['--starting-gold', gold, '--reactor-hp', hp, '--last-wave', wave]
            if interval is not None:
                args += ['--spawn-interval-ticks', interval]
            outputs = [self.root / f'case-{index}-{native}.yaml' for native in (False, True)]
            receipts = [self.success(native, output, args) for native, output in zip((False, True), outputs)]
            expected = legacy.scene_text(gold, hp, wave, interval).encode()
            self.assertEqual(outputs[0].read_bytes(), expected)
            self.assertEqual(outputs[1].read_bytes(), expected)
            self.assertEqual(receipts[0], receipts[1])
            self.assertEqual(b'CoopDefense::WavePacing:' in expected, interval is not None)
        explicit = self.root / 'explicit-defaults.yaml'
        self.success(True, explicit, ['--starting-gold', 150, '--reactor-hp', 30, '--last-wave', 10])
        self.assertEqual(explicit.read_bytes(), (self.root / 'case-0-True.yaml').read_bytes())

    def test_invalid_numeric_options_never_publish(self):
        for name, values in [('--starting-gold', [-1, 10001]), ('--reactor-hp', [0, 1001]),
                             ('--last-wave', [0, 11]), ('--spawn-interval-ticks', [34, 351])]:
            for value in [*values, '1.5', 'True', 'NaN', '1e2', '1\nother: 2', '18446744073709551616']:
                with self.subTest(name=name, value=value):
                    self.rejected_both([name, value])

    def test_native_wave_pairs_exact_bytes_boundaries_and_default_equivalence(self):
        defaults = [10, 12, 14, 16, 18, 20, 22, 24, 26, 28]
        omitted = self.success(True, self.root / 'omitted-counts.yaml')
        for index, counts in enumerate(([1] * 10, [28] * 10,
                                        [3, 1, 28, 4, 5, 6, 7, 8, 9, 10], defaults)):
            with self.subTest(counts=counts):
                output = self.root / f'counts-{index}.yaml'
                extra = ['--wave-pairs', ','.join(map(str, counts))]
                receipt = self.success(True, output, extra)
                expected = (legacy.scene_text() + '  CoopDefense::WaveCounts:\n'
                            + '    pair_counts: [' + ', '.join(map(str, counts)) + ']\n').encode()
                self.assertEqual(output.read_bytes(), expected)
                repeated = self.root / f'repeated-counts-{index}.yaml'
                self.assertEqual(self.success(True, repeated, extra), receipt)
                result = self.execute(self.command(True, output, extra))
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(output.read_bytes(), expected)
                if counts == defaults:
                    self.assertEqual(receipt['initialChecksum'], omitted['initialChecksum'])
                else:
                    self.assertNotEqual(receipt['initialChecksum'], omitted['initialChecksum'])

    def test_native_health_exact_bytes_and_default_semantics(self):
        omitted = self.success(True, self.root / 'omitted-health.yaml')
        for multiplier in range(1, 5):
            with self.subTest(multiplier=multiplier):
                output = self.root / f'health-{multiplier}.yaml'
                extra = ['--enemy-health-multiplier', multiplier]
                receipt = self.success(True, output, extra)
                expected = (legacy.scene_text() + '  CoopDefense::EnemyHealth:\n'
                            + f'    multiplier: {multiplier}\n').encode()
                self.assertEqual(output.read_bytes(), expected)
                self.assertEqual(self.success(True, self.root / f'repeated-health-{multiplier}.yaml', extra), receipt)
                if multiplier == 1:
                    self.assertEqual(receipt['initialChecksum'], omitted['initialChecksum'])
                    self.assertNotEqual(receipt['sceneSha256'], omitted['sceneSha256'])
                else:
                    self.assertNotEqual(receipt['initialChecksum'], omitted['initialChecksum'])
                self.assertEqual(packer.validate_scene(expected, self.wasm_bytes), receipt)
                result = self.execute(self.command(True, output, extra))
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(output.read_bytes(), expected)

    def test_native_invalid_health_never_publishes_or_replaces_output(self):
        invalid = ['', '0', '5', '-1', '1.5', 'true', 'NaN', '1e2', '0x1',
                   '1__0', '١', '1\nother: 2', '18446744073709551616']
        cases = [['--enemy-health-multiplier', value] for value in invalid]
        cases += [['--enemy-health-multiplier'],
                  ['--enemy-health-multiplier', '1', '--enemy-health-multiplier', '4']]
        for extra in cases:
            for existing in (False, True):
                with self.subTest(extra=extra, existing=existing):
                    output = self.root / f'refused-health-{existing}.yaml'
                    if existing:
                        output.write_bytes(b'keep output')
                    result = self.execute(self.command(True, output, extra))
                    self.assertEqual(result.returncode, 2, result.stderr)
                    if existing:
                        self.assertEqual(output.read_bytes(), b'keep output')
                    else:
                        self.assertFalse(os.path.lexists(output))
                    self.assertEqual(self.wasm.read_bytes(), self.wasm_bytes)
                    self.assertEqual(self.sentinel.read_bytes(), b'keep')

    def test_native_health_offline_relocated_source_hidden_consumption(self):
        import base64
        scene = self.root / 'health.scene.yaml'
        receipt = self.success(True, scene, ['--starting-gold', 250, '--reactor-hp', 75,
                                            '--last-wave', 3, '--spawn-interval-ticks', 70,
                                            '--wave-pairs', '1,2,3,4,5,6,7,8,9,28',
                                            '--enemy-health-multiplier', 4])
        scene_bytes = scene.read_bytes()
        web = self.root / 'health-web'
        shutil.copytree(self.build, web)
        html = self.root / 'health.html'
        result = self.execute([self.binary, 'pack-coop-offline', '--build-dir', str(web),
                               '--scene', str(scene), '--output', str(html)])
        self.assertEqual(result.returncode, 0, result.stderr)
        checked = self.execute([sys.executable, '-B', str(TOOLS / 'check_coop_defense_offline.py'), str(web), str(html)])
        self.assertEqual(checked.returncode, 0, checked.stderr)
        self.assertTrue(json.loads(checked.stdout)['passed'])
        consumer = self.root / 'relocated-consumer'
        consumer.mkdir()
        relocated = consumer / 'game.html'
        html.rename(relocated)
        # Hide all original temporary producer inputs before extracting only the
        # delivered HTML. This is Node/WASM consumption, not browser-rendered QA.
        for source in (scene, self.wasm, web):
            source.rename(source.with_name(source.name + '.hidden'))
            self.assertFalse(source.exists())
        parsed = EmbeddedScripts()
        text = relocated.read_text()
        parsed.feed(text)
        embedded_wasm = base64.b64decode(parsed.scripts['wasm-core'], validate=True)
        embedded_scene = base64.b64decode(parsed.scripts['authored-scene'], validate=True)
        self.assertEqual(embedded_wasm, self.wasm_bytes)
        self.assertEqual(embedded_scene, scene_bytes)
        self.assertEqual(json.loads(parsed.scripts['authored-scene-receipt']), receipt)
        self.assertEqual(packer.validate_scene(embedded_scene, embedded_wasm), receipt)
        for source in (scene, self.wasm, web):
            self.assertNotIn(str(source), text)
        (consumer / 'embedded.wasm').write_bytes(embedded_wasm)
        (consumer / 'embedded.yaml').write_bytes(embedded_scene)
        script = """
const fs=require('fs'),assert=require('assert/strict');
(async()=>{const w=(await WebAssembly.instantiate(fs.readFileSync('embedded.wasm'),{})).instance.exports;
const scene=fs.readFileSync('embedded.yaml');
new Uint8Array(w.memory.buffer,w.defense_scene_buffer(),scene.length).set(scene);
assert.equal(w.defense_init_scene(scene.length),0);
const read=()=>{const p=w.defense_snapshot();return JSON.parse(new TextDecoder().decode(new Uint8Array(w.memory.buffer,p,w.defense_snapshot_len())))};
for(let run=0;run<2;run++){
 const initial=read();assert.equal(initial.tick,0);if(run===0)assert.equal(initial.checksum,process.argv[1]);
 assert.equal(initial.hp,75);assert.equal(initial.lastWave,3);assert.deepEqual(initial.gold,[250,250]);
 for(let tick=1;tick<=210;tick++)w.defense_step(0,0,0,0);
 const s=read();assert.equal(s.tick,210);assert.equal(s.enemies.length,2);
 assert.deepEqual(s.enemies.map(e=>e[1]),[0,1]);
 // First wave, first pair is FAST: base HP 24 + 1*15, multiplied by four.
 for(const e of s.enemies){assert.equal(e[3],156);assert.equal(e[4],156);}
 if(run===0)w.defense_step(5,0,0,1);
}
console.log(JSON.stringify({passed:true}));
})().catch(e=>{console.error(e);process.exitCode=1});
"""
        result = subprocess.run(['node', '-e', script, receipt['initialChecksum']], cwd=consumer,
                                capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(json.loads(result.stdout)['passed'])
        self.assertEqual(self.sentinel.read_bytes(), b'keep')

    def test_native_invalid_wave_pairs_never_publish(self):
        invalid = ['', '1', '1,2,3,4,5,6,7,8,9', '1,2,3,4,5,6,7,8,9,10,11',
                   ',2,3,4,5,6,7,8,9,10', '1,2,3,4,5,6,7,8,9,',
                   '1,2,3,4,,6,7,8,9,10', '1,2,3,4,5,6,7,8,9,0',
                   '1,2,3,4,5,6,7,8,9,29', '-1,2,3,4,5,6,7,8,9,10',
                   '1.0,2,3,4,5,6,7,8,9,10', 'true,2,3,4,5,6,7,8,9,10',
                   '١,2,3,4,5,6,7,8,9,10', '1,2,3,4,5,6,7,8,9,10\nother: 1',
                   '1,2,3,4,5,6,7,8,9,18446744073709551616']
        cases = [['--wave-pairs', value] for value in invalid]
        valid = '1,2,3,4,5,6,7,8,9,10'
        cases += [['--wave-pairs'], ['--wave-pairs', valid, '--wave-pairs', valid]]
        for extra in cases:
            with self.subTest(extra=extra):
                output = self.root / 'refused-counts.yaml'
                result = self.execute(self.command(True, output, extra))
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertFalse(os.path.lexists(output))
                self.assertEqual(self.wasm.read_bytes(), self.wasm_bytes)
                self.assertEqual(self.sentinel.read_bytes(), b'keep')

    def test_native_saved_wave_pairs_control_spawn_totals_and_survive_restart(self):
        script = """
const fs=require('fs'),assert=require('assert/strict');
(async()=>{const w=(await WebAssembly.instantiate(fs.readFileSync(process.argv[1]),{})).instance.exports;
const scene=fs.readFileSync(process.argv[2]),pairs=Number(process.argv[3]),interval=70;
new Uint8Array(w.memory.buffer,w.defense_scene_buffer(),scene.length).set(scene);
assert.equal(w.defense_init_scene(scene.length),0);
const read=()=>{const p=w.defense_snapshot();return JSON.parse(new TextDecoder().decode(new Uint8Array(w.memory.buffer,p,w.defense_snapshot_len())))};
const first=Math.ceil(180/interval)*interval,traces=[];
for(let run=0;run<2;run++){
 const initial=read();assert.equal(initial.tick,0);assert.equal(initial.hp,1000);assert.equal(initial.lastWave,1);
 const ticks=[];
 for(let tick=1;tick<=first+pairs*interval;tick++){
  w.defense_step(0,0,0,0);const s=read();assert.equal(s.tick,tick);
  assert.equal(s.presentation.status,'complete');
  const added=s.presentation.events.filter(e=>e.kind==='enemy_spawn');
  if(added.length){assert.equal(added.length,2);assert.deepEqual(added.map(e=>e.board),[0,1]);ticks.push(tick);}
 }
 assert.deepEqual(ticks,Array.from({length:pairs},(_,i)=>first+i*interval));traces.push(ticks);
 if(run===0)w.defense_step(5,0,0,1);
}
console.log(JSON.stringify({pairs,traces}));
})().catch(e=>{console.error(e);process.exitCode=1});
"""
        for pairs in (1, 3, 28):
            with self.subTest(pairs=pairs):
                output = self.root / f'counted-restart-{pairs}.yaml'
                self.success(True, output, ['--reactor-hp', 1000, '--last-wave', 1,
                                            '--spawn-interval-ticks', 70,
                                            '--wave-pairs', ','.join([str(pairs)] * 10)])
                result = self.execute(['node', '-e', script, str(self.wasm), str(output), str(pairs)])
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(json.loads(result.stdout)['pairs'], pairs)

    def test_invalid_truncated_and_wrong_module_wasm_never_publish(self):
        # Empty valid WASM module is structurally valid but lacks required game ABI;
        # it is a bounded inert fixture, not an arbitrary third-party executable.
        for index, data in enumerate([b'not wasm', self.wasm_bytes[:7], self.wasm_bytes[:-1], b'\0asm\x01\0\0\0']):
            source = self.root / f'invalid-{index}.wasm'
            source.write_bytes(data)
            self.rejected_both(wasm=source)
        for name, data, reason in inert_wasm_admission_failures():
            source = self.root / f'{name}.wasm'
            source.write_bytes(data)
            for native in (False, True):
                output = self.root / f'{name}-{native}.yaml'
                result = self.execute(self.command(native, output, wasm=source))
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(os.path.lexists(output))
            # Legacy CLI wraps CalledProcessError without forwarding Node stderr.
            # Invoke the unchanged verifier directly to distinguish intended guard
            # rejection from a malformed handcrafted module, without product overrides.
            scene = self.root / f'{name}-input.yaml'
            scene.write_text(legacy.scene_text())
            checked = self.execute(['node', str(TOOLS / 'validate_coop_defense_scene.mjs'), str(source), str(scene)])
            self.assertNotEqual(checked.returncode, 0)
            self.assertIn(reason, checked.stderr)
            self.assertEqual(source.read_bytes(), data)

    def test_nofollow_fifo_and_existing_output_preservation(self):
        link = self.root / 'linked.wasm'
        link.symlink_to(self.wasm)
        fifo = self.root / 'fifo.wasm'
        os.mkfifo(fifo)
        for source in (link, fifo, self.root):
            self.rejected_both(wasm=source)
        for native in (False, True):
            for kind in ('file', 'directory', 'symlink'):
                output = self.root / f'existing-{native}-{kind}'
                if kind == 'file':
                    output.write_bytes(b'keep output')
                elif kind == 'directory':
                    output.mkdir()
                    (output / 'sentinel').write_bytes(b'keep directory')
                else:
                    output.symlink_to('absent')
                result = self.execute(self.command(native, output))
                self.assertNotEqual(result.returncode, 0, result.stdout)
                if kind == 'file':
                    self.assertEqual(output.read_bytes(), b'keep output')
                elif kind == 'directory':
                    self.assertEqual((output / 'sentinel').read_bytes(), b'keep directory')
                    self.assertEqual([p.name for p in output.iterdir()], ['sentinel'])
                else:
                    self.assertEqual(os.readlink(output), 'absent')
            alias = self.root / f'parent-alias-{native}'
            alias.symlink_to(self.root, target_is_directory=True)
            result = self.execute(self.command(native, alias / 'never-created.yaml'))
            self.assertNotEqual(result.returncode, 0, result.stdout)
            self.assertFalse((self.root / 'never-created.yaml').exists())
        self.assertEqual(self.wasm.read_bytes(), self.wasm_bytes)

    def test_native_unknown_execution_flags_are_usage_errors(self):
        for flag in ('--node', '--node-bin', '--command', '--script', '--executable'):
            output = self.root / 'no-output.yaml'
            result = self.execute(self.command(True, output, [flag, str(self.sentinel)]))
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertFalse(output.exists())
            self.assertEqual(self.sentinel.read_bytes(), b'keep')

    def test_actual_offline_html_binds_rust_scene_receipt_and_original_wasm(self):
        import base64
        scene = self.root / 'authored.scene.yaml'
        receipt = self.success(True, scene, ['--starting-gold', 250, '--reactor-hp', 75,
                                            '--last-wave', 3, '--spawn-interval-ticks', 70])
        web = self.root / 'actual-web'
        shutil.copytree(self.build, web)
        html = self.root / 'game.html'
        packer.package(web, scene=scene, output=html)
        parsed = EmbeddedScripts()
        parsed.feed(html.read_text())
        self.assertEqual(base64.b64decode(parsed.scripts['wasm-core'], validate=True), self.wasm_bytes)
        self.assertEqual(base64.b64decode(parsed.scripts['authored-scene'], validate=True), scene.read_bytes())
        self.assertEqual(json.loads(parsed.scripts['authored-scene-receipt']), receipt)
        self.assertEqual(packer.validate_scene(scene.read_bytes(), self.wasm_bytes), receipt)
        result = self.execute([sys.executable, '-B', str(TOOLS / 'check_coop_defense_offline.py'), str(web), str(html)])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((web / 'game.wasm').read_bytes(), self.wasm_bytes)
        self.assertNotIn(str(scene), html.read_text())
        self.assertNotIn(str(self.wasm), html.read_text())
        # The unchanged checker executes actual trusted-WASM scene admission and
        # Node syntax checking. It does not run a browser or certify rendered play.

    def test_counted_scene_same_build_native_and_legacy_offline_receipts(self):
        import base64
        scene = self.root / 'counted.scene.yaml'
        receipt = self.success(True, scene, ['--starting-gold', 250, '--reactor-hp', 75,
                                            '--last-wave', 3, '--spawn-interval-ticks', 70,
                                            '--wave-pairs', '1,2,3,4,5,6,7,8,9,28'])
        web = self.root / 'counted-web'
        shutil.copytree(self.build, web)
        for native in (False, True):
            with self.subTest(native=native):
                html = self.root / f'counted-{native}.html'
                if native:
                    result = self.execute([self.binary, 'pack-coop-offline', '--build-dir', str(web),
                                           '--scene', str(scene), '--output', str(html)])
                    self.assertEqual(result.returncode, 0, result.stderr)
                else:
                    packer.package(web, scene=scene, output=html)
                parsed = EmbeddedScripts()
                parsed.feed(html.read_text())
                self.assertEqual(base64.b64decode(parsed.scripts['wasm-core'], validate=True), self.wasm_bytes)
                self.assertEqual(base64.b64decode(parsed.scripts['authored-scene'], validate=True), scene.read_bytes())
                self.assertEqual(json.loads(parsed.scripts['authored-scene-receipt']), receipt)
                result = self.execute([sys.executable, '-B', str(TOOLS / 'check_coop_defense_offline.py'), str(web), str(html)])
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertTrue(json.loads(result.stdout)['passed'])
        self.assertEqual(packer.validate_scene(scene.read_bytes(), self.wasm_bytes), receipt)
        self.assertEqual((web / 'game.wasm').read_bytes(), self.wasm_bytes)
        self.assertEqual(self.sentinel.read_bytes(), b'keep')


if __name__ == '__main__':
    unittest.main()
