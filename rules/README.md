# Rules

`00-default.json` ships with the package and wheel. It holds every piece of device interpretation: the platform
tables (`platforms`), one entity spec per platform (`entities`: roles and their DP candidates, plan variants, state,
actions and property layout), units, maps, feature bits, quirks, and the `overrides` section for product/device
fixes. `schema.json` defines the complete format; partial user files are validated after merging with the
defaults. The engine supplies only the interpreter, DP types and codecs, so a change here needs no rebuild.

The host reads the default pack and any user files, chooses their order, and passes decoded JSON to
`merge_rules(layers, base=pack)`. Construct `TuyaDriver(device, rules=result["rules"])`.
No directory discovery, rule-file reads or environment-selected rule layers occur in the library.
The default pack ships as an artifact for hosts to load; `00-default.json` has no special runtime priority.

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
