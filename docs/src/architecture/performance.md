# Performance

jlens is a tree-walking interpreter, like jsonata-js. This chapter covers three things: how
performance is measured, what the numbers are, and how to use the API to get the most from it.

## Benchmarks

The workload is in `benches/` at the repository root. It is shared by the native benchmarks and
by the comparisons with jsonata-js and with the other Python packages:
- `benches/cases.json` lists twelve expressions: path navigation, a name mapped over the
  items, filters with aggregation, grouping, sorting, string and regex functions, number formatting, an object constructor, a
  transform, a recursive function, and the shipping rule from the guides.
- `benches/orders-200.json` is a 24 KB store order with 200 items.

```sh
cargo bench -p jlens-core                     # criterion, native
cargo xtask ab main .                         # two revisions against each other, native
cargo xtask wasm && cd benches/compare && npm install && node compare.mjs   # against jsonata-js
python benches/compare/python.py speed jlens=<python> ...   # against the Python packages
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

### Python against the other implementations

`benches/compare/python.py` compares the Python package with three other JSONata packages on
PyPI: `jsonatapy`, the Python package of jsonata-core, a JSONata engine in Rust;
`jsonata-python`, a port of jsonata-java in pure Python, by far the most downloaded; and
`jsonata-rs`, a port of jsonata-java in Rust. Two of them install a module named `jsonata`, so
each package needs a virtual environment of its own; the script's docstring says how to run it.
It first checks that each package gives jlens's result for each case, and all of them do. The
fourth package, `jsonata`, which runs jsonata-js in the Duktape JavaScript interpreter, is left
out: it has had no release since 2023, its module cannot load jsonata-js on Windows, and it
compiles the expression at each call, taking up to 3.5 s on these cases.

Measured for 0.17 (c76a814) on Python 3.14.8, under Linux: Ubuntu 26.04 under WSL 2, on the
machine above. `jsonatapy` has wheels up to Python 3.13 and was built from its source
distribution. The input is the document as Python values; for jlens and `jsonatapy`, also an
input converted once (`Document`, `JsonataData`):

| Case | jsonata-python 0.7.1 | jsonata-rs 0.1.4 | jsonatapy 2.2.11 | jsonatapy, `JsonataData` | jlens `evaluate` | jlens `evaluate_document` |
|---|---:|---:|---:|---:|---:|---:|
| path | 1.50 ms | 212 µs | 0.7 µs | 0.4 µs | 1.0 µs | 0.5 µs |
| names over items | 1.97 ms | 245 µs | 24 µs | 4.7 µs | 37 µs | 6.3 µs |
| filter + aggregate | 4.63 ms | 403 µs | 122 µs | 88 µs | 89 µs | 42 µs |
| group-by | 6.10 ms | 843 µs | 167 µs | 108 µs | 138 µs | 74 µs |
| sort + map | 24.5 ms | 1.88 ms | 459 µs | 395 µs | 383 µs | 279 µs |
| strings | 4.46 ms | 454 µs | 167 µs | 136 µs | 113 µs | 67 µs |
| regex | 5.09 ms | 3.87 ms | 2.69 ms | 2.68 ms | 137 µs | 107 µs |
| number formatting | 4.17 ms | 437 µs | 154 µs | 145 µs | 167 µs | 140 µs |
| object constructor | 5.99 ms | 815 µs | 181 µs | 140 µs | 357 µs | 287 µs |
| transform | 5.86 ms | 1.81 ms | 1.01 ms | 881 µs | 868 µs | 721 µs |
| recursion | 41.0 ms | 2.28 ms | 731 µs | 834 µs | 700 µs | 698 µs |
| shipping rule | 9.53 ms | 689 µs | 347 µs | 238 µs | 230 µs | 101 µs |

- With Python values in, jlens is the fastest on eight cases, by 20 times on `regex` and 51%
  on the shipping rule; `jsonatapy` on four: `path`, `names over items` (24 µs against 37 µs),
  number formatting and the object constructor. Before a path step read its one field of a
  `dict` (see below), jlens took 138 µs on `names over items` and 177 µs on `filter +
  aggregate`, and was the fastest on five cases.
- With the input converted once, jlens is the fastest on nine cases, by up to 2.4 times on the
  shipping rule and 25 times on `regex`; `jsonatapy` on `path`, `names over items` and the
  object constructor, which takes it half the time.
- On every case but `path`, jlens `evaluate` takes 2.1 (transform) to 28 times (`regex`) less
  time than `jsonata-rs`, and 7 to 64 times less than `jsonata-python`. Both read the whole
  input before evaluating: on `path` they take 0.2 and 1.5 ms where jlens takes 1.0 µs.

What reading Python values costs, per value, measured with inputs that each exercise one step
and the time of the same evaluation against a `Document` taken off. The first column is
c4ba685, whose `evaluate` took 156 µs on `names over items`; the second 22c2e66 (138 µs); the
third c76a814, where a path step reads one field (37 µs):

| Step | jlens, c4ba685 | jlens, 22c2e66 | jlens | jsonatapy |
|---|---:|---:|---:|---:|
| An `int` in a `list` | 21 ns | 20 ns | 17 ns | 14 ns |
| A `str` in a `list` | 61 ns | 52 ns | 50 ns | 20 ns |
| A `dict` in a `list`, not read | 116 ns | 90 ns | 86 ns | 30 ns |
| A `dict` in a `list`, read for one of its six `int` fields | 593 ns | 534 ns | 116 ns | 65 ns |

- 22c2e66 gained from two changes. Each value's type is first compared with the built-in
  types, before the tests that also accept subclasses, which call into Python to read the type's
  flags. The node that lets a level detect cycles is made once per level read, when the level
  holds an array or an object, rather than once for each of them.
- It still read a whole level to use one of its fields. On the last line, inserting the six
  fields into the object's map took 20% of the time, sharing their keys between objects 11%,
  allocating and freeing 17%, and the calls into Python 16%. `jsonatapy` reads only the field
  asked for, looking it up in the `dict`, and so does jlens since c76a814: a source can read one
  field of an object not read yet, and the object keeps it only when it is an array or an
  object ([ADR 0010, amended](../adr/0010-lazy-input.md#amendment-one-field-at-a-time-017)).
  The same change serves `evaluate` on a `serde_json::Value`: natively, `names over items`
  went from 152 to 18.5 µs with `cargo xtask ab --mode evaluate`, and no case was slower.
- `jsonatapy` also reads items through borrowed pointers, without changing their reference
  counts and without locking a `list` while it reads it. That takes `unsafe` code, which jlens
  forbids, and is not safe under free-threaded Python, where another thread can change the list.

How each behaves at the limits, measured by `python.py limits` on the same system. Each case
runs in a process of its own, killed after 30 seconds. `$string($)` reads the whole input, and
the cyclic input is `{"n": 1, "child": {"parent": <the input>}}`:

| Case | jsonata-python | jsonata-rs | jsonatapy | jlens |
|---|---|---|---|---|
| Input nested 900 levels, `$string($)` | result | result | result | result |
| Input nested 2,000 levels | result | result | result | `U1001` |
| Input nested 100,000 levels | `RecursionError` | process crashes (`SIGSEGV`) | process crashes (`SIGSEGV`) | `U1001` |
| Cyclic input, `n` | `RecursionError` | process crashes (`SIGSEGV`) | `1` | `1` |
| Cyclic input, `$string($)` | `RecursionError` | process crashes (`SIGSEGV`) | process crashes (`SIGSEGV`) | `U1004` |
| Unbounded recursion, `1 + $f($n + 1)` | `RecursionError` | `STACK_OVERFLOW` | `U1001` | `D1011` |
| The same, with a depth limit of 500 | `D1011` | `STACK_OVERFLOW` | `U1001` | `D1011` |
| Endless tail call, `$f($n + 1)` | runs on | runs on | `U1001` | runs on |
| The same, with a timeout of 1 s | `D1012` after 1.0 s | `TIMEOUT` after 1.0 s | `D1012` after 1.0 s | `D1012` after 1.0 s |

- jlens fails with a JSONata error in each case, and never ends the process. It refuses input
  nested more than 1,000 levels, which the others accept as long as their stack or Python's
  recursion limit lasts: a fixed limit that fails the same way on every platform, at the
  cost of inputs between 1,000 levels and that point.
- jlens and `jsonatapy` meet a cycle only when the expression reaches it; the other two read
  the whole input first, so `n` fails too.
- `jsonata-rs` and `jsonatapy` crash the interpreter on input nested 100,000 levels, and when
  they read a cycle. On Windows, whose main thread has a smaller stack, unbounded recursion
  crashes `jsonata-rs` too.
- `jsonatapy` limits recursion to 302 levels, even with a depth limit of 500, and tail calls to
  500: the endless tail call stops after 0.02 s, and so does a loop of 100,000 tail calls that
  jlens finishes.
- Otherwise, no package stops an endless evaluation by default: set a timeout.

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
