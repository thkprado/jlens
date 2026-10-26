# Audit and explainability

A JSONata expression returns a value. jlens can also tell you **why** it returned that value:
- which checks passed and which failed;
- what the compared values were;
- which items a filter rejected;
- which values were bound to variables.

The same evaluation, with the same result, produces an *audit tree*. An *explanation* is a
readable summary of that tree.

## A complete example

`fixtures/shipping-rule.jsonata` decides whether a store order gets free shipping.
Its conditions carry [annotations](../language/annotations.md):

```jsonata
$approved := (
  /** @label "Authorized customer" @message "The customer login must be on the approved partners list." */
  customer.login in ["alice.walker99", "bob.harrison42"]
  and
  /** @label "Minimum order grand total" @message "The order grand total must exceed 50,000.00." */
  $total > 50000
  and …
);
```

When a order grand totals only 37,000, `jlens eval --explain` prints:

```text
$ jlens eval -f fixtures/shipping-rule.jsonata -i order.json --explain
result: {"action":"REGULAR_FEE","code":"SHIP_FEE_402","delivery":"SLOW"}
• $total := $sum(items.(unitPrice * quantity)) = 37000
• $electronics := $sum(items[type = 'electronic'].(unitPrice * quantity)) = 17000
  • [type = 'electronic']: 2 of 3 items passed; rejected: {"code":"1","color":"blue",…}
• $blueTax := $sum(items[color = 'blue'].tax) = 1500
• $averageTax := $average(items.tax) = 2833.3333333333335
✘ $approved := ( customer.login in [...] and ... ) = false
  ✘ Minimum order grand total — $total > 50000 → false (37000 > 50000)
      The order grand total must exceed 50,000.00.
• $approved ? { ... } : { ... } → {"action":"REGULAR_FEE",…} [else]
```

The explanation follows only the decisions that mattered. The `and` chain failed, so it shows
only the operand that failed, with its business message. The checks that passed are left out.
For an approved order, every check is listed with `✔`.

## In Rust

```rust
use jlens_core::audit::{AuditOptions, explain};
use jlens_core::{Bindings, Expression, Options};

let rule = Expression::compile(&rule_source)?;
let (result, audit) = rule.evaluate_audited(
    Some(&document),
    &Bindings::new(),
    &Options::default(),
    &AuditOptions::decisions(),
);
println!("{}", explain(&audit));              // text
let json = serde_json::to_string(&audit)?;    // the full tree, e.g. to store with the decision
```

`evaluate_audited` returns the audit even when the evaluation fails. The error is in
`audit.header.error`, and on the node that raised it.

## What is recorded

The audit tree mirrors the evaluation, not the syntax: a node evaluated three times (once per
item, or once per function call) appears three times. Each `AuditNode` has these fields:

| Field | Meaning |
|---|---|
| `node`, `kind`, `op`, `span`, `source` | The syntax-tree node, its type, its operator or function, its location and text |
| `label`, `message`, `checkId` | From the node's annotation |
| `output`, `outcome` | The value produced, and whether it was `true`, `false`, another value, `undefined`, or an `error` |
| `operands` | For comparisons: the two values compared |
| `shortCircuited` | For `and`/`or`: the right side was not evaluated |
| `branch` | For conditionals: `then`, `else` or `none` |
| `binding` | For `$x := …`: the variable bound |
| `filters` | Predicate filters applied: items tested, items passed, and a sample of rejected items |
| `input` | The context value (level `full` only) |
| `children` | Recorded evaluations nested inside this one |

The header records:
- the engine version;
- FNV-1a hashes of the expression and of the input document, so you can tell later exactly
  what was evaluated;
- the evaluation timestamp (the value of `$now()`) and its duration;
- the result, or the error.

### Levels

| Level | Records | Typical use |
|---|---|---|
| `decisions` (default) | The root, plus nodes that decide: comparisons, `and`/`or`, conditionals, bindings, nodes producing booleans, nodes with filters, annotated nodes | Explanations, storing the reasons for a decision |
| `full` | Every evaluated node, with its input | Debugging an expression |
| `off` | Only the header | Recording what was evaluated and the result |

At level `decisions`, nodes that decide nothing are collapsed: their recorded descendants move
up to the nearest recorded ancestor. Inside a filter, the per-item evaluations are summarized
into the filter's statistics instead of being recorded one by one.

### Bounds

Audits of large documents stay bounded. `CaptureLimits` sets:

| Limit | Default | Effect |
|---|---|---|
| `max_items` | 10 | Items (or object fields) kept per captured value; the rest becomes `"… N more"` |
| `max_string` | 200 | Characters kept per string; longer strings end with `…` |
| `max_depth` | 4 | Nesting depth kept in captured values |
| `max_rejected` | 5 | Rejected items kept per filter |
| `max_nodes` | 10,000 | Recorded nodes, the root included; past it, a node is dropped, its recorded descendants and filters going to its parent, and `truncated` is set. A cut tree holds `node_count` nodes, all reached from the root |
| `max_source` | 200 | Characters of source text kept per node |

## Event logs and custom tracers

Auditing is built on a lower-level interface, the `Tracer` trait. The evaluator calls a
tracer's methods as it goes:
- `enter`/`exit` for each node;
- `operands` for each comparison;
- `short_circuit`, `branch` and `bind`;
- `filter_begin`, `filter_item` and `filter_end` for filters.

You can implement your own tracer, for example to collect metrics, and pass it to
`Expression::evaluate_traced`. jlens provides three:

| Tracer | Output |
|---|---|
| `AuditCollector` | The `AuditTree` described above |
| `JsonlTracer` | One JSON object per event (`jlens eval --trace-log events.jsonl`) |
| `TracingTracer` (feature `tracing`) | Events on the `tracing` target `jlens::eval`: entries and exits at `TRACE`, decisions at `DEBUG` |

## Performance

When no tracer is attached, auditing costs one check per evaluated node. Compared with the
evaluator before tracing existed, interleaved benchmark runs measured between −5.7% and +2.7%
across five workloads, within the noise of the machine. With a tracer attached, evaluation does
more work in proportion to what is recorded. Level `decisions` is the cheapest useful level:
for the shipping rule on a 200-item document, it takes about three times as long as an
evaluation without a tracer, and `full` about fifteen times. `cargo bench -p jlens-core -- audit`
measures this; see [Performance](../architecture/performance.md).

## Limitations

- Node spans refer to the expression's own source. For expressions evaluated through `$eval`,
  and for the internal helper behind function composition (`$f ~> $g`), the reported source
  snippets refer to a different text.
- The audit captures values as JSON. Functions appear as `"<function>"`, and undefined values
  are omitted.
