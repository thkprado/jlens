# Coming from jsonata-js

jlens targets the behaviour of jsonata-js 2.2.2. An expression that works there gives the same
result here: jlens is checked against the official test suite on every change, and against
jsonata-js itself with generated expressions every week
([Conformance testing](../architecture/conformance.md)). What changes is the API around the
expression.

This page is for a JavaScript application that calls jsonata-js today.
[Compatibility with jsonata-js](../language/compatibility.md) lists every difference in the
language, with its reason.

## The same evaluation in both

With jsonata-js:

```js
import jsonata from "jsonata";

const expression = jsonata("$double($sum(items[type = $kind].(price * qty)))");
expression.registerFunction("double", (n) => n * 2, "<n:n>");
const result = await expression.evaluate(order, { kind: "a" });
```

With jlens, from the npm package `jlens-wasm`:

```js
import init, { Expression } from "jlens-wasm";

await init(); // once, before anything else

const expression = new Expression("$double($sum(items[type = $kind].(price * qty)))");
expression.registerFunction("double", (n) => n * 2, "<n:n>");
const result = expression.evaluate(order, { kind: "a" });
```

In Node.js and Deno, `init` takes the bytes of the `.wasm` file: see
[Loading the module](../guides/wasm.md#loading-the-module).

## The API, side by side

| jsonata-js | jlens-wasm |
|---|---|
| `jsonata(source)` | `new Expression(source)`, after `await init()` once |
| `await expression.evaluate(input, bindings)` | `expression.evaluate(input, bindings, options)`. It is synchronous and returns the value; an `await` left in front of it still works |
| `expression.evaluate(input, bindings, callback)` | No callback: the call returns the value or throws |
| `expression.assign(name, value)` | No `assign`: pass the variable in the bindings of each evaluation |
| `expression.registerFunction(name, implementation, signature)` | The same |
| `expression.ast()` | `expression.ast()`, a tree of jlens's own: each node has an `id` and a `span` where jsonata-js has a `position` |
| `jsonata(source, { recover: true })` and `expression.errors()` | The diagnostics of the [language service](../guides/wasm.md#the-language-service) |
| An error is a plain object with `code`, `position`, `token` and `value` | An `Error` named `JlensError` with the same fields, the same codes and the same positions, plus the range `start` to `end` |

The [WebAssembly guide](../guides/wasm.md) describes each of them.

## What to check in your application

- **Asynchronous functions.** A host function that returns a promise fails with `U1002`, and
  jlens does not plan to change that
  ([Not planned](../architecture/roadmap.md#not-planned)). Fetch what the expression needs
  before evaluating it, and pass it in the bindings.
- **Input that is not plain JSON.** jlens reads the input as `JSON.stringify` does: a `Date`
  arrives as its ISO 8601 string, so `$type(d)` is `"string"` where jsonata-js gives `"object"`,
  and `undefined` fields and functions are left out
  ([the input in WebAssembly](../language/compatibility.md#the-input-in-webassembly)).
- **Host functions receive copies.** Arguments and results pass through JSON, so a function
  cannot change the values of the evaluation. `this` is `undefined`: a signature with `-`
  passes the context value as an argument
  ([host functions](../language/compatibility.md#host-functions)).
- **Recursion that is too deep** fails with `D1011`, at 500 levels by default, where jsonata-js
  throws a `RangeError`. The options of `evaluate` set this limit and the others
  ([Limits and memory](../guides/wasm.md#limits-and-memory)).
- **Objects with integer-like keys.** JavaScript puts keys such as `"1"` and `"2"` first; jlens
  keeps the order in which an object was built. A result returned as a JavaScript object looks
  the same in both, as JavaScript orders it. The difference shows where the expression reads the
  order of an object it built, as in `$keys({"b": 1, "2": 2})`, which is `["b", "2"]` here and
  `["2", "b"]` in jsonata-js, and in the text `evaluateJson` returns.
- **Code that reads error messages.** Match on `code`: jlens promises the codes, not the text.

## What jlens adds

- **Explanations.** `evaluateWithAudit` returns the result with the reason for it: which
  comparisons passed or failed, with their values
  ([Explaining results](../guides/wasm.md#explaining-results)).
- **Limits for expressions you did not write.** A timeout, a step limit that is the same on
  every machine, size limits, and regular expressions that stop at them
  ([ADR 0009](../adr/0009-bounded-regex.md)). An evaluation ends with a result or a coded error.
- **One document for many evaluations.** A `Document` is converted once and shared by a set of
  rules ([Reusing a document](../guides/wasm.md#reusing-a-document)).
  [Performance](../architecture/performance.md) compares both engines.
- **Checks before running.** The language service completes and diagnoses expressions against
  a schema inferred from sample documents, and the command line checks and tests them
  ([`jlens check`](../guides/cli.md#jlens-check), [`jlens test`](../guides/cli.md#jlens-test)).

## Other platforms

The [Python](../guides/python.md) and [Java](../guides/java.md) packages have the API above in
their language's style, and the [Rust crate](../guides/crate.md) is the engine itself. An
expression moves between them, and to and from jsonata-js, unchanged.
