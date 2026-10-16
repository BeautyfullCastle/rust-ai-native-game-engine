#!/usr/bin/env python3
"""Reproduce the OFL-1.1 Orrery Korean UI derivative; no network access needed."""
import argparse
import hashlib
import io
import json
from pathlib import Path

import fontTools
from fontTools import subset
from fontTools.ttLib import TTFont

SOURCE_SHA256 = "b76b0433203017ca80401b2ee0dd69350349871c4b19d504c34dbdd80541690a"
FONTTOOLS_VERSION = "4.61.1"
RANGES = [(0x20, 0x7E), (0x1100, 0x11FF), (0x3131, 0x318E), (0xAC00, 0xD7A3)]
EXTRA = [0xA0, 0xB7, 0x2013, 0x2014, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2026, 0x2190, 0x2191, 0x2192, 0x2193, 0x3000, 0x3001, 0x3002]
CODEPOINTS = set(EXTRA).union(*(range(a, b + 1) for a, b in RANGES))
FAMILY = "Orrery Korean UI"
PSNAME = "OrreryKoreanUI-Regular"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def generate(source):
    font = TTFont(io.BytesIO(source), fontNumber=1, recalcTimestamp=False)
    assert font['name'].getDebugName(1) == 'Noto Sans CJK KR'
    assert font['name'].getDebugName(5).startswith('Version 2.004;')
    assert not CODEPOINTS.difference(font.getBestCmap()), 'source coverage changed'
    options = subset.Options()
    options.name_IDs = ['*']  # Preserve upstream copyright, license and attribution.
    options.name_languages = ['*']
    options.name_legacy = True
    options.recalc_timestamp = False
    options.canonical_order = True
    worker = subset.Subsetter(options=options)
    worker.populate(unicodes=sorted(CODEPOINTS))
    worker.subset(font)
    replacements = {1: FAMILY, 2: 'Regular', 3: '1.000;ORRY;' + PSNAME,
                    4: FAMILY + ' Regular', 5: 'Version 1.000; derived from upstream 2.004',
                    6: PSNAME, 16: FAMILY, 17: 'Regular', 18: FAMILY + ' Regular',
                    21: FAMILY, 22: 'Regular', 25: 'OrreryKoreanUI'}
    for record in font['name'].names:
        if record.nameID in replacements:
            record.string = replacements[record.nameID].encode(record.getEncoding())
    cff = font['CFF '].cff
    cff.fontNames = [PSNAME]
    top = cff.topDictIndex[0]
    top.FamilyName = FAMILY
    top.FullName = FAMILY + ' Regular'
    for i, fd in enumerate(top.FDArray):
        fd.FontName = PSNAME + '-FD' + str(i)
    font['head'].created = font['head'].modified = 2082844800  # Unix epoch, OpenType epoch units.
    output = io.BytesIO()
    font.save(output, reorderTables=True)
    return output.getvalue()


def validate(data):
    assert 0 < len(data) <= 16 * 1024 * 1024, 'UI font exceeds 16 MiB limit'
    font = TTFont(io.BytesIO(data), lazy=False)
    font.ensureDecompiled()
    assert set(font.getBestCmap()) == CODEPOINTS, 'incorrect cmap coverage'
    assert font['name'].getDebugName(1) == FAMILY
    assert font['name'].getDebugName(6) == PSNAME
    assert font['CFF '].cff.fontNames == [PSNAME]
    for cp in CODEPOINTS:
        assert font.getBestCmap()[cp] != '.notdef'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, default=Path('/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc'))
    parser.add_argument('--output-dir', type=Path, default=Path(__file__).resolve().parents[1] / 'assets/game_ui_font')
    parser.add_argument('--check', action='store_true', help='regenerate twice and compare with checked-in font and metadata')
    args = parser.parse_args()
    assert fontTools.__version__ == FONTTOOLS_VERSION, 'install exact FontTools 4.61.1 for reproducibility'
    source = args.source.read_bytes()
    assert digest(source) == SOURCE_SHA256, 'unverified source font hash'
    first, second = generate(source), generate(source)
    assert first == second, 'non-deterministic font generation'
    validate(first)
    metadata = {
        'schema': 1, 'license': 'OFL-1.1', 'family': FAMILY,
        'source': {'file': 'NotoSansCJK-Regular.ttc', 'face_index': 1,
                   'family': 'Noto Sans CJK KR', 'version': '2.004', 'sha256': SOURCE_SHA256,
                   'upstream': 'https://github.com/notofonts/noto-cjk'},
        'recipe': {'tool': 'tools/subset_game_ui_font.py', 'fonttools': FONTTOOLS_VERSION,
                   'ranges': [f'U+{a:04X}-U+{b:04X}' for a, b in RANGES],
                   'additional_codepoints': [f'U+{cp:04X}' for cp in EXTRA],
                   'opentype_timestamp': 2082844800, 'retain_layout_features': True},
        'output': {'file': 'OrreryKoreanUI.otf', 'sha256': digest(first), 'bytes': len(first),
                   'cmap_codepoints': len(CODEPOINTS), 'modern_hangul_syllables': 11172},
    }
    encoded = (json.dumps(metadata, indent=2, ensure_ascii=False) + '\n').encode()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    for name, data in [('OrreryKoreanUI.otf', first), ('font-manifest.json', encoded)]:
        path = args.output_dir / name
        if args.check:
            assert path.read_bytes() == data, f'checked-in artifact mismatch: {path}'
        else:
            path.write_bytes(data)
    license_text = (args.output_dir / 'OFL.txt').read_text()
    assert 'SIL OPEN FONT LICENSE Version 1.1' in license_text
    assert 'PERMISSION & CONDITIONS' in license_text
    assert 'OTHER DEALINGS IN THE FONT SOFTWARE.' in license_text
    assert 'Files: debian' not in license_text
    corpus = (args.output_dir / 'corpus.txt').read_text()
    assert not {ord(c) for c in corpus if not c.isspace()}.difference(CODEPOINTS), 'corpus has unsupported glyphs'
    print(json.dumps(metadata['output'], sort_keys=True))
    print('PASS: verified source hash, repeat generation byte equality, complete cmap, full font parse, corpus')


if __name__ == '__main__':
    main()
