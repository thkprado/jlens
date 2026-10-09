# Evaluation

The evaluator (`crates/jlens-core/src/eval.rs`) is a tree-walking interpreter over the
processed [syntax tree](parser.md). It is a function-by-function port of `src/jsonata.js` in
jsonata-js. That code effectively *defines* JSONata's subtler behaviour, such as when a result
is a sequence or an array, so jlens keeps its structure recognisable. Unlike jsonata-js,
evaluation is synchronous.

```text
Expression::evaluate(input, bindings)
  └─ eval::evaluate          converts input JSON, wraps an input array, binds $$ and bindings
       └─ Evaluator::eval    one call per node: dispatch, predicates, group-by, sequence unwrapping
            ├─ path / step / tuple_step / filter / stages      location paths
            ├─ binary / array_constructor / group / sort       operators and constructors
            └─ call / apply / partial / apply_expression       functions
```

## Values

See [ADR 0003](../adr/0003-value-model.md) for the reasoning. The short version:

| `Value` | Notes |
|---|---|
| `Undefined` | Never escapes the API: results become `Option<serde_json::Value>` |
| `Null`, `Bool`, `Number(f64)`, `String(Rc<str>)` | Numbers are IEEE doubles, as in JavaScript |
| `Array(Rc<Array>)` | Items plus flags: `SEQUENCE`, `KEEP_SINGLETON`, `OUTER_WRAPPER`, `CONS`, `TUPLE_STREAM` |
| `Object(Rc<RefCell<Map>>)` | Insertion-ordered. Shared and mutable because transforms update matches in place |
| `Function(Rc<Function>)` | A lambda closure, a built-in, a partial application, a transformer, or a regex |

### Sequences

A *sequence* is the result of navigating a path. After evaluating any node, a sequence of
length 0 becomes undefined, and a sequence of length 1 becomes its only item, unless
`KEEP_SINGLETON` is set by the `[]` modifier. Arrays that come from the input, and arrays built
with `[...]`, are not sequences, so they never collapse. This single rule, applied at the end of
`Evaluator::eval`, is why `Account.Order.Product.Price` returns a number for one product and an
array for several.

When the input document is itself an array, it is wrapped in a one-item sequence flagged
`OUTER_WRAPPER`, so that the array counts as a single context value. `$` unwraps it again.

## Paths and tuple streams

`path` evaluates its steps left to right. Each step maps over the current sequence and flattens
the results by one level. Array constructors flagged `CONS` are not flattened.

Steps that bind variables switch the path into *tuple-stream* mode from that step on. These are
`@$focus`, `#$index`, and steps marked as the ancestor for a `%`. In this mode each item is
carried as a tuple, an object holding `@` (the current value) and the bound variables. Later
steps, predicates, sort keys and group-by expressions are evaluated in a frame created from the
tuple. At the end of the path the tuples are mapped back to their `@` values, unless an
enclosing path still needs the ancestry.

## Functions and the trampoline

A lambda value captures its definition (`Arc<LambdaDef>`, shared with the tree), the
environment frame in which it was created, and the context value. Applying it:

1. validates the arguments against the lambda's signature, if it has one;
2. creates a child frame of the captured environment and binds the parameters (missing ones are
   bound to undefined, so they shadow outer variables);
3. evaluates the body.

When post-processing marked a call as being in tail position, the body evaluates to a
*thunk* instead of making the call. `Evaluator::apply` runs thunks in a loop, so tail-recursive
functions run in constant stack depth. `$loop(100000, 0)` in the test suite exercises this.

Built-in functions are `Native` entries in `functions/mod.rs`, each with its signature and declared
parameters. Partial application (`$f(?, 2)`) works for both lambdas and built-ins.

## Signature validation

Signatures such as `<s-nn?:s>` are checked by turning the arguments into a string of type
symbols (`s`, `n`, `b`, `l`, `a`, `o`, `f`, or `m` for undefined). That string is matched
against the parameter patterns by a small greedy backtracking matcher. The matcher is equivalent
to the regular expression jsonata-js builds, but needs no regex engine. The match then:
- fills in the context value for `-` parameters (`T0411` if its type is wrong);
- wraps single values for `a` parameters;
- checks array subtypes (`T0412`).

## Environments and memory

Frames form a chain from the innermost scope to the root; the built-ins sit behind the root. A
closure stored in the frame that it captured creates a reference cycle, as in
`$f := function(){ $f() }`. Reference counting cannot reclaim such a cycle on its own. The
evaluator therefore records every frame that receives a function, array or object binding, and
clears those frames when the evaluation ends.

## Guardrails

| Limit | Option | Error | Default |
|---|---|---|---|
| Evaluation depth | `Options::max_depth` | `D1011` | 50,000 native, 3,000 in Wasm |
| Wall-clock time | `Options::timeout` | `D1012` | none |

jsonata-js has no depth limit of its own; it stops when the JavaScript stack overflows. jlens
needs a limit so that it fails with an error instead of crashing the process. On native
targets, the evaluator grows its stack on demand (with the `stacker` crate), so the default
limit is generous.
