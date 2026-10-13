# jlens web editor

A JSONata editor for the browser, built on the jlens WebAssembly package and CodeMirror 6.
It needs no build step, and runs online at <https://thkprado.github.io/jlens/editor/>. It shows:
- completion of fields, variables and functions, with the fields taken from a schema inferred
  from the input document;
- diagnostics: syntax errors, and fields that are not in the schema (with "did you mean");
- hover and signature help;
- live evaluation, with the explanation and the audit tree of every result (click a node to
  select its source). It runs in a Web Worker, so typing never waits for it: when the expression
  changes during an evaluation, the worker is terminated and replaced;
- ready-made examples, chosen in the header: a free-shipping rule for a store order, the
  samples of [try.jsonata.org](https://try.jsonata.org) (Invoice, Address, Schema, Library,
  Bindings), and last a host function that calls the function it receives (`#host-functions`,
  see below). The choice is kept in the address (`#invoice`), so an example can be linked;
- shared addresses: the Share button puts the expression, the input document and the bindings
  in the address and copies it (see below).

Everything runs in the browser: no expression or document leaves it.

## Host functions

The bindings are JSON, so an expression cannot bring JavaScript of its own. The editor offers
one host function to every expression instead, as an application would with
`registerFunction`: `$try(step, fallback)`, in `host-functions.js`. It calls `step` and, when
that fails, gives `fallback`, or calls it with the error (`{code, message}`) when it is a
function. JSONata has no way to catch an error, so this is something only the host can add. It
lets the guardrails through (`D1011`, `D1012` and the `U` errors), so that a catch cannot keep
a runaway evaluation alive.

The worker registers it on each expression, and the page declares it to the language service,
for completion, hover and signature help. The Host functions example uses it to price order
lines whose prices may not be numbers.

## Shared addresses

A shared address holds its state in the fragment, which browsers never send to the server:
`#s=` followed by the JSON text `{"expression": ..., "input": ..., "bindings": ...}`,
compressed with `deflate-raw` (`CompressionStream`) and written in base64url without padding.
`share.js` encodes and decodes it; `share.test.mjs` tests the round trip in Node.js
(`cargo xtask wasm-test --example` runs it, and loads the editor from a shared address).

Some tools (chat applications, mail clients, proxies) cut addresses beyond about 8 KB, so the
Share button refuses a longer one and says so, rather than giving a link that would arrive
broken. Opening a shared address expands at most 8 MB of state.

## Running it

From the repository root:

```sh
cargo xtask wasm     # builds crates/jlens-wasm/pkg
cargo xtask serve    # serves the repository; open the printed /examples/web-editor/ address
```

The page imports the package by its name, `jlens-wasm`, which the import map of `index.html`
resolves to `../../crates/jlens-wasm/pkg/jlens.js`. The same import map loads CodeMirror from
esm.sh, so the page needs network access.

## The site

The editor is published with the book. To preview the site:

```sh
cargo xtask site          # builds the package and the book into target/site
cargo xtask serve --site  # serves target/site; the editor is under /editor/
```

`cargo xtask site` copies this directory to `target/site/editor/`, without this README, and the
package to `editor/pkg/`, and rewrites the `jlens-wasm` entry of the import map to
`./pkg/jlens.js`; nothing else changes. `cargo xtask wasm-test --example` loads both versions
of the editor.

## Files

| File | Contents |
|---|---|
| `jlens-codemirror.js` | The CodeMirror integration: highlighting, completion, linting, hover, signature help. Copy it into your application |
| `audit-view.js` | Renders explanations and audit trees |
| `main.js` | Wires the editor, the input document, the examples and the result panels |
| `worker.js` | Evaluates in a Web Worker; the page passes it the address of the package, as import maps do not apply to workers |
| `samples.js` | The examples: expression, input document and bindings of each |
| `host-functions.js` | The host function `$try`, and its declaration for the language service |
| `share.js`, `share.test.mjs` | Shared addresses, and the test of their round trip (left out of the site) |
| `index.html`, `style.css` | The page |

The [WebAssembly guide](../../docs/src/guides/wasm.md) documents the API used here.

The try.jsonata.org samples come from the
[JSONata Exerciser](https://github.com/jsonata-js/jsonata-exerciser) (`src/sample.js`), MIT license.
Its Bindings sample binds a JavaScript function (`Math.cos`); jlens
bindings are JSON values, so this version binds `$pi` and writes the cosine in JSONata.
