# Rules

`00-default.json` ships with the package and wheel. It holds every piece of device interpretation: the platform
tables (`platforms`), one entity spec per platform (`entities`: roles and their DP candidates, plan variants, state,
actions and property layout), units, maps, feature bits, quirks, and the `overrides` section for product/device
fixes. `schema.json` defines the complete format; partial user files are validated after merging with the
defaults. The engine supplies only the interpreter, DP types and codecs, so a change here needs no rebuild.

Put user files in a separate directory and set `TUYA_ENGINE_RULES` to that directory,
or use `load_rules([directory])` and pass the result as `TuyaDriver(..., rules=...)`.
The native loader collects visible `*.json` files from all supplied locations and applies them in
ascending filename byte order (ASCII ascending for ASCII names). Equal filenames
preserve the supplied location order. `00-default.json` has no special handling. Loading is non-recursive. Hidden
files and `schema.json` are ignored. Saved settings in `zz_settings.json` follow the same filename order. Later
filenames can override them.

Objects merge recursively. Arrays and scalar values replace earlier values. `{"$delete": true}` deletes an object
member, `{"$append": [...]}` extends an array, and `null` is a literal value.

A product fix, `10_curtain.json`:

```json
{
  "overrides": {
    "my_product_id": {
      "cover": {"invert_position": true}
    }
  }
}
```

A device that reports its room temperature on another DP, `20_room_temperature.json`:

```json
{"entities": {"climate": {"params": {"codes": {"cur_temp_c": {"$append": ["room_temperature"]}}}}}}
```

A category Home Assistant does not list, `30_category.json`:

```json
{"platforms": {"switch": {"categories": {"zz": [{"key": "switch_1"}]}}}}
```

New roles, state fields, actions, layout rows and whole platforms work the same way; `tests/test_specs.py` has
examples of each. Fixes published between releases are the [override pack](../pack/README.md), rule files a host
fetches into its rule directory.

`scripts/build_rules.py` refreshes the generated parts (`platforms`, `units`, `quirks`, `host_units`) from the
generators' output and leaves the hand-authored parts alone.

See [the engine documentation](../docs/rust-engine.md) for the entity spec and expression language, declarative
JSON converters, loading operations and the C ABI.
