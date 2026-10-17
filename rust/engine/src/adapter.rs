use crate::EngineError;
use crate::data::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value as V, json};
/// How one local DP ID maps to a DP code. Serialized as `[code, convert, config]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "(String, String, V)", into = "(String, String, V)")]
pub struct DpEntry {
    pub code: String,
    /// The value conversion (`local_strategy.value_convert`), such as `default` or `enum`.
    pub convert: String,
    /// The conversion's `config_item`.
    pub config: V,
}
impl DpEntry {
    pub fn new(code: impl Into<String>, convert: impl Into<String>, config: V) -> Self {
        Self {
            code: code.into(),
            convert: convert.into(),
            config,
        }
    }
}
impl From<(String, String, V)> for DpEntry {
    fn from((code, convert, config): (String, String, V)) -> Self {
        Self::new(code, convert, config)
    }
}
impl From<DpEntry> for (String, String, V) {
    fn from(e: DpEntry) -> Self {
        (e.code, e.convert, e.config)
    }
}
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Adapter {
    pub entries: IndexMap<String, DpEntry>,
    pub enum_ranges: V,
    pub unsupported: V,
    pub remaps: V,
}
pub fn b64(v: &V) -> Option<Vec<u8>> {
    let s = v.as_str()?;
    let filtered: Vec<u8> = s
        .bytes()
        .filter(|c| c.is_ascii_alphanumeric() || b"+/=".contains(c))
        .collect();
    STANDARD.decode(filtered).ok()
}
pub fn hexbytes(s: &str) -> Option<Vec<u8>> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}
pub fn uint(b: &[u8]) -> f64 {
    b.iter().fold(0f64, |a, v| a * 256.0 + *v as f64)
}
pub fn electricity(raw: &[u8]) -> V {
    let signed = raw.len() == 18 && raw[..2] == [2, 15];
    let rich = signed || (raw.len() == 17 && raw[..2] == [1, 15]);
    let b = if rich { &raw[2..17] } else { raw };
    if b.len() < 8 {
        return V::Null;
    }
    let mut out = json!({"voltage":uint(&b[..2])/10.0,"current":uint(&b[2..5]),"power":uint(&b[5..8]),"reactive_power":null,"apparent_power":null,"power_factor":null});
    if rich {
        out["reactive_power"] = number(uint(&b[8..11]));
        out["apparent_power"] = number(uint(&b[11..14]));
        out["power_factor"] = number(b[14] as f64 / 100.0);
    }
    if signed {
        for (bit, k) in [
            (1, "current"),
            (2, "power"),
            (4, "reactive_power"),
            (8, "power_factor"),
        ] {
            if raw[17] & bit != 0 {
                out[k] = number(-n(&out[k]));
            }
        }
    }
    out
}
pub fn hsv_rgb(h: f64, sat: f64, val: f64) -> [f64; 3] {
    if sat == 0.0 {
        return [val; 3];
    }
    let x = h * 6.0;
    let i = x.floor() as i64;
    let f = x - i as f64;
    let p = val * (1.0 - sat);
    let q = val * (1.0 - sat * f);
    let t = val * (1.0 - sat * (1.0 - f));
    match i.rem_euclid(6) {
        0 => [val, t, p],
        1 => [q, val, p],
        2 => [p, val, t],
        3 => [p, q, val],
        4 => [t, p, val],
        _ => [val, p, q],
    }
}
pub fn rgb_hsv(r: f64, g: f64, b: f64) -> [f64; 3] {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    if delta == 0.0 {
        return [0.0, 0.0, max];
    }
    let rc = (max - r) / delta;
    let gc = (max - g) / delta;
    let bc = (max - b) / delta;
    let h = if r == max {
        bc - gc
    } else if g == max {
        2.0 + rc - bc
    } else {
        4.0 + gc - rc
    };
    [(h / 6.0).rem_euclid(1.0), delta / max, max]
}
fn hx(raw: &str, a: usize, b: usize) -> Result<f64, String> {
    let text = raw
        .get(a.min(raw.len())..b.min(raw.len()))
        .ok_or("invalid hex")?;
    if text.is_empty() {
        Ok(0.0)
    } else {
        u64::from_str_radix(text, 16)
            .map(|x| x as f64)
            .map_err(|_| "invalid hex".into())
    }
}
fn dumps(v: V) -> V {
    dumps_float(
        v,
        &[
            "voltage",
            "electricCurrent",
            "power",
            "reactivePower",
            "apparentPower",
            "powerFactor",
            "electricTotal",
        ],
    )
}
fn dumps_float(mut v: V, keep: &[&str]) -> V {
    fn normalize(v: &mut V, keep: &[&str], key: &str) {
        match v {
            V::Number(_) => {
                if !keep.contains(&key) {
                    *v = number(n(v));
                }
            }
            V::Array(xs) => {
                for x in xs {
                    normalize(x, keep, key)
                }
            }
            V::Object(o) => {
                for (k, x) in o {
                    normalize(x, keep, k)
                }
            }
            _ => {}
        }
    }
    fn encode(v: &V) -> String {
        match v {
            V::Array(xs) => format!("[{}]", xs.iter().map(encode).collect::<Vec<_>>().join(", ")),
            V::Object(o) => format!(
                "{{{}}}",
                o.iter()
                    .map(|(k, v)| format!("{}: {}", json!(k), encode(v)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            _ => v.to_string(),
        }
    }
    normalize(&mut v, keep, "");
    json!(encode(&v))
}
fn default(ci: &V) -> V {
    match s(&ci["valueType"]).to_ascii_lowercase().as_str() {
        "boolean" => json!(false),
        "integer" => parse(&ci["valueDesc"])["min"].clone(),
        "enum" => parse(&ci["valueDesc"])["range"][0].clone(),
        _ => json!(""),
    }
}
pub fn strategy_code(meta: &V) -> String {
    let fmt = parse(&meta["config_item"]["statusFormat"]);
    fmt.as_object()
        .and_then(|o| o.keys().next_back())
        .cloned()
        .unwrap_or_else(|| s(&meta["status_code"]).into())
}
fn rounded(v: f64, d: i32) -> f64 {
    // Fixed decimal formatting rounds the exact binary64 value, like Python round(x, ndigits).
    // Multiplying by 10 first can create a tie that the original value did not contain.
    format!("{v:.precision$}", precision = d as usize)
        .parse()
        .unwrap_or(v)
}

fn rgb_object(rgb: &str) -> Result<V, String> {
    let h = rgb_hsv(
        hx(rgb, 0, 2)? / 255.0,
        hx(rgb, 2, 4)? / 255.0,
        hx(rgb, 4, 6)? / 255.0,
    );
    Ok(json!({"h":rounded(h[0]*360.0,1),"s":rounded(h[1]*255.0,1),"v":rounded(h[2]*255.0,1)}))
}
pub fn strategy_read(name: &str, raw: &V, ci: &V, rules: &V) -> Result<V, String> {
    if name == "default" {
        return Ok(if raw.is_null() || raw == "" {
            default(ci)
        } else {
            raw.clone()
        });
    }
    if name == "enum" {
        let key = if raw.is_boolean() {
            if raw == true { "True" } else { "False" }.into()
        } else if raw.is_string() {
            s(raw).into()
        } else {
            raw.to_string()
        };
        let m = &ci["enumMappingMap"];
        return Ok(m
            .get(&key)
            .or_else(|| m.get(key.to_lowercase()))
            .and_then(|e| e.get("value"))
            .cloned()
            .unwrap_or_else(|| default(ci)));
    }
    if raw.is_null() {
        return Ok(match name {
            "hb_jsq_lightv1" | "hb_range_v1" | "hb_range_v2" => json!(""),
            _ => V::Null,
        });
    }
    let text = s(raw);
    match name {
        "dj_v2_color_alg" => Ok(dumps(
            json!({"h":hx(text,0,4)?,"s":hx(text,4,8)?,"v":hx(text,8,12)?}),
        )),
        "dj_v2_contr_alg" | "dj_v2_music_alg" => {
            if text.is_empty() {
                Ok(json!(""))
            } else {
                Ok(dumps(
                    json!({"change_mode":match hx(text,0,1)? as i64{0=>"direct",1=>"gradient",_=>""},"h":hx(text,1,5)?,"s":hx(text,5,9)?,"v":hx(text,9,13)?,"bright":hx(text,13,17)?,"temperature":hx(text,17,text.len())?}),
                ))
            }
        }
        "dj_v2_scene_alg" => {
            let mut units = vec![];
            for i in 0..text.len().saturating_sub(2) / 26 {
                let x = text
                    .get(2 + i * 26..2 + (i + 1) * 26)
                    .ok_or("invalid scene")?;
                units.push(json!({"unit_switch_duration":hx(x,0,2)?,"unit_gradient_duration":hx(x,2,4)?,"unit_change_mode":match hx(x,4,6)? as i64{0=>"static",1=>"jump",2=>"gradient",_=>""},"h":hx(x,6,10)?,"s":hx(x,10,14)?,"v":hx(x,14,18)?,"bright":hx(x,18,22)?,"temperature":hx(x,22,26)?}));
            }
            Ok(dumps(
                json!({"scene_num":1.0+hx(text,0,2)?,"scene_units":units}),
            ))
        }
        "dj_v1_hsv_alg" | "voice_atm_color" => {
            if text.get(6..) == Some("0168ffff") {
                return Ok(dumps_float(rgb_object(&text[..6])?, &["h", "s", "v"]));
            }
            let digits = if name == "voice_atm_color" { 4 } else { 3 };
            Ok(dumps_float(
                json!({"h":hx(text,6,10)?,"s":rounded(rounded(hx(text,10,12)?/255.0,digits)*255.0,1),"v":rounded(rounded(hx(text,12,text.len())?/255.0,digits)*255.0,1)}),
                &["h", "s", "v"],
            ))
        }
        "dj_v1_scene_alg" => {
            if text.is_empty() {
                return Ok(json!(""));
            }
            let mut hsv = vec![];
            for i in (8..text.len()).step_by(6) {
                hsv.push(rgb_object(text.get(i..i + 6).ok_or("invalid scene")?)?);
            }
            Ok(dumps_float(
                json!({"frequency":hx(text,4,6)?,"bright":hx(text,0,2)?,"temperature":hx(text,2,4)?,"hsv":hsv}),
                &["h", "s", "v"],
            ))
        }
        "hb_jsq_lightv1" => Ok(json!(format!("{text}|0|0"))),
        "hb_djv1_color" => {
            let o = parse(raw);
            let c = hsv_rgb(n(&o["h"]) / 360.0, n(&o["s"]) / 255.0, n(&o["v"]) / 255.0);
            Ok(json!(format!(
                "{}|{}|{}",
                (c[0] * 255.0) as i64,
                (c[1] * 255.0) as i64,
                (c[2] * 255.0) as i64
            )))
        }
        "hb_range_v1" | "hb_range_v2" => {
            let v = raw
                .as_f64()
                .or_else(|| text.parse().ok())
                .ok_or("invalid range")?;
            let (a, b, c, d) = if name == "hb_range_v1" {
                (25.0, 255.0, 0.0, 100.0)
            } else {
                (0.0, 255.0, 1000.0, 12000.0)
            };
            Ok(number(
                remap(v.trunc().clamp(a, b), a, b, c, d).floor().clamp(c, d),
            ))
        }
        "ms_dp_syn_alg" => {
            let b = b64(raw).ok_or("invalid base64")?;
            let (pairs, remainder) = b.as_chunks::<2>();
            if !remainder.is_empty() {
                return Err("incomplete synchronization record".into());
            }
            let mut out = vec![];
            for pair in pairs {
                for bit in 0..8 {
                    if pair[1] >> bit & 1 != 0 {
                        out.push(((pair[0] & 127) as i64 - 1) * 8 + bit);
                    }
                }
            }
            out.sort();
            out.dedup();
            Ok(json!(out))
        }
        "cz_timer1_alg" | "cz_timer2_alg" => {
            let b = b64(raw).ok_or("invalid base64")?;
            let width = if name == "cz_timer1_alg" { 6 } else { 10 };
            if b.len() % width != 0 {
                return Err("invalid timer".into());
            }
            let mut out = vec![];
            for t in b.chunks(width) {
                let time = |i| {
                    let n = t[i] as usize * 256 + t[i + 1] as usize;
                    format!("{:02}:{:02}", n / 60, n % 60)
                };
                let mut o = json!({"timer_switch":t[0]>0,"week_day":(0..8).filter(|d|t[1]>>d&1!=0).collect::<Vec<_>>(),"start_time":time(2),"end_time":time(4)});
                if width == 10 {
                    o["open_time"] = json!(time(6));
                    o["close_time"] = json!(time(8));
                }
                out.push(o);
            }
            Ok(dumps(json!(out)))
        }
        "sd_clean_record" => {
            let (rec, t, a, m) = match text.len() {
                6 => ("", text.get(..3), text.get(3..6), ""),
                11 => (
                    "",
                    text.get(..3),
                    text.get(3..6),
                    text.get(6..11).unwrap_or(""),
                ),
                18 => (
                    text.get(..12).unwrap_or(""),
                    text.get(12..15),
                    text.get(15..18),
                    "",
                ),
                _ => (
                    text.get(..12).unwrap_or(""),
                    text.get(12..15),
                    text.get(15..18),
                    text.get(18..23).unwrap_or(""),
                ),
            };
            Ok(dumps(
                json!({"record_time":rec,"clean_time":t.and_then(|x|x.parse::<i64>().ok()).ok_or("invalid clean time")?,"clean_area":a.and_then(|x|x.parse::<i64>().ok()).ok_or("invalid area")?,"map_id":m}),
            ))
        }
        "db_v1_params" => {
            let b = b64(raw).ok_or("invalid base64")?;
            let e = electricity(&b);
            if e.is_null() {
                return Err("short electricity payload".into());
            }
            let mut o = json!({"voltage":e["voltage"],"electricCurrent":n(&e["current"])/1000.0,"power":n(&e["power"])/1000.0});
            if !e["reactive_power"].is_null() {
                o["reactivePower"] = number(n(&e["reactive_power"]) / 1000.0);
                o["apparentPower"] = number(n(&e["apparent_power"]) / 1000.0);
                o["powerFactor"] = e["power_factor"].clone();
            }
            Ok(dumps(o))
        }
        "db_v1_daily" | "db_v1_month" | "db_v1_frozen" => {
            let b = b64(raw).ok_or("invalid base64")?;
            if name == "db_v1_frozen" {
                let [day, hour] = b.as_slice() else {
                    return Err("invalid frozen payload length".into());
                };
                return Ok(dumps(json!({"day":day,"hour":hour})));
            }
            let [start_major, start_minor, end_major, end_minor, total @ ..] = b.as_slice() else {
                return Err("short energy payload".into());
            };
            let total: &[u8; 4] = total
                .try_into()
                .map_err(|_| "invalid energy payload length")?;
            let keys = if name == "db_v1_daily" {
                ["startMonth", "startDay", "endMonth", "endDay"]
            } else {
                ["startYear", "startMonth", "endYear", "endMonth"]
            };
            let mut o = json!({});
            for (key, value) in
                keys.into_iter()
                    .zip([start_major, start_minor, end_major, end_minor])
            {
                o[key] = json!(value);
            }
            o["electricTotal"] = json!(uint(total) / 100.0);
            Ok(dumps(o))
        }
        "db_v1_alarm" => {
            let b = b64(raw).ok_or("invalid base64")?;
            let mut out = vec![];
            let (records, remainder) = b.as_chunks::<4>();
            if !remainder.is_empty() {
                return Err("incomplete alarm record".into());
            }
            for rec in records {
                let id = rec[0].to_string();
                let row = &rules["meter_alarms"][&id];
                if row.is_null() {
                    continue;
                }
                let mut o = json!({"alarmCode":row[0],"doAction":rec[1]==1});
                if !row[1].is_null() {
                    let scale = n(&row[1]);
                    let t = uint(&rec[2..]) / 10f64.powi(scale as i32);
                    o["threshold"] = json!(if scale > 0.0 {
                        if t.fract() == 0.0 {
                            format!("{t:.1}")
                        } else {
                            t.to_string()
                        }
                    } else {
                        (t as i64).to_string()
                    });
                }
                out.push(o);
            }
            Ok(dumps(json!(out)))
        }
        _ => Ok(raw.clone()),
    }
}
pub fn strategy_write(name: &str, v: &V, ci: &V) -> Result<V, EngineError> {
    let o = parse(v);
    let text = s(v);
    let hex = |k: &str, w: usize| format!("{:0width$x}", n(&o[k]) as i64, width = w);
    match name {
        "default" => Ok(v.clone()),
        "enum" => {
            let pairs = obj(&ci["enumMappingMap"]);
            let keys: Vec<_> = pairs
                .iter()
                .filter(|(_, e)| e["value"] == *v)
                .map(|(k, _)| k)
                .collect();
            Ok(keys
                .iter()
                .find(|k| k.bytes().all(|c| c.is_ascii_digit()))
                .or(keys.first())
                .map(|k| json!(k))
                .unwrap_or_else(|| v.clone()))
        }
        "dj_v2_color_alg" => Ok(json!(format!(
            "{}{}{}",
            hex("h", 4),
            hex("s", 4),
            hex("v", 4)
        ))),
        "dj_v2_contr_alg" | "dj_v2_music_alg" => Ok(json!(format!(
            "{}{}{}{}{}{}",
            if o["change_mode"] == "gradient" { 1 } else { 0 },
            hex("h", 4),
            hex("s", 4),
            hex("v", 4),
            hex("bright", 4),
            hex("temperature", 4)
        ))),
        "dj_v2_scene_alg" => {
            let mut out = format!("{:02x}", n(&o["scene_num"]) as i64 - 1);
            for u in arr(&o["scene_units"]) {
                out += &format!(
                    "{:02x}{:02x}{:02x}",
                    n(&u["unit_switch_duration"]) as i64,
                    n(&u["unit_gradient_duration"]) as i64,
                    match s(&u["unit_change_mode"]) {
                        "jump" => 1,
                        "gradient" => 2,
                        _ => 0,
                    }
                );
                for k in ["h", "s", "v", "bright", "temperature"] {
                    out += &format!("{:04x}", n(&u[k]) as i64);
                }
            }
            Ok(json!(out))
        }
        "dj_v1_hsv_alg" | "voice_atm_color" => {
            let h = n(&o["h"]).round_ties_even();
            let sat = n(&o["s"]).round_ties_even();
            let val = n(&o["v"]).round_ties_even();
            let rgb = hsv_rgb(h / 360.0, sat / 255.0, val / 255.0);
            Ok(json!(format!(
                "{:02x}{:02x}{:02x}{:04x}{:02x}{:02x}",
                (rgb[0] * 255.0).round_ties_even() as i64,
                (rgb[1] * 255.0).round_ties_even() as i64,
                (rgb[2] * 255.0).round_ties_even() as i64,
                h as i64,
                sat as i64,
                val as i64
            )))
        }
        "hb_jsq_lightv1" => Ok(json!(text.strip_suffix("|0|0").unwrap_or(text))),
        "cz_timer1_alg" | "cz_timer2_alg" => {
            let mut b = vec![];
            for t in arr(&o) {
                b.push(if truth(&t["timer_switch"]) { 1 } else { 0 });
                b.push(
                    items(&t["week_day"])
                        .iter()
                        .map(|d| 1u8.checked_shl(n(d) as u32).unwrap_or(0))
                        .fold(0, |bits, bit| bits | bit),
                );
                for k in if name == "cz_timer1_alg" {
                    vec!["start_time", "end_time"]
                } else {
                    vec!["start_time", "end_time", "open_time", "close_time"]
                } {
                    let parts: Vec<_> = s(&t[k]).split(':').collect();
                    if parts.len() != 2 {
                        return Err("invalid time".into());
                    }
                    let hours = parts[0].parse::<u16>().map_err(|_| "invalid time")?;
                    let minute = parts[1].parse::<u16>().map_err(|_| "invalid time")?;
                    let minutes = hours
                        .checked_mul(60)
                        .and_then(|v| v.checked_add(minute))
                        .ok_or("invalid time")?;
                    b.extend(minutes.to_be_bytes());
                }
            }
            Ok(json!(STANDARD.encode(b)))
        }
        _ => Err(EngineError::Unsupported(
            "this dp's value conversion has no inverse".into(),
        )),
    }
}
impl Adapter {
    pub fn new(device: &V, dpmap: &V, rules: &V) -> Self {
        let mut a = Self {
            enum_ranges: json!({}),
            unsupported: json!({}),
            remaps: json!({}),
            ..Default::default()
        };
        for (id, meta) in obj(&device["local_strategy"]) {
            let code = strategy_code(&meta);
            if code.is_empty() {
                continue;
            }
            let mut name = if truth(&meta["value_convert"]) {
                s(&meta["value_convert"])
            } else {
                "default"
            };
            if !items(&rules["strategies"]).contains(&json!(name)) {
                a.unsupported[&id] = json!(name);
                name = "default";
            }
            a.entries
                .insert(id, DpEntry::new(code, name, meta["config_item"].clone()));
        }
        for (id, code) in obj(dpmap) {
            a.entries
                .entry(id)
                .or_insert_with(|| DpEntry::new(s(&code), "default", json!({})));
        }
        for (code, spec) in obj(&device["status_range"]) {
            if normalized(s(&spec["type"])) == "Enum" {
                a.enum_ranges[code] = parse(&spec["values"])["range"].clone();
            }
        }
        a
    }
    pub fn flip(&self, code: &str, v: &V) -> V {
        let r = &self.remaps[code];
        if !truth(&r["invert"]) {
            return v.clone();
        }
        if v.is_boolean() {
            return json!(!truth(v));
        }
        if v.is_number() && r["bounds"].is_array() {
            return number(n(&r["bounds"][0]) + n(&r["bounds"][1]) - n(v));
        }
        v.clone()
    }
    pub fn read(&self, dps: &V, rules: &V) -> V {
        let mut out = json!({});
        for (id, raw) in fields(dps) {
            if let Some(DpEntry {
                code,
                convert,
                config,
            }) = self.entries.get(id)
                && let Ok(mut v) = strategy_read(convert, raw, config, rules)
            {
                if v.is_string()
                    && let Some(alias) = self.remaps[code]["alias"].get(s(&v))
                {
                    v = alias.clone();
                }
                v = self.flip(code, &v);
                if self.enum_ranges.get(code).is_some()
                    && !items(&self.enum_ranges[code]).contains(&v)
                {
                    continue;
                }
                out[code] = v;
            }
        }
        out
    }
    pub fn write(&self, commands: &[V]) -> Result<V, EngineError> {
        let mut out = json!({});
        for c in commands {
            let code = s(&c["code"]);
            let Some((id, entry)) = self.entries.iter().find(|(_, e)| e.code == code) else {
                return Err(EngineError::Unsupported(format!("no dp for {code}")));
            };
            let mut v = self.flip(code, &c["value"]);
            if let Some((key, _)) = fields(&self.remaps[code]["alias"]).find(|(_, std)| **std == v)
            {
                v = json!(key);
            }
            out[id] = strategy_write(&entry.convert, &v, &entry.config)?;
        }
        if !truth(&out) {
            return Err(EngineError::Unsupported("no dp for command".into()));
        }
        Ok(out)
    }
    pub fn invert(&mut self, code: &str, device: &V) {
        if self.remaps.get(code).is_none() {
            self.remaps[code] = json!({});
        }
        self.remaps[code]["invert"] = json!(!truth(&self.remaps[code]["invert"]));
        self.set_bounds(code, device);
    }
    /// An Integer DP's raw range, which inversion mirrors values within.
    pub fn set_bounds(&mut self, code: &str, device: &V) {
        let spec = if device["status_range"].get(code).is_some() {
            &device["status_range"][code]
        } else {
            &device["function"][code]
        };
        let v = parse(&spec["values"]);
        if normalized(s(&spec["type"])) == "Integer" {
            self.remaps[code]["bounds"] = json!([v["min"], v["max"]]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_bytes(name: &str, bytes: &[u8]) -> Result<V, String> {
        strategy_read(
            name,
            &json!(STANDARD.encode(bytes)),
            &json!({}),
            &json!({"meter_alarms":{"1":["overcurrent",1]}}),
        )
    }

    #[test]
    fn binary_records_reject_truncated_and_oversized_payloads() {
        for name in ["db_v1_daily", "db_v1_month"] {
            for len in 0..8 {
                assert!(read_bytes(name, &vec![0; len]).is_err(), "{name}: {len}");
            }
            assert!(read_bytes(name, &[0; 9]).is_err());
            let result = read_bytes(name, &[1, 2, 3, 4, 0, 0, 0, 123]).unwrap();
            assert_eq!(parse(&result)["electricTotal"], json!(1.23));
        }
        for len in [0, 1, 3] {
            assert!(read_bytes("db_v1_frozen", &vec![0; len]).is_err());
        }
        assert_eq!(
            parse(&read_bytes("db_v1_frozen", &[12, 23]).unwrap()),
            json!({"day":12,"hour":23})
        );
        for len in [1, 2, 3, 5, 6, 7] {
            assert!(read_bytes("db_v1_alarm", &vec![1; len]).is_err());
        }
        assert_eq!(parse(&read_bytes("db_v1_alarm", &[]).unwrap()), json!([]));
        assert_eq!(
            parse(&read_bytes("db_v1_alarm", &[1, 1, 0, 123]).unwrap()),
            json!([{"alarmCode":"overcurrent","doAction":true,"threshold":"12.3"}])
        );
        assert!(read_bytes("ms_dp_syn_alg", &[1]).is_err());
        assert_eq!(read_bytes("ms_dp_syn_alg", &[1, 3]).unwrap(), json!([0, 1]));
    }
}
