"""Explicit loading of the official rules shipped with the Python package."""
import json
from importlib.resources import files
from pathlib import Path

from .native import merge_rules


def load_builtin_rules() -> dict:
    """Load official layers in filename order and return the engine-validated pack.

    Supports installed wheels and editable source checkouts. Each call returns
    fresh data; user files and remote updates are never loaded by this helper.
    """
    builtin = files("tuya_interpreter").joinpath("rules", "builtin")
    if not builtin.is_dir():
        builtin = Path(__file__).resolve().parents[2] / "rules" / "builtin"
    layers = [
        json.loads(layer.read_text(encoding="utf-8"))
        for layer in sorted(builtin.iterdir(), key=lambda item: item.name)
        if layer.is_file() and layer.name.endswith(".json") and layer.name != "manifest.json"
    ]
    return merge_rules(layers)["rules"]
