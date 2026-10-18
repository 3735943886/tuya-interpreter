//! Device entities, interpreted from the rule pack's `entities` specs.
//!
//! A spec names its roles (DP candidates and accepted types), the variants that build a plan,
//! the state it reads and the actions it writes. The engine contributes only the interpreter
//! and DP-type semantics; platform knowledge lives in the rule pack.
use crate::DpKind;
use crate::EngineError;
use crate::adapter::{Adapter, DpEntry};
use crate::data::*;
use crate::error::OrEval;
use crate::expr::{Effects, Flow, Restore, Scope, execute, target, text};
use crate::plan::Plan;
use crate::protocol::SlotKind;
use serde_json::{Value as V, json};

/// `{"$use": name}` stands for `templates.name`; anything else is itself.
pub fn shared<'r>(rules: &'r V, mut v: &'r V) -> &'r V {
    for _ in 0..8 {
        match v.get("$use").and_then(V::as_str) {
            Some(name) => v = &rules["templates"][name],
            None => break,
        }
    }
    v
}

/// One platform's spec within a rule pack.
#[derive(Clone, Copy)]
pub struct Entity<'r> {
    pub rules: &'r V,
    pub platform: &'r str,
    spec: &'r V,
}

impl<'r> Entity<'r> {
    pub fn of(rules: &'r V, platform: &str) -> Option<Self> {
        let (platform, spec) = rules["entities"].as_object()?.get_key_value(platform)?;
        Some(Self {
            rules,
            platform,
            spec,
        })
    }

    pub fn field(&self, name: &str) -> &'r V {
        shared(self.rules, &self.spec[name])
    }

    /// The device kind this entity gives a composite device, if it is one.
    pub fn composite(&self) -> Option<&'r str> {
        truth(self.field("composite")).then(|| self.field("kind").as_str().unwrap_or(self.platform))
    }

    /// Whether the entity becomes ildevice properties (and claims its DPs).
    pub fn exposed(&self) -> bool {
        self.field("expose") != &V::Bool(false)
    }

    /// Frames every plan-level expression sees.
    pub fn scope<'a>(&self, plan: &'a Plan, status: &'a V) -> Scope<'a>
    where
        'r: 'a,
    {
        self.base_scope(plan, status)
            .with("roles", views(&plan.roles))
            .with("values", values(&plan.roles, status))
    }

    pub fn base_scope<'a>(&self, plan: &'a Plan, status: &'a V) -> Scope<'a>
    where
        'r: 'a,
    {
        Scope::rules(self.rules)
            .with("entity", self.spec)
            .with("status", status)
            .with("key", json!(plan.key))
            .with("identity", &plan.identity)
            .with("config", &plan.config)
    }
}

pub fn views(roles: &Roles) -> V {
    V::Object(roles.iter().map(|(k, r)| (k.clone(), r.view())).collect())
}

pub fn values(roles: &Roles, status: &V) -> V {
    V::Object(
        roles
            .iter()
            .map(|(k, r)| (k.clone(), r.read(status)))
            .collect(),
    )
}

/// The frames a variant may assign; roles change only through `role` statements.
const ASSIGNABLE: [&str; 3] = ["identity", "config", "plan"];

/// Statements of a variant: role resolution, requirements and assignments.
struct Build<'d> {
    device: &'d V,
    roles: Roles,
}

impl Effects for Build<'_> {
    fn exec(&mut self, stmt: &V, scope: &mut Scope) -> Result<Flow, EngineError> {
        if let Some(name) = stmt.get("role") {
            let name = s(name).to_string();
            let candidates = codes(&scope.eval(&stmt["codes"]).or_eval()?);
            let prefer_function = stmt["prefer"] == "function";
            let mut found = None;
            for kind in codes(&scope.eval(&stmt["types"]).or_eval()?) {
                found = resolve(self.device, &candidates, &kind, prefer_function).or_eval()?;
                if found.is_some() {
                    break;
                }
            }
            match found {
                Some(role) => {
                    scope
                        .assign(&["roles".into(), name.clone()], role.view())
                        .or_eval()?;
                    scope
                        .assign(
                            &["values".into(), name.clone()],
                            role.read(&self.device["status"]),
                        )
                        .or_eval()?;
                    self.roles.insert(name, role);
                }
                None if stmt["required"] == true => return Ok(Flow::Stop),
                None => {}
            }
        } else if let Some(cond) = stmt.get("require") {
            if !truth(&scope.eval(cond).or_eval()?) {
                return Ok(Flow::Stop);
            }
        } else if let Some(path) = stmt.get("set") {
            let path = target(path);
            if !path
                .first()
                .is_some_and(|f| ASSIGNABLE.contains(&f.as_str()))
            {
                return Err(EngineError::eval(format!(
                    "a variant can assign only {ASSIGNABLE:?}, not {path:?}"
                )));
            }
            if let Some(merge) = stmt.get("merge") {
                for (k, v) in obj(&scope.eval(merge).or_eval()?) {
                    scope
                        .assign(&[path.clone(), vec![k]].concat(), v)
                        .or_eval()?;
                }
            } else {
                let value = scope.eval(&stmt["value"]).or_eval()?;
                scope.assign(&path, value).or_eval()?;
            }
        } else if let Some(cond) = stmt.get("assert") {
            if !truth(&scope.eval(cond).or_eval()?) {
                return Err(EngineError::eval(text(
                    &scope.eval(&stmt["error"]).or_eval()?,
                )));
            }
        } else {
            return Err(EngineError::eval(format!("unknown spec statement {stmt}")));
        }
        Ok(Flow::Next)
    }
}

/// Fresh per-variant frames; `plan` collects `slot_kind`, `update_all` and `depends_on`.
fn open<'s, 'a>(
    scope: &'s mut Scope<'a>,
    identity: V,
    roles: &Roles,
    status: &V,
) -> Restore<'s, 'a> {
    let mut inner = scope.mark();
    inner.push("identity", identity);
    inner.push("config", json!({}));
    inner.push("roles", views(roles));
    inner.push("values", values(roles, status));
    inner.push("plan", json!({}));
    inner
}

fn finish(
    entity: Option<&Entity>,
    platform: &str,
    key: &str,
    mut roles: Roles,
    scope: &mut Scope,
) -> Plan {
    let meta = scope.take("plan");
    if let Some(order) = entity
        .map(|e| e.field("role_order"))
        .filter(|o| o.is_array())
    {
        let mut ordered = Roles::new();
        for k in codes(order) {
            if let Some(r) = roles.shift_remove(&k) {
                ordered.insert(k, r);
            }
        }
        ordered.extend(roles);
        roles = ordered;
    }
    let mut depends_on = codes(&meta["depends_on"]);
    if depends_on.is_empty() {
        for r in roles.values() {
            if !depends_on.contains(&r.code) {
                depends_on.push(r.code.clone());
            }
        }
    }
    Plan {
        platform: platform.into(),
        key: key.into(),
        identity: scope.take("identity"),
        roles,
        depends_on,
        slot_kind: meta["slot_kind"].as_str().and_then(SlotKind::parse),
        update_all: truth(&meta["update_all"])
            || entity.is_some_and(|e| truth(e.field("update_all"))),
        config: scope.take("config"),
    }
}

/// The first variant (depth first) whose steps complete.
fn attempt<'a>(
    entity: &Entity<'a>,
    variant: &'a V,
    scope: &mut Scope<'a>,
    device: &'a V,
) -> Result<Option<Plan>, EngineError> {
    let variant = shared(entity.rules, variant);
    if let Some(when) = variant.get("when")
        && !truth(&scope.eval(when).or_eval()?)
    {
        return Ok(None);
    }
    if let Some(group) = variant.get("variants") {
        let group = shared(entity.rules, group);
        let list = match variant.get("each") {
            Some(e) => arr(&scope.eval(e).or_eval()?),
            None => vec![V::Null],
        };
        let name = variant.get("as").map(s).unwrap_or("item");
        for item in list {
            let mut inner = scope.mark();
            inner.push(name, item);
            for v in items(group) {
                if let Some(plan) = attempt(entity, v, &mut inner, device)? {
                    return Ok(Some(plan));
                }
            }
        }
        return Ok(None);
    }
    let desc = scope.get("desc").cloned().unwrap_or_default();
    let key = s(&desc["key"]).to_string();
    let identity = identity(&desc, &entity.rules["identity_defaults"]);
    let mut inner = open(scope, identity, &Roles::new(), &device["status"]);
    let mut build = Build {
        device,
        roles: Roles::new(),
    };
    for steps in [
        shared(entity.rules, &variant["steps"]),
        entity.field("finish"),
    ] {
        if execute(steps, &mut inner, &mut build)? == Flow::Stop {
            return Ok(None);
        }
    }
    Ok(Some(finish(
        Some(entity),
        entity.platform,
        &key,
        build.roles,
        &mut inner,
    )))
}

pub fn build(p: &str, d: &V, env: &V, desc: &V, rules: &V) -> Result<Option<Plan>, EngineError> {
    let Some(entity) = Entity::of(rules, p) else {
        return Ok(None);
    };
    let mut scope = Scope::rules(rules)
        .with("entity", entity.spec)
        .with("device", d)
        .with("status", &d["status"])
        .with("env", env)
        .with("desc", desc)
        .with("key", desc["key"].clone());
    for variant in items(entity.field("variants")) {
        if let Some(plan) = attempt(&entity, variant, &mut scope, d)? {
            return Ok(Some(plan));
        }
    }
    Ok(None)
}

pub fn classify(d: &V, env: &V, rules: &V, only: &V) -> Result<Vec<Plan>, EngineError> {
    let mut out = vec![];
    for (p, group) in fields(&rules["platforms"]) {
        if only.is_array() && !items(only).contains(&json!(p)) {
            continue;
        }
        let descs = &group["categories"][s(&d["category"])];
        let descs = if descs.is_object() {
            std::slice::from_ref(descs)
        } else {
            items(descs)
        };
        for desc in descs {
            out.extend(build(p, d, env, desc, rules)?);
        }
    }
    Ok(out)
}

/// The entity's state for a DP status: the spec's `state.fields`, after its `state.bind` values.
pub fn state(plan: &Plan, status: &V, total: f64, rules: &V) -> Result<V, String> {
    let Some(entity) = Entity::of(rules, &plan.platform) else {
        return Ok(json!({}));
    };
    let spec = entity.field("state");
    let mut scope = entity.scope(plan, status).with("total", json!(total));
    for (name, e) in fields(&spec["bind"]) {
        let v = scope.eval(e)?;
        scope.push(name, v);
    }
    let mut out = serde_json::Map::new();
    for (name, e) in fields(&spec["fields"]) {
        out.insert(name.clone(), scope.eval(e)?);
    }
    Ok(V::Object(out))
}

/// Commands an action produces: role-validated writes and raw sends.
struct Commands<'p> {
    plan: &'p Plan,
    out: Vec<V>,
}

impl Effects for Commands<'_> {
    fn exec(&mut self, stmt: &V, scope: &mut Scope) -> Result<Flow, EngineError> {
        if let Some(cmd) = stmt.get("command") {
            let role = scope.eval(&cmd["role"]).or_eval()?;
            let value = scope.eval(&cmd["value"]).or_eval()?;
            self.out.push(self.plan.cmd(s(&role), &value)?);
        } else if let Some(send) = stmt.get("send") {
            let code = scope.eval(&send["code"]).or_eval()?;
            let value = scope.eval(&send["value"]).or_eval()?;
            self.out.push(json!({"code":code,"value":value}));
        } else if let Some(message) = stmt.get("fail") {
            return Err(EngineError::InvalidValue(text(
                &scope.eval(message).or_eval()?,
            )));
        } else {
            return Err(EngineError::eval(format!(
                "unknown action statement {stmt}"
            )));
        }
        Ok(Flow::Next)
    }
}

/// DP commands for an entity action, from the spec's `actions` (`*` handles any other action).
pub fn act(
    plan: &Plan,
    action: &str,
    args: &V,
    status: &V,
    rules: &V,
) -> Result<Vec<V>, EngineError> {
    let entity = Entity::of(rules, &plan.platform)
        .ok_or_else(|| EngineError::Unsupported("no write path".into()))?;
    let actions = entity.field("actions");
    let program = actions
        .get(action)
        .or_else(|| actions.get("*"))
        .ok_or_else(|| EngineError::Unsupported("no write path".into()))?;
    let current = state(plan, status, 0.0, rules).or_eval()?;
    let mut scope = entity
        .scope(plan, status)
        .with("action", json!(action))
        .with("args", args)
        .with("state", current);
    let mut commands = Commands { plan, out: vec![] };
    execute(shared(rules, program), &mut scope, &mut commands)?;
    Ok(commands.out)
}

/// Plans for DPs no entity claimed, from the rule pack's `unused` variants.
pub fn unused(d: &V, a: &Adapter, plans: &mut Vec<Plan>, rules: &V) -> Result<(), EngineError> {
    let mut consumed: Vec<_> = plans
        .iter()
        .filter(|p| Entity::of(rules, &p.platform).is_none_or(|e| e.exposed()))
        .flat_map(|p| p.depends_on.clone())
        .collect();
    let mut ids: Vec<_> = a.entries.keys().collect();
    ids.sort_by_key(|id| {
        (
            id.parse::<u64>().is_err(),
            id.parse::<u64>().unwrap_or(0),
            *id,
        )
    });
    for id in ids {
        let DpEntry {
            code, config: ci, ..
        } = &a.entries[id];
        if consumed.contains(code) {
            continue;
        }
        consumed.push(code.clone());
        let e = d["status_range"]
            .get(code)
            .or_else(|| d["function"].get(code))
            .cloned()
            .unwrap_or_else(|| json!({"type":ci.get("valueType").cloned().unwrap_or(json!("String")),"values":ci["valueDesc"]}));
        let kind = normalized(s(&e["type"]));
        if kind.is_empty() {
            continue;
        }
        let spec = parse(&e["values"]);
        if ["Integer", "Enum", "Bitmap"].contains(&kind) && !truth(&spec) {
            continue;
        }
        let mut roles = Roles::new();
        roles.insert(
            s(&rules["unused"]["role"]).into(),
            Role {
                code: code.clone(),
                kind: DpKind::parse(kind).ok_or("invalid dp kind").or_eval()?,
                spec,
                report_type: e["report_type"].clone(),
            },
        );
        let mut scope = Scope::rules(rules)
            .with("device", d)
            .with("status", &d["status"])
            .with("code", json!(code))
            .with("kind", json!(kind))
            .with("rw", json!(d["function"].get(code).is_some()));
        let mut chosen = None;
        for variant in items(&rules["unused"]["variants"]) {
            if truth(&scope.eval(&variant["when"]).or_eval()?) {
                chosen = Some(variant);
                break;
            }
        }
        let Some(variant) = chosen else {
            continue;
        };
        let mut inner = open(&mut scope, json!({}), &roles, &d["status"]);
        let mut build = Build { device: d, roles };
        if execute(shared(rules, &variant["steps"]), &mut inner, &mut build)? == Flow::Stop {
            continue;
        }
        let platform = s(&variant["platform"]);
        plans.push(finish(None, platform, code, build.roles, &mut inner));
    }
    Ok(())
}
