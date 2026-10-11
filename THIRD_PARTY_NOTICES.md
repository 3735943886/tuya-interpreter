# Third-party notices

## home-assistant/core and tuya-device-handlers

The rule pack (`rules/00-default.json`) and the engine (`rust/engine/`) are a sans-I/O, data-driven reproduction of
[home-assistant/core](https://github.com/home-assistant/core)'s official `tuya` integration (Apache-2.0) and of
[tuya-device-handlers](https://github.com/make-all/tuya-local) (Apache-2.0). The pack's category/platform tables
(`platforms`) and quirk registry (`quirks`) are *generated* from those sources by `scripts/gen_core_tables.py` and
`scripts/gen_quirks.py` against a pinned checkout of each and folded in by `scripts/build_rules.py`, not hand-copied;
the entity specs (`entities`) are hand-authored but reviewed against tuya-device-handlers' `definition/*.py`. Behaviour is
pinned by the golden tests in `scripts/golden/` (built from core's own test fixtures and snapshots).

Same author as this project (MIT); the generated data derived from Apache-2.0 sources keeps that notice.

## tuya-device-sharing-sdk

`rust/engine/src/adapter.rs` reproduces (does not vendor) the `default`, `enum` and `dj_v2_{color,contr,music,scene}_alg`
`value_convert` strategies of [tuya-device-sharing-sdk](https://github.com/tuya/tuya-device-sharing-sdk)
(`strategy_repo/`, Apache-2.0) — specifically `Manager._on_device_report`'s read direction — plus the inverse write
direction the SDK does not have. It is tested against the SDK's own strategies.
