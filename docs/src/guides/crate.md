# Rust crate

`jlens-core` is the JSONata engine. `jlens-schema` adds schema inference and the language
service; see the [schema guide](schema.md). This chapter covers `jlens-core`. Its API reference
is on [docs.rs](https://docs.rs/jlens-core).

```toml
[dependencies]
jlens-core = "0.2"
serde_json = "1"
```

The snippets in this chapter come from `crates/jlens-core/tests/guide.rs`, which runs them as
tests.

## Compile once, evaluate many times

```rust,ignore
{{#include ../../../crates/jlens-core/tests/guide.rs:quick_start}}
```

[`Expression::compile`](https://docs.rs/jlens-core/latest/jlens_core/struct.Expression.html#method.compile)
parses the expression and prepares it for evaluation. A compiled expression is immutable and
cheap to clone. Keep it, and evaluate it as often as needed, from any thread (see
[Threads](#threads)).

Documents and results are `serde_json::Value`s. Object fields keep their order (jlens enables
`serde_json`'s `preserve_order`). Variables come from
[`Bindings`](https://docs.rs/jlens-core/latest/jlens_core/struct.Bindings.html): `$kind` above.
Bindings can also be collected from `(String, Value)` pairs, such as the fields of a JSON object.

### Undefined

JSONata distinguishes *nothing* (an empty sequence) from `null`. In the API, nothing is `None`,
both for the input and for the result:

```rust,ignore
{{#include ../../../crates/jlens-core/tests/guide.rs:undefined}}
```

## Guardrails

Expressions can recurse and loop. Bound their cost with
[`Options`](https://docs.rs/jlens-core/latest/jlens_core/struct.Options.html), especially for
expressions that users write:

```rust,ignore
{{#include ../../../crates/jlens-core/tests/guide.rs:options}}
```

The default depth limit is 50,000 levels natively; the stack grows as needed. There is no
default timeout. Expressions nested more than 1,000 levels deep (counting each operator of a
chain such as `1 + 2 + 3`) are rejected when compiled (`U1001`). A string longer than `Options::max_string_length` (256 MiB), an array longer
than `Options::max_array_length` (16,777,216 items) or a result larger than
`Options::max_result_size` (256 MiB of JSON text) stops the evaluation with `U1001`
instead of exhausting memory. See [Guardrails](../architecture/evaluator.md#guardrails).

### Cancellation

A [`CancelToken`](https://docs.rs/jlens-core/latest/jlens_core/struct.CancelToken.html) stops an
evaluation from another thread, for example when the request that started it goes away. Clones
of a token share one flag:

```rust,ignore
{{#include ../../../crates/jlens-core/tests/guide.rs:cancel}}
```

The evaluation checks the token between steps, as it checks the timeout, and fails with
`U1003`. A host function that is running finishes first. A cancelled token stays cancelled, so
create one per job.

### Reproducible results

`$now()` and `$millis()` read the system clock once per evaluation, and `$random()` and
`$shuffle()` draw from a generator seeded from it. Fix both in `Options` to get the same result
from the same expression, input and options, for tests or for replaying a decision:

```rust,ignore
{{#include ../../../crates/jlens-core/tests/guide.rs:reproducible}}
```

A seed reproduces results with the same version of jlens, on any platform; it does not
reproduce the numbers jsonata-js draws from `Math.random()`.

## Errors

Every error has a stable [code](../language/errors.md): `S0xxx` for syntax errors, `T` for
type errors, `D` for dynamic errors, and `U1001` for limits. Match on the code, not on the
message:

```rust,ignore
{{#include ../../../crates/jlens-core/tests/guide.rs:errors}}
```

`position` matches jsonata-js: a UTF-16 offset, usually just after the offending token. `span`
is a byte range, for slicing the source in Rust. Errors serialize to JSON with their code,
message and details.

## Host functions

The application can provide its own functions, called from expressions like built-in ones. Bind
a [`HostFunction`](https://docs.rs/jlens-core/latest/jlens_core/struct.HostFunction.html) with
`Bindings::function`:

```rust,ignore
{{#include ../../../crates/jlens-core/tests/guide.rs:host_functions}}
```

- **Arguments and results are JSON.** `None` stands for *undefined*. A function passed as an
  argument (such as a lambda) arrives as `None`.
- **Signatures** use the [notation of the built-in functions](../language/programming.md#signatures).
  `HostFunction::with_signature` checks the arguments before each call, with the same errors
  (`T0410`, `T0412`). A parameter with `-` takes the context value when the argument is missing.
  `HostFunction::new(arity, ...)` skips the check and passes the arguments as given.
- **The arity** tells higher-order functions how many arguments to pass: `$map` passes the index
  only to functions with two parameters or more. Partial application (`$shipping(?, 'north')`)
  uses it too.
- **The context value** is available through `HostCall::input`, which converts it to JSON only
  when called.
- **Errors** returned by the function stop the evaluation. Give them a code of your own and a
  `message` detail. Like other errors, they get the position and token of the call.
- **Bindings take precedence** over built-in functions of the same name, as in jsonata-js. A
  name holds a value or a function: binding one replaces the other.

Host functions are `Send + Sync`, so bindings that hold them can be shared between threads. A
host function runs to completion: a [timeout](#guardrails) is checked again only once it
returns.

## Many expressions, one document

Each `evaluate` call converts the parts of its input document that the expression reads into
jlens's internal representation, one level at a time: a path costs the width of the objects it
crosses. An expression that reads most of a document of a few dozen kilobytes (a filter over
every item, say) spends more on that conversion than on the rule. To evaluate several rules
against the same document, convert it once into a
[`Document`](https://docs.rs/jlens-core/latest/jlens_core/struct.Document.html):

```rust,ignore
{{#include ../../../crates/jlens-core/tests/guide.rs:documents}}
```

`Document::parse` reads JSON text straight into the internal representation, and
`Document::new` converts a `serde_json::Value`. A `Document` is not `Send`: use it on the thread
that created it. [Performance](../architecture/performance.md) has measurements.

## Explaining results

`evaluate_audited` records an audit tree of the evaluation: comparisons and their operands,
`and`/`or` short circuits, conditionals, bindings, filters, and the labels of
[annotations](../language/annotations.md). `explain` summarizes it:

```rust,ignore
{{#include ../../../crates/jlens-core/tests/guide.rs:audit}}
```

The [audit guide](audit.md) describes the tree, the levels and the bounds that keep audits of
large documents small.

### Custom tracers

Audits are built by a tracer, an implementation of the
[`Tracer`](https://docs.rs/jlens-core/latest/jlens_core/audit/trait.Tracer.html) trait that
receives evaluation events. Every method has a default that does nothing, so a tracer
implements only what it needs:

```rust,ignore
{{#include ../../../crates/jlens-core/tests/guide.rs:tracer}}
```

jlens provides three tracers:
- `AuditCollector` builds audit trees;
- `JsonlTracer` writes every event as a line of JSON;
- `TracingTracer`, with the `tracing` feature, emits [`tracing`](https://docs.rs/tracing)
  events.

Without a tracer, the evaluator runs its fast path, so tracing costs nothing when unused.

## Tools for editors

[`parse_recover`](https://docs.rs/jlens-core/latest/jlens_core/fn.parse_recover.html) keeps
going after syntax errors. The syntax tree is public, with node ids, spans and annotations:

```rust,ignore
{{#include ../../../crates/jlens-core/tests/guide.rs:tooling}}
```

The function catalog describes every built-in function, for completion lists and
documentation:

```rust,ignore
{{#include ../../../crates/jlens-core/tests/guide.rs:functions}}
```

For completion, diagnostics and hover against a schema, use `jlens_schema::LanguageService`;
see the [schema guide](schema.md#in-rust).

## Threads

`Expression` is `Send + Sync`. Share one compiled expression between threads, for example
behind an `Arc`:

```rust,ignore
{{#include ../../../crates/jlens-core/tests/guide.rs:threads}}
```

Each evaluation is independent and single-threaded. Values created during an evaluation never
leave it, except as the `serde_json::Value` result.

## Features

| Feature | Effect |
|---|---|
| `tracing` | Adds `audit::TracingTracer`, which reports evaluations as `tracing` spans and events |

The minimum supported Rust version is 1.88. jlens-core compiles to `wasm32-unknown-unknown`;
the [WebAssembly guide](wasm.md) covers the browser package built on it.
