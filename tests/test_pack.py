"""The official rule pack: its manifest is current and every file is a rule layer the bundled rules take.
Fetching and copying the pack is a host's (rustuya-local's `host.pack`)."""
import json
import subprocess
import sys
from pathlib import Path

from tuya_interpreter import merge_rules

ROOT = Path(__file__).resolve().parent.parent
PACK = ROOT / "rules" / "builtin"


def test_the_manifest_is_current():
    check = subprocess.run([sys.executable, str(ROOT / "scripts" / "build_pack.py"), "--check"], capture_output=True,
                           check=False, text=True)
    assert check.returncode == 0, check.stderr


def test_every_pack_file_is_a_rule_layer_for_this_format():
    entries = json.loads((PACK / "manifest.json").read_text())["files"]
    assert all(e.get("requires", 0) >= 4 for e in entries), "rule-layer files need `requires: 4`"
    files = [PACK / e["name"] for e in entries]
    merged = merge_rules([json.loads(p.read_text()) for p in files], tolerant=True)
    assert not merged["warnings"]
