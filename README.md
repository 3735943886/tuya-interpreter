# tuya-interpreter

A portable Tuya device interpreter with bidirectional conversion. It interprets device metadata and DPS packets,
produces an [ildevice](https://github.com/3735943886/ildevice) intermediate device description,
values and events, and converts ildevice commands back into Tuya DPS.

ildevice is a human-readable, developer-friendly intermediate device model. It describes device
properties, values, events and commands in a common format that is easy to inspect and use in
downstream applications. The converter keeps Tuya knowledge in one place so hosts can use the
same device semantics across transports and languages.

```text
Tuya device JSON + rule pack
            │
            ▼
Tuya DPS ⇄ interpretation engine ⇄ ildevice descriptor / values / events / commands
```

The implementation is a Sans-IO Rust engine with Python and JavaScript/WebAssembly bindings
and a JSON C ABI. The host supplies data and time, and executes the returned effects.
The engine opens no sockets and reads no clock.

## Device JSON: what the converter uses

Use a JSON-safe export of a device populated by
[tuya-device-sharing-sdk](https://github.com/tuya/tuya-device-sharing-sdk), or a device entry from
[tuyawizard](https://github.com/3735943886/tuyawizard)'s `tuyadevices.json`.
The input is one decoded device object, rather than the entire device list or an SDK object.
Device metadata supplies the schema; live input uses Tuya's standard DP ID–value pairs,
such as `{"1": true, "2": 235}`. Hosts can obtain these pairs through
[rustuya](https://github.com/3735943886/rustuya),
[TinyTuya](https://github.com/jasonacox/tinytuya), or any other Tuya transport implementation.
The host unwraps the transport response and passes the flat DP map to the interpreter;
the transport library does not need to supply the device schema.

There is no separate `schema` field. Here, **device schema** means the definitions in
`function` and `status_range`, with `local_strategy` connecting local DP IDs to those definitions.

| JSON field | Contents | How it is used |
| --- | --- | --- |
| `id` | Device ID, as a string | Required for driver construction; device identity and device-specific overrides. |
| `name` | Device name | Descriptor label. |
| `category` | Tuya category such as `cz`, `dj` or `cl` | Selects the category's classification rules. |
| `product_id` | Product ID | Selects product-specific quirks and overrides; supplies model identity. |
| `product_name` | Product name | Default model name, unless a quirk or override supplies it. |
| `function` | Object keyed by DP code, containing writable feature definitions | Resolves command sources and writable properties, including types and allowed values. |
| `status_range` | Object keyed by DP code, containing reported feature definitions | Resolves state sources, types, ranges, scales, units and report behavior. |
| `local_strategy` | Object keyed by local DP ID | Maps raw DPS to codes and applies the named value conversion with its configuration. |
| `status` | Initial values keyed by DP code | Used by classification probes and initial device interpretation; live packets are supplied separately. |

Other connection fields, such as `local_key`, IP address, protocol version and parent-device routing,
belong to the host. Link availability is supplied through `Connected` and `Disconnected` inputs.

### Inside the schema fields

A `function` or `status_range` entry provides `code`, `type` and `values`.
`status_range` may also provide `report_type`, including `sum` for increments that need accumulation.
`values` can be a decoded object or a JSON string; both forms are accepted.

Supported DP types are `Boolean`, `Integer`, `Enum`, `Bitmap`, `String`, `Raw` and `Json`.
Integer definitions describe `min`, `max`, `scale`, `step` and an optional `unit`;
Enum definitions contain `range`, and Bitmap definitions can contain `label`.
The actual interpretation of Raw and Json values depends on the relevant codec and rules.

A `local_strategy` entry provides:

| Nested field | Purpose |
| --- | --- |
| `status_code` or a code in `config_item.statusFormat` | Connects the DP ID to a semantic code. |
| `value_convert` | Names the raw-to-code value conversion, such as `default` or `enum`. |
| `config_item` | Conversion configuration, including `valueType`, `valueDesc` and `enumMappingMap` where applicable. |

The host can also supply `TuyaDriver(..., dpmap={"1": "switch_1"})` when it already knows
an ID-to-code mapping. A mapping identifies a DP; its feature definitions still come from
schema metadata or rules/overrides.

### Example device entry

This illustrative plug record has one writable Boolean feature:

```json
{
  "id": "example-device",
  "name": "Desk plug",
  "category": "cz",
  "product_id": "example-product",
  "product_name": "Smart plug",
  "function": {
    "switch_1": {"code": "switch_1", "type": "Boolean", "values": {}}
  },
  "status_range": {
    "switch_1": {"code": "switch_1", "type": "Boolean", "values": {}}
  },
  "local_strategy": {
    "1": {
      "status_code": "switch_1",
      "value_convert": "default",
      "config_item": {"valueType": "Boolean", "valueDesc": {}}
    }
  },
  "status": {"switch_1": true}
}
```

`status["switch_1"]` is a code-keyed initial value. A local packet such as `{"1": true}`
is DP-ID-keyed and goes through the adapter. Do not feed a code-keyed SDK `status` map directly
as a DPS message. The host must adapt cloud updates to the driver's DP-ID input contract.

### SDK exports and tuyawizard exports

The Sharing SDK populates `device.function` and `device.status_range` with definition objects,
`device.local_strategy` with DP mappings, and `device.status` with code-keyed values.
To export an SDK device, recursively convert its objects into dictionaries and serialize DP ID
keys as strings. Passing the SDK's Python objects directly to `TuyaDriver` is not supported.

[tuyawizard's exporter](https://github.com/3735943886/tuyawizard/blob/master/tuyawizard/wizard.py)
already recursively converts SDK device objects and parses JSON object strings. Its saved device
list can therefore be decoded and individual entries passed to the driver.

A raw cloud API response is a different shape: specification responses contain `functions` and
`status` arrays, which the SDK turns into the code-keyed `function` and `status_range` objects.
Use the populated device export, or normalize the raw response to the structure above.

### Missing schema and unclassified DPs

Normal automatic classification needs feature definitions in `function` / `status_range`.
The engine does not infer a device's function from a DP number and current value alone.
Product quirks and `dp` overrides can supply missing definitions before classification.

`expose_unused=True` exposes DPs that the normal rules did not claim as generic properties.
It can use type/range metadata from `local_strategy.config_item` when feature definitions are missing.
Integer, Enum and Boolean properties are writable only when defined in `function`;
String, Raw, Json and Bitmap properties are read-only. This fallback does not invent semantic roles.

## Python quick start

```sh
pip install tuya-interpreter
```

Supported wheels include the native engine and official rule files. Runtime dependencies do not
include the Sharing SDK or tuyawizard: those are ways to obtain device metadata.
Building from source requires Cargo and Rust 1.88 or newer.

Official rules are bundled with the package. `load_builtin_rules()` reads the official layers
in filename order and returns a merged, validated pack. It also works in editable source checkouts.
The driver still takes `rules` explicitly and performs no rule-file I/O itself.

```python
from tuya_interpreter import Command, Connected, Message, TuyaDriver, load_builtin_rules

# device is a decoded device entry from the Sharing SDK or tuyawizard.
driver = TuyaDriver(device, rules=load_builtin_rules())
print(driver.descriptor)

# For the example plug above; the host supplies actual time and packets.
driver.handle(0.0, Connected())
values = driver.handle(1.0, Message("active", {"1": True}))
commands = driver.handle(2.0, Command("switch_1", "off"))
# commands contains SendMessage("set", {"dps": {"1": False}}).
# The host sends that payload to the device.
```

To load a saved tuyawizard device list:

```python
import json
from pathlib import Path

from tuya_interpreter import TuyaDriver, load_builtin_rules

devices = json.loads(Path("tuyadevices.json").read_text(encoding="utf-8"))
driver = TuyaDriver(devices[0], rules=load_builtin_rules())
```

Use `descriptor_of(device, rules=rules)` when only a descriptor is needed.
Keep one driver per device for live conversion, events, accumulated values and timers.

## Interactive examples: one UI, different Tuya devices

Two virtual devices expose the same switch and temperature properties through different DP IDs
and scales. The CLI builds its controls from the ildevice descriptor; it has no hardcoded
DP mapping, feature names or temperature conversion.

```sh
pip install tinytuya tuyamock
python examples/python/device_cli.py --device a
# After quitting, try the same UI with the other device:
python examples/python/device_cli.py --device b
```

Both accept `set switch_1 true` and report `temp_current = 23.5 °C`. Device A sends DP 1
and reports temperature as `235`; device B sends DP 101 and reports temperature as `2350`.
The interpreter applies each schema's mapping and scale and rejects writes to the read-only sensor.

Run from a source checkout with the interpreter installed. Use `--memory` for a demo without
network dependencies. See [the example guide](examples/README.md#same-application-different-devices)
for the profiles and [Rust/rustuya version](examples/README.md#rust-use-the-same-two-device-profiles).
The original [minimal toggle CLI](examples/python/toggle_cli.py) remains available for connection checks
and real-device use with private metadata.

## Rules and supported device functions

The [official rule pack](rules/README.md) contains category tables, entity specifications,
units, mappings, feature definitions, codecs' configuration and product-specific quirks.
The Rust engine supplies the interpreter and Tuya primitives. Changes expressible in the rule
language, including new platforms, do not require rebuilding the engine.

The converter assembles switch, button, select, number, sensor, binary sensor, event, light,
cover, fan, siren, valve, humidifier, climate, alarm and vacuum functions into ildevice.
Camera classification exists, but camera streams are outside the current IL assembly;
`driver.state["assembly"]["unsupported"]` records unsupported plans. Doorbell events carry a kind without
message text because the intermediate event contract has no text payload.

Support is determined by category, available feature definitions and rules; there is no requirement
to register every product separately. A matching category alone does not guarantee that every
feature of a product can be interpreted.

User rule layers merge recursively: arrays and scalars replace earlier values, `{"$delete": true}`
removes a member, `{"$append": [...]}` extends an array, and `null` is literal.
Load official layers in filename order, then user layers. Invalid final packs are rejected.
See [rule composition and the official manifest](rules/README.md).

```python
from tuya_interpreter import load_builtin_rules, merge_rules

custom_rules = merge_rules(user_layers, base=load_builtin_rules())["rules"]
driver = TuyaDriver(device, rules=custom_rules)
```

Overrides can correct DP definitions and codes, aliases, categories, descriptor metadata and
cover direction. Declarative converters add computed properties, commands, state and timers.
See [device customization](docs/device-customization.md) for examples and all cover/delta settings,
and [the engine documentation](docs/rust-engine.md#json-converters) for JSON programs.

## Runtime and host responsibilities

| Direction | Data |
| --- | --- |
| Host → driver | `Connected`, `Disconnected`, `Message(channel, flat_dps)`, `Timer(name)` |
| Driver → ildevice consumer | `Descriptor`, `Value`, `Absent`, `Event`, `Reject` |
| ildevice consumer → driver | `Command(prop, value)` |
| Driver → Tuya transport | `SendMessage("set", {"dps": ...})` |
| Driver → host | `SetTimer`, `CancelTimer`, `SettingsChanged` |

The host unwraps transport envelopes into flat DP-ID maps, manages authentication, connections,
subscriptions, retained-message policy and device routing, and sends returned commands.
It supplies `now`, executes timer requests and persists settings, and initiates device reads when needed.

Message channels have distinct semantics:

- `active`: device push; updates values, fires events and accumulates increments.
- `passive`: readback or live report; changed values are treated as live by converters,
  but events do not fire. Delta accumulation requires `delta.accept_passive`.
- `state`: snapshot; seeds values without treating retained commands as new movement or events.

Numeric values use their published native units. For example, `500 mA` stays `500` with unit `mA`;
a suggested display unit does not rescale the wire value. Commands are checked against the ildevice
contract before conversion and rejected with `Reject` when invalid. Brightness conversions can
introduce quantization. Covers use the device's position direction, configurable through cover settings;
garage/gate controls are read-only unless `allow_hazardous=True`.

Changing rules or overrides requires a replacement driver. `new.carry(old)` preserves compatible
delta totals; the host then seeds its current snapshot and replaces the descriptor.
With `device_settings=True`, setting writes return `SettingsChanged` for the host to save and reload.

## Rust, C ABI and WebAssembly

Rust hosts use `RulePack`, `Driver`, `DriverOptions`, `Input` and `Output` directly with driver
state retained in memory. Python uses the JSON C ABI; `TUYA_ENGINE_LIBRARY` selects an alternative
native library. JavaScript uses the same request protocol through WebAssembly.
All bindings share the rule interpreter and Tuya semantics.

See [native Rust API](docs/rust-engine.md#native-rust-api),
[C ABI](docs/rust-engine.md#c-abi), [other languages](docs/rust-engine.md#another-language)
and [local performance measurements](docs/performance.md).

## Development and validation

```sh
cargo test --locked --manifest-path rust/Cargo.toml
pip install -e ".[test]"
python -m pytest
```

Classification, values, commands and quirks are checked against fixtures and golden data derived
from Home Assistant's Tuya integration and the Sharing SDK. The pinned reference behavior and
intentional differences are described in [the engine specification](docs/engine-spec.md).
Tuya's standard category assignments take precedence over differing HA category assignments;
cover positions keep the device's direction rather than HA's mirrored representation.

SDK adapter comparisons require `tuya-device-sharing-sdk==0.2.15` from the test extras.
IL conformance tests use the specification checkout at `../ildevice` or `$ILDEVICE` and are skipped
when absent. Chain tests require the IL host's `ildevice.core` planner and are not collected without it.

| Path | Contents |
| --- | --- |
| `rust/engine/` | Interpreter, driver, Tuya primitives, C ABI and rule-pack JSON Schema |
| `src/tuya_interpreter/` | Python binding, inputs and outputs |
| `bindings/js/` | JavaScript/WASM binding and smoke test |
| `rules/builtin/` | Official rule layers and manifest |
| `scripts/` | Rule generators, golden-data tooling and benchmarks |
| `tests/` | Conversion, boundary, rule and conformance tests |

`rust/engine/schema.json` validates the **rule-pack format**; it is separate from the Tuya device
schema in `function` and `status_range`.

## Releases

The [release workflow](.github/workflows/release.yml) builds and tests 13 platform wheels:

| Platform | Architectures |
| --- | --- |
| Linux manylinux (glibc) | ARMv7, ARM64, x86, x64 |
| Linux musllinux (musl) | ARMv7, ARM64, x86, x64 |
| macOS | ARM64, x64 |
| Windows | ARM64, x86, x64 |

The planned first release uses tag `v0.1.0-alpha.1`: crate version `0.1.0-alpha.1` and
PyPI version `0.1.0a1`, both named `tuya-interpreter`. The workflow checks that the tag matches
Cargo and builds Python distributions with the corresponding version.

A pushed version tag publishes to **crates.io and PyPI** after tests, all wheels, the source
archive and crate packaging pass. PR/manual runs build artifacts without publishing.
See [release setup](docs/releases.md) for PyPI Trusted Publishing, the crates.io first-registration
token, subsequent OIDC publishing and release commands.
