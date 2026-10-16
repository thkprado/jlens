# WebAssembly (browser)

`jlens-wasm` compiles jlens to WebAssembly as an **ES module for browsers**. It covers what the
Rust crates do:
- evaluation;
- audit trees and explanations;
- schema inference;
- the language service (completion, diagnostics, hover, signature help) for editors.

It also runs in Node.js 20+ and Deno, provided `init` is given the `.wasm` bytes.

```js
import init, { Expression } from "jlens-wasm";

await init();

const expr = new Expression("$sum(items[type = $kind].(price * qty))");
expr.evaluate({ items: [{ type: "a", price: 2.5, qty: 4 }] }, { kind: "a" }); // 10
```

## Building the package

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version <the wasm-bindgen version in Cargo.lock> --locked
cargo xtask wasm            # release build into crates/jlens-wasm/pkg
cargo xtask wasm --dev      # unoptimized, with Rust panics reported on the console
```

`cargo xtask wasm` does four things:
1. It compiles the crate with the `wasm-release` profile, which optimizes for size
   and speed (`opt-level = "s"`).
2. It generates the JavaScript glue and the TypeScript declarations with `wasm-bindgen --target web`.
3. It runs `wasm-opt -Os` from [binaryen](https://github.com/WebAssembly/binaryen): the installed one, or the
   npm package through `npx`. Without either, the module is about 45% larger.
4. It writes `package.json`.

The `wasm-bindgen` CLI must be the exact version in `Cargo.lock`, and the task checks this.

| File | Contents |
|---|---|
| `jlens.js` | The ES module |
| `jlens.d.ts` | TypeScript declarations |
| `jlens_bg.wasm` | The WebAssembly module: about 1.2 MiB, 460 KiB gzipped |
| `package.json` | npm metadata: `"type": "module"`, `exports`, `types` |

A size budget for the gzipped module is enforced in `xtask/src/wasm.rs`.

## Loading the module

Call `init()` once, and await it, before using anything else. By default it fetches
`jlens_bg.wasm` from next to `jlens.js`:

```html
<script type="module">
  import init, { Expression } from "./pkg/jlens.js";
  await init();
</script>
```

Bundlers that understand `new URL(..., import.meta.url)` handle this as is. Vite, webpack 5,
Rollup and esbuild all do (esbuild with a loader for `.wasm`). Otherwise, pass the module's URL
or its bytes:

```js
// Vite: get the URL of the asset.
import wasmUrl from "jlens-wasm/jlens_bg.wasm?url";
await init({ module_or_path: wasmUrl });

// Node.js or Deno: pass the bytes.
import { readFile } from "node:fs/promises";
await init({ module_or_path: await readFile(new URL("./pkg/jlens_bg.wasm", import.meta.url)) });
```

`initSync({ module })` instantiates synchronously from bytes or a `WebAssembly.Module`. This is
useful in workers.

## Evaluating expressions

```js
const expr = new Expression(source);         // compile once (throws JlensError on syntax errors)
expr.evaluate(input, bindings, options);     // evaluate many times
expr.evaluateJson(jsonText, bindings);       // JSON text in, JSON text out
expr.free();                                 // release it (or let the garbage collector do so)
```

- **`input`** can be any JSON-compatible value. `undefined`, or no argument, means there is no
  input document, so `$` is undefined. Values are read as `JSON.stringify` reads them. As a
  result, `Date` objects become strings, and functions and `undefined` fields are dropped. See
  [how the input is read](#how-the-input-is-read).
- **`bindings`** is an object of variables and functions: `{limit: 10}` makes `$limit`
  available, and a function becomes a [host function](#host-functions).
- **`options`** sets the [limits](#limits-and-memory) (`timeout`, `maxDepth`,
  `maxStringLength`, `maxArrayLength`, `maxResultSize`), and the clock and seed that make an
  evaluation reproducible: `now` fixes `$now()` and `$millis()` (milliseconds since the epoch,
  a `Date` or an ISO 8601 timestamp), and `seed` fixes `$random()` and `$shuffle()`.
- **The result** is a plain value, or `undefined` when the expression evaluates to undefined.
  Numbers and object key order are the same as in jsonata-js.

`evaluateJson` takes the input as JSON text and returns the result as JSON text. For large
documents that you already have as text, such as a `fetch` response body, it avoids converting
the document to JavaScript objects and back.

### How the input is read

`evaluate` reads its input during the evaluation, one object or array at a time, when the
expression first reaches it ([ADR 0010](../adr/0010-lazy-input.md)). A path such as
`customer.name` reads two small objects, however large the rest of the document is. An object or
array of more than 16 values is read whole, with everything it holds, through `JSON.stringify`,
which is faster than reading its values one by one.

Two consequences follow:

- **Errors appear when their part is read.** A cycle, a `BigInt`, or a `toJSON` method that
  throws makes `JSON.stringify` throw. `evaluate` throws the same exception, but only when the
  expression reads the object or array that holds it: an expression that never reaches it
  succeeds.
- **The input must not change during the evaluation.** A [host function](#host-functions) that
  modifies the input object can change what the rest of the evaluation reads. Copy the input
  first if a function must modify it.

`evaluateJson`, `evaluateWithAudit` and `Document` convert their whole input first.

### Reusing a document

`evaluate` reads the parts of the input that the expression reaches, on every call. For an
expression that reads most of a document of a few dozen kilobytes, that costs hundreds of
microseconds, often more than the evaluation itself. To evaluate several expressions against
one document (a set of rules, say), convert it once:

```js
import { Document } from "jlens-wasm";

const doc = new Document(input);              // or Document.fromJson(text)
const results = rules.map((rule) => rule.evaluateDocument(doc, bindings));
doc.free();
```

Evaluations never modify a document. [Performance](../architecture/performance.md) has
measurements.

With explicit resource management, `using expr = new Expression(...)` frees the expression at
the end of the block.

### Host functions

JavaScript functions, in the bindings or registered on an expression, are called from the
expression like built-in functions:

```js
const expr = new Expression("$shipping($sum(items.weight), region)");
expr.evaluate(order, { shipping: (weight, region) => weight * rates[region] });

// As in jsonata-js: available to every evaluation, with an optional signature.
expr.registerFunction("shipping", (weight, region) => weight * rates[region], "<ns:n>");
```

- **Arguments and the result** pass through JSON, like the input. `undefined` stands for
  undefined, and a function argument arrives as a JavaScript function that calls it
  ([below](#functions-as-arguments)).
- **A signature** (`registerFunction`'s third argument) checks the arguments before each call,
  with the errors of the built-in functions (`T0410`, `T0412`). A parameter with `-` receives the
  context value when the argument is missing: `<n-:n>` makes `price.$double()` work.
- **`this` is `undefined`.** Use a signature with `-` to receive the context value.
- **The function's `length`** tells higher-order functions how many arguments to pass: `$map`
  passes the index only to functions with two parameters or more.
- **An exception** thrown by the function is rethrown by `evaluate` as is. Its `position` and
  `token` are set to the call when it has none. `evaluateWithAudit` reports it with its `code`,
  or its `name`, and its `message`.
- **Asynchronous functions are not supported.** A function that returns a promise fails with
  `U1002`. Fetch what the expression needs before evaluating it.
- **Precedence:** a function in the bindings replaces a registered function of the same name,
  and both replace a built-in function.

The [Rust crate guide](crate.md#host-functions) describes the same model.

### Functions as arguments

A host function can call a JSONata function it receives, such as a fallback or a step to retry:
a lambda, a built-in function, a partial application, a chain or another host function.

```js
const retry = (step, times) => {
  for (let i = 1; ; i++) {
    try {
      return step(i);
    } catch (e) {
      if (i >= times) throw e;
    }
  }
};
new Expression("$retry(function($i) { $lookup(sku, $i) }, 3)").evaluate(order, { retry, lookup });
```

- **The call is part of the evaluation**, as when `$map` calls its function. The context value
  is `null`, and a lambda keeps the variables of its definition. The call counts as a level of
  depth, so a recursion through a host function ends with `D1011`; it checks the timeout; its
  result is limited by `maxResultSize`.
- **It is synchronous**: it returns the result, where jsonata-js returns a promise. Code written
  for jsonata-js with `await step(i)` keeps working.
- **Arguments and the result pass through JSON.** A sequence comes back as an array or a single
  value; a JavaScript function among the arguments becomes `undefined`, and a function result
  comes back as `undefined`.
- **Its `length` is 0** and its `arity` property is the number of parameters, as in jsonata-js.
- **Errors are thrown** as a `JlensError` with their codes, or as the exception a host function
  threw. The host function can catch them, or let them through, which ends the evaluation with
  that error.
- **It is valid until the host function returns.** Called later, from a promise or a timer, it
  throws `U1005`.

## Errors

Errors are `Error` objects named `JlensError`. They carry the JSONata error fields; every offset is
in UTF-16 code units, like JavaScript string indexes:

```js
try {
  new Expression("Account.Order[0;]");
} catch (err) {
  if (err.name !== "JlensError") throw err;
  err.code;      // "S0202"
  err.message;   // 'Expected "]", got ";"'
  err.position;  // 16: where it was detected, as in jsonata-js
  err.start;     // 15: the offending source range
  err.end;       // 16
  err.token;     // ";"
}
```

Evaluation errors carry `value` too, when there is one (for `T2001`, the value that is not a
number). Invalid arguments, such as bindings that are not an object or malformed JSON for
`evaluateJson`, throw a `TypeError` instead. The [error reference](../language/errors.md) lists
every code.

## The fields an expression reads

`reads()` tells which fields of its input and of each binding an expression may read, without
evaluating it ([ADR 0016](../adr/0016-fields-read.md)): an application can fetch only those from
a database, or know which rules depend on a field. `unionReads` combines the answers of several
expressions.

```js
const expr = new Expression("$sum(items[type = $kind].(price * quantity)) * $rate(country)");
expr.reads({ rate: (country) => rates[country] });
// {input: {country: true, items: {price: true, quantity: true, type: true}},
//  bindings: {kind: true, rate: true}}

unionReads([expr.reads({ rate }), new Expression("customer.name").reads()]);
// {input: {country: true, customer: {name: true}, items: {...}}, bindings: {...}}
```

- **The answer is an upper bound.** `true` reads the value whole; an object reads only the
  fields it lists, of an object, or of each item of an array; an empty object needs the value
  but none of its fields, as `$count` does. On its input and bindings cut down that way, an
  expression gives the same result, or fails with the same error code. Where the analysis cannot
  follow a value, under `*` or `**`, in a constructed object or in the result, it reads the
  value whole.
- **Pass the bindings** the expression will be evaluated with. A JavaScript function cannot
  read the context of its call (`this` is `undefined`), so a call to one in the bindings, or
  registered on the expression, reads only its arguments, and the context a `-` parameter may
  take. A call to a function the analysis does not know may read its context: the whole input,
  at the top of an expression.
- **`everyBinding: true`** says the expression may read any binding whole, as `$eval` can: keep
  them all.
- **A binding named after a built-in function** shadows it during the evaluation, but the
  analysis takes the name as the built-in's: such an application cannot cut its input by it.

## Explaining results

`evaluateWithAudit` evaluates and records how the result was reached; see the
[audit guide](audit.md). Evaluation errors are reported in the result, not thrown:

```js
const expr = new Expression('/** @label "Big order" */ $sum(items.(price * qty)) > $limit');
const { result, error, audit, explanation, explanationText } = expr.evaluateWithAudit(
  { items: [{ price: 2.5, qty: 4 }, { price: 1, qty: 3 }] },
  { limit: 20 },
  { level: "decisions", limits: { maxNodes: 1000 } },
);
result;           // false
explanationText;  // 'result: false\n✘ Big order — $sum(items.(price * qty)) > $limit → false (13 > 20)'
audit.root;       // { kind: "binary", op: ">", label: "Big order", operands: [13, 20], outcome: "false", ... }
```

The options are those of `evaluate`, plus two more:
- `level` is `"off"`, `"decisions"` (the default) or `"full"`.
- `limits` sets the capture limits: `maxItems`, `maxString`, `maxDepth`, `maxRejected`,
  `maxNodes` and `maxSource`.

`explanation.lines` holds structured lines (`depth`, `passed`, `text`, `message`, `node`) for
rendering in a user interface. Audit node spans are UTF-16 offsets, so a viewer can highlight
a node's source in the editor. The [web editor](#the-web-editor-example) example does this.

## Tooling

`parseRecover(source)` never throws. It returns the partial syntax tree and every diagnostic:

```js
parseRecover("Account.Order[0;].");
// { ast: {...}, diagnostics: [{ severity: "error", code: "S0202", message: ..., span: {start: 15, end: 16} }] }
```

`expr.ast()` returns the processed tree of a compiled expression, and `functions()` returns the
catalog of built-in functions. Spans are UTF-16 offsets throughout.

### Schemas

`inferSchema(samples, overrides?)` infers a JSON Schema 2020-12 from sample documents. The
[schema guide](schema.md) describes what it records and the `overrides` format:

```js
const schema = inferSchema([doc1, doc2], {
  "items[].type": { description: "Product category", enum: ["apparel", "electronic", "toy"] },
  "items[].internalCode": { hidden: true },
});
```

The result is an ordinary JSON Schema object. Store it, edit it by hand, or pass it to
`LanguageService`.

### The language service

```js
const service = new LanguageService(schema, { limit: { type: "number" } });

const text = "items[type = 'a'].p";
service.complete(text, text.length);
// { range: {start: 18, end: 19},
//   items: [{ label: "price", kind: "field", detail: "integer", documentation: "Examples: `1`", insertText: "price" }] }

service.diagnostics("customer.Login");
// [{ severity: "warning", code: "W0201", span: {start: 9, end: 14},
//    message: 'Field "Login" is not in the schema; did you mean `login`?' }]

service.hover("customer.login", 10);
// { range: {start: 9, end: 14}, contents: '`login`: string\n\nExamples: `"1"`' }

service.signatureHelp("$substring(name, ", 17);
// { name: "$substring", signature: "<s-nn?:s>", params: ["$str", "$start", "$length"], activeParam: 1, ... }
```

`schema` is optional: without one, functions and variables are still completed. The second
argument declares the variables the expressions are evaluated with. Its keys are variable
names, with or without `$`, and its values are schemas, or `null` when the shape is unknown.
The third argument declares host functions, as `{name, signature, params, summary}` objects.
They are completed first, and hover and signature help show them:

```js
new LanguageService(schema, null, [
  { name: "shipping", signature: "<ns:n>", params: ["weight", "region"], summary: "Shipping cost" },
]);
```

Create a new service when the schema, the variables or the functions change. Construction is
cheap.

Completion items are ranked best first, and `insertText` replaces `range`. Field names that are
not plain identifiers come quoted with backquotes. `documentation` and hover `contents` are
Markdown.

#### CodeMirror 6

CodeMirror positions are UTF-16 offsets, like jlens offsets, so no conversion is needed.
`examples/web-editor/jlens-codemirror.js` in the repository
is a self-contained module to copy. It provides highlighting, completion, linting, hover and
signature help. The core of it is:

```js
autocompletion({
  override: [(context) => {
    const { range, items } = service.complete(context.state.doc.toString(), context.pos);
    return {
      from: range.start, to: range.end, filter: false,
      options: items.map((i) => ({ label: i.label, detail: i.detail, apply: i.insertText })),
    };
  }],
}),
linter((view) =>
  service.diagnostics(view.state.doc.toString()).map((d) => ({
    from: d.span.start, to: d.span.end, severity: d.severity, message: d.message,
  })),
),
```

#### Monaco

Monaco works with line and column positions; convert with the model:

```js
monaco.languages.registerCompletionItemProvider("jsonata", {
  triggerCharacters: [".", "[", "$"],
  provideCompletionItems(model, position) {
    const { range, items } = service.complete(model.getValue(), model.getOffsetAt(position));
    const start = model.getPositionAt(range.start);
    const end = model.getPositionAt(range.end);
    return {
      suggestions: items.map((i) => ({
        label: i.label,
        insertText: i.insertText,
        detail: i.detail,
        documentation: i.documentation && { value: i.documentation },
        kind: monaco.languages.CompletionItemKind.Field, // map i.kind as needed
        range: new monaco.Range(start.lineNumber, start.column, end.lineNumber, end.column),
      })),
    };
  },
});
// Diagnostics: monaco.editor.setModelMarkers(model, "jlens", ...) with the same conversion.
```

## Limits and memory

| Limit | Default | Error |
|---|---|---|
| Evaluation depth (`maxDepth`) | 500 | `D1011` |
| Evaluation time (`timeout`, milliseconds) | none | `D1012` |
| Length of a string built (`maxStringLength`, bytes of UTF-8) | 256 MiB | `U1001` |
| Items of an array built (`maxArrayLength`) | 16,777,216 | `U1001` |
| Size of the result as JSON text (`maxResultSize`, bytes) | 256 MiB | `U1001` |
| Expression nesting | 1,000 levels | `U1001` |
| Nesting of arrays and objects in a value that is returned, compared or serialized | 1,000 levels | `U1001` |

The size of the result counts a value each time it appears, as the JSON text does, and also
limits the arguments passed to a host function. Wasm evaluations are synchronous, so there is
no cancellation: use `timeout`. In a browser, an evaluation can also run in a Web Worker, and
terminating the worker stops it; the web editor does this.

The depth limit is lower than on native targets (50,000). WebAssembly runs on the JavaScript
engine's stack, which is about 1 MiB in V8, and deep recursion beyond it raises a JavaScript
`RangeError`. Such an error, like any trap, leaves the module instance unusable. The defaults
keep evaluations inside the engine's stack, with headroom. Raise `maxDepth` only if you have
measured what your target engines allow. The jlens errors themselves (`JlensError`,
`TypeError`) never affect the instance.

How deep an evaluation fits depends on the shape of the recursion and on V8's compiler. V8
first runs the module with its baseline compiler (Liftoff), then recompiles the functions an
evaluation uses often with its optimizing compiler (TurboFan), whose frames for jlens's
evaluator are about 1.6 times larger. Measured in Node.js 24 once the module is optimized, an
audited recursion overflows V8's stack near these depths (an unaudited one fits about 10% more):

| The recursive function's body | Overflows near |
|---|---|
| `[$n, $n]^($f($n - 1))` (order-by) | 670 |
| `{'a': $f($n - 1)}`, `[$n]{'k': $f($n - 1)}` (group) | 710 |
| `{'a': 1} ~> \|$\|{'b': $f($n - 1)}\|` (transform) | 720 |
| `[$n][$f($n - 1)]` (filter) | 750 |
| `$f($n - 1) + 1`, `[$f($n - 1)]` | 840 |
| `$n = 0 ? 0 : 1 + $f($n - 1)` | 920 |

The default of 500 leaves room for shapes not measured here, and for engines with smaller
stacks. The [evaluator chapter](../architecture/evaluator.md#guardrails) has the details.

`Expression`, `Document` and `LanguageService` objects live in WebAssembly memory. The garbage collector
frees them through `FinalizationRegistry`. Call `free()`, or use `using`, to release them
promptly in loops.

## TypeScript

The package ships `jlens.d.ts`. Every argument and result is typed: `AuditResult`, `AuditTree`,
`AuditNode`, `Explanation`, `Diagnostic`, `Completions`, `CompletionItem`, `Hover`,
`SignatureHelp`, `FunctionInfo`, `JsonSchema`, `SchemaOverride`, `EvaluateOptions`,
`AuditOptions` and `JlensError`. Results of `evaluate` are `unknown`; narrow them as usual.

## The web editor example

`examples/web-editor` is a
complete page with no build step, published with this book as the
[web editor](https://thkprado.github.io/jlens/editor/):
- a JSONata editor (CodeMirror 6) with completion, diagnostics, hover and signature help, driven
  by a schema inferred from the input document;
- live evaluation, in a Web Worker (`worker.js`), so that a slow expression never blocks
  typing;
- the explanation and an audit tree viewer; clicking a node selects its source;
- the inferred schema;
- ready-made examples, including the samples of [try.jsonata.org](https://try.jsonata.org);
- a host function, `$try` (`host-functions.js`), that calls the function it receives, with an
  example that uses it: the worker registers it on each expression, and the page declares it to
  the language service.

```sh
cargo xtask wasm
cargo xtask serve        # then open the printed /examples/web-editor/ address
```

CodeMirror is loaded from esm.sh through an import map, so the page needs network access. The
page imports the package by its name, `jlens-wasm`, which the same import map resolves.
`cargo xtask site` builds the published site in `target/site`, the book with the editor in
`editor/`, and `cargo xtask serve --site` serves it.

## Testing

`cargo xtask wasm-test` builds the package, then runs `crates/jlens-wasm/tests/suite.mjs` twice:
- in Node.js;
- in a headless Chrome, Chromium or Edge (set `JLENS_BROWSER` to choose the executable).

`--require-browser` fails when no browser is found, instead of skipping. `--example` also
checks that the web editor loads and evaluates its sample, from the repository and as the site
serves it.
