# Rules

Official rules live together in `builtin/`: `00-default.json`, any additional official layers,
and `manifest.json`. All of them ship with the wheel. The top level of a host's `rules/` directory
is for user layers. Hosts load official layers in filename order, then user layers.
The library does not discover directories. Hosts migrating from the old layout must update
the bundled default path and remote manifest URL.

`builtin/00-default.json` ships with the package and wheel. It holds every piece of device interpretation: the platform
tables (`platforms`), one entity spec per platform (`entities`: roles and their DP candidates, plan variants, state,
actions and property layout), units, maps, feature bits, quirks, and the `overrides` section for product/device
fixes. The engine's schema ([`rust/engine/schema.json`](../rust/engine/schema.json)) defines the complete format;
partial user files are validated after merging with the defaults. The engine supplies only the interpreter, DP types and codecs, so a change here needs no rebuild.

The host reads the default pack and any user files, chooses their order, and passes decoded JSON to
`merge_rules(layers, base=pack)`. Construct `TuyaDriver(device, rules=result["rules"])`.
No directory discovery, rule-file reads or environment-selected rule layers occur in the library.
The default pack ships as an artifact for hosts to load; `builtin/00-default.json` has no special runtime priority.

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
examples of each. Official changes may update the defaults directly or add a layer to the [official rule pack](#the-official-rule-pack).

`scripts/build_rules.py` refreshes the generated parts (`platforms`, `units`, `quirks`, `host_units`) from the
generators' output and leaves the hand-authored parts alone.

See [the engine documentation](../docs/rust-engine.md) for the entity spec and expression language, declarative
JSON converters, loading operations and the C ABI.

## The official rule pack

Change `builtin/00-default.json` directly for changes to the defaults, including product fixes.
If a separate file is easier to maintain, put it alongside the defaults, for example
`builtin/10_curtain.json`. Both approaches ship in the same release.

- Each JSON file other than `manifest.json` is a rule layer. Product/device fixes go under
  `overrides` (`{"overrides": {"<product_id>": {...}}}`); layers may also patch other rule fields.
- Name additional layers so they sort after `00-default.json`. User layers are applied after all official layers.
- After adding, changing or removing a rule file, run `python scripts/build_pack.py` to update
  `builtin/manifest.json`. The manifest lists every official layer, including the defaults, with its SHA-256.
- Manifest entries use `requires` / `until` to bound the host's compatible `LEVEL`.
  New entries default to `requires: 4`, the rule-layer format used by tuya2ildevice 0.4.

A host can also fetch the manifest and layers from `master` to update official rules between releases.
It should keep fetched layers under `rules/builtin/` and apply user layers afterward.
The host handles fetching and loading; the engine receives decoded JSON only.
