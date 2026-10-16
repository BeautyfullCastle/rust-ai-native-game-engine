#!/usr/bin/env python3
"""Reproduce the original Orrery lantern-keeper pixel atlas without dependencies."""
from pathlib import Path
import struct
import zlib

ROOT = Path(__file__).parent
W, H = 64, 16
pixels = bytearray(W * H * 4)

def rect(frame, x0, y0, x1, y1, color):
    for y in range(y0, y1):
        for x in range(x0, x1):
            offset = (y * W + frame * 16 + x) * 4
            pixels[offset:offset + 4] = bytes(color)

for frame in range(4):
    bob = 1 if frame == 1 else 0
    rect(frame, 5, 2 + bob, 11, 7 + bob, (246, 200, 140, 255))
    rect(frame, 4, 2 + bob, 11, 4 + bob, (51, 45, 74, 255))
    rect(frame, 9, 4 + bob, 10, 5 + bob, (26, 28, 41, 255))
    rect(frame, 4, 7 + bob, 11, 12, (54, 154, 165, 255))
    rect(frame, 5, 8 + bob, 9, 11, (104, 211, 183, 255))
    left = 3 if frame == 2 else 5
    right = 10 if frame == 2 else 8
    if frame == 3:
        left, right = 6, 9
    rect(frame, left, 12, left + 3, 15, (51, 45, 74, 255))
    rect(frame, right, 12, right + 3, 15, (51, 45, 74, 255))
    rect(frame, 11, 8 + bob, 13, 10 + bob, (246, 200, 140, 255))
    rect(frame, 12, 10 + bob, 15, 13 + bob, (255, 201, 76, 255))
    rect(frame, 13, 11 + bob, 14, 12 + bob, (255, 245, 181, 255))

def chunk(name, data):
    return struct.pack('!I', len(data)) + name + data + struct.pack('!I', zlib.crc32(name + data))

raw = b''.join(b'\0' + pixels[y * W * 4:(y + 1) * W * 4] for y in range(H))
png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('!IIBBBBB', W, H, 8, 6, 0, 0, 0))
png += chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b'')
(ROOT / 'lantern_keeper.png').write_bytes(png)
(ROOT / 'lantern_keeper.rgba').write_bytes(pixels)
