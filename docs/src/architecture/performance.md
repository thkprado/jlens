# Performance

jlens is a tree-walking interpreter, like jsonata-js. This chapter covers three things: how
performance is measured, what the numbers are, and how to use the API to get the most from it.

## Benchmarks

The workload is in `benches/` at the repository root. It is shared by the native benchmarks and
by the comparison with jsonata-js:
- `benches/cases.json` lists ten expressions: path navigation, filters with aggregation,
  grouping, sorting, string and regex functions, number formatting, a transform, a recursive
  function, and the shipping rule from the guides.
- `benches/orders-200.json` is a 24 KB store order with 200 items.

```sh
cargo bench -p jlens-core                     # criterion, native
cargo xtask wasm && cd benches/compare && npm install && node compare.mjs   # against jsonata-js
```

The numbers below were measured on an AMD Ryzen 7 3700X under Windows 11, with Rust 1.99 and
Node.js 24. They are meant for comparing alternatives on one machine. They are not
guarantees, and your hardware will give different absolute values.

### Native (criterion)

Each `evaluate` call converts its input from `serde_json::Value` first. `evaluate_document`
evaluates against a [`Document`](#reusing-documents) converted beforehand.

| Case | `evaluate` | `evaluate_document` |
|---|---:|---:|
| path (`customer.name`) | 153 µs | 0.9 µs |
| filter + aggregate | 313 µs | 162 µs |
| group-by | 701 µs | 531 µs |
| sort + map | 1.92 ms | 1.77 ms |
| strings | 354 µs | 203 µs |
| regex | 380 µs | 236 µs |
| number formatting | 338 µs | 197 µs |
| transform | 1.14 ms | 989 µs |
| recursion (`$f(15)`, Fibonacci) | 1.14 ms | 1.00 ms |
| shipping rule | 569 µs | 423 µs |

| Other measurements | Time |
|---|---:|
| Compile `customer.name` | 1.1 µs |
| Compile the shipping rule (30 lines) | 41 µs |
| `Document::parse` of the 24 KB document | 214 µs |
| `Document::new` from a `serde_json::Value` | 140 µs |
| `serde_json::from_str` of the same text, for reference | 208 µs |
| Shipping rule, audit level `decisions` | 1.84 ms (3.2× without a tracer) |
| Shipping rule, audit level `full` | 7.85 ms (14× without a tracer) |

### WebAssembly against jsonata-js (Node.js)

Both run in the same Node.js process, on the same input. The comparison script first checks
that both produce the same result for each case. The jsonata-js times include the `await`
that its asynchronous `evaluate` requires.

| Case | jsonata-js 2.2.2 | jlens-wasm `evaluate` | jlens-wasm `evaluateDocument` |
|---|---:|---:|---:|
| path | 2.1 µs | 432 µs | 2.7 µs |
| filter + aggregate | 633 µs | 629 µs | 193 µs |
| group-by | 1.01 ms | 1.26 ms | 824 µs |
| sort + map | 4.94 ms | 2.66 ms | 2.22 ms |
| strings | 591 µs | 705 µs | 266 µs |
| regex | 710 µs | 797 µs | 354 µs |
| number formatting | 575 µs | 766 µs | 402 µs |
| transform | 1.35 ms | 2.52 ms | 1.80 ms |
| recursion | 8.12 ms | 1.66 ms | 1.23 ms |
| shipping rule | 1.67 ms | 965 µs | 525 µs |

jsonata-js works on the JavaScript objects it is given, so it never pays for converting the
input. jlens-wasm `evaluate` has to convert it: `JSON.stringify` on the JavaScript side, then a
parse into jlens's representation. For this document that costs about 430 µs. Against a
`Document` converted once, jlens-wasm is faster than jsonata-js on eight of the ten cases, by up
to 6.6× on recursion.

It is slower on two cases:
- `path`, which does almost no work, so the overhead of calling into WebAssembly dominates;
- `transform`, whose deep copy of the document is the job V8's own JSON round trip does best.

## Using the API efficiently

- **Compile once.** `Expression::compile` (`new Expression` in JavaScript) parses and
  post-processes. A compiled expression is immutable: keep it, share it between threads (it is
  `Send + Sync`), and evaluate it as often as needed.
- **Reuse documents.** Converting the input dominates cheap evaluations: a 24 KB document costs
  about 150 µs natively, and more in WebAssembly. To evaluate several expressions against one
  document, such as a set of rules, or one expression repeatedly, convert the document once.
- **Prefer JSON text when you have it.** `Document::parse` (`evaluateJson` in JavaScript)
  parses straight into jlens's representation. That is faster than parsing into a
  `serde_json::Value` and converting it.
- **Audit at the level you need.** Without a tracer, auditing costs nothing measurable; see
  [ADR 0004](../adr/0004-audit-tracer.md). Level `decisions` records only what explains a
  result. `full` records every node, with its input, and is for debugging.

### Reusing documents

```rust
use jlens_core::{Bindings, Document, Expression, Options};

let doc = Document::parse(r#"{"items": [{"price": 2}, {"price": 3}]}"#)?;
let rules: Vec<Expression> = ["$sum(items.price) > 4", "$count(items) < 10"]
    .iter()
    .map(|source| Expression::compile(source))
    .collect::<Result<_, _>>()?;
for rule in &rules {
    let outcome = rule.evaluate_document(&doc, &Bindings::new(), &Options::default())?;
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

```js
const doc = new Document(input);              // or Document.fromJson(text)
for (const rule of rules) rule.evaluateDocument(doc);
doc.free();
```

Evaluations never modify a document, so one document serves any number of evaluations. A
`Document` is not `Send`. Convert it on the thread that uses it.

## Where the time goes

- **Allocation.** Values are reference-counted (`Rc`). Objects are `IndexMap`s that keep
  insertion order, hashed with `foldhash`, which is fast and randomly seeded. Building results,
  sequences and tuple streams allocates, and that is most of the cost of evaluation.
- **Sequences.** Following jsonata-js, paths build sequences step by step and flatten them.
  Predicates evaluate per item, in a fresh frame.
- **Deep copies.** A transform (`|...|`) copies its input with `$clone` before changing it, as
  jsonata-js does. That makes transforms of large documents comparatively expensive.
