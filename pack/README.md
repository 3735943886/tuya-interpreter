# The override pack

Fixes for non-standard devices that reach users without a release. The rule pack (`rules/00-default.json`) never
carries a block for one product; this is where those fixes live. A host copies the files listed in `manifest.json`
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
