#!/usr/bin/env python3
"""Fetch a SCAP Security Guide release and keep the datastreams the agents can use (Debian and Ubuntu), for the server
to hand to them: hosts then audit with recent content even without internet access or when their distribution's
package is old. Run at image build time:  scap-content.py <version> <sha256 of the zip> <output dir>

Only datastreams that define at least one profile are kept (0.1.82 ships an ssg-ubuntu2604-ds.xml with 2 rules and no
profile: a placeholder). index.json lists each file with its SHA-256, which the agent checks after downloading it.
"""
import hashlib
import json
import re
import sys
import urllib.request
import zipfile
from pathlib import Path

KEEP = re.compile(r"^ssg-(debian\d+|ubuntu\d{4})-ds\.xml$")


def main() -> None:
    version, want_sha, out = sys.argv[1], sys.argv[2].lower(), Path(sys.argv[3])
    out.mkdir(parents=True, exist_ok=True)
    url = f"https://github.com/ComplianceAsCode/content/releases/download/v{version}/scap-security-guide-{version}.zip"
    zpath = Path("/tmp/ssg.zip")
    with urllib.request.urlopen(url, timeout=600) as r, zpath.open("wb") as f:
        h = hashlib.sha256()
        while chunk := r.read(1 << 20):
            h.update(chunk)
            f.write(chunk)
    if h.hexdigest() != want_sha:
        sys.exit(f"SHA-256 mismatch for {url}: {h.hexdigest()} (expected {want_sha})")
    files = []
    with zipfile.ZipFile(zpath) as z:
        for info in z.infolist():
            name = info.filename.rsplit("/", 1)[-1]
            if not KEEP.match(name):
                continue
            data = z.read(info)
            if b"content_profile_" not in data:
                print(f"skipped {name}: no profile")
                continue
            (out / name).write_bytes(data)
            files.append({"name": name, "sha256": hashlib.sha256(data).hexdigest(), "size": len(data)})
            print(f"kept {name} ({len(data) // 1024} KiB)")
    zpath.unlink()
    if not files:
        sys.exit("no datastream kept")
    (out / "index.json").write_text(json.dumps({"version": version, "files": sorted(files, key=lambda f: f["name"])}, indent=1))


if __name__ == "__main__":
    main()
