"""Official rules load consistently from package resources and source checkouts."""
import json
from pathlib import Path

import pytest

from tuya_interpreter import load_builtin_rules, merge_rules
from tuya_interpreter import rules as loader


def test_builtin_rules_include_all_official_layers_and_return_fresh_data():
    builtin = Path(__file__).resolve().parents[1] / "rules" / "builtin"
    layers = [json.loads(p.read_text()) for p in sorted(builtin.glob("*.json")) if p.name != "manifest.json"]
    expected = merge_rules(layers)["rules"]
    pack = load_builtin_rules()
    assert pack == expected
    assert "5rta89nj" in pack["overrides"]
    pack["overrides"].clear()
    assert load_builtin_rules() == expected


def test_packaged_rules_load_in_order_and_ignore_manifest(tmp_path, monkeypatch):
    builtin = tmp_path / "rules" / "builtin"
    builtin.mkdir(parents=True)
    # Create in reverse filename order to catch reliance on directory order.
    (builtin / "20-later.json").write_text('{"overrides":{"example":{"device":{"label":"Later"}}}}')
    (builtin / "10-earlier.json").write_text('{"overrides":{"example":{"device":{"label":"Earlier"}}}}')
    base = Path(__file__).resolve().parents[1] / "rules" / "builtin" / "00-default.json"
    (builtin / "00-default.json").write_text(base.read_text())
    (builtin / "manifest.json").write_text("not a rule layer")
    monkeypatch.setattr(loader, "files", lambda package: tmp_path)
    assert load_builtin_rules()["overrides"]["example"]["device"]["label"] == "Later"
    (builtin / "20-later.json").write_text('{"version":-1}')
    with pytest.raises(ValueError):
        load_builtin_rules()
