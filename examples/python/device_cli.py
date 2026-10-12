"""One descriptor-driven CLI for devices with different DP layouts and scales."""
import argparse
import json
import time
from contextlib import ExitStack
from pathlib import Path

from toggle_cli import unwrap_dps

from tuya_interpreter import (
    Absent,
    Command,
    Connected,
    Descriptor,
    Disconnected,
    Event,
    Message,
    Reject,
    SendMessage,
    TuyaDriver,
    Value,
    load_builtin_rules,
    merge_rules,
)


class MemoryDevice:
    def __init__(self, dps):
        self.dps = dict(dps)

    def status(self):
        return {"dps": dict(self.dps)}

    def set_multiple_values(self, dps):
        self.dps.update(dps)
        return self.status()

    def close(self):
        pass


class DeviceSession:
    """UI uses only the descriptor; feature names and DP IDs are never hardcoded."""
    def __init__(self, metadata, rules, transport):
        self.driver = TuyaDriver(metadata, rules=rules)
        self.transport = transport
        self.values = {}

    def handle(self, inp):
        for effect in self.driver.handle(time.monotonic(), inp):
            if isinstance(effect, Descriptor):
                print(f"Device: {effect.desc.get('label', 'Unnamed')}")
            elif isinstance(effect, Value):
                self.values[effect.prop] = effect.value
            elif isinstance(effect, Absent):
                self.values.pop(effect.prop, None)
            elif isinstance(effect, Event):
                print(f"Event: {effect.prop} {effect.kind}")
            elif isinstance(effect, Reject):
                print(f"Rejected {effect.prop}: {effect.code} {effect.reason}")
            elif isinstance(effect, SendMessage):
                if effect.channel != "set":
                    raise RuntimeError("Example supports only set transport effects")
                dps = effect.json["dps"]
                print(f"Interpreter → wire: {json.dumps(dps)}")
                self.feed(self.transport.set_multiple_values(dps), "passive")
            else:
                raise RuntimeError("This on-demand example does not run timers or persist settings")  # noqa: TRY004

    def feed(self, response, channel):
        dps = unwrap_dps(response)
        if dps:
            self.handle(Message(channel, dps))
        return dps

    def read(self):
        response = self.transport.status()
        print(f"Wire → interpreter: {json.dumps(unwrap_dps(response))}")
        if not self.feed(response, "state"):
            raise RuntimeError("Status contains no DPS")

    def show(self):
        print("Property             Type      Access       Value / unit / constraints")
        for prop, spec in self.driver.descriptor["props"].items():
            value = json.dumps(self.values.get(prop), ensure_ascii=False)
            unit = spec.get("unit", "")
            details = [f"{k}={json.dumps(spec[k], ensure_ascii=False)}" for k in ("min", "max", "step", "options") if k in spec]
            access = "read/write" if spec.get("rw") else "read only"
            print(f"{prop:20} {spec['type']:9} {access:12} {value} {unit} {' '.join(details)}".rstrip())
        print("Available writes (from descriptor):")
        for prop, spec in self.driver.descriptor["props"].items():
            if spec.get("rw"):
                print(f"  set {prop} <JSON value>")
                if spec["type"] == "binary":
                    print(f"  toggle {prop}")

    def execute(self, line):
        parts = line.strip().split(maxsplit=2)
        if not parts:
            return True
        command = parts[0]
        if command in ("quit", "exit"):
            return False
        if command in ("read", "status"):
            self.read()
            self.show()
        elif command in ("props", "help"):
            self.show()
        elif command == "describe":
            print(json.dumps(self.driver.descriptor, indent=2, ensure_ascii=False))
        elif command == "set" and len(parts) == 3:
            # Let the interpreter enforce property existence, type, range and access.
            self.handle(Command(parts[1], json.loads(parts[2])))
            self.read()
            self.show()
        elif command == "toggle" and len(parts) == 2:
            prop = parts[1]
            spec = self.driver.descriptor["props"].get(prop, {})
            if spec.get("type") != "binary" or not spec.get("rw"):
                raise ValueError("Select a writable binary property shown by props")
            self.read()
            current = self.values.get(prop)
            if not isinstance(current, bool):
                raise ValueError("Property state is unknown")
            self.handle(Command(prop, not current))
            self.read()
            self.show()
        else:
            print("Commands: props, read, describe, set <property> <JSON>, toggle <property>, quit")
        return True

    def run(self):
        self.handle(Connected())
        self.read()
        self.show()
        print("Commands: props, read, describe, set <property> <JSON>, toggle <property>, quit")
        while True:
            try:
                line = input("device> ")
            except EOFError:
                break
            try:
                if not self.execute(line):
                    break
            except (ValueError, RuntimeError) as error:
                print(error)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--device", choices=("a", "b"), default="a", help="Fictional device profile")
    parser.add_argument("--memory", action="store_true", help="In-memory transport instead of tuyamock TCP")
    args = parser.parse_args(argv)
    root = Path(__file__).resolve().parents[1] / "devices"
    metadata = json.loads((root / f"device-{args.device}.json").read_text(encoding="utf-8"))
    layer = json.loads((root / "rules.json").read_text())
    rules = merge_rules([layer], base=load_builtin_rules())["rules"]
    # Host-only setup: these example codecs are default, so status values are raw.
    # This mapping initializes the virtual endpoint; the UI never uses it.
    initial = {dp: metadata["status"][entry["status_code"]] for dp, entry in metadata["local_strategy"].items()}
    with ExitStack() as resources:
        if args.memory:
            transport = MemoryDevice(initial)
        else:
            try:
                import tinytuya
                from tuyamock import MockDevice
            except ImportError:
                parser.error("Install dependencies: pip install tinytuya tuyamock; or use --memory")
            key = "0123456789abcdef"
            mock = resources.enter_context(MockDevice(
                local_key=key, version="3.5", host="127.0.0.1", port=0,
                gw_id=metadata["id"], dps=initial,
            ))
            transport = tinytuya.Device(metadata["id"], "127.0.0.1", key, version=3.5, port=mock.port)
            transport.set_socketTimeout(5)
            print(f"tuyamock: 127.0.0.1:{mock.port}")
        resources.callback(transport.close)
        session = DeviceSession(metadata, rules, transport)
        try:
            session.run()
        except KeyboardInterrupt:
            print()
        finally:
            session.handle(Disconnected())


if __name__ == "__main__":
    main()
