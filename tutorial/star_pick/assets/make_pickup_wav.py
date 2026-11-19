"""Writes pickup.wav: the "ding" of a picked star in step 2 of the tutorial.

Two sine tones one after the other (A5 880 Hz, then E6 1318.5 Hz), a short rise at the start of each
tone and a fade that ends at zero. 44100 Hz, 16-bit PCM, mono, 0.18 seconds.

The sound is made only from these numbers (no recording, no sample library), so it has no author
besides this repository and no license to keep: it is released under CC0 1.0 (public domain).

Run: python tutorial/star_pick/assets/make_pickup_wav.py
The output is byte for byte the same on every run.
"""
import math
import struct
import wave
from pathlib import Path

RATE = 44_100
LENGTH = 0.18
STEP_AT = 0.07
AMPLITUDE = 0.25


def sample(i: int) -> float:
    t = i / RATE
    hz, since = (880.0, 0.0) if t < STEP_AT else (1318.5, STEP_AT)
    rise = min((t - since) / 0.004, 1.0)
    fade = max(1.0 - t / LENGTH, 0.0)
    return AMPLITUDE * rise * fade * fade * math.sin((t - since) * math.tau * hz)


def main() -> None:
    count = int(RATE * LENGTH)
    frames = b"".join(struct.pack("<h", round(sample(i) * 32767)) for i in range(count))
    path = Path(__file__).with_name("pickup.wav")
    with wave.open(str(path), "wb") as out:
        out.setnchannels(1)
        out.setsampwidth(2)
        out.setframerate(RATE)
        out.writeframes(frames)
    print(f"{path}: {count} frames, {path.stat().st_size} bytes")


if __name__ == "__main__":
    main()
