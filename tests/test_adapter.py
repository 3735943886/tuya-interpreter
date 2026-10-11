"""Adapter.read == tuya_sharing Manager._on_device_report (SDK as oracle; needs tuya_sharing) + write round-trips.
Run with the oracle venv (tuya-device-sharing-sdk==0.2.15)."""
import base64
import json
import pathlib
import random

import fixtures
import pytest
from engine import STRATEGIES, Adapter, NoWritePath, dp

here = pathlib.Path(__file__).parent / "golden"
try:
    import tuya_sharing.strategy_repo  # noqa: F401  (registers strategies)
    from tuya_sharing.strategy import strategy as SDK
except ImportError:                       # the SDK is the oracle; without it these tests cannot run
    import pytest
    pytest.skip("tuya_sharing (tuya-device-sharing-sdk==0.2.15) not installed", allow_module_level=True)

random.seed(7)


def sdk_read(local_strategy, status_range, dpid, raw):
    """Exactly Manager._on_device_report for one item (device.support_local True)."""
    m = local_strategy[dpid]
    code, value = SDK.convert(m["value_convert"], (m["status_code"], raw), m["config_item"])
    sr = status_range.get(code)
    if sr and sr["type"] == "Enum" and value not in json.loads(sr["values"]).get("range", []):
        return None
    return {code: value}


def samples(strategy, ci):
    base = [None, "", 0, 1, 2, "0", "1", "2", "on", "off", "memory", "single_click", "long_press", True, False, 220]
    if strategy.startswith("dj_v2"):
        def hx(n):
            return "".join(random.choice("0123456789abcdef") for _ in range(n))
        return base + [hx(12), "00b403e803e8", hx(21), hx(2 + 26 * 3), hx(2)]
    return base


def test_read_matches_sdk():
    n = 0
    for code in fixtures.all_codes():
        d = fixtures.load(code)
        ls = d["local_strategy"]
        if not ls:
            continue
        rng = {k: {"type": v["type"], "values": v["values"]} for k, v in d["status_range"].items()}
        ad = Adapter.from_local_strategy(ls, {k: dp(v["type"], v["values"]) for k, v in d["status_range"].items()})
        for dpid, meta in ls.items():
            if meta["value_convert"] not in STRATEGIES:
                continue
            for raw in samples(meta["value_convert"], meta["config_item"]):
                try:
                    want = sdk_read(ls, rng, dpid, raw)
                except Exception:  # noqa: BLE001, S112  (the SDK itself raises on this input: adapter drops it, checked below)
                    continue
                assert ad.read({dpid: raw}) == (want or {}), (code, dpid, meta["value_convert"], raw)
                n += 1
    assert n > 200, n
    print("compared", n, "sdk conversions")


def test_write_roundtrips():
    _r_color, _w_color = STRATEGIES["dj_v2_color_alg"]
    _r_contr, _w_contr = STRATEGIES["dj_v2_contr_alg"]
    _r_scene, _w_scene = STRATEGIES["dj_v2_scene_alg"]
    for raw in ("00b403e803e8", "012c00ff0010"):
        assert _w_color(_r_color(raw, {}), {}) == raw
    for raw in ("1" + "00b4" + "03e8" + "03e8" + "0064" + "0100", "0" + "0000" * 5):
        assert len(raw) == 21 and _w_contr(_r_contr(raw, {}), {}) == raw
    scene = "01" + "0a0b02" + "00b4" + "03e8" + "03e8" + "0100" + "0100"
    assert _w_scene(_r_scene(scene, {}), {}) == scene
    ci = {"enumMappingMap": {"0": {"value": "power_off"}, "off": {"value": "power_off"}, "1": {"value": "power_on"}}}
    ad = Adapter.from_local_strategy({"5": {"value_convert": "enum", "status_code": "relay_status",
                                            "config_item": {"statusFormat": '{"relay_status":"$"}', "valueType": "Enum",
                                                            "valueDesc": '{"range":["power_off","power_on"]}', **ci}}})
    assert ad.write([{"code": "relay_status", "value": "power_off"}]) == ({"5": "0"}, [])
    assert ad.write([{"code": "nope", "value": 1}]) == ({}, ["nope"])


def test_statusformat_key_and_unsupported_and_enum_guard():
    ls = {"2": {"value_convert": "default", "status_code": "humidity_value",
                "config_item": {"statusFormat": '{"va_humidity":"$"}', "valueType": "Integer", "valueDesc": '{"min":0,"max":100}'}},
          "9": {"value_convert": "db_v1_data", "status_code": "t", "config_item": {"statusFormat": '{"timer":"$"}'}},
          "3": {"value_convert": "default", "status_code": "mode",
                "config_item": {"statusFormat": {"mode": "$"}, "valueType": "Enum", "valueDesc": '{"range":["a","b"]}'}}}
    ad = Adapter.from_local_strategy(ls, {"mode": dp("Enum", '{"range":["a","b"]}')})
    assert ad.read({"2": 55, "9": "xx", "3": "b", "77": 1}) == {"va_humidity": 55, "timer": "xx", "mode": "b"}
    assert ad.read({"3": "zzz"}) == {}                      # Enum guard drops
    assert ad.unsupported == {"9": "db_v1_data"}


def _b64(bs):
    return base64.b64encode(bytes(bs)).decode()


def _hex(n):
    return "".join(random.choice("0123456789abcdef") for _ in range(n))


def _new_samples(name):
    """Raw values as a device would send them for each strategy ported after the first six, plus edge cases."""
    r = random.randrange
    if name in ("cz_timer1_alg", "cz_timer2_alg"):
        w = 6 if name == "cz_timer1_alg" else 10
        return [None, ""] + [_b64(r(256) for _ in range(w * k)) for k in (1, 2, 3) for _ in range(15)]
    if name in ("dj_v1_hsv_alg", "voice_atm_color"):
        return [None, "ff00000168ffff", "00ff800168ffff"] + [_hex(6) + "0168ffff" for _ in range(20)] + \
               [_hex(6) + f"{r(361):04x}" + _hex(4) for _ in range(40)]
    if name == "dj_v1_scene_alg":
        return [None, ""] + [_hex(6) + f"{k:02x}" + _hex(6 * k) for k in (1, 2, 5) for _ in range(10)]
    if name == "hb_djv1_color":
        return [None] + [json.dumps({"h": r(361), "s": r(256), "v": r(256)}) for _ in range(40)]
    if name == "hb_jsq_lightv1":
        return [None, "", "abc", "12"]
    if name in ("hb_range_v1", "hb_range_v2"):
        return [None, -5, 0, 24, 25, 26, 100, 254, 255, 300, "30"] + [r(256) for _ in range(40)]
    if name == "ms_dp_syn_alg":
        return [None, "", "!!"] + [_b64(v for _ in range(k) for v in (r(1, 128), r(256))) for k in (1, 2, 4) for _ in range(10)]
    if name == "sd_clean_record":
        digits = "0123456789"
        return [None] + ["".join(random.choice(digits) for _ in range(n)) for n in (6, 11, 18, 23, 25) for _ in range(5)]
    if name == "db_v1_params":
        return [None, _b64(r(256) for _ in range(8))] + [_b64([1, 15] + [r(256) for _ in range(15)]) for _ in range(15)] + \
               [_b64([2, 15] + [r(256) for _ in range(15)] + [r(16)]) for _ in range(15)]
    if name in ("db_v1_daily", "db_v1_month"):
        return [None] + [_b64(r(256) for _ in range(8)) for _ in range(20)]
    if name == "db_v1_frozen":
        return [None] + [_b64(r(256) for _ in range(2)) for _ in range(20)]
    if name == "db_v1_alarm":
        return [None, ""] + [_b64(v for _ in range(k) for v in (r(1, 16), r(3), r(256), r(256))) for k in (1, 3, 6)
                             for _ in range(10)]
    raise AssertionError(name)


NEW = ("cz_timer1_alg", "cz_timer2_alg", "dj_v1_hsv_alg", "voice_atm_color", "dj_v1_scene_alg", "hb_djv1_color",
       "hb_jsq_lightv1", "hb_range_v1", "hb_range_v2", "ms_dp_syn_alg", "sd_clean_record", "db_v1_params",
       "db_v1_daily", "db_v1_month", "db_v1_frozen", "db_v1_alarm")


def test_the_later_strategies_read_as_the_sdk_does():
    ci = {"statusFormat": '{"x":"$"}', "valueType": "String", "valueDesc": "{}"}
    n = 0
    for name in NEW:
        assert name in STRATEGIES, name
        for raw in _new_samples(name):
            try:
                _, want = SDK.convert(name, ("x", raw), ci)
            except Exception:  # noqa: BLE001, S112  (the SDK raises: the adapter drops the value, checked in read())
                continue
            got = STRATEGIES[name][0](raw, ci)
            if isinstance(want, set):
                want = sorted(want)                                  # ms_dp_syn_alg: the same data, JSON-safe
            assert got == want, (name, raw, got, want)
            n += 1
    assert n > 500, n


def test_the_later_strategies_that_can_be_written_round_trip():
    ci = {}
    for name, width in (("cz_timer1_alg", 6), ("cz_timer2_alg", 10)):
        read, write = STRATEGIES[name]
        for _ in range(30):
            raw = _b64([random.randrange(2)] + [random.randrange(128)] +
                       [b for _ in range((width - 2) // 2) for b in divmod(random.randrange(24 * 60), 256)])
            assert write(read(raw, ci), ci) == raw, (name, raw)
    read, write = STRATEGIES["dj_v1_hsv_alg"]
    for _ in range(50):                        # reading rounds s and v through a 0..1 share (SDK), so compare reads
        raw = _hex(6) + f"{random.randrange(361):04x}" + _hex(4)
        assert read(write(read(raw, ci), ci), ci) == read(raw, ci), raw
    read, write = STRATEGIES["hb_jsq_lightv1"]
    assert write(read("abc", ci), ci) == "abc"
    for name in ("dj_v1_scene_alg", "hb_djv1_color", "hb_range_v1", "hb_range_v2", "ms_dp_syn_alg", "sd_clean_record",
                 "db_v1_params", "db_v1_daily", "db_v1_month", "db_v1_frozen", "db_v1_alarm"):
        with pytest.raises(NoWritePath):
            STRATEGIES[name][1]("x", ci)


if __name__ == "__main__":
    for n, f in list(globals().items()):
        if n.startswith("test_"):
            f()
            print("ok", n)
