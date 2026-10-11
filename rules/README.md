# Rules

`00-default.json` ships with the package and wheel. It holds every piece of device interpretation: the platform
tables (`platforms`), one entity spec per platform (`entities`: roles and their DP candidates, plan variants, state,
actions and property layout), units, maps, feature bits, quirks, and the `overrides` section for product/device
fixes. The engine's schema ([`rust/engine/schema.json`](../rust/engine/schema.json)) defines the complete format;
partial user files are validated after merging with the defaults. The engine supplies only the interpreter, DP types and codecs, so a change here needs no rebuild.

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
examples of each. Fixes published between releases are the [override pack](#the-override-pack).

`scripts/build_rules.py` refreshes the generated parts (`platforms`, `units`, `quirks`, `host_units`) from the
generators' output and leaves the hand-authored parts alone.

See [the engine documentation](../docs/rust-engine.md) for the entity spec and expression language, declarative
JSON converters, loading operations and the C ABI.

## The override pack

Fixes for non-standard devices that reach users without a release: the files in `pack/`, which the wheel does not
bundle (hosts fetch them from `master`). `00-default.json` never
carries a block for one product; this is where those fixes live. A host copies the files listed in `pack/manifest.json`
into its rule directory (rustuya-local's `rustuya_local.host.pack.sync`, by default daily) and loads them like the
user's own files. The engine reads none of this: the host fetches the files and passes their contents.

- Each file is a rule layer, a JSON object in the rule-pack format; product/device fixes go under `overrides`
  (`{"overrides": {"<product_id>": {...}}}`). A file may also patch other rule-pack fields (a DP candidate, a map).
- A cover that runs the other way is set by a product block's `cover` settings (`invert_position`,
  `invert_set_position`, `invert_control`, `state_source`, `settle`, ...): it is the starting point, and the
  device's own switches (saved in the host's `zz_settings.json`, which the pack never writes) win.
- Name files `00_pack_<what>.json` so a user's own files (applied later, by name) refine them.
- After adding, changing or removing a file, run `python scripts/build_pack.py`; the tests check the manifest and
  that every file merges into the bundled rules.
- A file bounds the host's `LEVEL` it is for with `"requires"` / `"until"` in its manifest entry. Level 4 is this
  rule-layer format (tuya2ildevice 0.4): every file needs `"requires": 4`. When a release makes a file unnecessary,
  the host's `LEVEL` goes up and the file gets `"until": <that LEVEL>`.
