# Performance

jlens is a tree-walking interpreter, like jsonata-js. This chapter covers three things: how
performance is measured, what the numbers are, and how to use the API to get the most from it.

## Benchmarks

The workload is in `benches/` at the repository root. It is shared by the native benchmarks and
by the comparison with jsonata-js:
- `benches/cases.json` lists twelve expressions: path navigation, a name mapped over the
  items, filters with aggregation, grouping, sorting, string and regex functions, number formatting, an object constructor, a
  transform, a recursive function, and the shipping rule from the guides.
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
Node.js 24, for jlens 0.16.0. They are meant
for comparing alternatives on one machine. They are not guarantees, and your hardware will give
different absolute values.

### Native (criterion)

Each `evaluate` call reads its `serde_json::Value` input as the expression reaches it, one
object or array at a time ([ADR 0010](../adr/0010-lazy-input.md)). `evaluate_document`
evaluates against a [`Document`](#reusing-documents) converted beforehand.

| Case | `evaluate` | `evaluate_document` |
|---|---:|---:|
| path (`customer.name`) | 1.2 µs | 0.24 µs |
| names over items (`$sum(items.quantity)`) | 114 µs | 7.5 µs |
| filter + aggregate | 153 µs | 48 µs |
| group-by | 193 µs | 88 µs |
| sort + map | 549 µs | 476 µs |
| strings | 202 µs | 102 µs |
| regex | 294 µs | 178 µs |
| number formatting | 213 µs | 182 µs |
| object constructor | 449 µs | 328 µs |
| transform | 989 µs | 852 µs |
| recursion (`$f(15)`, Fibonacci) | 1.03 ms | 1.02 ms |
| shipping rule | 224 µs | 114 µs |

| Other measurements | Time |
|---|---:|
| Compile `customer.name` | 1.1 µs |
| Compile the shipping rule (30 lines) | 42 µs |
| `Document::parse` of the 24 KB document | 168 µs |
| `Document::new` from a `serde_json::Value` | 108 µs |
| `serde_json::from_str` of the same text, for reference | 230 µs |
| Shipping rule, audit level `decisions` | 1.92 ms (7.8× without a tracer) |
| Shipping rule, audit level `full` | 9.17 ms (37× without a tracer) |

An audit costs about what it did in 0.8 (1.95 ms and 7.56 ms; level `full` 21% more), but the
ratios have doubled: without a tracer, evaluation takes the shortcuts described under
[Where the time goes](#where-the-time-goes), and with one it takes the general path, so that
the audit reports every node.

The path case reads two objects of the input, the document and its `customer`: `evaluate`
takes 1.2 µs, where converting the whole input first took 115 µs (91 µs to convert, 17 µs to
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

Measured for 0.16 (2f069f7) with `evaluate`, the median of ten rounds, on the machine above;
Linux is Ubuntu 26.04 under WSL 2, on the same machine:

| Case | Windows | + mimalloc | Linux, glibc | + mimalloc | Linux, musl | + mimalloc |
|---|---:|---:|---:|---:|---:|---:|
| path | 1.21 µs | 698 ns | 573 ns | 611 ns | 12.5 µs | 12.1 µs |
| names over items | 156 µs | 69 µs | 79 µs | 60 µs | 285 µs | 83 µs |
| filter + aggregate | 194 µs | 106 µs | 114 µs | 94 µs | 350 µs | 120 µs |
| group-by | 213 µs | 137 µs | 151 µs | 125 µs | 500 µs | 151 µs |
| sort + map | 553 µs | 351 µs | 338 µs | 303 µs | 845 µs | 346 µs |
| strings | 204 µs | 123 µs | 132 µs | 109 µs | 419 µs | 140 µs |
| regex | 294 µs | 168 µs | 176 µs | 151 µs | 527 µs | 179 µs |
| number formatting | 210 µs | 145 µs | 149 µs | 131 µs | 382 µs | 157 µs |
| object constructor | 443 µs | 254 µs | 362 µs | 234 µs | 890 µs | 265 µs |
| transform | 1.01 ms | 595 µs | 681 µs | 548 µs | 1.88 ms | 606 µs |
| recursion | 1.01 ms | 867 µs | 738 µs | 775 µs | 1.03 ms | 726 µs |
| shipping rule | 220 µs | 174 µs | 178 µs | 153 µs | 393 µs | 180 µs |

- musl's own allocator is the slowest of the three: with it, the release binary for Linux takes
  up to 3.6 times as long as a glibc build (`names over items`), and took ten times as long on
  `group-by` in 0.15, when that case copied each group at every item. On every case but `path`,
  `mimalloc` makes it 30 to 71% faster, and within 40% of glibc with `mimalloc`.
- On Windows, `mimalloc` saves 14 to 56%.
- glibc's allocator is the fastest of the three. `mimalloc` still saves 10 to 36% on ten cases;
  `path` takes 38 ns more, and `recursion` 5% more.
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
| path | 2.4 µs | 7.3 µs | 1.7 µs |
| names over items | 70 µs | 355 µs | 16 µs |
| filter + aggregate | 682 µs | 389 µs | 55 µs |
| group-by | 1.11 ms | 453 µs | 109 µs |
| sort + map | 5.33 ms | 864 µs | 528 µs |
| strings | 599 µs | 422 µs | 100 µs |
| regex | 641 µs | 518 µs | 206 µs |
| number formatting | 563 µs | 591 µs | 262 µs |
| object constructor | 1.22 ms | 805 µs | 484 µs |
| transform | 1.37 ms | 1.60 ms | 1.24 ms |
| recursion | 7.90 ms | 1.04 ms | 1.03 ms |
| shipping rule | 1.66 ms | 469 µs | 148 µs |

jsonata-js works on the JavaScript objects it is given, so it never pays for converting the
input. jlens-wasm `evaluate` reads the input object as the expression reaches it: an object or
array of up to 16 values through one call to a JavaScript helper, a larger one whole, through
`JSON.stringify` and a parse into jlens's representation. The path case reads two small
objects. An expression that reads every item pays what converting the whole input does:
`JSON.stringify` of the input takes 49 µs, and parsing that text 263 µs.

`evaluate` is faster than jsonata-js on eight of the twelve cases, by up to 7.6× on recursion.
It is slower on four:
- `path`, which does almost no work, so the overhead of calling into WebAssembly dominates;
- `names over items`, by 5.1×: it reads every item and does little with each, so converting
  the input is nearly all of its time;
- `number formatting`, by 5%, because it reads every item;
- `transform`, whose deep copy of the document is the job V8's own JSON round trip does best.

Against a `Document` converted once, jlens-wasm is faster on all twelve cases, by up to 12× on
filter + aggregate, 11× on the shipping rule and 10× on group-by and on sort + map. On
`transform` its lead is 10%.

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
  depends on. A path of names evaluated on a single object, such as `price` in
  `items.(price * quantity)` or `customer.name`, or starting with a variable, such as `$i.code`,
  looks its fields up instead of building a path's sequences, unless a field before the last
  holds an array. A name mapped over the items of a path (`code` in `items.code`) is looked up in each
  item without evaluating the step for it.
- **String literals.** `=` and `!=` with a string literal compare with the literal's text
  instead of building a string, and so does a grouping with a literal key, which finds its group
  by the text: in `items{type: {'total': ..., 'n': ...}}`, `'total'` and `'n'` are built once
  per group instead of once per item (group-by took 12% less time). The
  literal's other uses (arguments, `&`, object values) still build one each time. In 0.16 that
  was measured at about 3% of expressions that use a literal once per item, such as
  `items.(code & '-' & type)` or `items.$pad(code, 8, '0')`: too little for a slot per literal
  in each evaluation, which the AST needs because it is `Send` while values use `Rc`.
- **Tracing.** With a tracer, evaluation takes the general path, so audits report every node; a
  property test checks that both paths give the same result.
- **Blocks and sorting.** A block (`(...)`) creates a frame for its variables only when it
  binds one. The order-by operator (`^(...)`) evaluates each item's keys once, not at each
  comparison.
- **Fixed cost.** An evaluation reads the clock only when it needs it (`$now`, `$millis`, the
  seed of `$random`, a timeout), so `1` takes about 0.12 µs, against 0.06 µs in jsonata-core.
  `customer.name` takes 0.23 µs, against 0.14 µs: that fixed cost, the result converted to a
  `serde_json::Value` (a new string; `'a'` alone takes 0.21 µs), and two lookups. Before 0.16 a
  path of two names built a sequence for each step and took 0.69 µs; `$.customer.name` took
  1 µs and now takes 0.24 µs.
- **Names over many items.** A path step that reads an array passes it on as it is: `items`
  returns the input's array, and the step after it maps over that array's items instead of a
  copy of them. A step whose results hold no array to flatten makes them its sequence without
  copying them again. Before 0.16, `$count(items.code)` took 9.8 µs and now takes 7.0 µs,
  against 4.9 µs in jsonata-core; `$count(items)` went from 0.94 to 0.67 µs (0.39 µs). Looking
  the field up is not what remains: `$count(items.nope)` takes 4.9 µs, against 5.7 µs. A field
  found costs about 10 ns more per item, to keep the value, and `items.code` takes 20 µs,
  against 14 µs, most of it converting the 200 strings of its result.
- **Converting the input.** `evaluate` converts the parts of the input that the expression
  reads, as it reads them ([ADR 0010](../adr/0010-lazy-input.md)). An expression that reads
  every item pays for converting them, and that is now most of its time: filter + aggregate
  takes 153 µs with `evaluate` and 48 µs against a `Document`.
- **Deep copies.** A transform (`|...|`) copies its input with `$clone` before changing it, as
  jsonata-js does. That makes transforms of large documents comparatively expensive.
