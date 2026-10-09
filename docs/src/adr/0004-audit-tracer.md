# 0004 — Audit evaluations through a tracer interface in the evaluator

**Status:** Accepted

## Context

The original rule-engine design produced an **audit tree** for every evaluation. It recorded what
each condition checked, the actual and expected values, and whether the condition passed. jlens
keeps the language pure JSONata, with no separate rules layer, but it must still explain *why*
an expression produced its result. JSONata itself only returns the final value.

Options considered:

1. **Re-evaluate sub-expressions after the fact.** This is simple, but it doubles the cost, and
   it cannot see the state inside the evaluation: per-item filter outcomes, short-circuits and
   bound variables.
2. **Rewrite the expression** to emit diagnostics. This is brittle and changes semantics.
3. **Instrument the evaluator** with a tracer that receives events as evaluation proceeds.

## Decision

The evaluator is generic over a `Tracer` trait. The trait receives these events:
- `enter` and `exit` per node;
- `predicate` per filtered item;
- `branch`, `bind`, `call` and `short_circuit`.

Two implementations exist:

- **`NoopTracer`** is the default. All of its methods are empty and inlined, so tracing costs
  close to nothing when it is off. The budget is under 3%, checked by benchmarks.
- **`AuditCollector`** builds a serializable `AuditTree`. It offers capture levels (`Off`,
  `Decisions`, `Full`) and capture limits: value size, items per sequence, depth and node count.

An `Explanation` is derived from the tree: which operands made an `and` fail, which items a
filter rejected, and so on. It renders as text for the CLI or as JSON for user interfaces.
Optional `tracing` integration and JSONL event export cover operational logging.

## Measurement

The untraced hot path stays the same as before: when no tracer is attached, `Evaluator::eval`
checks for one and calls the original code, while the traced variant is a separate, cold
function. Interleaved A/B runs of `examples/bench_eval.rs` against the evaluator before
tracing measured between −5.7% and +2.7% across five workloads, which is within the machine's
noise and inside the 3% budget.

## Consequences

- Auditing reuses the real evaluation, so what the audit shows is exactly what happened.
- The evaluator code has to emit events consistently. This adds work that is comparable in size
  to the core evaluator, and the roadmap plans for it as its own phase.
- Audit trees for large inputs are bounded by the capture limits, so audits stay usable in
  production.
