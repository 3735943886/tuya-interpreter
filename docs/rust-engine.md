# Portable Tuya rule engine

The complete `TuyaDriver` executes in Rust. Python transports JSON through a two-function C ABI without
conversion callbacks.

## The contract

1. **Device interpretation is JSON.** `rules/00-default.json` holds every piece of device and platform knowledge:
   category tables, entity specs, DP candidates, units, maps, feature bits, layouts and quirks.
2. **The Rust core is shared and does no I/O.** It is an interpreter plus primitives that do not depend on any
   device: DP types (validate, scale, invert), Tuya codecs (`value_convert` strategies, electricity, HSV), the
   expression language, the cover motion tracker, and the driver state machine. It reads no
   files, network or clock: hosts read rule files (and fetch anything else) and pass the decoded JSON.
3. **Bindings are thin.** A binding moves JSON across two C functions; there are no plugin callbacks. The Python
   package is such a binding. A JavaScript or other binding needs the same two functions (or
   the WebAssembly build, below) and no Tuya knowledge.
4. **Spec changes are rule layers.** A new DP code for a role, a new category, a new enum word, a new role, action,
   property or whole platform is a decoded JSON rule layer supplied by the host (see `tests/test_specs.py`). Only a new
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
| `cover_defaults`, `cover_switches`, `motion_property` | cover settings, their switch labels and the `cover_state` property |
| `strategies` | the codec names a device's `local_strategy` may use |
| `rw_roles` | roles that imply a writable property; an override's `rw: false` drops such a role |
| `overrides` | product/device fixes (README) |

`rust/engine/schema.json`, compiled into the engine, validates the whole pack. `scripts/build_rules.py` folds the generators' output (HA core tables,
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
  unknown and numbers are canonical IL values;
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
or `{"source": name, "rules": object}` to name it in reports), validates the result against the engine's schema and
returns `{rules, sources, warnings}`; with `tolerant` a layer that would make the rules invalid is reported and left
out.

Neither engine nor bindings load rule files. Hosts decode and order layers, then call
`merge_rules(layers, base=pack, tolerant=False)` in Python or `op: merge_rules` through the ABI.
Python `TuyaDriver(device, rules=pack)` requires an explicit pack.
`TUYA_ENGINE_LIBRARY` selects the native library used by the Python binding.

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
different contents, including object declaration order. Pack IDs and content fingerprints preserve that order:
platform classification and expression bindings execute in declaration order. Registered and inline packs are
validated by the same schema. Operations that interpret devices require a pack; an unknown ID is an error.
`handle` verifies the ordered content fingerprint saved at creation, so replacing rules requires a new driver.
The registry caches rules, never device state.

| `op` | Request fields | Result |
| --- | --- | --- |
| `create` | `device`, `options` | `{driver}` |
| `handle` | `driver`, `input`, `now` | `{driver, outputs}` |
| `carry` | `driver`, `old` | `{driver}` with compatible delta totals |
| `settings` | `driver` | settings block |
| `check` | `descriptor`, `state`, `prop`, `value` | `{accept}` or `{reject, reason}` |
| `merge` | `mappings` | merged override blocks |
| `install_rules`, `merge_rules` | see above | |
| `classify`, `read`, `write`, `on_update`, `unused`, `quirk_schema`, `status_quirk`, `device_info`, `adapter_*`, `strategy_read`, `strategy_write` | plan / schema inputs | introspection for golden tests |

Driver state is explicit JSON: DP values, published values, delta totals, cover motion, converter state, settings
and timer names. No sockets or clocks are read; serialize the returned driver to continue in another process with
the same engine and pack. Internal driver state is version-specific, not a durable storage format.

Input/output tags are snake_case: `connected`, `disconnected`, `message`, `command`, `timer`; outputs `descriptor`,
`value`, `absent`, `event`, `send_message`, `reject`, `set_timer`, `cancel_timer`, `settings_changed`.
`message.json` is the flat DP map with optional `t`.

## Another language

A binding loads the library or WebAssembly module, registers decoded rules, and keeps one driver JSON per device.
Call `create`, then `handle` with `input` and an explicit `now`. Inputs and outputs follow the IL driver contract;
there are no MQTT topics, payload encoders, producer actions, or bridge orchestration operations.
`send_message` carries Tuya set DPS as data; the host sends them through its chosen transport.
The host also owns the device registry, clocks, timers, rule files, settings storage and transport adapters.

[`bindings/js/engine.mjs`](../bindings/js/engine.mjs) provides `Driver.handle(now, input)`,
`describe(now, seed)`, `carry(old)` and `settingsBlock()` over WebAssembly, without reading a clock.
`node bindings/js/smoke.mjs` verifies both directions using host-loaded rules and explicit time.

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
Each installed wheel runs the native driver and rule-composition tests; the separate
source test job runs the full suite. A source distribution is built once.

Push a version tag such as `v0.4.0` to build, test and publish all artifacts to PyPI
using the `pypi` Trusted Publishing environment. Pull requests affecting the
release inputs and `workflow_dispatch` build and test without publishing.
PyPI's Trusted Publisher must reference this repository, `release.yml` and the
`pypi` environment. No release tag is created by the workflow.

`hatch_version.py` derives versions from Git via setuptools-scm. An exact tag keeps its version;
commits after `v0.4.0.dev1` advance its development number (`+3` commits gives `0.4.0.dev4`).
Commits after a final release use the next patch development version. Source archives retain their
recorded version through `PKG-INFO`, so building a wheel from an sdist does not require Git metadata.

`TUYA_ENGINE_BUNDLED_LIBRARY` lets the build hook package a prebuilt library
(relative paths resolve against the project root). Without it Cargo builds the
library; `TUYA_ENGINE_TARGET` or `CARGO_BUILD_TARGET` selects an explicit target. Local Linux wheels
are tagged for their local platform; manylinux repair happens in release CI.

Existing HA core/quirk/action golden tests exercise the Rust-backed APIs. In addition,
`rust/engine/tests/driver_vectors.json` holds 324 complete driver traces / 3,421 steps,
frozen from an earlier reference implementation, covering descriptors, snapshots,
active/passive packets, writes, disconnect and reconnect. The Rust test runner consumes
them without Python; the adapter tests compare Rust with Tuya SDK 0.2.15. Do not
rewrite expected data to conceal a failing test: for new intended behavior, review the
rule change and update the affected traces and goldens deliberately.
