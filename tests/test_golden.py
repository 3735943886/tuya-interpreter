"""Compare the engine's classification against core snapshot goldens (golden.json), per implemented platform."""
import json
import pathlib

import fixtures
import pytest
import standard
from engine import classify, env, platforms, quirk_for, table

ENV = env()
PLATFORMS = platforms()

assert len(fixtures.all_codes()) == 324, 'fixtures missing'
GOLD = json.loads((pathlib.Path(__file__).parent / "golden" / "golden.json").read_text())

def fixture(code):
    return fixtures.load(code)

# core's button/binary_sensor/sensor snapshot tests run with entity_registry_enabled_by_default
NO_ENABLED_GOLD = {"button", "binary_sensor", "sensor"}
def _onoff(rd): return {True: "on", False: "off", None: "unknown"}[rd["is_on"]]
STATE_OF = {"switch": _onoff, "binary_sensor": _onoff,
            "humidifier": lambda rd: {True: "on", False: "off", None: "unknown"}[rd["is_on"]],
            "alarm_control_panel": lambda rd: rd["alarm_state"] or "unknown",
            "vacuum": lambda rd: rd["activity"] or "unknown",
            "climate": lambda rd: rd["hvac_mode"] or "unknown",
            "light": lambda rd: {True: "on", False: "off", None: "unknown"}[rd["is_on"]],
            "fan": lambda rd: {True: "on", False: "off", None: "unknown"}[rd["is_on"]],
            "cover": lambda rd: {True: "closed", False: "open", None: "unknown"}[rd["is_closed"]],
            "siren": _onoff, "valve": lambda rd: {True: "closed", False: "open", None: "unknown"}[rd["is_closed"]],
            "select": lambda rd: "unknown" if rd["current_option"] is None else rd["current_option"],
            "number": lambda rd: "unknown" if rd["native_value"] is None else str(rd["native_value"])}


def core_reverses(code, key):
    """Did core mirror this cover's position? The engine deliberately does not (platforms.cover): core reverses by its
    position wrapper (always, or unless control_back_mode is back), and a quirk's invert_int_max cancels that."""
    d = fixtures.load(code)
    desc = next((x for x in table("cover").get(d["category"], []) if x["key"] == key), {})
    wrapper = (desc.get("position_wrapper") or "$DPCodeInvertedPercentageWrapper").lstrip("$")
    rev = d["status"].get("control_back_mode") != "back" if wrapper == "ControlBackModePercentageMappingWrapper" else True
    quirk = quirk_for(d["product_id"]) or {}
    if any(op["op"] == "TypeOverride" and op.get("as") == "invert_int_max" for op in quirk.get("ops", [])):
        rev = not rev
    return rev


# HA (not the engine) converts to the unit system's default display unit for these; verified by hand
HOST_DISPLAY_UNIT = {("qxj_fsea1lat3vuktbt6", "windspeed_avg")}
ATTRS = (("translation_key", "translation_key"), ("device_class", "device_class"), ("entity_category", "entity_category"))

def run(platform):
    miss, extra, attr = [], [], []
    for code in fixtures.all_codes():
        got = {e.key: e for e in classify(fixture(code), ENV, (platform,))}
        want = {r["key"]: r for r in GOLD.get(code, {}).get(platform, [])}
        for k in want.keys() - got.keys():
            miss.append((code, k))
        for k in got.keys() - want.keys():
            extra.append((code, k))
        for k in want.keys() & got.keys():
            w, g = want[k], got[k].identity
            for gk, ik in ATTRS:
                want_v = w[gk]
                if platform == "switch" and gk == "device_class":
                    # core's golden, with Tuya's own category list applied over it (tuya/standard.py)
                    want_v = standard.switch_device_class(fixtures.load(code)["category"], want_v)
                if want_v != g.get(ik):
                    attr.append((code, k, gk, want_v, g.get(ik)))
            cap = w["capabilities"] or {}
            if platform in ("number", "sensor"):
                eu = g.get("suggested_unit") or g.get("native_unit")
                if w["unit"] != eu and (code, k) not in HOST_DISPLAY_UNIT:
                    attr.append((code, k, "unit", w["unit"], eu))
                if platform == "sensor" and w["device_class"] != g.get("device_class"):
                    pass
            if platform == "sensor":
                if cap.get("state_class") != g.get("state_class"):
                    attr.append((code, k, "state_class", cap.get("state_class"), g.get("state_class")))
                if cap.get("options") != g.get("options"):
                    attr.append((code, k, "options", cap.get("options"), g.get("options")))
            if platform == "number":
                for ck, ik in (("min","native_min_value"),("max","native_max_value"),("step","native_step")):
                    if cap.get(ck) != g.get(ik):
                        attr.append((code, k, "cap."+ck, cap.get(ck), g.get(ik)))
            st = fixture(code)
            rd = got[k].read(st["status"]) if platform != "button" else None
            if rd is not None and platform == "sensor":
                exp, val = w["state"], rd["native_value"]
                if not st["online"]:
                    val = "unavailable"
                elif val is None:
                    val = "unknown"
                conv = g.get("suggested_unit") not in (None, g.get("native_unit")) or (code, k) in HOST_DISPLAY_UNIT
                if not conv and exp != "unavailable":
                    try:
                        same = abs(float(exp) - float(val)) < 1e-6 * max(1, abs(float(exp)))
                    except (TypeError, ValueError):
                        same = exp == val
                    if not same and k not in ("wind_direction",):
                        attr.append((code, k, "state", exp, val))
            flip = platform == "cover" and core_reverses(code, k)
            if rd is not None and platform in STATE_OF:
                exp = w["state"]
                core_pos = (w["attributes"] or {}).get("current_position") if platform == "cover" else None
                if flip and core_pos is not None and exp in ("open", "closed"):
                    exp = "closed" if 100 - core_pos == 0 else "open"   # closed is position 0, the other end now
                val = STATE_OF[platform](rd)
                if not st["online"]:
                    val = "unavailable"
                if val != exp:
                    attr.append((code, k, "state", exp, val))
            if platform == "cover":
                if int(w["supported_features"]) != g["supported_features"]:
                    attr.append((code, k, "features", w["supported_features"], g["supported_features"]))
                a = w["attributes"] or {}
                for ak, rk in (("current_position", "current_position"), ("current_tilt_position", "current_tilt_position")):
                    want_p = a.get(ak)
                    if flip and ak == "current_position" and want_p is not None:
                        want_p = 100 - want_p
                    if want_p != rd[rk] and st["online"]:
                        attr.append((code, k, ak, want_p, rd[rk]))
            if platform == "humidifier":
                a = w["attributes"] or {}
                if int(w["supported_features"]) != g["supported_features"]:
                    attr.append((code, k, "features", w["supported_features"], g["supported_features"]))
                for ck, ik in (("min_humidity", "min_humidity"), ("max_humidity", "max_humidity")):
                    if cap.get(ck) != g[ik]:
                        attr.append((code, k, ck, cap.get(ck), g[ik]))
                if cap.get("available_modes") != g.get("available_modes") and "available_modes" in cap:
                    attr.append((code, k, "modes", cap.get("available_modes"), g.get("available_modes")))
                if w["state"] not in ("unavailable",):
                    for ak, rk in (("humidity", "target_humidity"), ("current_humidity", "current_humidity"), ("mode", "mode")):
                        if a.get(ak) != rd[rk]:
                            attr.append((code, k, ak, a.get(ak), rd[rk]))
            if platform == "alarm_control_panel" and int(w["supported_features"]) != g["supported_features"]:
                attr.append((code, k, "features", w["supported_features"], g["supported_features"]))
            if platform == "vacuum":
                a = w["attributes"] or {}
                if int(w["supported_features"]) != g["supported_features"]:
                    attr.append((code, k, "features", w["supported_features"], g["supported_features"]))
                if a.get("fan_speed_list", []) != g["fan_speed_list"] and "fan_speed_list" in a:
                    attr.append((code, k, "fan_speed_list", a.get("fan_speed_list"), g["fan_speed_list"]))
                if a.get("fan_speed") != rd["fan_speed"]:
                    attr.append((code, k, "fan_speed", a.get("fan_speed"), rd["fan_speed"]))
            if platform == "climate":
                a = w["attributes"] or {}
                if int(w["supported_features"]) != g["supported_features"]:
                    attr.append((code, k, "features", w["supported_features"], g["supported_features"]))
                for ck, ik in (("hvac_modes", "hvac_modes"), ("fan_modes", "fan_modes"), ("preset_modes", "preset_modes"), ("swing_modes", "swing_modes")):
                    if a.get(ck) != g.get(ik):
                        attr.append((code, k, ck, a.get(ck), g.get(ik)))
                if g["supported_features"] & 1:
                    for ck, ik in (("min_temp", "min_temp"), ("max_temp", "max_temp"), ("target_temp_step", "target_temperature_step")):
                        if cap.get(ck, a.get(ck)) != g[ik]:
                            attr.append((code, k, ck, cap.get(ck, a.get(ck)), g[ik]))
                if w["state"] != "unavailable":
                    for ck, rk in (("current_temperature", "current_temperature"), ("temperature", "temperature"), ("current_humidity", "current_humidity"),
                                   ("fan_mode", "fan_mode"), ("preset_mode", "preset_mode"), ("swing_mode", "swing_mode")):
                        if a.get(ck) != rd[rk]:
                            attr.append((code, k, ck, a.get(ck), rd[rk]))
            if platform == "light":
                a = w["attributes"] or {}
                if sorted(cap.get("supported_color_modes", [])) != g["supported_color_modes"]:
                    attr.append((code, k, "modes", cap.get("supported_color_modes"), g["supported_color_modes"]))
                if w["state"] == "on":
                    if a.get("color_mode") != rd["color_mode"]:
                        attr.append((code, k, "color_mode", a.get("color_mode"), rd["color_mode"]))
                    # HA exposes color_temp_kelvin only in color_temp mode and hs_color only in hs mode
                    # (the other one is derived by HA), and brightness as-is
                    for ak in ("brightness",) + (("color_temp_kelvin",) if rd["color_mode"] == "color_temp" else ()):
                        if a.get(ak) != rd[ak]:
                            attr.append((code, k, ak, a.get(ak), rd[ak]))
                    hs = rd["hs_color"]
                    if rd["color_mode"] == "hs" and a.get("hs_color") is not None and (hs is None or [round(x, 3) for x in a["hs_color"]] != [round(x, 3) for x in hs]):
                        attr.append((code, k, "hs_color", a.get("hs_color"), hs))
            if platform == "fan":
                if int(w["supported_features"]) != g["supported_features"]:
                    attr.append((code, k, "features", w["supported_features"], g["supported_features"]))
                a = w["attributes"] or {}
                if a.get("preset_modes") != g.get("preset_modes") and "preset_modes" in a:
                    attr.append((code, k, "preset_modes", a.get("preset_modes"), g.get("preset_modes")))
                if w["state"] == "on":
                    for ak in ("percentage", "preset_mode", "oscillating", "direction"):
                        if ak in a and a[ak] != rd[ak]:
                            attr.append((code, k, ak, a[ak], rd[ak]))
            if platform == "event" and cap.get("event_types") != g.get("event_types"):
                attr.append((code, k, "event_types", cap.get("event_types"), g.get("event_types")))
            if platform == "select":
                rd = got[k].read(fixture(code)["status"])
                if cap.get("options") != rd["options"]:
                    attr.append((code, k, "cap.options", cap.get("options"), rd["options"]))
            if platform not in NO_ENABLED_GOLD and (w["disabled_by"] is not None) != (not g["entity_registry_enabled_default"]):
                attr.append((code, k, "enabled_default", w["disabled_by"], g["entity_registry_enabled_default"]))
    total = sum(len(v.get(platform, [])) for v in GOLD.values())
    print(f"{platform}: golden {total}  missing {len(miss)}  extra {len(extra)}  attr-mismatch {len(attr)}")
    for x in (miss[:5], extra[:5], attr[:8]):
        for i in x:
            print("   ", i)
    return not (miss or extra or attr)

ACTION = {"$SERVICE_TURN_ON": "turn_on", "$SERVICE_TURN_OFF": "turn_off", "$SERVICE_PRESS": "press",
          "$SERVICE_SELECT_OPTION": "select", "$SERVICE_SET_VALUE": "set_value", "$SERVICE_SET_HUMIDITY": "set_humidity", "$SERVICE_SET_MODE": "set_mode", "$SERVICE_ALARM_ARM_AWAY": "arm_away", "$SERVICE_ALARM_ARM_HOME": "arm_home", "$SERVICE_ALARM_DISARM": "disarm", "$SERVICE_ALARM_TRIGGER": "trigger", "$SERVICE_RETURN_TO_BASE": "return_to_base", "$SERVICE_SET_FAN_SPEED": "set_fan_speed", "$SERVICE_LOCATE": "locate", "$SERVICE_START": "start", "$SERVICE_STOP": "stop", "$SERVICE_PAUSE": "pause", "$SERVICE_SET_FAN_MODE": "set_fan_mode", "$SERVICE_SET_HVAC_MODE": "set_hvac_mode", "$SERVICE_SET_SWING_MODE": "set_swing_mode", "$SERVICE_SET_TEMPERATURE": "set_temperature", "$SERVICE_OSCILLATE": "oscillate", "$SERVICE_SET_DIRECTION": "set_direction", "$SERVICE_SET_PERCENTAGE": "set_percentage",
          "$SERVICE_SET_PRESET_MODE": "set_preset_mode", "$SERVICE_OPEN_COVER": "open_cover", "$SERVICE_CLOSE_COVER": "close_cover",
          "$SERVICE_STOP_COVER": "stop_cover", "$SERVICE_SET_COVER_POSITION": "set_cover_position",
          "$SERVICE_OPEN_VALVE": "open", "$SERVICE_CLOSE_VALVE": "close",
          "$SERVICE_ENABLE_MOTION": "enable_motion_detection", "$SERVICE_DISABLE_MOTION": "disable_motion_detection"}
ARG = {"$ATTR_BRIGHTNESS": "brightness", "$ATTR_COLOR_TEMP_KELVIN": "color_temp_kelvin", "$ATTR_HS_COLOR": "hs_color", "$ATTR_WHITE": "white", "$ATTR_FAN_MODE": "fan_mode", "$ATTR_HVAC_MODE": "hvac_mode", "$ATTR_SWING_MODE": "swing_mode", "$ATTR_TEMPERATURE": "temperature", "$ATTR_FAN_SPEED": "fan_speed", "$ATTR_HUMIDITY": "humidity", "$ATTR_MODE": "mode", "$ATTR_OSCILLATING": "oscillating", "$ATTR_DIRECTION": "direction", "$ATTR_PERCENTAGE": "percentage", "$ATTR_PRESET_MODE": "preset_mode", "$ATTR_POSITION": "position", "$ATTR_TILT_POSITION": "tilt_position", "$ATTR_OPTION": "option", "$ATTR_VALUE": "value"}

def run_actions():
    """Replay write goldens (service -> commands) for implemented platforms."""
    bad = n = 0
    for c in json.loads((pathlib.Path(__file__).parent / "golden" / "actions_golden.json").read_text()):
        plat = c["entity_id"].split(".")[0]
        act = ACTION.get(c.get("service"))
        if plat not in PLATFORMS or act is None or "mock_device_code" not in c:
            continue
        code = c["mock_device_code"]
        gold = next((r for r in GOLD[code].get(plat, []) if r["entity_id"] == c["entity_id"]), None)
        plan = next((e for e in classify(fixture(code), ENV, (plat,)) if gold and e.key == gold["key"]), None)
        n += 1
        args = {ARG.get(k, k): v for k, v in (c.get("service_data") or {}).items()}
        args = {k: (v.split(".", 1)[1].lower() if isinstance(v, str) and v.startswith("$HVACMode.") else v) for k, v in args.items()}
        for sk in ("fan_mode", "preset_mode", "swing_mode", "option"):    # HA service schemas coerce these to str (host)
            if sk in args:
                args[sk] = str(args[sk])
        if args.get("white") is True and plan:                # HA light service: white=True -> current brightness
            args["white"] = plan.read(fixture(code)["status"])["brightness"]
        got = plan.write(act, args, fixture(code)["status"]) if plan else None
        want = c["expected_commands"]
        if plat == "cover" and plan and core_reverses(code, gold["key"]):    # the engine does not mirror (see above)
            pos = {r.code: r.spec for n, r in plan.roles.items() if n in ("set_position", "current_position")}
            want = [{**w, "value": pos[w["code"]].min + pos[w["code"]].max - w["value"]} if w["code"] in pos else w
                    for w in want]
        if got != want:
            bad += 1
            print("   ACTION MISMATCH", c["test"], c["entity_id"], got, want)
    print(f"actions replayed {n}  mismatched {bad}")
    return bad == 0

@pytest.mark.parametrize("platform", PLATFORMS)
def test_platform_matches_core(platform):
    assert run(platform)


def test_write_actions_match_core():
    assert run_actions()
