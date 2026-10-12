"""Interactive Boolean switch: TinyTuya transports DPS; the interpreter owns semantics."""
import argparse
import getpass
import json
import os
import time
from contextlib import ExitStack
from pathlib import Path

from tuya_interpreter import (
    Command,
    Connected,
    Descriptor,
    Disconnected,
    Message,
    Reject,
    SendMessage,
    TuyaDriver,
    Value,
    load_builtin_rules,
)


class DemoTransport:
    """In-memory device using the sample's DP 1; no network or TinyTuya dependency."""
    def __init__(self):
        self.dps = {"1": False}

    def status(self):
        return {"dps": dict(self.dps)}

    def set_multiple_values(self, dps):
        self.dps.update(dps)
        return self.status()

    def close(self):
        pass


def load_device(path, device_id=None):
    payload = json.loads(path.read_text(encoding="utf-8"))
    if isinstance(payload, dict) and "devices" in payload:
        payload = payload["devices"]
    devices = [payload] if isinstance(payload, dict) and "id" in payload else payload
    if not isinstance(devices, list) or not all(isinstance(d, dict) for d in devices):
        raise ValueError("Expected a device object or a list of device objects")
    selected = [d for d in devices if device_id is None or d.get("id") == device_id]
    if len(selected) != 1:
        raise ValueError("Select exactly one device with --device-id")
    return selected[0]


def unwrap_dps(response):
    """Accept TinyTuya's top-level and nested DPS envelopes without exposing credentials."""
    if not isinstance(response, dict):
        raise RuntimeError("Device returned no response")  # noqa: TRY004 - transport failure
    if "Error" in response or "Err" in response:
        raise RuntimeError(f"TinyTuya transport error (code {response.get('Err', 'unknown')})")
    dps = response.get("dps")
    if dps is None and isinstance(response.get("data"), dict):
        dps = response["data"].get("dps")
    if dps is None:
        return {}  # Some successful write acknowledgements contain no state.
    if not isinstance(dps, dict):
        raise RuntimeError("Device returned an invalid DPS map")  # noqa: TRY004 - transport failure
    return {str(dp): value for dp, value in dps.items()}


class SwitchSession:
    def __init__(self, device, transport, prop="switch_1"):
        self.driver = TuyaDriver(device, rules=load_builtin_rules())
        spec = self.driver.descriptor["props"].get(prop, {})
        if spec.get("type") != "binary" or not spec.get("rw"):
            raise ValueError("--property must select a writable binary property in the descriptor")
        self.transport = transport
        self.prop = prop
        self.values = {}

    def handle(self, inp):
        for output in self.driver.handle(time.monotonic(), inp):
            if isinstance(output, Descriptor):
                print(f"Device: {output.desc.get('label', 'Switch')}")
            elif isinstance(output, Value):
                self.values[output.prop] = output.value
                print(f"{output.prop} = {json.dumps(output.value)}")
            elif isinstance(output, Reject):
                print(f"Rejected {output.prop}: {output.code} {output.reason}")
            elif isinstance(output, SendMessage):
                if output.channel != "set":
                    raise RuntimeError("This example only supports set messages")
                dps = output.json["dps"]
                print(f"Sending DPS: {json.dumps(dps)}")
                response = self.transport.set_multiple_values(dps)
                self.feed(response, "passive")

    def feed(self, response, channel):
        dps = unwrap_dps(response)
        if dps:
            self.handle(Message(channel, dps))
        return dps

    def read(self):
        if not self.feed(self.transport.status(), "state"):
            raise RuntimeError("Status response contains no DPS; check connection and protocol version")
        print(f"{self.prop}: {json.dumps(self.values.get(self.prop))}")

    def run(self):
        self.handle(Connected())
        self.read()
        print("Commands: on, off, toggle, read, describe, help, quit")
        while True:
            try:
                command = input("tuya> ").strip().lower()
            except EOFError:
                break
            if command in ("quit", "exit", "q"):
                break
            try:
                if command in ("on", "off", "toggle"):
                    if command == "toggle":
                        self.read()  # Decide from device readback, not optimistic state.
                        current = self.values.get(self.prop)
                        if not isinstance(current, bool):
                            raise RuntimeError("Switch state is unknown; cannot toggle")
                        value = not current
                    else:
                        value = command == "on"
                    self.handle(Command(self.prop, value))
                    self.read()  # Verify the device's actual state after the write.
                elif command == "read":
                    self.read()
                elif command == "describe":
                    print(json.dumps(self.driver.descriptor, indent=2, ensure_ascii=False))
                elif command:
                    print("Commands: on, off, toggle, read, describe, help, quit")
            except RuntimeError as error:
                print(error)
                print("Use read to retry or quit to disconnect.")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--demo", action="store_true", help="Use an in-memory device; no network")
    modes.add_argument("--mock", action="store_true", help="Start tuyamock and connect through TinyTuya on loopback")
    parser.add_argument("--file", type=Path, help="Private device JSON or tuyawizard device list")
    parser.add_argument("--device-id", help="Select a device from a list")
    parser.add_argument("--property", default="switch_1", help="Writable binary IL property")
    parser.add_argument("--ip", help="LAN address; defaults to the device JSON's ip")
    parser.add_argument("--version", type=float, help="Tuya protocol version; defaults to the device JSON's version")
    parser.add_argument("--port", type=int, help="TCP port for a separately running device or mock (default: 6668)")
    args = parser.parse_args(argv)
    sample = args.demo or args.mock
    if sample and (args.file or args.device_id or args.ip or args.version or args.port or args.property != "switch_1"):
        parser.error("--demo/--mock use the included sample and cannot be combined with device options")
    if not sample and args.file is None:
        parser.error("Use --mock, --demo or --file with a real device's private metadata")
    transport = None
    session = None
    resources = ExitStack()
    try:
        path = Path(__file__).resolve().parents[1] / "devices" / "toggle-switch.json" if sample else args.file
        device = load_device(path, args.device_id)
        if args.demo:
            transport = DemoTransport()
        elif args.mock:
            try:
                import tinytuya
                from tuyamock import MockDevice
            except ImportError as error:
                raise ValueError("Install mock dependencies: pip install tinytuya tuyamock") from error
            # Public test key only; never derived from the real device export.
            key = "0123456789abcdef"
            mock = resources.enter_context(MockDevice(
                local_key=key, version="3.5", dps={"1": False},
                host="127.0.0.1", port=0, gw_id=device["id"],
            ))
            print(f"tuyamock: 127.0.0.1:{mock.port} (Tuya 3.5)")
            transport = tinytuya.Device(device["id"], "127.0.0.1", key, version=3.5, port=mock.port)
            transport.set_socketTimeout(5)
        else:
            if device.get("sub"):
                raise ValueError("This example supports direct LAN devices, not gateway subdevices")
            address = args.ip or device.get("ip")
            version = args.version or device.get("version")
            if not address or version is None:
                raise ValueError("Provide --ip and --version, or include ip and version in the device JSON")
            try:
                import tinytuya
            except ImportError as error:
                raise ValueError("Install the optional transport: pip install tinytuya") from error
            key = os.environ.get("TUYA_LOCAL_KEY") or device.get("local_key")
            if not key:
                key = getpass.getpass("Local key: ")
            if not key:
                raise ValueError("A local key is required")
            options = {"port": args.port} if args.port is not None else {}
            transport = tinytuya.Device(device["id"], address, key, version=float(version), **options)
            transport.set_socketTimeout(5)
        session = SwitchSession(device, transport, args.property)
        session.run()
    except (ValueError, RuntimeError, OSError) as error:
        parser.exit(1, f"{error}\n")
    except KeyboardInterrupt:
        print("\nDisconnected.")
    finally:
        if session is not None:
            session.handle(Disconnected())
        if transport is not None:
            transport.close()
        resources.close()


if __name__ == "__main__":
    main()
