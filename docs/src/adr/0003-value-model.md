# 0003 — An explicit value model with `Undefined` and sequence flags

**Status:** Accepted

## Context

JSONata semantics depend on distinctions that plain JSON values cannot express:

- **undefined vs null.** A missing path yields *undefined*, which disappears from arrays and
  objects. JSON `null` is a real value.
- **Sequences vs arrays.** Path results are *sequences*. An empty sequence is undefined, and a
  singleton sequence collapses to its only item. Arrays that come from the input, or that an
  explicit `[...]` constructor builds, are never collapsed. The `[]` modifier (`keepArray`) and
  the wrapper that makes an input array count as a single context value add more cases.
- **Callables.** Lambdas, native functions and regexes are first-class values.

The Java port encodes these as a `null` for undefined, a `NULL_VALUE` sentinel for JSON null, and
an `ArrayList` subclass that carries flags. Its documentation names this the most error-prone
part of the port.

## Decision

The evaluator works on its own `Value` type:

```rust
enum Value {
    Undefined, Null, Bool(bool), Number(f64), String(Rc<str>),
    Array(Rc<Array>),            // Array carries: sequence | keep_singleton | outer_wrapper | cons | tuple_stream
    Object(Rc<IndexMap<..>>),    // insertion order is observable in JSONata
    Lambda(..), Native(..), Regex(..), Range(..),
}
```

- Numbers are `f64`, as in JavaScript. Number-to-text conversion uses `ryu-js` so it matches
  JavaScript exactly.
- `Undefined` never crosses the public API: results are `Option<serde_json::Value>`.
- Values are reference-counted (`Rc`) for the lifetime of a single evaluation. The compiled AST
  is immutable and `Send + Sync`, so a compiled `Expression` can be shared between threads, and
  each thread evaluates with its own values.

## Consequences

- Each JSONata rule about undefined and sequences maps to a variant or a flag, and each is
  covered by a dedicated test.
- Input is converted once from `serde_json::Value` at the start of an evaluation. If profiling
  shows that this conversion dominates, a borrowed view of the input can be introduced later.
