use crate::DpKind;
use crate::StateSource;
use crate::entity;
use crate::error::OrEval;
use crate::machine::{Machine, MachineEffects, Method};
use crate::protocol::SlotKind;
use crate::{Channel, DriverOptions, EngineError, Input, Output, RejectCode, SendChannel};
use crate::{adapter::*, assembly::*, data::*, plan::Plan};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value as V, json};
use std::sync::Arc;
mod check;
pub(crate) mod cover;
pub(crate) mod overrides;
pub use check::check;
use cover::{Cover, Motion, make_covers, own};
use overrides::{patch_descriptor, prepare};
#[derive(Clone, Serialize, Deserialize)]
pub struct Driver {
    #[serde(flatten)]
    pub config: Arc<DriverConfig>,
    pub assembly: Assembly,
    pub covers: Vec<Cover>,
    pub motions: Vec<Motion>,
    pub delta: Option<DeltaSettings>,
    pub timers: Vec<String>,
    pub codes: Table,
    pub dps: Table,
    pub values: Table,
    pub described: bool,
    pub linked: bool,
    pub synced: bool,
    pub seq: i64,
    pub machines: Vec<Machine>,
    pub rules_fingerprint: String,
}

/// Immutable interpretation shared by in-memory transactions.
#[derive(Serialize, Deserialize)]
pub struct DriverConfig {
    pub device: V,
    pub block: V,
    pub plans: Vec<Plan>,
    pub adapter: Adapter,
    pub switches: IndexMap<String, Setting>,
    pub own_blocks: OwnBlocks,
}

/// User overrides at each precedence level; snapshots retain their two-element array.
#[derive(Clone, Serialize, Deserialize)]
#[serde(from = "[V; 2]", into = "[V; 2]")]
pub struct OwnBlocks {
    product: V,
    device: V,
}
impl From<[V; 2]> for OwnBlocks {
    fn from([product, device]: [V; 2]) -> Self {
        Self { product, device }
    }
}
impl From<OwnBlocks> for [V; 2] {
    fn from(blocks: OwnBlocks) -> Self {
        [blocks.product, blocks.device]
    }
}

impl Driver {
    pub(crate) fn validate_state(&self) -> Result<(), EngineError> {
        let valid = self.assembly.descriptor.is_object()
            && self.assembly.descriptor["props"].is_object()
            && self
                .assembly
                .bindings
                .values()
                .all(|binding| match binding.source {
                    Source::Layout { plan, .. } | Source::Dp { plan, .. } => {
                        plan < self.config.plans.len()
                    }
                    Source::Machine { index } => index < self.machines.len(),
                    Source::Motion => true,
                })
            && self.config.switches.values().all(|setting| match setting {
                Setting::AcceptPassive => self.delta.is_some(),
                Setting::Cover { cover, .. } => *cover < self.covers.len(),
            })
            && self
                .covers
                .iter()
                .all(|cover| cover.plan < self.config.plans.len());
        if valid {
            Ok(())
        } else {
            Err(EngineError::InvalidInput("invalid driver state".into()))
        }
    }
    pub fn new(device: &V, options: &DriverOptions, rules: &V) -> Result<Self, EngineError> {
        if device["id"].as_str().is_none() {
            return Err(EngineError::InvalidDevice("device needs id".into()));
        }
        // Tables keyed by DP code or ID; absent is allowed, any other shape is a malformed export.
        for table in ["function", "status_range", "status", "local_strategy"] {
            let value = &device[table];
            if !value.is_null() && !value.is_object() {
                return Err(EngineError::InvalidDevice(format!(
                    "device {table} must be an object keyed by DP code or ID"
                )));
            }
        }
        if !options.dpmap.is_object()
            || fields(&options.dpmap).any(|(_, code)| code.as_str().is_none_or(str::is_empty))
        {
            return Err(EngineError::InvalidInput(
                "dpmap must be an object of nonempty codes".into(),
            ));
        }
        let (d, block, dpmap, info) =
            prepare(device, options, rules).map_err(EngineError::InvalidOverride)?;
        let mut adapter = build_adapter(&d, &dpmap, &block, rules);
        let env = match &options.env {
            Some(env) => json!(env),
            None => json!({"temperature_unit":"C","allowed_units":rules["host_units"]}),
        };
        let allow = options.allow_hazardous;
        let settings = options.device_settings;
        let mut plans = if block["auto"] == false {
            vec![]
        } else {
            entity::classify(&d, &env, rules, &V::Null)?
        };
        if block["auto"] != false
            && truth(
                block
                    .get("expose_unused")
                    .unwrap_or(&json!(options.expose_unused)),
            )
        {
            entity::unused(&d, &adapter, &mut plans, rules)?;
        }
        let mut assembly = assemble(&d, &mut plans, &info, allow, rules).or_eval()?;
        let mut covers = make_covers(&assembly, &plans, &block, settings, rules);
        let choices: Vec<_> = covers.iter().map(|c| c.position_choice).collect();
        // A cover that takes its position from the target is classified again without its
        // separate current-position DP.
        let selected = match without_target_positions(&d, &covers) {
            Some(selected) => {
                plans = entity::classify(&selected, &env, rules, &V::Null)?;
                assembly = assemble(&selected, &mut plans, &info, allow, rules).or_eval()?;
                covers = make_covers(&assembly, &plans, &block, settings, rules);
                selected
            }
            None => d.clone(),
        };
        for (i, c) in covers.iter_mut().enumerate() {
            c.position_choice = choices.get(i).copied().unwrap_or(false);
            invert_cover(c, &mut adapter, &d, rules);
        }
        check_cover_groups(&block, &covers)?;
        let (machines, taken) = declarative(&mut assembly, &block)?;
        let motions = cover_motions(&covers, &taken, rules);
        for motion in &motions {
            let mut def = rules["motion_property"].clone();
            if let Some(g) = &motion.group {
                def["group"] = json!(g);
            }
            Arc::make_mut(&mut assembly.descriptor)["props"][&motion.prop] = def;
            assembly
                .bindings
                .insert(motion.prop.clone(), Binding::new(Source::Motion, false));
        }
        let delta = delta_config(&assembly, &plans, &block);
        let mut switches = if settings {
            config_switches(&mut covers, &mut assembly, delta.is_some(), &taken, rules)
        } else {
            IndexMap::new()
        };
        patch_descriptor(&mut assembly, &mut plans, &block, &selected, rules)
            .map_err(EngineError::InvalidOverride)?;
        switches.retain(|k, _| assembly.descriptor["props"].get(k).is_some());
        let own_blocks = OwnBlocks {
            product: options.overrides[s(&device["product_id"])].clone(),
            device: options.overrides[s(&device["id"])].clone(),
        };
        Ok(Self {
            config: Arc::new(DriverConfig {
                device: d,
                block,
                plans,
                adapter,
                switches,
                own_blocks,
            }),
            assembly,
            covers,
            motions,
            delta,
            timers: vec![],
            codes: Table::default(),
            dps: Table::default(),
            values: Table::default(),
            described: false,
            linked: false,
            synced: false,
            seq: 0,
            machines,
            rules_fingerprint: String::new(),
        })
    }
    /// Take over what `old` counted: delta totals of the same DPs, and the message sequence.
    pub fn carry(&mut self, old: &Driver) {
        self.seq = self.seq.max(old.seq);
        let is_delta = |p: Option<&Plan>| p.is_some_and(|p| p.slot_kind == Some(SlotKind::Delta));
        for (name, b) in self.assembly.bindings.iter_mut() {
            let Some(ob) = old.assembly.bindings.get(name) else {
                continue;
            };
            let a = b.plan().and_then(|i| self.config.plans.get(i));
            let o = ob.plan().and_then(|i| old.config.plans.get(i));
            if is_delta(a) && is_delta(o) && a.map(|p| &p.depends_on) == o.map(|p| &p.depends_on) {
                b.total = ob.total;
                b.last_ts = ob.last_ts.clone();
            }
        }
    }

    pub fn set(&mut self, prop: &str, value: V, out: &mut Vec<Output>) {
        set_value(&mut self.values, prop, value, out);
    }
    pub fn describe(&mut self) -> Vec<Output> {
        if self.described {
            return vec![];
        }
        self.described = true;
        let mut out = vec![Output::Descriptor {
            desc: self.assembly.descriptor.as_ref().clone(),
        }];
        for m in &self.motions {
            if m.mode != StateSource::Inferred
                && self.assembly.descriptor["props"].get(&m.prop).is_some()
            {
                out.push(Output::Absent {
                    prop: m.prop.clone(),
                });
            }
        }
        for (prop, setting) in self.config.switches.clone() {
            let v = match setting {
                Setting::AcceptPassive => {
                    json!(self.delta.as_ref().is_some_and(|d| d.accept_passive))
                }
                Setting::Cover { cover, key } => self.covers[cover].value(&key),
            };
            self.set(&prop, v, &mut out);
        }
        out
    }
    pub fn settings_block(&self) -> V {
        let mut out = json!({});
        for c in &self.covers {
            let mut base = c.defaults.clone();
            base.apply(&own(&self.config.own_blocks.product, &c.group));
            let base = json!(base);
            let had = own(&self.config.own_blocks.device, &c.group);
            let mut vals = json!({});
            for (k, v) in obj(&json!(c.settings)) {
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
            // Kept when it differs from the product's setting or the device block already had it.
            let base = DeltaSettings::from_block(&self.config.own_blocks.product["delta"]);
            if delta.accept_passive != base.accept_passive
                || self.config.own_blocks.device["delta"]
                    .get("accept_passive")
                    .is_some()
            {
                out["delta"] = json!({"accept_passive": delta.accept_passive});
            }
        }
        out
    }
    fn apply_effects(
        &mut self,
        idx: usize,
        effects: MachineEffects,
        out: &mut Vec<Output>,
    ) -> Result<(), EngineError> {
        // Validate the entire send before publishing values or scheduling timers.
        let prepared = if effects.commands.is_empty() {
            None
        } else {
            if !self.linked {
                return Err(EngineError::Unsupported("device link is down".into()));
            }
            Some(self.config.adapter.write(&effects.commands)?)
        };
        for (prop, value) in effects.values {
            let owned =
                self.assembly
                    .bindings
                    .get(&prop)
                    .is_some_and(|binding| match binding.source {
                        Source::Machine { index } => index == idx,
                        Source::Motion => idx >= self.machines.len(),
                        _ => false,
                    });
            if owned {
                self.set(&prop, value, out);
            }
        }
        for (name, after) in effects.timers {
            let full = format!("c{idx}:{name}");
            if let Some(after) = after {
                if !self.timers.contains(&full) {
                    self.timers.push(full.clone());
                }
                out.push(Output::SetTimer { name: full, after });
            } else if self.timers.contains(&full) {
                self.timers.retain(|n| *n != full);
                out.push(Output::CancelTimer { name: full });
            }
        }
        if let Some(dps) = prepared {
            out.push(send(dps));
        }
        Ok(())
    }
    pub fn handle(
        &mut self,
        now: f64,
        input: &Input,
        rules: &V,
    ) -> Result<Vec<Output>, EngineError> {
        self.handle_inner(now, input, rules, 0)
    }

    fn handle_inner(
        &mut self,
        now: f64,
        input: &Input,
        rules: &V,
        depth: usize,
    ) -> Result<Vec<Output>, EngineError> {
        if depth >= 64 {
            return Err(EngineError::InvalidInput(
                "input nesting limit exceeded".into(),
            ));
        }
        let mut out = vec![];
        match input {
            Input::Connected => {
                self.linked = true;
                out = self.describe();
            }
            Input::Describe { seed } => {
                out = self.describe();
                for seed in seed {
                    out.extend(self.handle_inner(now, seed, rules, depth + 1)?);
                }
                if !self.synced {
                    self.set("available", json!(false), &mut out);
                }
            }
            Input::Disconnected => {
                self.linked = false;
                self.synced = false;
                self.codes = Table::default();
                self.dps = Table::default();
                let old = self.values.take();
                for (prop, v) in fields(&old) {
                    if self.config.switches.contains_key(prop) {
                        self.values.insert(prop, v.clone());
                    } else if prop != "available" {
                        out.push(Output::Absent { prop: prop.clone() });
                    }
                }
                self.values.insert("available", json!(false));
                out.push(Output::Value {
                    prop: "available".into(),
                    value: json!(false),
                });
                for machine in &mut self.machines {
                    machine.reset();
                }
                for m in &mut self.motions {
                    m.reset();
                }
                let mut timers = std::mem::take(&mut self.timers);
                timers.sort();
                for name in timers {
                    out.push(Output::CancelTimer { name });
                }
            }
            Input::Message { channel, json } => {
                out = self.describe();
                let mut dps = json.clone();
                if !dps.is_object() {
                    dps = json!({});
                }
                let t = remove(&mut dps, "t").unwrap_or(V::Null);
                if truth(&dps) {
                    self.dps.merge(&dps);
                    let new = self.config.adapter.read(&dps, rules);
                    let pushed = *channel == Channel::Active;
                    let moved: Vec<_> = obj(&new)
                        .iter()
                        .filter(|(c, v)| self.codes.get(*c).is_none_or(|old| !eq(old, v)))
                        .map(|(c, _)| c.clone())
                        .collect();
                    let first = !self.synced;
                    let (active, changed) = if pushed || first {
                        (pushed, obj(&new).keys().cloned().collect::<Vec<_>>())
                    } else if *channel == Channel::Passive {
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
                    self.codes.merge(&new);
                    if first {
                        self.synced = true;
                        self.linked = true;
                        self.set("available", json!(true), &mut out);
                    }
                    for idx in 0..self.machines.len() {
                        let result=self.machines[idx].invoke(Method::Update,&json!({"now":now,"codes":self.codes,"changed":changed,"active":active}))?;
                        self.apply_effects(idx, result, &mut out)?;
                    }
                    for idx in 0..self.motions.len() {
                        let (vals, timer) =
                            self.motions[idx].update(&self.codes, &changed, active, false);
                        let effects = MachineEffects {
                            values: vals.as_object().cloned().unwrap_or_default(),
                            timers: timer
                                .map(|t| [("settle".into(), t)].into_iter().collect())
                                .unwrap_or_default(),
                            ..MachineEffects::default()
                        };
                        self.apply_effects(self.machines.len() + idx, effects, &mut out)?;
                    }
                    let counted = pushed
                        || (*channel == Channel::Passive
                            && self.delta.as_ref().is_some_and(|d| d.accept_passive));
                    let mut reads = ReadCache::default();
                    for (name, b) in &mut self.assembly.bindings {
                        let Some(p) = b.plan().and_then(|i| self.config.plans.get(i)) else {
                            continue;
                        };
                        if p.slot_kind == Some(SlotKind::Event) {
                            if pushed
                                && let Some(event) =
                                    p.event(&self.codes, &changed, rules).or_eval()?
                                && items(&self.assembly.descriptor["props"][name]["options"])
                                    .contains(&event[0])
                            {
                                out.push(Output::Event {
                                    prop: name.clone(),
                                    kind: s(&event[0]).into(),
                                });
                            }
                            continue;
                        }
                        if !b.readable(&self.config.plans, rules) {
                            continue;
                        }
                        let mut update = first || p.affected(&changed);
                        if p.slot_kind == Some(SlotKind::Delta) {
                            let added = p.depends_on.first().is_some_and(|code| {
                                crate::plan::accumulate(
                                    &mut b.total,
                                    &mut b.last_ts,
                                    &self.codes[code],
                                    &ts,
                                    counted && new.get(code).is_some(),
                                )
                            });
                            update = added || first;
                        }
                        if update {
                            let v = b
                                .read(
                                    &self.config.plans,
                                    &self.codes,
                                    rules,
                                    &self.assembly.descriptor["props"][name],
                                    &mut reads,
                                )
                                .or_eval()?;
                            set_value(&mut self.values, name, v, &mut out);
                        }
                    }
                }
            }
            Input::Timer { name } => {
                let name = name.as_str();
                if self.timers.contains(&name.to_string()) {
                    self.timers.retain(|n| n != name);
                    if let Some((head, short)) = name.split_once(':')
                        && let Some(idx) =
                            head.strip_prefix('c').and_then(|x| x.parse::<usize>().ok())
                    {
                        if idx < self.machines.len() {
                            let result = self.machines[idx].invoke(
                                Method::Timer,
                                &json!({"now":now,"name":short,"codes":self.codes}),
                            )?;
                            self.apply_effects(idx, result, &mut out)?;
                        } else if let Some(m) = self.motions.get_mut(idx - self.machines.len())
                            && short == "settle"
                        {
                            let (vals, _) = m.update(&self.codes, &[], false, true);
                            self.apply_effects(
                                idx,
                                MachineEffects {
                                    values: vals.as_object().cloned().unwrap_or_default(),
                                    ..MachineEffects::default()
                                },
                                &mut out,
                            )?;
                        }
                    }
                }
            }
            Input::Command { prop, value } => {
                let prop = prop.as_str();
                match check(&self.assembly.descriptor, &self.values, prop, value) {
                    Err((code, reason)) => out.push(Output::Reject {
                        prop: prop.into(),
                        code,
                        reason,
                    }),
                    Ok(value) => {
                        if let Some(setting) = self.config.switches.get(prop).cloned() {
                            match setting {
                                Setting::AcceptPassive => {
                                    if let Some(delta) = &mut self.delta {
                                        delta.accept_passive = value == true;
                                    }
                                }
                                Setting::Cover { cover, key } => {
                                    self.covers[cover].settings.set(&key, &value);
                                }
                            }
                            self.set(prop, value, &mut out);
                            out.push(Output::SettingsChanged {
                                block: self.settings_block(),
                            });
                        } else if let Some(b) = self.assembly.bindings.get(prop) {
                            if !b.writable {
                                out.push(reject(prop, RejectCode::Unsupported, "no write path"));
                            } else if !self.linked {
                                out.push(reject(
                                    prop,
                                    RejectCode::Unavailable,
                                    "device link is down",
                                ));
                            } else {
                                match &b.source {
                                    Source::Machine { index } => {
                                        let index = *index;
                                        let old_state = self.machines[index].state.clone();
                                        // A failed program commits no state; failed effects roll it back.
                                        let result = self.machines[index]
                                            .invoke(Method::Write, &json!({"prop":prop,"value":value,"codes":self.codes,"now":now}))
                                            .and_then(|result| self.apply_effects(index, result, &mut out));
                                        if let Err(error) = result {
                                            self.machines[index].state = old_state;
                                            out.push(reject(
                                                prop,
                                                error.reject_code(),
                                                &error.to_string(),
                                            ));
                                        }
                                    }
                                    _ => {
                                        let definition = &self.assembly.descriptor["props"][prop];
                                        out.push(
                                            match b
                                                .write(
                                                    &self.config.plans,
                                                    &value,
                                                    &self.codes,
                                                    rules,
                                                    definition,
                                                )
                                                .and_then(|cmds| self.config.adapter.write(&cmds))
                                            {
                                                Ok(dps) => send(dps),
                                                Err(e) => {
                                                    reject(prop, e.reject_code(), &e.to_string())
                                                }
                                            },
                                        );
                                    }
                                }
                            }
                        } else {
                            out.push(reject(prop, RejectCode::Unsupported, "no write path"));
                        }
                    }
                }
            }
        }
        Ok(out)
    }
}
fn send(dps: V) -> Output {
    Output::SendMessage {
        channel: SendChannel::Set,
        json: json!({"dps":dps}),
    }
}
fn reject(prop: &str, code: RejectCode, reason: &str) -> Output {
    Output::Reject {
        prop: prop.into(),
        code,
        reason: reason.into(),
    }
}

/// The DP adapter for a prepared device, with the override block's DP definitions, removals and
/// remaps applied.
fn build_adapter(d: &V, dpmap: &V, block: &V, rules: &V) -> Adapter {
    let mut adapter = Adapter::new(d, dpmap, rules);
    for (id, patch) in obj(&block["dp"]) {
        let code = s(&patch["code"]).to_string();
        if patch.get("type").is_none() {
            if let Some(e) = adapter.entries.get_mut(&id) {
                e.code = code;
            }
        } else if adapter.entries.get(&id).is_none_or(|e| e.code != code) {
            adapter
                .entries
                .insert(id, DpEntry::new(code, "default", json!({})));
        }
    }
    let removed = codes(&block["remove"]);
    adapter.entries.retain(|_, e| !removed.contains(&e.code));
    for (code, r) in obj(&block["remap"]) {
        adapter.remaps[&code] = r;
        adapter.set_bounds(&code, d);
    }
    adapter
}

/// The device without the current-position DPs of covers set to `position_from_target`, or
/// `None` when no cover drops one.
fn without_target_positions(d: &V, covers: &[Cover]) -> Option<V> {
    let mut selected = d.clone();
    let mut dropped = false;
    for c in covers {
        if c.settings.position_from_target
            && c.separate()
            && let Some(r) = &c.cur
        {
            for t in ["function", "status_range", "status"] {
                if let Some(table) = selected[t].as_object_mut() {
                    table.remove(&r.code);
                }
            }
            dropped = true;
        }
    }
    dropped.then_some(selected)
}

/// Apply a cover's inversion settings to the adapter's DP conversions.
fn invert_cover(c: &Cover, adapter: &mut Adapter, d: &V, rules: &V) {
    for (invert, r) in [
        (c.settings.invert_position, c.cur.as_ref()),
        (
            c.settings.invert_set_position,
            if c.separate() { c.setp.as_ref() } else { None },
        ),
        (c.settings.invert_tilt, c.tilt.as_ref()),
    ] {
        if invert && let Some(r) = r {
            adapter.invert(&r.code, d);
        }
    }
    if c.settings.invert_control
        && c.applies("invert_control")
        && let Some(r) = &c.ins
    {
        if r.kind == DpKind::Boolean {
            adapter.invert(&r.code, d);
        } else {
            swap_control_words(adapter, r, &cover_words(r, &c.words, rules));
        }
    }
}

/// Swap an enum control's open and close words in its alias map, keeping existing aliases.
fn swap_control_words(adapter: &mut Adapter, r: &Role, words: &V) {
    let swaps = if r.range().contains(&words["open"]) && r.range().contains(&words["close"]) {
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

/// Every cover settings group in the override block must name a cover of the device.
fn check_cover_groups(block: &V, covers: &[Cover]) -> Result<(), EngineError> {
    for (k, v) in obj(&block["cover"]) {
        if v.is_object()
            && !covers
                .iter()
                .any(|c| c.group.as_deref() == Some(k.as_str()))
        {
            return Err(EngineError::InvalidOverride(format!(
                "cover.{k}: the device has no cover in a group of that name"
            )));
        }
    }
    Ok(())
}

/// Bind the declarative converter's properties; returns its machines and the properties it takes.
fn declarative(
    assembly: &mut Assembly,
    block: &V,
) -> Result<(Vec<Machine>, Vec<String>), EngineError> {
    let mut taken = vec![];
    let mut machines = vec![];
    if let Some(cfg) = block["converters"].get("declarative") {
        let index = machines.len();
        for (prop, def) in obj(&cfg["props"]) {
            if prop == "available" || taken.contains(&prop) {
                return Err(EngineError::InvalidOverride(
                    "converter property taken".into(),
                ));
            }
            taken.push(prop.clone());
            Arc::make_mut(&mut assembly.descriptor)["props"][&prop] = def.clone();
            let writable = truth(&def["rw"]) || def["type"] == "trigger";
            if writable && cfg["write"].get(&prop).is_none() {
                return Err(EngineError::InvalidOverride(
                    "writable declarative property needs write program".into(),
                ));
            }
            assembly
                .bindings
                .insert(prop, Binding::new(Source::Machine { index }, writable));
        }
        machines.push(Machine::new(cfg));
    }
    Ok((machines, taken))
}

/// A `cover_state` motion for each cover with a usable state source and no converter property
/// of that name.
fn cover_motions(covers: &[Cover], taken: &[String], rules: &V) -> Vec<Motion> {
    let mut motions = vec![];
    for c in covers {
        let prop = format!("{}cover_state", c.prefix);
        if taken.contains(&prop) {
            continue;
        }
        let Some(source) = c.source().filter(|source| match *source {
            StateSource::Control => c.ins.is_some(),
            StateSource::Inferred => c.can_infer(),
            StateSource::None => c.cur.is_some(),
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
            settle: c.settings.settle,
            mode: source,
            invert_report: c.settings.invert_reported_motion,
            ..Motion::default()
        });
    }
    motions
}

/// Settings of a device whose properties accumulate increments.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeltaSettings {
    /// Count `passive` reports as increments too.
    pub accept_passive: bool,
}
impl DeltaSettings {
    /// Settings from a validated override `delta` block; absent members keep their defaults.
    fn from_block(block: &V) -> Self {
        Self {
            accept_passive: block["accept_passive"] == true,
        }
    }
}

/// What a device-setting property changes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Setting {
    /// The delta `accept_passive` setting.
    AcceptPassive,
    /// The setting `key` of the cover at index `cover`.
    Cover { cover: usize, key: String },
}

/// Delta settings when any bound property accumulates increments.
fn delta_config(assembly: &Assembly, plans: &[Plan], block: &V) -> Option<DeltaSettings> {
    let accumulates = assembly.bindings.values().any(|b| {
        b.plan()
            .and_then(|i| plans.get(i))
            .is_some_and(|p| p.slot_kind == Some(SlotKind::Delta))
    });
    accumulates.then(|| DeltaSettings::from_block(&block["delta"]))
}

/// Device-setting properties for cover and delta settings, keyed by property name.
fn config_switches(
    covers: &mut [Cover],
    assembly: &mut Assembly,
    delta: bool,
    taken: &[String],
    rules: &V,
) -> IndexMap<String, Setting> {
    let mut switches = IndexMap::new();
    for (i, c) in covers.iter_mut().enumerate() {
        for (k, label) in obj(&rules["cover_switches"]) {
            if !c.applies(&k)
                || ["state_source", "invert_reported_motion"].contains(&k.as_str())
                    && taken.contains(&format!("{}cover_state", c.prefix))
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
            Arc::make_mut(&mut assembly.descriptor)["props"][&prop] = def;
            c.switches.push(k.clone());
            switches.insert(prop, Setting::Cover { cover: i, key: k });
        }
    }
    if delta {
        let prop = "delta_accept_passive";
        if assembly.descriptor["props"].get(prop).is_none() {
            Arc::make_mut(&mut assembly.descriptor)["props"][prop] = json!({"type":"binary","rw":true,"category":"config","label":"Count passive reports"});
            switches.insert(prop.into(), Setting::AcceptPassive);
        }
    }
    switches
}

fn set_value(values: &mut Table, prop: &str, value: V, out: &mut Vec<Output>) {
    if value.is_null() {
        if values.remove(prop).is_some() {
            out.push(Output::Absent { prop: prop.into() });
        }
    } else if values.get(prop).is_none_or(|v| !eq(v, &value)) {
        values.insert(prop, value.clone());
        out.push(Output::Value {
            prop: prop.into(),
            value,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_levels_preserve_snapshot_format_and_reject_wrong_arity() {
        let snapshot = json!([{"delta":{"accept_passive":true}}, {"cover":{"invert":true}}]);
        let blocks: OwnBlocks = serde_json::from_value(snapshot.clone()).unwrap();
        assert_eq!(blocks.product["delta"]["accept_passive"], true);
        assert_eq!(blocks.device["cover"]["invert"], true);
        assert_eq!(serde_json::to_value(blocks).unwrap(), snapshot);
        for invalid in [json!([]), json!([{}]), json!([{}, {}, {}])] {
            assert!(serde_json::from_value::<OwnBlocks>(invalid).is_err());
        }
    }
}
