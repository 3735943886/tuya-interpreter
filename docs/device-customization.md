# Device customization

Product and device overrides, declarative converters, cover settings and delta settings.
See the [README](../README.md) for the input device JSON and host contract.

## User overrides

Tuya devices are fragmented: a cloud schema can give a dp a non-standard code, leave a dp out, speak other words
(`on`/`off`/`pause` instead of `open`/`close`/`stop`) or run a position the other way. Overrides fix that in Tuya's own
terms, before classification, so the fixed device goes through the same tables as any other and every ildevice consumer sees the fix.

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
| `converters` | `declarative` (JSON programs, below) |
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
true}}`) for the host to persist and use when constructing a replacement driver.

The same settings are a `cover` block in any override file, where a product's block is the starting point for its
devices and a device's (the switches write the device's) wins:

| key | default | |
|---|---|---|
| `invert_position` | `false` | as the switch |
| `invert_set_position` | `false` | as the switch |
| `invert_control` | `false` | as the switch |
| `state_source` | `inferred` with `device_settings`, else unset | `control`, `inferred` or `none`; unset gives the cover no `cover_state` |
| `invert_reported_motion` | `false` | invert only direct control reports |
| `settle` | `0` | seconds without a position report after which a move counts as stopped (0: never) |
| `invert_tilt` | `false` | the tilt the other way |
| `position_from_target` | `false` | the reported position dp is left out and the target read as the position |

A later cover's keys go in an object under its group's name: `{"cover": {"invert_position": true, "control_2":
{"invert_position": true}}}`. An inversion is laid over the dp's `remap` (a `remap.invert` there cancels it), and
the swap is composed with the dp's `remap.alias`, so a device that says `on` / `off` / `pause` keeps its alias.

The state source decides the cover's `cover_state` (open / closed / opening / closing / stopped):

- `control` maps live open/close/stop reports directly to opening/closing/stopped, even without position feedback.
  Boolean true/false and FZ/ZZ/STOP use the same control vocabulary as inference. The order on input is
  `remap.alias`, `invert_control` (where applicable), then `invert_reported_motion`, which swaps only
  opening/closing from control reports: it never changes sent commands, positions, inferred direction or endpoint
  states. Live stop is always stopped. Position updates alone do not settle a directly reported movement, and this
  mode has no settle timer.
- `inferred`: a control word can start motion; subsequent position feedback corrects its direction, while an
  independent target can be compared with the current position. A shared current/target DP is never compared
  against itself. Without independent feedback, command direction and arrival cannot be verified; a target-only
  update does not prove movement.
- `none` generates only the resting state from the position, never movement.

Retained snapshots/readbacks (`state`, and non-live passive reports) never start motion: their control may be the
last command. A relevant snapshot reports only the position's resting state (closed at 0, open at 100, stopped
between), or no state without position. Disconnect clears motion; reconnect/reload seeds only the resting state and
waits for a live report. Duplicate live reports do not republish unchanged state.

A declarative converter that defines a cover's `cover_state` owns it: that cover gets neither the built-in state
producer nor its source/direction switches. Other covers of the device are independent.

Settings remain saved while their switches are hidden: target inversion is dormant while current and target share a
DP, and reported motion inversion is dormant outside control mode; switching back restores them. Current-position
inversion follows the selected display source; when that source is also the target, it inverts both reads and
writes. Property IDs and cover group IDs stay stable through source changes. A host reload cancels old timers and
clears the old cover state before publishing a fresh snapshot; a bare sans-IO caller saves `SettingsChanged` and
reloads, as for any switch.

### Delta settings

A delta dp (`report_type: sum`, a plug's `add_ele`) reports the energy used since its last report, not a meter
reading: its property is the running total tuya_interpreter adds up (`series: counter`), so a repeated increment (`5`,
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

Model-specific fixes use the `overrides` section of the same rules JSON, either in the shipped defaults or in a user file.

## Converters

A `converters.declarative` block derives properties from several dps with state and timers, and writes them back,
as JSON programs in the rule language (see [the engine documentation](rust-engine.md#json-converters)). Its
properties may replace the tables' ones of the same name; one with `rw: true` (or a trigger) needs a `write` program.
There are no code plugins: every converter is data and runs in the engine, so any binding drives it.

A cover's own `cover_state` comes from its [cover settings](#cover-settings) (`state_source`), not from a
converter. Timers come out as `SetTimer`; the host calls back with `driver.handle(now, Timer(name))`.

