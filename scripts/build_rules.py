"""Fold generator outputs into the canonical rule pack, rules/builtin/00-default.json.

Usage: python build_rules.py [--tables DIR] [--quirks FILE] [--host-units FILE] [--pack FILE]

* --tables: gen_core_tables.py's output directory. Each platform's description table in the pack is replaced by the
  generated one with Tuya's own category list applied (standard.py); `_units.json` becomes the pack's `units`.
* --quirks: gen_quirks.py's output; its `quirks` become the pack's.
* --host-units: gen_host_units.py's output; it becomes the pack's `host_units`.

Everything else in the pack (entities, maps, constants, ...) is hand-authored and left as it is.
"""
import argparse
import json
import pathlib

import standard

PACK = pathlib.Path(__file__).resolve().parents[1] / "rules/builtin/00-default.json"


def load(path: pathlib.Path):
    return json.loads(path.read_text(encoding="utf-8"))


def build(pack: dict, tables: pathlib.Path | None, quirks: pathlib.Path | None, host_units: pathlib.Path | None) -> dict:
    if tables:
        for platform, group in pack["platforms"].items():
            generated = load(tables / f"{platform}.json")["tables"][group["table"]]
            group["categories"] = standard.apply(platform, {group["table"]: generated})[group["table"]]
        pack["units"] = load(tables / "_units.json")
    if quirks:
        pack["quirks"] = load(quirks)["quirks"]
    if host_units:
        pack["host_units"] = load(host_units)
    return pack


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--tables", type=pathlib.Path)
    parser.add_argument("--quirks", type=pathlib.Path)
    parser.add_argument("--host-units", type=pathlib.Path)
    parser.add_argument("--pack", type=pathlib.Path, default=PACK)
    args = parser.parse_args()
    pack = build(load(args.pack), args.tables, args.quirks, args.host_units)
    args.pack.write_text(json.dumps(pack, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
