# jlens-wasm

[JSONata](https://jsonata.org/) for the browser, compiled from Rust to WebAssembly. It
evaluates expressions, **explains** how a result was reached, and gives editors completion,
diagnostics and hover driven by a JSON Schema inferred from sample documents.

This is an ES module for browsers (`--target web`). It also runs in Node.js 20+ and Deno if you
pass the `.wasm` bytes to `init`.

```js
import init, { Expression } from "jlens-wasm";

await init(); // fetches jlens_bg.wasm next to jlens.js

const expr = new Expression("$sum(items[type = $kind].(price * qty))");
expr.evaluate({ items: [{ type: "a", price: 2.5, qty: 4 }] }, { kind: "a" }); // 10
```

## API

| Export | Purpose |
|---|---|
| `init(options?)` | Loads the WebAssembly module; call (and await) it once before anything else |
| `new Expression(source)` | Compiles an expression; throws a `JlensError` on syntax errors |
| `expr.evaluate(input?, bindings?, options?)` | Evaluates against a JSON-compatible value; `undefined` when the result is undefined |
| `expr.evaluateJson(json?, bindings?, options?)` | The same with JSON text in and out (faster for large documents) |
| `new Document(input)`, `Document.fromJson(text)` | A document converted once, for `expr.evaluateDocument(doc, bindings?, options?)` |
| `expr.evaluateWithAudit(input?, bindings?, options?)` | Result, audit tree and explanation; errors are reported, not thrown |
| `expr.ast()` | The processed syntax tree |
| `parseRecover(source)` | Partial syntax tree and every diagnostic, without throwing |
| `inferSchema(samples, overrides?)` | A JSON Schema (2020-12) inferred from sample documents |
| `new LanguageService(schema?, variables?)` | `complete`, `diagnostics`, `hover` and `signatureHelp` for an editor |
| `functions()` | The built-in function catalog |
| `version()` | The jlens version |

`options` takes `timeout` (milliseconds) and `maxDepth`; for `evaluateWithAudit` also `level`
(`"off"`, `"decisions"`, `"full"`) and `limits`. Errors are `Error` objects named `JlensError`
with the JSONata error `code`, plus `position`, `start`, `end` and `token` when known.

All offsets, in both directions, are UTF-16 code units, like JavaScript string indexes.

The full guide is the "WebAssembly (browser)" chapter of the jlens book (`docs/src/guides/wasm.md`).

## License

MIT. The JSONata test suite and the documentation adapted from JSONata are © IBM Corp. and
JSONata contributors, also under the MIT license.
