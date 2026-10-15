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
Node.js 24, for jlens 0.11.0. They are meant
for comparing alternatives on one machine. They are not guarantees, and your hardware will give
different absolute values.

### Native (criterion)

Each `evaluate` call reads its `serde_json::Value` input as the expression reaches it, one
object or array at a time ([ADR 0010](../adr/0010-lazy-input.md)). `evaluate_document`
evaluates against a [`Document`](#reusing-documents) converted beforehand.

| Case | `evaluate` | `evaluate_document` |
|---|---:|---:|
| path (`customer.name`) | 1.7 µs | 0.68 µs |
| filter + aggregate | 154 µs | 44 µs |
| group-by | 490 µs | 348 µs |
| sort + map | 746 µs | 671 µs |
| strings | 208 µs | 93 µs |
| regex | 301 µs | 177 µs |
| number formatting | 212 µs | 190 µs |
| transform | 945 µs | 821 µs |
| recursion (`$f(15)`, Fibonacci) | 1.02 ms | 1.02 ms |
| shipping rule | 238 µs | 128 µs |

| Other measurements | Time |
|---|---:|
| Compile `customer.name` | 1.1 µs |
| Compile the shipping rule (30 lines) | 41 µs |
| `Document::parse` of the 24 KB document | 173 µs |
| `Document::new` from a `serde_json::Value` | 107 µs |
| `serde_json::from_str` of the same text, for reference | 208 µs |
| Shipping rule, audit level `decisions` | 1.82 ms (7.7× without a tracer) |
| Shipping rule, audit level `full` | 8.11 ms (34× without a tracer) |

An audit costs about what it did in 0.8 (1.95 ms and 7.56 ms), but the ratios have doubled:
without a tracer, evaluation takes the shortcuts described under
[Where the time goes](#where-the-time-goes), and with one it takes the general path, so that
the audit reports every node.

The path case reads two objects of the input, the document and its `customer`: `evaluate`
takes 1.7 µs, where converting the whole input first took 115 µs (91 µs to convert, 17 µs to
drop, 0.9 µs to evaluate). An expression that reads every item pays for converting them, as
before.

When measured for 0.8, on this machine, such an expression cost more with lazy input than it did when `evaluate`
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
the library leaves that choice to it. The `jlens` command sets one.

### The allocator of the command

Evaluation allocates for most of its work, so the allocator sets much of its speed. The `jlens`
command uses [mimalloc](https://github.com/microsoft/mimalloc) as its global allocator on every
release target. `cargo xtask ab . . --b-allocator mimalloc` measures the allocator alone: B is
the same revision with `mimalloc`. `--target x86_64-unknown-linux-musl` builds both programs
for musl, as the Linux release binaries are.

Measured for 0.15 (1d1a079) with `evaluate`, the median of ten rounds, on the machine above;
Linux is Ubuntu 26.04 under WSL 2, on the same machine:

| Case | Windows | + mimalloc | Linux, glibc | + mimalloc | Linux, musl | + mimalloc |
|---|---:|---:|---:|---:|---:|---:|
| path | 1.69 µs | 899 ns | 726 ns | 782 ns | 13.1 µs | 12.3 µs |
| filter + aggregate | 194 µs | 109 µs | 117 µs | 94 µs | 361 µs | 122 µs |
| group-by | 687 µs | 301 µs | 356 µs | 267 µs | 3.52 ms | 314 µs |
| sort + map | 740 µs | 446 µs | 418 µs | 376 µs | 962 µs | 413 µs |
| strings | 199 µs | 127 µs | 138 µs | 110 µs | 423 µs | 142 µs |
| regex | 291 µs | 171 µs | 180 µs | 152 µs | 561 µs | 177 µs |
| number formatting | 213 µs | 149 µs | 145 µs | 132 µs | 303 µs | 159 µs |
| transform | 933 µs | 589 µs | 668 µs | 543 µs | 1.84 ms | 612 µs |
| recursion | 1.01 ms | 870 µs | 743 µs | 762 µs | 1.02 ms | 730 µs |
| shipping rule | 235 µs | 191 µs | 190 µs | 167 µs | 457 µs | 192 µs |

- musl's own allocator is the slowest of the three: with it, the release binary for Linux took
  up to ten times as long as a glibc build (`group-by`). `mimalloc` makes it 28 to 91% faster,
  and within 30% of glibc with `mimalloc`, except on `path`.
- On Windows, `mimalloc` saves 14 to 56%.
- glibc's allocator is the fastest of the three. `mimalloc` still saves 9 to 25% on eight cases;
  `path` takes 56 ns more, and `recursion` is within the noise.
- `path` costs about 12 µs more on musl with either allocator: a fixed cost of each evaluation
  there, which the allocator does not explain. A command evaluates once, so its start-up hides
  it.
- macOS was not measured. Its binaries use `mimalloc` too, so that every platform runs the same
  allocator.

### WebAssembly against jsonata-js (Node.js)

Both run in the same Node.js process, on the same input. The WebAssembly module is the release
build optimized by `wasm-opt -Os`. The comparison script first checks
that both produce the same result for each case. The jsonata-js times include the `await`
that its asynchronous `evaluate` requires.

| Case | jsonata-js 2.2.2 | jlens-wasm `evaluate` | jlens-wasm `evaluateDocument` |
|---|---:|---:|---:|
| path | 2.1 µs | 7.3 µs | 1.9 µs |
| filter + aggregate | 582 µs | 363 µs | 53 µs |
| group-by | 983 µs | 683 µs | 368 µs |
| sort + map | 4.73 ms | 967 µs | 654 µs |
| strings | 562 µs | 416 µs | 103 µs |
| regex | 613 µs | 511 µs | 196 µs |
| number formatting | 537 µs | 574 µs | 256 µs |
| transform | 1.30 ms | 1.54 ms | 1.23 ms |
| recursion | 7.43 ms | 1.05 ms | 1.03 ms |
| shipping rule | 1.57 ms | 478 µs | 163 µs |

jsonata-js works on the JavaScript objects it is given, so it never pays for converting the
input. jlens-wasm `evaluate` reads the input object as the expression reaches it: an object or
array of up to 16 values through one call to a JavaScript helper, a larger one whole, through
`JSON.stringify` and a parse into jlens's representation. The path case reads two small
objects. An expression that reads every item pays what converting the whole input does:
`JSON.stringify` of the input takes 43 µs, and parsing that text 266 µs.

`evaluate` is faster than jsonata-js on seven of the ten cases, by up to 7.1× on recursion. It
is slower on three:
- `path`, which does almost no work, so the overhead of calling into WebAssembly dominates;
- `number formatting`, by 7%, because it reads every item;
- `transform`, whose deep copy of the document is the job V8's own JSON round trip does best.

Against a `Document` converted once, jlens-wasm is faster on all ten cases, by up to 11× on
filter + aggregate, 9.6× on the shipping rule and 7.2× on sort + map and on recursion. On
`path` and `transform` its lead is under 10%.

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
- **Shortcuts for common shapes.** Without a tracer, evaluation skips work that no result
  depends on. A name evaluated on a single object, such as `price` in
  `items.(price * quantity)`, looks the field up instead of building a path's sequences, unless
  the field holds an array. `=` and `!=` with a string literal compare with the literal's text
  instead of building a string; the literal's other uses (arguments, `&`, object values) still
  build one. With a tracer, evaluation takes the general path, so audits report every node; a
  property test checks that both paths give the same result.
- **Blocks and sorting.** A block (`(...)`) creates a frame for its variables only when it
  binds one. The order-by operator (`^(...)`) evaluates each item's keys once, not at each
  comparison.
- **Fixed cost.** An evaluation reads the clock only when it needs it (`$now`, `$millis`, the
  seed of `$random`, a timeout), so `1` takes about 0.1 µs.
- **Converting the input.** `evaluate` converts the parts of the input that the expression
  reads, as it reads them ([ADR 0010](../adr/0010-lazy-input.md)). An expression that reads
  every item pays for converting them, and that is now most of its time: filter + aggregate
  takes 154 µs with `evaluate` and 44 µs against a `Document`.
- **Deep copies.** A transform (`|...|`) copies its input with `$clone` before changing it, as
  jsonata-js does. That makes transforms of large documents comparatively expensive.
