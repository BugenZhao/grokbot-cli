#!/usr/bin/env python3
"""Minimal, dependency-free reader for Electron .asar archives.

Usage:
  asar.py list    <app.asar>               # list files with sizes
  asar.py extract <app.asar> <out_dir>     # extract every packed file

Format: [u32 4][u32 header_size][u32 payload_size][u32 json_len][json header]...
File data begins at 8 + header_size; each entry's "offset" is relative to that.
Entries marked "unpacked" live in app.asar.unpacked/ next to the archive.
"""
import json, os, struct, sys


def read_header(path):
    with open(path, "rb") as f:
        _, header_size, _, json_len = struct.unpack("<4I", f.read(16))
        header = json.loads(f.read(json_len))
    return header, 8 + header_size


def walk(node, prefix=""):
    for name, child in node.get("files", {}).items():
        p = f"{prefix}{name}"
        if "files" in child:
            yield from walk(child, p + "/")
        else:
            yield p, child


def main():
    cmd, path = sys.argv[1], sys.argv[2]
    header, base = read_header(path)
    if cmd == "list":
        for p, e in walk(header):
            print(f"{e.get('size', 0):>10}  {p}{'  (unpacked)' if e.get('unpacked') else ''}")
    elif cmd == "extract":
        out = sys.argv[3]
        with open(path, "rb") as f:
            for p, e in walk(header):
                if e.get("unpacked") or "offset" not in e:
                    continue
                dst = os.path.join(out, p)
                os.makedirs(os.path.dirname(dst), exist_ok=True)
                f.seek(base + int(e["offset"]))
                with open(dst, "wb") as o:
                    o.write(f.read(e["size"]))
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
