# Performance

jlens is a tree-walking interpreter, like jsonata-js. This chapter says how fast it is against
the other JSONata engines on each platform, how that is measured, and how to use the API to get
the most from it.

## Summary

Measured for 0.27.0 on one machine ([below](#benchmarks)), on seventeen expressions over a 24 KB
order and an array of 10,000 items:

| Platform | Against the reference | Against the fastest other engine |
|---|---|---|
| Rust | `customer.name` in 0.25 µs, a 30-line rule in 109 µs against a `Document` | – |
| Node.js (WebAssembly) | faster than jsonata-js on 15 of 17 cases with `evaluate`, all 17 against a `Document`, by up to 440× | jsonata2js, which compiles to JavaScript, is faster on 14 of 17 |
| Python | fastest of the Python packages on 13 of 17 cases with Python values, 14 of 17 against a `Document`; 16 to 150× jsonata-python | jsonatapy and jsonata2py lead on `path`, the index into 10,000 items, and a long text |
| Java | faster than jsonata-java on 16 of 17 cases against a `Document`, the most evaluations per second from JSON text on any number of threads | jsonata-jvm-compiler, which compiles to bytecode, is faster on most cases over converted input |

- jlens is fastest where the expression reads a little of a large input (an index, a path) and
  where an input converted once into a `Document` serves many evaluations.
- The engines that compile an expression to the host language's code (jsonata2js, jsonata2py,
  jsonata-jvm-compiler) are faster on most cases that do much work per item. jlens is an
  interpreter, and the same on every platform.
- On deep, cyclic or endless input, jlens fails with a JSONata error code and never ends the
  process ([Python](#limits-in-python), [Java](#limits-on-the-jvm)).

The numbers are meant for comparing alternatives on one machine. They are not guarantees, and
your hardware will give different absolute values. How each change was measured (methods, traps,
intermediate numbers) is kept outside the book, in
[`docs/notes/`](https://github.com/thkprado/jlens/tree/main/docs/notes): `perf-0.27.md`,
`perf-0.26.md` and `perf-earlier.md`.

## Benchmarks

The workload is in `benches/` at the repository root. It is shared by the native benchmarks and
by the comparisons with jsonata-js, with the other Python packages and with the JVM
implementations:
- `benches/cases.json` lists seventeen expressions: path navigation, a name mapped over the
  items, filters with aggregation, grouping, sorting, string and regex functions, number
  formatting, an object constructor, a transform, a recursive function, the shipping rule from
  the guides, two indexes into a long array (one of them held in a variable), a regex over a
  20 KB text, `$distinct` over the items, and a path with tuples.
- `benches/orders-200.json` is a 24 KB store order with 200 items, the input of most cases.
- `benches/items-10000.json` is an array of 10,000 small items, the input of the two index
  cases, `items[0].code` and `($i := 0; items[$i].code)`: they cost little only when an index
  does not touch the other items, which 200 items would hide.
- `benches/growth.json` lists the cases of `cargo xtask growth`, which times each built-in
  function and operator at 1,000 to 64,000 items and prints the exponent of its growth, 1 for
  linear and 2 for quadratic ([where the time goes](#where-the-time-goes)).
- `crates/jlens-core/benches/host_state.rs` builds a document from Rust values again and again,
  as an application that keeps its state in Rust types does
  ([below](#documents-built-from-rust-values)). It is native only.

```sh
cargo bench -p jlens-core                     # criterion, native
cargo xtask bench-table                       # the native table below: the median of three runs
cargo xtask ab main .                         # two revisions against each other, native
cargo xtask wasm && cd benches/compare && npm install && node compare.mjs   # against jsonata-js
python benches/compare/python.py speed jlens=<python> ...   # against the Python packages
python benches/compare/python.py ab A=<python> B=<python>   # two builds of the Python package
cargo xtask java-compare                      # against the JVM implementations (benches/compare/README.md)
cargo xtask java-compare ab <A.jar> <B.jar>   # two builds of the Java package
cargo bench -p jlens-regress --bench engine   # the regular expression engine alone (below)
cargo xtask growth                            # how each built-in's time grows with its input
```

The numbers below were measured on an AMD Ryzen 7 3700X under Windows 11 (Linux: Ubuntu 26.04
under WSL 2, on the same machine), with Rust 1.99 and Node.js 24, for jlens 0.27.0.

### Comparing two revisions

To measure a change, compare the revision before it with the revision that makes it. Criterion
alone can mislead here: on some machines its results are bimodal, so two runs of one build can
differ by more than the change. `cargo xtask ab` is built for this:

```sh
cargo xtask ab main .                         # main against the working tree
cargo xtask ab v0.9.0 HEAD --case regex --case strings --rounds 20
cargo xtask ab main . --mode document         # evaluate_document instead of evaluate
cargo xtask ab main . --mode source           # evaluate_source instead of evaluate
```

- A revision is anything git names, or `.` for the working tree. Each is built once with the
  release profile, in a temporary worktree, and kept in `target/ab/bin`.
- `--mode` picks how the cases read their input: a `serde_json::Value` (`evaluate`, the
  default), a `Document` converted beforehand (`document`), or a `Source` written in the
  program, one level at a time, as the Python and Java packages read their values (`source`).
- It runs the two programs in alternation, ten rounds by default (`--rounds`), A then B, then B
  then A, so that a drift of the machine reaches both, each on a new copy of its program. It
  prints the median of the rounds for each case and how much slower B is. Judge by the median.
- Run one comparison at a time: two at once disturb each other, by 8% on the machine above.

A time also follows the memory a program's code was given, which stays with a file while it is
cached: six copies of one program differed by up to 5.6% on Windows, and by 10% on one case under
Linux. The new copy for each round turns that into spread, and a revision compared with itself
shows the floor of a comparison: most cases within 1.7%, the transform within 2.9% and the object
constructor within 4.1% on Windows; within 2.6% under Linux.

Code placement moves cases too: a change can make a case that runs none of its code 5 to 8%
slower or faster, because the release profile (LTO, one codegen unit) lays the whole program out
again. To set that aside, build both programs with functions and blocks aligned, as the
[engine's benchmark](#the-regular-expression-engine) does, by setting `RUSTFLAGS` for the run.
The programs that `ab` keeps are named by commit, not by flags: delete the commit's program
before an aligned run, and again after it.

The Python and Java packages are compared the same way, wheel against wheel and JAR against
JAR, with `python.py ab` and `cargo xtask java-compare ab`, each run on a new copy of the native
library, five rounds by default. `*` marks a difference on which no run of one build reaches the
other's, above 3.5% in Python and 5% in Java, past what a build shows against itself
([`benches/compare/README.md`](https://github.com/thkprado/jlens/blob/main/benches/compare/README.md#two-builds-of-a-package)).
The WebAssembly, Python and Java tables below come from one build each, so that a change of a few
percent from an earlier table can be that build's; the release compares the packages with the
previous release by the commands above ([0.27 against 0.26.0](#027-against-0260)).

### Native (criterion)

The table is measured with `cargo xtask bench-table`, in about ten minutes. It builds the program
of `cargo bench -p jlens-core --bench evaluate` once, runs a new copy of it three times, and
prints, for each benchmark, the time criterion gives in each run and their median, which sets a
run that is far off aside. A case where most runs are more than 2% from the median is marked and
run again with `--runs 5`, and so is a case whose runs fall in two groups, more than 5% apart.

The programs are built with the release profile, without the alignment flags, as an
application builds them. A case more than 2% away from the previous table is compared with
`cargo xtask ab <previous tag> .`, built aligned, before the difference is put down to a change.

Each `evaluate` call reads its `serde_json::Value` input as the expression reaches it, one
object or array at a time ([ADR 0010](../adr/0010-lazy-input.md)). `evaluate_document`
evaluates against a [`Document`](#reusing-documents) converted beforehand.

| Case | `evaluate` | `evaluate_document` |
|---|---:|---:|
| path (`customer.name`) | 0.63 µs | 0.25 µs |
| names over items (`$sum(items.quantity)`) | 17.5 µs | 5.9 µs |
| filter + aggregate | 63 µs | 43 µs |
| group-by | 102 µs | 83 µs |
| sort + map | 414 µs | 388 µs |
| strings | 111 µs | 86 µs |
| regex | 175 µs | 160 µs |
| number formatting | 36 µs | 33 µs |
| object constructor | 245 µs | 221 µs |
| transform | 487 µs | 436 µs |
| recursion (`$f(15)`, Fibonacci) | 791 µs | 791 µs |
| shipping rule | 145 µs | 109 µs |
| index into 10,000 items (`items[0].code`) | 1.34 µs | 0.71 µs |
| index in a variable (`($i := 0; items[$i].code)`) | 1.68 µs | 1.05 µs |
| regex over a long text (20 KB) | 347 µs | 349 µs |
| distinct items | 130 µs | 16.4 µs |
| tuples (`items#$n@$i[...]`) | 208 µs | 192 µs |

The table is the median of five copies, as three runs of the first three copies were the
machine's, 25 to 44% slow. Two cases can draw one of two times with Windows's system allocator,
which places small blocks where the heap happens to leave room: `recursion` against a `Document`
790 or 890 µs, and `path` against a `Document` 0.23 or 0.43 µs. The table shows the fast draw;
`mimalloc`, which the command and the packages use, does not draw.

| Other measurements | Time |
|---|---:|
| Compile `customer.name` | 1.1 µs |
| Compile the shipping rule (30 lines) | 42 µs |
| `Document::parse` of the 24 KB document | 174 µs |
| `Document::new` from a `serde_json::Value` | 112 µs |
| `serde_json::from_str` of the same text, for reference | 210 µs |
| Shipping rule, audit level `decisions` | 1.73 ms (12× without a tracer) |
| Shipping rule, audit level `full` | 7.92 ms (55× without a tracer) |

Without a tracer, evaluation takes the shortcuts described under
[Where the time goes](#where-the-time-goes); with one it takes the general path, so that the
audit reports every node, hence the large ratios.

The path case reads one field of the input, `customer`, and one of that object, `name`:
`evaluate` takes 0.63 µs, where converting the whole input first took 115 µs. A path step reads
only the field it looks up
([ADR 0010, amended](../adr/0010-lazy-input.md#amendment-one-field-at-a-time-017)), so
`names over items` reads one field of each item and takes 17.5 µs. An expression that reads whole
items pays for converting them. With Windows's default allocator, that costs more than it does
with a faster one; an application that calls `evaluate` on whole documents can set a faster
global allocator, as the `jlens` command does.

### The allocator of the command

Evaluation allocates for most of its work, so the allocator sets much of its speed. The `jlens`
command uses [mimalloc](https://github.com/microsoft/mimalloc) as its global allocator on every
release target, and so do the Python and Java packages. `cargo xtask ab . . --b-allocator mimalloc`
measures the allocator alone: B is the same revision with `mimalloc`.
`--target x86_64-unknown-linux-musl` builds both programs for musl, as the Linux release binaries
are.

Measured for 0.17 with `evaluate`, the median of ten rounds of `ab`:

- musl's own allocator is the slowest of the three, up to 5.6 times a glibc build
  (`names over items` 77.8 against 14.0 µs); `mimalloc` makes it 28 to 67% faster.
- On Windows, `mimalloc` saves 17 to 52% (the transform 1.18 ms against 604 µs).
- glibc's allocator is the fastest; `mimalloc` still saves 3 to 20% on ten cases of twelve.
- The 11 µs that `path` took on musl were a fixed cost of each evaluation on the main thread,
  until 0.26: musl reports the main thread's stack as the part mapped so far, so that every
  evaluation ran on a new stack segment ([stacker#155](https://github.com/rust-lang/stacker/issues/155)).
  jlens works around it, and `path` takes 339 ns there with `mimalloc`.
- macOS was not measured. Its binaries use `mimalloc` too.

### WebAssembly against the other engines (Node.js)

All run in the same Node.js process, on the same input. The WebAssembly module is the release
build optimized by `wasm-opt -Os`, with link-time optimization since 0.27. The comparison script
first checks that each engine gives jsonata-js's result for each case, and all do. The jsonata-js
times include the `await` that its asynchronous `evaluate` requires. jsonata2js translates an
expression to JavaScript source and lets the JavaScript engine compile it.
[`benches/compare/README.md`](https://github.com/thkprado/jlens/blob/main/benches/compare/README.md)
says how each is called.

Measured for 0.27.0 at 083b6d2, the median of three runs, which agreed within 9% but for
jsonata-js's `names over items`, `sort + map` and jsonata2js's `distinct items` (12 to 19%):

| Case | jsonata-js 2.2.2 | jsonata2js 0.1.2 | jlens-wasm `evaluate` | jlens-wasm `evaluateJson` | jlens-wasm `evaluateDocument` |
|---|---:|---:|---:|---:|---:|
| path | 2.2 µs | 0.3 µs | 4.3 µs | 89 µs | 1.3 µs |
| names over items | 62 µs | 5.8 µs | 51 µs | 161 µs | 9.7 µs |
| filter + aggregate | 622 µs | 13 µs | 136 µs | 236 µs | 43 µs |
| group-by | 1.02 ms | 21 µs | 184 µs | 284 µs | 89 µs |
| sort + map | 4.81 ms | 80 µs | 449 µs | 540 µs | 358 µs |
| strings | 577 µs | 17 µs | 188 µs | 291 µs | 80 µs |
| regex | 632 µs | 17 µs | 214 µs | 317 µs | 151 µs |
| number formatting | 553 µs | 97 µs | 89 µs | 213 µs | 39 µs |
| object constructor | 1.19 ms | 83 µs | 271 µs | 449 µs | 179 µs |
| transform | 1.34 ms | 327 µs | 648 µs | 773 µs | 375 µs |
| recursion | 7.67 ms | 59 µs | 677 µs | 770 µs | 678 µs |
| shipping rule | 1.63 ms | 44 µs | 284 µs | 352 µs | 119 µs |
| index into 10,000 items | 2.2 µs | 181 µs | 6.2 µs | 4.00 ms | 1.8 µs |
| index in a variable | 3.02 ms | 208 µs | 6.8 µs | 4.02 ms | 2.2 µs |
| regex over a long text | 466 µs | 178 µs | 356 µs | 358 µs | 331 µs |
| distinct items | 9.24 ms | 66 µs | 292 µs | 270 µs | 24 µs |
| tuples | 1.01 ms | 39 µs | 286 µs | 396 µs | 201 µs |

- `evaluate` is faster than jsonata-js on fifteen of the seventeen cases, by up to 11× on
  recursion, 32× on `distinct items`, whose `$distinct` is quadratic in jsonata-js, and 440× on
  the index held in a variable, which jsonata-js tests against every item. It is slower on
  `path` and the index into 10,000 items, which do almost no work, so the overhead of calling
  into WebAssembly dominates.
- Against a `Document` converted once, jlens-wasm is faster than jsonata-js on all seventeen
  cases: by 11 to 14× on recursion, group-by, sort + map, number formatting and the shipping
  rule, 380× on `distinct items`, 3.6× on the transform and 22% on the index into 10,000 items.
- jsonata2js is faster than `evaluate` on fourteen of the seventeen cases, and than
  `evaluateDocument` on thirteen: by 1.15 times on the transform to 11 times on `recursion`. As
  compiled JavaScript, it reads the JavaScript objects it is given, with nothing to convert.
  jlens-wasm is faster on number formatting, the two indexes into 10,000 items and `distinct
  items` against a `Document`. Called again on the same text, its `compile` takes 2 to 4 times
  what jlens's `new Expression` takes, which it makes up within a few evaluations.
- jsonata-js works on the JavaScript objects it is given, so it never pays for converting the
  input. jlens-wasm `evaluate` reads the input object as the expression reaches it: an object or
  array of up to 16 values through one call to a JavaScript helper, a long array of objects in
  one call with the fields the expression reads of each item, and an index the items it picks
  alone. An expression that reads every item whole pays what converting the whole input does:
  `JSON.stringify` of the input takes 41 µs, and parsing that text 219 µs.
- `evaluateJson` takes JSON text and returns JSON text, and parses the input without the fields
  the expression does not read ([ADR 0016](../adr/0016-fields-read.md)). Parsing still costs
  far more than a small evaluation: 89 µs on `path`, and 4.0 ms for the text of 10,000 items. An
  input evaluated more than once is best converted once into a `Document`.

gnata-js 0.6.2, the Go engine gnata built for WebAssembly, which takes and returns JSON text,
was measured for 0.27.0 and left the table after it: it was slower than jlens-wasm on every
case, by 4.6 to 25 times against `evaluate` and 1.2 to 6 times against `evaluateJson`.

### Python against the other implementations

`benches/compare/python.py` compares the Python package with three other JSONata packages on
PyPI: `jsonatapy`, the Python package of jsonata-core, a JSONata engine in Rust; `jsonata2py`,
which translates an expression to Python source and compiles it; and `jsonata-python`, a port of
jsonata-java in pure Python, by far the most downloaded. Several JSONata packages install a
module named `jsonata`, so each package needs a virtual environment of its own; the script's docstring says
how to run it. It first checks that each package gives jlens's result for each case. All do but
on `regex over a long text`, where `jsonata2py` and `jsonata-python` give 4,150, not the 9,858 of
jsonata-js and jlens: Python's `\w` matches the accented, Greek and Japanese letters that
JavaScript's does not.

Measured for 0.27.0 (at 083b6d2), on Python 3.14.8 under Linux. `jsonatapy` has wheels up to
Python 3.13 and was built from its source distribution. The input is the document as Python
values; for jlens and `jsonatapy`, also an input converted once (`Document`, `JsonataData`).
jlens, `jsonatapy` and `jsonata2py` are the median of three runs, which agreed within 8% but for
`path` and `names over items` of `jsonatapy` and `jsonata2py` (14%); `jsonata-python` one run:

| Case | jsonata-python 0.7.1 | jsonatapy 2.2.14 | jsonatapy, `JsonataData` | jsonata2py 0.1.3 | jlens `evaluate` | jlens `evaluate_document` |
|---|---:|---:|---:|---:|---:|---:|
| path | 1.48 ms | 0.7 µs | 0.4 µs | 0.7 µs | 0.9 µs | 0.5 µs |
| names over items | 1.93 ms | 28 µs | 7.4 µs | 20 µs | 23 µs | 5.6 µs |
| filter + aggregate | 4.60 ms | 94 µs | 56 µs | 93 µs | 66 µs | 40 µs |
| group-by | 6.07 ms | 136 µs | 84 µs | 222 µs | 111 µs | 73 µs |
| sort + map | 25.1 ms | 517 µs | 352 µs | 372 µs | 285 µs | 233 µs |
| strings | 4.44 ms | 123 µs | 80 µs | 95 µs | 81 µs | 54 µs |
| regex | 5.02 ms | 155 µs | 114 µs | 231 µs | 116 µs | 91 µs |
| number formatting | 4.30 ms | 172 µs | 164 µs | 155 µs | 28 µs | 23 µs |
| object constructor | 6.08 ms | 213 µs | 156 µs | 251 µs | 156 µs | 118 µs |
| transform | 5.82 ms | 1.09 ms | 1.02 ms | 470 µs | 360 µs | 251 µs |
| recursion | 41.5 ms | 902 µs | 918 µs | 1.86 ms | 557 µs | 565 µs |
| shipping rule | 9.73 ms | 258 µs | 144 µs | 328 µs | 168 µs | 96 µs |
| index into 10,000 items | 28.5 ms | 276 µs | 0.6 µs | 1.0 µs | 1.4 µs | 0.8 µs |
| index in a variable | 49.0 ms | 895 µs | 597 µs | 1.93 ms | 1.6 µs | 1.0 µs |
| regex over a long text | differs | 165 µs | 166 µs | differs | 212 µs | 210 µs |
| distinct items | 1.96 ms | 841 µs | 673 µs | 448 µs | 124 µs | 15 µs |
| tuples | 5.92 ms | 368 µs | 319 µs | 401 µs | 162 µs | 139 µs |

- With Python values in, jlens is the fastest on thirteen of the seventeen cases: by 5.5 times
  on number formatting, 3.6 times on `distinct items`, 2.3 times on `tuples`, 560 times on the
  index held in a variable, which `jsonatapy` and `jsonata2py` test against every item, and by
  1.2 to 1.6 times on the rest. `jsonata2py` is ahead on `path` (0.7 against 0.9 µs), `names over
  items` (20 against 23 µs) and the index into 10,000 items (1.0 against 1.4 µs), and `jsonatapy`
  on `regex over a long text` (165 against 212 µs).
- With the input converted once, jlens is the fastest on fourteen of the seventeen cases, by 7
  times on number formatting and 45 times on `distinct items`; `jsonatapy` is faster on `path`,
  the index into 10,000 items (0.6 against 0.8 µs) and `regex over a long text`.
- `jsonata2py` reads the Python values it is given, with nothing to convert, as compiled Python.
  Besides the three cases where it leads, it takes 1.2 to 5.5 times what jlens `evaluate` takes,
  and 1,200 times on the index in a variable.
- `jsonata-python` takes 16 to 150 times what jlens `evaluate` takes on every case but `path` and
  the two indexes, and reads the whole input before evaluating: 1.5 ms on `path` and 28 ms on the
  index into 10,000 items.

`jsonata-rs` 0.1.4, a port of jsonata-java in Rust, was measured for 0.27.0 and left the tables
after it: it took 1.3 to 33 times what jlens `evaluate` takes on every case but `path` and the two
indexes, where it read the whole input first (210 µs and 4.3 ms), and it crashed the interpreter
on input nested 100,000 levels and on a cycle. The fifth package, `jsonata`, which runs
jsonata-js in the Duktape JavaScript interpreter, is left out: it has had no release since 2023,
its module cannot load jsonata-js on Windows, and it compiles the expression at each call.

#### Limits in Python

How each behaves at the limits, measured by `python.py limits` for 0.27.0, with the packages of
the table above. Each case runs in a process of its own, killed after 30 seconds. `$string($)`
reads the whole input, and the cyclic input is `{"n": 1, "child": {"parent": <the input>}}`:

| Case | jsonata-python | jsonatapy | jsonata2py | jlens |
|---|---|---|---|---|
| Input nested 900 levels, `$string($)` | result | result | `U1001` | result |
| Input nested 2,000 levels | result | result | `U1001` | `U1001` |
| Input nested 100,000 levels | `RecursionError` | result | `U1001` | `U1001` |
| Cyclic input, `n` | `RecursionError` | `1` | `1` | `1` |
| Cyclic input, `$string($)` | `RecursionError` | runs on | `U1001` | `U1004` |
| Unbounded recursion, `1 + $f($n + 1)` | `RecursionError` | `U1001` | `U1001` | `D1011` |
| The same, with a depth limit of 500 | `D1011` | `D1011` | no such option | `D1011` |
| Endless tail call, `$f($n + 1)` | runs on | runs on | `U1001` after 0.09 s | runs on |
| The same, with a timeout of 1 s | `D1012` after 1.0 s | `D1012` after 1.0 s | `U1001` after 0.09 s | `D1012` after 1.0 s |

- jlens fails with a JSONata error in each case, and never ends the process. It refuses input
  nested more than 1,000 levels, which the others but `jsonata2py` accept as long as their stack
  or Python's recursion limit lasts: a fixed limit that fails the same way on every platform, at
  the cost of inputs between 1,000 levels and that point.
- jlens, `jsonatapy` and `jsonata2py` meet a cycle only when the expression reaches it;
  `jsonata-python` reads the whole input first, so `n` fails too.
- `jsonata2py` stops each case with `U1001`, "stack overflow", within 0.1 s. Its limits are
  low: probed on Windows, it accepts input nested up to 495 levels, a recursion such as
  `1 + $f($n + 1)` up to 99 calls deep, and loops of 100,000 tail calls but not of 1,000,000.
- No package stops an endless evaluation by default: set a timeout.

### Java against the other implementations

`cargo xtask java-compare` compares the Java package with two other JSONata implementations for
the JVM on Maven Central: jsonata-java, a port of jsonata-js to Java objects, and
jsonata-jvm-compiler, which translates an expression to Java and compiles it with `javac` at run
time, over Jackson trees.
[`benches/compare/README.md`](https://github.com/thkprado/jlens/blob/main/benches/compare/README.md)
says how to run it, and which packages it leaves out and why. It first checks that each gives
jlens's result for each case: jsonata-java does on all of them, jsonata-jvm-compiler on all but
two: on `tuples` it gives arrays of the products where jsonata-js gives their sums, and on
`regex over a long text` 4,150 rather than 9,858, as its `\w` matches accented, Greek and
Japanese letters, as Python's does.

Measured for 0.27.0 (at 083b6d2), on OpenJDK 25.0.4 under Linux (16 hardware threads), in one
JVM with `-Xmx4g`. Each time is the best of five runs of 0.3 s after a second of warm-up. Each
engine returns its result as its own objects, not serialized: jlens as maps, lists and boxed
values. With the input converted once: Java maps for jsonata-java and for jlens's
`evaluate(Object)`, a Jackson tree for jsonata-jvm-compiler, and a `Document` for
`evaluate(Document)`:

| Case | jsonata-java 0.9.10 | jsonata-jvm-compiler 1.0.7 | jlens `evaluate(Object)` | jlens `evaluate(Document)` |
|---|---:|---:|---:|---:|
| path | 7.1 µs | 0.05 µs | 1.4 µs | 0.35 µs |
| names over items | 23 µs | 1.7 µs | 31 µs | 5.5 µs |
| filter + aggregate | 113 µs | 4.7 µs | 91 µs | 40 µs |
| group-by | 225 µs | 13 µs | 131 µs | 74 µs |
| sort + map | 767 µs | 49 µs | 286 µs | 230 µs |
| strings | 121 µs | 9.1 µs | 115 µs | 54 µs |
| regex | 179 µs | 18 µs | 129 µs | 90 µs |
| number formatting | 122 µs | 13 µs | 31 µs | 22 µs |
| object constructor | 187 µs | 8.4 µs | 172 µs | 115 µs |
| transform | 410 µs | 37 µs | 393 µs | 241 µs |
| recursion | 1.50 ms | 36 µs | 548 µs | 547 µs |
| shipping rule | 299 µs | 20 µs | 210 µs | 98 µs |
| index into 10,000 items | 188 µs | 0.06 µs | 1.8 µs | 0.60 µs |
| index in a variable | 962 µs | 42 µs | 2.1 µs | 0.81 µs |
| regex over a long text | 187 µs | differs | 243 µs | 208 µs |
| distinct items | 27 µs | 20 µs | 167 µs | 15 µs |
| tuples | 259 µs | differs | 187 µs | 139 µs |

- jsonata-jvm-compiler is the fastest on every case it agrees on but the index in a variable and
  `distinct items`, by 1.65 (number formatting) to 15 times (`recursion`) against a jlens
  `Document`: its expression is a Java class, which the JIT compiles with the application.
- Against jsonata-java, the other interpreter, jlens over a `Document` is faster on every case
  but `regex over a long text` (208 against 187 µs, which jsonata-java runs with Java's regular
  expressions). It takes about a third of the time on `recursion`, `sort + map` and the shipping
  rule, and less than a fifth on number formatting.
- Over Java maps, jlens takes 1.2 to 5.6 times as long as over a `Document` on the cases that
  read the order's items, and 11 times on `distinct items`, which reads whole items: each value
  read is a call back into Java, though a map is written with only the fields the expression
  reads ([ADR 0021, amended](../adr/0021-java-package.md#amendment-maps-written-with-what-the-expression-reads-026-4)).
  jsonata-java is then faster on three of the seventeen cases.

From JSON text, parsed at each evaluation, by its own parser for jsonata-java, by Jackson for
jsonata-jvm-compiler, and by `evaluateFromJson` for jlens, which skips the fields the expression
does not read:

| Case | jsonata-java 0.9.10 | jsonata-jvm-compiler 1.0.7 | jlens `evaluateFromJson` |
|---|---:|---:|---:|
| path | 178 µs | 109 µs | 31 µs |
| names over items | 171 µs | 114 µs | 75 µs |
| filter + aggregate | 265 µs | 118 µs | 131 µs |
| group-by | 399 µs | 127 µs | 168 µs |
| sort + map | 962 µs | 164 µs | 319 µs |
| strings | 278 µs | 124 µs | 149 µs |
| regex | 332 µs | 132 µs | 158 µs |
| number formatting | 270 µs | 124 µs | 107 µs |
| object constructor | 340 µs | 120 µs | 210 µs |
| transform | 550 µs | 146 µs | 357 µs |
| recursion | 1.63 ms | 145 µs | 579 µs |
| shipping rule | 484 µs | 132 µs | 209 µs |
| index into 10,000 items | 2.49 ms | 2.07 ms | 1.94 ms |
| index in a variable | 3.26 ms | 2.17 ms | 1.96 ms |
| regex over a long text | 225 µs | differs | 244 µs |
| distinct items | 174 µs | 139 µs | 130 µs |
| tuples | 424 µs | differs | 233 µs |

jlens is the fastest where the parsing is the cost (`path`, `names over items`, number
formatting, `distinct items` and the two index cases); jsonata-jvm-compiler where the evaluation
is, 145 against 579 µs on `recursion`. jlens is faster than jsonata-java on every case but
`regex over a long text`.

On the documents of ADR 0021's spike, 1,000 and 100,000 items of five fields each, an index or
`$count` takes the same time at both sizes in jlens, about 0.6 µs against a `Document`, where
jsonata-java takes 85 µs and 36 ms for `items[0].name`. From JSON text, jlens takes about half the
time of the others on 100,000 items, but on `$`, for which it builds the whole document as Java
objects.

Evaluations per second of `$sum(items.price)` on the JSON text of 100 items (9.5 KB), on 1 to 16
platform threads and on 1,000 virtual threads, each engine parsing the text at each evaluation;
jsonata-java, whose compiled expressions are not thread-safe, compiles one per thread:

| Engine | 1 thread | 2 threads | 4 threads | 8 threads | 16 threads | 1,000 virtual threads |
|---|---:|---:|---:|---:|---:|---:|
| jlens, JSON text | 27,173 (1.0×) | 51,712 (1.9×) | 98,953 (3.6×) | 173,776 (6.4×) | 241,992 (8.9×) | 241,879 (8.9×) |
| jlens, Java objects parsed by Jackson | 10,388 (1.0×) | 19,734 (1.9×) | 38,470 (3.7×) | 68,912 (6.6×) | 87,269 (8.4×) | 83,588 (8.0×) |
| jsonata-java 0.9.10 | 11,779 (1.0×) | 23,168 (2.0×) | 43,172 (3.7×) | 70,457 (6.0×) | 79,416 (6.7×) | 83,661 (7.1×) |
| jsonata-jvm-compiler 1.0.7 | 12,515 (1.0×) | 24,054 (1.9×) | 45,737 (3.7×) | 77,929 (6.2×) | 107,835 (8.6×) | 106,258 (8.5×) |

jlens leads at every count of threads. Parsed by Jackson into Java maps first, the parse, not the
evaluation, is the cost.

#### Limits on the JVM

Measured by `cargo xtask java-compare limits`, with the cases of `python.py limits`. Each case
runs in a JVM of its own, on its main thread with the default stack, killed after 30 seconds.
jlens reads Java objects (`evaluate(Object)`), jsonata-java maps, and jsonata-jvm-compiler a
Jackson tree, built the same way, since text cannot hold a cycle:

| Case | jsonata-java 0.9.10 | jsonata-jvm-compiler 1.0.7 | jlens |
|---|---|---|---|
| Input nested 900 levels, `$string($)` | result | result | result |
| Input nested 2,000 levels | result | result | `U1001` |
| Input nested 100,000 levels | `StackOverflowError` | `StackOverflowError` | `U1001` |
| Cyclic input, `n` | `StackOverflowError` | `1` | `1` |
| Cyclic input, `$string($)` | `StackOverflowError` | `StackOverflowError` | `U1004` |
| Unbounded recursion, `1 + $f($n + 1)` | `StackOverflowError` | `U1001` | `D1011` |
| The same, with a depth limit of 500 | `JException` (stack overflow, depth 501) | no such option | `D1011` |
| Endless tail call, `$f($n + 1)` | runs on | `U1001` after 0.6 s | runs on |
| The same, with a timeout of 1 s | `JException` (timeout) after 1.0 s | `U1001` after 0.6 s | `D1012` after 1.2 s |

- jlens fails with a JSONata error code in each case but the endless loop with no timeout. No
  JVM crashed: a `StackOverflowError` is thrown to the caller, who must catch an `Error` rather
  than an exception to survive it, and it can leave an application's own state half updated.
- jsonata-jvm-compiler turns a stack overflow into `U1001`. It has no tail calls, so the endless
  tail call stops there too, and so would a loop of 100,000 tail calls. It has no depth limit.
- jsonata-java takes a depth limit and a timeout (`Frame.setRuntimeBounds`), and reports them
  with messages, not codes.

### The regular expression engine

`jlens-regress`, jlens's fork of `regress` ([ADR 0020](../adr/0020-own-regress-fork.md)), has a
benchmark of its own, `crates/jlens-regress/benches/engine.rs`, whose cases are in
`engine.json` beside it. Each case finds every match of a pattern in a text, as jlens does: from
the end of the previous match, without an interrupt, or with one (when the evaluation has a
timeout or a cancellation token). The texts are 200 lines of a log (15 KB), a sentence of English
repeated to 12 KB, and a sentence of Portuguese, Greek and Japanese repeated to 20 KB of UTF-8.
`engine.mjs` runs the same cases in Node.js, with `exec` and the flag `g`, as jsonata-js does.

Each variant is a binary of its own, built with functions and blocks aligned, in a target
directory of its own, since two copies of the backtracker in one binary are inlined differently:

```sh
export RUSTFLAGS="-C llvm-args=-align-all-functions=6 -C llvm-args=-align-all-nofallthru-blocks=5"
export CARGO_TARGET_DIR=target/aligned
cargo bench -p jlens-regress --bench engine            # without an interrupt
JLENS_REGRESS_INTERRUPT=1 cargo bench -p jlens-regress --bench engine
node crates/jlens-regress/benches/engine.mjs           # V8
```

Each run saves its times and prints what the check costs against the last run of the other
variant. A change to the engine is measured the same way, aligned, against the revision before.
Measured for 0.23 (the median of 31 samples of at least 5 ms; V8 13.6, Node.js 24.19):

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
- jlens compiles a pattern once per expression, when the expression is compiled. V8's
  `new RegExp` only parses the pattern; the first `exec` compiles it.
- **The interrupt** costs between −0.2 and 0.8% over these 17 cases (the geometric mean), from
  8.1% faster to 13.5% slower per case, most of that spread from how the compiler allocates the
  backtracker's registers, not from the check itself.

Through jlens, `$match`, `$replace`, `$split` and `$contains` are linear in the text since 0.27,
and with a regex literal they build only what their result holds: on 20 KB of text with 1,558
matches of `/\w+/`, `$replace` takes 0.36 ms against 0.58 ms in jsonata-js, and `$match`, which
returns an object for each match, 1.09 ms against 0.48 ms.

### What the limits cost

Every evaluation counts its steps for the step limit ([ADR 0018](../adr/0018-step-limit.md)).
Without a limit the budget is too large to be spent, so a step costs the same comparison with or
without one. The timeout reads the clock at every 256th step rather than at every step, so a
timeout fires at most 256 steps late. On a loop of 300,000 small steps, a timeout that never
expires costs 0.3% natively and 2.4% in WebAssembly, where the clock is a call out of the module,
and a step limit that is never reached at most 1.1% (measured for 0.20). A regular expression matched under a timeout or
a cancellation token checks it every 1,024 steps of the backtracker
([ADR 0009](../adr/0009-bounded-regex.md)), the interrupt of the
[engine's benchmark](#the-regular-expression-engine).

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
`evaluateJson` evaluates the rule on 1,000 such records (1.4 MB) in 5.7 ms.

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
rebuilt at each iteration, on this machine, for 0.21:

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
$self.active`). A variable of `Bindings` is a `serde_json::Value`, of which each evaluation
converts [what the expression reads](#variables-against-a-document);
`evaluate_document_with` binds the record's part of the document
(`Document::pointer("/items/3")`), which is not converted:

| `host binding` | Time |
|---|---:|
| `$self` from `Bindings` | 0.84 µs |
| `$self` bound to `doc.pointer("/items/3")` | 0.42 µs |
| `items[3]` read from the document, for reference | 0.80 µs |

### Variables against a `Document`

An evaluation against a `Document` cannot read a variable lazily, as `evaluate_with` does: a lazy
value borrows the `Bindings`, and a document's values are `'static`
([ADR 0019](../adr/0019-documents-from-rust-values.md#variables-of-evaluate_document)). Since
0.27 it converts what [`reads()`](../adr/0016-fields-read.md#amendment-the-evaluation-reads-its-variables-by-the-analysis-027)
says the expression reads of each variable, analysed once for each expression; before, it
converted each variable whole.

A variable of 1,000 records of seven fields, about 130 KB, against a document of one field, with
functions and blocks aligned, in µs, the fastest of 15 rounds of one process:

| Expression | Before 0.27 | 0.27 | `evaluate_with` |
|---|---:|---:|---:|
| `$v.total` | 1,050 | 0.64 | 0.57 |
| `$v.owner.name` | 1,050 | 0.99 | 0.92 |
| `$v.items[3].price` | 1,050 | 233 | 1.77 |
| `$sum($v.items.price)` | 1,090 | 260 | 87 |
| `a + 1`, the variable not read | 1,050 | 0.21 | 0.59 |
| `$count($keys($v))`, read whole | 1,050 | 1,058 | 2.2 |
| `$self.hp > 10`, five fields | 1.55 | 0.68 | 0.52 |
| `a > 10`, five fields not read | 1.51 | 0.21 | 0.49 |
| `$x + a`, a number | 0.315 | 0.328 | 0.41 |
| `a + 1`, no variables | 0.199 | 0.202 | 0.30 |

What is still slower than a lazy read is an array read for a few of its items:
`$v.items[3].price` converts `price` of all 1,000 items. A variable read whole costs what it did.
To read a large variable for a few of its parts, bind a part of a `Document` instead
(`evaluate_document_with` and `Document::pointer`, above), which converts nothing. The Python and
Java packages evaluate this way too: from Java, `$v.total` takes 0.83 µs where it took 630.

## Where the time goes

- **Allocation.** Values are reference-counted (`Rc`). Objects are `IndexMap`s that keep
  insertion order, hashed with `foldhash`. Their keys are shared strings: converting a document
  allocates each distinct key once, not once per object. Building results, sequences and tuple
  streams allocates, and that is most of the cost of evaluation.
- **Sequences.** Following jsonata-js, paths build sequences step by step and flatten them.
  Predicates evaluate per item, in a fresh frame. A step that reads an array passes it on as it
  is, and a step whose results hold no array to flatten makes them its sequence without copying.
- **Shortcuts for common shapes.** Without a tracer, evaluation skips work that no result
  depends on. A path of names on a single object (`customer.name`, `$i.code`) looks its fields
  up instead of building sequences; a name mapped over the items of a path is looked up in each
  item; a predicate built from literals and variables (`[[0..19]]`, `[$i]`) is evaluated once and
  picks its items; an object constructor over one item evaluates its pairs without grouping; `=`
  with a string literal compares with the literal's text. With a tracer, evaluation takes the
  general path, so audits report every node; a property test checks that both paths agree.
- **Converting the input.** `evaluate` converts the parts of the input that the expression
  reads, as it reads them ([ADR 0010](../adr/0010-lazy-input.md)): a path step reads only the
  field it looks up, and an index only the items it picks. Through a `Source`, as the Python and
  Java packages read their values, each array or object read costs one allocation, and freeing
  it as much again. An expression that reads whole items pays for converting them.
- **Fixed cost.** An evaluation reads the clock only when it needs it (`$now`, `$millis`, the
  seed of `$random`, a timeout), so `1` takes about 0.12 µs natively and `customer.name` against
  a `Document` 0.23 µs. From Python a call adds about 0.2 µs (its arguments, the interpreter
  released during the evaluation, the result built); from Java the C API and the call add about
  0.1 µs, and 16 threads on one `Document` make 20.8 million evaluations of `customer.name` per
  second.
- **Calls of lambdas.** A call of a lambda allocates its frame and its arguments' list, two
  allocations in all; a name of up to 14 bytes stays in the frame's entry. Host functions are
  looked up on a miss, so that binding sixteen of them costs nothing per evaluation
  ([ADR 0006](../adr/0006-host-functions.md)).
- **Deep copies.** A transform (`|...|`) copies its input with `$clone` before changing it, as
  jsonata-js does; jlens copies the value itself, with the values a copy through JSON text would
  give, where jsonata-js writes and parses the text. On the benchmark's transform the copy takes
  about 85 of 447 µs; most of the rest is building the result.
- **Long inputs.** `$distinct` keeps its first 16 distinct items by comparing, and looks the rest
  up by a hash, of an object's first fields when they tell the items apart: 100,000 strings take
  15 ms, where jsonata-js, quadratic, takes 32 s. `&` in a loop holds its two sides and makes its
  text once, as V8's ropes do. `$substring` walks the text only as far as its result. What `cargo
  xtask growth` still finds quadratic is so in any engine, as each step copies what it has built:
  `$reduce` that builds an array with `$append` or an object with `$merge`, and `in` over a long
  list for each item. An expression that does this over large input can be bounded with
  `max_steps` or `max_array_length`.
- **Link-time optimization.** Since 0.27 the C API's library, under the Java package, and the
  WebAssembly module are built with LTO, as the command and the wheels were: from Java, the cases
  against a `Document` take 2 to 17% less time than in 0.26.0, and in WebAssembly every case 2 to
  24% less, the module 4% smaller. The inlining makes the evaluator's frames larger, so that a
  deep recursion overflows V8's stack 4 to 9% sooner; the default `maxDepth` of 500 keeps a third
  of headroom ([WebAssembly guide](../guides/wasm.md#limits-and-memory)).

## 0.27 against 0.26.0

Measured for the release (roadmap 0.27 #19), each run on a new copy of its build, against builds
of 0.26.0 made the same way on the same machine. A difference is read against
[what a build shows against itself](#comparing-two-revisions). The full comparison is in
`docs/notes/perf-0.27.md`.

**Natively**, on Windows, `cargo xtask ab v0.26.0 HEAD`, ten rounds, in each mode, built as an
application builds it (two comparisons) and aligned (one). The medians of the cases 0.27 changed:

| Case | `evaluate` | `document` | `source` |
|---|---:|---:|---:|
| `regex over a long text` | 56.5 ms → 351 µs, −99.4% | 56.4 ms → 346 µs, −99.4% | 56.7 ms → 347 µs, −99.4% |
| `distinct items` | 618 → 127 µs, −79.3 to −79.8% | 494 → 15.9 µs, −96.8% | 629 → 133 µs, −78.4 to −79.0% |
| `tuples` | 258 → 207 µs, −19.5 to −20.3% | 245 → 196 µs, −19.9 to −21.5% | 268 → 220 µs, −17.1 to −18.4% |
| number formatting | 44.5 → 35.8 µs, −18.4 to −19.6% | +1.3 to +1.9% | 46.8 → 40.3 µs, −13.8 to −14.7% |
| `regex` | −9.5 to −11.9% | −10.7 to −11.6% | −10.3 to −11.1% |

`strings` is −4.1 to −8.1% in all nine, `sort + map` −2.9 to −5.3%, the index in a variable −1.9
to −6.8%. Of the other 90 numbers, none is more than 2.5% slower but two, both against a
`Document` and both within what a build shows against itself.

**In Python**, under Linux, `python.py ab`: `regex over a long text` −99.6%, `distinct items`
−75% with Python values and −96% against a `Document`, number formatting −18.5 to −20.2% with
Python values, `regex` and `tuples` −4.3 to −9.1%. `group-by` against a `Document` takes about
4.5% more, natively too, from dropping the arrays of each group: dropping a value is a call since
0.27, and dropping them by hand instead made the index cases 4 to 5% slower.

**In Java**, under Linux, `java-compare ab`, built with LTO: `regex over a long text` −99.5%,
`distinct items` −57 to −96%, number formatting over Java maps −41% and `tuples` −5 to −12%; the
cases that were slower before LTO are 2 to 5% faster than 0.26.0.

WebAssembly has no measured floor, and is compared in the release's table alone: its cells are
faster than 0.26.0's by up to 16% with `evaluate` and 25% against a `Document`.
