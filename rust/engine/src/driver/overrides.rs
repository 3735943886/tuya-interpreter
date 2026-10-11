//! Override blocks: validated, applied to the device before classification and to the descriptor after.
use crate::{adapter::*, assembly::*, data::*, plan::Plan};
use serde_json::{Value as V, json};

pub(super) fn prepare(device: &V, options: &V, rules: &V) -> Result<(V, V, V, V), String> {
    let mut d = device.clone();
    if d["id"].as_str().is_none() {
        return Err("device needs id".into());
    }
    ensure_tables(&mut d);
    let mut dpmap = json!({});
    let quirk = if options["use_quirks"] == false {
        &V::Null
    } else {
        &rules["quirks"][s(&d["product_id"])]
    };
    apply_quirk(&mut d, &mut dpmap, quirk);
    map_initial_status(&mut d["status"], quirk);
    let info = device_info(&d, quirk, rules);
    merge(&mut dpmap, &options["dpmap"]);
    let mut original = dpmap.clone();
    for (id, meta) in obj(&d["local_strategy"]) {
        let code = strategy_code(&meta);
        if !code.is_empty() {
            original[&id] = json!(code);
        }
    }
    let mut overrides = rules["overrides"].clone();
    if !overrides.is_object() {
        overrides = json!({});
    }
    merge(&mut overrides, &options["overrides"]);
    let mut block = json!({});
    for key in [s(&d["product_id"]), s(&d["id"])] {
        let b = &overrides[key];
        if !b.is_null() {
            validate(b, rules)?;
            merge(&mut block, b);
        }
    }
    if truth(&block["category"]) {
        d["category"] = block["category"].clone();
    }
    for (id, patch) in obj(&block["dp"]) {
        let code = s(&patch["code"]);
        if patch.get("type").is_none() {
            let old = s(&original[&id]);
            if old.is_empty() {
                return Err(format!("override: dp.{id}: no dp {id} to rename"));
            }
            for table in ["function", "status_range", "status"] {
                if let Some(v) = d[table].as_object_mut().unwrap().remove(old) {
                    d[table][code] = v;
                }
            }
            for v in dpmap.as_object_mut().unwrap().values_mut() {
                if *v == old {
                    *v = json!(code);
                }
            }
        } else {
            let op = json!({"op":"DefineDp","dpid":id,"code":code,"type":patch["type"],"mode":patch.get("mode").cloned().unwrap_or(json!("RW")),"values":patch.get("values").cloned().unwrap_or(json!({})),"report_type":patch["report_type"]});
            patch_op(&mut d, &mut dpmap, &op);
        }
    }
    for (code, remap) in obj(&block["remap"]) {
        let alias = &remap["alias"];
        for table in ["function", "status_range"] {
            if normalized(s(&d[table][&code]["type"])) == "Enum" {
                let mut spec = parse(&d[table][&code]["values"]);
                spec["range"] = json!(
                    items(&spec["range"])
                        .iter()
                        .map(|v| alias.get(s(v)).cloned().unwrap_or_else(|| v.clone()))
                        .collect::<Vec<_>>()
                );
                d[table][&code]["values"] = spec;
            }
        }
        if d["status"][&code].is_string() {
            let raw = s(&d["status"][&code]);
            if let Some(v) = alias.get(raw) {
                d["status"][&code] = v.clone();
            }
        }
    }
    for code in codes(&block["remove"]) {
        for table in ["function", "status_range", "status"] {
            d[table].as_object_mut().unwrap().remove(&code);
        }
        dpmap.as_object_mut().unwrap().retain(|_, v| *v != code);
    }
    Ok((d, block, dpmap, info))
}
fn unknown(v: &V, allowed: &[&str]) -> Result<(), String> {
    if let Some(o) = v.as_object() {
        for k in o.keys() {
            if !allowed.contains(&k.as_str()) {
                return Err(format!("override: unknown key {k}"));
            }
        }
    } else if !v.is_null() {
        return Err("override: must be object".into());
    }
    Ok(())
}
/// The converters an override block can turn on.
const CONVERTERS: [&str; 1] = ["declarative"];

fn validate(b: &V, rules: &V) -> Result<(), String> {
    unknown(
        b,
        &[
            "dp",
            "remove",
            "category",
            "props",
            "device",
            "converters",
            "remap",
            "expose_unused",
            "auto",
            "cover",
            "delta",
        ],
    )?;
    for (_, d) in obj(&b["dp"]) {
        unknown(&d, &["code", "type", "values", "mode", "report_type"])?;
        if !truth(&d["code"]) || d.get("type").is_some() && normalized(s(&d["type"])).is_empty() {
            return Err("override: invalid dp definition".into());
        }
        if d.get("type").is_none() && obj(&d).len() != 1 {
            return Err("override: a rename takes only a `code`".into());
        }
        if d.get("mode").is_some() && !["R", "W", "RW"].contains(&s(&d["mode"])) {
            return Err("override: invalid mode".into());
        }
    }
    for k in ["auto", "expose_unused"] {
        if b.get(k).is_some() && !b[k].is_boolean() {
            return Err(format!("override: {k}: true or false"));
        }
    }
    if b.get("remove").is_some()
        && (!b["remove"].is_array() || items(&b["remove"]).iter().any(|v| !v.is_string()))
    {
        return Err("override: remove needs list of codes".into());
    }
    unknown(&b["device"], &["kind", "class", "label", "vendor", "model"])?;
    for (_, p) in obj(&b["props"]) {
        let src = p.get("src").is_some();
        unknown(
            &p,
            if src {
                &[
                    "src", "type", "label", "class", "category", "unit", "series", "rw", "role",
                    "min", "max", "step", "options", "send",
                ]
            } else {
                &[
                    "label", "class", "category", "unit", "series", "rw", "role", "hide",
                ]
            },
        )?;
        if !p["role"].is_null() && !p["role"].is_string() {
            return Err("override: invalid role".into());
        }
        if p.get("rw").is_some() && (!p["rw"].is_boolean() || !src && p["rw"] != false) {
            return Err("override: rw can only be false".into());
        }
        if src {
            if !p["src"].is_string() || !truth(&p["src"]) {
                return Err("override: src needs code".into());
            }
            if p.get("type").is_some()
                && !["binary", "number", "select", "text", "trigger"].contains(&s(&p["type"]))
            {
                return Err("override: invalid property type".into());
            }
            if (p["type"] == "trigger") != p.get("send").is_some() {
                return Err("override: trigger needs send".into());
            }
        } else {
            if !p["category"].is_null()
                && p["category"] != "diagnostic"
                && p["category"] != "config"
            {
                return Err("override: invalid category".into());
            }
            if !p["series"].is_null() && p["series"] != "counter" && p["series"] != "gauge" {
                return Err("override: invalid series".into());
            }
        }
    }
    for (_, r) in obj(&b["remap"]) {
        unknown(&r, &["alias", "invert"])?;
        if r.get("invert").is_some() && !r["invert"].is_boolean() {
            return Err("override: invert needs bool".into());
        }
        if !r["alias"].is_null() && !r["alias"].is_object() {
            return Err("override: alias needs object".into());
        }
        let values: Vec<_> = obj(&r["alias"]).values().cloned().collect();
        for (i, v) in values.iter().enumerate() {
            if values[..i].contains(v) {
                return Err("override: duplicate alias".into());
            }
        }
    }
    fn cover(c: &V, rules: &V, nested: bool) -> Result<(), String> {
        if !c.is_object() {
            return Err("override: cover needs object".into());
        }
        for (k, v) in obj(c) {
            if k == "settle" {
                if !v.is_number() || n(&v) < 0.0 {
                    return Err("override: invalid settle".into());
                }
            } else if k == "state_source" {
                if !["control", "inferred", "none"].contains(&s(&v)) {
                    return Err("override: invalid state_source".into());
                }
            } else if rules["cover_defaults"].get(&k).is_some() {
                if !v.is_boolean() {
                    return Err("override: cover needs bool".into());
                }
            } else if nested && v.is_object() {
                cover(&v, rules, false)?
            } else {
                return Err(format!("override: unknown cover key {k}"));
            }
        }
        Ok(())
    }
    if b.get("cover").is_some() {
        cover(&b["cover"], rules, true)?
    }
    for name in obj(&b["converters"]).keys() {
        if !CONVERTERS.contains(&name.as_str()) {
            return Err(format!("override: unknown converter {name}"));
        }
    }
    if b.get("delta").is_some() {
        unknown(&b["delta"], &["accept_passive"])?;
        if b["delta"].get("accept_passive").is_some() && !b["delta"]["accept_passive"].is_boolean()
        {
            return Err("override: delta needs bool".into());
        }
    }
    Ok(())
}
pub(super) fn patch_descriptor(
    a: &mut Assembly,
    plans: &mut Vec<Plan>,
    block: &V,
    device: &V,
    rules: &V,
) -> Result<(), String> {
    for (prop, p) in obj(&block["props"]) {
        if prop == "available" {
            return Err("override: props.available cannot be overridden".into());
        }
        if p.get("src").is_some() {
            let code = s(&p["src"]);
            let e = device["status_range"]
                .get(code)
                .or_else(|| device["function"].get(code))
                .ok_or_else(|| format!("override: props.{prop}.src: no dp {code}"))?;
            let kind = normalized(s(&e["type"]));
            let natural = match kind {
                "Boolean" => "binary",
                "Integer" => "number",
                "Enum" => "select",
                _ => "text",
            };
            let ty = p.get("type").map(s).unwrap_or(natural);
            if ty != "trigger" && ty != "text" && ty != natural {
                return Err("override: property type does not fit dp".into());
            }
            let writable = device["function"].get(code).is_some();
            let rw = ty != "trigger" && ty != "text" && p.get("rw").map(truth).unwrap_or(writable);
            if (rw || ty == "trigger") && !writable {
                return Err("override: dp is not writable".into());
            }
            let role = Role {
                code: code.into(),
                kind: kind.into(),
                spec: parse(&e["values"]),
                report_type: V::Null,
            };
            let mut def = json!({"type":ty,"src":code});
            if rw {
                def["rw"] = json!(true)
            }
            if ty == "number" {
                for (k, raw) in [
                    ("min", role.lo()),
                    ("max", role.hi()),
                    ("step", n(&role.spec["step"])),
                ] {
                    def[k] = p
                        .get(k)
                        .cloned()
                        .unwrap_or_else(|| number(raw / role.scale()));
                }
                let unit = p.get("unit").unwrap_or(&role.spec["unit"]);
                if truth(unit) {
                    def["unit"] = unit.clone();
                }
            }
            if ty == "select" {
                def["options"] = p
                    .get("options")
                    .filter(|v| truth(v))
                    .cloned()
                    .unwrap_or_else(|| json!(role.range()));
            }
            for k in ["role", "label", "class", "category", "series"] {
                if !p[k].is_null() {
                    def[k] = p[k].clone();
                }
            }
            let idx = plans.len();
            plans.push(Plan {
                platform: "override".into(),
                key: code.into(),
                identity: json!({}),
                roles: Roles::new(),
                depends_on: vec![code.into()],
                slot_kind: None,
                update_all: false,
                config: json!({}),
            });
            a.descriptor["props"][&prop] = def;
            let trigger = ty == "trigger";
            a.bindings.insert(
                prop,
                Binding::new(
                    Source::Dp {
                        plan: idx,
                        role: Box::new(role),
                        trigger,
                        send: p["send"].clone(),
                    },
                    trigger || rw,
                ),
            );
            continue;
        }
        let Some(def) = a.descriptor["props"].get_mut(&prop) else {
            return Err(format!("override: props.{prop}: no such property"));
        };
        if truth(&p["hide"]) {
            a.descriptor["props"].as_object_mut().unwrap().remove(&prop);
            a.bindings.shift_remove(&prop);
            continue;
        }
        for k in ["label", "class", "category", "unit", "series", "role"] {
            if let Some(v) = p.get(k) {
                if v.is_null() {
                    def.as_object_mut().unwrap().remove(k);
                } else if (k == "unit" || k == "series") && def["type"] != "number" {
                    return Err(format!("override: {k} only applies to number"));
                } else {
                    def[k] = v.clone();
                }
            }
        }
        if p["rw"] == false {
            if def["type"] == "trigger" || def["type"] == "event" {
                return Err("override: trigger/event cannot be made read only".into());
            }
            def.as_object_mut().unwrap().remove("rw");
            if items(&rules["rw_roles"]).contains(&def["role"]) {
                def.as_object_mut().unwrap().remove("role");
            }
            if let Some(b) = a.bindings.get_mut(&prop) {
                b.writable = false;
            }
        }
    }
    for (k, v) in obj(&block["device"]) {
        if v.is_null() {
            a.descriptor.as_object_mut().unwrap().remove(&k);
        } else {
            a.descriptor[&k] = v;
        }
    }
    let used: Vec<_> = obj(&a.descriptor["props"])
        .values()
        .map(|d| d["group"].clone())
        .collect();
    if let Some(groups) = a.descriptor.get_mut("groups").and_then(V::as_object_mut) {
        groups.retain(|k, _| used.contains(&json!(k)));
        if groups.is_empty() {
            a.descriptor.as_object_mut().unwrap().remove("groups");
        }
    }
    Ok(())
}
