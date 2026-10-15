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
| `Function(Rc<Function>)` | A lambda closure, a built-in, a host function, a partial application, a transformer, or a regex |

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
parameters. Host functions (`HostFunction`, from the bindings) have an optional signature and an
arity; their arguments and results are converted to and from JSON
([ADR 0006](../adr/0006-host-functions.md)). Partial application (`$f(?, 2)`) works for lambdas,
built-ins and host functions.

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
| Evaluation depth | `Options::max_depth` | `D1011` | 50,000 native, 500 in Wasm |
| Wall-clock time | `Options::timeout` | `D1012` | none |
| Cancellation from another thread | `Options::cancel_token` | `U1003` | none |
| Expression nesting | (fixed) | `U1001` | 1,000 levels of syntax tree, checked by the parser |
| Value nesting | (fixed) | `U1001` | 1,000 levels of arrays and objects, checked where a value is converted to JSON, compared or serialized |
| Length of a built string | `Options::max_string_length` | `U1001` | 256 MiB (in bytes of UTF-8) |
| Items of a built array | `Options::max_array_length` | `U1001` | 16,777,216 |
| Size of the result, and of the arguments of a host function | `Options::max_result_size` | `U1001` | 256 MiB (in bytes of compact JSON) |

jsonata-js has no depth limit of its own; it stops when the JavaScript stack overflows. jlens
needs limits so that it fails with an error instead of crashing the process.

- **Native targets.** The parser, the post-processing pass, the evaluator and the schema typing
  walker grow their stack on demand (with the `stacker` crate), so the default depth limit is
  generous. The CLI also runs on a thread with a 64 MiB stack, which covers the other recursive
  walks of the tree (printing it as JSON, for example).
- **WebAssembly.** The stack cannot grow, and an overflow traps, which leaves the module
  instance unusable. jlens links the module with a 4 MiB stack and checks the space left before
  each recursive step (`jlens_core::stack`), failing with `D1011` (evaluation) or `U1001`
  (parsing) instead. WebAssembly frames also use the JavaScript engine's own stack, which is about
  1 MiB in V8 and cannot be measured from inside the module; the lower default depth limit
  (500) keeps evaluations inside it. The [WebAssembly guide](../guides/wasm.md#limits-and-memory)
  has the measurements behind it. A larger `maxDepth` can raise a JavaScript `RangeError` and
  break the instance.
- **Calls from native functions.** A lambda whose body is a tail call leaves no expression on
  the stack, so a recursion through a higher-order function
  (`function($n){ $map([$n], function($v){ $f($v) }) }`) deepens only through the native
  function's calls back. Each of those calls counts as a level of depth and checks the stack
  too.

The nesting limit bounds every recursive walk of the syntax tree, including the ones that do
not grow the stack, such as cloning and dropping it. Both nested subexpressions and the operators
of a chain count: `1 + 2 + 3` is a tree two levels deep, built by a loop rather than by
recursion. 1,000 levels is far more than any hand-written expression uses.

Values can nest much deeper than expressions: an input read from a `Source` (or from JavaScript
in Wasm) has no depth limit, and a tail-recursive function can wrap a value in an object at each
of its steps, without deepening the evaluation. So the walks over values are bounded in three
ways:

- **Walks that only enter arrays** (`**`'s and `*`'s flattening, field lookups across nested
  arrays, `$boolean`, `$keys`, `$spread`) keep their own stack, and walk any depth.
- **Walks that enter objects too** fail with `U1001` beyond 1,000 levels of arrays and objects:
  converting the result or a host function's arguments to JSON, `=` and `$distinct`, `**`,
  `$string` and `$clone`. They also grow the stack natively. The limit protects the caller as
  well: the JSON returned is dropped and printed by recursive code (`serde_json`), and in Wasm
  the walks run on the JavaScript engine's stack. jsonata-js works at 2,000 levels and raises a
  `RangeError` before 5,000. Audits truncate the values they capture at the same depth.
- **Dropping** counts how deep the drops of arrays and objects nest. Past 100 levels, the
  arrays and objects that only the dropped one holds are emptied from a list, instead of each
  inside the drop of its parent. Those that hold no array or object of their own, most of them,
  skip the count.
- **Recursive walks** grow the stack natively every 16 levels rather than at each value.

The size limits stop an expression that doubles a string or an array on each step, or a path
whose steps multiply the length of a sequence, before it runs out of memory, which would abort
the process. They are checked by the operators that build strings and arrays (`&`, ranges,
array constructors, path steps) and on the value that each built-in or host function returns;
`$join` checks the length before joining. The operators check before they build the value; a
function's result is checked once it has returned. `*` and `**`, which flatten nested arrays,
count the items as they go.

The string and array limits apply to each string and array on its own, and values are shared
rather than copied: an object bound to a variable can be nested in a new object twice, then
that object twice, and so on. Thirty such steps stand for about a billion copies of the first object while
using little memory, until something writes them all out. So everything that writes a value
out is limited too:

- **The result**, which becomes JSON when the evaluation ends, is measured as it is converted:
  its size is the length of its compact JSON text, counting a shared value each time it
  appears. The conversion stops once the size passes `Options::max_result_size`, and the
  evaluation fails with `U1001`. Escaped characters count as one byte, so the size is slightly
  below the length of the text that `format_json` prints.
- **The arguments of a host function** are measured together in the same way, and so is its
  context value (`HostCall::input`).
- **`$string`, `$clone`** (and the transform operator, which clones its input) and `&` write
  JSON text, which is a string: the text is checked against `Options::max_string_length` as it
  is written.
- **The `value` of an error** (`T2001`, `T0410` and others) is left out when it is larger than
  the default size of a result.

## Tracing hooks

The evaluator reports its work to an optional `Tracer`; [ADR 0004](../adr/0004-audit-tracer.md)
records why. The hooks sit at these points:

| Hook | Where |
|---|---|
| `enter` / `exit` | `Evaluator::eval`, around every node, including errors |
| `operands` | `binary`, before a comparison or `in` |
| `short_circuit` | `binary`, when `and`/`or` skips its right side |
| `branch` | the conditional, after the condition |
| `bind` | `:=`, before binding |
| `filter_begin` / `filter_item` / `filter_end` | `filter`, around expression predicates (numeric indexes are not reported) |

When no tracer is attached, `eval` calls the untraced code directly. The traced variant is a
separate function marked cold, so the hot path does not change. The
[audit guide](../guides/audit.md) describes the tracers built on these hooks.
