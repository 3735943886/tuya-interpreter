"""Write pack/manifest.json from the files in pack/: name and SHA-256 of each, keeping the `requires` / `until` an
entry already has. Run it after changing a pack file; tests/test_pack.py fails while the manifest is stale.

    python scripts/build_pack.py [--check]
"""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

PACK = Path(__file__).resolve().parent.parent / "pack"


def build() -> str:
    path = PACK / "manifest.json"
    old = {e["name"]: e for e in json.loads(path.read_text())["files"]} if path.is_file() else {}
    files = []
    for p in sorted(PACK.iterdir()):
        if p.suffix != ".json" or p.name == "manifest.json" or p.name[0] in "._":
            continue
        entry = {"name": p.name, "sha256": hashlib.sha256(p.read_bytes()).hexdigest()}
        entry.update({k: v for k, v in old.get(p.name, {}).items() if k in ("requires", "until")})
        files.append(entry)
    return json.dumps({"version": 1, "files": files}, indent=2) + "\n"


if __name__ == "__main__":
    text = build()
    target = PACK / "manifest.json"
    if "--check" in sys.argv[1:]:
        sys.exit(0 if target.is_file() and target.read_text() == text else "pack/manifest.json is stale: run scripts/build_pack.py")
    target.write_text(text)
