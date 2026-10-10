# 0006 — Host functions with a JSON boundary

**Status:** Accepted

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
  take precedence over built-in functions of the same name, as in jsonata-js.
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
  this API.
- The evaluator's hot path only gains match arms. Tracing needs no new event: a host call is a
  function call node, with its `enter` and `exit` events.
- Host functions run to completion. The evaluation's timeout is checked again once they return.
