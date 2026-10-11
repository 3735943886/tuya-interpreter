# tuya2ildevice

The one place that interprets Tuya. A Sans-IO Rust engine with a thin Python wrapper: takes a Tuya device as [rustuya](https://github.com/3735943886/rustuya) /
[rustuya-bridge](https://github.com/3735943886/rustuya-bridge) know it and produces an
[ildevice](https://github.com/3735943886/ildevice): descriptor, live values and events from dps packets, and dps for
ildevice commands. MQTT, TCP and other transports belong to the host. The engine opens no sockets and reads no clock. Other projects
([rustuya-local](https://github.com/3735943886/rustuya-local), ildevice hosts,
[rustuya-manager](https://github.com/3735943886/rustuya-manager)) import this instead of carrying their own Tuya knowledge.

```
pip install tuya2ildevice              # no dependencies;  [test] for the tests
```

```python
from tuya2ildevice import TuyaDriver, Connected, Message, Command, descriptor_of

# device and rules are decoded data supplied by the host.

descriptor_of(device, rules=rules)                 # just the ildevice descriptor of a tuyadevices.json entry
d = TuyaDriver(device, rules=rules)                # the state machine (il.md section 8)
d.handle(now, Connected())            # -> [Descriptor]
d.handle(now, Message("active", {"1": True}))   # -> [Value(...), Event(...)]
d.handle(now, Command("switch_1", "off"))    # -> [SendMessage("set", {"dps": {"1": False}})]  or  [Reject(...)]
```

## Portable rules and Rust engine

The Python `TuyaDriver` delegates classification, descriptor assembly, commands, value conversion,
quirks, delta accumulation and cover state to Rust through a JSON C ABI. The wheel includes
the native library; building from source requires Cargo (Rust 1.88 or newer). Another language binds the same two C
functions (see [the engine documentation](docs/rust-engine.md)).

[`rules/00-default.json`](rules/00-default.json) is the canonical rule pack. Every device interpretation is data:
platform/category tables, and per platform an entity spec — its roles and their DP candidates, how a plan is built,
the state it reads, the actions it writes and the properties it lays out — plus units, mappings, feature bits and
quirks. The engine supplies only the interpreter, DP types and codecs, so changing these rules (even adding a
platform) does not require rebuilding Rust. The host supplies `rules` explicitly, optionally composing decoded layers with `merge_rules`.
`TUYA_ENGINE_LIBRARY` selects another native library. Packs are cached by content for the process lifetime.

A `converters.declarative` override adds computed values, write commands and state/timer programs
using JSON expressions; there are no code plugins. See [the rule language, ABI and build instructions](docs/rust-engine.md).

## Packets and commands

- Numeric properties use the native unit of their values. For example, a plug reporting `500 mA` publishes
  `500` with unit `mA`; a suggested display unit such as `A` does not change the wire unit or magnitude.
- `Message(channel, json)`: `active` (device push: fires events, accumulates `add_ele`-style deltas) or `passive` /
  `state` (readback, snapshot: values only; with `delta.accept_passive`, a passive report adds its delta too, see
  [Delta settings](#delta-settings)). `json` is a flat `{dp: value}` map (the host has already decoded whatever
  the bridge's own wire payload looked like — see the host boundary below).
- `Command(prop, value)` is checked as il.md section 5 says before anything is sent; failures come back as `Reject`.
  Values convert as in Home Assistant core (brightness goes through 0..255), so a written value can differ slightly
  from what is read back. A cover's position is the device's own number, never mirrored (core mirrors it); a device
  that counts the other way is set so by its cover settings (below).
- A `passive` report (a live one: the host drops retained ones) that changes a value is the device's own push, like
  `active`: converters see it as live; one that changes nothing is a readback. Events fire only from `active`. `state`, the bridge's
  merged view of what `active` / `passive` carried, is a snapshot: the first one after a connect sets everything, a
  later one only what it alone brings, and never as a push. An increment (`report_type: sum`) is added only from
  `active`.
- Covers of class garage/gate are read only unless `TuyaDriver(..., allow_hazardous=True)` (il.md S-1).
- A dp no platform table claims gets no property, as in Home Assistant core; `TuyaDriver(..., expose_unused=True)` gives
  each one a property chosen by its Tuya type. Integer, Enum and Boolean are writable only if the dp is
  in `function`; String, Raw, Json and Bitmap are read-only. `config` if writable, else `diagnostic`.
- Assembled: switch, button, select, number, sensor, binary_sensor, event, light, cover, fan, siren, valve,
  humidifier, climate, alarm (kind `alarm`), vacuum. Not assembled: camera (a stream is outside the IL; `driver.unsupported` lists it).
  A doorbell `alarm_message` event loses its message text (an ildevice event has only a kind).

## User overrides

Tuya devices are fragmented: a cloud schema can give a dp a non-standard code, leave a dp out, speak other words
(`on`/`off`/`pause` instead of `open`/`close`/`stop`) or run a position the other way. Overrides fix that in Tuya's own
terms, before classification, so the fixed device goes through the same tables as any other and every IL consumer
(il-ha, a discovery publisher, ...) sees the fix.

`TuyaDriver(device, rules=rules, overrides=mapping)`; `mapping` is `{product_id or device id: block}`,
already loaded (`merge_all([...])` merges several, later wins). File discovery and loading belong to the host.
The block format is described below; Rust validates and applies it:

```json
{ "<product_id>": {
    "dp":     {"104": {"code": "percent_control", "type": "Integer", "values": {"min": 0, "max": 100, "scale": 0, "step": 1}},
               "101": {"code": "control"}},
    "remove": ["percent_state"],
    "category": "cl",
    "remap":  {"control": {"alias": {"on": "open", "off": "close", "pause": "stop"}}},
    "cover":  {"invert_position": true, "settle": 5},
    "delta":  {"accept_passive": true},
    "props":  {"countdown_1": {"label": "Timer", "category": "config", "rw": false}},
    "device": {"model": "Sliding Window Opener"},
    "converters": {"declarative": {"props": {"glow": {"type": "binary"}}, "update": []}},
    "expose_unused": true } }
```

| key | fixes |
|---|---|
| `dp` with a `type` | a dp the schema lacks or describes wrongly (defined like a quirk's `DefineDp`) |
| `dp` with only a `code` | a dp with a non-standard code: renamed, its type, range and value strategy kept |
| `remove` | a dp that should not be used (the tables then fall back, e.g. position read from the target) |
| `category` | the Tuya category the tables are chosen by |
| `remap.<code>.alias` | other words: device value -> standard value, both ways (an Enum's range is translated too) |
| `remap.<code>.invert` | the other direction: a Boolean negated, an Integer mirrored in its range (not for a cover's position or command: use `cover`) |
| `cover` | a cover's direction and motion (below) |
| `props`, `device` | the finished descriptor: label, class, category, unit, role, read only, hidden; device kind/class/label/model |
| `props.<name>` with a `src` | a property defined from a dp (below), replacing one of that name |
| `converters` | `declarative` (JSON programs, below) and `cover_motion` |
| `expose_unused` | this device only: every dp no table claims gets a property of its own |
| `auto: false` | no property from the tables at all: only what `props` defines and the converters give |

A property defined from a dp builds what the tables cannot, such as a cover from a position dp alone (a window opener
whose category has no cover table):

```json
{ "<product_id>": {
    "dp":     {"104": {"code": "percent_control", "type": "Integer", "mode": "RW",
                       "values": {"unit": "%", "min": 0, "max": 100, "scale": 0, "step": 1}}},
    "device": {"kind": "cover", "class": "window"},
    "props":  {"position": {"src": "percent_control", "role": "position"},
               "open":     {"src": "percent_control", "type": "trigger", "role": "open", "send": 100},
               "close":    {"src": "percent_control", "type": "trigger", "role": "close", "send": 0},
               "battery":  {"src": "residual_electricity", "role": "battery", "category": "diagnostic"}} } }
```

Its `type` follows the dp's (Boolean `binary`, Integer `number`, Enum `select`, others `text`) unless given; `min`,
`max`, `step`, `unit` and `options` come from the dp unless given; it is writable when the dp is in `function` (`rw:
false` makes it read only); a `trigger` writes its `send` value. Values go through `remap` like any other.

Unknown keys raise `OverrideError`. To reload rules or overrides, the host creates a replacement driver,
optionally calls `new.carry(old)` to preserve compatible delta totals, and feeds its current snapshot.
The host handles descriptor replacement and removed properties for its transport.

### Cover settings

Which way a cover's position and commands run, and whether its motion is inferred, are the cover's settings, not
`remap`: a curtain whose direction is wrong is fixed from its own configuration switches. `TuyaDriver(device, rules=rules, device_settings=True)` (a host with somewhere to keep them) offers each switch only where it changes something:

| switch (property) | label | offered when the cover has | does |
|---|---|---|---|
| `cover_invert_position` | Invert current position | a reported position (`current_position`) | reads it the other way, 0..100; with one dp for position and target, writes it the other way too |
| `cover_invert_set_position` | Invert target position | a target (`set_position`) that is another dp | writes the target, and reads it back, the other way |
| `cover_invert_control` | Invert control | a command dp (`control`) and no target | swaps open and close (`FZ` / `ZZ` on the special command; a Boolean negated) |
| `cover_position_from_target` | Use target as current position | two distinct position sources before selection | ON uses the target DP for display; OFF restores the original feedback DP; the switch stays available |
| `cover_state_source` | Motion state source | a position or control DP, with no user converter owning this cover state | select `control` (control reports), `inferred` (position-based motion inference), or `none` (no motion generation); `control` requires a control DP |
| `cover_invert_reported_motion` | Invert reported motion | control report mode | swaps only the reported opening/closing meaning, leaving stop unchanged |

A cover with a target opens and closes by writing it (100 / 0), and its command dp only stops it: it has no *Invert
control*. The switches are `binary`, the state source is `select`; all are `rw`, `category: config`, and show the settings in effect; a hazardous cover's
(garage, gate, ...) are read only unless `allow_hazardous`. A later cover of the device (`control_2`, ...) has its own,
named by its group (`control_2_cover_invert_position`).

Writing a switch sends nothing to the device: the driver answers with the value and `SettingsChanged(block)`, with the device's whole settings block (e.g. `{"cover": {"invert_position":
true}}`) for the host to keep (rustuya-local keeps it in `zz_settings.json` in its rules directory and reloads).

The same settings are a `cover` block in any override file, where a product's block is the starting point for its
devices and a device's (the switches write the device's) wins:

| key | default | |
|---|---|---|
| `invert_position` | `false` | as the switch |
| `invert_set_position` | `false` | as the switch |
| `invert_control` | `false` | as the switch |
| `infer_motion` | `device_settings` | legacy Boolean, used only when `state_source` is absent; true maps to `inferred`, false to `none` |
| `state_source` | legacy `infer_motion` | `control`, `inferred`, or `none`; an explicit value takes precedence over the legacy Boolean |
| `invert_reported_motion` | `false` | invert only direct control reports |
| `settle` | `0` | seconds without a position report after which a move counts as stopped (0: never) |
| `invert_tilt` | `false` | the tilt the other way |
| `position_from_target` | `false` | the reported position dp is left out and the target read as the position |

A later cover's keys go in an object under its group's name: `{"cover": {"invert_position": true, "control_2":
{"invert_position": true}}}`. An inversion is laid over the dp's `remap` (a `remap.invert` still there cancels it), and
the swap is composed with the dp's `remap.alias`, so a device that says `on` / `off` / `pause` keeps its alias.
`remap.<code>.invert` on a cover's position or command dp and `converters.cover_motion` still work, and log that they
are deprecated: move them to `cover`.

`control` maps live open/close/stop reports directly to opening/closing/stopped, even without
position feedback. Boolean true/false and FZ/ZZ/STOP use the same control vocabulary as inference.
The order on input is `remap.alias`, existing `invert_control` (where applicable), then
`invert_reported_motion`. The last setting swaps only opening/closing from control reports;
it never changes sent commands, positions, inferred direction, or endpoint states. Live stop
is always stopped. Position updates alone do not settle a directly reported movement.

### Delta settings

A delta dp (`report_type: sum`, a plug's `add_ele`) reports the energy used since its last report, not a meter
reading: its property is the running total tuya2ildevice adds up (`series: counter`), so a repeated increment (`5`,
then `5` again) adds twice. The total is published in the DP’s native unit with its `scale` applied: for
`scale: 3` and unit `kWh`, a raw increment of `5` adds `0.005 kWh`. By default only an `active` push adds: a passive report is a readback of what the device
already pushed. A device that never pushes it, and reports it passively only, needs passive reports counted too:

| switch (property) | label | offered when the device has | does |
|---|---|---|---|
| `delta_accept_passive` | Count passive reports | a delta dp | a live passive report that carries the dp adds it as well, each one, even one that repeats the last value |

It is the `delta` block's `accept_passive` (default `false`) in an override file, and saves as the cover switches do
(`{"delta": {"accept_passive": true}}`). One report seen twice with the same Tuya `t` adds once; a readback the host
asks for (a `get`, on a reconnect) carries no new increment but adds once more, so turn it on only for a device that
does not push. A reload (an override or a setting changed) keeps the totals.

Retained snapshots/readbacks (`state`, and non-live passive reports) never start motion: their
control may be the last command. A relevant snapshot reports only the position's resting state
(closed at 0, open at 100, stopped between), or no state without position. Disconnect clears
motion; reconnect/reload seeds only resting state and waits for a live report. Duplicate live
reports do not republish unchanged state. Direct report mode has no settle timer.

`inferred` preserves CoverMotion: a control word can start motion; subsequent position feedback
corrects its direction, while an independent target can be compared with current position.
A shared current/target DP is never compared against itself. Without independent feedback,
command direction and arrival cannot be verified; a target-only update does not prove movement.
`none` generates only resting state from position and never movement. Legacy `infer_motion:false`
keeps its previous behavior of omitting the separate cover_state property (position remains
available); explicitly selecting `none` also provides the resting state property.

The old Infer motion switch is replaced by the state source select, not exposed alongside it.
Existing JSON and saved `zz_settings.json` Booleans remain readable; the select shows their mapped
mode and subsequent selections save `state_source`. Explicit state_source wins even if an older
infer_motion entry remains. A user converter owning a cover's `cover_state` retains ownership:
that cover gets neither the built-in state producer nor its source/direction controls. Other
covers remain independent, including when legacy `converters.cover_motion` owns the first one.

Settings remain saved while their controls are hidden: target inversion is dormant while current
and target share a DP, and reported motion inversion is dormant outside control mode. Switching
back restores them. Current-position inversion follows the selected display source; when that
source is also the target, it inverts both reads and writes. Property IDs and cover group IDs
stay stable through source changes. Host reload cancels old timers and clears the old cover state
before publishing a fresh snapshot. A bare sans-IO caller must save SettingsChanged and reload,
just as for the existing inversion switches.

For **f6jujmx0is5td50x**, remove its old `remove`, `remap` and `converters.cover_motion` JSON
entries, then configure the device (no product ID is hardcoded):

1. **Use target as current position**: ON.
2. **Invert current position**: ON.
3. **Motion state source**: `control`.
4. **Invert reported motion**: ON.

This selects percent_control (DP 2 in the referenced schema), inverts its reads and writes, and
reverses only the reported motion. The DP name does not establish that it reports physical position:
if firmware reports only a target, the displayed position is only that target; intermediate travel,
arrival and physical resting position cannot be determined from it.

The previous rustuya-homeassistant JSON selected DP 2, inverted position reads and target writes,
and mapped control directly; it was not enrolled in the Python converter because it had no
`state_stream: "derived"`. Its MQTT open/close commands used control open/close. This package
continues to send target 100/0 when set_position exists (after position inversion), so its command
path is **different from that earlier implementation**. Report inversion does not change that path.

Model-specific fixes use the `overrides` section of the same rules JSON, either in the shipped defaults or in a user file.

## Converters

A `converters.declarative` block derives properties from several dps with state and timers, and writes them back,
as JSON programs in the rule language (see [the engine documentation](docs/rust-engine.md#json-converters)). Its
properties may replace the tables' ones of the same name; one with `rw: true` (or a trigger) needs a `write` program.
There are no code plugins: every converter is data and runs in the engine, so any binding drives it.

The cover settings give a cover its `cover_state` role (open / closed / opening / closing / stopped) through the
engine's motion tracker (`state_source: "inferred"`, or legacy `infer_motion`), on the dps the cover actually has,
after its inversions. A command word only says a move started, the position reports after it decide which way; a
snapshot never starts motion. Naming it in a block (`{"converters": {"cover_motion": {...}}}`) still works,
deprecated. Timers come out as `SetTimer`; the host calls back with
`driver.handle(now, Timer(name))`.

## Rule files

The wheel ships [`rules/00-default.json`](rules/00-default.json). Fixes published between releases are the
[override pack](pack/README.md) (`pack/` on `master`): rule files a host fetches into its rule directory.
User files contain partial rule objects, for example:

```json
{
  "overrides": {
    "my_product": {
      "cover": {"invert_position": true}
    }
  }
}
```

The host reads rule files and chooses their order. Pass decoded layers to `merge_rules(layers, base=rules)`;
objects merge recursively, arrays and scalars replace earlier values, `{"$delete": true}` removes a member,
`{"$append": [...]}` extends an array, and `null` is literal. An invalid final rule set is rejected.
There is no automatic rule loading or `TUYA_ENGINE_RULES` support.

```python
from tuya2ildevice import merge_rules

custom = merge_rules(user_layers, base=rules)["rules"]
driver = TuyaDriver(device, rules=custom)
```

## Host boundary and migration

A host keeps one `TuyaDriver` per device and supplies decoded inputs and an explicit `now`.
The driver returns ildevice data, independent of MQTT, TCP or in-process consumption.

| Direction | Data |
|---|---|
| Tuya → driver | `Connected`, `Disconnected`, `Message(channel, flat_dps)` |
| Driver → IL consumer | `Descriptor`, `Value`, `Absent`, `Event`, `Reject` |
| IL consumer → driver | `Command(prop, value)` with a decoded value |
| Driver → Tuya | `SendMessage("set", {"dps": ...})` |
| Driver → host | `SetTimer`, `CancelTimer`, `SettingsChanged` |

The host executes timer requests and returns `Timer(name)`, persists settings, manages devices and snapshots,
and implements its transport's framing, subscriptions, presence and retained-message policy.
The driver never requests bridge registration or automatic `get` operations.

This is a breaking API change: `Hub`, `IlTopics`, MQTT actions, `op: hub`, `op: il`, and rule-file loaders have
been removed. Replace `Hub` calls with per-device `handle` calls and map their data in your host adapter.
Python construction requires `rules=...`. JavaScript uses `Driver.handle(now, input)` with the same JSON tags.
Native library loading remains binding infrastructure; neither binding reads rule files or a clock.
Creation diagnostics are returned in `driver.warnings`; the host decides how to display or log them.

## Layout

```
rules/         canonical language-neutral rule pack and schema
rust/engine/   rule interpreter, Tuya primitives, explicit driver state and C ABI
src/tuya2ildevice/
  native.py    ctypes transport, library loading and rule-pack cache
  driver.py    TuyaDriver: one device's state JSON through the engine
  io.py        inputs and outputs as data
scripts/       generators (HA core tables, quirks, host units, Tuya categories) and build_rules.py, which folds
               their output into the rule pack
scripts/rust/  export frozen Python-reference vectors and check the native ABI
bindings/js/   a JavaScript binding over the WebAssembly build, with a smoke test
tests/reference/  frozen pre-migration Python oracle; never used in production
```

The engine reproduces Home Assistant core's `tuya` integration exactly, including its quirks, and the golden tests
pin it: `tests/golden/golden.json` is core's own entity snapshots for every fixture. For a new HA core / tuya-device-handlers release, review its changes against the canonical JSON rules
and golden tests. The frozen Python oracle is a baseline, not the source of future runtime rules.

One deliberate exception: where Tuya's own category list (`scripts/data/tuya_categories.json`, generated by
`scripts/gen_tuya_categories.py` from Tuya's standard instruction set page) disagrees with core, Tuya's list wins.
The decisions are data too (`scripts/data/tuya_standard_rules.json`, applied by `scripts/standard.py` when
`scripts/build_rules.py` builds the pack's platform tables): a `kg` ("Switch") category's
switches are `switch`, not core's `outlet` plug icon; `cz` ("Socket") and `pc` ("Power strip") stay `outlet`. The golden
tests apply the same file to core's expectations, and each such difference is a tagged `deviate` row in
`docs/engine-spec.md` (P-28).

A second one: a cover's position is not mirrored. The bridge shows the device's number and so does the IL; an
installation inverts a device that counts the other way itself (its cover settings). The golden tests mirror core's
expected position and position writes where core mirrored them (`core_reverses` in `tests/test_golden.py`).

## Tests

```
cargo test --locked --manifest-path rust/Cargo.toml
pip install -e .[test] && python -m pytest
```

`tests/test_adapter.py` compares the raw-dps adapter with the Tuya SDK and is skipped unless `tuya-device-sharing-sdk==0.2.15`
is installed. The spec repository (`../ildevice`, or `$ILDEVICE`) supplies the schema and the language-neutral vectors
(commands, wire values, topics) read directly from its checkout; those tests are skipped if it is not there. `tests/chain/`
compares every Home Assistant core tuya fixture with core's entity snapshots through an IL host's HA-free planner
(needs that host's `ildevice.core` on `sys.path`; not collected without it).

## Release builds

[Release workflow](.github/workflows/release.yml) builds and tests 13 platform wheels:

| Platform | Architectures |
| --- | --- |
| Linux manylinux (glibc) | ARMv7, ARM64, x86, x64 |
| Linux musllinux (musl) | ARMv7, ARM64, x86, x64 |
| macOS | ARM64, x64 |
| Windows | ARM64, x86, x64 |

Push a `v*.*.*` version tag to publish the wheels and source archive to PyPI after
all jobs pass. PRs affecting release inputs and manual workflow runs build artifacts
without publishing. Configure PyPI Trusted Publishing for `release.yml` and the
`pypi` environment. Users installing a supported wheel need no Rust toolchain.
