use crate::entity::{self, Entity};
use crate::machine::Machine;
use crate::{adapter::*, assembly::*, data::*, plan::Plan};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value as V, json};
#[derive(Clone, Serialize, Deserialize)]
pub struct Cover {
    pub plan: usize,
    pub prefix: String,
    pub group: Option<String>,
    pub hazardous: bool,
    pub defaults: V,
    pub settings: V,
    pub cur: Option<Role>,
    pub setp: Option<Role>,
    pub ins: Option<Role>,
    pub tilt: Option<Role>,
    pub position_choice: bool,
    pub switches: Vec<String>,
    /// Map names of the command words this cover's instruction may use, preferred first.
    pub words: V,
}
impl Cover {
    pub fn source(&self) -> &str {
        if truth(&self.settings["state_source"]) {
            s(&self.settings["state_source"])
        } else if truth(&self.settings["infer_motion"]) {
            "inferred"
        } else {
            "none"
        }
    }
    pub fn separate(&self) -> bool {
        self.setp
            .as_ref()
            .is_some_and(|r| self.cur.as_ref().is_none_or(|c| c.code != r.code))
    }
    pub fn applies(&self, k: &str) -> bool {
        match k {
            "position_from_target" => self.position_choice,
            "state_source" => self.cur.is_some() || self.ins.is_some(),
            "invert_reported_motion" => self.ins.is_some() && self.source() == "control",
            "invert_position" => self.cur.is_some(),
            "invert_set_position" => self.separate(),
            "invert_control" => self.ins.is_some() && self.setp.is_none(),
            "infer_motion" => self.cur.is_some() && (self.separate() || self.ins.is_some()),
            _ => true,
        }
    }
    pub fn value(&self, k: &str) -> V {
        if k == "state_source" {
            json!(self.source())
        } else {
            self.settings[k].clone()
        }
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Motion {
    pub prop: String,
    pub group: Option<String>,
    pub command: Option<String>,
    pub target_code: Option<String>,
    pub position: Option<String>,
    pub words: V,
    pub invert: bool,
    pub settle: f64,
    pub mode: String,
    pub invert_report: bool,
    pub state: Option<String>,
    pub target: Option<f64>,
    pub by_word: bool,
    pub last: Option<f64>,
}
impl Motion {
    pub fn from_config(
        prop: String,
        group: Option<String>,
        config: &V,
        mode: String,
        invert_report: bool,
    ) -> Self {
        Self {
            prop,
            group,
            mode,
            invert_report,
            command: config["command"].as_str().map(String::from),
            target_code: config["set_position"].as_str().map(String::from),
            position: config["position"].as_str().map(String::from),
            words: config["words"].clone(),
            invert: truth(&config["invert"]),
            settle: n(&config["settle"]),
            ..Self::default()
        }
    }
    pub fn pos(&self, st: &V) -> Option<f64> {
        let v = st.get(self.position.as_deref()?)?;
        if !v.is_number() {
            return None;
        }
        Some(if self.invert { 100.0 - n(v) } else { n(v) })
    }
    pub fn reset(&mut self) {
        self.state = None;
        self.target = None;
        self.by_word = false;
        self.last = None;
    }
    pub fn update(
        &mut self,
        st: &V,
        changed: &[String],
        active: bool,
        timer: bool,
    ) -> (V, Option<Option<f64>>) {
        let pos = self.pos(st);
        let last = self.last;
        if pos.is_some() {
            self.last = pos;
        }
        let touched = |code: &Option<String>| code.as_ref().is_some_and(|c| changed.contains(c));
        let mut state = None;
        let mut timer_out = None;
        if self.mode != "inferred" {
            if !touched(&self.command) && !touched(&self.position) {
                return (json!({}), None);
            }
            if active && self.mode == "control" && touched(&self.command) {
                let word = &st[self.command.as_deref().unwrap_or("")];
                if *word == self.words["stop"] {
                    state = Some("stopped".to_string());
                } else if *word == self.words["open"] {
                    state = Some(
                        if self.invert_report {
                            "closing"
                        } else {
                            "opening"
                        }
                        .to_string(),
                    );
                } else if *word == self.words["close"] {
                    state = Some(
                        if self.invert_report {
                            "opening"
                        } else {
                            "closing"
                        }
                        .to_string(),
                    );
                }
                if let Some(ref s) = state {
                    self.state = Some(s.clone());
                    return (json!({self.prop.clone():s}), None);
                }
            }
            if active
                && self.mode == "control"
                && self
                    .state
                    .as_deref()
                    .is_some_and(|s| s == "opening" || s == "closing")
            {
                return (json!({}), None);
            }
            self.reset();
            return (
                json!({self.prop.clone():pos.map(|p|if p<=0.0{"closed"}else if p>=100.0{"open"}else{"stopped"})}),
                None,
            );
        }
        if timer {
            if self
                .state
                .as_deref()
                .is_some_and(|s| s == "opening" || s == "closing")
            {
                state = Some("stopped".into());
            } else {
                return (json!({}), None);
            }
        } else if !active {
            if pos.is_some()
                && (touched(&self.command) || touched(&self.position) || touched(&self.target_code))
            {
                state = Some("stopped".into());
            }
        } else {
            if touched(&self.command) {
                let word = &st[self.command.as_deref().unwrap_or("")];
                if *word == self.words["stop"] {
                    state = Some("stopped".into());
                } else if *word == self.words["open"] {
                    state = Some("opening".into());
                    self.target = Some(100.0);
                    self.by_word = true;
                } else if *word == self.words["close"] {
                    state = Some("closing".into());
                    self.target = Some(0.0);
                    self.by_word = true;
                }
            } else if touched(&self.target_code)
                && let Some(position) = pos
            {
                let raw = &st[self.target_code.as_deref().unwrap_or("")];
                if raw.is_number() {
                    let target = if self.invert { 100.0 - n(raw) } else { n(raw) };
                    self.target = Some(target);
                    self.by_word = false;
                    state = Some(
                        if target == position {
                            "stopped"
                        } else if target > position {
                            "opening"
                        } else {
                            "closing"
                        }
                        .into(),
                    );
                }
            } else if touched(&self.position)
                && let Some(p) = pos
                && self
                    .state
                    .as_deref()
                    .is_some_and(|s| s == "opening" || s == "closing")
            {
                if self.by_word
                    && let Some(last) = last
                    && last != p
                {
                    let heading = if p > last { "opening" } else { "closing" };
                    if self.state.as_deref() != Some(heading) {
                        state = Some(heading.into());
                        self.target = Some(if heading == "opening" { 100.0 } else { 0.0 });
                    }
                }
                let moving = state.as_deref().or(self.state.as_deref());
                if self.target.is_some_and(|t| {
                    if moving == Some("opening") {
                        p >= t
                    } else {
                        p <= t
                    }
                }) {
                    state = Some("stopped".into());
                }
            } else if touched(&self.position) && pos.is_some() {
                state = Some("stopped".into());
            }
            if pos.is_some_and(|p| {
                (p <= 0.0 && state.as_deref().or(self.state.as_deref()) == Some("closing"))
                    || (p >= 100.0 && state.as_deref().or(self.state.as_deref()) == Some("opening"))
            }) {
                state = Some("stopped".into());
            }
        }
        let moving = state
            .as_deref()
            .or(self.state.as_deref())
            .is_some_and(|s| s == "opening" || s == "closing")
            && state.as_deref() != Some("stopped");
        if self.settle > 0.0 && !timer {
            timer_out = Some(if moving { Some(self.settle) } else { None });
        }
        let mut vals = json!({});
        if let Some(new) = state {
            self.state = Some(new.clone());
            let out = if new == "stopped" {
                self.target = None;
                self.by_word = false;
                pos.map(|p| {
                    if p <= 0.0 {
                        "closed"
                    } else if p >= 100.0 {
                        "open"
                    } else {
                        "stopped"
                    }
                })
                .unwrap_or("stopped")
                .to_string()
            } else {
                new
            };
            vals[&self.prop] = json!(out);
        }
        (vals, timer_out)
    }
}
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
}
fn own(block: &V, group: &Option<String>) -> V {
    let c = &block["cover"];
    if let Some(g) = group {
        c[g].clone()
    } else {
        c.clone()
    }
}
fn make_covers(a: &Assembly, plans: &[Plan], block: &V, settings: bool, rules: &V) -> Vec<Cover> {
    a.covers
        .iter()
        .map(|(idx, pre, group, haz)| {
            let p = &plans[*idx];
            let spec = Entity::of(rules, &p.platform)
                .map(|e| e.field("cover"))
                .unwrap_or(&V::Null);
            let role = |k: &str| p.role(s(&spec[k])).cloned();
            let mut defaults = rules["cover_defaults"].clone();
            defaults["infer_motion"] = json!(settings);
            let mut cfg = defaults.clone();
            let own = own(block, group);
            for (k, v) in obj(&own) {
                if defaults.get(&k).is_some() {
                    cfg[&k] = v;
                }
            }
            let cur = codes(&spec["position"])
                .iter()
                .find_map(|k| p.role(k))
                .cloned();
            let setp = role("target");
            let choice = cur.is_some()
                && setp
                    .as_ref()
                    .is_some_and(|r| cur.as_ref().is_some_and(|c| c.code != r.code));
            Cover {
                plan: *idx,
                prefix: pre.clone(),
                group: group.clone(),
                hazardous: *haz,
                defaults,
                settings: cfg,
                cur,
                setp,
                ins: role("instruction"),
                tilt: role("tilt"),
                position_choice: choice,
                switches: vec![],
                words: spec["words"].clone(),
            }
        })
        .collect()
}
fn prepare(device: &V, options: &V, rules: &V) -> Result<(V, V, V, V), String> {
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
const CONVERTERS: [&str; 2] = ["declarative", "cover_motion"];

pub fn validate(b: &V, rules: &V) -> Result<(), String> {
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
        if let Some(cfg) = block["converters"].get("cover_motion") {
            let mut config = rules["motion_defaults"].clone();
            merge(&mut config, cfg);
            motions.push(Motion::from_config(
                "cover_state".into(),
                None,
                &config,
                "inferred".into(),
                false,
            ));
            taken.push("cover_state".into());
        }
        let motion_taken = taken.clone();
        for c in &covers {
            let prop = format!("{}cover_state", c.prefix);
            if taken.contains(&prop) {
                continue;
            }
            let source = c.source();
            let enabled = if source == "control" && c.ins.is_some() {
                true
            } else if source == "inferred" {
                c.applies("infer_motion")
            } else {
                c.cur.is_some() && c.settings["state_source"] == "none"
            };
            if !enabled {
                continue;
            }
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
                invert: false,
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

    /// Whether `other` would drive the device exactly as this one does (same rules, overrides,
    /// settings and descriptor).
    pub fn same_as(&self, other: &Driver) -> bool {
        self.rules_id == other.rules_id
            && self.block == other.block
            && self.delta == other.delta
            && self.assembly.descriptor == other.assembly.descriptor
            && self
                .covers
                .iter()
                .map(|c| &c.settings)
                .eq(other.covers.iter().map(|c| &c.settings))
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
pub fn check(desc: &V, state: &V, prop: &str, value: &V) -> Result<V, (String, String)> {
    let error = |code: &str, reason: String| Err((code.into(), reason));
    let Some(p) = desc["props"].get(prop) else {
        return error("unknown_property", prop.into());
    };
    let t = s(&p["type"]);
    if t == "event" || !truth(&p["rw"]) && t != "trigger" {
        return error("read_only", prop.into());
    }
    if let Some(req) = p.get("requires") {
        let ok = if req.is_string() {
            state[s(req)] == true
        } else {
            items(&req["in"])
                .iter()
                .any(|v| eq(v, &state[s(&req["prop"])]))
        };
        if !ok {
            return error("requires_unmet", format!("{prop} requires {req}"));
        }
    }
    match t {
        "trigger" => Ok(V::Null),
        "binary" => {
            if value.is_boolean() {
                return Ok(value.clone());
            }
            let key = if value.is_string() {
                s(value).to_lowercase()
            } else if value.is_i64() {
                value.to_string()
            } else {
                String::new()
            };
            match key.as_str() {
                "on" | "true" | "1" => Ok(json!(true)),
                "off" | "false" | "0" => Ok(json!(false)),
                _ => error("invalid_value", format!("not a boolean: {value}")),
            }
        }
        "number" => {
            if !value.is_number() {
                return error("invalid_value", format!("not a finite number: {value}"));
            }
            let v = n(value);
            let lo = &p["min"];
            let hi = &p["max"];
            if (!lo.is_null() && v < n(lo)) || (!hi.is_null() && v > n(hi)) {
                return error("out_of_range", format!("{lo}..{hi}"));
            }
            let step = n(&p["step"]);
            if step != 0.0 {
                let q = (v - n(lo)) / step;
                if (q - q.round_ties_even()).abs() * step.abs() > 1e-9 {
                    return error("bad_step", format!("step {step} from {}", n(lo)));
                }
            }
            Ok(value.clone())
        }
        "select" => {
            if items(&p["options"]).contains(value) {
                Ok(value.clone())
            } else {
                error("invalid_value", format!("not one of {}", p["options"]))
            }
        }
        "text" => {
            if value.is_string() && !s(value).is_empty() {
                Ok(value.clone())
            } else {
                error("invalid_value", "non-empty string required".into())
            }
        }
        _ => error("unsupported", format!("type {t}")),
    }
}
fn patch_descriptor(
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
