//! The tree-walking evaluator. Ported from jsonata-js `src/jsonata.js` (MIT).
//!
//! The structure mirrors the reference implementation function by function (`evaluatePath`,
//! `evaluateStep`, `evaluateTupleStep`, `evaluateFilter`, ...), because JSONata's semantics
//! around sequences, singleton arrays and tuple streams are defined by that code. Evaluation is
//! synchronous; there is no equivalent of jsonata-js's async functions.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::{Arc, OnceLock};

use indexmap::IndexMap;
use web_time::{Duration, Instant};

use crate::ast::{Ast, BinaryOp, Node, NodeKind, SortTerm, Stage, TransformDef};
use crate::audit::{Branch, EvalInfo, Outcome, Tracer, ValueRef};
use crate::error::{Error, Result};
use crate::functions::{self, Call, boolean};
use crate::regex::Regex;
use crate::signature::Signature;
use crate::value::{
    self, CONS, Function, KEEP_SINGLETON, Lambda, Map, OUTER_WRAPPER, SEQUENCE, TUPLE_STREAM, Value,
};

/// A scope of variable bindings.
pub(crate) struct Frame {
    vars: RefCell<HashMap<String, Value, foldhash::fast::RandomState>>,
    parent: Option<Env>,
}

/// A shared reference to a [`Frame`].
pub(crate) type Env = Rc<Frame>;

impl Frame {
    fn new(parent: Option<&Env>) -> Env {
        Rc::new(Frame {
            vars: RefCell::new(HashMap::default()),
            parent: parent.cloned(),
        })
    }

    /// Looks a variable up in this frame and its ancestors, then among the built-ins.
    pub fn lookup(&self, name: &str) -> Value {
        let mut frame = self;
        loop {
            if let Some(v) = frame.vars.borrow().get(name) {
                return v.clone();
            }
            match &frame.parent {
                Some(p) => frame = p,
                None => return functions::native_value(name).unwrap_or_default(),
            }
        }
    }
}

/// Limits applied to one evaluation, its cancellation token, and its fixed clock and seed.
#[derive(Debug, Clone)]
pub(crate) struct Limits {
    pub max_depth: usize,
    pub timeout: Option<Duration>,
    pub max_string_length: usize,
    pub max_array_length: usize,
    pub max_result_size: usize,
    pub cancel_token: Option<crate::CancelToken>,
    /// Milliseconds since the epoch to use instead of the system clock.
    pub now: Option<i64>,
    /// Seed for `$random` and `$shuffle` instead of one taken from the system clock.
    pub seed: Option<u64>,
}

/// Default maximum evaluation depth. jsonata-js has no limit of its own and relies on the
/// JavaScript stack; jlens needs one to fail cleanly instead of overflowing the native stack.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const DEFAULT_MAX_DEPTH: usize = 50_000;
#[cfg(target_arch = "wasm32")]
pub(crate) const DEFAULT_MAX_DEPTH: usize = 1_000;

/// Default maximum length of a built string, in bytes (256 MiB). jsonata-js is limited by the
/// JavaScript engine (about 2^29 UTF-16 units in V8); jlens fails cleanly instead of running
/// out of memory.
pub(crate) const DEFAULT_MAX_STRING_LENGTH: usize = 1 << 28;
/// Default maximum number of items of a built array (above the 10^7 of a range, `D2014`).
pub(crate) const DEFAULT_MAX_ARRAY_LENGTH: usize = 1 << 24;
/// Default maximum size of a result as JSON text, in bytes (256 MiB, like a string): a value
/// nested in itself a few times can stand for far more text than the memory it uses.
pub(crate) const DEFAULT_MAX_RESULT_SIZE: usize = 1 << 28;

/// The state of one evaluation.
pub(crate) struct Evaluator<'t> {
    depth: usize,
    limits: Limits,
    started: Instant,
    /// Frames that may be part of reference cycles (a frame binding a closure that captures
    /// it). Their bindings are cleared when the evaluation ends so the memory is reclaimed.
    cyclic: Vec<Weak<Frame>>,
    regexes: HashMap<(String, String), Rc<Regex>>,
    /// Milliseconds since the epoch, fixed for the whole evaluation (`$now`, `$millis`).
    timestamp: f64,
    /// State of the xorshift generator behind `$random` and `$shuffle`.
    rng: u64,
    /// Receives evaluation events (auditing, logging). `None` costs one check per hook.
    tracer: Option<&'t mut dyn Tracer>,
}

impl Drop for Evaluator<'_> {
    fn drop(&mut self) {
        for frame in self.cyclic.drain(..) {
            if let Some(frame) = frame.upgrade() {
                frame.vars.borrow_mut().clear();
            }
        }
    }
}

fn err_at(code: &str, node: &Node) -> Error {
    Error::new(code).at(node.span, node.position)
}

/// Adds the value as the `value` detail of an error (omitted for undefined, like jsonata-js).
trait WithValue {
    fn with_value(self, key: &str, v: &Value) -> Self;
}

impl WithValue for Error {
    fn with_value(self, key: &str, v: &Value) -> Self {
        match v.to_json_detail() {
            Some(json) => self.with(key, json),
            None => self,
        }
    }
}

fn js_truthy(v: &Value) -> bool {
    match v {
        Value::Undefined | Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => *n != 0.0 && !n.is_nan(),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

fn tuple_get(tuple: &Value, key: &str) -> Value {
    match tuple {
        Value::Object(o) => o.borrow().get(key).cloned().unwrap_or_default(),
        _ => Value::Undefined,
    }
}

fn tuple_map(tuple: &Value) -> Map {
    match tuple {
        Value::Object(o) => o.borrow().clone(),
        _ => Map::default(),
    }
}

/// `function($f, $g) { function($x){ $g($f($x)) } }`, used for `$f ~> $g` function chaining.
fn chain_ast() -> &'static Ast {
    static CHAIN: OnceLock<Ast> = OnceLock::new();
    CHAIN.get_or_init(|| {
        crate::parse("function($f, $g) { function($x){ $g($f($x)) } }").expect("valid chain")
    })
}

fn transform_signature() -> &'static Signature {
    static SIG: OnceLock<Signature> = OnceLock::new();
    SIG.get_or_init(|| Signature::parse("<(oa):o>").expect("valid signature"))
}

/// Evaluates a parsed expression. This is the entry point used by `Expression::evaluate_with`.
pub(crate) fn evaluate(
    ast: &Ast,
    input: Value,
    bindings: &crate::Bindings,
    limits: Limits,
    tracer: Option<&mut dyn Tracer>,
) -> Result<Option<serde_json::Value>> {
    if ast.has_errors() {
        return Err(Error::new("S0500").at(Default::default(), 0));
    }
    let mut ev = Evaluator {
        depth: 0,
        limits,
        started: Instant::now(),
        cyclic: Vec::new(),
        regexes: HashMap::new(),
        timestamp: 0.0,
        rng: 0,
        tracer,
    };
    let since_epoch = || {
        web_time::SystemTime::now()
            .duration_since(web_time::UNIX_EPOCH)
            .unwrap_or_default()
    };
    ev.timestamp = match ev.limits.now {
        Some(millis) => millis as f64,
        None => since_epoch().as_millis() as f64,
    };
    ev.rng = match ev.limits.seed {
        Some(seed) => splitmix64(seed) | 1,
        None => (since_epoch().as_nanos() as u64) | 1,
    };
    let env = Frame::new(None);
    for (name, value) in bindings.iter() {
        ev.bind(&env, name, Value::from_json(value));
    }
    for (name, function) in bindings.functions() {
        ev.bind(
            &env,
            name,
            Value::function(Function::Host(function.clone())),
        );
    }
    let mut input = input;
    ev.bind(&env, "$", input.clone());
    if let Value::Array(a) = &input
        && !a.is_sequence()
    {
        // An input array is wrapped so that it counts as a single context value.
        input = Value::array(vec![input.clone()], SEQUENCE | OUTER_WRAPPER);
    }
    if let Some(t) = ev.tracer.as_deref_mut() {
        t.begin(&EvalInfo {
            source: &ast.source,
            timestamp: ev.timestamp,
        });
    }
    let result = ev.eval(&ast.root, &input, &env).and_then(|v| {
        let limit = ev.limits.max_result_size;
        let json = v.to_json_within(limit).map_err(|_| {
            Error::new("U1001").with(
                "message",
                format!("The result would exceed the size limit of {limit} bytes"),
            )
        })?;
        Ok((v, json))
    });
    if let Some(t) = ev.tracer.as_deref_mut() {
        match &result {
            Ok((v, _)) => t.end(Outcome::Value(ValueRef(v))),
            Err(e) => t.end(Outcome::Error(e)),
        }
    }
    Ok(result?.1)
}

/// Spreads a user seed over all 64 bits, so that nearby seeds start the generator far apart.
fn splitmix64(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Evaluator<'_> {
    /// The evaluation timestamp in milliseconds since the epoch.
    pub(crate) fn timestamp(&self) -> f64 {
        self.timestamp
    }

    /// A pseudo-random number in [0, 1).
    pub(crate) fn random(&mut self) -> f64 {
        // xorshift64*
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }

    pub(crate) fn bind(&mut self, env: &Env, name: &str, value: Value) {
        if matches!(
            value,
            Value::Function(_) | Value::Array(_) | Value::Object(_)
        ) {
            self.cyclic.push(Rc::downgrade(env));
        }
        env.vars.borrow_mut().insert(name.to_owned(), value);
    }

    fn child(&mut self, parent: &Env) -> Env {
        Frame::new(Some(parent))
    }

    /// A frame binding every field of a tuple (`createFrameFromTuple`).
    fn frame_from_tuple(&mut self, env: &Env, tuple: &Value) -> Env {
        let frame = self.child(env);
        if let Value::Object(o) = tuple {
            for (k, v) in o.borrow().iter() {
                self.bind(&frame, k, v.clone());
            }
        }
        frame
    }

    fn guardrails(&self) -> Result<()> {
        if self.depth > self.limits.max_depth {
            return Err(Error::new("D1011").with("value", self.limits.max_depth));
        }
        self.time_limits()
    }

    /// Fails with `D1012` once the timeout has passed, or `U1003` once the token is cancelled.
    fn time_limits(&self) -> Result<()> {
        if let Some(timeout) = self.limits.timeout
            && self.started.elapsed() > timeout
        {
            return Err(Error::new("D1012").with("value", timeout.as_millis() as u64));
        }
        if let Some(token) = &self.limits.cancel_token
            && token.is_cancelled()
        {
            return Err(Error::new("U1003"));
        }
        Ok(())
    }

    /// Finds the first match of `re` at or after byte offset `from`. With a timeout or a
    /// cancellation token, the backtracker polls them and the match fails as an evaluation step
    /// would (ADR 0009); without either, it runs the unchecked code path.
    pub(crate) fn find_regex(
        &self,
        re: &regress::Regex,
        text: &str,
        from: usize,
    ) -> Result<Option<regress::Match>> {
        if self.limits.timeout.is_none() && self.limits.cancel_token.is_none() {
            return Ok(re.find_from(text, from).next());
        }
        let mut stopped = None;
        let found = re.find_from_interruptible(text, from, || {
            stopped = self.time_limits().err();
            stopped.is_some()
        });
        match found {
            Ok(m) => Ok(m),
            Err(regress::Interrupted) => Err(stopped.unwrap_or_else(|| Error::new("U1003"))),
        }
    }

    /// Fails with `U1001` when a string of `len` bytes would exceed the size limit.
    pub(crate) fn check_string(&self, len: usize) -> Result<()> {
        let limit = self.limits.max_string_length;
        if len > limit {
            return Err(value::string_too_long(limit));
        }
        Ok(())
    }

    /// The size limit of a built string, in bytes.
    pub(crate) fn max_string_length(&self) -> usize {
        self.limits.max_string_length
    }

    /// Fails with `U1001` when an array of `len` items would exceed the size limit.
    pub(crate) fn check_array(&self, len: usize) -> Result<()> {
        let limit = self.limits.max_array_length;
        if len > limit {
            return Err(array_too_long(limit));
        }
        Ok(())
    }

    /// Checks the size of a value returned by a built-in or host function.
    fn check_size(&self, v: Value) -> Result<Value> {
        match &v {
            Value::String(s) => self.check_string(s.len())?,
            Value::Array(a) => self.check_array(a.items.len())?,
            _ => {}
        }
        Ok(v)
    }

    /// Evaluates `expr` against `input` (jsonata-js `evaluate`).
    #[inline]
    pub(crate) fn eval(&mut self, expr: &Node, input: &Value, env: &Env) -> Result<Value> {
        if self.tracer.is_some() {
            return self.eval_traced(expr, input, env);
        }
        self.eval_grow(expr, input, env)
    }

    /// Evaluates with stack growth on native targets, and a stack check on WebAssembly.
    #[inline]
    fn eval_grow(&mut self, expr: &Node, input: &Value, env: &Env) -> Result<Value> {
        if crate::stack::exhausted() {
            return Err(Error::new("D1011").with("value", self.depth));
        }
        crate::stack::grow(|| self.eval_inner(expr, input, env))
    }

    /// [`Self::eval`] with `enter`/`exit` events, kept out of the untraced hot path.
    #[cold]
    #[inline(never)]
    fn eval_traced(&mut self, expr: &Node, input: &Value, env: &Env) -> Result<Value> {
        if let Some(t) = self.tracer.as_deref_mut() {
            t.enter(expr, ValueRef(input));
        }
        let result = self.eval_grow(expr, input, env);
        if let Some(t) = self.tracer.as_deref_mut() {
            match &result {
                Ok(v) => t.exit(expr, Outcome::Value(ValueRef(v))),
                Err(e) => t.exit(expr, Outcome::Error(e)),
            }
        }
        result
    }

    fn eval_inner(&mut self, expr: &Node, input: &Value, env: &Env) -> Result<Value> {
        self.depth += 1;
        self.guardrails()?;

        let mut result = match &expr.kind {
            NodeKind::Path {
                steps,
                keep_singleton_array,
            } => self.path(expr, steps, *keep_singleton_array, input, env)?,
            NodeKind::Binary { op, lhs, rhs } => self.binary(expr, *op, lhs, rhs, input, env)?,
            NodeKind::Negate { expr: operand } => {
                let v = self.eval(operand, input, env)?;
                let numeric = v.is_numeric().map_err(|e| e.at(expr.span, expr.position))?;
                match v {
                    Value::Undefined => Value::Undefined,
                    Value::Number(n) if numeric => Value::Number(-n),
                    other => {
                        return Err(err_at("D1002", expr)
                            .with("token", "-")
                            .with_value("value", &other));
                    }
                }
            }
            NodeKind::Array { items } => self.array_constructor(expr, items, input, env)?,
            NodeKind::Object { pairs } => self.group(pairs, expr, input.clone(), env)?,
            NodeKind::Name { name } => functions::lookup(input, name),
            NodeKind::String { value } => Value::string(value.as_str()),
            NodeKind::Number { value } => Value::Number(*value),
            NodeKind::Bool { value } => Value::Bool(*value),
            NodeKind::Null => Value::Null,
            NodeKind::Wildcard => wildcard(input, self.limits.max_array_length)?,
            NodeKind::Descendant => descendants(input, self.limits.max_array_length)?,
            NodeKind::Parent { label, .. } => env.lookup(label),
            NodeKind::Condition {
                condition,
                then,
                otherwise,
            } => {
                let c = self.eval(condition, input, env)?;
                let holds = boolean(&c).map_err(|e| e.at(expr.span, expr.position))? == Some(true);
                if let Some(t) = self.tracer.as_deref_mut() {
                    let taken = match (holds, otherwise.is_some()) {
                        (true, _) => Branch::Then,
                        (false, true) => Branch::Else,
                        (false, false) => Branch::None,
                    };
                    t.branch(expr, taken);
                }
                if holds {
                    self.eval(then, input, env)?
                } else if let Some(e) = otherwise {
                    self.eval(e, input, env)?
                } else {
                    Value::Undefined
                }
            }
            NodeKind::Block { exprs } => {
                let frame = self.child(env);
                let mut result = Value::Undefined;
                for e in exprs {
                    result = self.eval(e, input, &frame)?;
                }
                result
            }
            NodeKind::Bind { lhs, rhs } => {
                let value = self.eval(rhs, input, env)?;
                if let NodeKind::Variable { name } = &lhs.kind {
                    if let Some(t) = self.tracer.as_deref_mut() {
                        t.bind(expr, name, ValueRef(&value));
                    }
                    self.bind(env, name, value.clone());
                }
                value
            }
            NodeKind::Regex { pattern, flags } => {
                let key = (pattern.clone(), flags.clone());
                let re = match self.regexes.get(&key) {
                    Some(re) => re.clone(),
                    None => {
                        let re = Rc::new(
                            Regex::new(pattern, flags)
                                .map_err(|e| e.at(expr.span, expr.position))?,
                        );
                        self.regexes.insert(key, re.clone());
                        re
                    }
                };
                Value::function(Function::Regex(re))
            }
            NodeKind::Function { procedure, args } => {
                self.call(expr, procedure, args, input, env, None)?
            }
            NodeKind::Variable { name } => {
                if name.is_empty() {
                    match input {
                        Value::Array(a) if a.has(OUTER_WRAPPER) => {
                            a.items.first().cloned().unwrap_or_default()
                        }
                        other => other.clone(),
                    }
                } else {
                    env.lookup(name)
                }
            }
            NodeKind::Lambda(def) => Value::function(Function::Lambda(Lambda {
                def: def.clone(),
                params: None,
                validate: true,
                input: input.clone(),
                env: env.clone(),
            })),
            NodeKind::Partial { procedure, args } => {
                self.partial(expr, procedure, args, input, env)?
            }
            NodeKind::Apply { lhs, rhs } => self.apply_expression(expr, lhs, rhs, input, env)?,
            NodeKind::Transform(def) => Value::function(Function::Transform {
                def: def.clone(),
                env: env.clone(),
                span: expr.span,
                position: expr.position,
            }),
            NodeKind::Sort { .. } | NodeKind::Placeholder => Value::Undefined,
            NodeKind::Error { .. } => return Err(Error::new("S0500").at(expr.span, 0)),
        };

        for stage in &expr.attrs.predicate {
            if let Stage::Filter {
                expr: predicate, ..
            } = stage
            {
                result = self.filter(predicate, result, env)?;
            }
        }
        if !matches!(expr.kind, NodeKind::Path { .. })
            && let Some(group) = &expr.attrs.group
        {
            result = self.group(&group.pairs, expr, result, env)?;
        }

        if let Value::Array(a) = &result
            && a.is_sequence()
            && !a.is_tuple_stream()
        {
            if expr.attrs.keep_array {
                a.set(KEEP_SINGLETON);
            }
            if a.items.is_empty() {
                result = Value::Undefined;
            } else if a.items.len() == 1 && !a.has(KEEP_SINGLETON) {
                result = a.items[0].clone();
            }
        }

        self.depth -= 1;
        Ok(result)
    }

    // -----------------------------------------------------------------------------------------
    // Paths

    fn path(
        &mut self,
        expr: &Node,
        steps: &[Node],
        keep_singleton_array: bool,
        input: &Value,
        env: &Env,
    ) -> Result<Value> {
        // A path starting with a variable is absolute; otherwise an input array is iterated.
        let mut input_seq =
            if input.is_array() && !matches!(steps[0].kind, NodeKind::Variable { .. }) {
                input.clone()
            } else {
                Value::sequence(vec![input.clone()])
            };

        let mut result_seq = Value::Undefined;
        let mut is_tuple = false;
        let mut tuple_bindings: Option<Value> = None;
        let last = steps.len() - 1;

        for (ii, step) in steps.iter().enumerate() {
            if step.attrs.tuple {
                is_tuple = true;
            }
            if ii == 0 && step.attrs.cons_array {
                result_seq = self.eval(step, &input_seq, env)?;
            } else if is_tuple {
                tuple_bindings =
                    Some(self.tuple_step(step, &input_seq, tuple_bindings.as_ref(), env)?);
            } else {
                result_seq = self.step(step, &input_seq, env, ii == last)?;
            }
            if !is_tuple
                && (result_seq.is_undefined()
                    || result_seq.as_array().is_some_and(|a| a.items.is_empty()))
            {
                break;
            }
            if step.attrs.focus.is_none() {
                input_seq = result_seq.clone();
            }
        }

        if is_tuple {
            let bindings = tuple_bindings
                .clone()
                .unwrap_or_else(|| Value::array(vec![], SEQUENCE | TUPLE_STREAM));
            if expr.attrs.tuple {
                // The tuple stream carries ancestry information for an enclosing path.
                result_seq = bindings.clone();
            } else {
                let items = bindings
                    .as_array()
                    .map(|a| a.items.iter().map(|t| tuple_get(t, "@")).collect())
                    .unwrap_or_default();
                result_seq = Value::sequence(items);
            }
            tuple_bindings = Some(bindings);
        }

        if keep_singleton_array {
            if let Value::Array(a) = &result_seq
                && a.has(CONS)
                && !a.is_sequence()
            {
                result_seq = Value::sequence(vec![result_seq.clone()]);
            }
            if let Value::Array(a) = &result_seq {
                a.set(KEEP_SINGLETON);
            }
        }

        if let Some(group) = &expr.attrs.group {
            let grouped_input = if is_tuple {
                tuple_bindings.unwrap_or_default()
            } else {
                result_seq
            };
            result_seq = self.group(&group.pairs, expr, grouped_input, env)?;
        }
        Ok(result_seq)
    }

    fn step(&mut self, step: &Node, input: &Value, env: &Env, last: bool) -> Result<Value> {
        if let NodeKind::Sort { terms } = &step.kind {
            let sorted = self.sort(step, terms, input, env)?;
            return self.stages(&step.attrs.stages, sorted, env);
        }
        let items: Vec<Value> = input
            .as_array()
            .map(|a| a.items.clone())
            .unwrap_or_default();
        let mut results = Vec::with_capacity(items.len());
        for item in &items {
            let mut res = self.eval(step, item, env)?;
            for stage in &step.attrs.stages {
                if let Stage::Filter { expr, .. } = stage {
                    res = self.filter(expr, res, env)?;
                }
            }
            if !res.is_undefined() {
                results.push(res);
            }
        }

        if last && results.len() == 1 && results[0].as_array().is_some_and(|a| !a.is_sequence()) {
            return Ok(results.pop().unwrap_or_default());
        }
        let mut out = Vec::with_capacity(results.len());
        for res in results {
            match &res {
                Value::Array(a) if !a.has(CONS) => {
                    self.check_array(out.len() + a.items.len())?;
                    out.extend(a.items.iter().cloned());
                }
                _ => {
                    self.check_array(out.len() + 1)?;
                    out.push(res);
                }
            }
        }
        Ok(Value::sequence(out))
    }

    fn stages(&mut self, stages: &[Stage], input: Value, env: &Env) -> Result<Value> {
        let mut result = input;
        for stage in stages {
            match stage {
                Stage::Filter { expr, .. } => result = self.filter(expr, result, env)?,
                Stage::Index { var, .. } => {
                    if let Value::Array(a) = &result {
                        for (ee, tuple) in a.items.iter().enumerate() {
                            if let Value::Object(o) = tuple {
                                o.borrow_mut().insert(var.clone(), Value::Number(ee as f64));
                            }
                        }
                    }
                }
            }
        }
        Ok(result)
    }

    fn tuple_step(
        &mut self,
        step: &Node,
        input: &Value,
        tuple_bindings: Option<&Value>,
        env: &Env,
    ) -> Result<Value> {
        if let NodeKind::Sort { terms } = &step.kind {
            let result = match tuple_bindings {
                Some(bindings) => self.sort(step, terms, bindings, env)?,
                None => {
                    let sorted = self.sort(step, terms, input, env)?;
                    let tuples = sorted
                        .items_or_self()
                        .into_iter()
                        .enumerate()
                        .map(|(ss, item)| {
                            let mut tuple = Map::default();
                            tuple.insert("@".into(), item);
                            if let Some(index) = &step.attrs.index {
                                tuple.insert(index.clone(), Value::Number(ss as f64));
                            }
                            Value::object(tuple)
                        })
                        .collect();
                    Value::array(tuples, SEQUENCE | TUPLE_STREAM)
                }
            };
            return self.stages(&step.attrs.stages, result, env);
        }

        let bindings: Vec<Value> = match tuple_bindings {
            Some(b) => b.items_or_self(),
            None => input
                .as_array()
                .map(|a| {
                    a.items
                        .iter()
                        .map(|item| {
                            let mut t = Map::default();
                            t.insert("@".into(), item.clone());
                            Value::object(t)
                        })
                        .collect()
                })
                .unwrap_or_default(),
        };

        let mut out = Vec::new();
        for tuple in &bindings {
            let step_env = self.frame_from_tuple(env, tuple);
            let context = tuple_get(tuple, "@");
            let res = self.eval(step, &context, &step_env)?;
            if res.is_undefined() {
                continue;
            }
            let (items, res_is_tuple) = match &res {
                Value::Array(a) => (a.items.clone(), a.is_tuple_stream()),
                other => (vec![other.clone()], false),
            };
            self.check_array(out.len() + items.len())?;
            for (bb, item) in items.into_iter().enumerate() {
                let mut t = tuple_map(tuple);
                if res_is_tuple {
                    if let Value::Object(o) = &item {
                        for (k, v) in o.borrow().iter() {
                            t.insert(k.clone(), v.clone());
                        }
                    }
                } else {
                    if let Some(focus) = &step.attrs.focus {
                        t.insert(focus.clone(), item);
                        t.insert("@".into(), context.clone());
                    } else {
                        t.insert("@".into(), item);
                    }
                    if let Some(index) = &step.attrs.index {
                        t.insert(index.clone(), Value::Number(bb as f64));
                    }
                    if let Some(label) = &step.attrs.ancestor_label {
                        t.insert(label.clone(), context.clone());
                    }
                }
                out.push(Value::object(t));
            }
        }
        let result = Value::array(out, SEQUENCE | TUPLE_STREAM);
        self.stages(&step.attrs.stages, result, env)
    }

    /// Applies a predicate to a sequence (jsonata-js `evaluateFilter`).
    fn filter(&mut self, predicate: &Node, input: Value, env: &Env) -> Result<Value> {
        let is_tuple = input.has_flag(TUPLE_STREAM);
        let flags = SEQUENCE | if is_tuple { TUPLE_STREAM } else { 0 };
        let items = input.items_or_self();
        let len = items.len() as f64;
        let mut results = Vec::new();

        if let NodeKind::Number { value } = predicate.kind {
            let mut index = value.floor();
            if index < 0.0 {
                index += len;
            }
            let item = if index >= 0.0 && index < len {
                items[index as usize].clone()
            } else {
                Value::Undefined
            };
            match item {
                Value::Undefined => {}
                Value::Array(_) => return Ok(item),
                other => results.push(other),
            }
            return Ok(Value::array(results, flags));
        }

        if let Some(t) = self.tracer.as_deref_mut() {
            t.filter_begin(predicate);
        }
        let tested = self.filter_items(predicate, &items, is_tuple, env, &mut results);
        if let Some(t) = self.tracer.as_deref_mut() {
            t.filter_end(predicate);
        }
        tested?;
        Ok(Value::array(results, flags))
    }

    /// Tests each item against a predicate, collecting those that pass.
    fn filter_items(
        &mut self,
        predicate: &Node,
        items: &[Value],
        is_tuple: bool,
        env: &Env,
        results: &mut Vec<Value>,
    ) -> Result<()> {
        let len = items.len() as f64;
        for (index, item) in items.iter().enumerate() {
            let before = results.len();
            let (context, frame) = if is_tuple {
                (tuple_get(item, "@"), self.frame_from_tuple(env, item))
            } else {
                (item.clone(), env.clone())
            };
            let mut res = self.eval(predicate, &context, &frame)?;
            if res.is_numeric()? {
                res = Value::array(vec![res], 0);
            }
            if res.is_array_of_numbers()? {
                for n in res.items_or_self() {
                    let mut ii = n.as_number().unwrap_or(f64::NAN).floor();
                    if ii < 0.0 {
                        ii += len;
                    }
                    if ii == index as f64 {
                        results.push(item.clone());
                    }
                }
            } else if boolean(&res)? == Some(true) {
                results.push(item.clone());
            }
            if let Some(t) = self.tracer.as_deref_mut() {
                t.filter_item(predicate, index, ValueRef(item), results.len() > before);
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------------------------
    // Operators

    fn binary(
        &mut self,
        expr: &Node,
        op: BinaryOp,
        lhs_node: &Node,
        rhs_node: &Node,
        input: &Value,
        env: &Env,
    ) -> Result<Value> {
        let locate = |e: Error| {
            e.at_if_missing(expr.span, expr.position)
                .with_token_if_missing(op.symbol())
        };
        let lhs = self.eval(lhs_node, input, env)?;

        if matches!(op, BinaryOp::And | BinaryOp::Or) {
            let l = boolean(&lhs).map_err(locate)?.unwrap_or(false);
            let decided = match op {
                BinaryOp::And if !l => Some(false),
                BinaryOp::Or if l => Some(true),
                _ => None,
            };
            if decided.is_some()
                && let Some(t) = self.tracer.as_deref_mut()
            {
                t.short_circuit(expr);
            }
            let result = match decided {
                Some(value) => value,
                None => {
                    let rhs = self.eval(rhs_node, input, env).map_err(locate)?;
                    boolean(&rhs).map_err(locate)?.unwrap_or(false)
                }
            };
            return Ok(Value::Bool(result));
        }

        let rhs = self.eval(rhs_node, input, env)?;
        if let Some(t) = self.tracer.as_deref_mut()
            && matches!(
                op,
                BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge
                    | BinaryOp::In
            )
        {
            t.operands(expr, ValueRef(&lhs), ValueRef(&rhs));
        }
        let result = match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
                numeric(&lhs, &rhs, op)
            }
            BinaryOp::Eq | BinaryOp::Ne => Ok(if lhs.is_undefined() || rhs.is_undefined() {
                Value::Bool(false)
            } else {
                Value::Bool(lhs.deep_eq(&rhs) == (op == BinaryOp::Eq))
            }),
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => comparison(&lhs, &rhs, op),
            BinaryOp::Concat => {
                let limit = self.limits.max_string_length;
                let l = functions::to_string(&lhs, false, limit)?.unwrap_or_default();
                let r = functions::to_string(&rhs, false, limit)?.unwrap_or_default();
                self.check_string(l.len() + r.len())
                    .map(|()| Value::string(l + &r))
            }
            BinaryOp::Range => range(&lhs, &rhs).and_then(|v| self.check_size(v)),
            BinaryOp::In => Ok(Value::Bool(
                !lhs.is_undefined()
                    && !rhs.is_undefined()
                    && rhs.items_or_self().iter().any(|item| item.strict_eq(&lhs)),
            )),
            BinaryOp::And | BinaryOp::Or => unreachable!("handled above"),
        };
        result.map_err(locate)
    }

    fn array_constructor(
        &mut self,
        expr: &Node,
        items: &[Node],
        input: &Value,
        env: &Env,
    ) -> Result<Value> {
        let mut out = Vec::new();
        for item in items {
            let value = self.eval(item, input, env)?;
            if value.is_undefined() {
                continue;
            }
            if matches!(item.kind, NodeKind::Array { .. }) {
                self.check_array(out.len() + 1)?;
                out.push(value);
            } else {
                match &value {
                    Value::Array(a) => {
                        self.check_array(out.len() + a.items.len())?;
                        out.extend(a.items.iter().cloned());
                    }
                    other => {
                        self.check_array(out.len() + 1)?;
                        out.push(other.clone());
                    }
                }
            }
        }
        Ok(Value::array(
            out,
            if expr.attrs.cons_array { CONS } else { 0 },
        ))
    }

    /// Group-by and object construction (jsonata-js `evaluateGroupExpression`).
    fn group(
        &mut self,
        pairs: &[(Node, Node)],
        expr: &Node,
        input: Value,
        env: &Env,
    ) -> Result<Value> {
        let reduce = input.has_flag(TUPLE_STREAM);
        let mut items = input.items_or_self();
        if items.is_empty() && !reduce {
            // An empty input still produces a literal object.
            items.push(Value::Undefined);
        }

        // key -> (data, index of the key/value pair that produced it)
        let mut groups: IndexMap<String, (Value, usize)> = IndexMap::new();
        for item in &items {
            let frame = if reduce {
                self.frame_from_tuple(env, item)
            } else {
                env.clone()
            };
            let context = if reduce {
                tuple_get(item, "@")
            } else {
                item.clone()
            };
            for (pair_index, (key_expr, _)) in pairs.iter().enumerate() {
                let key = self.eval(key_expr, &context, &frame)?;
                let key = match key {
                    Value::Undefined => continue,
                    Value::String(s) => s,
                    other => return Err(err_at("T1003", expr).with_value("value", &other)),
                };
                match groups.get_mut(key.as_ref()) {
                    Some((data, index)) => {
                        if *index != pair_index {
                            return Err(err_at("D1009", expr).with("value", key.as_ref()));
                        }
                        *data = functions::append(std::mem::take(data), item.clone());
                    }
                    None => {
                        groups.insert(key.to_string(), (item.clone(), pair_index));
                    }
                }
            }
        }

        let mut result = Map::default();
        for (key, (data, pair_index)) in groups {
            let (context, frame) = if reduce {
                let mut tuple = reduce_tuple_stream(&data);
                let context = tuple.shift_remove("@").unwrap_or_default();
                let tuple = Value::object(tuple);
                (context, self.frame_from_tuple(env, &tuple))
            } else {
                (data, env.clone())
            };
            let value = self.eval(&pairs[pair_index].1, &context, &frame)?;
            if !value.is_undefined() {
                result.insert(key, value);
            }
        }
        Ok(Value::object(result))
    }

    /// Order-by (jsonata-js `evaluateSortExpression`).
    fn sort(&mut self, expr: &Node, terms: &[SortTerm], input: &Value, env: &Env) -> Result<Value> {
        let Value::Array(arr) = input else {
            return Ok(input.clone());
        };
        if arr.items.len() <= 1 {
            return Ok(input.clone());
        }
        let is_tuple = arr.is_tuple_stream();
        let mut swap = |a: &Value, b: &Value| -> Result<bool> {
            let mut comp = 0;
            for term in terms {
                if comp != 0 {
                    break;
                }
                let key = |ev: &mut Self, v: &Value| -> Result<Value> {
                    if is_tuple {
                        let frame = ev.frame_from_tuple(env, v);
                        ev.eval(&term.expr, &tuple_get(v, "@"), &frame)
                    } else {
                        ev.eval(&term.expr, v, env)
                    }
                };
                let aa = key(self, a)?;
                let bb = key(self, b)?;
                if aa.is_undefined() {
                    comp = if bb.is_undefined() { 0 } else { 1 };
                    continue;
                }
                if bb.is_undefined() {
                    comp = -1;
                    continue;
                }
                let comparable = |v: &Value| matches!(v, Value::String(_) | Value::Number(_));
                if !comparable(&aa) || !comparable(&bb) {
                    let bad = if comparable(&aa) { &bb } else { &aa };
                    return Err(err_at("T2008", expr).with_value("value", bad));
                }
                let ordering = match (&aa, &bb) {
                    (Value::Number(x), Value::Number(y)) => x.partial_cmp(y),
                    (Value::String(x), Value::String(y)) => Some(value::js_string_cmp(x, y)),
                    _ => {
                        return Err(err_at("T2007", expr)
                            .with_value("value", &aa)
                            .with_value("value2", &bb));
                    }
                };
                comp = match ordering {
                    Some(std::cmp::Ordering::Less) => -1,
                    Some(std::cmp::Ordering::Greater) => 1,
                    _ => continue,
                };
                if term.descending {
                    comp = -comp;
                }
            }
            Ok(comp == 1)
        };
        let sorted = functions::merge_sort(arr.items.clone(), &mut swap)?;
        Ok(Value::array(sorted, 0))
    }

    // -----------------------------------------------------------------------------------------
    // Functions

    /// A function call (jsonata-js `evaluateFunction`). `apply_to` is the left side of `~>`.
    fn call(
        &mut self,
        expr: &Node,
        procedure: &Node,
        args: &[Node],
        input: &Value,
        env: &Env,
        apply_to: Option<Value>,
    ) -> Result<Value> {
        let proc = self.eval(procedure, input, env)?;
        let name = procedure_name(procedure);
        if proc.is_undefined()
            && let Some(step_name) = first_step_name(procedure)
            && js_truthy(&env.lookup(step_name))
        {
            // The user probably forgot the leading `$`.
            return Err(err_at("T1005", expr).with("token", step_name));
        }
        let mut evaluated = Vec::with_capacity(args.len() + 1);
        if let Some(context) = apply_to {
            evaluated.push(context);
        }
        for a in args {
            evaluated.push(self.eval(a, input, env)?);
        }
        self.apply(&proc, evaluated, input, env).map_err(|e| {
            let e = e.at_if_missing(expr.span, expr.position);
            match name {
                Some(n) => e.with_token_if_missing(n),
                None => e,
            }
        })
    }

    /// Applies a function, running tail-call thunks in a loop (the trampoline).
    pub(crate) fn apply(
        &mut self,
        proc: &Value,
        args: Vec<Value>,
        input: &Value,
        env: &Env,
    ) -> Result<Value> {
        let mut result = self.apply_inner(proc, args, input, env)?;
        loop {
            let thunk = match &result {
                Value::Function(f) if f.is_thunk() => f.clone(),
                _ => return Ok(result),
            };
            let Function::Lambda(lambda) = &*thunk else {
                return Ok(result);
            };
            let NodeKind::Function { procedure, args } = &lambda.def.body.kind else {
                return Ok(result);
            };
            let next = self.eval(procedure, &lambda.input, &lambda.env)?;
            let mut evaluated = Vec::with_capacity(args.len());
            for a in args {
                evaluated.push(self.eval(a, &lambda.input, &lambda.env)?);
            }
            let name = procedure_name(procedure);
            result = self
                .apply_inner(&next, evaluated, input, env)
                .map_err(|e| {
                    let e = e.at_if_missing(procedure.span, procedure.position);
                    match name {
                        Some(n) => e.with_token_if_missing(n),
                        None => e,
                    }
                })?;
        }
    }

    fn apply_inner(
        &mut self,
        proc: &Value,
        args: Vec<Value>,
        input: &Value,
        env: &Env,
    ) -> Result<Value> {
        let Value::Function(f) = proc else {
            return Err(Error::new("T1006"));
        };
        match &**f {
            Function::Lambda(lambda) => {
                let args = match (&lambda.def.signature, lambda.validate) {
                    (Some(sig), true) => sig.validate(args, input)?,
                    _ => args,
                };
                self.apply_lambda(lambda, args)
            }
            Function::Native(native) => {
                let args = match &native.signature {
                    Some(sig) => sig.validate(args, input)?,
                    None => args,
                };
                let result = (native.imp)(self, args, &Call { input, env })?;
                self.check_size(result)
            }
            Function::Host(host) => {
                let args = match host.signature() {
                    Some(sig) => sig.validate(args, input)?,
                    None => args,
                };
                let result = host.call(&args, input, self.limits.max_result_size)?;
                self.check_size(result)
            }
            Function::Partial {
                target,
                args: bound,
            } => {
                let mut supplied = args.into_iter();
                let full: Vec<Value> = bound
                    .iter()
                    .map(|b| match b {
                        Some(v) => v.clone(),
                        None => supplied.next().unwrap_or_default(),
                    })
                    .collect();
                // Like jsonata-js, which wraps the function in a lambda: no signature check and
                // no context value.
                let result = match &**target {
                    Function::Native(native) => (native.imp)(
                        self,
                        full,
                        &Call {
                            input: &Value::Undefined,
                            env,
                        },
                    ),
                    Function::Host(host) => {
                        host.call(&full, &Value::Undefined, self.limits.max_result_size)
                    }
                    _ => Err(Error::new("T1006")),
                }?;
                self.check_size(result)
            }
            Function::Transform {
                def,
                env: closure_env,
                span,
                position,
            } => {
                let args = transform_signature().validate(args, input)?;
                let closure_env = closure_env.clone();
                self.transform(
                    def,
                    args.into_iter().next().unwrap_or_default(),
                    &closure_env,
                )
                .map_err(|e| e.at_if_missing(*span, *position))
            }
            Function::Regex(re) => {
                let mut args = args.into_iter();
                // jsonata-js matches any other value as JavaScript's `String(value)`
                // ("undefined", "[object Object]"); jlens gives undefined (compatibility.md).
                let Value::String(text) = args.next().unwrap_or_default() else {
                    return Ok(Value::Undefined);
                };
                let from = match args.next() {
                    Some(Value::Number(n)) if n > 0.0 => {
                        crate::regex::byte_offset(&text, n as usize)
                    }
                    _ => 0,
                };
                crate::regex::exec(self, re, &text, from)
            }
            Function::RegexNext { re, text, last } => crate::regex::next(self, re, text, *last),
        }
    }

    fn apply_lambda(&mut self, lambda: &Lambda, args: Vec<Value>) -> Result<Value> {
        let frame = self.child(&lambda.env);
        let mut args = args.into_iter();
        for param in lambda.params() {
            // Missing arguments are bound as undefined so they shadow outer variables.
            let value = args.next().unwrap_or_default();
            self.bind(&frame, param, value);
        }
        let def = lambda.def.clone();
        self.eval(&def.body, &lambda.input, &frame)
    }

    /// Partial application (jsonata-js `evaluatePartialApplication`).
    fn partial(
        &mut self,
        expr: &Node,
        procedure: &Node,
        args: &[Node],
        input: &Value,
        env: &Env,
    ) -> Result<Value> {
        let mut evaluated = Vec::with_capacity(args.len());
        for a in args {
            evaluated.push(if matches!(a.kind, NodeKind::Placeholder) {
                None
            } else {
                Some(self.eval(a, input, env)?)
            });
        }
        let proc = self.eval(procedure, input, env)?;
        if proc.is_undefined()
            && let Some(step_name) = first_step_name(procedure)
            && js_truthy(&env.lookup(step_name))
        {
            return Err(err_at("T1007", expr).with("token", step_name));
        }
        let not_a_function = || {
            let e = err_at("T1008", expr);
            match procedure_name(procedure) {
                Some(n) => e.with("token", n),
                None => e,
            }
        };
        let Value::Function(f) = &proc else {
            return Err(not_a_function());
        };
        Ok(match &**f {
            Function::Lambda(lambda) => {
                let frame = self.child(&lambda.env);
                let mut unbound = Vec::new();
                for (index, param) in lambda.params().iter().enumerate() {
                    match evaluated.get(index) {
                        Some(None) => unbound.push(param.clone()),
                        Some(Some(v)) => self.bind(&frame, param, v.clone()),
                        None => self.bind(&frame, param, Value::Undefined),
                    }
                }
                Value::function(Function::Lambda(Lambda {
                    def: lambda.def.clone(),
                    params: Some(unbound.into()),
                    validate: false,
                    input: lambda.input.clone(),
                    env: frame,
                }))
            }
            Function::Native(_) | Function::Host(_) => {
                let params = match &**f {
                    Function::Native(native) => native.params.len(),
                    _ => f.arity(),
                };
                Value::function(Function::Partial {
                    target: f.clone(),
                    args: (0..params)
                        .map(|i| match evaluated.get(i) {
                            Some(None) => None,
                            Some(Some(v)) => Some(v.clone()),
                            None => Some(Value::Undefined),
                        })
                        .collect(),
                })
            }
            Function::Partial {
                target,
                args: bound,
            } => {
                let mut supplied = evaluated.into_iter();
                Value::function(Function::Partial {
                    target: target.clone(),
                    args: bound
                        .iter()
                        .map(|b| match b {
                            Some(v) => Some(v.clone()),
                            None => supplied.next().unwrap_or(Some(Value::Undefined)),
                        })
                        .collect(),
                })
            }
            _ => return Err(not_a_function()),
        })
    }

    /// `lhs ~> rhs` (jsonata-js `evaluateApplyExpression`).
    fn apply_expression(
        &mut self,
        expr: &Node,
        lhs: &Node,
        rhs: &Node,
        input: &Value,
        env: &Env,
    ) -> Result<Value> {
        let value = self.eval(lhs, input, env)?;
        if let NodeKind::Function { procedure, args } = &rhs.kind {
            return self.call(rhs, procedure, args, input, env, Some(value));
        }
        let func = self.eval(rhs, input, env)?;
        if !func.is_function() {
            return Err(err_at("T2006", expr).with_value("value", &func));
        }
        if value.is_function() {
            // Function composition: λ($f, $g) { λ($x){ $g($f($x)) } }
            let chain = self.eval(&chain_ast().root, &Value::Undefined, env)?;
            self.apply(&chain, vec![value, func], &Value::Undefined, env)
        } else {
            self.apply(&func, vec![value], &Value::Undefined, env)
        }
    }

    /// Applies a transform expression to a value (jsonata-js `evaluateTransformExpression`).
    fn transform(&mut self, def: &Arc<TransformDef>, obj: Value, env: &Env) -> Result<Value> {
        if obj.is_undefined() {
            return Ok(Value::Undefined);
        }
        let clone = env.lookup("clone");
        if !clone.is_function() {
            return Err(Error::new("T2013"));
        }
        let result = self.apply(&clone, vec![obj], &Value::Undefined, env)?;
        let matches = self.eval(&def.pattern, &result, env)?;
        if matches.is_undefined() {
            return Ok(result);
        }
        for m in matches.items_or_self() {
            let update = self.eval(&def.update, &m, env)?;
            if !update.is_undefined() {
                let Value::Object(update) = &update else {
                    return Err(err_at("T2011", &def.update).with_value("value", &update));
                };
                let entries: Vec<(String, Value)> = update
                    .borrow()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                if let Value::Object(target) = &m {
                    // jsonata-js would build a circular object here, which cannot be
                    // converted to JSON; jlens reports it instead.
                    if entries.iter().any(|(_, v)| reaches(v, target)) {
                        return Err(Error::new("U1001")
                            .with(
                                "message",
                                "The transform would make an object contain itself",
                            )
                            .at(def.update.span, def.update.position));
                    }
                    let mut target = target.borrow_mut();
                    for (k, v) in entries {
                        target.insert(k, v);
                    }
                }
            }
            if let Some(delete) = &def.delete {
                let deletions = self.eval(delete, &m, env)?;
                if deletions.is_undefined() {
                    continue;
                }
                let list = deletions.items_or_self();
                if !list.iter().all(|d| matches!(d, Value::String(_))) {
                    return Err(err_at("T2012", delete).with_value("value", &deletions));
                }
                if let Value::Object(target) = &m {
                    let mut target = target.borrow_mut();
                    for d in list {
                        if let Value::String(key) = d {
                            target.shift_remove(key.as_ref());
                        }
                    }
                }
            }
        }
        Ok(result)
    }
}

impl Lambda {
    pub(crate) fn params(&self) -> &[String] {
        match &self.params {
            Some(p) => p,
            None => &self.def.params,
        }
    }
}

/// The name used as the `token` of errors raised by a call: `$name` → `name`.
fn procedure_name(procedure: &Node) -> Option<&str> {
    match &procedure.kind {
        NodeKind::Variable { name } => Some(name),
        NodeKind::Path { steps, .. } => match &steps[0].kind {
            NodeKind::Name { name } | NodeKind::Variable { name } => Some(name),
            _ => None,
        },
        _ => None,
    }
}

fn first_step_name(procedure: &Node) -> Option<&str> {
    match &procedure.kind {
        NodeKind::Path { steps, .. } => match &steps[0].kind {
            NodeKind::Name { name } => Some(name),
            _ => None,
        },
        _ => None,
    }
}

fn numeric(lhs: &Value, rhs: &Value, op: BinaryOp) -> Result<Value> {
    if !lhs.is_undefined() && !lhs.is_numeric()? {
        return Err(Error::new("T2001").with_value("value", lhs));
    }
    if !rhs.is_undefined() && !rhs.is_numeric()? {
        return Err(Error::new("T2002").with_value("value", rhs));
    }
    let (Some(l), Some(r)) = (lhs.as_number(), rhs.as_number()) else {
        return Ok(Value::Undefined);
    };
    Ok(Value::Number(match op {
        BinaryOp::Add => l + r,
        BinaryOp::Sub => l - r,
        BinaryOp::Mul => l * r,
        BinaryOp::Div => l / r,
        _ => l % r,
    }))
}

fn comparison(lhs: &Value, rhs: &Value, op: BinaryOp) -> Result<Value> {
    let comparable =
        |v: &Value| matches!(v, Value::Undefined | Value::String(_) | Value::Number(_));
    if !comparable(lhs) || !comparable(rhs) {
        let bad = if matches!(lhs, Value::String(_) | Value::Number(_)) {
            rhs
        } else {
            lhs
        };
        return Err(Error::new("T2010").with_value("value", bad));
    }
    if lhs.is_undefined() || rhs.is_undefined() {
        return Ok(Value::Undefined);
    }
    let result = match (lhs, rhs) {
        (Value::Number(l), Value::Number(r)) => match op {
            BinaryOp::Lt => l < r,
            BinaryOp::Le => l <= r,
            BinaryOp::Gt => l > r,
            _ => l >= r,
        },
        (Value::String(l), Value::String(r)) => {
            let ord = value::js_string_cmp(l, r);
            match op {
                BinaryOp::Lt => ord.is_lt(),
                BinaryOp::Le => ord.is_le(),
                BinaryOp::Gt => ord.is_gt(),
                _ => ord.is_ge(),
            }
        }
        _ => {
            return Err(Error::new("T2009")
                .with_value("value", lhs)
                .with_value("value2", rhs));
        }
    };
    Ok(Value::Bool(result))
}

fn is_integer(v: &Value) -> bool {
    matches!(v, Value::Number(n) if n.is_finite() && n.fract() == 0.0)
}

fn range(lhs: &Value, rhs: &Value) -> Result<Value> {
    if !lhs.is_undefined() && !is_integer(lhs) {
        return Err(Error::new("T2003").with_value("value", lhs));
    }
    if !rhs.is_undefined() && !is_integer(rhs) {
        return Err(Error::new("T2004").with_value("value", rhs));
    }
    let (Some(l), Some(r)) = (lhs.as_number(), rhs.as_number()) else {
        return Ok(Value::Undefined);
    };
    if l > r {
        return Ok(Value::Undefined);
    }
    let size = r - l + 1.0;
    if size > 1e7 {
        return Err(Error::new("D2014").with("value", value::json_number(size)));
    }
    let (l, r) = (l as i64, r as i64);
    Ok(Value::sequence(
        (l..=r).map(|n| Value::Number(n as f64)).collect(),
    ))
}

/// The error of an array that would exceed the size limit of `limit` items.
fn array_too_long(limit: usize) -> Error {
    Error::new("U1001").with(
        "message",
        format!("An array would exceed the size limit of {limit} items"),
    )
}

/// `*`, whose result holds at most `max` items.
fn wildcard(input: &Value, max: usize) -> Result<Value> {
    let input = match input {
        Value::Array(a) if a.has(OUTER_WRAPPER) && !a.items.is_empty() => &a.items[0],
        other => other,
    };
    // jsonata-js appends array values with `$append`, which turns the result into a plain
    // array; the flags are reproduced here.
    let mut items = Vec::new();
    let mut plain = false;
    // In JavaScript an array is an object whose keys are its indices, so `*` also yields the
    // items of an array.
    let values: Vec<Value> = match input {
        Value::Object(o) => o.borrow().values().cloned().collect(),
        Value::Array(a) => a.items.clone(),
        _ => Vec::new(),
    };
    {
        for value in &values {
            match value {
                Value::Array(_) => {
                    flatten(value, &mut items, max)?;
                    plain = true;
                }
                other => items.push(other.clone()),
            }
        }
    }
    Ok(Value::array(items, if plain { 0 } else { SEQUENCE }))
}

/// Appends the items of nested arrays to `out`, failing beyond `max` items: an array nested in
/// itself flattens to far more items than it holds.
fn flatten(v: &Value, out: &mut Vec<Value>, max: usize) -> Result<()> {
    match v {
        Value::Array(a) => a.items.iter().try_for_each(|i| flatten(i, out, max)),
        other => push_within(out, other.clone(), max),
    }
}

fn push_within(out: &mut Vec<Value>, v: Value, max: usize) -> Result<()> {
    if out.len() >= max {
        return Err(array_too_long(max));
    }
    out.push(v);
    Ok(())
}

/// `**`, whose result holds at most `max` items.
fn descendants(input: &Value, max: usize) -> Result<Value> {
    if input.is_undefined() {
        return Ok(Value::Undefined);
    }
    let mut out = Vec::new();
    recurse_descendants(input, &mut out, max)?;
    Ok(if out.len() == 1 {
        out.pop().unwrap_or_default()
    } else {
        Value::sequence(out)
    })
}

fn recurse_descendants(input: &Value, out: &mut Vec<Value>, max: usize) -> Result<()> {
    match input {
        Value::Array(a) => a
            .items
            .iter()
            .try_for_each(|i| recurse_descendants(i, out, max)),
        Value::Object(o) => {
            push_within(out, input.clone(), max)?;
            o.borrow()
                .values()
                .try_for_each(|v| recurse_descendants(v, out, max))
        }
        other => push_within(out, other.clone(), max),
    }
}

/// Merges a sequence of tuples into one, appending the values of each field.
fn reduce_tuple_stream(stream: &Value) -> Map {
    let Value::Array(a) = stream else {
        return tuple_map(stream);
    };
    let mut result = a.items.first().map(tuple_map).unwrap_or_default();
    for tuple in a.items.iter().skip(1) {
        for (k, v) in tuple_map(tuple) {
            let entry = result.entry(k).or_default();
            let existing = std::mem::take(entry);
            *entry = functions::append(existing, v);
        }
    }
    result
}

/// Whether `object` can be reached from `value` (or is `value`). Shared substructures are
/// visited once, so this is linear in the size of `value`.
fn reaches(value: &Value, object: &Rc<RefCell<Map>>) -> bool {
    let mut seen: std::collections::HashSet<*const ()> = std::collections::HashSet::new();
    let mut stack = vec![value.clone()];
    while let Some(v) = stack.pop() {
        match &v {
            Value::Object(o) if Rc::ptr_eq(o, object) => return true,
            Value::Object(o) if seen.insert(Rc::as_ptr(o) as *const ()) => {
                stack.extend(o.borrow().values().cloned());
            }
            Value::Array(a) if seen.insert(Rc::as_ptr(a) as *const ()) => {
                stack.extend(a.items.iter().cloned());
            }
            _ => {}
        }
    }
    false
}
