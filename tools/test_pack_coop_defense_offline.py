#!/usr/bin/env python3
"""Portable regressions for the optional web package's exact module closure."""
import base64
import json
import hashlib
import io
import struct
import zlib
import os
import subprocess
from unittest import mock
import importlib.util
from pathlib import Path
import shutil
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("packer", Path(__file__).with_name("pack_coop_defense_offline.py"))
packer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(packer)
WEB = Path(__file__).resolve().parents[1] / "crates/orr_coop_defense_web/web"


class OfflineModules(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        for name in (*packer.MODULES, *packer.EXTRA_MODULES, "game.js", "index.html", "style.css"):
            (self.root / name).parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(WEB / name, self.root / name)
        (self.root / "game.wasm").write_bytes(b"test fixture, not executable WASM")
        (self.root / "assets").mkdir()
        for name, data in {"characters.json": b'{"heroes": [], "enemies": [], "label": "</script>"}', "units.png": b"heroes", "enemies.png": b"enemies"}.items():
            (self.root / "assets" / name).write_bytes(data)

    def test_exact_reviewed_modules_are_embedded_once(self):
        module = packer.bundle_game(self.root)
        for name in packer.MODULES:
            self.assertEqual(module.count((self.root / name).read_text()), 1)
        self.assertNotIn(packer.GAME_IMPORTS, module)
        html = packer.package(self.root).read_text()
        self.assertIn(module, html)
        self.assertNotIn('<script type="module" src=', html)
        self.assertIn('\\u003c/script>', html)

    def test_missing_dependency_fails(self):
        (self.root / packer.MODULES[0]).unlink()
        with self.assertRaises(FileNotFoundError): packer.package(self.root)
        self.assertFalse((self.root / "stormglass-foundry-offline.html").exists())

    def test_unknown_static_dynamic_and_reexport_dependencies_fail(self):
        original = (self.root / packer.MODULES[0]).read_text()
        for extra in ["import { x } from './missing.mjs';", "await import('./missing.mjs');", "export * from './missing.mjs';",
                      "import/* comment */ './missing.mjs';",
                      "export/* comment */ { x } from './missing.mjs';",
                      "await import/* comment */('./missing.mjs');",
                      "export/**/ * /**/ from './missing.mjs';",
                      "import\n// comment\n'./missing.mjs';"]:
            with self.subTest(extra=extra):
                (self.root / packer.MODULES[0]).write_text(original + '\n' + extra)
                with self.assertRaisesRegex(ValueError, "dependency"): packer.package(self.root)

    def test_changed_entry_graph_fails(self):
        (self.root / "game.js").write_text("import './extra.mjs';\n" + (self.root / "game.js").read_text())
        with self.assertRaisesRegex(ValueError, "import graph"): packer.package(self.root)

    def test_duplicate_anchor_fails(self):
        p = self.root / "index.html"; p.write_text(p.read_text() + '<script type="module" src="game.js"></script>')
        with self.assertRaisesRegex(ValueError, "one stylesheet"): packer.package(self.root)

    def test_script_and_style_breakouts_fail(self):
        p = self.root / "game.js"; original = p.read_text(); p.write_text(original + "\n// </script>")
        with self.assertRaisesRegex(ValueError, "terminate"): packer.package(self.root)
        p.write_text(original)
        p = self.root / "style.css"; p.write_text(p.read_text() + "/* </style> */")
        with self.assertRaisesRegex(ValueError, "terminate"): packer.package(self.root)

    def authored(self):
        scene = self.root / 'saved.scene.yaml'
        scene.write_text('schema: orr.scene/1\nentities: {}\nsingletons: {CoopDefense::Scenario: {starting_gold: 250, reactor_hp: 75}}\n', encoding='utf-8')
        return scene, self.root / 'delivered.html'

    def test_authored_requires_new_explicit_destination(self):
        scene, out = self.authored()
        with self.assertRaises(ValueError): packer.package(self.root, scene)
        for destination in [scene, self.root / 'game.wasm', self.root / 'assets']:
            with self.assertRaises(FileExistsError): packer.package(self.root, scene, destination)
        out.symlink_to(scene)
        with self.assertRaises(FileExistsError): packer.package(self.root, scene, out)

    def test_nonregular_symlink_and_bounded_inputs(self):
        scene, out = self.authored()
        for data in [b'x' * 65537, b'\xff']:
            scene.write_bytes(data)
            with self.assertRaises((ValueError, UnicodeDecodeError)): packer.package(self.root, scene, out)
            self.assertFalse(out.exists())
        scene.unlink(); scene.symlink_to(self.root / 'index.html')
        with self.assertRaises(OSError): packer.package(self.root, scene, out)
        scene.unlink(); os.mkfifo(scene)
        with self.assertRaises(ValueError): packer.package(self.root, scene, out)
        scene.unlink(); scene.write_bytes(b'valid')
        asset = self.root / 'assets/units.png'; asset.unlink(); asset.symlink_to(self.root / 'game.wasm')
        with self.assertRaises(OSError): packer.package(self.root, scene, out)

    def test_directory_symlink_and_midread_mutation_rejected(self):
        scene, out = self.authored()
        linked = self.root / 'linked'; linked.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(OSError): packer.package(linked, scene, out)
        with self.assertRaises(OSError): packer.publish_new(linked / 'new.html', 'content')
        before = scene.stat()
        changed = mock.Mock(st_size=before.st_size, st_mtime_ns=before.st_mtime_ns + 1, st_ctime_ns=before.st_ctime_ns)
        with mock.patch.object(packer.os, 'fstat', side_effect=[before, changed]):
            with self.assertRaisesRegex(ValueError, 'changed'): packer.snapshot_file(scene, 65536)

    def test_snapshotted_bytes_not_reread_after_validation(self):
        scene, out = self.authored(); original = scene.read_bytes()
        def validate(data, wasm):
            self.assertEqual(data, original)
            scene.write_bytes(b'changed after validation')
            return {'version': 1, 'initialChecksum': '0'*16}
        with mock.patch.object(packer, 'validate_scene', side_effect=validate): packer.package(self.root, scene, out)
        self.assertIn(base64.b64encode(original).decode(), out.read_text())
        self.assertNotIn(base64.b64encode(scene.read_bytes()).decode(), out.read_text())

    def test_validator_failures_leave_no_output(self):
        scene, out = self.authored()
        for failure in [FileNotFoundError('node missing'), subprocess.TimeoutExpired('node',15), subprocess.CalledProcessError(1,'node')]:
            with mock.patch.object(packer.subprocess, 'run', side_effect=failure):
                with self.assertRaises(type(failure)): packer.package(self.root, scene, out)
            self.assertFalse(out.exists())

    def test_publication_race_and_failure_preserve_destination(self):
        scene, out = self.authored()
        def validate(*_):
            out.write_bytes(b'other writer')
            return {'version': 1}
        with mock.patch.object(packer, 'validate_scene', side_effect=validate):
            with self.assertRaises(FileExistsError): packer.package(self.root, scene, out)
        self.assertEqual(out.read_bytes(), b'other writer')
        self.assertEqual(list(self.root.glob('.orr-offline-*')), [])
        out.unlink()
        with mock.patch.object(packer, 'validate_scene', return_value={'version':1}), mock.patch.object(packer.os, 'link', side_effect=OSError('publication failed')):
            with self.assertRaises(OSError): packer.package(self.root, scene, out)
        self.assertFalse(out.exists()); self.assertEqual(list(self.root.glob('.orr-offline-*')), [])

    @unittest.skipUnless(os.environ.get('ORR_DEFENSE_WASM'), 'actual WASM not provided')
    def test_actual_wasm_scene_admission_and_receipt(self):
        scene, out = self.authored()
        shutil.copyfile(os.environ['ORR_DEFENSE_WASM'], self.root / 'game.wasm')
        scene.write_text(scene.read_text() + '# Unicode 한글 </script>\n', encoding='utf-8')
        exact = scene.read_bytes() + b'#' + b'x' * (65536 - len(scene.read_bytes()) - 1)
        self.assertEqual(packer.validate_scene(exact, (self.root/'game.wasm').read_bytes())['sceneBytes'], 65536)
        receipt = packer.validate_scene(scene.read_bytes(), (self.root / 'game.wasm').read_bytes())
        self.assertEqual(receipt['sceneBytes'], len(scene.read_bytes()))
        packer.package(self.root, scene, out)
        self.assertIn(receipt['initialChecksum'], out.read_text())
        self.assertNotIn('Unicode 한글', out.read_text())
        # Python accepts nonzero base64 pad bits; runtime intentionally rejects them.
        payload = scene.read_bytes()
        while len(payload) % 3 == 0:
            payload += b' '
        scene.write_bytes(payload)
        canonical = base64.b64encode(payload).decode('ascii')
        alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'
        last = len(canonical.rstrip('=')) - 1
        corrupted = canonical[:last] + alphabet[alphabet.index(canonical[last]) | 1] + canonical[last+1:]
        self.assertEqual(base64.b64decode(corrupted, validate=True), payload)
        out.unlink(); packer.package(self.root, scene, out)
        out.write_text(out.read_text().replace(canonical, corrupted))
        checked = subprocess.run(['python3', str(Path(__file__).with_name('check_coop_defense_offline.py')), str(self.root), str(out)], capture_output=True, text=True)
        self.assertNotEqual(checked.returncode, 0)
        self.assertIn('noncanonical authored scene base64', checked.stderr)

        for bad in [b'', b'schema: wrong', scene.read_bytes().replace(b'reactor_hp: 75', b'reactor_hp: 0')]:
            with self.assertRaises(subprocess.CalledProcessError): packer.validate_scene(bad, (self.root/'game.wasm').read_bytes())
        with self.assertRaises(subprocess.CalledProcessError): packer.validate_scene(scene.read_bytes(), b'bad wasm')


    def animations(self, manifest=None):
        from PIL import Image
        png = io.BytesIO()
        image = Image.new('RGBA', (8, 4), (240, 40, 40, 255))
        image.paste((20, 180, 240, 255), (4, 0, 8, 4))
        image.save(png, format='PNG'); image.close()
        (self.root / 'assets/sprite.png').write_bytes(png.getvalue())
        manifest = manifest or {'unit:0': {'image': 'sprite.png', 'anchor': [.5, 1.16], 'pivot': [-1, 4], 'logicalSize': [8, 8], 'clips': {
            'idle': {'frames': [[0, 0, 4, 4], [4, 0, 4, 4]], 'fps': 12, 'loop': True, 'pivots': [[1, 3], [2, 4]]},
            'attack': {'frames': [[4, 0, 4, 4]], 'windupMs': 100, 'releaseMs': 70, 'recoveryMs': 160},
            'death': {'frames': [[4, 0, 4, 4], [0, 0, 4, 4]], 'loop': False}}}}
        path = self.root / 'animations.json'
        path.write_text(json.dumps(manifest), encoding='utf-8')
        return path, self.root / 'animated.html'

    def test_animations_exact_bytes_and_closed_inventory(self):
        source, output = self.animations()
        (self.root / 'assets/unused.png').write_bytes(b'not PNG, deliberately unreferenced')
        original = {p: p.read_bytes() for p in (source, self.root / 'assets/sprite.png')}
        with mock.patch.object(packer, 'snapshot_file', wraps=packer.snapshot_file) as snapshots:
            packer.package(self.root, output=output, animations=source)
        html = output.read_text()
        for path, data in original.items():
            self.assertEqual(path.read_bytes(), data)
            self.assertIn(base64.b64encode(data).decode(), html)
            self.assertIn(hashlib.sha256(data).hexdigest(), html)
            self.assertEqual(sum(Path(call.args[0]) == path for call in snapshots.call_args_list), 1)
        self.assertNotIn('not PNG, deliberately unreferenced', html)
        self.assertIn('authored-animation-image-0', html)
        self.assertNotIn('authored-animation-image-1', html)
        (self.root / 'assets/characters.json').write_text(json.dumps({'heroes': [[]]*6, 'enemies': [[]]*4}))
        output.unlink(); packer.package(self.root, output=output, animations=source)
        result = subprocess.run(['python3', str(Path(__file__).with_name('check_coop_defense_offline.py')), str(self.root), str(output)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        output.write_text(output.read_text().replace('"width":8', '"width":7'))
        result = subprocess.run(['python3', str(Path(__file__).with_name('check_coop_defense_offline.py')), str(self.root), str(output)], capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)

    def test_animation_new_output_and_scene_composition(self):
        source, output = self.animations(); scene, _ = self.authored()
        with self.assertRaises(ValueError): packer.package(self.root, animations=source)
        with mock.patch.object(packer, 'validate_scene', return_value={'version': 1}):
            packer.package(self.root, scene, output, source)
        self.assertIn('id="authored-scene"', output.read_text())
        before = output.read_bytes()
        with self.assertRaises(FileExistsError): packer.package(self.root, output=output, animations=source)
        self.assertEqual(output.read_bytes(), before)
        for path in [source, self.root / 'assets/sprite.png']:
            with self.assertRaises(FileExistsError): packer.package(self.root, output=path, animations=source)

    def test_animation_path_and_schema_rejections(self):
        source, output = self.animations(); valid = json.loads(source.read_text())
        def rejected(spec):
            source.write_text(json.dumps(spec))
            with self.assertRaises((ValueError, TypeError, OverflowError)):
                packer.package(self.root, output=output, animations=source)
            self.assertFalse(output.exists())
        for path in ['', '../sprite.png', '/sprite.png', 'https://host/x.png', 'file:///x.png', 'data:image/png;base64,AA==', r'a\sprite.png', 'a//sprite.png', './sprite.png', 'a/../sprite.png', '%2e%2e/x.png', 'a'*125+'.png', 'sprite.jpg']:
            spec = json.loads(json.dumps(valid)); spec['unit:0']['image'] = path; rejected(spec)
        for role in ['unit:6', 'enemy:5', '__proto__', 'unit:01', 'unit:-1']:
            rejected({role: valid['unit:0']})
        rejected({}); rejected({'src': 'manifest.json'}); rejected([])
        changes = [
            ('unknown', 1), ('anchor', [0, 17]), ('pivot', [0, 4097]), ('logicalSize', [0, 1]),
            ('logicalSize', [1, 4097]), ('anchor', [True, 0]), ('pivot', None),
            ('clips', {'unknown': {'frames': [[0, 0, 1, 1]]}}), ('clips', {}),
        ]
        for field, value in changes:
            spec = json.loads(json.dumps(valid)); spec['unit:0'][field] = value; rejected(spec)
        for field, value in [('fps', 0), ('fps', 121), ('fps', True), ('fps', float('inf')), ('loop', 1), ('frames', []), ('frames', [[0,0,1,1]]*257), ('frames', [[7,0,2,4]]), ('frames', [[-1,0,2,4]]), ('frames', [[0,0,0,1]]), ('frames', [[0,0,1]]), ('frames', [[0,0,float('nan'),1]]), ('pivots', [[1,2]]), ('pivots', [[1,2],[1,4097]]), ('windupMs', 100), ('src', 'other.json')]:
            spec = json.loads(json.dumps(valid)); spec['unit:0']['clips']['idle'][field] = value; rejected(spec)
        for value in [0, 2001, '10', False]:
            spec=json.loads(json.dumps(valid)); spec['unit:0']['clips']['attack']['releaseMs']=value; rejected(spec)
        for raw in [b'{}', b'\xff', b'[]', b'{"unit:0":{},"unit:0":{}}', b'{"src":"elsewhere"}', b'{"unit:0":{"image":"sprite.png","image":"units.png","clips":{}}}', b'['*10000+b']'*10000, b' '*65537]:
            source.write_bytes(raw)
            with self.assertRaises(ValueError): packer.package(self.root, output=output, animations=source)
            self.assertFalse(output.exists())

    def test_animation_exact_limits_and_single_snapshot_shared_atlas(self):
        source, output = self.animations()
        manifest = {'unit:0': {'image': 'units.png', 'pivot': [-4096,4096], 'anchor': [-16,16], 'logicalSize': [.5,4096], 'clips': {'idle': {'frames': [[0,0,.5,.5]]*256,'fps':120}}}}
        source.write_text(json.dumps(manifest)); source.write_bytes(source.read_bytes() + b' '*(65536-source.stat().st_size))
        (self.root/'assets/units.png').write_bytes((self.root/'assets/sprite.png').read_bytes())
        with mock.patch.object(packer, 'snapshot_file', wraps=packer.snapshot_file) as snapshots:
            packer.package(self.root, output=output, animations=source)
        self.assertEqual(sum(Path(call.args[0]) == self.root/'assets/units.png' for call in snapshots.call_args_list), 1)
        self.assertEqual(source.stat().st_size, 65536)
        self.assertEqual((packer.MAX_IMAGE,packer.MAX_IMAGES_TOTAL,packer.MAX_IMAGES,packer.MAX_DIMENSION), (8*1024*1024,32*1024*1024,11,4096))
        image=(self.root/'assets/sprite.png').read_bytes()
        eleven={f'unit:{i}':{'image':f'{i}.png','clips':{'idle':{'frames':[[0,0,1,1]]}}} for i in range(6)}
        eleven.update({f'enemy:{i}':{'image':f'{i+6}.png','clips':{'idle':{'frames':[[0,0,1,1]]}}} for i in range(5)})
        receipt=packer.validate_animations(json.dumps(eleven).encode(),{f'{i}.png':image for i in range(11)})
        self.assertEqual(len(receipt['images']),11)

    def test_animation_file_types_byte_limits_and_missing_reference(self):
        source, output = self.animations(); image=self.root/'assets/sprite.png'; original=image.read_bytes()
        for kind in ['missing','symlink','fifo','directory','oversized']:
            if image.exists() or image.is_symlink():
                if image.is_dir(): image.rmdir()
                else: image.unlink()
            if kind=='symlink': image.symlink_to(self.root/'index.html')
            elif kind=='fifo': os.mkfifo(image)
            elif kind=='directory': image.mkdir()
            elif kind=='oversized':
                with image.open('wb') as stream: stream.truncate(packer.MAX_IMAGE+1)
            with self.assertRaises((ValueError,OSError)): packer.package(self.root, output=output, animations=source)
            self.assertFalse(output.exists())
        image.unlink(); image.write_bytes(original)
        with mock.patch.object(packer, 'MAX_IMAGES_TOTAL', len(original)-1):
            with self.assertRaisesRegex(ValueError, 'byte limit'): packer.package(self.root, output=output, animations=source)
        nested=self.root/'assets/nested'; nested.symlink_to(self.root/'assets',target_is_directory=True)
        manifest=json.loads(source.read_text()); manifest['unit:0']['image']='nested/sprite.png'; source.write_text(json.dumps(manifest))
        with self.assertRaises(OSError): packer.package(self.root, output=output, animations=source)
        source.unlink(); source.symlink_to(self.root/'index.html')
        with self.assertRaises(OSError): packer.package(self.root, output=output, animations=source)
        source.unlink(); os.mkfifo(source)
        with self.assertRaises(ValueError): packer.package(self.root, output=output, animations=source)

    def test_png_full_decode_crc_bounds_and_static_only(self):
        source, output=self.animations(); path=self.root/'assets/sprite.png'; png=path.read_bytes()
        def chunk(kind,data): return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data))
        ihdr=chunk(b'IHDR',struct.pack('>IIBBBBB',8,4,8,6,0,0,0))
        end=chunk(b'IEND',b'')
        invalid_deflate=b'\x89PNG\r\n\x1a\n'+ihdr+chunk(b'IDAT',b'invalid deflate, valid CRC')+end
        too_wide=b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',4097,4,8,6,0,0,0))+chunk(b'IDAT',b'x')+end
        apng=png[:33]+chunk(b'acTL',struct.pack('>II',1,0))+png[33:]
        unknown_critical=png[:33]+chunk(b'ZZZZ',b'')+png[33:]
        for data in [b'not PNG',png[:-1],png+b'trailing',png[:-5]+b'X'+png[-4:],invalid_deflate,too_wide,apng,unknown_critical]:
            path.write_bytes(data)
            with self.assertRaises((ValueError,OSError,SyntaxError)): packer.package(self.root, output=output, animations=source)
            self.assertFalse(output.exists())
        # Exact maximum encoded size uses an inert ancillary chunk, not recompression.
        padding=packer.MAX_IMAGE-len(png)-12
        exact=png[:-12]+chunk(b'npAD',b'\x00'*padding)+end
        self.assertEqual(len(exact),packer.MAX_IMAGE);self.assertEqual(packer.validate_png(exact),(8,4))
        from PIL import Image
        buffer=io.BytesIO()
        with Image.new('1',(4096,4096)) as image: image.save(buffer,format='PNG')
        self.assertEqual(packer.validate_png(buffer.getvalue()),(4096,4096))

    def test_animation_snapshot_and_publication_fail_closed(self):
        source, output=self.animations(); image=self.root/'assets/sprite.png'; original=image.read_bytes(); validate=packer.validate_animations
        def mutate(data,images,sampler):
            self.assertEqual(images['sprite.png'],original)
            image.write_bytes(b'changed after snapshot');source.write_bytes(b'changed after snapshot')
            return validate(data,images,sampler)
        with mock.patch.object(packer,'validate_animations',side_effect=mutate): packer.package(self.root,output=output,animations=source)
        self.assertIn(base64.b64encode(original).decode(),output.read_text())
        output.unlink();source,output=self.animations()
        def race(data,images,sampler):
            output.write_text('other writer')
            return validate(data,images,sampler)
        with mock.patch.object(packer,'validate_animations',side_effect=race):
            with self.assertRaises(FileExistsError): packer.package(self.root,output=output,animations=source)
        self.assertEqual(output.read_text(),'other writer');self.assertEqual(list(self.root.glob('.orr-offline-*')),[])
        output.unlink()
        for failure in [FileNotFoundError('node missing'),subprocess.TimeoutExpired('node',15),subprocess.CalledProcessError(1,'node')]:
            with mock.patch.object(packer.subprocess,'run',side_effect=failure):
                with self.assertRaises(type(failure)): packer.package(self.root,output=output,animations=source)
            self.assertFalse(output.exists())

    def test_animation_midread_mutation_leaves_no_output(self):
        source,output=self.animations();image=self.root/'assets/sprite.png';snapshot=packer.snapshot_file
        before=image.stat();after=mock.Mock(st_size=before.st_size,st_mtime_ns=before.st_mtime_ns+1,st_ctime_ns=before.st_ctime_ns)
        def changed(path,limit):
            if Path(path)==image:
                with mock.patch.object(packer.os,'fstat',side_effect=[before,after]):return snapshot(path,limit)
            return snapshot(path,limit)
        with mock.patch.object(packer,'snapshot_file',side_effect=changed):
            with self.assertRaisesRegex(ValueError,'changed'):packer.package(self.root,output=output,animations=source)
        self.assertFalse(output.exists())

    def test_no_animation_does_not_require_pillow(self):
        import builtins
        original=builtins.__import__
        def unavailable(name,*args,**kwargs):
            if name=='PIL' or name.startswith('PIL.'):raise ImportError('Pillow deliberately unavailable')
            return original(name,*args,**kwargs)
        with mock.patch.object(builtins,'__import__',side_effect=unavailable):
            self.assertTrue(packer.package(self.root).exists())

    def test_animation_validator_uses_build_sampler_snapshot(self):
        source,output=self.animations()
        sampler=self.root/'combat-presentation.mjs'
        sampler.write_text(sampler.read_text().replace("const raw = Math.floor", "const raw = 1 + Math.floor"))
        with self.assertRaises(subprocess.CalledProcessError): packer.package(self.root,output=output,animations=source)
        self.assertFalse(output.exists())

    def test_animation_duplicate_ids_and_existing_served_hook_rejected(self):
        source,output=self.animations(); index=self.root/'index.html'; original=index.read_text()
        for suffix in ['<div id="authored-animation-manifest"></div>','<div id="authored-animation-image-0"></div>', '<script id="combat-animation-manifest">{}</script>', '<div id="wasm-core"></div>','<div id="ready"></div>','<div id="new" id="new"></div>']:
            index.write_text(original+suffix)
            with self.assertRaises(ValueError): packer.package(self.root,output=output,animations=source)
            self.assertFalse(output.exists())


if __name__ == "__main__": unittest.main()
