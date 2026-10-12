//! Post-parse processing. Ported from `processAST` and its helpers in jsonata-js
//! `src/parser.js` (MIT).
//!
//! This stage adds semantic structure to the raw parse tree so that the evaluator's job is
//! simpler:
//!
//! * chains of `.` become [`NodeKind::Path`] nodes with a flat list of steps;
//! * predicates become filter [`Stage`]s on steps (or a `predicate` list on other nodes);
//! * `@`, `#`, group-by and order-by become attributes or steps of the path;
//! * parent operators (`%`) are resolved to the step that binds the ancestor;
//! * function calls in tail position of lambdas are wrapped in thunks (tail-call optimisation).
//!
//! In recover mode errors become [`NodeKind::Error`] nodes and diagnostics instead of aborting.

use crate::ast::{BinaryOp, FallbackOp, LambdaDef, Slot, TransformDef};
use crate::ast::{Group, Node, NodeKind, SlotId, SortTerm, Span, Stage};
use crate::error::{Error, Result};
use crate::lexer::Lit;
use crate::parser::{Raw, RawKind};
use std::sync::Arc;

pub(crate) struct Processor<'a> {
    src: &'a str,
    recover: bool,
    pub errors: Vec<Error>,
    pub slots: Vec<Slot>,
}

impl<'a> Processor<'a> {
    pub fn new(src: &'a str, recover: bool) -> Self {
        Processor {
            src,
            recover,
            errors: Vec::new(),
            slots: Vec::new(),
        }
    }

    /// Processes the root of a raw tree and assigns node ids.
    pub fn process_root(&mut self, raw: Raw) -> Result<Node> {
        let mut root = self.process(raw)?;
        if matches!(root.kind, NodeKind::Parent { .. }) || !root.attrs.seeking_parent.is_empty() {
            // The parent of the top-level context cannot be derived.
            let err = self
                .error("S0217", root.span, root.pos)
                .with("token", root.type_name());
            self.report(err)?;
        }
        let mut eval = false;
        root.walk(&mut |node| {
            eval |= matches!(&node.kind, NodeKind::Variable { name } if name == "eval");
        });
        // Final pass: ids, labels of ancestor slots, UTF-16 positions and block frames.
        let utf16 = utf16_table(self.src);
        let slots = &self.slots;
        let mut next_id = 0;
        root.walk_mut(&mut |node| {
            node.id = next_id;
            next_id += 1;
            node.position = utf16[node.pos.min(utf16.len() - 1)];
            if let NodeKind::Parent { slot, label } = &mut node.kind {
                *label = slots[*slot].label.clone();
            }
            node.attrs.ancestor_label = node.attrs.ancestor.map(|s| slots[s].label.clone());
            mark_frame(node, eval);
        });
        Ok(root)
    }

    fn error(&self, code: &str, span: Span, pos: usize) -> Error {
        Error::new(code).located(self.src, span, pos)
    }

    /// Reports an error: recorded in recover mode, returned otherwise.
    fn report(&mut self, err: Error) -> Result<()> {
        if self.recover {
            self.errors.push(err);
            Ok(())
        } else {
            Err(err)
        }
    }

    /// Reports an error and returns an error node in its place.
    fn fail(&mut self, err: Error) -> Result<Node> {
        let span = err.span().unwrap_or_default();
        let pos = span.end;
        self.report(err.clone())?;
        Ok(Node::new(
            NodeKind::Error {
                error: Box::new(err),
            },
            span,
            pos,
        ))
    }

    fn process_boxed(&mut self, raw: Raw) -> Result<Box<Node>> {
        Ok(Box::new(self.process(raw)?))
    }

    fn process(&mut self, raw: Raw) -> Result<Node> {
        if crate::stack::exhausted() {
            return Err(Error::new("U1001")
                .with("message", "The expression is nested too deeply")
                .located(self.src, raw.span, raw.pos));
        }
        crate::stack::grow(|| self.process_inner(raw))
    }

    fn process_inner(&mut self, raw: Raw) -> Result<Node> {
        let Raw {
            kind,
            span,
            pos,
            keep_array,
            annotation,
        } = raw;
        let mut annotation = annotation;

        let mut result = match kind {
            RawKind::Binary { op, lhs, rhs } => match op {
                "." => self.path(*lhs, *rhs, span)?,
                "[" => self.filter(*lhs, *rhs, span, pos)?,
                ":=" => {
                    let lhs = self.process_boxed(*lhs)?;
                    let rhs = self.process_boxed(*rhs)?;
                    let seeking = ancestry(&rhs);
                    let mut node = Node::new(NodeKind::Bind { lhs, rhs }, span, pos);
                    node.attrs.seeking_parent = seeking;
                    node
                }
                "??" | "?:" => {
                    let op = if op == "??" {
                        FallbackOp::Coalesce
                    } else {
                        FallbackOp::Default
                    };
                    let lhs = self.process_boxed(*lhs)?;
                    let rhs = self.process_boxed(*rhs)?;
                    let mut seeking = ancestry(&lhs);
                    seeking.extend(ancestry(&rhs));
                    let mut node = Node::new(NodeKind::Fallback { op, lhs, rhs }, span, pos);
                    node.attrs.seeking_parent = seeking;
                    node
                }
                "@" => self.focus(*lhs, *rhs, span, pos, keep_array)?,
                "#" => self.index(*lhs, *rhs, span, pos)?,
                "~>" => {
                    let lhs = self.process_boxed(*lhs)?;
                    let rhs = self.process_boxed(*rhs)?;
                    let keep = lhs.attrs.keep_array || rhs.attrs.keep_array;
                    let mut node = Node::new(NodeKind::Apply { lhs, rhs }, span, pos);
                    node.attrs.keep_array = keep;
                    node
                }
                _ => {
                    let op = BinaryOp::from_symbol(op).expect("binary operator");
                    let lhs = self.process_boxed(*lhs)?;
                    let rhs = self.process_boxed(*rhs)?;
                    let mut seeking = ancestry(&lhs);
                    seeking.extend(ancestry(&rhs));
                    let mut node = Node::new(NodeKind::Binary { op, lhs, rhs }, span, pos);
                    node.attrs.seeking_parent = seeking;
                    node
                }
            },
            RawKind::Group { lhs, pairs } => {
                let mut result = self.process(*lhs)?;
                if result.attrs.group.is_some() {
                    let err = self.error("S0210", span, pos);
                    return self.fail(err);
                }
                let pairs = self.pairs(pairs, None)?;
                result.attrs.group = Some(Box::new(Group {
                    pairs,
                    span: Span::new(pos.saturating_sub(1), span.end),
                }));
                result.span = result.span.to(span);
                result
            }
            RawKind::Sort { lhs, terms } => {
                let mut result = self.process(*lhs)?;
                if !matches!(result.kind, NodeKind::Path { .. }) {
                    result = Node::path(vec![result]);
                }
                let mut seeking = Vec::new();
                let mut sort_terms = Vec::with_capacity(terms.len());
                for (descending, expr) in terms {
                    let expr = self.process(expr)?;
                    seeking.extend(ancestry(&expr));
                    sort_terms.push(SortTerm { descending, expr });
                }
                let mut sort = Node::new(
                    NodeKind::Sort { terms: sort_terms },
                    Span::new(pos.saturating_sub(1), span.end),
                    pos,
                );
                sort.attrs.seeking_parent = seeking;
                result.steps_mut().expect("path").push(sort);
                self.resolve_ancestry(&mut result)?;
                result.span = result.span.to(span);
                result
            }
            RawKind::Array { items } => {
                let mut seeking = Vec::new();
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    let item = self.process(item)?;
                    seeking.extend(ancestry(&item));
                    out.push(item);
                }
                let mut node = Node::new(NodeKind::Array { items: out }, span, pos);
                node.attrs.seeking_parent = seeking;
                node
            }
            RawKind::Object { pairs } => {
                let mut seeking = Vec::new();
                let pairs = self.pairs(pairs, Some(&mut seeking))?;
                let mut node = Node::new(NodeKind::Object { pairs }, span, pos);
                node.attrs.seeking_parent = seeking;
                node
            }
            RawKind::Negate { expr } => {
                let inner = self.process(*expr)?;
                if let NodeKind::Number { value } = inner.kind {
                    // Fold `-number` into a negative literal.
                    let mut node = inner;
                    node.kind = NodeKind::Number { value: -value };
                    node.span = span;
                    node
                } else {
                    let seeking = ancestry(&inner);
                    let mut node = Node::new(
                        NodeKind::Negate {
                            expr: Box::new(inner),
                        },
                        span,
                        pos,
                    );
                    node.attrs.seeking_parent = seeking;
                    node
                }
            }
            RawKind::Function {
                procedure,
                args,
                partial,
            } => {
                let mut seeking = Vec::new();
                let mut out = Vec::with_capacity(args.len());
                for arg in args {
                    let arg = self.process(arg)?;
                    seeking.extend(ancestry(&arg));
                    out.push(arg);
                }
                let procedure = self.process_boxed(*procedure)?;
                let kind = if partial {
                    NodeKind::Partial {
                        procedure,
                        args: out,
                    }
                } else {
                    NodeKind::Function {
                        procedure,
                        args: out,
                    }
                };
                let mut node = Node::new(kind, span, pos);
                node.attrs.seeking_parent = seeking;
                node
            }
            RawKind::Placeholder => Node::new(NodeKind::Placeholder, span, pos),
            RawKind::Lambda {
                params,
                signature,
                body,
            } => {
                let params = params
                    .into_iter()
                    .map(|p| match p.kind {
                        RawKind::Variable { value } => value,
                        _ => String::new(),
                    })
                    .collect();
                let body = self.process(*body)?;
                Node::new(
                    NodeKind::Lambda(Arc::new(LambdaDef {
                        params,
                        signature,
                        body: tail_call_optimize(body),
                        thunk: false,
                    })),
                    span,
                    pos,
                )
            }
            RawKind::Condition {
                condition,
                then,
                otherwise,
            } => {
                let condition = self.process_boxed(*condition)?;
                let mut seeking = ancestry(&condition);
                let then = self.process_boxed(*then)?;
                seeking.extend(ancestry(&then));
                let otherwise = match otherwise {
                    Some(e) => {
                        let e = self.process_boxed(*e)?;
                        seeking.extend(ancestry(&e));
                        Some(e)
                    }
                    None => None,
                };
                let mut node = Node::new(
                    NodeKind::Condition {
                        condition,
                        then,
                        otherwise,
                    },
                    span,
                    pos,
                );
                node.attrs.seeking_parent = seeking;
                node
            }
            RawKind::Transform {
                pattern,
                update,
                delete,
            } => {
                let pattern = self.process(*pattern)?;
                let update = self.process(*update)?;
                let delete = match delete {
                    Some(d) => Some(self.process(*d)?),
                    None => None,
                };
                Node::new(
                    NodeKind::Transform(Arc::new(TransformDef {
                        pattern,
                        update,
                        delete,
                    })),
                    span,
                    pos,
                )
            }
            RawKind::Block { exprs } => {
                let mut seeking = Vec::new();
                let mut cons_array = false;
                let mut out = Vec::with_capacity(exprs.len());
                for expr in exprs {
                    let part = self.process(expr)?;
                    seeking.extend(ancestry(&part));
                    let first_step_cons = match &part.kind {
                        NodeKind::Path { steps, .. } => {
                            steps.first().is_some_and(|s| s.attrs.cons_array)
                        }
                        _ => false,
                    };
                    if part.attrs.cons_array || first_step_cons {
                        cons_array = true;
                    }
                    out.push(part);
                }
                let mut node = Node::new(
                    NodeKind::Block {
                        exprs: out,
                        frame: true,
                    },
                    span,
                    pos,
                );
                node.attrs.seeking_parent = seeking;
                node.attrs.cons_array = cons_array;
                node
            }
            RawKind::Name { value } => {
                let mut step = Node::new(NodeKind::Name { name: value }, span, pos);
                step.attrs.keep_array = keep_array;
                step.annotation = annotation.take();
                let mut path = Node::path(vec![step]);
                if let NodeKind::Path {
                    keep_singleton_array,
                    ..
                } = &mut path.kind
                {
                    *keep_singleton_array = keep_array;
                }
                path
            }
            RawKind::Parent => {
                let slot = self.slots.len();
                self.slots.push(Slot {
                    label: format!("!{slot}"),
                    level: 1,
                });
                Node::new(
                    NodeKind::Parent {
                        slot,
                        label: String::new(),
                    },
                    span,
                    pos,
                )
            }
            RawKind::Variable { value } => Node::new(NodeKind::Variable { name: value }, span, pos),
            RawKind::String { value } => Node::new(NodeKind::String { value }, span, pos),
            RawKind::Number { value } => Node::new(NodeKind::Number { value }, span, pos),
            RawKind::Value { value } => match value {
                Lit::Bool(value) => Node::new(NodeKind::Bool { value }, span, pos),
                Lit::Null => Node::new(NodeKind::Null, span, pos),
            },
            RawKind::Regex { pattern, flags } => {
                Node::new(NodeKind::Regex { pattern, flags }, span, pos)
            }
            RawKind::Wildcard => Node::new(NodeKind::Wildcard, span, pos),
            RawKind::Descendant => Node::new(NodeKind::Descendant, span, pos),
            RawKind::End => {
                let err = self.error("S0207", span, pos).with("token", "(end)");
                self.fail(err)?
            }
            RawKind::Error { error, lhs } => match (lhs, error) {
                (Some(lhs), _) => self.process(*lhs)?,
                (None, Some(error)) => Node::new(NodeKind::Error { error }, span, pos),
                (None, None) => {
                    let err = self.error("S0201", span, pos);
                    self.fail(err)?
                }
            },
        };
        if keep_array {
            result.attrs.keep_array = true;
        }
        if let Some(a) = annotation {
            result.annotation = Some(a);
        }
        Ok(result)
    }

    fn pairs(
        &mut self,
        pairs: Vec<(Raw, Raw)>,
        mut seeking: Option<&mut Vec<SlotId>>,
    ) -> Result<Vec<(Node, Node)>> {
        let mut out = Vec::with_capacity(pairs.len());
        for (key, value) in pairs {
            let key = self.process(key)?;
            let value = self.process(value)?;
            if let Some(seeking) = seeking.as_deref_mut() {
                seeking.extend(ancestry(&key));
                seeking.extend(ancestry(&value));
            }
            out.push((key, value));
        }
        Ok(out)
    }

    /// `lhs.rhs`
    fn path(&mut self, lhs: Raw, rhs: Raw, span: Span) -> Result<Node> {
        let lstep = self.process(lhs)?;
        let parent_slot = match lstep.kind {
            NodeKind::Parent { slot, .. } => Some(slot),
            _ => None,
        };
        let mut result = if matches!(lstep.kind, NodeKind::Path { .. }) {
            lstep
        } else {
            Node::path(vec![lstep])
        };
        if let Some(slot) = parent_slot {
            result.attrs.seeking_parent = vec![slot];
        }
        let mut rest = self.process(rhs)?;
        let steps = result.steps_mut().expect("path");
        match rest.kind {
            NodeKind::Path { steps: more, .. } => steps.extend(more),
            _ => {
                if !rest.attrs.predicate.is_empty() {
                    rest.attrs.stages = std::mem::take(&mut rest.attrs.predicate);
                }
                steps.push(rest);
            }
        }

        // Literal steps: strings become names; numbers, booleans and null are not allowed.
        let mut bad = None;
        for (i, step) in steps.iter_mut().enumerate() {
            match &step.kind {
                NodeKind::String { value } => {
                    step.kind = NodeKind::Name {
                        name: value.clone(),
                    }
                }
                NodeKind::Number { value } => {
                    bad.get_or_insert((i, serde_json::Value::from(*value), step.span, step.pos));
                }
                NodeKind::Bool { value } => {
                    bad.get_or_insert((i, serde_json::Value::from(*value), step.span, step.pos));
                }
                NodeKind::Null => {
                    bad.get_or_insert((i, serde_json::Value::Null, step.span, step.pos));
                }
                _ => {}
            }
        }
        if let Some((i, value, step_span, step_pos)) = bad {
            let err = self
                .error("S0213", step_span, step_pos)
                .with("value", value);
            let replacement = self.fail(err)?;
            result.steps_mut().expect("path")[i] = replacement;
        }

        let steps = result.steps_mut().expect("path");
        let keep = steps.iter().any(|s| s.attrs.keep_array);
        if let Some(first) = steps.first_mut()
            && matches!(first.kind, NodeKind::Array { .. })
        {
            first.attrs.cons_array = true;
        }
        if let Some(last) = steps.last_mut()
            && matches!(last.kind, NodeKind::Array { .. })
        {
            last.attrs.cons_array = true;
        }
        if keep
            && let NodeKind::Path {
                keep_singleton_array,
                ..
            } = &mut result.kind
        {
            *keep_singleton_array = true;
        }
        self.resolve_ancestry(&mut result)?;
        result.span = result.span.to(span);
        Ok(result)
    }

    /// `lhs[rhs]`
    fn filter(&mut self, lhs: Raw, rhs: Raw, span: Span, pos: usize) -> Result<Node> {
        let mut result = self.process(lhs)?;
        let in_path = matches!(result.kind, NodeKind::Path { .. });
        let has_group = last_step(&mut result).attrs.group.is_some();
        if has_group {
            let err = self.error("S0209", span, pos);
            return self.fail(err);
        }
        let predicate = self.process(rhs)?;
        let stage_span = Span::new(pos.saturating_sub(1), span.end);
        {
            let step = last_step(&mut result);
            for slot in predicate.attrs.seeking_parent.clone() {
                if self.slots[slot].level == 1 {
                    self.seek_parent(step, slot)?;
                } else {
                    self.slots[slot].level -= 1;
                }
            }
            step.attrs.seeking_parent.extend(ancestry(&predicate));
            let stage = Stage::Filter {
                expr: Box::new(predicate),
                span: stage_span,
            };
            if in_path {
                step.attrs.stages.push(stage);
            } else {
                step.attrs.predicate.push(stage);
            }
            step.span = step.span.to(span);
        }
        result.span = result.span.to(span);
        Ok(result)
    }

    /// `lhs@$var`
    fn focus(&mut self, lhs: Raw, rhs: Raw, span: Span, pos: usize, keep: bool) -> Result<Node> {
        let mut result = self.process(lhs)?;
        let var = variable_name(&rhs);
        let (has_predicates, is_sort) = {
            let step = last_step(&mut result);
            (
                !step.attrs.stages.is_empty() || !step.attrs.predicate.is_empty(),
                matches!(step.kind, NodeKind::Sort { .. }),
            )
        };
        if has_predicates {
            let err = self.error("S0215", span, pos);
            return self.fail(err);
        }
        if is_sort {
            let err = self.error("S0216", span, pos);
            return self.fail(err);
        }
        let step = last_step(&mut result);
        if keep {
            step.attrs.keep_array = true;
        }
        step.attrs.focus = Some(var);
        step.attrs.tuple = true;
        result.span = result.span.to(span);
        Ok(result)
    }

    /// `lhs#$var`
    fn index(&mut self, lhs: Raw, rhs: Raw, span: Span, pos: usize) -> Result<Node> {
        let mut result = self.process(lhs)?;
        let var = variable_name(&rhs);
        if !matches!(result.kind, NodeKind::Path { .. }) {
            if !result.attrs.predicate.is_empty() {
                result.attrs.stages = std::mem::take(&mut result.attrs.predicate);
            }
            result = Node::path(vec![result]);
        }
        let step = last_step(&mut result);
        if step.attrs.stages.is_empty() {
            step.attrs.index = Some(var);
        } else {
            step.attrs.stages.push(Stage::Index {
                var,
                span: Span::new(pos.saturating_sub(1), span.end),
            });
        }
        step.attrs.tuple = true;
        result.span = result.span.to(span);
        Ok(result)
    }

    /// Walks back from `node` to find the step that a parent operator refers to.
    fn seek_parent(&mut self, node: &mut Node, slot: SlotId) -> Result<()> {
        crate::stack::grow(|| self.seek_parent_inner(node, slot))
    }

    fn seek_parent_inner(&mut self, node: &mut Node, slot: SlotId) -> Result<()> {
        match &mut node.kind {
            NodeKind::Name { .. } | NodeKind::Wildcard => {
                self.slots[slot].level -= 1;
                if self.slots[slot].level == 0 {
                    if let Some(existing) = node.attrs.ancestor {
                        // Reuse the label of the ancestor already bound here.
                        let label = self.slots[existing].label.clone();
                        self.slots[slot].label = label;
                    }
                    node.attrs.ancestor = Some(slot);
                    node.attrs.tuple = true;
                }
            }
            NodeKind::Parent { .. } => self.slots[slot].level += 1,
            NodeKind::Block { exprs, .. } => {
                if let Some(last) = exprs.last_mut() {
                    node.attrs.tuple = true;
                    self.seek_parent(last, slot)?;
                }
            }
            NodeKind::Path { steps, .. } => {
                node.attrs.tuple = true;
                let mut index = steps.len() as isize - 1;
                self.seek_parent(&mut steps[index as usize], slot)?;
                index -= 1;
                while self.slots[slot].level > 0 && index >= 0 {
                    self.seek_parent(&mut steps[index as usize], slot)?;
                    index -= 1;
                }
            }
            _ => {
                let err = self
                    .error("S0217", node.span, node.pos)
                    .with("token", node.type_name());
                self.report(err)?;
            }
        }
        Ok(())
    }

    /// Resolves the parent operators sought by the last step of `path` against earlier steps.
    fn resolve_ancestry(&mut self, path: &mut Node) -> Result<()> {
        let Node { kind, attrs, .. } = path;
        let NodeKind::Path { steps, .. } = kind else {
            return Ok(());
        };
        let Some(last) = steps.last() else {
            return Ok(());
        };
        let mut slots = last.attrs.seeking_parent.clone();
        if let NodeKind::Parent { slot, .. } = last.kind {
            slots.push(slot);
        }
        for slot in slots {
            let mut index = steps.len() as isize - 2;
            while self.slots[slot].level > 0 {
                if index < 0 {
                    attrs.seeking_parent.push(slot);
                    break;
                }
                let mut i = index as usize;
                index -= 1;
                // Multiple contiguous steps that bind the focus are skipped together.
                while index >= 0
                    && steps[i].attrs.focus.is_some()
                    && steps[index as usize].attrs.focus.is_some()
                {
                    i = index as usize;
                    index -= 1;
                }
                self.seek_parent(&mut steps[i], slot)?;
            }
        }
        Ok(())
    }
}

/// The ancestor slots that `value` passes up to its parent node (`pushAncestry`).
fn ancestry(value: &Node) -> Vec<SlotId> {
    let mut slots = value.attrs.seeking_parent.clone();
    if let NodeKind::Parent { slot, .. } = value.kind {
        slots.push(slot);
    }
    slots
}

fn last_step(node: &mut Node) -> &mut Node {
    if matches!(node.kind, NodeKind::Path { .. }) {
        node.steps_mut()
            .and_then(|s| s.last_mut())
            .expect("non-empty path")
    } else {
        node
    }
}

fn variable_name(raw: &Raw) -> String {
    match &raw.kind {
        RawKind::Variable { value } => value.clone(),
        _ => String::new(),
    }
}

/// Wraps function calls in tail position in a thunk, so the trampoline in the evaluator can
/// run tail-recursive functions without growing the stack.
fn tail_call_optimize(expr: Node) -> Node {
    // A chain of conditions in a lambda's body recurses hundreds of times, with nodes by value.
    crate::stack::grow(|| tail_call_optimize_inner(expr))
}

fn tail_call_optimize_inner(mut expr: Node) -> Node {
    match &mut expr.kind {
        NodeKind::Function { .. } if expr.attrs.predicate.is_empty() => {
            let (span, pos) = (expr.span, expr.pos);
            Node::new(
                NodeKind::Lambda(Arc::new(LambdaDef {
                    params: Vec::new(),
                    signature: None,
                    body: expr,
                    thunk: true,
                })),
                span,
                pos,
            )
        }
        NodeKind::Condition {
            then, otherwise, ..
        } => {
            let t = std::mem::replace(then.as_mut(), placeholder());
            **then = tail_call_optimize(t);
            if let Some(e) = otherwise {
                let inner = std::mem::replace(e.as_mut(), placeholder());
                **e = tail_call_optimize(inner);
            }
            expr
        }
        NodeKind::Fallback { rhs, .. } => {
            let inner = std::mem::replace(rhs.as_mut(), placeholder());
            **rhs = tail_call_optimize(inner);
            expr
        }
        NodeKind::Block { exprs, .. } => {
            if let Some(last) = exprs.pop() {
                exprs.push(tail_call_optimize(last));
            }
            expr
        }
        _ => expr,
    }
}

/// Decides whether a block needs a frame of its own: when one of its expressions may bind a
/// variable in the frame it is evaluated in. Each other block gets a frame of its own if it
/// needs one, and a lambda binds in the frame of its call, so neither is looked into.
///
/// `$eval` binds in the frame of its caller, as in jsonata-js, so when `eval` is set (the
/// expression names `$eval`, or was parsed by it) a block whose expressions call a function
/// keeps its frame too: the function called may be `$eval` under another name.
pub(crate) fn mark_frame(node: &mut Node, eval: bool) {
    fn binds(node: &Node, eval: bool) -> bool {
        match &node.kind {
            NodeKind::Bind { .. } => true,
            NodeKind::Function { .. } | NodeKind::Apply { .. } if eval => true,
            NodeKind::Block { .. } | NodeKind::Lambda(_) => false,
            _ => {
                let mut found = false;
                node.for_each_child(|c| found = found || crate::stack::grow(|| binds(c, eval)));
                found
            }
        }
    }
    if let NodeKind::Block { exprs, frame } = &mut node.kind {
        *frame = exprs.iter().any(|e| binds(e, eval));
    }
}

fn placeholder() -> Node {
    Node::new(NodeKind::Null, Span::default(), 0)
}

/// Maps every byte offset of `src` (and one past the end) to its UTF-16 offset.
fn utf16_table(src: &str) -> Vec<usize> {
    let mut table = Vec::with_capacity(src.len() + 1);
    let mut units = 0;
    for ch in src.chars() {
        for _ in 0..ch.len_utf8() {
            table.push(units);
        }
        units += ch.len_utf16();
    }
    table.push(units);
    table
}
