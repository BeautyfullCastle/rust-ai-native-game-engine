#!/usr/bin/env python3
"""Mandatory optional material sample gates. Standard library; no device in cpu phase."""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import shutil
import signal
import struct
import subprocess
import sys
import threading
import time
import zlib

LOG_CAP = 16 * 1024 * 1024
CAPTURE_CAP = 128 * 1024 * 1024
DISK_FLOOR = 4 * 1024**3
FEATURES = {"material": "material-demo", "combined": "material-demo,orr_render/animation,orr_render/irradiance-probes"}
NAMES = ['arena_audio::tests::audio_modes_are_explicit_and_off_never_needs_a_device', 'arena_audio::tests::missing_native_feature_is_reported_and_required_fails', 'net_client::wss_options_tests::cli_parses_wss_ca_and_name_without_changing_quic_default', 'net_client::wss_options_tests::wss_defaults_to_webpki_and_url_name', 'net_client::wss_options_tests::wt_cli_defaults_to_webpki_and_url_host', 'particle_demo::material_scene::tests::actual_floor_occludes_behind_particle_but_keeps_above_control', 'particle_demo::material_scene::tests::actual_grounded_scene_tracks_player_and_preserves_shadow_controls', 'particle_demo::material_scene::tests::actual_isolated_material_silhouettes_match_independent_projection', 'particle_demo::material_scene::tests::actual_material_scene_preserves_pickup_and_expired_particle_baseline', 'particle_demo::material_scene::tests::actual_metallic_and_roughness_sensitivity_changes_only_one_parameter', 'particle_demo::material_scene::tests::explicit_material_cli_rejects_invalid_before_runtime', 'particle_demo::material_scene::tests::grounded_camera_floor_and_shadow_bounds_cover_playable_extrema', 'particle_demo::material_scene::tests::grounded_follow_preserves_authoritative_movement_events_and_restart', 'particle_demo::material_scene::tests::immutable_extraction_tracks_real_pickup_restart_and_particle_parity', 'particle_demo::material_scene::tests::independent_projection_and_billboard_controls_have_fixed_margins', 'particle_demo::material_scene::tests::original_models_have_bounded_outward_geometry_and_cooked_provenance', 'particle_demo::material_scene::tests::semantic_actor_tags_and_bounds_are_explicit', 'particle_demo::showcase_tests::actual_temporal_dust_and_sparks_spread_fade_and_expire', 'particle_demo::showcase_tests::explicit_restart_clears_old_cycle_and_zero_time_does_not_restart_boundary', 'particle_demo::showcase_tests::focus_pause_and_zero_delta_preserve_time_state_and_particles', 'particle_demo::showcase_tests::genuine_pickup_cycles_keep_modes_authoritatively_identical', 'particle_demo::showcase_tests::unchanged_presets_spread_fade_and_expire_after_real_pickup', 'particle_demo::tests::actual_sample_pixels_preserve_frame_and_optionally_capture_png', 'particle_demo::tests::checked_pickup_origins_and_fresh_session_restart_discard_effects', 'particle_demo::tests::composed_geometry_settings_request_one_sample', 'particle_demo::tests::options_require_explicit_mode_and_reject_duplicate_or_unknown_flags', 'particle_demo::tests::real_pickups_drive_presets_without_changing_authoritative_checksums', 'particle_demo::tests::repeated_zero_delta_poll_does_not_replay_bursts', 'yard3d_stream::tests::authoritative_frame_preserves_entities_floor_and_poses', 'yard3d_stream::tests::discontinuity_collapses_previous_pose_and_keeps_wire_flags', 'yard3d_stream::tests::yard_schema_has_real_input_offsets_and_four_byte_command', 'yard3d_view::tests::bake_classification_uses_body_kind_and_exact_same_frame_mesh', 'yard3d_view::tests::input_ray_round_trips_through_the_integer_encoding']
GPU = [{'name': 'grounding', 'filter': 'particle_demo::material_scene::tests::actual_grounded_scene_tracks_player_and_preserves_shadow_controls', 'adapter_scope': 'full', 'audit': 'grounding'}, {'name': 'material', 'filter': 'particle_demo::material_scene::tests::actual_material_scene_preserves_pickup_and_expired_particle_baseline', 'adapter_scope': 'full', 'audit': 'material'}, {'name': 'sensitivity', 'filter': 'particle_demo::material_scene::tests::actual_metallic_and_roughness_sensitivity_changes_only_one_parameter', 'adapter_scope': 'full', 'audit': None}, {'name': 'silhouettes', 'filter': 'particle_demo::material_scene::tests::actual_isolated_material_silhouettes_match_independent_projection', 'adapter_scope': 'full', 'audit': None}, {'name': 'occlusion', 'filter': 'particle_demo::material_scene::tests::actual_floor_occludes_behind_particle_but_keeps_above_control', 'adapter_scope': 'full', 'audit': None}, {'name': 'static', 'filter': 'particle_demo::tests::actual_sample_pixels_preserve_frame_and_optionally_capture_png', 'adapter_scope': 'sample', 'audit': 'static'}, {'name': 'temporal', 'filter': 'particle_demo::showcase_tests::actual_temporal_dust_and_sparks_spread_fade_and_expire', 'adapter_scope': 'temporal', 'audit': 'temporal'}]
SHAPES = {'static/particles-off.png': [320, 240], 'static/pickup-sparks.png': [320, 240], 'static/capture.txt': None, 'temporal/dust-000ms.png': [640, 480], 'temporal/dust-100ms.png': [640, 480], 'temporal/dust-250ms.png': [640, 480], 'temporal/dust-450ms.png': [640, 480], 'temporal/dust-700ms.png': [640, 480], 'temporal/sparks-000ms.png': [640, 480], 'temporal/sparks-100ms.png': [640, 480], 'temporal/sparks-250ms.png': [640, 480], 'temporal/sparks-450ms.png': [640, 480], 'temporal/sparks-700ms.png': [640, 480], 'temporal/sequence.txt': None, 'material-scene/legacy-initial.png': [640, 480], 'material-scene/showcase-initial.png': [640, 480], 'material-scene/legacy-pickup.png': [640, 480], 'material-scene/showcase-pickup.png': [640, 480], 'material-scene/legacy-spread.png': [640, 480], 'material-scene/showcase-spread.png': [640, 480], 'material-scene/legacy-expired.png': [640, 480], 'material-scene/showcase-expired.png': [640, 480], 'material-scene/legacy-restart.png': [640, 480], 'material-scene/showcase-restart.png': [640, 480], 'material-scene/manifest.txt': None, 'grounding/initial-off.png': [640, 480], 'grounding/initial-off.rgba': 'raw', 'grounding/initial-on.png': [640, 480], 'grounding/initial-on.rgba': 'raw', 'grounding/corner-nw-off.png': [640, 480], 'grounding/corner-nw-off.rgba': 'raw', 'grounding/corner-nw-on.png': [640, 480], 'grounding/corner-nw-on.rgba': 'raw', 'grounding/corner-ne-off.png': [640, 480], 'grounding/corner-ne-off.rgba': 'raw', 'grounding/corner-ne-on.png': [640, 480], 'grounding/corner-ne-on.rgba': 'raw', 'grounding/corner-sw-off.png': [640, 480], 'grounding/corner-sw-off.rgba': 'raw', 'grounding/corner-sw-on.png': [640, 480], 'grounding/corner-sw-on.rgba': 'raw', 'grounding/corner-se-off.png': [640, 480], 'grounding/corner-se-off.rgba': 'raw', 'grounding/corner-se-on.png': [640, 480], 'grounding/corner-se-on.rgba': 'raw', 'grounding/initial-repeat-on.png': [640, 480], 'grounding/initial-repeat-on.rgba': 'raw', 'grounding/initial-restart-on.png': [640, 480], 'grounding/initial-restart-on.rgba': 'raw', 'grounding/pickup-100ms-particles-off.png': [640, 480], 'grounding/pickup-100ms-particles-off.rgba': 'raw', 'grounding/pickup-100ms-particles-on.png': [640, 480], 'grounding/pickup-100ms-particles-on.rgba': 'raw', 'grounding/pickup-250ms-particles-off.png': [640, 480], 'grounding/pickup-250ms-particles-off.rgba': 'raw', 'grounding/pickup-250ms-particles-on.png': [640, 480], 'grounding/pickup-250ms-particles-on.rgba': 'raw', 'grounding/pickup-700ms-particles-off.png': [640, 480], 'grounding/pickup-700ms-particles-off.rgba': 'raw', 'grounding/pickup-700ms-particles-on.png': [640, 480], 'grounding/pickup-700ms-particles-on.rgba': 'raw', 'grounding/manifest.txt': None}
GROUNDING_NAMES = ['initial-off', 'initial-on', 'corner-nw-off', 'corner-nw-on', 'corner-ne-off', 'corner-ne-on', 'corner-sw-off', 'corner-sw-on', 'corner-se-off', 'corner-se-on', 'initial-repeat-on', 'initial-restart-on', 'pickup-100ms-particles-off', 'pickup-100ms-particles-on', 'pickup-250ms-particles-off', 'pickup-250ms-particles-on', 'pickup-700ms-particles-off', 'pickup-700ms-particles-on']

IGNORED = sorted(c["filter"] for c in GPU)
CPU = sorted(set(NAMES) - set(IGNORED))

def require(condition, message="contract failed"):
    if not condition:
        raise AssertionError(message)

def digest(data):
    return hashlib.sha256(data).hexdigest()

def save(path, value):
    with path.open("x", encoding="utf-8") as file:
        json.dump(value, file, indent=2)
        file.write("\n")

def inventory(text, expected):
    names = re.findall(r"^([A-Za-z0-9_:]+): test$", text, re.M)
    require(sorted(names) == sorted(expected) and len(set(names)) == len(names), "test inventory mismatch")
    require(re.findall(r"^\s*(\d+) tests?, 0 benchmarks\s*$", text, re.M) == [str(len(expected))], "inventory summary")

def result(text, passed, ignored, filtered):
    require("SKIP:" not in text and "adapter unavailable" not in text.lower(), "device skip")
    summaries = re.findall(r"^test result: (.+)$", text, re.M)
    prefix = f"ok. {len(passed)} passed; 0 failed; {len(ignored)} ignored; 0 measured; {filtered} filtered out; finished in "
    require(len(summaries) == 1 and summaries[0].startswith(prefix), "result summary")
    # Device stdout can be interleaved with the harness line under --nocapture;
    # --show-output keeps exact named status lines and retains failure output.
    require(sorted(re.findall(r"^test (.+) \.\.\. ok$", text, re.M)) == sorted(passed), "passed names")
    require(sorted(re.findall(r"^test (.+) \.\.\. ignored(?:,.*)?$", text, re.M)) == sorted(ignored), "ignored names")


def png_rgba(data, shape):
    """Decode only bounded, noninterlaced RGBA8 PNG; reject malformed chunks."""
    require(len(data) <= 2 * 1024**2 and data[:8] == b"\x89PNG\r\n\x1a\n", "PNG signature/size")
    width, height = shape
    require(0 < width <= 640 and 0 < height <= 480, "PNG dimensions")
    pos, compressed, state, seen_header = 8, bytearray(), 0, False
    while pos < len(data):
        require(pos + 12 <= len(data), "truncated chunk")
        length = struct.unpack_from(">I", data, pos)[0]
        require(length <= 2 * 1024**2 and pos + 12 + length <= len(data), "chunk length")
        kind = data[pos+4:pos+8]; payload = data[pos+8:pos+8+length]
        require(all(65 <= v <= 90 or 97 <= v <= 122 for v in kind) and 65 <= kind[2] <= 90, "chunk type/reserved bit")
        crc = struct.unpack_from(">I", data, pos+8+length)[0]
        require(zlib.crc32(kind+payload) & 0xffffffff == crc, "chunk CRC")
        pos += length + 12
        if kind == b"IHDR":
            require(not seen_header and pos == 33 and length == 13, "IHDR order")
            require(struct.unpack(">IIBBBBB", payload) == (width, height, 8, 6, 0, 0, 0), "RGBA8 format")
            seen_header = True
        elif kind == b"IDAT":
            require(seen_header and state in (0, 1), "IDAT order")
            state = 1; compressed.extend(payload)
        elif kind == b"IEND":
            require(state in (1, 2) and length == 0 and pos == len(data), "IEND/order/trailing bytes")
            state = 3
        else:
            require(seen_header and kind[0] & 32, "unknown critical chunk")
            if state == 1:
                state = 2
    require(state == 3, "missing IEND")
    count = height * (1 + width * 4)
    decoder = zlib.decompressobj()
    raw = decoder.decompress(compressed, count + 1)
    require(len(raw) == count and decoder.eof and not decoder.unused_data and not decoder.unconsumed_tail, "bounded zlib stream")
    stride = width * 4; pixels = bytearray(); previous = bytearray(stride)
    for y in range(height):
        start = y * (stride + 1); filt = raw[start]
        require(filt <= 4, "PNG filter")
        row = bytearray(raw[start+1:start+1+stride])
        for x in range(stride):
            a = row[x-4] if x >= 4 else 0; b = previous[x]; c = previous[x-4] if x >= 4 else 0
            if filt == 1: prediction = a
            elif filt == 2: prediction = b
            elif filt == 3: prediction = (a+b)//2
            elif filt == 4:
                q = a+b-c; pa,pb,pc = abs(q-a),abs(q-b),abs(q-c)
                prediction = a if pa <= pb and pa <= pc else b if pb <= pc else c
            else: prediction = 0
            row[x] = (row[x]+prediction) & 255
        pixels.extend(row); previous = row
    return bytes(pixels)


class WindowsJob:
    """Place a gated launcher in a kill-on-close job before it starts cargo/tests."""
    def __init__(self):
        from ctypes import wintypes as w
        class Basic(ctypes.Structure):
            _fields_ = [("process_time",ctypes.c_int64),("job_time",ctypes.c_int64),("flags",w.DWORD),("min_ws",ctypes.c_size_t),("max_ws",ctypes.c_size_t),("active",w.DWORD),("affinity",ctypes.c_size_t),("priority",w.DWORD),("scheduling",w.DWORD)]
        class Io(ctypes.Structure):
            _fields_ = [(name,ctypes.c_uint64) for name in ("ro","wo","oo","rt","wt","ot")]
        class Extended(ctypes.Structure):
            _fields_ = [("basic",Basic),("io",Io),("process_mem",ctypes.c_size_t),("job_mem",ctypes.c_size_t),("peak_process",ctypes.c_size_t),("peak_job",ctypes.c_size_t)]
        self.api = ctypes.WinDLL("kernel32", use_last_error=True)
        self.api.CreateJobObjectW.argtypes = [ctypes.c_void_p,w.LPCWSTR]; self.api.CreateJobObjectW.restype = w.HANDLE
        self.api.SetInformationJobObject.argtypes = [w.HANDLE,ctypes.c_int,ctypes.c_void_p,w.DWORD]
        self.api.AssignProcessToJobObject.argtypes = [w.HANDLE,w.HANDLE]
        self.api.TerminateJobObject.argtypes = [w.HANDLE,w.UINT]
        self.api.CloseHandle.argtypes = [w.HANDLE]
        self.api.QueryInformationJobObject.argtypes = [w.HANDLE,ctypes.c_int,ctypes.c_void_p,w.DWORD,ctypes.c_void_p]
        self.handle = self.api.CreateJobObjectW(None,None)
        require(bool(self.handle), "CreateJobObjectW")
        info = Extended(); info.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        if not self.api.SetInformationJobObject(self.handle,9,ctypes.byref(info),ctypes.sizeof(info)):
            self.api.CloseHandle(self.handle); raise OSError(ctypes.get_last_error(), "SetInformationJobObject")
    def assign(self, process):
        require(bool(self.api.AssignProcessToJobObject(self.handle,int(process._handle))), "AssignProcessToJobObject")
    def close(self):
        if self.handle:
            require(bool(self.api.TerminateJobObject(self.handle,1)), "TerminateJobObject")
            accounting = ctypes.create_string_buffer(48)
            deadline = time.monotonic()+10
            while True:
                require(bool(self.api.QueryInformationJobObject(self.handle,1,accounting,48,None)), "QueryInformationJobObject")
                active = struct.unpack_from("<I",accounting.raw,40)[0]
                if active == 0: break
                require(time.monotonic() < deadline, "Windows job descendants not reaped")
                time.sleep(.01)
            require(bool(self.api.CloseHandle(self.handle)), "CloseHandle job"); self.handle = None


def bounded_run(argv, cwd, env, log_path, timeout, expected_exit=0, disk_floor=DISK_FLOOR):
    """Bound output with a reader queue, reap descendants on success and failure."""
    require(not log_path.exists(), "fresh command log")
    start = time.monotonic(); reason = None; count = 0; job = None; process = None; reaped = False; assigned = False
    events = queue.Queue(maxsize=16); stopped = threading.Event(); reader = None
    def pump(pipe):
        try:
            while True:
                chunk = pipe.read1(65536)
                while not stopped.is_set():
                    try: events.put(chunk, timeout=.05); break
                    except queue.Full: pass
                if not chunk or stopped.is_set(): break
        finally:
            pipe.close()
    try:
        if os.name == "nt":
            job = WindowsJob()
            # Waiting for GO prevents the target or any descendants escaping
            # before assignment to the job; argv is JSON, never a shell string.
            launcher = "import sys,json,subprocess; a=json.loads(sys.argv[1]); assert sys.stdin.readline()=='GO\\n'; sys.exit(subprocess.call(a))"
            process = subprocess.Popen([sys.executable,"-c",launcher,json.dumps(argv)],cwd=cwd,env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
            job.assign(process); assigned = True; process.stdin.write(b"GO\n"); process.stdin.flush(); process.stdin.close()
        else:
            process = subprocess.Popen(argv,cwd=cwd,env=env,stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,start_new_session=True)
        reader = threading.Thread(target=pump,args=(process.stdout,),daemon=True); reader.start()
        with log_path.open("xb") as log:
            while True:
                if time.monotonic()-start > timeout: reason = "timeout"; break
                if shutil.disk_usage(log_path.parent).free < disk_floor: reason = "disk floor"; break
                try: chunk = events.get(timeout=.05)
                except queue.Empty: continue
                if not chunk: break
                if count+len(chunk) > LOG_CAP:
                    retained = chunk[:LOG_CAP-count]; log.write(retained); log.flush(); count += len(retained)
                    reason = "log cap"; break
                log.write(chunk); log.flush(); count += len(chunk)
        if reason is None:
            process.wait(timeout=max(.01,timeout-(time.monotonic()-start)))
    except BaseException as error:
        reason = reason or repr(error)
    finally:
        try:
            if job is not None:
                if process is not None and not assigned:
                    process.kill(); process.wait(timeout=10)
                job.close()
            elif process is not None:
                try: os.killpg(process.pid,signal.SIGKILL)
                except ProcessLookupError: pass
            if process is not None:
                process.wait(timeout=10)
                if process.stdin is not None and not process.stdin.closed: process.stdin.close()
                if reader is None and process.stdout is not None: process.stdout.close()
            stopped.set()
            if reader is not None:
                reader.join(timeout=10)
                require(not reader.is_alive(), "output reader not reaped")
            reaped = True
        except BaseException as error:
            reason = (reason or "") + " cleanup: " + repr(error)
        receipt = {"argv":argv,"exit":process.returncode if process else None,"stop_reason":reason,"seconds":time.monotonic()-start,"log_bytes":count,"log_sha256":digest(log_path.read_bytes()) if log_path.exists() else None,"owned_children_reaped":reaped}
        save(log_path.with_suffix(".json"),receipt)
    require(reason is None and process.returncode == expected_exit, f"command failed: {receipt}")
    return log_path.read_text(encoding="utf-8",errors="replace")


def relative_capture_key(path, root):
    return path.relative_to(root).as_posix()


def inspect(path, shape):
    require(path.is_file() and not path.is_symlink(), "capture file")
    require(path.stat().st_size <= (640*480*4 if shape == "raw" else 2*1024**2 if shape else 64*1024), "capture size")
    data = path.read_bytes(); record = {"bytes":len(data),"sha256":digest(data)}
    if shape == "raw": require(len(data) == 640*480*4, "raw size")
    elif shape:
        raw = png_rgba(data,shape)
        record.update(width=shape[0],height=shape[1],raw_rgba_sha256=digest(raw))
    return record

# audit context is set once per serial producer; no concurrent use.
capture_root = None
baseline = {}
adapter = None
audits = []
def audit(kind):
 directory={'material':'material-scene'}.get(kind,kind)
 keys=[k for k in SHAPES if k.startswith(directory+'/')]
 actual={relative_capture_key(p,capture_root)for p in (capture_root/directory).rglob('*')if p.is_file()}
 assert actual==set(keys),(kind,actual,set(keys))
 files={k:inspect(capture_root/k,SHAPES[k])for k in keys}
 if kind in ['static','temporal']:
  for k in keys:
   prior = baseline[k]
   assert (capture_root/k).read_bytes() == prior.read_bytes(), ('procedural regression',k)
 elif kind=='material':
  text=(capture_root/directory/'manifest.txt').read_text();assert text.startswith('STATE=RUNNING;')and text.endswith('STATE=PASS frames=10\n')
  assert 'adapter '+adapter in text
  assert 'eye_offset=2.4,3.6,-1.8' in text and 'shadows=true radius=6.5' in text
  lines=[x for x in text.splitlines()if x.startswith('frame=')];assert len(lines)==10
  records={}
  expected=[f'{mode}-{stage}.png'for stage in ['initial','pickup','spread','expired','restart']for mode in ['legacy','showcase']]
  for line,name in zip(lines,expected):
   m=re.fullmatch(r'frame=(\S+) raw_rgba_sha256=([0-9a-f]{64}) png_sha256=([0-9a-f]{64}) tick=(\d+) checksum=([0-9a-f]{16}) elapsed_us=(\d+) pickup_us=(None|Some\(\d+\)) score=(\d+) particles=(\d+) actors=(\d+) lighting=(.+)',line)
   assert m and m[1]==name
   rec=files[directory+'/'+name];assert rec['raw_rgba_sha256']==m[2]and rec['sha256']==m[3]
   stage=name.split('-',1)[1][:-4];score=0 if stage in ['initial','restart']else 1;particles=16 if stage in ['pickup','spread']else 0
   assert int(m[8])==score and int(m[9])==particles and int(m[10])==3-score
   # Recorded Demo lighting stays off; actual material route clones/enables shadows, documented above.
   assert 'shadows: false' in m[11];records[name]=m.groups()
  for stage in ['initial','pickup','spread','expired','restart']:assert records['legacy-'+stage+'.png'][3:]==records['showcase-'+stage+'.png'][3:]
  for mode in ['legacy','showcase']:
   assert files[f'{directory}/{mode}-initial.png']==files[f'{directory}/{mode}-restart.png']
   anchor=int(records[f'{mode}-pickup.png'][5]);assert anchor>0
   assert int(records[f'{mode}-spread.png'][5])-anchor==100000 and int(records[f'{mode}-expired.png'][5])-anchor==700000
  assert files[directory+'/legacy-initial.png']['raw_rgba_sha256']!=files[directory+'/showcase-initial.png']['raw_rgba_sha256']
 else:
  text=(capture_root/directory/'manifest.txt').read_text();assert text.startswith('STATE=RUNNING partial')and text.endswith('STATE=PASS frames=18 visual_review_pending\n')
  assert 'adapter '+adapter in text
  lines=[x for x in text.splitlines()if x.startswith('frame=')];assert len(lines)==18
  records={}
  for name,line in zip(GROUNDING_NAMES,lines):
   m=re.fullmatch(r'frame=(\S+) raw=([0-9a-f]{64}) png=([0-9a-f]{64}) player=(\[[^\n]+?\]) checksum=([0-9a-f]{16}) score=(\d+) particles=(\d+)',line)
   assert m and m[1]==name
   raw=files[directory+'/'+name+'.rgba'];png=files[directory+'/'+name+'.png']
   assert raw['sha256']==m[2]==png['raw_rgba_sha256']and png['sha256']==m[3];records[name]=m.groups()
  for pose in ['initial','corner-nw','corner-ne','corner-sw','corner-se']:
   assert records[pose+'-off'][3:]==records[pose+'-on'][3:]
   assert files[directory+'/'+pose+'-off.rgba']['sha256']!=files[directory+'/'+pose+'-on.rgba']['sha256']
  for name in ['initial-repeat-on','initial-restart-on']:assert files[directory+'/'+name+'.rgba']['sha256']==files[directory+'/initial-on.rgba']['sha256']
  for ms in [100,250,700]:
   off=f'pickup-{ms}ms-particles-off';on=f'pickup-{ms}ms-particles-on';assert records[off][3:]==records[on][3:]
   assert int(records[on][5])==1 and int(records[on][6])==(0 if ms==700 else 16)
   equal=files[directory+'/'+off+'.rgba']['sha256']==files[directory+'/'+on+'.rgba']['sha256'];assert equal==(ms==700)
 receipt={'state':'CAPTURE_AUDIT_PASSED_VISUAL_REVIEW_PENDING','kind':kind,'files':files,'adapter_scope':'name/software plus pinned driver'if kind in ['static','temporal']else'full tuple'}
 out=capture_root.parent/('capture-audit-'+kind+'.json')
 with out.open('x')as f:json.dump(receipt,f,indent=2);f.write('\n')
 audits.append({'path':str(out),'sha256':digest(out.read_bytes())})


def observed_adapter(text, scope):
    prefix = {"full":"material sample adapter: ","sample":"sample particle adapter=","temporal":"temporal particle adapter="}[scope]
    matches = re.findall(re.escape(prefix)+r"([^\r\n]+)",text)
    require(len(matches) == 1, "one observed adapter")
    if scope == "full":
        require(re.fullmatch(r'name=".+" backend=Vulkan device_type=Cpu vendor=\d+ device=\d+ driver=".*" driver_info=".*" software=true',matches[0]), "software Vulkan tuple")
    else: require(matches[0].endswith(" software=true"), "software adapter")
    return matches[0]


def fixture_controls(command, text):
    if command["name"] == "grounding":
        rows = re.findall(r'grounding eligible=(\d+) dark=(\d+) lit=(\d+) shadow_inset_world=0.06 lit_outset_world=0.12 object_band_px=2',text)
        require(len(rows) == 5 and all(int(a)>=8 and int(b)>=8 and int(c)>=500 for a,b,c in rows), "shadow/lit controls")
    if command["name"] == "silhouettes":
        rows = re.findall(r'material silhouette index=(\d+) independent_interior=(\d+) independent_background=(\d+) edge_margin_px=2',text)
        require([v[0] for v in rows] == ["0","1","2"] and all(int(v[1])>16 and int(v[2])>100 for v in rows), "silhouette controls")
    if command["name"] == "material":
        require(len(re.findall(r'^material capture frame=',text,re.M)) == 10 and 'material capture STATE=PASS frames=10' in text, "material capture result")


class Gate:
    def __init__(self, root, evidence):
        self.root = root.resolve(); self.evidence = evidence.resolve()
        self.evidence.mkdir(parents=True,exist_ok=False)
        self.env = os.environ.copy(); self.env["CARGO_TERM_COLOR"] = "never"
        for key in list(self.env):
            if key.startswith("ORR_") and ("CAPTURE" in key or "SCREENSHOT" in key): del self.env[key]
        require(not any(self.env.get(k) for k in ("RUSTC_WRAPPER","RUSTC_WORKSPACE_WRAPPER","RUSTFLAGS","CARGO_ENCODED_RUSTFLAGS")), "unexpected compiler override")
        self.source = self.source_hashes()
        save(self.evidence/"source.json",self.source)
        self.producers = {}; self.commands = []; self.number = 0
    def source_hashes(self):
        names = subprocess.check_output(["git","ls-files","-z"],cwd=self.root).decode().split("\0")
        return {name:digest((self.root/name).read_bytes()) for name in names if name}
    def run(self,label,argv,expected_exit=0,env=None,timeout=1200):
        require(self.source_hashes() == self.source, "source changed")
        for record in self.producers.values():
            require(digest(Path(record["path"]).read_bytes()) == record["sha256"], "producer changed")
        self.number += 1
        path = self.evidence/f"{self.number:03d}-{label}.log"
        self.commands.append({"label":label,"argv":argv,"log":path.name})
        text = bounded_run(argv,self.root,env or self.env,path,timeout,expected_exit)
        require(self.source_hashes() == self.source, "post-command source changed")
        for record in self.producers.values():
            require(digest(Path(record["path"]).read_bytes()) == record["sha256"], "post-command producer changed")
        return text
    def compile(self,label,features,binary=False):
        args = ["cargo","build" if binary else "test","--locked","--release","-p","orr_sample","--no-default-features","--features",features]
        args += ["--bin","material_demo"] if binary else ["--lib","--no-run"]
        args += ["--message-format=json"]
        text = self.run(label+"-compile",args)
        artifacts = []
        for line in text.splitlines():
            try: item=json.loads(line)
            except json.JSONDecodeError: continue
            if item.get("reason") == "compiler-artifact" and item.get("executable") and item.get("target",{}).get("name") == ("material_demo" if binary else "orr_sample") and bool(item.get("profile",{}).get("test")) != binary:
                artifacts.append(item)
        require(len(artifacts) == 1, "unique producer")
        original = Path(artifacts[0]["executable"])
        require(original.is_file(), "producer missing")
        saved = self.evidence/(label+(".exe" if os.name == "nt" else ""))
        shutil.copy2(original,saved)
        self.producers[label] = {"path":str(saved),"sha256":digest(saved.read_bytes()),"features":features,"compile_log_sha256":digest((self.evidence/self.commands[-1]["log"]).read_bytes())}
        save(self.evidence/(label+"-producer.json"),self.producers[label])
        return str(saved)
    def cpu(self):
        helper = self.run("helper-contracts",[sys.executable,"-m","unittest","discover","-s","tools","-p","test_material_sample_gate.py","-v"],timeout=180)
        require(re.search(r"Ran 20 tests in [0-9.]+s",helper) and helper.rstrip().endswith("OK"), "helper test census")
        for label,features in FEATURES.items():
            exe = self.compile(label,features)
            inventory(self.run(label+"-list",[exe,"--list"]),NAMES)
            inventory(self.run(label+"-ignored-list",[exe,"--ignored","--list"]),IGNORED)
            result(self.run(label+"-cpu",[exe,"--test-threads=1","--show-output"]),CPU,IGNORED,0)
            self.run(label+"-strict",["cargo","clippy","--locked","-p","orr_sample","--no-default-features","--features",features,"--all-targets","--","-D","warnings"])
        exe = self.compile("material-cli",FEATURES["material"],binary=True)
        help_text = self.run("cli-help",[exe,"--help"])
        require("--materials" in help_text and "--particles" in help_text, "CLI help")
        invalid = [[],["--materials","bad","--particles","off"],["--materials","legacy","--materials","showcase","--particles","off"],["--materials","legacy"],["--materials","showcase","--particles","off","--unknown"]]
        for index,args in enumerate(invalid):
            require("material_demo:" in self.run(f"cli-invalid-{index}",[exe,*args],1), "CLI invalid diagnostic")
        outputs = {}
        for mode in ("legacy","showcase"):
            for particles in ("off","dust","sparks"):
                text = self.run(f"cli-{mode}-{particles}",[exe,"--materials",mode,"--particles",particles,"--headless"])
                outputs[mode,particles] = text
            outputs[mode,"cycling"] = self.run(f"cli-{mode}-cycling",[exe,"--materials",mode,"--particles","sparks","--headless","--showcase"])
        for particles in ("off","dust","sparks","cycling"):
            # The validated source emits identical CPU-loop lines for both materials.
            require(outputs["legacy",particles] == outputs["showcase",particles], "headless mode parity")
            matches = re.findall(r"score=(\d+).*particles=(\d+)",outputs["legacy",particles])
            require(matches and matches[-1] == ("1",{"off":"0","dust":"12","sparks":"16","cycling":"16"}[particles]), "headless score/particles")
    def gpu(self, baseline_root):
        global capture_root, baseline, adapter, audits
        require(os.name != "nt" and self.env.get("ORR_REQUIRE_GPU") == "1", "Linux mandatory device phase")
        require(self.env.get("VK_DRIVER_FILES") == self.env.get("VK_ICD_FILENAMES") and Path(self.env["VK_DRIVER_FILES"]).is_file(), "one explicit Vulkan ICD")
        save(self.evidence/"driver.json",{"icd":self.env["VK_DRIVER_FILES"],"sha256":digest(Path(self.env["VK_DRIVER_FILES"]).read_bytes())})
        baseline = {key:baseline_root/("particle-sample-captures" if key.startswith("static/") else "particle-temporal-captures")/key.split("/",1)[1] for key in SHAPES if key.startswith(("static/","temporal/"))}
        baseline_hash = {k:digest(p.read_bytes()) for k,p in baseline.items()}
        save(self.evidence/"baseline.json",baseline_hash)
        full_adapter = None
        for label,features in FEATURES.items():
            exe = self.compile(label,features)
            inventory(self.run(label+"-list",[exe,"--list"]),NAMES)
            inventory(self.run(label+"-ignored-list",[exe,"--ignored","--list"]),IGNORED)
            lane = self.evidence/(label+"-captures"); lane.mkdir()
            capture_root = lane/"frames"; capture_root.mkdir(); audits=[]
            for directory in ("grounding","material-scene","static","temporal"): (capture_root/directory).mkdir()
            for command in GPU:
                env = self.env.copy()
                capture = {"grounding":("ORR_GROUNDING_CAPTURE_DIR","grounding"),"material":("ORR_MATERIAL_CAPTURE_DIR","material-scene"),"static":("ORR_PARTICLE_CAPTURE_DIR","static"),"temporal":("ORR_PARTICLE_SHOWCASE_CAPTURE_DIR","temporal")}.get(command["name"])
                if capture: env[capture[0]] = str(capture_root/capture[1])
                text = self.run(label+"-"+command["name"],[exe,command["filter"],"--exact","--ignored","--test-threads=1","--show-output"],env=env,timeout=180)
                result(text,[command["filter"]],[],32)
                observed = observed_adapter(text,command["adapter_scope"])
                if command["adapter_scope"] == "full":
                    if full_adapter is None: full_adapter = observed
                    require(observed == full_adapter,"adapter tuple changed")
                    adapter = observed
                else:
                    name = re.fullmatch(r'name="(.+)" backend=.+',full_adapter)[1]
                    require(observed == name+" software=true", "inherited adapter mismatch")
                fixture_controls(command,text)
                if command["audit"]: audit(command["audit"])
                require(sum(p.stat().st_size for p in capture_root.rglob("*") if p.is_file()) <= CAPTURE_CAP,"capture budget")
                require({k:digest(p.read_bytes()) for k,p in baseline.items()} == baseline_hash,"baseline changed")
            require({relative_capture_key(p,capture_root) for p in capture_root.rglob("*") if p.is_file()} == set(SHAPES),"62 capture artifacts")
            save(lane/"summary.json",{"adapter":full_adapter,"audits":audits,"files":{k:digest((capture_root/k).read_bytes()) for k in SHAPES}})
        save(self.evidence/"device-summary.json",{"adapter":full_adapter,"invocations":14,"capture_artifacts":124,"pngs":80,"draw_readback_budget":128,"visual_review_required":True})


def main():
    require(sys.flags.optimize == 0, "Python optimization would disable capture assertions")
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase",choices=("cpu","gpu")); parser.add_argument("--evidence",type=Path,required=True); parser.add_argument("--baseline-root",type=Path)
    args=parser.parse_args(); gate=Gate(Path(__file__).resolve().parents[1],args.evidence)
    state="STOPPED"
    try:
        if args.phase == "cpu": gate.cpu()
        else:
            require(args.baseline_root is not None,"baseline root required");gate.gpu(args.baseline_root)
        require(gate.source_hashes() == gate.source,"final source fence")
        state="PASSED"
    finally:
        save(gate.evidence/"terminal.json",{"state":state,"phase":args.phase,"commands":gate.commands,"producers":gate.producers,"source_sha256":digest((gate.evidence/"source.json").read_bytes()),"owned_children_reaped":bool(gate.commands) and len(list(gate.evidence.glob("[0-9][0-9][0-9]-*.json"))) == len(gate.commands) and all(json.loads(p.read_text())["owned_children_reaped"] for p in gate.evidence.glob("[0-9][0-9][0-9]-*.json"))})

if __name__ == "__main__":
    main()
