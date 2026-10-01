#!/usr/bin/env python3
"""A view stream client in plain Python (standard library only).

Proves the view stream is language neutral: it connects to an Orrery host
over plain TCP (newline-delimited JSON-RPC, see docs/view-stream.md),
subscribes to the `viewstream` topic, reads the schema, decodes the binary
frames (they arrive hex-encoded on this transport) and prints a summary.

    python3 tools/viewstream_client.py --addr 127.0.0.1:7777 --step 30
    python3 tools/viewstream_client.py --addr 127.0.0.1:7777 --frames 5 --show 3

With --step N it starts a play session (if none runs) and steps N ticks, then
waits for the frame of that tick. The last line printed is one JSON object:
{"tick": ..., "entities": ..., "flags": ..., "fnv": "0x..."}, where fnv is the
FNV-1a 64 of the entity records of that frame (the same checksum the C test
client computes).
"""
import argparse
import json
import socket
import struct
import sys
import time

MAGIC = b"OVS1"
HEADER_LEN = 56
RECORD_LEN = 48
FLAG_ROLLED_BACK, FLAG_DISCONTINUITY, FLAG_PAUSED = 1, 2, 4
SHAPES = ["circle", "quad", "capsule"]
MODES = ["prediction", "snapshot", "none"]
# Format version 2: the 3D frame (message type 3), 88 byte records.
RECORD3D_LEN = 88
SHAPES3 = ["sphere", "box", "capsule", "plane"]
STYLE_CHECKER = 1


def fnv1a64(data, h=0xCBF29CE484222325):
    for b in data:
        h ^= b
        h = (h * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


def decode_frame(b):
    """Decodes a ViewFrame message (docs/view-stream.md) into a dict."""
    magic, version, msg_type, flags = struct.unpack_from("<4sHBB", b, 0)
    if magic != MAGIC or version > 1 or msg_type != 1:
        raise ValueError("not a version 1 view frame")
    tick, verified, seq, rb_from, rb_to, count, props_len = struct.unpack_from("<QQQQQII", b, 8)
    if len(b) != HEADER_LEN + count * RECORD_LEN + props_len:
        raise ValueError("frame size does not match its header")
    entities = []
    for i in range(count):
        (eid, kind, shape, mode, size, half_y, r, g, bl, a, px, py, pr, cx, cy, cr) = struct.unpack_from(
            "<QHBBff4B6f", b, HEADER_LEN + i * RECORD_LEN
        )
        entities.append(
            {
                "id": eid,
                "index": eid & 0xFFFFFFFF,
                "version": eid >> 32,
                "kind": kind,
                "shape": SHAPES[shape],
                "mode": MODES[mode],
                "size": size,
                "half_y": half_y,
                "rgba": (r, g, bl, a),
                "prev": (px, py, pr),
                "cur": (cx, cy, cr),
            }
        )
    return {
        "tick": tick,
        "verified_tick": verified,
        "seq": seq,
        "flags": flags,
        "rollback": (rb_from, rb_to) if flags & FLAG_ROLLED_BACK else None,
        "entities": entities,
        "records": b[HEADER_LEN : HEADER_LEN + count * RECORD_LEN],
        "props": b[HEADER_LEN + count * RECORD_LEN :],
    }


def decode_frame3(b):
    """Decodes a 3D ViewFrame (format version 2, message type 3) into a dict.

    Entity `prev` and `cur` are 7-tuples: x, y, z position then the x, y, z, w quaternion.
    """
    magic, version, msg_type, flags = struct.unpack_from("<4sHBB", b, 0)
    if magic != MAGIC or version > 2 or msg_type != 3:
        raise ValueError("not a version 2 3D view frame")
    tick, verified, seq, rb_from, rb_to, count, props_len = struct.unpack_from("<QQQQQII", b, 8)
    if len(b) != HEADER_LEN + count * RECORD3D_LEN + props_len:
        raise ValueError("frame size does not match its header")
    entities = []
    for i in range(count):
        v = struct.unpack_from("<QHBB3f4B4B7f7f", b, HEADER_LEN + i * RECORD3D_LEN)
        eid, kind, shape, mode = v[0:4]
        entities.append(
            {
                "id": eid,
                "index": eid & 0xFFFFFFFF,
                "version": eid >> 32,
                "kind": kind,
                "shape": SHAPES3[shape],
                "mode": MODES[mode],
                "size": v[4:7],
                "rgba": v[7:11],
                "roughness": v[11],
                "metallic": v[12],
                "checker": bool(v[13] & STYLE_CHECKER),
                "prev": v[15:22],
                "cur": v[22:29],
            }
        )
    return {
        "dimensions": 3,
        "tick": tick,
        "verified_tick": verified,
        "seq": seq,
        "flags": flags,
        "rollback": (rb_from, rb_to) if flags & FLAG_ROLLED_BACK else None,
        "entities": entities,
        "records": b[HEADER_LEN : HEADER_LEN + count * RECORD3D_LEN],
        "props": b[HEADER_LEN + count * RECORD3D_LEN :],
    }


def decode_message(b):
    """Decodes a frame message of either dimension; None for other message types."""
    if len(b) < 8:
        raise ValueError("message too short")
    if b[6] == 1:
        return decode_frame(b)
    if b[6] == 3:
        return decode_frame3(b)
    return None


class Erp:
    """Newline-delimited JSON-RPC 2.0 over TCP."""

    def __init__(self, addr, token=None, timeout=30.0):
        host, port = addr.rsplit(":", 1)
        self.sock = socket.create_connection((host, int(port)), timeout=timeout)
        self.buf = b""
        self.next_id = 1
        self.schema = None
        self.frames = []  # decoded frames, oldest first
        if token:
            self.call("auth", {"token": token})

    def _read_message(self, deadline):
        while b"\n" not in self.buf:
            self.sock.settimeout(max(0.01, deadline - time.time()))
            chunk = self.sock.recv(1 << 16)
            if not chunk:
                raise ConnectionError("the host closed the connection")
            self.buf += chunk
        line, self.buf = self.buf.split(b"\n", 1)
        return json.loads(line)

    def _note(self, msg):
        method = msg.get("method")
        if method == "watch.viewstream.schema":
            self.schema = msg["params"]
        elif method == "watch.viewstream":
            p = msg["params"]
            assert p["encoding"] == "hex"
            data = bytes.fromhex(p["data"])
            frame = decode_message(data)  # type 2 is an event batch: None
            if frame is not None:
                self.frames.append(frame)

    def call(self, method, params=None, timeout=30.0):
        rid = self.next_id
        self.next_id += 1
        req = {"jsonrpc": "2.0", "id": rid, "method": method, "params": params or {}}
        self.sock.sendall(json.dumps(req).encode() + b"\n")
        deadline = time.time() + timeout
        while True:
            msg = self._read_message(deadline)
            if msg.get("id") == rid:
                if "error" in msg:
                    raise RuntimeError("%s: %s" % (method, msg["error"].get("message")))
                return msg.get("result")
            self._note(msg)

    def pump(self, timeout):
        """Reads one message (or times out) and files it."""
        try:
            self._note(self._read_message(time.time() + timeout))
        except socket.timeout:
            pass

    def wait_frame(self, min_tick, timeout=30.0):
        end = time.time() + timeout
        while time.time() < end:
            while self.frames:
                f = self.frames.pop(0)
                if f["tick"] >= min_tick:
                    return f
            self.pump(0.2)
        raise TimeoutError("no frame of tick >= %d" % min_tick)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--addr", required=True, help="host:port of the ERP server")
    ap.add_argument("--token", help="ERP token (not needed for a dev-mode host)")
    ap.add_argument("--step", type=int, default=0, help="start a session if needed and step this many ticks")
    ap.add_argument("--frames", type=int, default=0, help="without --step: print this many frames as they arrive")
    ap.add_argument("--show", type=int, default=0, help="print the first N entity records of the last frame")
    args = ap.parse_args()

    c = Erp(args.addr, args.token)
    c.call("watch.subscribe", {"topics": ["viewstream"], "max_fps": 1000})
    end = time.time() + 10
    while c.schema is None and time.time() < end:
        c.pump(0.2)
    if c.schema is None:
        sys.exit("no schema received")
    s = c.schema
    print("game %s, %d players, %d ticks/s, input %d bytes, kinds %s" % (
        s["game"], s["player_count"], s["tick_rate"], s["input"]["size"], [k["name"] for k in s["kinds"]]))

    last = None
    if args.step:
        if c.call("sim.state")["mode"] != "play":
            c.call("sim.start", {})
        start_tick = c.call("sim.state")["head_tick"]
        c.call("sim.step", {"n": args.step})
        last = c.wait_frame(start_tick + args.step)
    else:
        for _ in range(args.frames):
            last = c.wait_frame(0)
            print("tick %d: %d entities, flags %d" % (last["tick"], len(last["entities"]), last["flags"]))
    if last is None:
        return
    for e in last["entities"][: args.show]:
        if last.get("dimensions") == 3:
            print("  id %016x kind %d %s at (%.3f, %.3f, %.3f) was (%.3f, %.3f, %.3f)" % (
                e["id"], e["kind"], e["shape"], *e["cur"][:3], *e["prev"][:3]))
        else:
            print("  id %016x kind %d %s at (%.3f, %.3f) was (%.3f, %.3f)" % (
                e["id"], e["kind"], e["shape"], e["cur"][0], e["cur"][1], e["prev"][0], e["prev"][1]))
    print(json.dumps({
        "tick": last["tick"],
        "entities": len(last["entities"]),
        "flags": last["flags"],
        "fnv": "0x%016x" % fnv1a64(last["records"]),
    }))


if __name__ == "__main__":
    main()
