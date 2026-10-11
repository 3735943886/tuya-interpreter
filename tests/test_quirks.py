"""apply_quirk == tuya-device-handlers' post-quirk schema (oracle dumps: quirk_golden.json real fixtures,
quirk_synthetic_golden.json empty-schema patches)."""
import json
import pathlib

import fixtures
from engine import (
    apply_quirk,
    apply_status_quirk,
    classify,
    device,
    device_info,
    dp,
    quirk_for,
    quirks,
)

here = pathlib.Path(__file__).parent / "golden"


def norm_values(v):
    if isinstance(v, str):
        try:
            return json.loads(v) if v else None
        except ValueError:
            return v
    return v


def dump(s):
    return {"category": s["category"],
            "function": {k: (v["type"], norm_values(v["values"])) for k, v in s["function"].items()},
            "status_range": {k: (v["type"], norm_values(v["values"]), v.get("report_type"))
                             for k, v in s["status_range"].items()}}


def oracle(o):
    return {"category": o["category"],
            "function": {k: (v["type"], norm_values(v["values"])) for k, v in o["function"].items()},
            "status_range": {k: (v["type"], norm_values(v["values"]), v.get("report_type")) for k, v in o["status_range"].items()}}


def test_real_fixtures():
    gold = json.loads((here / "quirk_golden.json").read_text())
    assert gold, "no quirk fixtures"
    for code, g in gold.items():
        s = fixtures.load(code)
        got = dump(apply_quirk(s))
        want = oracle(g["after"])
        assert got == want, code
        assert dump(s) == oracle(g["before"]), "apply_quirk must not mutate its input"


def test_synthetic_empty_schema_patches():
    gold = json.loads((here / "quirk_synthetic_golden.json").read_text())
    assert len(gold) == 32 == len(quirks())
    for pid, g in gold.items():
        s = device("x", "", pid, status={"temp_set": 100})
        got = dump(apply_quirk(s))
        want = oracle(g)
        assert got == want, pid


def test_when_guard_and_status_quirk_and_device_info():
    assert quirk_for("hw50w7qvxluhslkk") is not None
    for raw, applies in ((600, True), (300, False), ("600", False), (None, False)):
        s = apply_quirk(device("x", "kt", "hw50w7qvxluhslkk", status={"temp_set": raw} if raw is not None else {}))
        assert ("temp_set" in s["status_range"]) == applies, raw
    assert apply_status_quirk({"ops": [{"op": "MapInitialStatus", "code": "a", "mapping": [["true", True]]}]}, {"a": "true"}) == {"a": True}
    assert device_info(device("x", "c", "nothing", product_name="P"))["manufacturer"] == "Tuya"
    di = device_info(device("x", "c", "avriaapskyik4eaa", product_name="P"))
    assert di["manufacturer"] == "Konyks" and di["model_id"] is None


def test_invert_int_max_leaves_the_position_as_the_device_counts_it():
    """Quirk TypeOverride invert_int_max only cancelled core's cover reversal; the engine does not reverse, so the
    position is the device's number with or without the quirk."""
    rng = '{"unit":"%","min":0,"max":100,"scale":0,"step":1}'
    def plan(pid):
        sch = device("x", "cl", pid,
                     function={"control": dp("Enum", '{"range":["open","close","stop"]}'),
                               "percent_control": dp("Integer", rng)},
                     status_range={"percent_state": dp("Integer", rng)},
                     status={"percent_state": 30})
        sch = apply_quirk(sch)
        return next(e for e in classify(sch, only=("cover",)) if e.key == "control")
    plain, inv = plan("unmapped_pid"), plan("68nvbio9")
    st = {"percent_state": 30}
    for p in (plain, inv):
        assert p.read(st)["current_position"] == 30
        assert p.write("set_cover_position", {"position": 25}, st) == [{"code": "percent_control", "value": 25}]


if __name__ == "__main__":
    for n, f in list(globals().items()):
        if n.startswith("test_"):
            f()
            print("ok", n)
