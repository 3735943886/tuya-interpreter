use crate::DpKind;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as V, json};
pub type Obj = Map<String, V>;
pub type Roles = IndexMap<String, Role>;
pub fn s(v: &V) -> &str {
    v.as_str().unwrap_or("")
}
pub fn n(v: &V) -> f64 {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
        .unwrap_or_else(|| if v == true { 1.0 } else { 0.0 })
}
pub fn truth(v: &V) -> bool {
    match v {
        V::Null => false,
        V::Bool(b) => *b,
        V::Number(_) => n(v) != 0.0,
        V::String(x) => !x.is_empty(),
        V::Array(a) => !a.is_empty(),
        V::Object(o) => !o.is_empty(),
    }
}
pub fn eq(a: &V, b: &V) -> bool {
    if (a.is_number() || a.is_boolean()) && (b.is_number() || b.is_boolean()) {
        n(a) == n(b)
    } else {
        a == b
    }
}
pub fn items(v: &V) -> &[V] {
    v.as_array().map(Vec::as_slice).unwrap_or_default()
}
pub fn arr(v: &V) -> Vec<V> {
    v.as_array().cloned().unwrap_or_default()
}
pub fn fields(v: &V) -> impl Iterator<Item = (&String, &V)> {
    v.as_object().into_iter().flat_map(|o| o.iter())
}
pub fn obj(v: &V) -> Obj {
    v.as_object().cloned().unwrap_or_default()
}
pub fn codes(v: &V) -> Vec<String> {
    if v.is_string() {
        vec![s(v).into()]
    } else {
        items(v).iter().map(|x| s(x).into()).collect()
    }
}
pub fn number(x: f64) -> V {
    if !x.is_finite() {
        V::Null
    } else if x.fract() == 0.0 && x.abs() < 9.22e18 {
        json!(x as i64)
    } else {
        json!(x)
    }
}
pub fn parse(v: &V) -> V {
    if let Some(x) = v.as_str() {
        serde_json::from_str(x).unwrap_or(V::Null)
    } else {
        v.clone()
    }
}
pub fn normalized(t: &str) -> &str {
    DpKind::parse(t).map(DpKind::as_str).unwrap_or("")
}
pub fn merge(dst: &mut V, src: &V) {
    if let (Some(d), Some(sr)) = (dst.as_object_mut(), src.as_object()) {
        for (k, v) in sr {
            let target = d.entry(k.clone()).or_insert(V::Null);
            if target.is_object() && v.is_object() {
                merge(target, v);
            } else {
                *target = v.clone();
            }
        }
    }
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Role {
    pub code: String,
    pub kind: DpKind,
    pub spec: V,
    pub report_type: V,
}
impl Role {
    pub fn lo(&self) -> f64 {
        n(&self.spec["min"])
    }
    pub fn hi(&self) -> f64 {
        n(&self.spec["max"])
    }
    pub fn scale(&self) -> f64 {
        10f64.powi(n(&self.spec["scale"]) as i32)
    }
    pub fn range(&self) -> Vec<V> {
        arr(&self.spec["range"])
    }
    /// The role as rule expressions see it, with limits already scaled.
    pub fn view(&self) -> V {
        json!({
            "code": self.code,
            "kind": self.kind,
            "spec": self.spec,
            "report_type": self.report_type,
            "range": self.range(),
            "lo": number(self.lo() / self.scale()),
            "hi": number(self.hi() / self.scale()),
            "step": number(n(&self.spec["step"]) / self.scale()),
            "scale": number(self.scale()),
        })
    }
    pub fn read(&self, st: &V) -> V {
        let raw = &st[&self.code];
        match self.kind {
            // Python equality accepts 0 and 1; the read value is always a canonical boolean.
            DpKind::Boolean => {
                if !raw.is_null() && (eq(raw, &json!(true)) || eq(raw, &json!(false))) {
                    json!(eq(raw, &json!(true)))
                } else {
                    V::Null
                }
            }
            DpKind::Enum => {
                if items(&self.spec["range"]).contains(raw) {
                    raw.clone()
                } else {
                    V::Null
                }
            }
            DpKind::Integer => {
                if (raw.is_i64() || raw.is_u64() || raw.is_boolean())
                    && n(raw) >= self.lo()
                    && n(raw) <= self.hi()
                {
                    json!(if self.spec["inverted"] == true {
                        self.hi() / self.scale() - n(raw) / self.scale()
                    } else {
                        n(raw) / self.scale()
                    })
                } else {
                    V::Null
                }
            }
            _ => raw.clone(),
        }
    }
    pub fn write(&self, v: &V) -> Result<V, String> {
        match self.kind {
            DpKind::Boolean => {
                if v.is_boolean() {
                    Ok(v.clone())
                } else {
                    Err("invalid boolean value".into())
                }
            }
            DpKind::Enum => {
                if v.is_string() && items(&self.spec["range"]).contains(v) {
                    Ok(v.clone())
                } else {
                    Err("invalid enum value".into())
                }
            }
            DpKind::Integer => {
                if !v.is_number() && !v.is_boolean() {
                    return Err("invalid numeric value".into());
                }
                let input = if self.spec["inverted"] == true {
                    self.hi() / self.scale() - n(v)
                } else {
                    n(v)
                };
                let x = (input * self.scale()).round_ties_even();
                if x < self.lo() || x > self.hi() {
                    Err("value out of range".into())
                } else {
                    Ok(number(x))
                }
            }
            _ => Ok(v.clone()),
        }
    }
}
pub fn remap(v: f64, a: f64, b: f64, c: f64, d: f64) -> f64 {
    (v - a) / (b - a) * (d - c) + c
}
pub fn resolve(
    device: &V,
    candidates: &[String],
    kind: &str,
    ff: bool,
) -> Result<Option<Role>, String> {
    for code in candidates {
        for table in if ff {
            ["function", "status_range"]
        } else {
            ["status_range", "function"]
        } {
            let e = &device[table][code];
            if normalized(s(&e["type"])) != kind {
                continue;
            }
            let mut spec = parse(&e["values"]);
            if matches!(kind, "Integer" | "Enum" | "Bitmap") && !truth(&spec) {
                continue;
            }
            if kind == "Integer"
                && ["min", "max", "scale", "step"]
                    .iter()
                    .any(|k| spec[*k].is_null())
            {
                return Err(format!("schema {code}: missing integer field"));
            }
            if kind == "Integer" {
                for field in ["min", "max", "scale", "step"] {
                    let v = &spec[field];
                    let integer = if let Some(text) = v.as_str() {
                        text.trim().parse::<i64>().ok()
                    } else if v.is_number() || v.is_boolean() {
                        Some(n(v).trunc() as i64)
                    } else {
                        None
                    };
                    spec[field] = json!(
                        integer.ok_or_else(|| format!("schema {code}: invalid integer {field}"))?
                    );
                }
            }
            return Ok(Some(Role {
                code: code.clone(),
                kind: DpKind::parse(kind).ok_or("invalid dp kind")?,
                spec,
                report_type: device["status_range"][code]["report_type"].clone(),
            }));
        }
    }
    Ok(None)
}
/// A description's public fields (`$`-prefixed ones are generator metadata) over `defaults`.
pub fn identity(d: &V, defaults: &V) -> V {
    let mut o = obj(d);
    o.retain(|k, _| !k.starts_with('$'));
    for (k, v) in fields(defaults) {
        o.entry(k.clone()).or_insert_with(|| v.clone());
    }
    V::Object(o)
}
pub fn cond(c: &V, status: &V) -> V {
    let Some(o) = c.as_object() else {
        return c.clone();
    };
    let Some((k, v)) = o.iter().next() else {
        return V::Null;
    };
    match k.as_str() {
        "status" => status[s(v)].clone(),
        "is_int" => {
            let x = cond(v, status);
            json!(x.is_i64() || x.is_u64() || x.is_boolean())
        }
        "ge" => {
            let a = cond(&v[0], status);
            let b = cond(&v[1], status);
            json!(
                (a.is_number() || a.is_boolean())
                    && (b.is_number() || b.is_boolean())
                    && n(&a) >= n(&b)
            )
        }
        "and" => json!(items(v).iter().all(|x| truth(&cond(x, status)))),
        "or" => json!(items(v).iter().any(|x| truth(&cond(x, status)))),
        "not" => json!(!truth(&cond(v, status))),
        _ => V::Null,
    }
}
pub fn patch_op(device: &mut V, dpmap: &mut V, op: &V) {
    let code = s(&op["code"]);
    match s(&op["op"]) {
        "SetCategory" => device["category"] = op["category"].clone(),
        "DefineDp" => {
            for (t, m) in [("function", "W"), ("status_range", "R")] {
                if s(&op["mode"]).contains(m) {
                    let mut spec = json!({"type":op["type"],"values":op["values"]});
                    if t == "status_range" {
                        spec["report_type"] = op["report_type"].clone();
                    }
                    device[t][code] = spec;
                } else {
                    device[t].as_object_mut().unwrap().remove(code);
                }
            }
            let id = if op["dpid"].is_string() {
                s(&op["dpid"]).to_string()
            } else {
                op["dpid"].to_string()
            };
            dpmap[&id] = json!(code);
        }
        "TypeOverride" => {
            device["type_overrides"][code] = op["as"].clone();
        }
        "RemoveDp" => {
            for t in ["function", "status_range", "status"] {
                device[t].as_object_mut().unwrap().remove(code);
            }
            let id = op["dpid"].to_string();
            dpmap.as_object_mut().unwrap().remove(id.trim_matches('"'));
        }
        _ => {}
    }
}

/// Give a device record its DP tables, so patches can assume objects.
pub fn ensure_tables(d: &mut V) {
    for t in ["function", "status_range", "status"] {
        if !d[t].is_object() {
            d[t] = json!({});
        }
    }
}

/// Apply a quirk's schema patches whose `when` holds; DP id definitions land in `dpmap`.
pub fn apply_quirk(d: &mut V, dpmap: &mut V, quirk: &V) {
    for op in items(&quirk["ops"]) {
        if op.get("when").is_none() || truth(&cond(&op["when"], &d["status"])) {
            patch_op(d, dpmap, op);
        }
    }
}

/// A quirk's initial-status mappings (`MapInitialStatus`), first matching pair per DP.
pub fn map_initial_status(status: &mut V, quirk: &V) {
    for op in items(&quirk["ops"])
        .iter()
        .filter(|op| op["op"] == "MapInitialStatus")
    {
        let code = s(&op["code"]);
        if let Some(pair) = items(&op["mapping"])
            .iter()
            .find(|p| eq(&status[code], &p[0]))
        {
            status[code] = pair[1].clone();
        }
    }
}

/// Vendor and model: the quirk's metadata when it names a manufacturer, else the record's.
pub fn device_info(device: &V, quirk: &V, rules: &V) -> V {
    let meta = &quirk["meta"];
    if truth(&meta["manufacturer"]) {
        json!({"manufacturer":meta["manufacturer"],"model":meta["model"],"model_id":meta["model_id"]})
    } else {
        json!({"manufacturer":rules["constants"]["manufacturer"],"model":device["product_name"],"model_id":device["product_id"]})
    }
}
/// The command words a cover instruction role uses: the first named map whose open and close
/// words it accepts, else the first map.
pub fn cover_words(role: &Role, names: &V, rules: &V) -> V {
    if role.kind == DpKind::Boolean {
        return json!({"open":true,"close":false,"stop":{"never":true}});
    }
    let range = role.range();
    let maps = codes(names);
    maps.iter()
        .map(|name| &rules["maps"][name])
        .find(|words| range.contains(&words["open"]) && range.contains(&words["close"]))
        .or_else(|| maps.first().map(|name| &rules["maps"][name]))
        .cloned()
        .unwrap_or(V::Null)
}
