#!/usr/bin/env python3
"""Reproducible original CC0 clips, using the triangle_decay_v1 integer contract.

This source tool is never executed by the package loader or by gameplay.
Run from any directory with Python 3. Only this package's cooked files change.
"""
import hashlib
import json
from pathlib import Path
import struct

ROOT = Path(__file__).resolve().parent


def trunc_div(numerator, denominator):
    """Rust's integer division truncates toward zero (Python // rounds down)."""
    return (1 if numerator >= 0 else -1) * (abs(numerator) // denominator)


def generate():
    manifest = bytearray(b"ORAM" + struct.pack("<III", 1, 2, 2))
    for asset_id, source in [(0x3001, "pickup.json"), (0x3002, "pickup_alt.json")]:
        spec = json.loads((ROOT / "source" / source).read_text())
        assert spec["generator"] == "triangle_decay_v1"
        rate, frames, period, peak = [spec[key] for key in
                                     ("sample_rate", "frames", "period_frames", "peak_pcm16")]
        assert rate == 48000 and 1 <= frames <= 48000
        assert 4 <= period <= 48000 and period % 4 == 0 and 0 <= peak <= 8192
        payload = bytearray(struct.pack("<II", rate, frames))
        for i in range(frames):
            phase = i % period
            triangle = 4 * phase - period if phase < period // 2 else 3 * period - 4 * phase
            payload += struct.pack("<h", trunc_div(peak * triangle * (frames - i), period * frames))
        digest = hashlib.sha256(payload).digest()
        (ROOT / "cooked" / "objects").mkdir(parents=True, exist_ok=True)
        (ROOT / "cooked" / "objects" / (digest.hex() + ".bin")).write_bytes(payload)
        manifest += struct.pack("<QIIQ", asset_id, 2, 1, len(payload)) + digest
    (ROOT / "cooked" / "view.manifest.bin").write_bytes(manifest)


if __name__ == "__main__":
    generate()
