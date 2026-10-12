"""Fixed CPU-only contract negatives; no Cargo, window, adapter or network."""
import json
import os
from pathlib import Path
import struct
import sys
import tempfile
import time
import unittest
from unittest.mock import patch
import zlib

import material_sample_gate as gate


def chunk(kind, data):
    return struct.pack('>I', len(data))+kind+data+struct.pack('>I', zlib.crc32(kind+data)&0xffffffff)


def png(raw=b'\x00\x01\x02\x03\xff', shape=(1, 1), extra=b''):
    return b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',*shape,8,6,0,0,0))+extra+chunk(b'IDAT',zlib.compress(raw))+chunk(b'IEND',b'')


class MaterialGateTests(unittest.TestCase):
    def test_inventory_accepts_exact_and_rejects_missing_extra_duplicate(self):
        gate.inventory('a: test\nb: test\n2 tests, 0 benchmarks\n',['a','b'])
        for text in ['a: test\n1 test, 0 benchmarks\n','a: test\nb: test\nc: test\n3 tests, 0 benchmarks\n','a: test\na: test\n2 tests, 0 benchmarks\n']:
            with self.assertRaises(AssertionError): gate.inventory(text,['a','b'])

    def test_inventory_rejects_summary_mismatch(self):
        with self.assertRaises(AssertionError): gate.inventory('a: test\n2 tests, 0 benchmarks\n',['a'])

    def test_results_reject_wrong_counts_names_duplicates_and_skip(self):
        text='test a ... ok\ntest b ... ignored, device\ntest result: ok. 1 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.1s\n'
        gate.result(text,['a'],['b'],0)
        for bad in [text.replace('1 passed','2 passed'),text.replace('test a','test c'),text+'test a ... ok\n',text+'SKIP: device\n',text+text]:
            with self.assertRaises(AssertionError): gate.result(bad,['a'],['b'],0)

    def test_png_rgba_positive_all_five_filters(self):
        # First pixel has zero left/up context, so all predictors are zero.
        for filt in range(5): self.assertEqual(gate.png_rgba(png(bytes([filt,1,2,3,255])),(1,1)),bytes([1,2,3,255]))

        expected=bytes([10,20,30,40,50,60,70,80,90,100,110,120,130,140,150,160])
        # Independent literal PNG residuals for2×2 RGBA: nonzero left/up/
        # upper-left. Sub, Up, Average and Paeth cannot degrade to None.
        residuals=[
            [0,10,20,30,40,50,60,70,80,0,90,100,110,120,130,140,150,160],
            [1,10,20,30,40,40,40,40,40,1,90,100,110,120,40,40,40,40],
            [2,10,20,30,40,50,60,70,80,2,80,80,80,80,80,80,80,80],
            [3,10,20,30,40,45,50,55,60,3,85,90,95,100,60,60,60,60],
            [4,10,20,30,40,40,40,40,40,4,80,80,80,80,40,40,40,40],
        ]
        for data in residuals: self.assertEqual(gate.png_rgba(png(bytes(data),(2,2)),(2,2)),expected)
        # Last pixel's channels independently force Paeth upper-left, up,
        # left, and a deterministic tie, respectively.
        raw=bytes([0,100,20,100,0,80,100,20,0,4,20,10,166,0,80,20,90,0])
        self.assertEqual(gate.png_rgba(png(raw,(2,2)),(2,2)),bytes([100,20,100,0,80,100,20,0,120,30,10,0,180,120,100,0]))

    def test_png_rejects_crc_truncation_trailing_and_size(self):
        good=png()
        for bad in [good[:-1],good+b'x',good[:29]+bytes([good[29]^1])+good[30:],b'x'*(2*1024**2+1)]:
            with self.assertRaises(AssertionError): gate.png_rgba(bad,(1,1))

    def test_png_rejects_dimensions_format_and_filter(self):
        for data,shape in [(png(),(2,1)),(png(b'\x05\x01\x02\x03\xff'),(1,1)),(png(),(641,1))]:
            with self.assertRaises(AssertionError): gate.png_rgba(data,shape)

    def test_png_rejects_expansion_extra_stream_and_critical_chunk(self):
        bomb=png(b'\0'*10000)
        stream=b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',1,1,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(b'\0\1\2\3\xff')+zlib.compress(b'x'))+chunk(b'IEND',b'')
        for data in [bomb,stream,png(extra=chunk(b'XXXX',b'x')),png(extra=chunk(b'1EXt',b'x')),png(extra=chunk(b'tExt',b'x'))]:
            with self.assertRaises(AssertionError): gate.png_rgba(data,(1,1))

    def test_png_rejects_duplicate_header_and_interrupted_idat(self):
        header=chunk(b'IHDR',struct.pack('>IIBBBBB',1,1,8,6,0,0,0))
        malformed=b'\x89PNG\r\n\x1a\n'+header+chunk(b'IDAT',b'x')+chunk(b'tEXt',b'x')+chunk(b'IDAT',b'x')+chunk(b'IEND',b'')
        for data in [png(extra=header),malformed]:
            with self.assertRaises(AssertionError): gate.png_rgba(data,(1,1))

    def test_capture_file_size_and_symlink_rejection(self):
        with tempfile.TemporaryDirectory() as folder:
            p=Path(folder)/'raw';p.write_bytes(b'wrong')
            with self.assertRaises(AssertionError): gate.inspect(p,'raw')
            # Mock only the filesystem symlink predicate; Windows needs no
            # privileged symlink creation for this admission control.
            with patch.object(Path,'is_symlink',return_value=True):
                with self.assertRaises(AssertionError): gate.inspect(p,None)

    def test_capture_audit_requires_exact_files(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);(root/'static').mkdir()
            with patch.object(gate,'capture_root',root):
                with self.assertRaises(AssertionError): gate.audit('static')
                (root/'static'/'unexpected').write_text('x')
                with self.assertRaises(AssertionError): gate.audit('static')

    def test_capture_audit_static_positive_and_changed_baseline(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder); captures=root/'captures';(captures/'static').mkdir(parents=True);old=root/'old';old.mkdir()
            rows=(b'\0'+bytes([1,2,3,255])*320)*240
            baseline={}
            for name in ['particles-off.png','pickup-sparks.png','capture.txt']:
                data=png(rows,(320,240)) if name.endswith('.png') else b'size=320x240\n'
                (captures/'static'/name).write_bytes(data);(old/name).write_bytes(data);baseline['static/'+name]=old/name
            with patch.object(gate,'capture_root',captures),patch.object(gate,'baseline',baseline),patch.object(gate,'audits',[]):
                gate.audit('static')
                (old/'capture.txt').write_text('changed')
                with self.assertRaises(AssertionError): gate.audit('static')

    def test_material_manifest_rejects_missing_or_false_pass(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);d=root/'material-scene';d.mkdir()
            for key in gate.SHAPES:
                if key.startswith('material-scene/'): (root/key).write_bytes(b'x')
            with patch.object(gate,'capture_root',root),patch.object(gate,'inspect',return_value={}):
                for text in ['STATE=RUNNING;\n','STATE=PASS frames=9\n']:
                    (d/'manifest.txt').write_text(text)
                    with self.assertRaises(AssertionError): gate.audit('material')

    def test_grounding_manifest_rejects_missing_raw_hash(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);d=root/'grounding';d.mkdir()
            for key in gate.SHAPES:
                if key.startswith('grounding/'): (root/key).write_bytes(b'x')
            manifest='STATE=RUNNING partial\nadapter tuple\n'+'frame=bad\n'*18+'STATE=PASS frames=18 visual_review_pending\n'
            (d/'manifest.txt').write_text(manifest)
            with patch.object(gate,'capture_root',root),patch.object(gate,'adapter','tuple'),patch.object(gate,'inspect',return_value={}):
                with self.assertRaises(AssertionError): gate.audit('grounding')

    def test_full_manifest_positive_then_hash_pixels_and_metadata_mutations(self):
        # Metadata unit isolation: inspect() is mocked only here; real PNG
        # decode and raw size admission are covered separately above.
        for kind,directory,count in [('material','material-scene',10),('grounding','grounding',18)]:
            with tempfile.TemporaryDirectory() as folder:
                root=Path(folder);d=root/directory;d.mkdir();records={};lines=[]
                names=([f'{mode}-{stage}.png' for stage in ['initial','pickup','spread','expired','restart'] for mode in ['legacy','showcase']] if kind=='material' else gate.GROUNDING_NAMES)
                for name in names:
                    if kind=='material':
                        mode,stage=name[:-4].split('-',1);canonical=mode+'-'+('initial' if stage=='restart' else stage)
                        score=int(stage not in ('initial','restart'));particles=16 if stage in ('pickup','spread') else 0
                        elapsed={'initial':0,'restart':0,'pickup':1000,'spread':101000,'expired':701000}[stage]
                        raw=gate.digest(canonical.encode());encoded=gate.digest(('png'+canonical).encode())
                        records[name]={'bytes':100,'sha256':encoded,'raw_rgba_sha256':raw,'width':640,'height':480}
                        lines.append(f'frame={name} raw_rgba_sha256={raw} png_sha256={encoded} tick=0 checksum=0000000000000000 elapsed_us={elapsed} pickup_us=None score={score} particles={particles} actors={3-score} lighting=shadows: false')
                    else:
                        canonical='initial-on' if name in ('initial-repeat-on','initial-restart-on') else name
                        if name.startswith('pickup-700ms'): canonical='expired'
                        score=int(name.startswith('pickup-'));particles=16 if score and not name.startswith('pickup-700ms') else 0
                        raw=gate.digest(canonical.encode());encoded=gate.digest(('png'+canonical).encode())
                        records[name+'.rgba']={'bytes':1228800,'sha256':raw}
                        records[name+'.png']={'bytes':100,'sha256':encoded,'raw_rgba_sha256':raw,'width':640,'height':480}
                        lines.append(f'frame={name} raw={raw} png={encoded} player=[0, 0.2, 0] checksum=0000000000000000 score={score} particles={particles}')
                records['manifest.txt']={'bytes':100,'sha256':'a'*64}
                for key in records: (d/key).write_bytes(b'x')
                header=('STATE=RUNNING;\nadapter tuple\neye_offset=2.4,3.6,-1.8 shadows=true radius=6.5\n' if kind=='material' else 'STATE=RUNNING partial\nadapter tuple\n')
                trailer=('STATE=PASS frames=10\n' if kind=='material' else 'STATE=PASS frames=18 visual_review_pending\n')
                manifest=header+'\n'.join(lines)+'\n'+trailer;(d/'manifest.txt').write_text(manifest)
                with patch.object(gate,'capture_root',root),patch.object(gate,'adapter','tuple'),patch.object(gate,'audits',[]),patch.object(gate,'inspect',side_effect=lambda path,shape:dict(records[path.name])):
                    gate.audit(kind)
                    # Wrong encoded/raw hash is rejected after the valid prefix.
                    first=names[0];image=first if kind=='material' else first+'.png'
                    old=records[image]['raw_rgba_sha256'];records[image]['raw_rgba_sha256']='f'*64
                    with self.assertRaises(AssertionError): gate.audit(kind)
                    records[image]['raw_rgba_sha256']=old
                    (d/'manifest.txt').write_text(manifest.replace('checksum=0000000000000000','checksum=1111111111111111',1))
                    with self.assertRaises(AssertionError): gate.audit(kind)

    def test_adapter_requires_one_full_software_vulkan_tuple(self):
        valid='name="Device" backend=Vulkan device_type=Cpu vendor=1 device=2 driver="driver" driver_info="" software=true'
        self.assertEqual(gate.observed_adapter('material sample adapter: '+valid,'full'),valid)
        for text in ['',valid.replace('software=true','software=false'),valid.replace('Vulkan','Gl')]:
            with self.assertRaises(AssertionError): gate.observed_adapter('material sample adapter: '+text,'full')
        with self.assertRaises(AssertionError): gate.observed_adapter(('material sample adapter: '+valid+'\n')*2,'full')

    def test_fixed_geometry_controls_reject_vacuous_regions(self):
        row='grounding eligible=400 dark=400 lit=500 shadow_inset_world=0.06 lit_outset_world=0.12 object_band_px=2\n'
        gate.fixture_controls({'name':'grounding'},row*5)
        for text in [row*4,row.replace('dark=400','dark=7')*5,row.replace('lit=500','lit=499')*5]:
            with self.assertRaises(AssertionError): gate.fixture_controls({'name':'grounding'},text)

    def test_fixed_inventory_and_work_census(self):
        self.assertEqual((len(gate.NAMES),len(gate.CPU),len(gate.IGNORED)),(33,26,7))
        self.assertEqual((len(gate.SHAPES),sum(k.endswith('.png') for k in gate.SHAPES)),(62,40))
        self.assertEqual(len(gate.GROUNDING_NAMES),18)

    def test_subprocess_success_and_nonzero_receipts(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder)
            text=gate.bounded_run([sys.executable,'-c','print("hello")'],root,os.environ.copy(),root/'ok.log',10,disk_floor=0)
            self.assertEqual(text.strip(),'hello')
            with self.assertRaises(AssertionError): gate.bounded_run([sys.executable,'-c','raise SystemExit(3)'],root,os.environ.copy(),root/'bad.log',10,disk_floor=0)
            self.assertEqual(json.loads((root/'bad.json').read_text())['exit'],3)
            # This branch is portable: the fake Job rejects assignment before
            # the gated launcher can execute the target on either host OS.
            with patch.object(gate.os,'name','nt'),patch.object(gate,'WindowsJob') as job:
                job.return_value.assign.side_effect=OSError('assignment denied')
                with self.assertRaises(AssertionError): gate.bounded_run([sys.executable,'-c','print("must not run")'],root,os.environ.copy(),root/'assign.log',10,disk_floor=0)
                job.return_value.close.assert_called_once()
            receipt=json.loads((root/'assign.json').read_text());self.assertTrue(receipt['owned_children_reaped']);self.assertIn('assignment denied',receipt['stop_reason'])

    def test_subprocess_timeout_kills_descendant_before_sentinel(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);sentinel=root/'escaped'
            child='import time,pathlib; time.sleep(3); pathlib.Path('+repr(str(sentinel))+').write_text("escaped")'
            parent='import subprocess,sys,time; subprocess.Popen([sys.executable,"-c",'+repr(child)+']); print("spawned",flush=True); time.sleep(20)'
            with self.assertRaises(AssertionError): gate.bounded_run([sys.executable,'-c',parent],root,os.environ.copy(),root/'timeout.log',2,disk_floor=0)
            receipt=json.loads((root/'timeout.json').read_text());self.assertEqual(receipt['stop_reason'],'timeout');self.assertTrue(receipt['owned_children_reaped'])
            self.assertIn('spawned',(root/'timeout.log').read_text());time.sleep(3.2);self.assertFalse(sentinel.exists())

    def test_subprocess_output_cap_preserves_failure(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder)
            with patch.object(gate,'LOG_CAP',128):
                with self.assertRaises(AssertionError): gate.bounded_run([sys.executable,'-c','print("x"*1024)'],root,os.environ.copy(),root/'cap.log',10,disk_floor=0)
            receipt=json.loads((root/'cap.json').read_text());self.assertEqual(receipt['stop_reason'],'log cap');self.assertLessEqual((root/'cap.log').stat().st_size,128)


if __name__ == '__main__':
    unittest.main()
