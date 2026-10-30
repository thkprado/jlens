# 0006 — Host functions with a JSON boundary

**Status:** Accepted; extended by [ADR 0011](0011-host-callbacks.md), which lets host functions
call the functions they receive. Amended in 0.25: host functions are looked up when no frame
holds a name ([below](#amendment-host-functions-looked-up-on-a-miss-025)).

## Context

jsonata-js lets applications pass their own functions in the bindings of an evaluation, or
register them on an expression with `registerFunction`. Rule engines use this to reach data and
services the document does not hold: rate tables, currency conversion, lookups. jlens had no
equivalent, because `Bindings` held only JSON values.

A host function has to fit four constraints:

- `Expression` is `Send + Sync`, and `Bindings` is too. Applications share both between threads.
- The run-time `Value` (ADR 0003) is internal. It is built on `Rc`, so it is not `Send`. It also
  carries the sequence flags of jsonata-js, which a host should not have to know about.
- Evaluations never mutate their input, and a `Document` is shared between evaluations.
- Without a tracer, the evaluator must not get slower (ADR 0004).

Options considered for the values that cross the boundary:

1. **Expose `Value`**, with accessors. This is the fastest option, with no conversion, and
   callbacks are possible. But it makes the internal model, its flags and its `Rc`s a public API.
2. **A new public value type.** It needs a conversion anyway, and adds a third value model next
   to `serde_json::Value` and the internal one.
3. **`serde_json::Value`**, with `None` for *undefined*. This is the type of documents, results
   and variable bindings already, and every Rust JSON tool works with it.

## Decision

- **The boundary is JSON.** A `HostFunction` wraps an
  `Arc<dyn Fn(&[Option<JsonValue>], &HostCall) -> Result<Option<JsonValue>> + Send + Sync>`.
  Arguments are converted with the same rules as results (`JSON.stringify`): a function argument
  arrives as `None`. The context value is converted only when the function asks for it
  (`HostCall::input`), since it can be the whole document.
- **Optional signatures** reuse the parser and validator of the built-in functions. A validated
  call behaves like a call to a built-in: same type errors, same context substitution. A
  function without a signature declares its arity, which plays the part of a JavaScript
  function's `length` for higher-order functions and partial application.
- **Bindings hold values and functions.** A name holds one or the other: binding one replaces the
  other. Each evaluation binds the host functions in its root frame next to the variables. So they
  take precedence over built-in functions of the same name, as in jsonata-js. (Since 0.25 they
  are looked up when no frame holds a name, with the same precedence: see the amendment.)
- **Function values** gain a `Host` variant. Partial application of built-in functions is
  generalized to host functions: `Partial` holds the target function.
- **Errors** are `jlens_core::Error`s with a code chosen by the host. As for any error, the call
  site fills in the position and token when the error has none.

## Consequences

- `Bindings` and `HostFunction` are `Send + Sync`. In WebAssembly, JavaScript functions are not
  `Send`, so the bindings refer to them indirectly, through a registry local to the evaluation.
- In WebAssembly, `this` is `undefined`, where jsonata-js passes the context value. Passing it
  would convert the context value to JavaScript at every call, which is the whole document for
  a call at the top of the expression. A signature with `-` passes it as an argument instead.
  An exception thrown by a JavaScript function is kept, and the evaluation rethrows it as is. A
  promise is refused (`U1002`), since evaluation is synchronous.
- Each call copies its arguments and its result. For the functions that rule engines call, which
  take a few scalars, the cost is small. It is large only for a function that receives a large
  array or object.
- A host function cannot call a JSONata function it receives as an argument. A later decision can
  add a callable handle, or a public value type, without changing the functions written against
  this API. [ADR 0011](0011-host-callbacks.md) added that handle (`HostCall::function`) in 0.13.0.
- The evaluator's hot path only gains match arms. Tracing needs no new event: a host call is a
  function call node, with its `enter` and `exit` events.
- Host functions run to completion. The evaluation's timeout is checked again once they return.

## Amendment: host functions looked up on a miss (0.25)

### Context

Each evaluation bound every host function in its root frame before it started, a
`HostFunction` cloned and its name allocated, called or not. The Java package measured it
(ADR 0021, 0.25 #7): about 0.09 µs per function per evaluation, and in Rust `$a + $b` took
2.82 µs with sixteen functions bound against 0.39 µs with none. A rule engine that binds dozens
of functions paid microseconds for each rule, in every package.

### Decision

The root frame looks a name up among the bindings' host functions when no frame holds it,
before the built-ins, and keeps the value it finds, as it keeps a built-in's. The precedence
does not change: a variable, of the bindings or a document bound by `evaluate_document_with`,
is in the root frame and found first; then the host functions; then the built-ins. A function
bound by the expression or by a lambda's parameter is in a nearer frame. The value kept makes
`$f = $f` hold, wherever in the expression each `$f` is.

### Consequences

- An evaluation pays only for the host functions it names, once each. On the maintainer's
  Windows machine, `$a + $b` takes 0.40 µs with one, four or sixteen functions bound, against
  0.39 µs with none; `$add(1, 2)` with sixteen bound 0.75 µs instead of 3.11.
- A name that no frame holds costs a lookup in the bindings' functions before the built-ins,
  once per evaluation and name, since the value is kept.
- Every lookup carries the bindings to the root frame, which a name found in a nearer frame
  never reaches, yet costs: `cargo xtask ab` against 0.24.0, built aligned, put `recursion`
  (`$f(15)`, about 2,000 calls of a lambda) 2.5 to 5.6% slower in every mode, the other cases
  within 2% (0.25 #15). Inlining `Frame::lookup` and `variable` brings it to about 2% (between
  1.2 and 2.6% over nine builds, by the fastest round); 0.24.0 rebuilt is within 1.1% of
  itself. The rest stays: keeping the functions in the root frame instead would tie the
  bindings to the lifetime of the values. (Undone in 0.26, below.)

### Follow-up: the functions in the root frame (0.26 #2)

Built as an application builds it, without the alignment flags, the 0.25 code ran `recursion`
4.0% slower than 0.24.0 with `evaluate` and 2.9% against a `Document`. The root frame now holds
the host functions itself: `Bindings` keeps them behind an `Arc`, which the root frame clones,
and holds `None` when there are none. The lifetime of the values is not involved, since the
frame owns its share of them. `Frame::lookup` and `variable` take no bindings again and are no
longer marked `#[inline]`, as in 0.24.0. The precedence and the value kept do not change.
Since 0.26 #11, `Bindings` itself holds `None` until its first function, so that empty bindings,
which a host may build at each call, allocate nothing (26 ns with mimalloc).

- `cargo xtask ab v0.24.0 .` puts `recursion` within 1.1% in both modes, built aligned or not
  (`evaluate` +0.7% unaligned and +1.0% aligned, `document` +0.5% and +1.1%), and every other
  case within 2.3%. One unaligned run against a `Document` gave `recursion` +8.8%; two more
  runs of the same build gave +0.1% (the case alone) and +0.5%.
- An evaluation with host functions pays one atomic increment and decrement of the `Arc`;
  `$a + $b` still takes 0.38 to 0.39 µs with none, one or sixteen bound, and `$add(1, 2)` with
  sixteen 0.72 µs, as before within 1%.
- Cloning `Bindings` shares the functions' map instead of copying it; adding or removing a
  function on a clone copies the map then, as cloning did before.
