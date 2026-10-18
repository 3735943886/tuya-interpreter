//! ildevice properties from entity plans, laid out by each entity spec's `layout` rows.
use crate::EngineError;
use crate::data::*;
use crate::entity::{Entity, shared};
use crate::error::OrEval;
use crate::expr::{Scope, text};
use crate::plan::Plan;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value as V, json};
use std::collections::HashMap;
use std::{borrow::Cow, sync::Arc};

/// Where a property's value comes from and where its writes go.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// A row of the plan's entity layout.
    Layout { plan: usize, row: usize },
    /// An override property bound directly to one DP.
    Dp {
        plan: usize,
        role: Box<Role>,
        trigger: bool,
        send: V,
    },
    /// A declarative converter, by index.
    Machine { index: usize },
    /// Cover motion state, driven by the driver's motion trackers.
    Motion,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Binding {
    pub source: Source,
    pub writable: bool,
    pub total: f64,
    pub last_ts: V,
}

impl Binding {
    pub fn new(source: Source, writable: bool) -> Self {
        Self {
            source,
            writable,
            total: 0.0,
            last_ts: V::Null,
        }
    }

    pub fn plan(&self) -> Option<usize> {
        match self.source {
            Source::Layout { plan, .. } | Source::Dp { plan, .. } => Some(plan),
            _ => None,
        }
    }

    /// Whether DP updates produce this property's value.
    pub fn readable(&self, plans: &[Plan], rules: &V) -> bool {
        match &self.source {
            Source::Layout { plan, row } => plans
                .get(*plan)
                .is_some_and(|p| !layout_row(rules, p, *row)["read"].is_null()),
            Source::Dp { trigger, .. } => !trigger,
            _ => false,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Assembly {
    pub descriptor: Arc<V>,
    pub bindings: IndexMap<String, Binding>,
    pub unsupported: Vec<String>,
    pub covers: Vec<(usize, String, Option<String>, bool)>,
}

fn layout_row<'r>(rules: &'r V, plan: &Plan, row: usize) -> &'r V {
    Entity::of(rules, &plan.platform)
        .and_then(|e| items(e.field("layout")).get(row))
        .map_or(&V::Null, |r| shared(rules, r))
}

/// A layout binding's plan, row and entity.
fn layout<'r, 'p>(
    plans: &'p [Plan],
    plan: usize,
    row: usize,
    rules: &'r V,
) -> Result<(&'p Plan, &'r V, Entity<'r>), String> {
    let p = plans.get(plan).ok_or("binding refers to a missing plan")?;
    let entity = Entity::of(rules, &p.platform).ok_or("binding refers to an unknown entity")?;
    Ok((p, layout_row(rules, p, row), entity))
}

fn name(base: &str, props: &V) -> String {
    let base: String = base
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let base = if base.is_empty() {
        "prop".to_string()
    } else {
        base
    };
    let mut out = base.clone();
    let mut i = 2;
    while props.get(&out).is_some() || out == "available" {
        out = format!("{base}_{i}");
        i += 1;
    }
    out
}

/// Evaluate a definition template; members that evaluate to null are left out.
fn definition(template: &V, rules: &V, scope: &mut Scope) -> Result<V, String> {
    let mut def = json!({});
    for (k, v) in fields(shared(rules, template)) {
        let value = scope.eval(v)?;
        if !value.is_null() {
            def[k] = value;
        }
    }
    Ok(def)
}

pub fn assemble(
    device: &V,
    plans: &mut [Plan],
    info: &V,
    allow: bool,
    rules: &V,
) -> Result<Assembly, String> {
    let mut props = json!({});
    let mut bindings = IndexMap::new();
    let mut groups = json!({});
    let mut kind = V::Null;
    let mut klass = V::Null;
    let mut covers = vec![];
    let mut unsupported = vec![];
    for (idx, p) in plans.iter().enumerate() {
        let Some(entity) = Entity::of(rules, &p.platform) else {
            continue;
        };
        if !entity.exposed() {
            unsupported.push(format!("{}:{}", p.platform, p.key));
            continue;
        }
        let mut scope = entity
            .scope(p, &device["status"])
            .with("allow_hazardous", json!(allow));
        let hazard = truth(&scope.eval(entity.field("hazard"))?);
        scope.push("hazard", json!(hazard));
        let mut extra = definition(&rules["layout_common"], rules, &mut scope)?;
        let mut pre = String::new();
        let mut group = None;
        if let Some(k) = entity.composite() {
            if kind.is_null() {
                kind = json!(k);
                klass = p.identity["device_class"].clone();
            } else {
                let g = name(&p.key, &props);
                let mut gd = json!({"kind":k});
                if truth(&p.identity["device_class"]) {
                    gd["class"] = p.identity["device_class"].clone();
                }
                groups[&g] = gd;
                extra["group"] = json!(g);
                pre = format!("{g}_");
                group = Some(g);
            }
        }
        if entity.field("cover").is_object() {
            covers.push((idx, pre.clone(), group, hazard));
        }
        for (i, row) in items(entity.field("layout")).iter().enumerate() {
            let row = shared(rules, row);
            if let Some(when) = row.get("when")
                && !truth(&scope.eval(when)?)
            {
                continue;
            }
            let base = text(&scope.eval(&row["name"])?);
            let base = if row["grouped"] == false {
                base
            } else {
                format!("{pre}{base}")
            };
            let prop = name(&base, &props);
            let mut def = definition(&row["definition"], rules, &mut scope)?;
            merge(&mut def, &extra);
            def["src"] = json!(p.depends_on.first().unwrap_or(&p.key));
            let writable = !row["write"].is_null()
                && row
                    .get("writable")
                    .map(|w| scope.eval(w).map(|v| truth(&v)))
                    .transpose()?
                    .unwrap_or(true);
            props[&prop] = def;
            bindings.insert(
                prop,
                Binding::new(Source::Layout { plan: idx, row: i }, writable),
            );
        }
    }
    let mut available = json!({"available":{"type":"binary","role":"available"}});
    merge(&mut available, &props);
    let mut descriptor = json!({"il":0,"id":device["id"],"source":"tuya","props":available,"identifiers":{"tuya_id":device["id"]}});
    if truth(&device["product_id"]) {
        descriptor["identifiers"]["tuya_product_id"] = device["product_id"].clone();
    }
    for (k, v) in [
        ("kind", kind),
        ("class", klass),
        ("label", device["name"].clone()),
        ("vendor", info["manufacturer"].clone()),
        ("model", info["model"].clone()),
    ] {
        if truth(&v) {
            descriptor[k] = v;
        }
    }
    if truth(&groups) {
        descriptor["groups"] = groups;
    }
    Ok(Assembly {
        descriptor: Arc::new(descriptor),
        bindings,
        unsupported,
        covers,
    })
}

impl Binding {
    pub fn read(
        &self,
        plans: &[Plan],
        st: &V,
        rules: &V,
        definition: &V,
        cache: &mut ReadCache,
    ) -> Result<V, String> {
        match &self.source {
            Source::Dp { role, .. } => Ok(il_value(dp_read(role, st, definition), definition)),
            Source::Layout { plan, row } => {
                let (p, row, entity) = layout(plans, *plan, *row, rules)?;
                let mut status = Cow::Borrowed(st);
                for (role, value) in fields(&row["assume"]) {
                    if let Some(r) = p.role(role) {
                        status.to_mut()[&r.code] = value.clone();
                    }
                }
                let v = if matches!(status, Cow::Borrowed(_)) {
                    let key = (*plan, self.total.to_bits());
                    if let std::collections::hash_map::Entry::Vacant(entry) = cache.0.entry(key) {
                        entry.insert(ReadFrame {
                            state: p.read(st, self.total, rules)?,
                            roles: crate::entity::views(&p.roles),
                            values: crate::entity::values(&p.roles, st),
                        });
                    }
                    let frame = &cache.0[&key];
                    entity
                        .base_scope(p, st)
                        .with("roles", &frame.roles)
                        .with("values", &frame.values)
                        .with("state", &frame.state)
                        .with("definition", definition)
                        .eval(&row["read"])?
                } else {
                    let state = p.read(&status, self.total, rules)?;
                    entity
                        .scope(p, &status)
                        .with("state", state)
                        .with("definition", definition)
                        .eval(&row["read"])?
                };
                Ok(il_value(v, definition))
            }
            _ => Ok(V::Null),
        }
    }

    pub fn write(
        &self,
        plans: &[Plan],
        value: &V,
        st: &V,
        rules: &V,
        definition: &V,
    ) -> Result<Vec<V>, EngineError> {
        match &self.source {
            Source::Dp {
                role,
                trigger,
                send,
                ..
            } => {
                let v = if *trigger { send } else { value };
                Ok(vec![
                    json!({"code":role.code,"value":role.write(v).map_err(EngineError::InvalidValue)?}),
                ])
            }
            Source::Layout { plan, row } => {
                let (p, row, entity) = layout(plans, *plan, *row, rules).or_eval()?;
                let state = p.read(st, self.total, rules).or_eval()?;
                let call = entity
                    .scope(p, st)
                    .with("state", state)
                    .with("definition", definition)
                    .with("value", value)
                    .eval(&row["write"])
                    .or_eval()?;
                p.write(s(&call["action"]), &call["args"], st, rules)
            }
            _ => Err(EngineError::Unsupported("no write path".into())),
        }
    }
}

/// A packet-local cache; assumed roles and independently accumulated totals never share state.
#[derive(Default)]
pub(crate) struct ReadCache(HashMap<(usize, u64), ReadFrame>);
struct ReadFrame {
    state: V,
    roles: V,
    values: V,
}

/// ildevice value rules: a select value outside its options is unknown; a binary value is a
/// boolean (core reads leak 0 and 1, parity P-12); numbers are canonical.
pub(crate) fn il_value(v: V, definition: &V) -> V {
    if definition["type"] == "select" && !v.is_null() && !items(&definition["options"]).contains(&v)
    {
        V::Null
    } else if definition["type"] == "binary" && v.is_number() {
        match n(&v) {
            0.0 => json!(false),
            1.0 => json!(true),
            _ => V::Null,
        }
    } else if v.is_number() {
        number(n(&v))
    } else {
        v
    }
}

fn dp_read(r: &Role, st: &V, definition: &V) -> V {
    let raw = &st[&r.code];
    match s(&definition["type"]) {
        "text" => {
            if raw.is_null() {
                V::Null
            } else if raw.is_string() {
                raw.clone()
            } else {
                json!(raw.to_string())
            }
        }
        "select" => {
            let v = r.read(st);
            if items(&definition["options"]).contains(&v) {
                v
            } else {
                V::Null
            }
        }
        _ => r.read(st),
    }
}
