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
cargo xtask ab main .                         # two revisions against each other, native
cargo xtask wasm && cd benches/compare && npm install && node compare.mjs   # against jsonata-js
```

### Comparing two revisions

To measure a change, compare the revision before it with the revision that makes it. Criterion
alone can mislead here: on some machines its results are bimodal, so two runs of one build can
differ by more than the change. `cargo xtask ab` is built for this:

```sh
cargo xtask ab main .                         # main against the working tree
cargo xtask ab v0.9.0 HEAD --case regex --case strings --rounds 20
cargo xtask ab main . --mode document         # evaluate_document instead of evaluate
```

- It builds one program per revision, against that revision's `jlens-core`, with the release
  profile. A revision is anything git names (a commit, a tag, a branch), or `.` for the working
  tree with its uncommitted changes. A commit is built once, in a temporary git worktree, and
  its program is kept in `target/ab/bin`.
- Two runs may overlap: each builds in its own directory, `target/ab/run-<process id>`, and
  they share the programs already built. Their measurements disturb each other, though: two
  runs of nearly the same code at once differed by 8% on the machine below. Compare revisions
  one run at a time.
- It runs the two programs in alternation, ten rounds by default (`--rounds`): A then B, then B
  then A, so that a drift of the machine reaches both. In each run, a case takes the fastest of
  five samples of about 10 ms each (`--sample-ms`).
- It prints, for each case, the median of the rounds for each revision and how much slower B is,
  then the same for the fastest round. Judge by the median. A whole process is sometimes faster
  than usual, by up to 10% on the machine below, and one such run sets the fastest time.

Comparing a revision with itself shows the noise. On the machine below, with ten rounds, the
medians of most cases agree within 1%, and a few cases (`group-by`) within 3%.

The numbers below were measured on an AMD Ryzen 7 3700X under Windows 11, with Rust 1.99 and
Node.js 24, for jlens 0.8.0. They are meant
for comparing alternatives on one machine. They are not guarantees, and your hardware will give
different absolute values.

### Native (criterion)

Each `evaluate` call reads its `serde_json::Value` input as the expression reaches it, one
object or array at a time ([ADR 0010](../adr/0010-lazy-input.md)). `evaluate_document`
evaluates against a [`Document`](#reusing-documents) converted beforehand.

| Case | `evaluate` | `evaluate_document` |
|---|---:|---:|
| path (`customer.name`) | 1.9 µs | 0.9 µs |
| filter + aggregate | 292 µs | 163 µs |
| group-by | 695 µs | 526 µs |
| sort + map | 1.94 ms | 1.76 ms |
| strings | 321 µs | 200 µs |
| regex | 356 µs | 233 µs |
| number formatting | 226 µs | 198 µs |
| transform | 1.09 ms | 962 µs |
| recursion (`$f(15)`, Fibonacci) | 1.03 ms | 1.03 ms |
| shipping rule | 550 µs | 421 µs |

| Other measurements | Time |
|---|---:|
| Compile `customer.name` | 1.1 µs |
| Compile the shipping rule (30 lines) | 39 µs |
| `Document::parse` of the 24 KB document | 171 µs |
| `Document::new` from a `serde_json::Value` | 108 µs |
| `serde_json::from_str` of the same text, for reference | 224 µs |
| Shipping rule, audit level `decisions` | 1.95 ms (3.7× without a tracer) |
| Shipping rule, audit level `full` | 7.56 ms (14× without a tracer) |

The path case reads two objects of the input, the document and its `customer`: `evaluate`
takes 1.9 µs, where converting the whole input first took 115 µs (91 µs to convert, 17 µs to
drop, 0.9 µs to evaluate). An expression that reads every item pays for converting them, as
before.

On this machine, such an expression costs more with lazy input than it did when `evaluate`
converted the whole input first: measured with `cargo xtask ab` against the revision before
lazy input (d0e9789), `filter + aggregate` is 7% slower, `strings` 10% and `regex` 4%. The
conversion is not the cause:
- The whole cost comes from step 2 of [ADR 0010](../adr/0010-lazy-input.md), and it does not depend on reading
  lazily. Converting the whole input eagerly at the current revision costs the same.
- Converting the document alone (`Document::new` and its drop) takes 94 µs at both
  revisions, and `evaluate_document` is as fast as before.
- The cause is Windows' default allocator. With `mimalloc` as the global allocator in both
  programs, the gap falls to 1–3%, which is the cost of the lazy values themselves, and every
  one of these cases runs in about half the time.

An application that calls `evaluate` on whole documents can set a faster global allocator;
the library leaves that choice to it.

### WebAssembly against jsonata-js (Node.js)

Both run in the same Node.js process, on the same input. The WebAssembly module is the release
build optimized by `wasm-opt -Os`. The comparison script first checks
that both produce the same result for each case. The jsonata-js times include the `await`
that its asynchronous `evaluate` requires.

| Case | jsonata-js 2.2.2 | jlens-wasm `evaluate` | jlens-wasm `evaluateDocument` |
|---|---:|---:|---:|
| path | 2.2 µs | 8.1 µs | 2.5 µs |
| filter + aggregate | 668 µs | 494 µs | 169 µs |
| group-by | 1.00 ms | 868 µs | 539 µs |
| sort + map | 4.88 ms | 2.20 ms | 1.86 ms |
| strings | 594 µs | 533 µs | 204 µs |
| regex | 631 µs | 582 µs | 253 µs |
| number formatting | 570 µs | 614 µs | 291 µs |
| transform | 1.35 ms | 1.78 ms | 1.44 ms |
| recursion | 7.83 ms | 1.03 ms | 1.03 ms |
| shipping rule | 1.65 ms | 762 µs | 444 µs |

jsonata-js works on the JavaScript objects it is given, so it never pays for converting the
input. jlens-wasm `evaluate` reads the input object as the expression reaches it: an object or
array of up to 16 values through one call to a JavaScript helper, a larger one whole, through
`JSON.stringify` and a parse into jlens's representation. The path case reads two small
objects. An expression that reads every item pays what converting the whole input does:
`JSON.stringify` of the input takes 46 µs, and parsing that text 273 µs.

`evaluate` is faster than jsonata-js on seven of the ten cases. Against a `Document` converted
once, jlens-wasm is faster on eight, by up to 7.6× on recursion.

It is slower on three cases:
- `path`, which does almost no work, so the overhead of calling into WebAssembly dominates;
- `number formatting`, by 8%, because it reads every item;
- `transform`, whose deep copy of the document is the job V8's own JSON round trip does best.

## Using the API efficiently

- **Compile once.** `Expression::compile` (`new Expression` in JavaScript) parses and
  post-processes. A compiled expression is immutable: keep it, share it between threads (it is
  `Send + Sync`), and evaluate it as often as needed.
- **Reuse documents.** `evaluate` converts the parts of the input that the expression reads.
  For an expression that reads all of a 24 KB document, that costs about 110 µs natively, and
  about 300 µs in WebAssembly. To evaluate several expressions against one document, such as a
  set of rules, or one expression repeatedly, convert the document once.
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
  insertion order, hashed with `foldhash`, which is fast and randomly seeded. Their keys are
  shared strings: converting a document allocates each distinct key once, not once per object. Building results,
  sequences and tuple streams allocates, and that is most of the cost of evaluation.
- **Sequences.** Following jsonata-js, paths build sequences step by step and flatten them.
  Predicates evaluate per item, in a fresh frame.
- **Converting the input.** `evaluate` converts the whole input before it reads any of it, so
  a path into a large document costs as much as the conversion. Converting only what an
  evaluation reads needs a different value model; it is planned for 0.8 (see the
  [roadmap](roadmap.md)).
- **Deep copies.** A transform (`|...|`) copies its input with `$clone` before changing it, as
  jsonata-js does. That makes transforms of large documents comparatively expensive.
