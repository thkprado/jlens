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
1. It compiles the crate with the `wasm-release` profile, which optimizes for size.
2. It generates the JavaScript glue and the TypeScript declarations with `wasm-bindgen --target web`.
3. It runs `wasm-opt -Oz` from [binaryen](https://github.com/WebAssembly/binaryen): the installed one, or the
   npm package through `npx`. Without either, the module is about 60% larger.
4. It writes `package.json`.

The `wasm-bindgen` CLI must be the exact version in `Cargo.lock`, and the task checks this.

| File | Contents |
|---|---|
| `jlens.js` | The ES module |
| `jlens.d.ts` | TypeScript declarations |
| `jlens_bg.wasm` | The WebAssembly module: about 950 KiB, 370 KiB gzipped |
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
  input document, so `$` is undefined. Values pass through `JSON.stringify`. As a result, `Date`
  objects become strings, and functions and `undefined` fields are dropped.
- **`bindings`** is an object of variables: `{limit: 10}` makes `$limit` available.
- **`options`** takes `timeout` (milliseconds; error `D1012`) and `maxDepth` (error `D1011`).
- **The result** is a plain value, or `undefined` when the expression evaluates to undefined.
  Numbers and object key order are the same as in jsonata-js.

`evaluateJson` takes the input as JSON text and returns the result as JSON text. For large
documents that you already have as text, such as a `fetch` response body, it avoids converting
the document to JavaScript objects and back.

### Reusing a document

Each `evaluate` call converts its input. For a document of a few dozen kilobytes that costs
hundreds of microseconds, often more than the evaluation itself. To evaluate several
expressions against one document (a set of rules, say), convert it once:

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
Create a new service when the schema or the variables change. Construction is cheap.

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
| Evaluation depth (`maxDepth`) | 1,000 | `D1011` |
| Evaluation time (`timeout`) | none | `D1012` |
| Expression nesting | 1,000 levels | `U1001` |

The depth limit is lower than on native targets (50,000). WebAssembly runs on the JavaScript
engine's stack, which is about 1 MiB in V8, and deep recursion beyond it raises a JavaScript
`RangeError`. Such an error, like any trap, leaves the module instance unusable. The defaults
keep evaluations inside the engine's stack, with headroom. Raise `maxDepth` only if you have
measured what your target engines allow. The jlens errors themselves (`JlensError`,
`TypeError`) never affect the instance. The
[evaluator chapter](../architecture/evaluator.md#guardrails) has the details.

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
complete page with no build step:
- a JSONata editor (CodeMirror 6) with completion, diagnostics, hover and signature help, driven
  by a schema inferred from the input document;
- live evaluation;
- the explanation and an audit tree viewer; clicking a node selects its source;
- the inferred schema;
- ready-made examples, including the samples of [try.jsonata.org](https://try.jsonata.org).

```sh
cargo xtask wasm
cargo xtask serve        # then open the printed /examples/web-editor/ address
```

CodeMirror is loaded from esm.sh through an import map, so the page needs network access.

## Testing

`cargo xtask wasm-test` builds the package, then runs `crates/jlens-wasm/tests/suite.mjs` twice:
- in Node.js;
- in a headless Chrome, Chromium or Edge (set `JLENS_BROWSER` to choose the executable).

`--require-browser` fails when no browser is found, instead of skipping. `--example` also
checks that the web editor loads and evaluates its sample.
