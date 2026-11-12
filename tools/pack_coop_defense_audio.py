"""Bounded PCM admission for the optional CoopDefense authored soundbank."""
import base64
import hashlib
import json
import re
import struct
from pathlib import Path

CUES = ('attack-light', 'attack-heavy', 'attack-magic', 'impact-light',
        'impact-heavy', 'summon', 'merge', 'ui-confirm', 'ui-reject', 'death')


def snapshot_audio(manifest_path, snapshot_file, strict_json):
    """Return one escaped JSON script; reads use the packer's no-follow helper.

    Inputs must be in trusted/quiescent directories, like the parent packer.
    Snapshot all bytes before publication; no source paths survive in the HTML.
    """
    path = Path(manifest_path)
    manifest = strict_json(snapshot_file(path, 128 * 1024))
    if (not isinstance(manifest, dict) or type(manifest.get('version')) is not int
            or manifest['version'] != 1 or type(manifest.get('sampleRate')) is not int
            or manifest['sampleRate'] != 48000 or not isinstance(manifest.get('cues'), list)
            or len(manifest['cues']) != len(CUES)):
        raise ValueError('Unsupported audio bank')
    objects, seen, cooked, decoded = {}, set(), 0, 0
    for record in manifest['cues']:
        if not isinstance(record, dict):
            raise ValueError('Invalid audio record')
        cue, frames = record.get('cue'), record.get('frames')
        if (cue not in CUES or cue in seen or type(frames) is not int or not 1 <= frames <= 48000
                or record.get('assetId') != f'a_000000005444{CUES.index(cue) + 1:04x}'
                or record.get('file') != cue + '.pcm' or type(record.get('bytes')) is not int
                or record['bytes'] != 8 + frames * 2
                or not isinstance(record.get('sha256'), str)
                or re.fullmatch('[0-9a-f]{64}', record['sha256']) is None):
            raise ValueError('Invalid audio record')
        seen.add(cue)
        cooked += record['bytes']
        decoded += frames * 8
        if cooked > 2 * 1024 * 1024 or decoded > 4 * 1024 * 1024:
            raise ValueError('Audio bank exceeds budget')
        data = snapshot_file(path.parent / record['file'], record['bytes'])
        if (len(data) != record['bytes'] or hashlib.sha256(data).hexdigest() != record['sha256']
                or struct.unpack_from('<II', data) != (48000, frames)
                or any(abs(sample[0]) > 8192 for sample in struct.iter_unpack('<h', data[8:]))):
            raise ValueError('Invalid PCM object')
        objects[record['file']] = base64.b64encode(data).decode('ascii')
    encoded = json.dumps({'manifest': manifest, 'objects': objects}, separators=(',', ':')).replace('<', '\\u003c')
    return '<script type="application/json" id="authored-audio-bank">' + encoded + '</script>'
