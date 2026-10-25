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
- `crates/jlens-core/benches/host_state.rs` builds a document from Rust values again and again,
  as an application that keeps its state in Rust types does
  ([below](#documents-built-from-rust-values)). It is native only.

```sh
cargo bench -p jlens-core                     # criterion, native
cargo xtask ab main .                         # two revisions against each other, native
cargo xtask wasm && cd benches/compare && npm install && node compare.mjs   # against jsonata-js
python benches/compare/python.py speed jlens=<python> ...   # against the Python packages
cargo bench -p jlens-regress --bench engine   # the regular expression engine alone (below)
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
Node.js 24, for jlens 0.23.0. They are meant
for comparing alternatives on one machine. They are not guarantees, and your hardware will give
different absolute values.

### Native (criterion)

Each `evaluate` call reads its `serde_json::Value` input as the expression reaches it, one
object or array at a time ([ADR 0010](../adr/0010-lazy-input.md)). `evaluate_document`
evaluates against a [`Document`](#reusing-documents) converted beforehand.

| Case | `evaluate` | `evaluate_document` |
|---|---:|---:|
| path (`customer.name`) | 0.62 µs | 0.24 µs |
| names over items (`$sum(items.quantity)`) | 17.8 µs | 6.3 µs |
| filter + aggregate | 64 µs | 46 µs |
| group-by | 102 µs | 84 µs |
| sort + map | 466 µs | 433 µs |
| strings | 120 µs | 95 µs |
| regex | 203 µs | 180 µs |
| number formatting | 45 µs | 34 µs |
| object constructor | 265 µs | 233 µs |
| transform | 930 µs | 793 µs |
| recursion (`$f(15)`, Fibonacci) | 1.02 ms | 1.02 ms |
| shipping rule | 148 µs | 110 µs |

| Other measurements | Time |
|---|---:|
| Compile `customer.name` | 1.1 µs |
| Compile the shipping rule (30 lines) | 40 µs |
| `Document::parse` of the 24 KB document | 171 µs |
| `Document::new` from a `serde_json::Value` | 113 µs |
| `serde_json::from_str` of the same text, for reference | 206 µs |
| Shipping rule, audit level `decisions` | 1.67 ms (11× without a tracer) |
| Shipping rule, audit level `full` | 8.04 ms (55× without a tracer) |

An audit costs about what it did in 0.8 (1.95 ms and 7.56 ms; level `full` 6% more), but the
ratios are much larger: without a tracer, evaluation takes the shortcuts described under
[Where the time goes](#where-the-time-goes), and with one it takes the general path, so that
the audit reports every node.

The path case reads one field of the input, `customer`, and one of that object, `name`:
`evaluate` takes 0.62 µs, where converting the whole input first took 115 µs (91 µs to
convert, 17 µs to drop, 0.9 µs to evaluate). A path step reads only the field it looks up
([ADR 0010, amended](../adr/0010-lazy-input.md#amendment-one-field-at-a-time-017)), so
`names over items` reads one field of each item and takes 17.8 µs, against 114 µs in 0.16. An
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
| path | 2.1 µs | 6.8 µs | 110 µs | 1.4 µs |
| names over items | 62 µs | 335 µs | 194 µs | 13 µs |
| filter + aggregate | 604 µs | 368 µs | 282 µs | 49 µs |
| group-by | 1.00 ms | 420 µs | 330 µs | 98 µs |
| sort + map | 4.76 ms | 776 µs | 670 µs | 447 µs |
| strings | 567 µs | 417 µs | 349 µs | 99 µs |
| regex | 612 µs | 517 µs | 387 µs | 188 µs |
| number formatting | 535 µs | 369 µs | 247 µs | 43 µs |
| object constructor | 1.17 ms | 538 µs | 536 µs | 220 µs |
| transform | 1.33 ms | 1.24 ms | 1.36 ms | 911 µs |
| recursion | 7.53 ms | 1.00 ms | 1.11 ms | 1.00 ms |
| shipping rule | 1.61 ms | 467 µs | 416 µs | 136 µs |

jsonata-js works on the JavaScript objects it is given, so it never pays for converting the
input. jlens-wasm `evaluate` reads the input object as the expression reaches it: an object or
array of up to 16 values through one call to a JavaScript helper, a larger one whole, through
`JSON.stringify` and a parse into jlens's representation. The path case reads two small
objects. An expression that reads every item pays what converting the whole input does:
`JSON.stringify` of the input takes 41 µs, and parsing that text 272 µs. Unlike the native
and Python inputs, a JavaScript object is still read a whole level at a time: a path step does
not read its one field alone, so `names over items` converts every item.

`evaluate` is faster than jsonata-js on ten of the twelve cases, by up to 7.5× on recursion.
It is slower on two:
- `path`, which does almost no work, so the overhead of calling into WebAssembly dominates;
- `names over items`, by 5.4×: it reads every item and does little with each, so converting
  the input is nearly all of its time.

In 0.18.0 it was also slower on `number formatting`, by 11%; it is now 31% faster (roadmap 0.19
#2 and #4).

Against a `Document` converted once, jlens-wasm is faster on all twelve cases, by up to 12× on
number formatting, on filter + aggregate and on the shipping rule, 11× on sort + map and 10× on
group-by. On `transform` its lead is 46%.

From 0.19 on, `evaluate` and `evaluateDocument` build their result straight into
JavaScript values ([ADR 0017](../adr/0017-result-builder.md)): the builder writes it as a list
of operations, its numbers and its strings, and one call to a JavaScript helper builds the
values, decoding each key once. Before, the result was built as `serde_json` values, printed
as JSON text and given to `JSON.parse`. The cases with large results gained most: against a
`Document`, the object constructor took 466 µs, the transform 1.24 ms and `$` alone 376 µs,
against 305 µs, 947 µs and 114 µs after it. In 0.18.0, `transform` was slower than jsonata-js with
`evaluate`.

`evaluateJson` takes JSON text and returns JSON text. From 0.18.0 on, it parses the input
without the fields the expression does not read ([ADR 0016](../adr/0016-fields-read.md)), so
it is faster than `evaluate` on eight of the twelve cases, and as fast on the object
constructor; it still writes its result through
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

Measured for 0.23.0, on Python 3.14.8, under Linux: Ubuntu 26.04 under WSL 2, on the machine
above. `jsonatapy` has wheels up to Python 3.13 and was built from its source distribution. The
input is the document as Python values; for jlens and `jsonatapy`, also an input converted once
(`Document`, `JsonataData`). Two more runs of jlens and `jsonatapy` alone gave the same times
within 4% for jlens and 9% for `jsonatapy`:

| Case | jsonata-python 0.7.1 | jsonata-rs 0.1.4 | jsonatapy 2.2.11 | jsonatapy, `JsonataData` | jlens `evaluate` | jlens `evaluate_document` |
|---|---:|---:|---:|---:|---:|---:|
| path | 1.52 ms | 216 µs | 0.7 µs | 0.4 µs | 0.9 µs | 0.6 µs |
| names over items | 1.99 ms | 237 µs | 24 µs | 4.7 µs | 23 µs | 5.9 µs |
| filter + aggregate | 4.66 ms | 403 µs | 120 µs | 88 µs | 65 µs | 39 µs |
| group-by | 6.31 ms | 861 µs | 165 µs | 109 µs | 107 µs | 71 µs |
| sort + map | 25.0 ms | 1.86 ms | 456 µs | 396 µs | 313 µs | 257 µs |
| strings | 4.49 ms | 466 µs | 165 µs | 141 µs | 84 µs | 56 µs |
| regex | 5.11 ms | 3.87 ms | 2.70 ms | 2.65 ms | 126 µs | 100 µs |
| number formatting | 4.21 ms | 433 µs | 155 µs | 148 µs | 35 µs | 24 µs |
| object constructor | 6.10 ms | 815 µs | 185 µs | 142 µs | 157 µs | 116 µs |
| transform | 5.95 ms | 1.79 ms | 1.05 ms | 883 µs | 586 µs | 456 µs |
| recursion | 41.4 ms | 2.30 ms | 729 µs | 829 µs | 748 µs | 746 µs |
| shipping rule | 9.64 ms | 687 µs | 330 µs | 238 µs | 164 µs | 94 µs |

- With Python values in, jlens is the fastest on ten cases, by 21 times on `regex`, 4.4 times
  on number formatting and twice on the shipping rule; `jsonatapy` on `path` (0.7 against
  0.9 µs) and, by 3% or less in all three runs, on `recursion`. At 0a1c6ad, before items 9 to 11 of roadmap 0.19,
  jlens took 1.1 µs on `path`, 35 µs on `names over items` and 203 µs on the object
  constructor, and was the fastest on nine cases; in 0.17 (c76a814) it took 357 µs on the
  object constructor, 167 µs on number formatting and 868 µs on the transform, and was the
  fastest on eight; before a path step read its one field of a `dict` (see below), it took
  138 µs on `names over items` and was the fastest on five.
- With the input converted once, jlens is the fastest on ten cases, by up to 6.3 times on
  number formatting and 26 times on `regex`, and on the object constructor by 116 against
  142 µs; `jsonatapy` is faster on `path` and `names over items`.
- On every case but `path`, jlens `evaluate` takes 3 (`transform`, `recursion`) to 31 times (`regex`) less
  time than `jsonata-rs`, and 10 to 120 times less than `jsonata-python`. Both read the whole
  input before evaluating: on `path` they take 0.2 and 1.5 ms where jlens takes 0.9 µs.

From 0.19 on, the Python package builds a result as Python objects directly (ADR 0017), without a
`serde_json` value in between, and creates each key of the result once, so that a key repeated
in every object is created and hashed once. Measured before and after on the same system, with
Python values in and against a `Document`:

| Case | Before | After | Before, `Document` | After, `Document` |
|---|---:|---:|---:|---:|
| object constructor | 351 µs | 255 µs | 281 µs | 193 µs |
| transform | 886 µs | 759 µs | 733 µs | 559 µs |
| sort + map | 392 µs | 360 µs | 287 µs | 266 µs |
| names over items | 38.9 µs | 37.5 µs | 6.6 µs | 6.8 µs |

The rest of the gap to `jsonatapy` on the object constructor was the evaluation, which the
object constructor over a single item (roadmap 0.19 #4) and the changes below closed.

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

From 0.19 on, the arrays and objects of a level cost one allocation each, not two: the source
writes them as members of one value it allocates for the level, which holds their Python
values (`input::Sources`, [ADR 0010, amended](../adr/0010-lazy-input.md#amendment-one-allocation-per-member-019)),
instead of boxing a source for each. A level that holds a single array or object keeps it
inline, so it allocates no more than before. Measured before and after on the same system,
with Python values in, the median of three alternated runs; `rows` is a `list` of 200 `list`s
`[i, [i]]`:

| Expression | Before | After |
|---|---:|---:|
| `$count(items)` | 19.0 µs | 15.4 µs |
| `$sum(items.quantity)` (names over items) | 37.4 µs | 35.1 µs |
| `$count(rows)` | 23.6 µs | 20.2 µs |
| `$count(rows.*)` | 116 µs | 112 µs |

Natively, `cargo xtask ab --mode evaluate` over 20 rounds finds no case slower but group-by,
by 2%. A first version made each lazy object 16 bytes larger, to hold the member's index, and
names over items 17% slower; the index now sits in room the layout of an object left free.

From 0.19 on, the options of the `evaluate` methods are keyword-only parameters instead of
`**options`, so that a call with options builds no tuple and no `dict`. `python.py speed` passes
`timeout=None, max_depth=None` to jlens. Measured before and after on the same system, the
median of three alternated runs: `path` went from 1.1 to 0.9 µs, the other cases within 3%.
`customer.name` took 1.01 µs with these two keywords and 0.82 µs without before, and now 0.86
against 0.82 µs.

Also from 0.19 on, each `Expression` keeps an interned `str` for every field name of what it reads
(`reads()`), made on its first evaluation: a step into a `dict` looks its field up with it,
instead of building a `str`, hashing it and freeing it at each lookup. A name the expression
does not read by name, as when it reads its input whole, is looked up as before. Measured the
same way, against the input as Python values:

| Case | Before | After |
|---|---:|---:|
| path | 0.9 µs | 0.8 µs |
| names over items | 34.3 µs | 28.2 µs |
| filter + aggregate | 83.2 µs | 70.6 µs |
| group-by | 132.5 µs | 112.8 µs |
| sort + map | 364.0 µs | 333.4 µs |
| strings | 114.6 µs | 100.6 µs |
| regex | 133.7 µs | 125.9 µs |
| object constructor | 202.6 µs | 179.8 µs |
| shipping rule | 225.2 µs | 179.9 µs |

Number formatting, the transform and the recursion stayed within 1%, as did every case against a
converted input, which looks no key up in a `dict`.

Also from 0.19 on, the package uses mimalloc as its allocator, as the command does, with
`local_dynamic_tls`, the thread-local model of a library loaded at run time ([ADR 0014,
amended](../adr/0014-python-package.md#amendment-mimalloc-as-the-allocator-019)). Measured the
same way, with glibc's allocator before; the last rows come from `variants.py`, kept with the
profiles of roadmap 0.19 #8:

| Case | Before | After | Before, `Document` | After, `Document` |
|---|---:|---:|---:|---:|
| path | 0.9 µs | 0.9 µs | 0.5 µs | 0.5 µs |
| names over items | 28.1 µs | 23.1 µs | 6.6 µs | 6.4 µs |
| filter + aggregate | 70.2 µs | 67.3 µs | 39.6 µs | 39.5 µs |
| group-by | 112.2 µs | 107.9 µs | 71.6 µs | 72.0 µs |
| sort + map | 337.8 µs | 311.2 µs | 263.3 µs | 260.4 µs |
| strings | 103.2 µs | 85.5 µs | 64.5 µs | 57.0 µs |
| regex | 127.8 µs | 122.4 µs | 96.2 µs | 99.6 µs |
| number formatting | 43.2 µs | 34.9 µs | 24.1 µs | 23.4 µs |
| object constructor | 182.9 µs | 149.9 µs | 138.7 µs | 115.6 µs |
| transform | 676.0 µs | 579.4 µs | 532.1 µs | 456.7 µs |
| recursion | 700.1 µs | 750.6 µs | 703.2 µs | 764.5 µs |
| shipping rule | 179.4 µs | 172.4 µs | 99.7 µs | 99.3 µs |
| `1`, no input | 0.31 µs | 0.33 µs | – | – |
| `customer.name` | 0.74 µs | 0.78 µs | 0.49 µs | 0.51 µs |
| `$count(items)` | 14.8 µs | 9.6 µs | 0.77 µs | 0.80 µs |

- `recursion` takes 7% more, as natively with glibc (6%, in the allocator table above).
- `1` reads no input, so it has no `Document` to evaluate against. Against a `Document`,
  `customer.name` and `$count(items)` read no `dict` and allocate nothing per item: like the
  smallest evaluations with Python values in, they take 0.02 to 0.03 µs more.
- Plain mimalloc, with initial-exec thread-locals, gave the same times within 4%, but for the
  smallest cases, 0.01 to 0.02 µs faster, and reading lists of `dict`s, 7 to 11% slower
  (`names over items` 24.7 µs, `$count(items)` 10.8 µs).
- The wheel grows from 1.23 to 1.30 MB for Linux x86-64, and from 1.13 to 1.19 MB for
  Windows.

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

### The regular expression engine

`jlens-regress`, jlens's fork of `regress` ([ADR 0020](../adr/0020-own-regress-fork.md)), has a
benchmark of its own, `crates/jlens-regress/benches/engine.rs`, whose cases are in
`engine.json` beside it. Each case finds every match of a pattern in a text, as jlens does: from
the end of the previous match, without an interrupt, or with one (when the evaluation has a
timeout or a cancellation token). The texts are 200 lines of a log (15 KB), a sentence of English
repeated to 12 KB, and a sentence of Portuguese, Greek and Japanese repeated to 20 KB of UTF-8.
`engine.mjs` runs the same cases in Node.js, with `exec` and the flag `g`, as jsonata-js does.

Each variant is a binary of its own, built with functions and blocks aligned, in a target
directory of its own:

```sh
export RUSTFLAGS="-C llvm-args=-align-all-functions=6 -C llvm-args=-align-all-nofallthru-blocks=5"
export CARGO_TARGET_DIR=target/aligned
cargo bench -p jlens-regress --bench engine            # without an interrupt
JLENS_REGRESS_INTERRUPT=1 cargo bench -p jlens-regress --bench engine
node crates/jlens-regress/benches/engine.mjs           # V8
```

Each run saves its times and prints what the check costs against the last run of the other
variant. These numbers are the baseline of the engine: a change to it is measured the same way,
aligned, against the revision before. Without the alignment, a change that runs no differently
moves cases by up to 8 % ([ADR 0009](../adr/0009-bounded-regex.md)). Measured for 0.23 (the
median of 31 samples of at least 5 ms; V8 13.6, Node.js 24.19):

| Case | Pattern | Matches | `jlens-regress` | With an interrupt | V8 |
|---|---|---:|---:|---:|---:|
| literal | `status=200` | 200 | 12.3 µs | 12.5 µs (+1.4%) | 16.4 µs |
| literal, no match | `status=500` | 0 | 395 ns | 396 ns (+0.3%) | 5.4 µs |
| case-insensitive literal | `lazy dog`, `i` | 200 | 12.9 µs | 12.9 µs (−0.1%) | 12.7 µs |
| classes | `\d{4}-\d{2}-\d{2}T[\d:]+Z` | 200 | 48.3 µs | 46.7 µs (−3.3%) | 22.5 µs |
| classes, no match | `\d{3}-\d{4}` | 0 | 5.9 µs | 5.8 µs (−1.0%) | 1.5 µs |
| alternation of words | `\b(?:fox\|dog\|cat)s?\b` | 400 | 185 µs | 181 µs (−2.1%) | 22.4 µs |
| words | `\b\w+\b` | 2,400 | 348 µs | 338 µs (−2.9%) | 121 µs |
| captures | `user=(\w+) id=(\d+)` | 200 | 59.0 µs | 57.7 µs (−2.2%) | 15.8 µs |
| lazy | `path=(.*?) ` | 200 | 68.0 µs | 62.5 µs (−8.1%) | 11.9 µs |
| lines | `^.*status=200$`, `m` | 200 | 48.6 µs | 51.5 µs (+5.9%) | 17.1 µs |
| backreference, no match | `(\w+)\s+\1` | 0 | 570 µs | 647 µs (+13.5%) | 147 µs |
| lookahead | `\w+(?=,)` | 200 | 638 µs | 647 µs (+1.4%) | 131 µs |
| lookbehind | `(?<=the )\w+` | 200 | 250 µs | 242 µs (−3.5%) | 48.7 µs |
| Unicode text | `\S+` | 2,000 | 296 µs | 299 µs (+1.2%) | 103 µs |
| Unicode text, case-insensitive | `γράμματα`, `i` | 200 | 31.8 µs | 31.9 µs (+0.1%) | 9.9 µs |
| Unicode property | `\p{L}+`, `u` | 2,000 | 396 µs | 392 µs (−0.9%) | 463 µs |
| anchored, short text | `^1[0-9]$` on `17` | 1 | 105 ns | 103 ns (−1.5%) | 51 ns |

| Compilation | Pattern | `jlens-regress` | V8 `new RegExp` | V8, and a first `exec` |
|---|---|---:|---:|---:|
| date-time | `^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?(Z\|[+-]\d{2}:\d{2})$` | 11.9 µs | 2.6 µs | 24.0 µs |
| e-mail, case-insensitive | `^[a-z0-9._%+-]+@(?:[a-z0-9-]+\.)+[a-z]{2,}$`, `i` | 8.4 µs | 1.8 µs | 37.5 µs |
| Unicode property | `\p{L}+`, `u` | 3.9 µs | 34.5 µs | 81.7 µs |

- V8 compiles a pattern to machine code, and is 2 to 8 times faster wherever the engine runs
  its instructions at many positions of the text: alternations, classes, lookarounds,
  backreferences. `jlens-regress` is a backtracker that interprets its instructions. It is
  faster where it can search for a literal first (`memchr`): 1.3 times on `literal`, 14 times
  on a literal that does not occur; and on `\p{L}+`.
- **The lookarounds** took 10 times V8's time in 0.22, and take 5 times since 0.23. Each one
  runs an attempt of its own, which needs an empty backtracking stack, and a copy of the groups
  it holds, to restore them. Both were allocated every time the match reached the lookaround,
  and the stack grew from one entry as the attempt pushed. `\w+(?=,)` runs about 25,000
  lookaheads over this text: a profile (`perf`, on Linux) put a quarter of the time in `malloc`
  and `free`. The matcher now keeps both and reuses them from one lookaround to the next,
  nested ones included. Measured aligned, both variants, against 0.22.0: lookahead 1.40 ms to
  638 µs (−55 %), lookbehind 510 to 250 µs (−52 %). Most of the cost was Windows's allocator:
  on Linux (WSL), the two cases took 621 and 240 µs before and 5 and 8 % less after.
- The storage is behind a pointer, allocated by the first lookaround of a search. As two fields
  of the matcher, it changed the layout the backtracker's main loop works on, and made cases
  without a lookaround up to 23 % slower on Linux (`lines`). Behind a pointer, that loop
  compiles to the same instructions as before (compared in the Linux build): the other cases
  move by −12 to +5 % from 0.22.0, both ways, and differently on Windows and Linux, which is
  code placement. On Windows, `backreference, no match` is 5 % slower and `case-insensitive
  literal` 12 % faster; on Linux, the same build makes `classes` 4 to 9 % slower.
- V8's `new RegExp` only parses the pattern; the first `exec` compiles it, and V8 keeps both by
  the pattern's source, so the benchmark gives each compilation a pattern V8 has not seen (with
  an alternative `|<n>` added). jlens compiles a pattern once per expression, when the
  expression is compiled.
- **The interrupt** costs between −0.2 and 0.8 % over these 17 cases (the geometric mean, three
  runs of 0.23), from 8.1 % faster to 13.5 % slower per case. For the same code, 0.22.0 gave
  1.7 % when it was released and 3.5 % on the day of the lookaround change: the check itself
  did not change. Most of that spread is not the check: in a variant that has the interrupt's code
  but counts no step, `literal` is 12.8 % slower, and `words` 9 % faster. The backtracker is one
  large function, and any change to it, the check included, makes the compiler allocate its
  registers anew. Counting itself costs 7 to 13 % on `(\w+)\s+\1`, which backtracks one
  character at a time, in steps of a few instructions, each with a decrement and a branch more.
  In 0.21 the counter was in memory, so every step waited for the store of the one before: the
  check cost about 2.5 % on average, 7 to 11 % on the lookahead, the backreference and
  case-insensitive Unicode text. Each attempt now counts on a copy that the compiler keeps in a
  register.
- Two copies of the backtracker in one binary, as `find_from` and `find_from_interruptible`
  build, are inlined differently: two copies that both count nothing differ by up to 16 % there.
  That is why each variant is a binary of its own.

Through jlens, the engine's changes leave the library as fast as in 0.21. `cargo xtask ab
v0.21.2 HEAD`, with the 0.22 code complete and ten rounds, puts every case of `evaluate` between
2.8% faster (`transform`) and 1.4% slower (`filter + aggregate`); `regex` is 0.2% faster. With
`--mode document`, every case is between 3.8% faster and 1.5% slower, except `path` at 3.0% and
`filter + aggregate` at 2.0% slower, still 2.6% and 2.2% over 20 rounds. Neither runs code that
changed in 0.22. Built with functions and blocks aligned, as above, `filter + aggregate` is
0.6% slower and `path` 13.5% faster (238 ns against 275 ns): the difference is where the code
lies, not what it does.

The lookarounds of 0.23 leave the rest as fast as in 0.22: the change is in `jlens-regress`
alone, and the `regex` case has no lookaround. `cargo xtask ab v0.22.0 HEAD`, ten rounds, puts
every case of `evaluate` between 2.8% faster (`shipping rule`) and 1.6% slower, except
`sort + map` at 2.5%, 1.3% over 20 rounds. With `--mode document`, `sort + map`, `recursion` and
`filter + aggregate` stay 2.9%, 2.3% and 2.2% slower over 20 rounds, and none of them runs a
regular expression. Built aligned, the three are between 1.8% faster and 1.6% slower in both
modes, and `number formatting`, unaligned 0.2% faster, becomes 3.4% slower: code placement
again.

### What the limits cost

From 0.20 on, every evaluation counts its steps for the step limit
([ADR 0018](../adr/0018-step-limit.md)). Without a limit the budget is too large to be spent,
so a step costs the same comparison with or without one. The timeout reads the clock at every
256th step rather than at every step, so a timeout fires at most 256 steps late. Measured for
0.20 against 0.19.0, on the machine above, once the rest of the release was in:

- **Natively**, `cargo xtask ab v0.19.0 .`, ten rounds: every case between 2.9% faster
  (`recursion`) and 1.0% slower (`group-by`). With a step limit (`--b-max-steps 1000000000`,
  0.20 with a limit against 0.19.0 without one), every case between 2.5% faster and 1.7%
  slower, except `path` at 6.0% slower, which was 0.5% slower over 30 rounds. With a timeout of
  60 s in both (`--timeout 60000`), 0.20 is 4% (`path`) to 38% (`shipping rule`) faster.
- **In WebAssembly**, one build per Node.js process, the processes alternating, four each:
  `evaluate` between 2.0% faster (`recursion`) and 1.9% slower, `evaluateJson` within 2.3%,
  `evaluateDocument` between 14% faster (`names over items`) and 1.5% slower. `evaluate` was
  about 1% slower in most cases; three of them measured again, six processes each, were within
  0.7% of 0.19.0. With `maxSteps`, `node compare.mjs --max-steps N` gives the times without a
  limit, within 2.2%, plus what an options object costs. In 0.19.0 the options
  went through `JSON.stringify` and a parse, about 0.6 µs for `{}` and 1.1 µs for one option,
  so `path` against a `Document` took 1.3 µs without options, and 2.4 µs with `{maxSteps}` or
  `{timeout}`. From 0.20 the options are read from the object, by one call to a JavaScript
  helper that reads each option at its own site:

  | `path` against a `Document` | Through JSON | From the object |
  |---|---:|---:|
  | no options | 1.27 µs | 1.22 µs |
  | `{}` | 1.93 µs | 1.41 µs |
  | `{maxSteps}` | 2.44 µs | 1.61 µs |
  | `{timeout}` | 2.51 µs | 1.72 µs |
  | `{now: new Date(0)}` | 32.5 µs | 1.76 µs |
  | `{timeout, maxSteps, seed}` | 3.40 µs | 1.80 µs |

  A `Date` for `now` cost the most, as its string was parsed by an expression compiled on each
  call; it now gives its time directly, and a string is parsed by an expression compiled once.
- **In Python**, `python.py speed`, three runs of each: every case between 2.2% faster
  (`recursion`, `shipping rule`) and 2.2% slower, except `number formatting` at 2.6% slower
  (35.8 µs against 34.9 µs). That case runs no code that changed in 0.20 but the counter, and
  the gap depends on how it is measured: 1.6% in a loop of 200,000 evaluations, none under
  `perf` (38.0 µs in both), where samples move between functions whose code did not change, as
  code layout does. `max_steps` (`python.py speed --max-steps N`) costs about
  40 ns per call, about what `max_depth` costs: `path` against a `Document` takes 0.43 µs
  without options, 0.47 µs with `max_steps` or `max_depth`, and 0.49 µs with `timeout`.
  Until the 0.20.0 release, `python.py` passed `**limits` to each call even without a limit,
  which made each call build a tuple and a dictionary: about 0.3 µs more for both revisions
  alike (`path` 1.1 µs instead of 0.9 µs), so the comparisons above stand. The
  [Python table](#python-against-the-other-implementations) is measured without it.

A loop of small steps shows what the clock cost before. Measured on `$sum($map([1..300000],
function($x) { $x * 2 + 1 }))`, with a timeout of 60 s that never expires and a step limit of
10⁹ that is never reached (the median of 21 runs of `jlens eval`, and of 15 evaluations in
Node.js):

| | 0.19.0, without | 0.19.0, timeout | 0.20, without | 0.20, timeout | 0.20, step limit |
|---|---:|---:|---:|---:|---:|
| Native (`jlens eval`, whole process) | 96 ms | 157 ms (+63%) | 92 ms | 92 ms (+0.3%) | 91 ms (−0.3%) |
| WebAssembly (Node.js) | 107 ms | 264 ms (+145%) | 105 ms | 107 ms (+2.4%) | 106 ms (+1.1%) |

In WebAssembly the clock is `performance.now()`, a call out of the module, which made it the
larger cost there.

A regular expression matched under a timeout or a cancellation token checks it every 1,024
steps of the backtracker ([ADR 0009](../adr/0009-bounded-regex.md)): 1.7 % on average on the
[engine's benchmark](#the-regular-expression-engine), with its spread explained there.

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

### Documents built from Rust values

An application that keeps its state in Rust types builds its document with
`Document::from_serialize`, which serializes the state straight into it
([ADR 0019](../adr/0019-documents-from-rust-values.md)), rather than through a
`serde_json::Value`. The benchmark `host_state` (`cargo bench -p jlens-core --bench host_state`)
measures a state of ten records of twenty fields (strings, numbers, booleans, small lists),
rebuilt at each iteration, on this machine, for 0.21 with the release's other changes in:

| `host build`: building the document | Time |
|---|---:|
| `serde_json::to_value` alone, for reference | 21.6 µs |
| `to_value` + `Document::new` | 40.2 µs |
| `serde_json::to_string` + `Document::parse` | 35.0 µs |
| `Document::from_serialize` | 17.1 µs |

`host rebuild` builds the document and evaluates the first 1, 4 or 10 of ten small expressions
(filters, sorts, `[0]`) against it, as an application does between two changes of its state.
For comparison, the last two rows build no document: lazy `evaluate` over the
`serde_json::Value`, and `evaluate_source` over a `Source` written over the state, which reads
only the fields each expression reaches:

| `host rebuild` | 1 | 4 | 10 |
|---|---:|---:|---:|
| `to_value` + `Document::new` + `evaluate_document` | 40.7 µs | 49.6 µs | 69.7 µs |
| `Document::from_serialize` + `evaluate_document` | 18.6 µs | 27.2 µs | 46.9 µs |
| `to_value` + `evaluate` | 24.1 µs | 39.3 µs | 67.8 µs |
| `evaluate_source` | 3.7 µs | 19.7 µs | 53.7 µs |

For up to four evaluations per state, a `Source` is the fastest; for ten, a document built with
`from_serialize`.

`host binding` reads one of the ten records through a variable (`$self.score > 50 and
$self.active`). A variable of `Bindings` is a `serde_json::Value`, converted whole at each
evaluation; `evaluate_document_with` binds the record's part of the document
(`Document::pointer("/items/3")`), which is not converted:

| `host binding` | Time |
|---|---:|
| `$self` from `Bindings` | 3.39 µs |
| `$self` bound to `doc.pointer("/items/3")` | 0.42 µs |
| `items[3]` read from the document, for reference | 0.80 µs |

The rest of the library runs as fast as in 0.20. `cargo xtask ab v0.20.0 .`, with the 0.21
code complete and ten rounds, puts every case of `evaluate` between 3.3% faster (`names over
items`) and 1.5% slower (`group-by`), except `transform` at 3.1% slower, which was 0.7% slower
over 20 rounds. With `--mode document`, every case is between 1.0% faster and 1.9% slower,
except `path`, 16% faster (240 ns against 287 ns). That was not looked into: no code on its
path changed but the order in which variables and functions are bound.

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
  `evaluate`; in Python, 164.5 against 81.3 µs with Python values in).
- **Object constructors over one item.** An object constructor whose input is a single item,
  as in `items.{...}`, which evaluates it once per item, or the update of a transform,
  evaluates its pairs against the item instead of grouping: no group per key, no items to
  join. Its keys are still evaluated before its values, so the errors are those of a grouping
  (`T1003` for a key that is not a string, `D1009` for two pairs with the same key). In 0.19
  this made `object constructor` 22% faster with `evaluate` (399 against 312 µs) and 25%
  against a `Document` (376 against 281 µs), and `transform` 6% against a `Document`. In
  Python, the object constructor took 205 against 256 µs with Python values in, and 143
  against 188 µs against a `Document`, against 182 and 141 µs in `jsonatapy`.
- **`$formatNumber`.** Until 0.19 a call spent most of its time on work that does not
  depend on the number: it parsed and validated its picture again (about 1.3 µs). Rounding to
  the picture's decimals shifted the number through three heap strings, twice (0.55 µs).
  Detecting a tie wrote the number with 30 extra digits, and the result took four allocations.
  An evaluation now keeps the last eight pictures it parsed, by picture and options. Rounding
  shifts in a buffer on the stack, a tie is read from the bits of the number, and the result
  is written once. `number formatting` (20 calls) takes 45.1 against 91.9 µs with `evaluate`
  and 33.9 against 81.0 µs against a `Document`, and `$round` with decimals is about three
  times faster. In Python, it takes 45 against 75.9 µs with Python values in and 24.7 against
  53 µs against a `Document`, against 155 and 146 µs in `jsonatapy`. What remains of a call is mostly Rust's fixed-precision formatting of the
  digits (about 0.1 µs), the rounding (0.16 µs) and the string of its result.
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
