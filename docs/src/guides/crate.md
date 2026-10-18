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
chain such as `1 + 2 + 3`) are rejected when compiled (`U1001`), and so is a value nested more
than 1,000 levels of arrays and objects deep when it is returned, compared or serialized. A
string longer than `Options::max_string_length` (256 MiB), an array longer
than `Options::max_array_length` (16,777,216 items) or a result larger than
`Options::max_result_size` (256 MiB of JSON text) stops the evaluation with `U1001`
instead of exhausting memory. See [Guardrails](../architecture/evaluator.md#guardrails).

A timeout reads the clock every 256 steps (defined below), so it costs almost nothing, and
fires at most 256 steps late; a regular expression reads it at each of its checks. Being a
clock, it lets the same evaluation pass on a fast machine and fail on a slow one. `Options::max_steps` is a budget that does not depend on the machine: each node
evaluated is a step, and so is each function a built-in or a host function calls back, and each
1,024 steps of a regular expression's backtracking. The same expression, input and options take
the same steps everywhere, natively and in WebAssembly, with or without a tracer, and fail with
`U1006` once the budget is spent. The header of an [audit](#explaining-results) reports the
steps an evaluation took (`steps`), to set a budget from. The limit does not bound the work
inside one step, which the size limits bound: a range, `$sort` without a function, `$string`.
For the same outcome on every platform, set `max_depth` too, since its default differs in
WebAssembly. See [ADR 0018](../adr/0018-step-limit.md).

### Cancellation

A [`CancelToken`](https://docs.rs/jlens-core/latest/jlens_core/struct.CancelToken.html) stops an
evaluation from another thread, for example when the request that started it goes away. Clones
of a token share one flag:

```rust,ignore
{{#include ../../../crates/jlens-core/tests/guide.rs:cancel}}
```

The evaluation checks the token at every step and fails with `U1003`. A host function that is running finishes first. A cancelled token stays cancelled, so
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
type errors, `D` for dynamic errors, `U1001` for size limits and `U1006` for the step limit.
Match on the code, not on the message:

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
  argument (such as a lambda) arrives as `None`; `HostCall::function` gives a handle to call it
  ([below](#functions-as-arguments)).
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
returns, or when it calls a function it received.

### Functions as arguments

A host function can call a function it receives, such as a fallback or a step to retry.
`HostCall::function(i)` gives a
[`Callable`](https://docs.rs/jlens-core/latest/jlens_core/struct.Callable.html) for argument `i`
when it is a function: a lambda, a built-in function, a partial application, a chain or another
host function.

```rust,ignore
{{#include ../../../crates/jlens-core/tests/guide.rs:host_callbacks}}
```

- **The call is part of the evaluation**, as when `$map` calls its function. The context value
  is `null`, and a lambda keeps the variables of its definition. The call counts as a level of
  depth, so a recursion through a host function ends with `D1011`; it checks the timeout and the
  cancellation token; its result is limited by `max_result_size`. An audit shows its steps under
  the host function's call.
- **Arguments and the result are JSON**, as for the host function itself. A sequence comes back
  as an array or a single value; a function crosses as `None`, in both directions.
- **Errors come back as `Err`**, with their codes and positions. The host function can handle
  them, or return them, which ends the evaluation.
- **The handle borrows the call**: it cannot be kept and called after the host function returns.

## Many expressions, one document

Each `evaluate` call converts the parts of its input document that the expression reads into
jlens's internal representation, one level at a time: a path step reads its one field of an
object, other operations the object's whole level. An expression that reads most of a document of a few dozen kilobytes (a filter over
every item, say) spends more on that conversion than on the rule. To evaluate several rules
against the same document, convert it once into a
[`Document`](https://docs.rs/jlens-core/latest/jlens_core/struct.Document.html):

```rust,ignore
{{#include ../../../crates/jlens-core/tests/guide.rs:documents}}
```

`Document::parse` reads JSON text straight into the internal representation, and
`Document::new` converts a `serde_json::Value`. A `Document` is not `Send`: use it on the thread
that created it. [Performance](../architecture/performance.md) has measurements.

### Documents built from Rust values

An application that keeps its state in Rust types and evaluates a few expressions each time it
changes needs no `serde_json::Value` in between: `Document::from_serialize` serializes any
`Serialize` value straight into a document
([ADR 0019](../adr/0019-documents-from-rust-values.md)). It builds the document that
`Document::new(&serde_json::to_value(&state)?)` builds, in less than half the time, and fails
where `to_value` fails (a map key that is not a string, number or boolean, say).

```rust,ignore
{{#include ../../../crates/jlens-core/tests/guide.rs:from_serialize}}
```

An application that evaluates only once or twice per state can skip the document: a
[`Source`](#other-input-representations) written over its types reads only the fields an
expression reaches, and `Expression::evaluate_source` evaluates against it. For ten records of
twenty fields, one expression takes 3.7 µs that way, against 19 µs for building the document and
evaluating; by four expressions the two are close, and by ten the document is faster
([Performance](../architecture/performance.md#documents-built-from-rust-values)).

## Other input representations

An input held in another representation than `serde_json` (another JSON library, a database
row, a host language's objects) can be read the same way, one level at a time, without being
converted first. Implement
[`input::Source`](https://docs.rs/jlens-core/latest/jlens_core/input/trait.Source.html) for
it, and evaluate with `Expression::evaluate_source`; implement its `read_field` too when the
representation looks up one field for less than it reads a whole object. The documentation of the
[`input`](https://docs.rs/jlens-core/latest/jlens_core/input/index.html) module has an
example. A level that the source fails to read stops the evaluation with `U1004`. The
WebAssembly package reads JavaScript objects this way.

A result can be written into another representation the same way, without building a
`serde_json::Value` first ([ADR 0017](../adr/0017-result-builder.md)). Implement
[`output::Builder`](https://docs.rs/jlens-core/latest/jlens_core/output/trait.Builder.html), and
evaluate with `Expression::evaluate_source_to` or `evaluate_document_to`: they give the result to
a closure, which writes it with the builder, within the size limit of a result. The
documentation of the
[`output`](https://docs.rs/jlens-core/latest/jlens_core/output/index.html) module has an
example.

## The fields an expression reads

`Expression::reads` tells which fields of its input and of each binding an expression may read,
without evaluating it ([ADR 0016](../adr/0016-fields-read.md)): an application can fetch only
those from a database, or know which rules depend on a field. The answer is a
[`Reads`](https://docs.rs/jlens-core/latest/jlens_core/reads/struct.Reads.html), a tree of field
names for the input and for each binding; `union` combines the answers of several expressions.

```rust,ignore
{{#include ../../../crates/jlens-core/tests/guide.rs:reads}}
```

- **The answer is an upper bound.** On its input and bindings cut by its reads (`Read::cut`), an
  expression gives the same result, or fails with the same error code. Where the analysis cannot
  follow a value, under `*` or `**`, in a constructed object or in the result, it reads the value
  whole (`true` in the JSON form).
- **A host function can read the context of its call** through `HostCall::input`, so a call to
  one reads that context whole: the whole input, at the top of an expression. Mark a function
  that does not with `HostFunction::without_input`, and analyse with `Expression::reads_with`,
  given the bindings: a call to it then reads only its arguments, and `HostCall::input` gives
  `None`. A parameter with `-` in its signature still reads the context it may take.
- **`Document::parse_cut`** parses JSON text cut by a `Read`, skipping the fields it leaves
  out: given the union of several rules' reads, one document serves them all, and the parse
  takes less time the less the rules read ([Performance](../architecture/performance.md#reusing-documents)).
- **A binding named after a built-in function** shadows it during the evaluation, but the
  analysis takes the name as the built-in's: such an application cannot cut its input by it.

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
