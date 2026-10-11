"""Test views of the native engine's introspection operations: classification, entity read/write, quirks, the DP
adapter. The package itself only drives devices; these exist so the goldens can look inside."""
from __future__ import annotations

import json
from dataclasses import dataclass
from types import SimpleNamespace

from rule_data import call, rules


class WriteRejected(ValueError):
    """A write the entity's DP types reject (never clamped)."""


class NoWritePath(Exception):
    """A codec that cannot encode."""


def env(temperature_unit: str = "C") -> dict:
    return {"temperature_unit": temperature_unit, "allowed_units": rules()["host_units"]}


def dp(type_: str, values=None, report_type: str | None = None) -> dict:
    spec = {"type": type_, "values": values if values is not None else ""}
    if report_type is not None:
        spec["report_type"] = report_type
    return spec


def device(id: str = "d", category: str = "", product_id: str = "", *, function=None, status_range=None,
           status=None, name: str = "", product_name: str = "") -> dict:
    return {"id": id, "category": category, "product_id": product_id, "name": name, "product_name": product_name,
            "function": function or {}, "status_range": status_range or {}, "status": status or {}}


def platforms() -> list[str]:
    return sorted(rules()["entities"])


def table(platform: str) -> dict:
    """A platform's description table (category -> descriptions) from the rule pack."""
    return rules()["platforms"][platform]["categories"]


@dataclass
class Slot:
    total: float = 0.0
    last_ts: int | None = None


class Plan:
    def __init__(self, raw: dict):
        self.raw = raw
        self.platform, self.key = raw["platform"], raw["key"]
        self.identity, self.config = raw["identity"], raw["config"]
        self.depends_on, self.slot_kind, self.update_all = raw["depends_on"], raw["slot_kind"], raw["update_all"]
        self.roles = {name: SimpleNamespace(code=r["code"], kind=r["kind"], report_type=r["report_type"],
                                            spec=SimpleNamespace(**r["spec"]) if isinstance(r["spec"], dict) else r["spec"])
                      for name, r in raw["roles"].items()}

    def read(self, status: dict, slot: Slot | None = None) -> dict:
        result = call("read", plan=self.raw, status=status, total=slot.total if slot else 0)
        for name in ("hs_color", "event"):
            if isinstance(result.get(name), list):
                result[name] = tuple(result[name])
        return result

    def write(self, action: str, args: dict, status: dict) -> list[dict]:
        try:
            return call("write", plan=self.raw, action=action, args=args, status=status)
        except ValueError as error:
            raise WriteRejected(str(error)) from error


def classify(dev: dict, environment: dict | None = None, only=None) -> list[Plan]:
    return [Plan(p) for p in call("classify", device=dev, env=environment or env(), platforms=list(only) if only else None)]


def unused_plans(dev: dict, entries: dict, consumed=()) -> list[Plan]:
    return [Plan(p) for p in call("unused", device=dev, entries=entries, consumed=sorted(consumed))]


def new_slot(plan: Plan) -> Slot | None:
    return Slot() if plan.slot_kind else None


def on_update(plan: Plan, slot: Slot | None, changed, timestamps, status) -> SimpleNamespace:
    result = call("on_update", plan=plan.raw, slot=vars(slot) if slot else None, changed=changed,
                  timestamps=timestamps, status=status)
    if slot and result["slot"] is not None:
        slot.total, slot.last_ts = result["slot"]["total"], result["slot"]["last_ts"]
    return SimpleNamespace(write_state=result["write_state"], fire=tuple(result["fire"]) if result["fire"] else None)


# ---- quirks --------------------------------------------------------------------------------------
def quirks() -> dict:
    return rules()["quirks"]


def quirk_for(product_id: str) -> dict | None:
    return quirks().get(product_id)


def apply_quirk(dev: dict, quirk: dict | None = None) -> dict:
    return call("quirk_schema", device=dev, quirk=quirk)


def apply_status_quirk(quirk: dict | None, status: dict) -> dict:
    return call("status_quirk", quirk=quirk or {}, status=status)


def device_info(dev: dict, quirk: dict | None = None) -> dict:
    return call("device_info", device=dev, quirk=quirk)


# ---- DP adapter ----------------------------------------------------------------------------------
def _strategy(name: str, direction: str):
    def convert(value, config):
        try:
            return call("strategy_" + direction, name=name, value=value, config=config)
        except ValueError as error:
            if direction == "write" and str(error).startswith("unsupported:"):
                raise NoWritePath(str(error)) from error
            raise
    return convert


STRATEGIES = {name: (_strategy(name, "read"), _strategy(name, "write")) for name in rules()["strategies"]}


class Adapter:
    def __init__(self, raw: dict):
        self.raw = raw
        self.unsupported = raw["unsupported"]

    @classmethod
    def from_local_strategy(cls, local_strategy: dict, status_range: dict | None = None) -> Adapter:
        return cls(call("adapter_create", device={"local_strategy": local_strategy or {}, "status_range": status_range or {}},
                        dpmap={}))

    def read(self, dps: dict) -> dict:
        return call("adapter_read", adapter=self.raw, dps={str(k): v for k, v in dps.items()})

    def write(self, commands: list[dict]) -> tuple[dict, list]:
        try:
            dps, missing = call("adapter_write", adapter=self.raw, commands=commands)
        except ValueError as error:
            if str(error).startswith("unsupported:"):
                raise NoWritePath(str(error)) from error
            raise
        return dps, missing


def values(spec: dict | str | None) -> dict | None:
    """A DP spec's `values`, decoded."""
    return json.loads(spec) if isinstance(spec, str) and spec else spec or None
