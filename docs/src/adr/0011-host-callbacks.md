# 0011 — Host functions call the functions they receive

**Status:** Accepted

## Context

ADR 0006 gave host functions a JSON boundary: a function passed as an argument arrives as
`None`, so a host function cannot apply it. Rule engines need that for helpers written in the
host language that take a function: `$retry($fetch, 3)`, `$memoize(function($k) {...})`, a lookup
with a fallback function.

What jsonata-js 2.2.2 does, checked in `benches/compare`:

- **Every function argument is wrapped.** A lambda, a built-in function (`$uppercase`), a partial
  application (`$substring(?, 1)`), a chain (`$uppercase ~> $trim`) and a host function all
  arrive as an `async` JavaScript function. Its `length` is 0 and its `arity` property holds the
  arity of the wrapped function. Only the arguments themselves are wrapped, not functions inside
  an array or an object.
- **Calling it applies the function** as `$map` does: extra arguments are passed on, and the
  context value is `null` (`$string` called with no argument gives `"null"`). A lambda keeps the
  context and the variables of its definition.
- **It can be called after the host function returns**, since it holds its environment.
- **Errors reject the promise** with their codes (`D3137` for `$error`, `T2001`). The host
  function can catch them, or let them through, which ends the evaluation.
- **Results are internal values.** A sequence keeps its `sequence` flag, and a function comes
  back as the internal object (`{"_jsonata_function": true, ...}`).
- **There is no limit.** A recursion through a host function
  (`$f := function($n){ $apply($f, $n + 1) }`) runs until the heap is exhausted.

The constraints of ADR 0006 still hold: `HostFunction` and `Bindings` are `Send + Sync`, the
internal `Value` is not public, and the untraced evaluator must not get slower. Functions written
against ADR 0006 must keep working.

## Decision

- **A callable handle, through the call.** `HostCall::function(i) -> Option<Callable<'_>>` gives
  argument `i` (indexed as in the arguments the function receives, after signature validation)
  when it is a function, and `None` otherwise. Arguments still arrive as `None` in the JSON
  slice: nothing changes for existing functions.
- **The boundary stays JSON.** `Callable::call(&self, args: &[Option<JsonValue>]) ->
  Result<Option<JsonValue>>` converts the arguments to values and the result back, with the
  rules of ADR 0006: a sequence is an array or a single value, a function is `None`.
  `Callable::arity()` gives the arity, like the `arity` property in jsonata-js.
- **The handle borrows the call.** It cannot outlive the host function's call, so the evaluator
  it refers to is still running. In Rust the borrow checker enforces this; the WebAssembly
  bindings enforce it at run time (consequences).
- **A call is a callback of the evaluator**, as for `$map`: the same path native functions use.
  It applies the function with the context `null`, counts as a level of depth, checks the
  machine stack, the timeout and the cancellation flag, and evaluates within the same size
  limits. A recursion through a host function ends with `D1011`.
- **Tracing needs no new event.** The callback's events come from the same evaluator, between
  the `enter` and `exit` of the host call's node, so they nest under it.
- **Errors come back as `Err`**, with their codes and positions. When the host function returns
  one, it ends the evaluation like any error from the expression.
- **Internally**, `HostCall` gains a reference to the evaluator, the evaluated arguments and the
  environment of the call, behind a private trait object, so the type keeps its single lifetime.
  The evaluator is borrowed mutably through a `RefCell` during a call. A host function cannot
  reach its own `HostCall` from a nested evaluation (the implementation is `'static`), so the
  borrow cannot fail; if it ever did, the call returns an error instead of panicking.

## Consequences

- Planned differences from jsonata-js, for `compatibility.md`:
  - Results are JSON: no sequence flag and no internal function object reaches the host.
  - A callable cannot be called after the host function returns. In WebAssembly, calling it
    then throws with a new `U` code.
  - In WebAssembly the callable is synchronous: it returns the result, not a promise. Code
    written for jsonata-js with `await f(x)` keeps working; code that calls `.then` does not.
  - A recursion through a host function ends with `D1011` instead of exhausting memory.
- A host function cannot pass a function to a callable, or receive one as its result: they
  cross as `None`. A public value type could lift that later, without changing this API.
- In WebAssembly, a JavaScript host function receives a JavaScript function for a function
  argument, with an `arity` property, where it receives `undefined` today. A function that
  tested for `undefined` sees a different argument: the change is noted in the changelog.
- Each call to a callable converts its arguments and its result, as host calls do. Callbacks
  over large values cost a copy each time.
- The untraced hot path gains nothing beyond passing the evaluator and the environment to the
  host call: no new event, no check outside host calls.
