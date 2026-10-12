"""Optional protocol round trip for the separately built Rust example."""
import os
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
BINARY = os.environ.get("TUYA_RUST_TOGGLE_BIN")


@pytest.mark.skipif(not BINARY, reason="Set TUYA_RUST_TOGGLE_BIN to the built Rust CLI")
@pytest.mark.parametrize("version", ["3.3", "3.5"])
def test_rustuya_interpreter_round_trip(version):
    pytest.importorskip("tuyamock")
    server = subprocess.Popen(
        [sys.executable, "-m", "tuyamock", "--host", "127.0.0.1", "--port", "0",
         "--version", version, "--local-key", "0123456789abcdef",
         "--gw-id", "example-toggle-switch", "--dps", '{"1":false}'],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
    )
    try:
        port = server.stdout.readline().strip()
        assert port.isdigit()
        result = subprocess.run(
            [BINARY, "--ip", "127.0.0.1", "--port", port, "--version", version],
            input="on\ntoggle\nread\nquit\n", capture_output=True, text=True, timeout=30, check=False,
            env={**os.environ, "TUYA_LOCAL_KEY": "0123456789abcdef"}, cwd=ROOT,
        )
        assert result.returncode == 0, result.stderr
        assert 'Sending DPS: {"1":true}' in result.stdout
        assert 'Sending DPS: {"1":false}' in result.stdout
        assert "switch_1 = true" in result.stdout
        # Every read after toggle must reflect off, including write confirmation.
        after_toggle = result.stdout.split('Sending DPS: {"1":false}', 1)[1]
        assert "switch_1 = false" in after_toggle
        assert "switch_1: false" in after_toggle
        assert "switch_1: true" not in after_toggle
        assert "available = false" in result.stdout
    finally:
        server.terminate()
        try:
            server.wait(timeout=5)
        except subprocess.TimeoutExpired:
            server.kill()
            server.wait(timeout=5)


@pytest.mark.skipif(not BINARY, reason="Set TUYA_RUST_TOGGLE_BIN to the built Rust CLI")
@pytest.mark.parametrize("profile,switch_dp,temp_dp,raw", [("a", "1", "2", 235), ("b", "101", "102", 2350)])
def test_rust_descriptor_ui_supports_both_profiles(profile, switch_dp, temp_dp, raw):
    import json

    pytest.importorskip("tuyamock")
    server = subprocess.Popen(
        [sys.executable, "-m", "tuyamock", "--host", "127.0.0.1", "--port", "0",
         "--version", "3.5", "--local-key", "0123456789abcdef",
         "--gw-id", f"example-device-{profile}", "--dps", json.dumps({switch_dp: False, temp_dp: raw})],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
    )
    try:
        port = server.stdout.readline().strip()
        assert port.isdigit()
        result = subprocess.run(
            [BINARY, "--file", f"examples/devices/device-{profile}.json", "--ip", "127.0.0.1",
             "--port", port, "--version", "3.5", "--rules-layer", "examples/devices/rules.json"],
            input="props\nset switch_1 true\nset temp_current 30\ntoggle switch_1\nquit\n",
            capture_output=True, text=True, timeout=30, check=False,
            env={**os.environ, "TUYA_LOCAL_KEY": "0123456789abcdef"}, cwd=ROOT,
        )
        assert result.returncode == 0, result.stderr
        assert "temp_current: number / read only / 23.5 °C" in result.stdout
        assert f'Sending DPS: {{"{switch_dp}":true}}' in result.stdout
        assert f'Sending DPS: {{"{switch_dp}":false}}' in result.stdout
        assert "Rejected temp_current: ReadOnly" in result.stdout
        assert "set switch_1 <JSON value>" in result.stdout
        assert "set temp_current <JSON value>" not in result.stdout
    finally:
        server.terminate()
        try:
            server.wait(timeout=5)
        except subprocess.TimeoutExpired:
            server.kill()
            server.wait(timeout=5)
