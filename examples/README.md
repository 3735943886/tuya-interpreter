# Tuya interpreter examples

Start with the descriptor-driven example to see why an interpreter is useful.
The smaller `toggle_cli.py` and Rust CLI also demonstrate the transport boundary.
All examples use the real interpreter; TinyTuya or rustuya supplies transport.

```text
examples/
  python/   Python CLIs using TinyTuya
  rust/     Rust CLI using rustuya (standalone Cargo project)
  devices/  Shared fictional device JSON and example rule layer
```

## Same application, different devices

Two fictional device profiles have different DP layouts and temperature scales:

| Profile | Switch wire value | Temperature wire value | Interpreted properties |
| --- | --- | --- | --- |
| [A](devices/device-a.json) | DP 1, Boolean | DP 2, `235`, scale 1 | `switch_1`, `temp_current = 23.5 °C` |
| [B](devices/device-b.json) | DP 101, Boolean | DP 102, `2350`, scale 2 | `switch_1`, `temp_current = 23.5 °C` |

These are illustrative virtual devices derived from the earlier anonymized schema shape,
not claims about the capabilities of the real r5c device. No real IDs or credentials are used.
A shared [rule layer](devices/rules.json) adds temperature classification to the fictional
switch category. The host explicitly merges it with the official pack; no core changes are needed.

Run the **same CLI** against either tuyamock device:

```sh
pip install -e .
pip install tinytuya tuyamock
python examples/python/device_cli.py --device a
# Exit with quit, then switch metadata and mock endpoint:
python examples/python/device_cli.py --device b
```

Use `--memory` for an in-memory transport without TinyTuya/tuyamock dependencies.
The [CLI](python/device_cli.py) builds its property table and available commands from the descriptor.
There are no feature names, DP numbers, temperature scales or units hardcoded in its UI.
The host's setup uses the sample metadata to initialize each mock's raw state.

```text
device> props
available            binary    read only    true
switch_1             binary    read/write   false
temp_current         number    read only    23.5 °C
Available writes (from descriptor):
  set switch_1 <JSON value>
  toggle switch_1
device> set switch_1 true
```

For A, the interpreter generates `{"1": true}`; for B, `{"101": true}`.
The user command is identical. Raw wire maps are printed next to interpreted values so the
DP mapping and scale conversion are visible.

Try `set temp_current 30`: the interpreter rejects the read-only property and sends no DPS.
`set switch_1 "invalid"` demonstrates type validation. `toggle switch_1`, `read`, `describe`
and `quit` work for either profile. The application delegates validation and conversion to
the interpreter rather than reproducing Tuya schema logic.

This example polls on demand. It does not run background timers, persist settings or subscribe
to unsolicited pushes; those remain host responsibilities in a full application.

## Anonymized device JSON

[toggle-switch.json](devices/toggle-switch.json) is a fictional single-channel toggle switch based on
the shape of a real tuyawizard export inspected on r5c. It preserves the `kg` category,
Boolean `switch_1` definition and DP 1 mapping. Other features were removed; names and IDs
were replaced. Local keys, IP addresses, owner/account IDs, coordinates, UUIDs, timestamps
and product-specific metadata are omitted. It cannot authenticate to a real device.

## Protocol-level virtual device with tuyamock

Install the example dependencies, then run:

```sh
pip install tinytuya tuyamock
python examples/python/toggle_cli.py --mock
```

This starts a [tuyamock](https://github.com/3735943886/tuyamock) switch on `127.0.0.1`
at an OS-assigned TCP port, and connects with a real TinyTuya client using Tuya protocol 3.5.
It uses the anonymized schema and a public test key. Commands pass through the real interpreter,
then through TinyTuya's encrypted TCP transport, and change the mock device's actual DPS state.
The mock stops when the CLI exits. No physical device or private credentials are needed.

For a separate server/client demonstration, use two terminals:

```sh
# Terminal 1: virtual device on loopback, without discovery broadcasts.
python -m tuyamock --host 127.0.0.1 --port 16668 --version 3.5 \
  --local-key 0123456789abcdef --gw-id example-toggle-switch --dps '{"1": false}'
```

```sh
# Terminal 2: the same CLI's normal TinyTuya connection path.
TUYA_LOCAL_KEY=0123456789abcdef python examples/python/toggle_cli.py \
  --file examples/devices/toggle-switch.json --ip 127.0.0.1 --port 16668 --version 3.5
```

Stop the separate mock with Ctrl-C. The key above is a deliberately public mock credential.

## Offline interactive demo

From a source checkout after `pip install -e .`:

```sh
python examples/python/toggle_cli.py --demo
```

The transport is simulated in memory; the interpreter and official rules are real.
Enter `on`, `off`, `toggle`, `read`, `describe` or `quit`.

```text
tuya> on
Sending DPS: {"1": true}
switch_1 = true
switch_1: true
tuya> toggle
switch_1: true
Sending DPS: {"1": false}
switch_1 = false
switch_1: false
```

## Connect a real device through TinyTuya

Install the interpreter and optional transport library:

```sh
pip install tuya-interpreter tinytuya
```

Use a private tuyawizard device list or a single real device object. The record needs the
schema and `local_strategy` mapping described in the main README, plus its real device ID.
The CLI uses `ip`, `version` and `local_key` from the selected record. IP and protocol version
can be overridden; the local key can also be provided through `TUYA_LOCAL_KEY`, or entered
at a hidden prompt when absent from the record. Keep real credentials outside the repository.

```sh
python examples/python/toggle_cli.py \
  --file /path/to/private/tuyadevices.json \
  --device-id YOUR_DEVICE_ID
```

If the export lacks the LAN address or protocol version:

```sh
python examples/python/toggle_cli.py \
  --file /path/to/private/tuyadevices.json \
  --device-id YOUR_DEVICE_ID \
  --ip 192.0.2.10 --version 3.3
```

Replace the example address and version with your device's actual values. For a single-device
JSON file, `--device-id` can be omitted. `--property switch_2` selects a different writable
Boolean property. Gateway subdevices are outside this example's scope.

The loop is:

1. TinyTuya `status()` returns a response containing DP ID–value pairs.
2. The host unwraps `dps` (including nested `data.dps`) and supplies `Message("state", dps)`.
3. The interpreter emits ildevice values such as `switch_1 = true`.
4. An interactive `on` becomes `Command("switch_1", True)`.
5. The interpreter validates and emits `SendMessage("set", {"dps": {"1": true}})`.
6. TinyTuya `set_multiple_values()` sends those generated DPS; a fresh status read checks the result.

TinyTuya supplies transport only. All feature interpretation and command-to-DP mapping remain
in the interpreter. See the [TinyTuya API implementation](https://github.com/jasonacox/tinytuya/blob/master/tinytuya/core/Device.py).

## Rust CLI using rustuya

[`rust`](rust/src/main.rs) implements the same interactive flow using the native
Rust interpreter API and `rustuya::tokio` for transport. It is a standalone Cargo project;
transport dependencies are not added to the interpreter core. Requires Rust 1.89 or newer.

Start the virtual device in one terminal:

```sh
pip install tuyamock
python -m tuyamock --host 127.0.0.1 --port 16668 --version 3.5 \
  --local-key 0123456789abcdef --gw-id example-toggle-switch --dps '{"1": false}'
```

In a second terminal, from this repository:

```sh
cargo run --locked --manifest-path examples/rust/Cargo.toml
```

Defaults select the included fictional schema, loopback port 16668, protocol 3.5 and the
public mock key. Enter `on`, `off`, `toggle`, `read`, `describe` or `quit`. Stop the separate
mock with Ctrl-C when finished.

For a real direct LAN switch, use your private device metadata:

```sh
cargo run --locked --manifest-path examples/rust/Cargo.toml -- \
  --file /path/to/private/tuyadevices.json --device-id YOUR_DEVICE_ID
```

With `--file`, the CLI reads `ip`, `version` and `local_key` from the selected record and
uses port 6668 by default. Override with `--ip`, `--version`, `--port`, or `TUYA_LOCAL_KEY`.
Use `--property` for another writable Boolean property and `--rules-dir` for an explicit
rule directory. Credentials are never printed or passed as command-line arguments.

The host reads and sorts official JSON rule layers, then uses `merge_rules` for validation
and `Driver` for interpretation. `rustuya` queues commands and delivers replies on a listener;
the example subscribes before status queries, unwraps received DPS and supplies `Channel::State`.
Only interpreter-generated `SendMessage` effects are sent via `rustuya.set_dps()`.
Before continuing, the CLI waits for a report matching the generated DPS; firmware that
acknowledges without reporting state is queried after a bounded wait.

Like the Python example, this is an on-demand Boolean-switch CLI without a background push or
timer loop. A transport failure exits and closes the device. It does not emulate a Tuya device
itself; the separate tuyamock server provides that endpoint.

### Rust: use the same two device profiles

The Rust CLI also derives its property menu from the descriptor and accepts generic
`set <property> <JSON value>` and `toggle <property>` commands. Start profile A's wire state:

```sh
python -m tuyamock --host 127.0.0.1 --port 16668 --version 3.5 \
  --local-key 0123456789abcdef --gw-id example-device-a --dps '{"1": false, "2": 235}'
```

Then connect with its schema and the shared temperature rule layer:

```sh
TUYA_LOCAL_KEY=0123456789abcdef cargo run --locked \
  --manifest-path examples/rust/Cargo.toml -- \
  --file examples/devices/device-a.json --ip 127.0.0.1 --port 16668 --version 3.5 \
  --rules-layer examples/devices/rules.json
```

For B, restart tuyamock with `--gw-id example-device-b --dps '{"101": false, "102": 2350}'`
and select `--file examples/devices/device-b.json` in the same client command.
Both display `temp_current = 23.5 °C` and accept `set switch_1 true`.
