#!/usr/bin/env python3
"""Create a new optional Stormglass Scene through trusted engine admission."""
import argparse
import json
from pathlib import Path
import subprocess
import sys

from pack_coop_defense_offline import (
    MAX_ASSET, publish_new, snapshot_file, validate_scene,
)


def scene_text(starting_gold=150, reactor_hp=30, last_wave=10, spawn_interval_ticks=None):
    """Emit existing scene singletons; omitted pacing keeps legacy bytes."""
    for name, value, low, high in (
        ("starting_gold", starting_gold, 0, 10000),
        ("reactor_hp", reactor_hp, 1, 1000),
        ("last_wave", last_wave, 1, 10),
    ):
        if type(value) is not int or not low <= value <= high:
            raise ValueError(f"{name} must be an integer in {low}..{high}")
    if spawn_interval_ticks is not None and (
        type(spawn_interval_ticks) is not int or not 35 <= spawn_interval_ticks <= 350
    ):
        raise ValueError("spawn_interval_ticks must be an integer in 35..350")
    text = (
        "schema: orr.scene/1\nentities: {}\nsingletons:\n"
        "  CoopDefense::Scenario:\n"
        f"    starting_gold: {starting_gold}\n"
        f"    reactor_hp: {reactor_hp}\n"
        "  CoopDefense::Campaign:\n"
        f"    last_wave: {last_wave}\n"
    )
    if spawn_interval_ticks is not None:
        text += ("  CoopDefense::WavePacing:\n"
                 f"    spawn_interval_ticks: {spawn_interval_ticks}\n")
    return text


def create_scene(wasm_path, output, starting_gold=150, reactor_hp=30, last_wave=10,
                 spawn_interval_ticks=None):
    text = scene_text(starting_gold, reactor_hp, last_wave, spawn_interval_ticks)
    # This executes trusted local code, not arbitrary authored WASM. Snapshot the
    # same bounded bytes used by the existing offline packer; never rebuild/fetch.
    wasm = snapshot_file(Path(wasm_path), MAX_ASSET)
    receipt = validate_scene(text.encode("utf-8"), wasm)
    publish_new(Path(output), text)
    return receipt


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wasm", required=True, type=Path,
                        help="trusted locally built CoopDefense game.wasm (executes in Node)")
    parser.add_argument("--output", required=True, type=Path,
                        help="new Scene YAML file in an existing directory; never overwritten")
    parser.add_argument("--starting-gold", type=int, default=150)
    parser.add_argument("--reactor-hp", type=int, default=30)
    parser.add_argument("--last-wave", type=int, default=10)
    parser.add_argument("--spawn-interval-ticks", type=int, default=None,
                        help="optional enemy spawn interval (35..350 ticks); omitted keeps engine default")
    args = parser.parse_args(argv)
    try:
        receipt = create_scene(args.wasm, args.output, args.starting_gold,
                               args.reactor_hp, args.last_wave, args.spawn_interval_ticks)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"Scene creation failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps(receipt, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
