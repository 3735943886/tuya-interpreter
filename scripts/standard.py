"""Tuya's own category list, applied over HA core's tables.

Core's tables are the behaviour reference, but where Tuya's standard instruction set names a category differently
from what core assumes, the standard wins. Both inputs are data (tables/_tuya_categories.json, generated from Tuya's
page; tables/_tuya_standard_rules.json, the decisions) -- nothing about a category is hard-coded here.
"""
from __future__ import annotations

import functools
from typing import Any

from . import load_json


@functools.cache
def categories() -> dict[str, str]:
    """Category code -> the name Tuya's standard gives it (`kg` -> "Switch")."""
    return load_json("tables", "_tuya_categories.json")["categories"]


@functools.cache
def _rules() -> dict[str, Any]:
    return load_json("tables", "_tuya_standard_rules.json")


def switch_device_class(category: str, core_class: str | None) -> str | None:
    """The switch device class core's table gives a description of `category`, after the standard's say."""
    rule = _rules()["switch_device_class"]
    if core_class != rule["replaces"]:
        return core_class
    return rule["by_name"].get(categories().get(category, ""), core_class)


def apply(platform: str, tables: dict[str, Any]) -> dict[str, Any]:
    """`tables` (one platform's `{table name: {category: [description, ...]}}`) with the standard applied."""
    if platform == "switch":
        for by_category in tables.values():
            for category, descs in by_category.items():
                for desc in descs if isinstance(descs, list) else [descs]:
                    if "device_class" in desc:
                        desc["device_class"] = switch_device_class(category, desc["device_class"])
    return tables
