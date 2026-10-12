"""Exercise the example's interpreter/transport boundary without contacting a device."""
import importlib.util
import json
import sys
from pathlib import Path
from types import SimpleNamespace

import pytest

from tuya_interpreter import Command

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("toggle_cli", ROOT / "examples" / "python" / "toggle_cli.py")
cli = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cli)


def test_demo_commands_use_interpreter(monkeypatch, capsys):
    commands = iter(["on", "toggle", "off", "quit"])
    monkeypatch.setattr("builtins.input", lambda prompt: next(commands))
    cli.main(["--demo"])
    output = capsys.readouterr().out
    assert 'Sending DPS: {"1": true}' in output
    assert 'Sending DPS: {"1": false}' in output
    assert "available = false" in output


def test_invalid_command_is_rejected_before_transport_write(capsys):
    transport = cli.DemoTransport()
    session = cli.SwitchSession(cli.load_device(ROOT / "examples" / "devices" / "toggle-switch.json"), transport)
    session.handle(Command("switch_1", "invalid"))
    assert transport.dps == {"1": False}
    assert "Rejected switch_1" in capsys.readouterr().out


def test_live_cli_uses_credentials_and_generated_dps(tmp_path, monkeypatch, capsys):
    device = cli.load_device(ROOT / "examples" / "devices" / "toggle-switch.json")
    device.update(id="test-device", ip="192.0.2.1", version="3.3", local_key="test-only-key")
    path = tmp_path / "devices.json"
    path.write_text(json.dumps([device]))
    calls = []

    class FakeDevice(cli.DemoTransport):
        def __init__(self, device_id, address, key, version):
            super().__init__()
            calls.append((device_id, address, key, version))

        def set_socketTimeout(self, timeout):
            assert timeout == 5

        def status(self):
            return {"data": {"dps": dict(self.dps)}}

        def set_multiple_values(self, dps):
            calls.append(dps)
            self.dps.update(dps)
            return {}  # ACK without state must be followed by a read.

        def close(self):
            calls.append("closed")

    monkeypatch.delenv("TUYA_LOCAL_KEY", raising=False)
    monkeypatch.setitem(sys.modules, "tinytuya", SimpleNamespace(Device=FakeDevice))
    commands = iter(["on", "quit"])
    monkeypatch.setattr("builtins.input", lambda prompt: next(commands))
    cli.main(["--file", str(path), "--device-id", "test-device"])
    assert calls == [("test-device", "192.0.2.1", "test-only-key", 3.3), {"1": True}, "closed"]
    output = capsys.readouterr().out
    assert "switch_1 = true" in output
    assert "test-only-key" not in output


def test_transport_failure_is_not_interpreted_as_empty_state():
    with pytest.raises(RuntimeError, match="transport error"):
        cli.unwrap_dps({"Error": "Network error", "Err": "901"})


def test_tuyamock_cli_round_trip_over_real_tcp(monkeypatch, capsys):
    tuyamock = pytest.importorskip("tuyamock")
    pytest.importorskip("tinytuya")
    original = tuyamock.MockDevice
    devices = []

    def start_mock(**kwargs):
        mock = original(**kwargs)
        devices.append(mock)
        return mock

    monkeypatch.setattr(tuyamock, "MockDevice", start_mock)
    commands = iter(["on", "read", "toggle", "quit"])
    monkeypatch.setattr("builtins.input", lambda prompt: next(commands))
    cli.main(["--mock"])
    assert dict(devices[0].dps) == {"1": False}
    output = capsys.readouterr().out
    assert "tuyamock: 127.0.0.1:" in output
    assert "switch_1 = true" in output
    assert "switch_1 = false" in output
    assert 'Sending DPS: {"1": true}' in output
    assert 'Sending DPS: {"1": false}' in output
