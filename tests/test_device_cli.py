"""Different wire layouts expose the same descriptor-driven controls and values."""
import importlib
import json
from pathlib import Path

import pytest

from tuya_interpreter import load_builtin_rules, merge_rules

ROOT = Path(__file__).resolve().parents[1]


@pytest.mark.parametrize("profile,switch_dp,temp_dp", [("a", "1", "2"), ("b", "101", "102")])
@pytest.mark.parametrize("tcp", [False, True])
def test_descriptor_cli_maps_devices_without_feature_specific_ui(profile, switch_dp, temp_dp, tcp, monkeypatch, capsys):
    monkeypatch.syspath_prepend(str(ROOT / "examples" / "python"))
    cli = importlib.import_module("device_cli")
    metadata = json.loads((ROOT / "examples" / "devices" / f"device-{profile}.json").read_text())
    layer = json.loads((ROOT / "examples" / "devices" / "rules.json").read_text())
    rules = merge_rules([layer], base=load_builtin_rules())["rules"]
    initial = {dp: metadata["status"][entry["status_code"]] for dp, entry in metadata["local_strategy"].items()}
    from contextlib import ExitStack

    with ExitStack() as resources:
        if tcp:
            tinytuya = pytest.importorskip("tinytuya")
            tuyamock = pytest.importorskip("tuyamock")
            mock = resources.enter_context(tuyamock.MockDevice(
                local_key="0123456789abcdef", version="3.5", dps=initial, gw_id=metadata["id"],
            ))
            transport = tinytuya.Device(metadata["id"], "127.0.0.1", "0123456789abcdef", version=3.5, port=mock.port)
        else:
            transport = cli.MemoryDevice(initial)
        resources.callback(transport.close)
        session = cli.DeviceSession(metadata, rules, transport)
        session.handle(cli.Connected())
        session.read()
        assert session.values["temp_current"] == 23.5
        assert session.driver.descriptor["props"]["temp_current"]["unit"] == "°C"
        session.show()
        menu = capsys.readouterr().out
        assert "set switch_1 <JSON value>" in menu
        assert "set temp_current" not in menu
        session.execute("set switch_1 true")
        session.execute("set temp_current 30")
        session.execute('set switch_1 "invalid"')
        session.execute("toggle switch_1")
        wire = cli.unwrap_dps(transport.status())
        assert wire == {switch_dp: False, temp_dp: initial[temp_dp]}
        output = capsys.readouterr().out
        assert f'Interpreter → wire: {{"{switch_dp}": true}}' in output
        assert f'Interpreter → wire: {{"{switch_dp}": false}}' in output
        assert output.count("Interpreter → wire:") == 2
        assert "Rejected temp_current: read_only" in output
        assert "Rejected switch_1" in output
