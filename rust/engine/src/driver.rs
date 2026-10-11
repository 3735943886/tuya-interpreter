use crate::entity;
use crate::machine::Machine;
use crate::{adapter::*, assembly::*, data::*, plan::Plan};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value as V, json};
mod check;
mod cover;
mod overrides;
pub use check::check;
use cover::{Cover, Motion, make_covers, own};
use overrides::{patch_descriptor, prepare};
#[derive(Clone, Serialize, Deserialize)]
pub struct Driver {
    pub device: V,
    pub block: V,
    pub plans: Vec<Plan>,
    pub assembly: Assembly,
    pub adapter: Adapter,
    pub covers: Vec<Cover>,
    pub motions: Vec<Motion>,
    pub delta: Option<V>,
    pub switches: IndexMap<String, (i64, String)>,
    pub own_blocks: Vec<V>,
    pub timers: Vec<String>,
    pub codes: V,
    pub dps: V,
    pub values: V,
    pub described: bool,
    pub linked: bool,
    pub synced: bool,
    pub seq: i64,
    pub machines: Vec<Machine>,
    /// The registered rule pack the driver was made with (empty for inline rules).
    #[serde(default)]
    pub rules_id: String,
    /// Ordered rule content identity; driver state is specific to this pack.
    pub rules_fingerprint: String,
}
impl Driver {
    pub fn new(device: &V, options: &V, rules: &V) -> Result<Self, String> {
        if rules["version"] != 1 {
            return Err("unsupported rule version".into());
        }
        let (d, block, dpmap, info) = prepare(device, options, rules)?;
        let mut adapter = Adapter::new(&d, &dpmap, rules);
        for (id, patch) in obj(&block["dp"]) {
            let code = s(&patch["code"]).to_string();
            if patch.get("type").is_none() {
                if let Some(e) = adapter.entries.get_mut(&id) {
                    e.0 = code;
                }
            } else if adapter.entries.get(&id).is_none_or(|e| e.0 != code) {
                adapter
                    .entries
                    .insert(id, (code, "default".into(), json!({})));
            }
        }
        let removed = codes(&block["remove"]);
        adapter.entries.retain(|_, e| !removed.contains(&e.0));
        for (code, r) in obj(&block["remap"]) {
            adapter.remaps[&code] = r;
            let e = if d["status_range"].get(&code).is_some() {
                &d["status_range"][&code]
            } else {
                &d["function"][&code]
            };
            let spec = parse(&e["values"]);
            if normalized(s(&e["type"])) == "Integer" {
                adapter.remaps[&code]["bounds"] = json!([spec["min"], spec["max"]]);
            }
        }
        let env = options
            .get("env")
            .cloned()
            .unwrap_or_else(|| json!({"temperature_unit":"C","allowed_units":rules["host_units"]}));
        let allow = truth(&options["allow_hazardous"]);
        let settings = truth(&options["device_settings"]);
        let mut plans = if block["auto"] == false {
            vec![]
        } else {
            entity::classify(&d, &env, rules, &V::Null)?
        };
        if block["auto"] != false
            && truth(
                block
                    .get("expose_unused")
                    .unwrap_or(&options["expose_unused"]),
            )
        {
            entity::unused(&d, &adapter, &mut plans, rules)?;
        }
        let mut assembly = assemble(&d, &mut plans, &info, allow, rules)?;
        let mut covers = make_covers(&assembly, &plans, &block, settings, rules);
        let mut selected = d.clone();
        let choices: Vec<_> = covers.iter().map(|c| c.position_choice).collect();
        let mut dropped = false;
        for c in &covers {
            if truth(&c.settings["position_from_target"])
                && c.separate()
                && let Some(r) = &c.cur
            {
                for t in ["function", "status_range", "status"] {
                    selected[t].as_object_mut().unwrap().remove(&r.code);
                }
                dropped = true;
            }
        }
        if dropped {
            plans = entity::classify(&selected, &env, rules, &V::Null)?;
            assembly = assemble(&selected, &mut plans, &info, allow, rules)?;
            covers = make_covers(&assembly, &plans, &block, settings, rules);
        }
        for (i, c) in covers.iter_mut().enumerate() {
            c.position_choice = choices.get(i).copied().unwrap_or(false);
            for (k, r) in [
                ("invert_position", c.cur.as_ref()),
                (
                    "invert_set_position",
                    if c.separate() { c.setp.as_ref() } else { None },
                ),
                ("invert_tilt", c.tilt.as_ref()),
            ] {
                if truth(&c.settings[k])
                    && let Some(r) = r
                {
                    adapter.invert(&r.code, &d);
                }
            }
            if truth(&c.settings["invert_control"])
                && c.applies("invert_control")
                && let Some(r) = &c.ins
            {
                if r.kind == "Boolean" {
                    adapter.invert(&r.code, &d);
                } else {
                    let words = cover_words(r, &c.words, rules);
                    let swaps = if r.range().contains(&words["open"])
                        && r.range().contains(&words["close"])
                    {
                        json!({s(&words["open"]):words["close"],s(&words["close"]):words["open"]})
                    } else {
                        json!({})
                    };
                    let old = obj(&adapter.remaps[&r.code]["alias"]);
                    let mut alias = json!({});
                    let mut devs: Vec<String> = old.keys().cloned().collect();
                    for k in obj(&swaps).keys() {
                        if !old.values().any(|v| *v == k.as_str()) {
                            devs.push(k.clone());
                        }
                    }
                    for dev in devs {
                        let std = old.get(&dev).cloned().unwrap_or(json!(dev));
                        let new = swaps.get(s(&std)).cloned().unwrap_or(std);
                        if new != dev {
                            alias[&dev] = new;
                        }
                    }
                    if adapter.remaps.get(&r.code).is_none() {
                        adapter.remaps[&r.code] = json!({});
                    }
                    adapter.remaps[&r.code]["alias"] = alias;
                }
            }
        }
        for (k, v) in obj(&block["cover"]) {
            if v.is_object()
                && !covers
                    .iter()
                    .any(|c| c.group.as_deref() == Some(k.as_str()))
            {
                return Err(format!(
                    "override: cover.{k}: the device has no cover in a group of that name"
                ));
            }
        }
        let mut taken = vec![];
        let mut machines = vec![];
        if let Some(cfg) = block["converters"].get("declarative") {
            let index = machines.len();
            for (prop, def) in obj(&cfg["props"]) {
                if prop == "available" || taken.contains(&prop) {
                    return Err("override: converter property taken".into());
                }
                taken.push(prop.clone());
                assembly.descriptor["props"][&prop] = def.clone();
                let writable = truth(&def["rw"]) || def["type"] == "trigger";
                if writable && cfg["write"].get(&prop).is_none() {
                    return Err(
                        "override: writable declarative property needs write program".into(),
                    );
                }
                assembly
                    .bindings
                    .insert(prop, Binding::new(Source::Machine { index }, writable));
            }
            machines.push(Machine {
                config: cfg.clone(),
                state: cfg.get("initial_state").cloned().unwrap_or(json!({})),
            });
        }
        let mut motions = vec![];
        let motion_taken = taken.clone();
        for c in &covers {
            let prop = format!("{}cover_state", c.prefix);
            if taken.contains(&prop) {
                continue;
            }
            let Some(source) = c.source().filter(|source| match *source {
                "control" => c.ins.is_some(),
                "inferred" => c.can_infer(),
                "none" => c.cur.is_some(),
                _ => false,
            }) else {
                continue;
            };
            let words = c
                .ins
                .as_ref()
                .map(|r| cover_words(r, &c.words, rules))
                .unwrap_or_else(|| rules["maps"][s(&c.words[0])].clone());
            motions.push(Motion {
                prop,
                group: c.group.clone(),
                command: c.ins.as_ref().map(|r| r.code.clone()),
                target_code: if c.separate() {
                    c.setp.as_ref().map(|r| r.code.clone())
                } else {
                    None
                },
                position: c.cur.as_ref().map(|r| r.code.clone()),
                words,
                settle: n(&c.settings["settle"]),
                mode: source.into(),
                invert_report: truth(&c.settings["invert_reported_motion"]),
                ..Motion::default()
            });
        }
        for motion in &motions {
            let mut def = rules["motion_property"].clone();
            if let Some(g) = &motion.group {
                def["group"] = json!(g);
            }
            assembly.descriptor["props"][&motion.prop] = def;
            assembly
                .bindings
                .insert(motion.prop.clone(), Binding::new(Source::Motion, false));
        }
        let delta = if assembly.bindings.values().any(|b| {
            b.plan()
                .and_then(|i| plans.get(i))
                .is_some_and(|p| p.slot_kind.as_deref() == Some("delta"))
        }) {
            let mut cfg = json!({"accept_passive":false});
            merge(&mut cfg, &block["delta"]);
            Some(cfg)
        } else {
            None
        };
        let mut switches = IndexMap::new();
        if settings {
            for (i, c) in covers.iter_mut().enumerate() {
                for (k, label) in obj(&rules["cover_switches"]) {
                    if !c.applies(&k)
                        || ["state_source", "invert_reported_motion"].contains(&k.as_str())
                            && motion_taken.contains(&format!("{}cover_state", c.prefix))
                    {
                        continue;
                    }
                    let prop = format!("{}cover_{k}", c.prefix);
                    if assembly.descriptor["props"].get(&prop).is_some() {
                        continue;
                    }
                    let mut def = json!({"type":"binary","category":"config","label":label});
                    if !c.hazardous {
                        def["rw"] = json!(true);
                    }
                    if k == "state_source" {
                        def["type"] = json!("select");
                        def["options"] = if c.ins.is_some() {
                            json!(["control", "inferred", "none"])
                        } else {
                            json!(["inferred", "none"])
                        };
                    }
                    if let Some(g) = &c.group {
                        def["group"] = json!(g);
                    }
                    assembly.descriptor["props"][&prop] = def;
                    c.switches.push(k.clone());
                    switches.insert(prop, (i as i64, k));
                }
            }
            if delta.is_some() {
                let prop = "delta_accept_passive";
                if assembly.descriptor["props"].get(prop).is_none() {
                    assembly.descriptor["props"][prop] = json!({"type":"binary","rw":true,"category":"config","label":"Count passive reports"});
                    switches.insert(prop.into(), (-1, "accept_passive".into()));
                }
            }
        }
        patch_descriptor(&mut assembly, &mut plans, &block, &selected, rules)?;
        switches.retain(|k, _| assembly.descriptor["props"].get(k).is_some());
        let own_blocks = [s(&device["product_id"]), s(&device["id"])]
            .iter()
            .map(|k| options["overrides"][*k].clone())
            .collect();
        Ok(Self {
            device: d,
            block,
            plans,
            assembly,
            adapter,
            covers,
            motions,
            delta,
            switches,
            own_blocks,
            timers: vec![],
            codes: json!({}),
            dps: json!({}),
            values: json!({}),
            described: false,
            linked: false,
            synced: false,
            seq: 0,
            machines,
            rules_id: String::new(),
            rules_fingerprint: String::new(),
        })
    }
    /// Take over what `old` counted: delta totals of the same DPs, and the message sequence.
    pub fn carry(&mut self, old: &Driver) {
        self.seq = self.seq.max(old.seq);
        let is_delta =
            |p: Option<&Plan>| p.is_some_and(|p| p.slot_kind.as_deref() == Some("delta"));
        for (name, b) in self.assembly.bindings.iter_mut() {
            let Some(ob) = old.assembly.bindings.get(name) else {
                continue;
            };
            let a = b.plan().and_then(|i| self.plans.get(i));
            let o = ob.plan().and_then(|i| old.plans.get(i));
            if is_delta(a) && is_delta(o) && a.map(|p| &p.depends_on) == o.map(|p| &p.depends_on) {
                b.total = ob.total;
                b.last_ts = ob.last_ts.clone();
            }
        }
    }

    pub fn set(&mut self, prop: &str, value: V, out: &mut Vec<V>) {
        if value.is_null() {
            if self.values.as_object_mut().unwrap().remove(prop).is_some() {
                out.push(json!({"type":"absent","prop":prop}));
            }
        } else if self.values.get(prop).is_none_or(|v| !eq(v, &value)) {
            self.values[prop] = value.clone();
            out.push(json!({"type":"value","prop":prop,"value":value}));
        }
    }
    pub fn describe(&mut self) -> Vec<V> {
        if self.described {
            return vec![];
        }
        self.described = true;
        let mut out = vec![json!({"type":"descriptor","desc":self.assembly.descriptor})];
        for m in &self.motions {
            if m.mode != "inferred" && self.assembly.descriptor["props"].get(&m.prop).is_some() {
                out.push(json!({"type":"absent","prop":m.prop}));
            }
        }
        for (prop, (idx, key)) in self.switches.clone() {
            let v = if idx < 0 {
                self.delta.as_ref().unwrap()[&key].clone()
            } else {
                self.covers[idx as usize].value(&key)
            };
            self.set(&prop, v, &mut out);
        }
        out
    }
    pub fn settings_block(&self) -> V {
        let mut out = json!({});
        for c in &self.covers {
            let mut base = c.defaults.clone();
            merge(&mut base, &own(&self.own_blocks[0], &c.group));
            let had = own(&self.own_blocks[1], &c.group);
            let mut vals = json!({});
            for (k, v) in obj(&c.settings) {
                if v != base[&k] || had.get(&k).is_some() {
                    vals[&k] = v;
                }
            }
            if truth(&vals) {
                if !out["cover"].is_object() {
                    out["cover"] = json!({})
                }
                if let Some(g) = &c.group {
                    out["cover"][g] = vals
                } else {
                    merge(&mut out["cover"], &vals)
                }
            }
        }
        if let Some(delta) = &self.delta {
            let mut base = json!({"accept_passive":false});
            merge(&mut base, &self.own_blocks[0]["delta"]);
            let mut vals = json!({});
            for (k, v) in obj(delta) {
                if base[&k] != v || self.own_blocks[1]["delta"].get(&k).is_some() {
                    vals[&k] = v;
                }
            }
            if truth(&vals) {
                out["delta"] = vals;
            }
        }
        out
    }
    pub fn converted(&mut self, idx: usize, result: &V, out: &mut Vec<V>) {
        for (prop, v) in obj(&result["values"]) {
            if self
                .assembly
                .bindings
                .get(&prop)
                .is_some_and(|b| b.plan().is_none())
            {
                self.set(&prop, v, out);
            }
        }
        for (name, after) in obj(&result["timers"]) {
            let full = format!("c{idx}:{name}");
            if after.is_null() {
                if self.timers.contains(&full) {
                    self.timers.retain(|n| *n != full);
                    out.push(json!({"type":"cancel_timer","name":full}));
                }
            } else {
                if !self.timers.contains(&full) {
                    self.timers.push(full.clone());
                }
                out.push(json!({"type":"set_timer","name":full,"after":after}));
            }
        }
    }
    pub fn handle(&mut self, now: f64, input: &V, rules: &V) -> Result<Vec<V>, String> {
        let mut out = vec![];
        match s(&input["type"]) {
            "connected" => {
                self.linked = true;
                out = self.describe();
            }
            "describe" => {
                out = self.describe();
                for seed in items(&input["seed"]) {
                    out.extend(self.handle(now, seed, rules)?);
                }
                if !self.synced {
                    self.set("available", json!(false), &mut out);
                }
            }
            "disconnected" => {
                self.linked = false;
                self.synced = false;
                self.codes = json!({});
                self.dps = json!({});
                let old = std::mem::replace(&mut self.values, json!({}));
                for (prop, v) in obj(&old) {
                    if self.switches.contains_key(&prop) {
                        self.values[&prop] = v;
                    } else if prop != "available" {
                        out.push(json!({"type":"absent","prop":prop}));
                    }
                }
                self.values["available"] = json!(false);
                out.push(json!({"type":"value","prop":"available","value":false}));
                for machine in &mut self.machines {
                    machine.invoke("reset", &json!({}))?;
                }
                for m in &mut self.motions {
                    m.reset();
                }
                let mut timers = std::mem::take(&mut self.timers);
                timers.sort();
                for name in timers {
                    out.push(json!({"type":"cancel_timer","name":name}));
                }
            }
            "message" => {
                out = self.describe();
                let mut dps = input["json"].clone();
                if !dps.is_object() {
                    dps = json!({});
                }
                let t = dps.as_object_mut().unwrap().remove("t").unwrap_or(V::Null);
                if truth(&dps) {
                    merge(&mut self.dps, &dps);
                    let new = self.adapter.read(&dps, rules);
                    let pushed = input["channel"] == "active";
                    let moved: Vec<_> = obj(&new)
                        .iter()
                        .filter(|(c, v)| self.codes.get(*c).is_none_or(|old| !eq(old, v)))
                        .map(|(c, _)| c.clone())
                        .collect();
                    let first = !self.synced;
                    let (active, changed) = if pushed || first {
                        (pushed, obj(&new).keys().cloned().collect::<Vec<_>>())
                    } else if input["channel"] == "passive" {
                        (!moved.is_empty(), moved)
                    } else {
                        (false, moved)
                    };
                    self.seq += 1;
                    let ts = if t.is_i64() || t.is_boolean() {
                        t
                    } else {
                        json!(self.seq)
                    };
                    merge(&mut self.codes, &new);
                    if first {
                        self.synced = true;
                        self.linked = true;
                        self.set("available", json!(true), &mut out);
                    }
                    for idx in 0..self.machines.len() {
                        let result=self.machines[idx].invoke("update",&json!({"now":now,"codes":self.codes,"changed":changed,"active":active}))?;
                        self.converted(idx, &result, &mut out);
                    }
                    for idx in 0..self.motions.len() {
                        let (vals, timer) =
                            self.motions[idx].update(&self.codes, &changed, active, false);
                        let timers = if let Some(t) = timer {
                            json!({"settle":t})
                        } else {
                            json!({})
                        };
                        self.converted(
                            self.machines.len() + idx,
                            &json!({"values":vals,"timers":timers}),
                            &mut out,
                        );
                    }
                    let counted = pushed
                        || (input["channel"] == "passive"
                            && self
                                .delta
                                .as_ref()
                                .is_some_and(|d| truth(&d["accept_passive"])));
                    let keys: Vec<_> = self.assembly.bindings.keys().cloned().collect();
                    for name in keys {
                        let b = self.assembly.bindings.get_mut(&name).unwrap();
                        let Some(p) = b.plan().and_then(|i| self.plans.get(i)) else {
                            continue;
                        };
                        if p.slot_kind.as_deref() == Some("event") {
                            if pushed && p.depends_on.first().is_some_and(|c| changed.contains(c)) {
                                let ev = p.read(&self.codes, 0.0, rules)?["event"].clone();
                                if truth(&ev)
                                    && items(&self.assembly.descriptor["props"][&name]["options"])
                                        .contains(&ev[0])
                                {
                                    out.push(json!({"type":"event","prop":name,"kind":ev[0]}));
                                }
                            }
                            continue;
                        }
                        if !b.readable(&self.plans, rules) {
                            continue;
                        }
                        let mut update = first
                            || p.update_all
                            || p.depends_on.iter().any(|c| changed.contains(c));
                        if p.slot_kind.as_deref() == Some("delta") {
                            let code = &p.depends_on[0];
                            let raw = &self.codes[code];
                            let added = counted
                                && new.get(code).is_some()
                                && ts != b.last_ts
                                && !raw.is_null();
                            if added {
                                let v = raw
                                    .as_f64()
                                    .or_else(|| s(raw).parse().ok())
                                    .unwrap_or(n(raw));
                                b.total += v;
                                b.last_ts = ts.clone();
                            }
                            update = added || first;
                        }
                        if update {
                            let v = b.read(
                                &self.plans,
                                &self.codes,
                                rules,
                                &self.assembly.descriptor["props"][&name],
                            )?;
                            self.set(&name, v, &mut out);
                        }
                    }
                }
            }
            "timer" => {
                let name = s(&input["name"]);
                if self.timers.contains(&name.to_string()) {
                    self.timers.retain(|n| n != name);
                    if let Some((head, short)) = name.split_once(':')
                        && let Some(idx) =
                            head.strip_prefix('c').and_then(|x| x.parse::<usize>().ok())
                    {
                        if idx < self.machines.len() {
                            let result = self.machines[idx].invoke(
                                "timer",
                                &json!({"now":now,"name":short,"codes":self.codes}),
                            )?;
                            self.converted(idx, &result, &mut out);
                        } else if let Some(m) = self.motions.get_mut(idx - self.machines.len())
                            && short == "settle"
                        {
                            let (vals, _) = m.update(&self.codes, &[], false, true);
                            self.converted(idx, &json!({"values":vals,"timers":{}}), &mut out);
                        }
                    }
                }
            }
            "command" => {
                let prop = s(&input["prop"]);
                match check(
                    &self.assembly.descriptor,
                    &self.values,
                    prop,
                    &input["value"],
                ) {
                    Err((code, reason)) => {
                        out.push(json!({"type":"reject","prop":prop,"code":code,"reason":reason}))
                    }
                    Ok(value) => {
                        if let Some((idx, key)) = self.switches.get(prop).cloned() {
                            if idx < 0 {
                                self.delta.as_mut().unwrap()[&key] = value.clone();
                            } else {
                                self.covers[idx as usize].settings[&key] = value.clone();
                            }
                            self.set(prop, value, &mut out);
                            out.push(
                                json!({"type":"settings_changed","block":self.settings_block()}),
                            );
                        } else if let Some(b) = self.assembly.bindings.get(prop) {
                            if !b.writable {
                                out.push(reject(prop, "unsupported", "no write path"));
                            } else if !self.linked {
                                out.push(reject(prop, "unavailable", "device link is down"));
                            } else {
                                match &b.source {
                                    Source::Machine { index } => {
                                        let result = self.machines.get_mut(*index).ok_or("invalid machine index")?.invoke("write",&json!({"prop":prop,"value":value,"codes":self.codes,"now":now}))?;
                                        out.push(
                                            match self.adapter.write(&arr(&result["commands"])) {
                                                Ok(dps) => send(dps),
                                                Err(error) => reject(prop, "unsupported", &error),
                                            },
                                        );
                                    }
                                    _ => {
                                        let definition = &self.assembly.descriptor["props"][prop];
                                        out.push(
                                            match b
                                                .write(
                                                    &self.plans,
                                                    &value,
                                                    &self.codes,
                                                    rules,
                                                    definition,
                                                )
                                                .and_then(|cmds| self.adapter.write(&cmds))
                                            {
                                                Ok(dps) => send(dps),
                                                Err(e) => reject(
                                                    prop,
                                                    if e.starts_with("unsupported:") {
                                                        "unsupported"
                                                    } else {
                                                        "invalid_value"
                                                    },
                                                    &e,
                                                ),
                                            },
                                        );
                                    }
                                }
                            }
                        } else {
                            out.push(reject(prop, "unsupported", "no write path"));
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(out)
    }
}
fn send(dps: V) -> V {
    json!({"type":"send_message","channel":"set","json":{"dps":dps}})
}
fn reject(prop: &str, code: &str, reason: &str) -> V {
    json!({"type":"reject","prop":prop,"code":code,"reason":reason})
}
