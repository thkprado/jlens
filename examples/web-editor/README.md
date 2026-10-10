# jlens web editor

A JSONata editor for the browser, built on the jlens WebAssembly package and CodeMirror 6.
It needs no build step. It shows:
- completion of fields, variables and functions, with the fields taken from a schema inferred
  from the input document;
- diagnostics: syntax errors, and fields that are not in the schema (with "did you mean");
- hover and signature help;
- live evaluation, with the explanation and the audit tree of every result (click a node to
  select its source);
- ready-made examples, chosen in the header: a free-shipping rule for a store order, and the
  samples of [try.jsonata.org](https://try.jsonata.org) (Invoice, Address, Schema, Library,
  Bindings). The choice is kept in the address (`#invoice`), so an example can be linked.

## Running it

From the repository root:

```sh
cargo xtask wasm     # builds crates/jlens-wasm/pkg
cargo xtask serve    # serves the repository; open the printed /examples/web-editor/ address
```

The page imports the package from `../../crates/jlens-wasm/pkg/`. It loads CodeMirror from
esm.sh through an import map, so it needs network access.

## Files

| File | Contents |
|---|---|
| `jlens-codemirror.js` | The CodeMirror integration: highlighting, completion, linting, hover, signature help. Copy it into your application |
| `audit-view.js` | Renders explanations and audit trees |
| `main.js` | Wires the editor, the input document, the examples and the result panels |
| `samples.js` | The examples: expression, input document and bindings of each |
| `index.html`, `style.css` | The page |

The [WebAssembly guide](../../docs/src/guides/wasm.md) documents the API used here.

The try.jsonata.org samples come from the
[JSONata Exerciser](https://github.com/jsonata-js/jsonata-exerciser) (`src/sample.js`), © IBM Corp.
2016, 2020, MIT license. Its Bindings sample binds a JavaScript function (`Math.cos`); jlens
bindings are JSON values, so this version binds `$pi` and writes the cosine in JSONata.
