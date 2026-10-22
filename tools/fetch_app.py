#!/usr/bin/env python3
"""Download the latest Grok Bot app.asar without installing the app (any OS).

  fetch_app.py [--out .cache/app] [--track stable]

1. Ask the app's own update feed for the newest macOS build (no auth needed):
     GET https://api2.cursor.sh/updates/api/update/darwin-arm64/sand/<current>/<machine-id>/stable
     -> {"url": ".../Grok_Bot_<ver>.zip", "name": "<ver>"}
2. Read the zip over HTTP range requests and pull out only
   `Grok Bot.app/Contents/Resources/app.asar` and `Contents/Info.plist`
   (~50 MB instead of the ~140 MB archive). The asar is platform-neutral JavaScript,
   so the macOS build is used everywhere; it is also a plain zip, which the Linux
   (AppImage) and Windows (Setup.exe) builds are not.

Prints the output directory; pass it to `extract_schema.py --app <dir>`.
"""
import argparse, io, json, os, secrets, shutil, sys, urllib.request, zipfile

FEED = "https://api2.cursor.sh/updates/api/update/darwin-arm64/{channel}/0.0.1/{machine}/stable"
# downloads.cursor.com rejects urllib's default "Python-urllib/x.y" user agent.
HEADERS = {"User-Agent": "grokbot-cli-fetch/1"}
CHANNELS = {"stable": "sand", "nightly": "sand-nightly", "dogfood": "sand-dogfood"}
WANTED = ("Contents/Resources/app.asar", "Contents/Info.plist")


class HttpRangeFile(io.RawIOBase):
    """Seekable read-only view of a URL, fetched lazily with Range requests."""

    CHUNK = 1 << 20

    def __init__(self, url):
        self.url, self.pos = url, 0
        req = urllib.request.Request(url, method="HEAD", headers=HEADERS)
        with urllib.request.urlopen(req) as r:
            if r.headers.get("Accept-Ranges") != "bytes":
                raise RuntimeError("server does not support range requests")
            self.size = int(r.headers["Content-Length"])
        self.buf_start, self.buf = 0, b""
        self.fetched = 0

    def readable(self):
        return True

    def seekable(self):
        return True

    def tell(self):
        return self.pos

    def seek(self, off, whence=0):
        self.pos = {0: off, 1: self.pos + off, 2: self.size + off}[whence]
        return self.pos

    def _fetch(self, start, end):
        req = urllib.request.Request(self.url, headers={**HEADERS, "Range": f"bytes={start}-{end - 1}"})
        with urllib.request.urlopen(req) as r:
            data = r.read()
        self.fetched += len(data)
        return data

    def read(self, n=-1):
        if n < 0:
            n = self.size - self.pos
        n = min(n, self.size - self.pos)
        if n <= 0:
            return b""
        end = self.pos + n
        if not (self.buf_start <= self.pos and end <= self.buf_start + len(self.buf)):
            # Read ahead so zipfile's many small reads do not each cost a request.
            fetch_end = min(self.size, max(end, self.pos + self.CHUNK))
            self.buf_start, self.buf = self.pos, self._fetch(self.pos, fetch_end)
        off = self.pos - self.buf_start
        data = self.buf[off:off + n]
        self.pos += len(data)
        return data


def latest(track):
    url = FEED.format(channel=CHANNELS[track], machine=secrets.token_hex(32))
    with urllib.request.urlopen(urllib.request.Request(url, headers=HEADERS)) as r:
        info = json.load(r)
    return info["name"], info["url"]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=".cache/app")
    ap.add_argument("--track", default="stable", choices=CHANNELS)
    args = ap.parse_args()

    version, url = latest(args.track)
    print(f"latest {args.track}: Grok Bot {version}\n  {url}", file=sys.stderr)
    dest = os.path.join(args.out, version)
    if os.path.exists(os.path.join(dest, "Contents/Resources/app.asar")):
        print("already downloaded", file=sys.stderr)
        print(dest)
        return

    src = HttpRangeFile(url)
    with zipfile.ZipFile(src) as z:
        for info in z.infolist():
            rel = info.filename.split(".app/", 1)[-1]
            if rel not in WANTED:
                continue
            out = os.path.join(dest, rel)
            os.makedirs(os.path.dirname(out), exist_ok=True)
            with z.open(info) as f, open(out + ".part", "wb") as o:
                shutil.copyfileobj(f, o, 1 << 20)
            os.replace(out + ".part", out)
            print(f"  {rel}: {info.file_size / 1e6:.1f} MB", file=sys.stderr)
    missing = [w for w in WANTED if not os.path.exists(os.path.join(dest, w))]
    if missing:
        sys.exit(f"not found in the archive: {missing}")
    print(f"  downloaded {src.fetched / 1e6:.1f} of {src.size / 1e6:.1f} MB", file=sys.stderr)
    print(dest)


if __name__ == "__main__":
    main()
