//! The rule language shared by device specs and declarative converters.
//!
//! Expressions are JSON objects with an `op`; any other JSON value is a literal. Programs are
//! statement arrays. Evaluation is pure: no I/O, clock, imports or host callbacks, and every
//! evaluation is bounded by an instruction budget and a nesting limit.
use crate::EngineError;
use crate::adapter::{b64, electricity, hexbytes, hsv_rgb, rgb_hsv};
use crate::data::*;
use serde_json::{Map, Value as V, json};
use std::borrow::Cow;
use std::ops::{Deref, DerefMut};

const EVAL_LIMIT: usize = 1_000_000;
const DEPTH_LIMIT: usize = 64;
pub const STATEMENT_LIMIT: usize = 10_000;
const VALUE_LIMIT: usize = 100_000;
const TEXT_LIMIT: usize = 1_048_576;

fn bounded(value: &V) -> Result<(usize, usize), String> {
    fn visit(value: &V, nodes: &mut usize, bytes: &mut usize, depth: usize) -> Result<(), String> {
        if depth >= DEPTH_LIMIT {
            return Err("rule nesting limit exceeded".into());
        }
        *nodes += 1;
        if *nodes > VALUE_LIMIT {
            return Err("rule value size limit exceeded".into());
        }
        match value {
            V::String(text) => *bytes = bytes.saturating_add(text.len()),
            V::Array(values) => {
                for value in values {
                    visit(value, nodes, bytes, depth + 1)?;
                }
            }
            V::Object(fields) => {
                for (key, value) in fields {
                    *bytes = bytes.saturating_add(key.len());
                    visit(value, nodes, bytes, depth + 1)?;
                }
            }
            _ => {}
        }
        if *bytes > TEXT_LIMIT {
            return Err("rule text size limit exceeded".into());
        }
        Ok(())
    }
    let (mut nodes, mut bytes) = (0, 0);
    visit(value, &mut nodes, &mut bytes, 0)?;
    Ok((nodes, bytes))
}

/// A frame value: borrowed from the caller or owned by the scope.
pub trait Frame<'a> {
    fn frame(self) -> Cow<'a, V>;
}

impl<'a> Frame<'a> for &'a V {
    fn frame(self) -> Cow<'a, V> {
        Cow::Borrowed(self)
    }
}

impl<'a> Frame<'a> for V {
    fn frame(self) -> Cow<'a, V> {
        Cow::Owned(self)
    }
}

/// Named values visible to expressions. The innermost frame with a name wins.
pub struct Scope<'a> {
    frames: Vec<(String, Cow<'a, V>)>,
    functions: &'a V,
    budget: usize,
    depth: usize,
    allocation_nodes: usize,
    allocation_bytes: usize,
}

/// Restores a scope's frames when dropped, so temporary bindings cannot leak.
pub struct Restore<'s, 'a> {
    scope: &'s mut Scope<'a>,
    len: usize,
}

impl<'a> Deref for Restore<'_, 'a> {
    type Target = Scope<'a>;
    fn deref(&self) -> &Scope<'a> {
        self.scope
    }
}

impl<'a> DerefMut for Restore<'_, 'a> {
    fn deref_mut(&mut self) -> &mut Scope<'a> {
        self.scope
    }
}

impl Drop for Restore<'_, '_> {
    fn drop(&mut self) {
        self.scope.frames.truncate(self.len);
    }
}

impl<'a> Scope<'a> {
    /// A scope whose `call` expressions resolve against `functions` (a name -> function map).
    pub fn new(functions: &'a V) -> Self {
        Self {
            frames: vec![],
            functions,
            budget: EVAL_LIMIT,
            depth: 0,
            allocation_nodes: EVAL_LIMIT,
            allocation_bytes: 16 * TEXT_LIMIT,
        }
    }

    /// A scope over a rule pack: `rules` is visible and its `functions` are callable.
    pub fn rules(rules: &'a V) -> Self {
        Self::new(&rules["functions"]).with("rules", rules)
    }

    pub fn with(mut self, name: &str, value: impl Frame<'a>) -> Self {
        self.push(name, value);
        self
    }

    /// Every field of `fields` as its own frame.
    pub fn with_fields(mut self, fields: &'a V) -> Self {
        for (name, value) in fields.as_object().into_iter().flatten() {
            self.frames.push((name.clone(), Cow::Borrowed(value)));
        }
        self
    }

    /// Bindings pushed through the returned guard are removed when it is dropped.
    pub fn mark(&mut self) -> Restore<'_, 'a> {
        let len = self.frames.len();
        Restore { scope: self, len }
    }

    pub fn push(&mut self, name: &str, value: impl Frame<'a>) {
        self.frames.push((name.into(), value.frame()));
    }

    pub fn get(&self, name: &str) -> Option<&V> {
        self.frames
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_ref())
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut V> {
        self.frames
            .iter_mut()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.to_mut())
    }

    /// Replace the innermost frame called `name`, or add one.
    pub fn set(&mut self, name: &str, value: V) {
        match self.get_mut(name) {
            Some(slot) => *slot = value,
            None => self.push(name, value),
        }
    }

    pub fn take(&mut self, name: &str) -> V {
        self.get_mut(name).map(V::take).unwrap_or(V::Null)
    }

    /// Assign at a dotted path (`config.mode` or `["config", "mode"]`); null removes the field.
    pub fn assign(&mut self, path: &[String], value: V) -> Result<(), String> {
        let (frame, rest) = path.split_first().ok_or("empty assignment path")?;
        let target = self
            .get_mut(frame)
            .ok_or_else(|| format!("unknown assignment target {frame}"))?;
        assign(target, rest, value);
        Ok(())
    }

    pub fn eval(&mut self, e: &V) -> Result<V, String> {
        let Some(op) = e.get("op").and_then(V::as_str) else {
            self.charge(e)?;
            return Ok(e.clone());
        };
        self.budget = self
            .budget
            .checked_sub(1)
            .ok_or("rule evaluation limit exceeded")?;
        if self.depth >= DEPTH_LIMIT {
            return Err("rule nesting limit exceeded".into());
        }
        self.depth += 1;
        let result = self.op(op, e);
        self.depth -= 1;
        let result = result?;
        self.charge(&result)?;
        Ok(result)
    }

    fn charge(&mut self, value: &V) -> Result<(), String> {
        let (nodes, bytes) = bounded(value)?;
        self.allocation_nodes = self
            .allocation_nodes
            .checked_sub(nodes)
            .ok_or("rule allocation limit exceeded")?;
        self.allocation_bytes = self
            .allocation_bytes
            .checked_sub(bytes)
            .ok_or("rule allocation limit exceeded")?;
        Ok(())
    }

    fn eval_args(&mut self, e: &V) -> Result<Vec<V>, String> {
        items(&e["args"]).iter().map(|x| self.eval(x)).collect()
    }

    /// Evaluate `body` once per item of `list`, with the item bound to the expression's `as` name.
    fn each(&mut self, e: &V, mut visit: impl FnMut(V, &V) -> bool) -> Result<(), String> {
        let list = self.eval(&e["args"][0])?;
        let name = e.get("as").map(s).unwrap_or("item").to_string();
        for item in items(&list) {
            let mut inner = self.mark();
            inner.push(&name, item.clone());
            let result = inner.eval(&e["do"])?;
            if !visit(item.clone(), &result) {
                break;
            }
        }
        Ok(())
    }

    fn path(&mut self, e: &V) -> Result<Vec<V>, String> {
        let parts = &e["path"];
        if let Some(text) = parts.as_str() {
            return Ok(text.split('.').map(|p| json!(p)).collect());
        }
        items(parts).iter().map(|p| self.eval(p)).collect()
    }

    fn op(&mut self, op: &str, e: &V) -> Result<V, String> {
        match op {
            "literal" => return Ok(e["value"].clone()),
            "get" => {
                let parts = self.path(e)?;
                let found = parts.split_first().and_then(|(head, rest)| {
                    rest.iter()
                        .try_fold(self.get(s(head))?, |v, part| index(v, part))
                        .cloned()
                });
                return match found {
                    Some(v) => Ok(v),
                    None => self.eval(&e["default"]),
                };
            }
            "if" => {
                let yes = truth(&self.eval(&e["condition"])?);
                return self.eval(if yes { &e["then"] } else { &e["else"] });
            }
            "and" | "or" => {
                let want = op == "or";
                for x in items(&e["args"]) {
                    if truth(&self.eval(x)?) == want {
                        return Ok(json!(want));
                    }
                }
                return Ok(json!(!want));
            }
            "coalesce" => {
                for x in items(&e["args"]) {
                    let v = self.eval(x)?;
                    if !v.is_null() {
                        return Ok(v);
                    }
                }
                return Ok(V::Null);
            }
            "let" => {
                let mut inner = self.mark();
                for (name, x) in e["bind"].as_object().into_iter().flatten() {
                    let v = inner.eval(x)?;
                    inner.push(name, v);
                }
                return inner.eval(&e["in"]);
            }
            "record" => {
                let mut out = Map::new();
                for (name, x) in e["fields"].as_object().into_iter().flatten() {
                    out.insert(name.clone(), self.eval(x)?);
                }
                return Ok(V::Object(out));
            }
            "call" => {
                let name = s(&e["fn"]);
                let function = &self.functions[name];
                if !function.is_object() {
                    return Err(format!("unknown function {name}"));
                }
                let args = self.eval_args(e)?;
                let mut inner = self.mark();
                for (i, param) in items(&function["params"]).iter().enumerate() {
                    inner.push(s(param), args.get(i).cloned().unwrap_or(V::Null));
                }
                return inner.eval(&function["body"]);
            }
            "error" => return Err(s(&self.eval(&e["message"])?).to_string()),
            "map" => {
                let mut out = vec![];
                self.each(e, |_, v| {
                    out.push(v.clone());
                    true
                })?;
                return Ok(V::Array(out));
            }
            "filter" => {
                let mut out = vec![];
                self.each(e, |item, keep| {
                    if truth(keep) {
                        out.push(item);
                    }
                    true
                })?;
                return Ok(V::Array(out));
            }
            "find" => {
                let mut found = V::Null;
                self.each(e, |item, hit| {
                    if truth(hit) {
                        found = item;
                    }
                    !truth(hit)
                })?;
                return Ok(found);
            }
            "any" | "all" => {
                let want = op == "any";
                let mut result = !want;
                self.each(e, |_, v| {
                    if truth(v) == want {
                        result = want;
                    }
                    result != want
                })?;
                return Ok(json!(result));
            }
            "count" => {
                let mut count = 0;
                self.each(e, |_, v| {
                    count += usize::from(truth(v));
                    true
                })?;
                return Ok(json!(count));
            }
            "flags" => {
                let table = s(&self.eval(&e["table"])?).to_string();
                let mut bits = 0i64;
                for (flag, x) in e["when"].as_object().into_iter().flatten() {
                    if truth(&self.eval(x)?) {
                        bits |= self.feature(&table, flag);
                    }
                }
                return Ok(json!(bits));
            }
            _ => {}
        }
        let args = self.eval_args(e)?;
        let arg = |i: usize| args.get(i).unwrap_or(&V::Null);
        let (a, b) = (arg(0), arg(1));
        let numbers = |xs: &[&V]| xs.iter().all(|x| x.is_number());
        if ["concat", "flatten", "list", "zip", "object"].contains(&op) {
            let nodes = args.iter().try_fold(0usize, |sum, v| {
                bounded(v).map(|(nodes, _)| sum.saturating_add(nodes))
            })?;
            if nodes > VALUE_LIMIT {
                return Err("rule value size limit exceeded".into());
            }
        }
        Ok(match op {
            "eq" => json!(eq(a, b)),
            "same" => json!(a == b),
            "ne" => json!(!eq(a, b)),
            "not" => json!(!truth(a)),
            "truthy" => json!(truth(a)),
            "lt" | "le" | "gt" | "ge" => json!(
                numbers(&[a, b])
                    && match op {
                        "lt" => n(a) < n(b),
                        "le" => n(a) <= n(b),
                        "gt" => n(a) > n(b),
                        _ => n(a) >= n(b),
                    }
            ),
            "add" | "sub" | "mul" | "div" | "min" | "max" => {
                if !numbers(&[a, b]) {
                    V::Null
                } else {
                    number(match op {
                        "add" => n(a) + n(b),
                        "sub" => n(a) - n(b),
                        "mul" => n(a) * n(b),
                        "div" => n(a) / n(b),
                        "min" => n(a).min(n(b)),
                        _ => n(a).max(n(b)),
                    })
                }
            }
            "round_half_even" | "floor" => {
                if a.is_number() {
                    number(if op == "floor" {
                        n(a).floor()
                    } else {
                        n(a).round_ties_even()
                    })
                } else {
                    V::Null
                }
            }
            "remap" => {
                if numbers(&args.iter().collect::<Vec<_>>()) && args.len() == 5 {
                    number(remap(n(a), n(b), n(arg(2)), n(arg(3)), n(arg(4))))
                } else {
                    V::Null
                }
            }
            "num" => number(n(a)),
            "is_number" => json!(a.is_number()),
            "is_integer" => json!(a.is_i64() || a.is_u64()),
            "bit_test" => json!(
                a.is_number()
                    && (n(a) as i64)
                        .checked_shr(n(b) as u32)
                        .is_some_and(|x| x & 1 != 0)
            ),
            "is_null" => json!(a.is_null()),
            "type_of" => json!(match a {
                V::Null => "null",
                V::Bool(_) => "boolean",
                V::Number(_) => "number",
                V::String(_) => "string",
                V::Array(_) => "array",
                V::Object(_) => "object",
            }),
            "some" => {
                if truth(a) {
                    a.clone()
                } else {
                    V::Null
                }
            }
            "contains" => json!(match a {
                V::Array(xs) => xs.iter().any(|v| eq(v, b)),
                V::Object(o) => o.contains_key(s(b)),
                V::String(text) => b.is_string() && text.contains(s(b)),
                _ => false,
            }),
            "lookup" => index(a, b).cloned().unwrap_or_else(|| e["default"].clone()),
            "list" => json!(args),
            "concat" => V::Array(
                args.iter()
                    .flat_map(|x| match x {
                        V::Array(xs) => xs.clone(),
                        V::Null => vec![],
                        other => vec![other.clone()],
                    })
                    .collect(),
            ),
            "flatten" => V::Array(items(a).iter().flat_map(arr).collect()),
            "len" => match a {
                V::Array(xs) => json!(xs.len()),
                V::Object(o) => json!(o.len()),
                V::String(text) => json!(text.chars().count()),
                _ => V::Null,
            },
            "index_of" => items(a)
                .iter()
                .position(|x| x == b)
                .map_or(V::Null, |i| json!(i)),
            "without" => V::Array(items(a).iter().filter(|x| *x != b).cloned().collect()),
            "sort" => {
                let mut xs = arr(a);
                xs.sort_by(|x, y| s(x).cmp(s(y)));
                V::Array(xs)
            }
            "zip" => V::Array(
                items(a)
                    .iter()
                    .zip(items(b))
                    .map(|(x, y)| json!([x, y]))
                    .collect(),
            ),
            "range" => {
                let count = n(a).max(0.0);
                if !count.is_finite() || count >= VALUE_LIMIT as f64 {
                    return Err("rule value size limit exceeded".into());
                }
                V::Array((0..count as usize).map(|i| json!(i)).collect())
            }
            "keys" => V::Array(fields(a).map(|(k, _)| json!(k)).collect()),
            "values" => V::Array(fields(a).map(|(_, v)| v.clone()).collect()),
            "entries" => V::Array(fields(a).map(|(k, v)| json!([k, v])).collect()),
            "object" => V::Object(
                items(a)
                    .iter()
                    .map(|pair| (s(&pair[0]).to_string(), pair[1].clone()))
                    .collect(),
            ),
            "codes" => json!(codes(a)),
            "text" => json!(s(a)),
            "to_string" => json!(text(a)),
            "str_concat" => {
                let mut out = String::new();
                for arg in &args {
                    let part = text(arg);
                    if out.len().saturating_add(part.len()) > TEXT_LIMIT {
                        return Err("rule text size limit exceeded".into());
                    }
                    out.push_str(&part);
                }
                json!(out)
            }
            "lower" => a.as_str().map_or(V::Null, |t| json!(t.to_lowercase())),
            "capitalize" => a.as_str().map_or(V::Null, |t| {
                let mut chars = t.chars();
                json!(
                    chars
                        .next()
                        .map(|c| c.to_uppercase().collect::<String>()
                            + &chars.as_str().to_lowercase())
                        .unwrap_or_default()
                )
            }),
            "starts_with" => json!(a.is_string() && s(a).starts_with(s(b))),
            "ends_with" => json!(a.is_string() && s(a).ends_with(s(b))),
            "trim_start" => a
                .as_str()
                .map_or(V::Null, |t| json!(t.trim_start_matches(s(b)))),
            "trim_end" => a
                .as_str()
                .map_or(V::Null, |t| json!(t.trim_end_matches(s(b)))),
            "replace" => {
                if let Some(t) = a.as_str() {
                    let count = t.matches(s(b)).count();
                    let size = t
                        .len()
                        .saturating_sub(count.saturating_mul(s(b).len()))
                        .saturating_add(count.saturating_mul(s(arg(2)).len()));
                    if size > TEXT_LIMIT {
                        return Err("rule text size limit exceeded".into());
                    }
                    json!(t.replace(s(b), s(arg(2))))
                } else {
                    V::Null
                }
            }
            "hex" => {
                let width = n(b);
                if !width.is_finite() || width < 0.0 || width > TEXT_LIMIT as f64 {
                    return Err("rule text size limit exceeded".into());
                }
                json!(format!("{:0width$x}", n(a) as i64, width = width as usize))
            }
            "hex_ints" => hex_ints(a, b, arg(2))?,
            "parse_json" => parse(a),
            "b64_utf8" => b64(a)
                .and_then(|x| String::from_utf8(x).ok())
                .map_or(V::Null, |t| json!(t)),
            "b64_utf16be" => b64(a)
                .filter(|x| x.len() % 2 == 0)
                .and_then(|x| {
                    String::from_utf16(
                        &x.as_chunks::<2>()
                            .0
                            .iter()
                            .map(|c| u16::from_be_bytes(*c))
                            .collect::<Vec<_>>(),
                    )
                    .ok()
                })
                .map_or(V::Null, |t| json!(t)),
            "electricity_b64" => b64(a).map_or(V::Null, |x| electricity(&x)),
            "electricity_hex" => hexbytes(s(a)).map_or(V::Null, |x| electricity(&x)),
            "hs_to_rgb_hex" => {
                let rgb = hsv_rgb(
                    n(&a[0]).rem_euclid(360.0) / 360.0,
                    n(&a[1]).clamp(0.0, 100.0) / 100.0,
                    1.0,
                );
                let [r, g, b] = rgb.map(|x| (x * 255.0).round_ties_even() as i64);
                json!(format!("#{r:02x}{g:02x}{b:02x}"))
            }
            "rgb_hex_to_hs" => s(a)
                .strip_prefix('#')
                .and_then(hexbytes)
                .filter(|x| x.len() == 3)
                .map_or(V::Null, |x| {
                    let [h, sat, _] = rgb_hsv(
                        x[0] as f64 / 255.0,
                        x[1] as f64 / 255.0,
                        x[2] as f64 / 255.0,
                    );
                    json!([h * 360.0, sat * 100.0])
                }),
            "flag" => json!(n(arg(2)) as i64 & self.feature(s(a), s(b)) != 0),
            _ => return Err(format!("unknown expression op {op}")),
        })
    }

    fn feature(&self, table: &str, flag: &str) -> i64 {
        self.get("rules")
            .map_or(0.0, |rules| n(&rules["features"][table][flag])) as i64
    }
}

fn index<'v>(v: &'v V, key: &V) -> Option<&'v V> {
    match v {
        V::Object(o) => o.get(s(key)),
        V::Array(xs) => key
            .as_u64()
            .or_else(|| key.as_str().and_then(|t| t.parse().ok()))
            .and_then(|i| xs.get(i as usize)),
        _ => None,
    }
}

fn assign(target: &mut V, path: &[String], value: V) {
    let Some((last, parents)) = path.split_last() else {
        *target = value;
        return;
    };
    let mut v = target;
    for part in parents {
        if !v.is_object() {
            *v = json!({});
        }
        v = v
            .as_object_mut()
            .expect("object")
            .entry(part.clone())
            .or_insert_with(|| json!({}));
    }
    if !v.is_object() {
        *v = json!({});
    }
    let o = v.as_object_mut().expect("object");
    if value.is_null() {
        o.remove(last);
    } else {
        o.insert(last.clone(), value);
    }
}

/// A value as text: strings verbatim, integral numbers without a fraction.
pub fn text(v: &V) -> String {
    match v {
        V::String(t) => t.clone(),
        V::Number(_) => {
            let x = n(v);
            if x.fract() == 0.0 && x.abs() < 1e15 {
                format!("{}", x as i64)
            } else {
                v.to_string()
            }
        }
        V::Null => String::new(),
        other => other.to_string(),
    }
}

fn hex_ints(v: &V, width: &V, count: &V) -> Result<V, String> {
    let dimension = |value: &V, min: f64, limit: usize| {
        value
            .as_f64()
            .filter(|x| x.is_finite() && x.fract() == 0.0 && *x >= min && *x <= limit as f64)
            .map(|x| x as usize)
            .ok_or_else(|| "invalid hex_ints dimensions".to_string())
    };
    let width = dimension(width, 1.0, TEXT_LIMIT)?;
    let count = dimension(count, 0.0, VALUE_LIMIT - 1)?;
    let len = width
        .checked_mul(count)
        .filter(|len| *len <= TEXT_LIMIT)
        .ok_or("rule text size limit exceeded")?;
    let Some(t) = v.as_str().filter(|t| t.is_ascii() && t.len() == len) else {
        return Ok(V::Null);
    };
    Ok(t.as_bytes()
        .chunks_exact(width)
        .map(|chunk| {
            std::str::from_utf8(chunk)
                .ok()
                .and_then(|chunk| u64::from_str_radix(chunk, 16).ok())
        })
        .collect::<Option<Vec<_>>>()
        .map_or(V::Null, |xs| json!(xs)))
}

/// Whether a program keeps going or stopped on purpose (a failed requirement).
#[derive(PartialEq, Eq)]
pub enum Flow {
    Next,
    Stop,
}

/// The statements a program's host understands beyond the shared control flow.
pub trait Effects {
    fn exec(&mut self, stmt: &V, scope: &mut Scope) -> Result<Flow, EngineError>;
}

/// Run a statement array. `if`/`then`/`else` and `let` are shared; other statements go to `fx`.
fn run(
    program: &V,
    scope: &mut Scope,
    fx: &mut impl Effects,
    budget: &mut usize,
    depth: usize,
) -> Result<Flow, EngineError> {
    if depth >= DEPTH_LIMIT {
        return Err("rule nesting limit exceeded".into());
    }
    if !program.is_null() && !program.is_array() {
        return Err("rule program must be an array".into());
    }
    for stmt in items(program) {
        *budget = budget
            .checked_sub(1)
            .ok_or("rule instruction limit exceeded")?;
        let flow = if let Some(cond) = stmt.get("if") {
            let branch = if truth(&scope.eval(cond)?) {
                &stmt["then"]
            } else {
                &stmt["else"]
            };
            run(branch, scope, fx, budget, depth + 1)?
        } else if let Some(name) = stmt.get("$use") {
            let program = scope
                .get("rules")
                .map(|rules| rules["templates"][s(name)].clone())
                .ok_or("templates need a rule pack")?;
            run(&program, scope, fx, budget, depth + 1)?
        } else if let Some(name) = stmt.get("let") {
            let value = scope.eval(&stmt["value"])?;
            scope.set(s(name), value);
            Flow::Next
        } else {
            fx.exec(stmt, scope)?
        };
        if flow == Flow::Stop {
            return Ok(Flow::Stop);
        }
    }
    Ok(Flow::Next)
}

/// Run a program with a fresh statement budget.
pub fn execute(program: &V, scope: &mut Scope, fx: &mut impl Effects) -> Result<Flow, EngineError> {
    run(program, scope, fx, &mut STATEMENT_LIMIT.clone(), 0)
}

/// A `set` target as path parts: `"config.mode"` or `["config", "mode"]`.
pub fn target(v: &V) -> Vec<String> {
    match v.as_str() {
        Some(t) => t.split('.').map(String::from).collect(),
        None => codes(v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(e: V) -> Result<V, String> {
        Scope::new(&V::Null)
            .with("x", json!({"a": [1, 2, 3]}))
            .eval(&e)
    }

    #[test]
    fn bindings_are_scoped_and_dropped() {
        let mut scope = Scope::new(&V::Null).with("x", json!(1));
        {
            let mut inner = scope.mark();
            inner.push("x", json!(2));
            assert_eq!(inner.get("x"), Some(&json!(2)));
        }
        assert_eq!(scope.get("x"), Some(&json!(1)));
        let e =
            json!({"op": "map", "args": [[10, 20]], "as": "x", "do": {"op": "get", "path": "x"}});
        assert_eq!(scope.eval(&e).unwrap(), json!([10, 20]));
        assert_eq!(scope.get("x"), Some(&json!(1)));
    }

    #[test]
    fn paths_index_arrays_and_evaluate_segments() {
        assert_eq!(
            eval(json!({"op": "get", "path": "x.a.1"})).unwrap(),
            json!(2)
        );
        let dynamic = json!({"op": "get", "path": ["x", "a", {"op": "add", "args": [1, 1]}]});
        assert_eq!(eval(dynamic).unwrap(), json!(3));
        let missing = json!({"op": "get", "path": "x.b", "default": {"op": "add", "args": [1, 2]}});
        assert_eq!(eval(missing).unwrap(), json!(3));
    }

    #[test]
    fn short_circuits_skip_errors() {
        let boom = json!({"op": "error", "message": "boom"});
        assert_eq!(
            eval(json!({"op": "or", "args": [true, boom]})).unwrap(),
            json!(true)
        );
        assert_eq!(
            eval(json!({"op": "coalesce", "args": [1, boom]})).unwrap(),
            json!(1)
        );
        assert_eq!(
            eval(json!({"op": "if", "condition": false, "then": boom, "else": 2})).unwrap(),
            json!(2)
        );
        assert_eq!(eval(boom).unwrap_err(), "boom");
    }

    #[test]
    fn functions_bind_parameters() {
        let functions = json!({"twice": {"params": ["v"], "body": {"op": "mul", "args": [{"op": "get", "path": "v"}, 2]}}});
        let e = json!({"op": "call", "fn": "twice", "args": [21]});
        assert_eq!(Scope::new(&functions).eval(&e).unwrap(), json!(42));
        assert!(
            Scope::new(&functions)
                .eval(&json!({"op": "call", "fn": "nope"}))
                .is_err()
        );
    }

    #[test]
    fn runaway_rules_are_bounded() {
        let functions = json!({"loop": {"params": [], "body": {"op": "call", "fn": "loop"}}});
        let e = json!({"op": "call", "fn": "loop"});
        assert_eq!(
            Scope::new(&functions).eval(&e).unwrap_err(),
            "rule nesting limit exceeded"
        );
    }

    #[test]
    fn hex_ints_validates_dimensions_and_preserves_decoding() {
        for (width, count) in [
            (json!(9_223_372_036_854_775_808u64), json!(2)),
            (json!(0), json!(2)),
            (json!(-1), json!(2)),
            (json!(1.5), json!(2)),
            (json!(2), json!(-1)),
            (json!(2), json!(1.5)),
            (json!(2), json!(VALUE_LIMIT)),
            (json!(TEXT_LIMIT), json!(2)),
            (json!("2"), json!(2)),
        ] {
            assert!(eval(json!({"op":"hex_ints","args":["",width,count]})).is_err());
        }
        for (text, width, count, expected) in [
            ("00ff", 2.0, 2.0, json!([0, 255])),
            ("", 2.0, 0.0, json!([])),
            ("zz", 2.0, 1.0, V::Null),
            ("0", 2.0, 1.0, V::Null),
            ("é", 2.0, 1.0, V::Null),
        ] {
            assert_eq!(
                eval(json!({"op":"hex_ints","args":[text,width,count]})).unwrap(),
                expected
            );
        }
    }
}
