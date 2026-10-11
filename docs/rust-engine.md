# Portable Tuya rule engine

The complete production `TuyaDriver` executes in Rust. Python transports JSON
through a two-function C ABI and dispatches explicitly registered Python plugins.
There is no Python conversion fallback and no Python interpreter in the Rust library.
The original implementation is frozen under `tests/reference/` for oracle tooling.

## The contract

1. **Device interpretation is JSON.** `rules/00-default.json` holds every piece of device and platform knowledge:
   category tables, entity specs, DP candidates, units, maps, feature bits, layouts and quirks.
2. **The Rust core is shared and does no I/O.** It is an interpreter plus primitives that do not depend on any
   device: DP types (validate, scale, invert), Tuya codecs (`value_convert` strategies, electricity, HSV), the
   expression language, the cover motion tracker, the driver state machine and the il-mqtt wire mapping. It reads no
   files, network or clock: hosts read rule files (and fetch anything else) and pass the decoded JSON.
3. **Bindings are thin.** A binding moves JSON across two C functions; there are no plugin callbacks. The Python
   package is such a binding. A JavaScript or other binding needs the same two functions (or
   the WebAssembly build, below) and no Tuya knowledge.
4. **Spec changes are rule layers.** A new DP code for a role, a new category, a new enum word, a new role, action,
   property or whole platform is a JSON file under `TUYA_ENGINE_RULES` (see `tests/test_specs.py`). Only a new
   *primitive* (a codec byte layout, an expression operation) needs an engine release.

## The rule pack

| Field | Content |
| --- | --- |
| `platforms` | per platform, in classification order: `table` (HA core's name) and `categories` (category -> descriptions), generated |
| `entities` | per platform, the entity spec (below) |
| `unused` | `role` and `variants` for DPs no entity claimed (`expose_unused`) |
| `layout_common` | definition members every property gets (e.g. `category`) |
| `identity_defaults` | identity members a description lacks |
| `templates` | named fragments; `{"$use": name}` stands for one anywhere a spec field or statement is expected |
| `functions` | named expressions with `params`, called with `{"op": "call", "fn": name, "args": [...]}` |
| `quirks` | tuya-device-handlers' schema patches by product id, generated |
| `units`, `host_units` | unit aliases and HA's allowed units, generated |
| `maps`, `features`, `constants`, `wind`, `electricity`, `meter_alarms` | lookup data the specs and codecs use |
| `cover_defaults`, `cover_switches`, `motion_defaults`, `motion_property` | cover settings and the motion tracker's defaults |
| `strategies` | the codec names a device's `local_strategy` may use |
| `overrides` | product/device fixes (README) |

`rules/schema.json` validates the whole pack. `scripts/build_rules.py` folds the generators' output (HA core tables,
quirks, host units) into it, leaving the hand-authored parts as they are.

## Entity specs

Classification walks `platforms` in order. For each description of the device's category, the platform's entity
spec builds at most one *plan*: resolved roles plus `identity` (HA's entity attributes) and `config` (what reads and
writes need).

| Spec field | Meaning |
| --- | --- |
| `params` | the spec's own data, visible as `entity.params`; DP candidates live in `params.codes` so a rule layer can patch them |
| `variants` | alternatives, tried in order; the first whose steps complete builds the plan |
| `finish` | steps run after the chosen variant (a layer can `$append` roles here) |
| `role_order` | role order in the plan (and so in `depends_on`) |
| `update_all` | every DP update refreshes the entity, not only its own DPs |
| `state` | `bind` (ordered named expressions) and `fields`: the entity's state for a DP status |
| `actions` | action name -> program; `*` handles any other action |
| `layout` | property rows (below) |
| `composite`, `kind` | the entity is a device kind (`kind` defaults to the platform); later ones become groups |
| `expose` | `false`: plan only, no properties, and its DPs stay unclaimed |
| `hazard` | expression; a hazardous entity's actions are not offered |
| `cover` | role names the cover motion tracker uses: `position` (candidates), `target`, `instruction`, `tilt`, and `words` (map names of command words) |

A variant is `{"when", "steps"}` or a group `{"when", "each", "as", "variants"}` that tries its variants per item of
`each`. Steps are statements:

| Statement | Effect |
| --- | --- |
| `{"role": name, "codes": expr, "types": expr, "prefer": "function"\|"status_range", "required": bool}` | resolve the first candidate DP of the first accepted type, preferring one table; a missing required role fails the variant |
| `{"require": expr}` | the variant fails unless it holds |
| `{"set": "identity.x"\|"config.x"\|"plan.x", "value": expr}` | assign (null removes); `plan.slot_kind`, `plan.update_all`, `plan.depends_on` |
| `{"set": path, "merge": expr}` | assign every member of an object |
| `{"assert": expr, "error": expr}` | fail the classification with an error |
| `{"let": name, "value": expr}`, `{"if": expr, "then": [...], "else": [...]}`, `{"$use": template}` | shared control flow |

Expressions see `rules`, `entity`, `device`, `status`, `env`, `desc` (the description), `key`, and the plan so far:
`roles` (each role's `code`, `kind`, `spec`, `report_type`, `range` and scaled `lo`, `hi`, `step`, `scale`),
`values` (each role's current value), `identity`, `config`, `plan`.

State and actions see `roles`, `values`, `status`, `identity`, `config`, `key`, `total` (a delta accumulator), and
actions also `action`, `args` and `state`. Action programs use `{"command": {"role", "value"}}` (validated by the
role's DP type), `{"send": {"code", "value"}}` (raw) and `{"fail": message}`.

A layout row is `{"name", "grouped", "when", "definition", "read", "write", "writable", "assume"}`:

- `name` is an expression; with `grouped: false` it is not prefixed with the group's name;
- each `definition` member is an expression, and null leaves the member out;
- `read` computes the value from `state` (and `values`, `definition`); a `select` value outside its options is
  unknown and numbers are canonical (il-mqtt);
- `write` computes `{action, args}` for the entity's actions from the written `value`;
- `assume` reads with some roles' DPs set (`{"switch": true}` shows a mode while off).

## Expression language

Expressions are objects with an `op`; other JSON values are literals (`literal` quotes an object). Evaluation is
pure and bounded (1,000,000 evaluations, nesting 64; programs 10,000 statements). Device specs and declarative
converters share it.

| Group | Operations |
| --- | --- |
| access | `get` (`path`: dotted string or array whose items may be expressions; `default` when missing), `lookup` (object key or array index, `default`), `literal` |
| control | `if` (`condition`, `then`, `else`), `and`, `or`, `coalesce` (all short-circuit), `let` (`bind`, `in`), `call` (`fn`, `args`), `error` (`message`) |
| compare | `eq`, `ne` (numbers and booleans compare as numbers), `same` (exact JSON equality), `lt`, `le`, `gt`, `ge` |
| logic, types | `not`, `truthy`, `some` (the value if truthy, else null), `is_null`, `is_number`, `is_integer`, `type_of` |
| numbers | `add`, `sub`, `mul`, `div`, `min`, `max`, `floor`, `round_half_even`, `remap` (v, from lo, from hi, to lo, to hi), `num` (coerce), `bit_test`, `flags` (`table`, `when`: OR of `features` bits), `flag` |
| collections | `list`, `record` (`fields`), `object` (from pairs), `concat` (arrays; null skipped), `flatten`, `len`, `index_of`, `contains`, `without`, `sort`, `zip`, `range`, `keys`, `values`, `entries`, `codes` |
| iteration | `map`, `filter`, `find`, `any`, `all`, `count` (`args: [list]`, `as`, `do`) |
| text | `text`, `to_string`, `str_concat`, `lower`, `capitalize`, `starts_with`, `ends_with`, `trim_start`, `trim_end`, `replace`, `hex` |
| codecs | `parse_json`, `hex_ints`, `b64_utf8`, `b64_utf16be`, `electricity_b64`, `electricity_hex`, `hs_to_rgb_hex`, `rgb_hex_to_hs` |

## JSON converters

A product/device override may declare `converters.declarative`:

```json
{
  "my_product": {
    "converters": {
      "declarative": {
        "props": {
          "filter_low": {"type": "binary", "class": "problem"},
          "updates": {"type": "number"}
        },
        "initial_state": {"count": 0},
        "update": [
          {"set": ["state", "count"], "value": {"op": "add", "args": [{"op": "get", "path": ["state", "count"]}, 1]}},
          {"emit": "updates", "value": {"op": "get", "path": ["state", "count"]}},
          {"emit": "filter_low", "value": {"op": "lt", "args": [{"op": "get", "path": ["codes", "filter_life"], "default": 100}, 10]}}
        ]
      }
    }
  }
}
```

Programs are statement arrays: `set` (a path beginning with `state`), `emit` (a property; null clears it), `timer`
(`after`: seconds or null), `send` (`{code, value}`), `if`/`then`/`else` and `let`. `update`, `timer` and `reset`
programs receive their inputs; `write` maps property names to programs and receives `prop`, `value`, `codes` and
`now`. A writable/trigger property must have a write program. Context also contains `state` and the converter's
`config`; updates receive `codes`, `changed`, `active`, `now`; timer programs `name`, `codes`, `now`. Only state slots
can be assigned and a failed program commits nothing. Timer names use the `c<index>:<name>` namespace. Use `reset` to
define which state is cleared on disconnect.

## Rule layers and loading

Layers merge recursively: objects merge, other values replace, `{"$delete": true}` removes a member and
`{"$append": [...]}` extends an array. `merge_rules` takes an optional `base` and ordered `layers` (each a rule object,
or `{"source": name, "rules": object}` to name it in reports), validates the result against the bundled schema and
returns `{rules, sources, warnings}`; with `tolerant` a layer that would make the rules invalid is reported and left
out.

The engine reads no files. The Python binding's `load_rules(paths)` reads them: the bundled directory, then `paths`;
directories are non-recursive; files from all locations are applied in filename byte order (equal names keep the
location order); hidden files, `_` files and `schema.json` are excluded and `zz_settings.json` follows the same
order. `load_rules_tolerant` also reports a file that does not parse.

`TUYA_ENGINE_RULES` supplies additional file/directory paths (OS path separator) over the bundled pack and
`TUYA_ENGINE_LIBRARY` another library; environment-selected rules are cached until restart. Explicit
`load_rules(paths)` returns an independent set for `TuyaDriver(..., rules=...)` and `Hub.reload(..., rules=...)`.

## C ABI

See `rust/engine/tuya_engine.h`:

```c
char *tuya_engine_eval(const char *request_json);
void tuya_engine_free(char *response_json);
unsigned char *tuya_engine_alloc(size_t len);              /* for hosts that write into the library's memory */
void tuya_engine_dealloc(unsigned char *buffer, size_t len);
```

Input is borrowed UTF-8 with a terminating NUL, valid during the call. Output is owned by Rust and must be released
exactly once with this library's free function. Null input returns an error; freeing null is allowed. No exception
or panic crosses the ABI. Every response is `{"ok": true, "value": ...}` or `{"ok": false, "error": "..."}`; output
`reject` codes are the stable command-rejection contract.

Supply `rules` with each request, or register an immutable pack once with
`{"op": "install_rules", "rules_id": "<content id>", "rules": pack}` and pass `rules_id`. IDs cannot be reused for
different contents. The registry caches rules, never device state.

| `op` | Request fields | Result |
| --- | --- | --- |
| `create` | `device`, `options` | `{driver, warnings}` |
| `handle` | `driver`, `input`, `now` | `{driver, outputs}` |
| `carry` | `driver`, `old` | `{driver}` with compatible delta totals |
| `settings` | `driver` | settings block |
| `check` | `descriptor`, `state`, `prop`, `value` | `{accept}` or `{reject, reason}` |
| `merge` | `mappings` | merged override blocks |
| `hub` | `action` (below), `topics` | many devices behind one producer |
| `il` | `action`, `topics` (`prefix`, `source`) and the action's fields | the il-mqtt wire mapping (below) |
| `install_rules`, `merge_rules` | see above | |
| `classify`, `read`, `write`, `on_update`, `unused`, `quirk_schema`, `status_quirk`, `device_info`, `adapter_*`, `strategy_read`, `strategy_write` | plan / schema inputs | introspection for golden tests |

Driver state is explicit JSON: DP values, published values, delta totals, cover motion, converter state, settings
and timer names. No sockets or clocks are read; serialize the returned driver to continue in another process with
the same engine and pack. Internal driver state is version-specific, not a durable storage format.

Input/output tags are snake_case: `connected`, `disconnected`, `message`, `command`, `timer`; outputs `descriptor`,
`value`, `absent`, `event`, `send_message`, `reject`, `set_timer`, `cancel_timer`, `settings_changed`.
`message.json` is the flat DP map with optional `t`.

The `il` actions: `actions` (`device`, `outputs` -> host actions: `publish {topic, payload, retain, qos}`,
`schedule`, `unschedule`, `bridge {action, dps}`, `save_settings`), `clear` (`device`, `props`, `descriptor`),
`presence` (`online`), `topics` (`device`, `prop`), `parse_set` (`topic`), `encode` (`value`), `decode` (`type`,
`payload`) and `usable_id` (`device`).

The `hub` actions keep no state of their own; the host keeps each device's driver JSON and passes the one concerned:
`adopt` (`old`, `new` from `create`, `seed`, `reload`: what replacing a device publishes; with `reload` an unchanged
driver is `kept` and the old values are replayed), `remove` (`old`), `input` (`driver`, a bridge `input` or timer,
`retained`) and `write` (`driver`, `prop`, `payload`: an il write). Each returns the updated `driver` and the host
`actions`, including `bridge` `get` requests.

## Another language

A binding loads the library (or the WebAssembly module), and for each device keeps the driver JSON: `create`, then
`{"op": "hub", "action": "adopt"}` once and `"input"` / `"write"` per message, executing the host actions they return. Device registry, timers, rule files and the MQTT connection are the host's:
it decodes rule files and calls `merge_rules`. Built for `wasm32-unknown-unknown` the engine imports nothing; the host
writes each request into a `tuya_engine_alloc` buffer.

[`bindings/js/engine.mjs`](../bindings/js/engine.mjs) is such a binding for JavaScript (about 60 lines);
`node bindings/js/smoke.mjs` drives a device with the bundled rule pack, and CI runs it against the WebAssembly
build.

## Build, packaging and validation

Rust 1.88+ and Cargo are needed to build from source. Installed wheels include
the platform library and JSON pack; they need no Rust toolchain or Python
runtime dependencies.

```sh
cargo test --locked --manifest-path rust/Cargo.toml
cargo clippy --locked --manifest-path rust/Cargo.toml --all-targets -- -D warnings
pip install -e '.[test]'
python -m pytest -q
python -m build
```

The custom Hatch hook builds the release library and produces a `py3-none`
platform wheel. Release CI builds 13 wheels: Linux glibc and musl each on ARMv7,
ARM64, x86 and x64; macOS on ARM64 and x64; Windows on ARM64, x86 and x64.
Linux libraries are cross-compiled with cargo-zigbuild, then bundled and repaired
inside manylinux/musllinux containers. ARMv7 container tests use QEMU. Windows
and macOS build for an explicit Rust target so the library matches the wheel.
Each installed wheel runs the native driver and rule-loader tests; the separate
source test job runs the full suite. A source distribution is built once.

Push a version tag such as `v0.4.0` to build, test and publish all artifacts to PyPI
using the `pypi` Trusted Publishing environment. Pull requests affecting the
release inputs and `workflow_dispatch` build and test without publishing.
PyPI's Trusted Publisher must reference this repository, `release.yml` and the
`pypi` environment. No release tag is created by the workflow.

`TUYA_ENGINE_BUNDLED_LIBRARY` lets the build hook package a prebuilt library
(relative paths resolve against the project root). Without it Cargo builds the
library; `TUYA_ENGINE_TARGET` or `CARGO_BUILD_TARGET` selects an explicit target. Local Linux wheels
are tagged for their local platform; manylinux repair happens in release CI.

Existing HA core/quirk/action golden tests exercise the Rust-backed APIs. An
additional frozen oracle contains 324 complete driver traces / 3,421 steps,
covering descriptors, snapshots, active/passive packets, writes, disconnect and
reconnect. The Rust test runner consumes them without Python. The numeric corpus
contains 144 cases; the adapter tests compare Rust with Tuya SDK 0.2.15.

Oracle tooling under `scripts/rust/` imports the frozen reference only to export
vectors. Do not regenerate expected data to conceal a failing Rust test. For
new intended behavior, review the rule change and update independent goldens.
