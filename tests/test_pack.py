"""The published override pack: its manifest is current and every file is a rule layer the bundled rules take.
Fetching and copying the pack is a host's (rustuya-local's `host.pack`)."""
import json
import subprocess
import sys
from pathlib import Path

from tuya2ildevice.native import merge_rule_files, rule_paths

ROOT = Path(__file__).resolve().parent.parent
PACK = ROOT / "pack"


def test_the_manifest_is_current():
    check = subprocess.run([sys.executable, str(ROOT / "scripts" / "build_pack.py"), "--check"], capture_output=True,
                           check=False, text=True)
    assert check.returncode == 0, check.stderr


def test_every_pack_file_is_a_rule_layer_for_this_format():
    entries = json.loads((PACK / "manifest.json").read_text())["files"]
    assert all(e.get("requires", 0) >= 4 for e in entries), "rule-layer files need `requires: 4`"
    files = [PACK / e["name"] for e in entries]
    merged = merge_rule_files([*rule_paths(), *files], tolerant=True)
    assert not merged["warnings"]
