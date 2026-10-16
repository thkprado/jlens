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
Node.js 24, for jlens 0.17.0, and the WebAssembly table for 0.18.0. They are meant
for comparing alternatives on one machine. They are not guarantees, and your hardware will give
different absolute values.

### Native (criterion)

Each `evaluate` call reads its `serde_json::Value` input as the expression reaches it, one
object or array at a time ([ADR 0010](../adr/0010-lazy-input.md)). `evaluate_document`
evaluates against a [`Document`](#reusing-documents) converted beforehand.

| Case | `evaluate` | `evaluate_document` |
|---|---:|---:|
| path (`customer.name`) | 0.62 µs | 0.23 µs |
| names over items (`$sum(items.quantity)`) | 18.8 µs | 6.9 µs |
| filter + aggregate | 80 µs | 44 µs |
| group-by | 101 µs | 80 µs |
| sort + map | 463 µs | 429 µs |
| strings | 119 µs | 94 µs |
| regex | 206 µs | 178 µs |
| number formatting | 206 µs | 191 µs |
| object constructor | 361 µs | 324 µs |
| transform | 979 µs | 849 µs |
| recursion (`$f(15)`, Fibonacci) | 1.01 ms | 1.02 ms |
| shipping rule | 149 µs | 111 µs |

| Other measurements | Time |
|---|---:|
| Compile `customer.name` | 1.1 µs |
| Compile the shipping rule (30 lines) | 40 µs |
| `Document::parse` of the 24 KB document | 165 µs |
| `Document::new` from a `serde_json::Value` | 108 µs |
| `serde_json::from_str` of the same text, for reference | 213 µs |
| Shipping rule, audit level `decisions` | 1.77 ms (12× without a tracer) |
| Shipping rule, audit level `full` | 8.30 ms (55× without a tracer) |

An audit costs about what it did in 0.8 (1.95 ms and 7.56 ms; level `full` 10% more), but the
ratios are much larger: without a tracer, evaluation takes the shortcuts described under
[Where the time goes](#where-the-time-goes), and with one it takes the general path, so that
the audit reports every node.

The path case reads one field of the input, `customer`, and one of that object, `name`:
`evaluate` takes 0.62 µs, where converting the whole input first took 115 µs (91 µs to
convert, 17 µs to drop, 0.9 µs to evaluate). A path step reads only the field it looks up
([ADR 0010, amended](../adr/0010-lazy-input.md#amendment-one-field-at-a-time-017)), so
`names over items` reads one field of each item and takes 18.8 µs, against 114 µs in 0.16. An
expression that reads whole items pays for converting them, as before.

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

Measured for 0.17 (0ba8143) with `evaluate`, the median of ten rounds, on the machine above;
Linux is Ubuntu 26.04 under WSL 2, on the same machine:

| Case | Windows | + mimalloc | Linux, glibc | + mimalloc | Linux, musl | + mimalloc |
|---|---:|---:|---:|---:|---:|---:|
| path | 597 ns | 348 ns | 290 ns | 315 ns | 11.7 µs | 11.7 µs |
| names over items | 18.5 µs | 13.1 µs | 14.0 µs | 12.1 µs | 77.8 µs | 26.3 µs |
| filter + aggregate | 64.1 µs | 52.5 µs | 48.3 µs | 46.3 µs | 139 µs | 64.9 µs |
| group-by | 100 µs | 83.5 µs | 81.1 µs | 78.5 µs | 294 µs | 97.8 µs |
| sort + map | 464 µs | 296 µs | 274 µs | 255 µs | 662 µs | 292 µs |
| strings | 118 µs | 70.6 µs | 75.3 µs | 61.0 µs | 207 µs | 85.6 µs |
| regex | 195 µs | 113 µs | 102 µs | 99.3 µs | 261 µs | 121 µs |
| number formatting | 204 µs | 145 µs | 138 µs | 133 µs | 259 µs | 156 µs |
| object constructor | 410 µs | 198 µs | 234 µs | 187 µs | 670 µs | 218 µs |
| transform | 1.18 ms | 604 µs | 685 µs | 554 µs | 1.84 ms | 618 µs |
| recursion | 1.07 ms | 869 µs | 723 µs | 767 µs | 1.02 ms | 738 µs |
| shipping rule | 150 µs | 122 µs | 116 µs | 105 µs | 212 µs | 131 µs |

- musl's own allocator is the slowest of the three: with it, the release binary for Linux takes
  up to 5.6 times as long as a glibc build (`names over items`), and took ten times as long on
  `group-by` in 0.15, when that case copied each group at every item. On every case but `path`,
  `mimalloc` makes it 28 to 67% faster, and within 40% of glibc with `mimalloc`, except
  `names over items`, whose 12 µs of work the fixed cost below about doubles.
- On Windows, `mimalloc` saves 17 to 52%.
- glibc's allocator is the fastest of the three. `mimalloc` still saves 3 to 20% on ten cases;
  `path` takes 25 ns more, and `recursion` 6% more.
- `path` costs about 11 µs more on musl with either allocator: a fixed cost of each evaluation
  there, which the allocator does not explain. A command evaluates once, so its start-up hides
  it.
- macOS was not measured. Its binaries use `mimalloc` too, so that every platform runs the same
  allocator.

### WebAssembly against jsonata-js (Node.js)

Both run in the same Node.js process, on the same input. The WebAssembly module is the release
build optimized by `wasm-opt -Os`. The comparison script first checks
that both produce the same result for each case. The jsonata-js times include the `await`
that its asynchronous `evaluate` requires.

| Case | jsonata-js 2.2.2 | jlens-wasm `evaluate` | jlens-wasm `evaluateJson` | jlens-wasm `evaluateDocument` |
|---|---:|---:|---:|---:|
| path | 2.1 µs | 6.7 µs | 109 µs | 1.5 µs |
| names over items | 75 µs | 331 µs | 196 µs | 15 µs |
| filter + aggregate | 616 µs | 360 µs | 279 µs | 51 µs |
| group-by | 985 µs | 410 µs | 330 µs | 99 µs |
| sort + map | 4.78 ms | 773 µs | 679 µs | 455 µs |
| strings | 576 µs | 410 µs | 347 µs | 101 µs |
| regex | 623 µs | 504 µs | 381 µs | 187 µs |
| number formatting | 536 µs | 593 µs | 492 µs | 284 µs |
| object constructor | 1.19 ms | 621 µs | 625 µs | 305 µs |
| transform | 1.31 ms | 1.27 ms | 1.45 ms | 947 µs |
| recursion | 7.62 ms | 1.03 ms | 1.13 ms | 1.03 ms |
| shipping rule | 1.61 ms | 463 µs | 418 µs | 147 µs |

jsonata-js works on the JavaScript objects it is given, so it never pays for converting the
input. jlens-wasm `evaluate` reads the input object as the expression reaches it: an object or
array of up to 16 values through one call to a JavaScript helper, a larger one whole, through
`JSON.stringify` and a parse into jlens's representation. The path case reads two small
objects. An expression that reads every item pays what converting the whole input does:
`JSON.stringify` of the input takes 41 µs, and parsing that text 264 µs. Unlike the native
and Python inputs, a JavaScript object is still read a whole level at a time: a path step does
not read its one field alone, so `names over items` converts every item.

`evaluate` is faster than jsonata-js on nine of the twelve cases, by up to 7.4× on recursion.
It is slower on three:
- `path`, which does almost no work, so the overhead of calling into WebAssembly dominates;
- `names over items`, by 4.4×: it reads every item and does little with each, so converting
  the input is nearly all of its time;
- `number formatting`, by 11%, because it reads every item.

Against a `Document` converted once, jlens-wasm is faster on all twelve cases, by up to 12× on
filter + aggregate, 11× on the shipping rule and 10× on group-by and on sort + map. On
`transform` its lead is 38%.

From 0.19 on, `evaluate` and `evaluateDocument` build their result straight into
JavaScript values ([ADR 0017](../adr/0017-result-builder.md)): the builder writes it as a list
of operations, its numbers and its strings, and one call to a JavaScript helper builds the
values, decoding each key once. Before, the result was built as `serde_json` values, printed
as JSON text and given to `JSON.parse`. The cases with large results gained most: against a
`Document`, the object constructor took 466 µs, the transform 1.24 ms and `$` alone 376 µs,
against 305 µs, 947 µs and 114 µs now. In 0.18.0, `transform` was slower than jsonata-js with
`evaluate`.

`evaluateJson` takes JSON text and returns JSON text. From 0.18.0 on, it parses the input
without the fields the expression does not read ([ADR 0016](../adr/0016-fields-read.md)), so
it is faster than `evaluate` on seven of the twelve cases; it still writes its result through
`serde_json` values. In 0.17.0, which parsed the whole text,
`path` took 296 µs, `names over items` 307 µs and `regex` 487 µs; the cases that read most of
the input, `transform` and the shipping rule, take the same time as then. On `path`, parsing
24 KB of text still costs far more than the evaluation: an input evaluated more than once is
best converted once into a `Document`.

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

Since then, the Python package builds a result as Python objects directly (ADR 0017), without a
`serde_json` value in between, and creates each key of the result once, so that a key repeated
in every object is created and hashed once. Measured before and after on the same system, with
Python values in and against a `Document`:

| Case | Before | After | Before, `Document` | After, `Document` |
|---|---:|---:|---:|---:|
| object constructor | 351 µs | 255 µs | 281 µs | 193 µs |
| transform | 886 µs | 759 µs | 733 µs | 559 µs |
| sort + map | 392 µs | 360 µs | 287 µs | 266 µs |
| names over items | 38.9 µs | 37.5 µs | 6.6 µs | 6.8 µs |

The object constructor still takes `jsonatapy` 182 µs: the rest of the gap is the evaluation
(roadmap 0.19).

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

`Document::parse_cut` parses JSON text without the fields that the rules do not read, given
what they read ([ADR 0016](../adr/0016-fields-read.md)): one rule's `Reads::input`, or the
union of the rules'. It checks the syntax of the fields it skips but does not convert them, so
the narrower the reads, the faster the parse, on this machine:

| Parse | `Document::parse` | `Document::parse_cut` |
|---|---:|---:|
| The 24 KB document, cut by what `names over items` reads | 173 µs | 73 µs |
| The same, cut by the union of every case (the transform reads it whole) | 173 µs | 170 µs |
| 1,000 records of 40 fields (1 MB), cut by what `$sum(records[status = 'open'].amount)` reads | 6.42 ms | 1.56 ms |

A rule that reads a document whole leaves nothing to skip, and then `parse_cut` costs what
`parse` does.

The WebAssembly and Python packages parse this way: `Document.fromJson` and `Document.from_json`
take a `Read`, and `evaluateJson` and `evaluate_json` cut their input by what the expression
reads. In Python, 1,000 records of 40 fields (790 KB) parse in 5.5 ms whole and 1.6 ms cut by
the rule above, and `evaluate_json` evaluates it on them in 1.6 ms in all. In Node.js,
`evaluateJson` evaluates the rule on 1,000 such records (1.4 MB) in 5.7 ms, against 10.6 ms in
0.17.0, which parsed the whole text.

`jlens eval` parses its input this way too, by what the expression reads. Measured end to end,
process start included, before and after:

| `jlens eval` | Parsed whole | Parsed cut |
|---|---:|---:|
| `$sum(items.price)` on 30,000 items of 5 fields (10 MB) | 76 ms | 27 ms |
| `$count(items)` on the same | 73 ms | 21 ms |
| `--ndjson --filter '$sum(items.price) > 470'` on 20,000 lines of 5 items (34 MB) | 175 ms | 95 ms |
| `$length($string($))` on the 10 MB document, which reads it whole | 159 ms | 107 ms |

The last row gains too: the input used to be parsed into a `serde_json::Value` and converted,
where it is now parsed straight into a `Document`.

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
- **Predicates of literals.** A predicate built from literals alone, with operators, array
  constructors and parentheses, gives the same value for every item: `[[0..19]]` is evaluated
  once, not once per item, and its indexes pick their items directly. A predicate that reads
  its context or a variable, or calls a function, is still evaluated per item, as in jsonata-js. In
  0.19 this halved `number formatting` (`items[[0..19]]...`, 203 against 92.7 µs with
  `evaluate`).
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
  `serde_json::Value` (a new string; `'a'` alone takes 0.20 µs), and two lookups. Before 0.16 a
  path of two names built a sequence for each step and took 0.69 µs; `$.customer.name` took
  1 µs and now takes 0.24 µs.
- **Names over many items.** A path step that reads an array passes it on as it is: `items`
  returns the input's array, and the step after it maps over that array's items instead of a
  copy of them. A step whose results hold no array to flatten makes them its sequence without
  copying them again. Before 0.16, `$count(items.code)` took 9.8 µs and now takes 7.0 µs,
  against 4.6 µs in jsonata-core; `$count(items)` went from 0.94 to 0.67 µs (0.39 µs). Looking
  the field up is not what remains: `$count(items.nope)` takes 4.9 µs, against 5.6 µs. A field
  found costs about 10 ns more per item, to keep the value, and `items.code` takes 22 µs,
  against 14 µs, most of it converting the 200 strings of its result.
- **Converting the input.** `evaluate` converts the parts of the input that the expression
  reads, as it reads them ([ADR 0010](../adr/0010-lazy-input.md)), and a path step reads only
  the field it looks up. An expression that reads a few fields of each item pays for those:
  filter + aggregate takes 80 µs with `evaluate` and 44 µs against a `Document`, against
  153 µs and 48 µs in 0.16. One that reads whole items, such as an object built from each, pays
  for converting them.
- **Deep copies.** A transform (`|...|`) copies its input with `$clone` before changing it, as
  jsonata-js does. That makes transforms of large documents comparatively expensive.
