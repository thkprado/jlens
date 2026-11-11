# Performance

jlens is a tree-walking interpreter, like jsonata-js. This chapter says how fast it is against
the other JSONata engines on each platform, how that is measured, and how to use the API to get
the most from it.

## Summary

Measured for 0.33.0 on an Apple M1 ([below](#benchmarks)), on seventeen expressions over a 24 KB
order and an array of 10,000 items:

| Platform | Against the reference | Against the fastest other engine |
|---|---|---|
| Rust | `customer.name` in 0.10 µs, a 30-line rule in 42 µs against a `Document` | – |
| Node.js (WebAssembly) | faster than jsonata-js on 15 of 17 cases with `evaluate`, all 17 against a `Document`, by up to 520× | jsonata2js, which compiles to JavaScript, is faster on 14 of 17 |
| Python | fastest of the Python packages on 14 of 17 cases with Python values, 15 of 17 against a `Document`; 18 to 160× jsonata-python | jsonata2py leads on `names over items` and the index into 10,000 items, jsonatapy on that index once converted |
| Java | faster than jsonata-java on all 17 cases against a `Document`, the most evaluations per second from JSON text on any number of threads | jsonata-jvm-compiler, which compiles to bytecode, is faster on most cases over converted input |

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
[`docs/notes/`](https://github.com/thkprado/jlens/tree/main/docs/notes): `perf-0.33.md`, `perf-0.32.md`,
`perf-0.31.md`, `perf-0.30.md`, `perf-0.29.md`, `perf-0.28.md`, `perf-0.27.md`, `perf-0.26.md` and
`perf-earlier.md`.

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

The numbers below were measured for jlens 0.33.0 on a MacBook Air M1 (4+4 cores, 16 GB, no fan)
under macOS 27, with Rust 1.99, Node.js 24, Python 3.14.10 and OpenJDK 25.0.5; the system's
thermal pressure stayed nominal through the runs. The columns of other
engines whose version has not changed keep the numbers measured there for 0.31.0, the first
release measured on it, and the regular expression engine, which 0.33 did not change, those of
0.32.0.

A table that names an earlier release was measured for that release on an AMD Ryzen 7 3700X under
Windows 11 (Linux: Ubuntu 26.04 under WSL 2), where every table up to 0.30.0 came from, and is
not measured again: its absolute times are about twice the M1's. x86_64 has no tables of its
own. A release whose plan changed the evaluation is compared with the release before it on that
machine, natively and in Python under Linux, and what that finds is in the release's notes.

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
- Run one comparison at a time: two at once disturb each other, by 8% on the Ryzen.

A time also follows the memory a program's code was given, which stays with a file while it is
cached: six copies of one program differed by up to 5.6% on Windows, and by 10% on one case under
Linux. The new copy for each round turns that into spread, and a revision compared with itself
shows the floor of a comparison: most cases within 1.7%, the transform within 2.9% and the object
constructor within 4.1% on Windows; within 2.6% under Linux; within 0.8% on the M1.

Code placement moves cases too: a change can make a case that runs none of its code 5 to 8%
slower or faster, as the release profile (LTO, one codegen unit) lays the whole program out again.
To set that aside, build both programs with functions and blocks aligned, as the [engine's
benchmark](#the-regular-expression-engine) does, through `RUSTFLAGS`; `ab` names its programs by
commit, not flags, so delete the commit's program before an aligned run and after it.

The Python and Java packages are compared the same way with `python.py ab` and `cargo xtask
java-compare ab`, five rounds by default. `*` marks a difference on which no run of one build
reaches the other's, above 3.5% in Python and 5% in Java ([`benches/compare/README.md`](https://github.com/thkprado/jlens/blob/main/benches/compare/README.md#two-builds-of-a-package)).
The tables below come from one build each, so a change of a few percent from an earlier table can
be that build's; the release compares builds as above ([0.33 against 0.32.0](#033-against-0320)).

### Native (criterion)

The table is measured with `cargo xtask bench-table`, in about ten minutes. It builds the program
of `cargo bench -p jlens-core --bench evaluate` once, with the release profile and no alignment
flags, as an application builds it, runs a new copy of it three times, and prints each case's
time in each run and their median. A case where most runs are more than 2% from the median, or
whose runs fall in two groups more than 5% apart, is marked and run again with `--runs 5`. A case
more than 2% away from the previous table is compared with `cargo xtask ab <previous tag> .`,
built aligned, before the difference is put down to a change.

Each `evaluate` call reads its `serde_json::Value` input as the expression reaches it, one
object or array at a time ([ADR 0010](../adr/0010-lazy-input.md)); `evaluate_document` evaluates
against a [`Document`](#reusing-documents) converted beforehand. With macOS's system allocator:

| Case | `evaluate` | `evaluate_document` |
|---|---:|---:|
| path (`customer.name`) | 0.23 µs | 0.10 µs |
| names over items (`$sum(items.quantity)`) | 7.0 µs | 1.73 µs |
| filter + aggregate | 26 µs | 17.1 µs |
| group-by | 48 µs | 39 µs |
| sort + map | 163 µs | 147 µs |
| strings | 42 µs | 31 µs |
| regex | 66 µs | 58 µs |
| number formatting | 16.4 µs | 15.0 µs |
| object constructor | 84 µs | 74 µs |
| transform | 227 µs | 166 µs |
| recursion (`$f(15)`, Fibonacci) | 227 µs | 227 µs |
| shipping rule | 60 µs | 42 µs |
| index into 10,000 items (`items[0].code`) | 0.52 µs | 0.28 µs |
| index in a variable (`($i := 0; items[$i].code)`) | 0.65 µs | 0.41 µs |
| regex over a long text (20 KB) | 87 µs | 86 µs |
| distinct items | 50 µs | 10.8 µs |
| tuples (`items#$n@$i[...]`) | 75 µs | 68 µs |

Three copies, which agreed within 1.4%. On Windows, the system
allocator makes some cases draw one of two times for each process (`recursion` against a
`Document` 776 or 890 µs on the Ryzen); `mimalloc`, which the command and the packages use, does
not draw.

| Other measurements | Time |
|---|---:|
| Compile `customer.name` | 0.59 µs |
| Compile the shipping rule (30 lines) | 21 µs |
| `Document::parse` of the 24 KB document | 65 µs |
| `Document::new` from a `serde_json::Value` | 39 µs |
| `serde_json::from_str` of the same text, for reference | 114 µs |
| Shipping rule, audit level `decisions` | 769 µs (13× without a tracer) |
| Shipping rule, audit level `full` | 3.70 ms (62× without a tracer) |

Without a tracer, evaluation takes the shortcuts described under
[Where the time goes](#where-the-time-goes); with one it takes the general path, so that the
audit reports every node, hence the large ratios.

The path case reads one field of the input, `customer`, and one of that object, `name`:
`evaluate` takes 0.23 µs, where converting the whole input first took 39 µs. A path step reads
only the field it looks up
([ADR 0010, amended](../adr/0010-lazy-input.md#amendment-one-field-at-a-time-017)), so
`names over items` reads one field of each item and takes 7.0 µs. An expression that reads whole
items pays for converting them, and the allocator sets much of that cost; an application that
calls `evaluate` on whole documents can set a faster global allocator, as the `jlens` command
does.

### The allocator of the command

Evaluation allocates for most of its work, so the allocator sets much of its speed. The `jlens`
command uses [mimalloc](https://github.com/microsoft/mimalloc) as its global allocator on every
release target, and so do the Python and Java packages. `cargo xtask ab . . --b-allocator mimalloc`
measures the allocator alone (B is the same revision with `mimalloc`), and `--target
x86_64-unknown-linux-musl` builds both programs for musl, as the Linux release binaries are.
Measured for 0.17 on the Ryzen with `evaluate`: musl's own allocator is the slowest of the three, up to 5.6
times a glibc build, and `mimalloc` makes it 28 to 67% faster; on Windows, `mimalloc` saves 17 to
52% (the transform 1.18 ms against 604 µs); glibc's allocator is the fastest, and `mimalloc` still
saves 3 to 20% on ten cases of twelve. On macOS, the system allocator was not compared with it.
Until 0.26, every evaluation on musl's main thread ran on a new stack segment
([stacker#155](https://github.com/rust-lang/stacker/issues/155)); jlens works around it, and
`path` takes 339 ns there with `mimalloc`.

### WebAssembly against the other engines (Node.js)

All run in the same Node.js process, on the same input. The WebAssembly module is the release
build optimized by `wasm-opt -Os`, with link-time optimization since 0.27. The comparison script
first checks that each engine gives jsonata-js's result for each case, and all do. The jsonata-js
times include the `await` that its asynchronous `evaluate` requires. jsonata2js translates an
expression to JavaScript source and lets the JavaScript engine compile it.
[`benches/compare/README.md`](https://github.com/thkprado/jlens/blob/main/benches/compare/README.md)
says how each is called.

Measured for 0.33.0, the median of four runs, each on a new copy of the module, which agreed
within 4% but for `path`, printed to a tenth (2.3 to 2.5 µs with `evaluate`, 0.6 or 0.7 µs
against a `Document`); jsonata2js for 0.31.0, as its version has not changed since:

| Case | jsonata-js 2.2.2 | jsonata2js 0.1.2 | jlens-wasm `evaluate` | jlens-wasm `evaluateJson` | jlens-wasm `evaluateDocument` |
|---|---:|---:|---:|---:|---:|
| path | 1.3 µs | 0.2 µs | 2.5 µs | 71 µs | 0.7 µs |
| names over items | 41 µs | 3.2 µs | 29 µs | 98 µs | 3.7 µs |
| filter + aggregate | 400 µs | 8.8 µs | 89 µs | 145 µs | 27 µs |
| group-by | 661 µs | 13.8 µs | 125 µs | 172 µs | 55 µs |
| sort + map | 3.11 ms | 61 µs | 287 µs | 345 µs | 232 µs |
| strings | 366 µs | 11.1 µs | 126 µs | 184 µs | 52 µs |
| regex | 386 µs | 10.1 µs | 133 µs | 195 µs | 93 µs |
| number formatting | 338 µs | 75 µs | 49 µs | 129 µs | 24 µs |
| object constructor | 740 µs | 52 µs | 151 µs | 278 µs | 86 µs |
| transform | 791 µs | 204 µs | 355 µs | 484 µs | 193 µs |
| recursion | 5.47 ms | 42 µs | 362 µs | 427 µs | 356 µs |
| shipping rule | 1.08 ms | 27 µs | 191 µs | 201 µs | 64 µs |
| index into 10,000 items | 1.3 µs | 145 µs | 3.3 µs | 2.25 ms | 0.9 µs |
| index in a variable | 1.95 ms | 197 µs | 3.7 µs | 2.25 ms | 1.1 µs |
| regex over a long text | 320 µs | 135 µs | 178 µs | 180 µs | 155 µs |
| distinct items | 6.49 ms | 56 µs | 181 µs | 164 µs | 16.7 µs |
| tuples | 661 µs | 28 µs | 156 µs | 213 µs | 95 µs |

- `evaluate` is faster than jsonata-js on fifteen of the seventeen cases, by up to 15× on
  recursion, 36× on `distinct items`, whose `$distinct` is quadratic in jsonata-js, and 520× on
  the index held in a variable, which jsonata-js tests against every item. It is slower on
  `path` and the index into 10,000 items, which do almost no work, so the overhead of calling
  into WebAssembly dominates.
- Against a `Document` converted once, jlens-wasm is faster than jsonata-js on all seventeen
  cases: by 12 to 17× on recursion, group-by, sort + map, `filter + aggregate`, number formatting
  and the shipping rule, 390× on `distinct items`, 4.1× on the transform and 1.4× on the index
  into 10,000 items.
- jsonata2js is faster than `evaluate` on fourteen of the seventeen cases, and than
  `evaluateDocument` on twelve: by 1.1 times on `regex over a long text` and `names over items`
  to 9 times on `recursion` and `regex`. As compiled JavaScript, it reads the JavaScript objects
  it is given, with nothing to convert. jlens-wasm is faster on number formatting, the two indexes
  into 10,000 items, and, against a `Document`, `distinct items` and the transform. Called again
  on the same text, its `compile` takes 2 to 4 times what jlens's `new Expression` takes, which it
  makes up within a few evaluations.
- jsonata-js works on the JavaScript objects it is given, with nothing to convert. jlens-wasm
  `evaluate` reads the input object as the expression reaches it: up to 16 values in one call to a
  JavaScript helper, a long array of objects in one call with the fields the expression reads, and
  an index the items it picks alone. An expression that reads every item whole pays what
  converting the whole input does: `JSON.stringify` takes 30 µs, parsing it into a `Document` 135 µs.
- `evaluateJson` takes JSON text and returns JSON text, and parses the input without the fields
  the expression does not read ([ADR 0016](../adr/0016-fields-read.md)). Parsing still costs
  far more than a small evaluation: 71 µs on `path`, and 2.3 ms for the text of 10,000 items. An
  input evaluated more than once is best converted once into a `Document`.

gnata-js 0.6.2, the Go engine gnata built for WebAssembly, left the table after 0.27.0: it was
slower than jlens-wasm on every case.

### Python against the other implementations

`benches/compare/python.py` compares the Python package with three other JSONata packages on
PyPI: `jsonatapy`, the Python package of jsonata-core, a JSONata engine in Rust; `jsonata2py`,
which translates an expression to Python source and compiles it; and `jsonata-python`, a port of
jsonata-java in pure Python, by far the most downloaded. Each package needs a virtual environment
of its own, as several install a module named `jsonata`; the script's docstring says how to run
it. Each package gives jlens's result on each case but `regex over a long text`, where
`jsonata2py` and `jsonata-python` give 4,150, not the 9,858 of jsonata-js and jlens: Python's `\w`
matches the accented, Greek and Japanese letters that JavaScript's does not.

Measured for 0.33.0, on Python 3.14.10; `jsonatapy` and `jsonata2py` for 0.31.0, as their
versions have not changed since (`jsonatapy`, with wheels up to Python 3.13, built from its
source distribution). The input is the document as Python values, and for jlens and `jsonatapy`
also an input converted once (`Document`, `JsonataData`). jlens is the median of three runs,
which agreed within 2.4%; `jsonatapy` and `jsonata2py` of three; `jsonata-python` one run,
within 3% of the one for 0.32.0:

| Case | jsonata-python 0.7.1 | jsonatapy 2.2.14 | jsonatapy, `JsonataData` | jsonata2py 0.1.3 | jlens `evaluate` | jlens `evaluate_document` |
|---|---:|---:|---:|---:|---:|---:|
| path | 1.03 ms | 0.5 µs | 0.3 µs | 0.5 µs | 0.5 µs | 0.3 µs |
| names over items | 1.37 ms | 15.2 µs | 3.7 µs | 13.2 µs | 13.9 µs | 2.0 µs |
| filter + aggregate | 3.12 ms | 59 µs | 35 µs | 62 µs | 38 µs | 17.7 µs |
| group-by | 4.27 ms | 88 µs | 61 µs | 151 µs | 67 µs | 38 µs |
| sort + map | 15.9 ms | 310 µs | 229 µs | 266 µs | 170 µs | 129 µs |
| strings | 3.00 ms | 82 µs | 54 µs | 64 µs | 50 µs | 28 µs |
| regex | 3.17 ms | 91 µs | 74 µs | 158 µs | 66 µs | 50 µs |
| number formatting | 2.76 ms | 73 µs | 68 µs | 90 µs | 16.8 µs | 13.8 µs |
| object constructor | 3.96 ms | 153 µs | 124 µs | 182 µs | 88 µs | 61 µs |
| transform | 3.75 ms | 734 µs | 664 µs | 347 µs | 203 µs | 133 µs |
| recursion | 28.1 ms | 581 µs | 578 µs | 1.39 ms | 236 µs | 236 µs |
| shipping rule | 6.75 ms | 184 µs | 101 µs | 221 µs | 100 µs | 42 µs |
| index into 10,000 items | 20.3 ms | 178 µs | 0.4 µs | 0.7 µs | 0.8 µs | 0.5 µs |
| index in a variable | 35.2 ms | 624 µs | 437 µs | 1.29 ms | 0.9 µs | 0.6 µs |
| regex over a long text | differs | 97 µs | 97 µs | differs | 87 µs | 86 µs |
| distinct items | 1.46 ms | 496 µs | 402 µs | 307 µs | 79 µs | 10.6 µs |
| tuples | 4.19 ms | 266 µs | 240 µs | 277 µs | 83 µs | 64 µs |

- With Python values in, jlens is the fastest on fourteen of the seventeen cases: by 4.3 times
  on number formatting, 3.9 times on `distinct items`, 3.2 times on `tuples`, 2.5 times on
  `recursion`, 690 times on the index held in a variable, which `jsonatapy` and `jsonata2py` test
  against every item, and by 1.1 to 1.8 times on the rest. It is level with both on `path`
  (0.5 µs), and `jsonata2py` is ahead on `names over items` (13.2 against 13.9 µs) and the index
  into 10,000 items (0.7 against 0.8 µs).
- With the input converted once, jlens is the fastest on fifteen of the seventeen cases, by 4.9
  times on number formatting and 38 times on `distinct items`; it is level with `jsonatapy` on
  `path` (0.3 µs, printed to a tenth), and `jsonatapy` is faster on the index into 10,000 items
  (0.4 against 0.5 µs).
- `jsonata2py` reads the Python values it is given, with nothing to convert, as compiled Python.
  Besides `path` and the two cases where it leads, it takes 1.3 to 5.9 times what jlens
  `evaluate` takes, and 1,400 times on the index in a variable.
- `jsonata-python` takes 18 to 160 times what jlens `evaluate` takes on every case but `path` and
  the two indexes, and reads the whole input before evaluating: 1.0 ms on `path` and 20 ms on the
  index into 10,000 items.

`jsonata-rs` 0.1.4, a port of jsonata-java in Rust, left the tables after 0.27.0: it was slower
than jlens on every case, and crashed the interpreter on deep and cyclic input. `jsonata`, which
runs jsonata-js in the Duktape interpreter, is left out: no release since 2023, and its module
cannot load jsonata-js on Windows.

#### Limits in Python

How each behaves at the limits, measured by `python.py limits` for 0.27.0 under Linux, with the
packages of the table above. Each case runs in a process of its own, killed after 30 seconds. `$string($)`
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
time, over Jackson trees. [`benches/compare/README.md`](https://github.com/thkprado/jlens/blob/main/benches/compare/README.md)
says how to run it, and which packages it leaves out and why. jsonata-java gives jlens's result on
every case, jsonata-jvm-compiler on all but two: on `tuples` it gives arrays of the products where
jsonata-js gives their sums, and on `regex over a long text` 4,150, as its `\w` is Python's.

Measured for 0.33.0, on OpenJDK 25.0.5 (8 hardware threads), every engine in one JVM with
`-Xmx4g`. Each time is the best of five runs of 0.3 s after a second of warm-up, in the faster
of two JVMs, as jsonata-java ran six cases 10 to 19% slower in one of them. Each engine
returns its own objects, not serialized: jlens maps, lists and boxed values. With the input
converted once: Java maps for jsonata-java and `evaluate(Object)`, a Jackson tree for
jsonata-jvm-compiler, and a `Document` for `evaluate(Document)`:

| Case | jsonata-java 0.9.10 | jsonata-jvm-compiler 1.0.7 | jlens `evaluate(Object)` | jlens `evaluate(Document)` |
|---|---:|---:|---:|---:|
| path | 8.6 µs | 0.03 µs | 0.87 µs | 0.21 µs |
| names over items | 16.8 µs | 1.5 µs | 16.5 µs | 1.8 µs |
| filter + aggregate | 58 µs | 4.1 µs | 52 µs | 17.1 µs |
| group-by | 122 µs | 13.5 µs | 77 µs | 38 µs |
| sort + map | 426 µs | 37 µs | 170 µs | 129 µs |
| strings | 68 µs | 7.8 µs | 71 µs | 28 µs |
| regex | 111 µs | 12.5 µs | 70 µs | 49 µs |
| number formatting | 87 µs | 8.5 µs | 19.8 µs | 13.9 µs |
| object constructor | 101 µs | 7.3 µs | 96 µs | 60 µs |
| transform | 237 µs | 27 µs | 210 µs | 126 µs |
| recursion | 780 µs | 43 µs | 244 µs | 231 µs |
| shipping rule | 161 µs | 15.6 µs | 128 µs | 41 µs |
| index into 10,000 items | 146 µs | 0.03 µs | 1.2 µs | 0.34 µs |
| index in a variable | 634 µs | 34 µs | 1.3 µs | 0.44 µs |
| regex over a long text | 116 µs | differs | 114 µs | 86 µs |
| distinct items | 25 µs | 19.7 µs | 94 µs | 10.7 µs |
| tuples | 150 µs | differs | 97 µs | 63 µs |

- jsonata-jvm-compiler is the fastest on every case it agrees on but the index in a variable and
  `distinct items`, by 1.2 (`names over items`) to 8.2 times (`object constructor`) against a
  jlens `Document` on the cases that take a microsecond or more, 5.4 times on `recursion`: its
  expression is a Java class, which the JIT
  compiles with the application.
- Against jsonata-java, the other interpreter, jlens over a `Document` is faster on every case.
  It takes less than a third of the time on `recursion`, `sort + map` and the shipping rule,
  and less than a sixth on number formatting.
- Over Java maps, jlens takes 1.3 to 3.1 times as long as over a `Document` on the cases that
  read the order's items, and 9 times on `names over items` and `distinct items`: each value
  read is a call back into Java, though a map is written with only the fields the expression
  reads ([ADR 0021, amended](../adr/0021-java-package.md#amendment-maps-written-with-what-the-expression-reads-026-4)).
  jsonata-java is then faster on `distinct items`, which reads whole items, and level on
  `strings`.

From JSON text, parsed at each evaluation, by its own parser for jsonata-java, by Jackson for
jsonata-jvm-compiler, and by `evaluateFromJson` for jlens, which skips the fields the expression
does not read:

| Case | jsonata-java 0.9.10 | jsonata-jvm-compiler 1.0.7 | jlens `evaluateFromJson` |
|---|---:|---:|---:|
| path | 93 µs | 69 µs | 31 µs |
| names over items | 101 µs | 71 µs | 41 µs |
| filter + aggregate | 147 µs | 74 µs | 64 µs |
| group-by | 212 µs | 87 µs | 85 µs |
| sort + map | 516 µs | 108 µs | 174 µs |
| strings | 155 µs | 79 µs | 81 µs |
| regex | 194 µs | 84 µs | 90 µs |
| number formatting | 175 µs | 81 µs | 58 µs |
| object constructor | 188 µs | 79 µs | 108 µs |
| transform | 325 µs | 98 µs | 184 µs |
| recursion | 950 µs | 115 µs | 278 µs |
| shipping rule | 250 µs | 87 µs | 99 µs |
| index into 10,000 items | 1.42 ms | 1.33 ms | 885 µs |
| index in a variable | 2.02 ms | 1.35 ms | 894 µs |
| regex over a long text | 134 µs | differs | 115 µs |
| distinct items | 108 µs | 95 µs | 68 µs |
| tuples | 238 µs | differs | 111 µs |

jlens is the fastest where the parsing is the cost (`path`, `names over items`, `filter +
aggregate`, `group-by`, number formatting, `distinct items` and the two index cases);
jsonata-jvm-compiler where the evaluation is, 115 against 278 µs on `recursion`. jlens is faster
than jsonata-java on every case.

On the documents of ADR 0021's spike, 1,000 and 100,000 items of five fields each, an index or
`$count` takes the same time at both sizes in jlens, 0.3 to 0.5 µs against a `Document`, where
jsonata-java takes 57 µs and 7.3 ms for `items[0].name`. From JSON text, jlens takes about half the
time of the others on 100,000 items, but on `$`, for which it builds the whole document as Java
objects.

Evaluations per second of `$sum(items.price)` on the JSON text of 100 items (9.5 KB), on 1 to 8
platform threads (four of the M1's eight cores are efficiency cores) and on 1,000 virtual
threads, each engine parsing the text at each evaluation; jsonata-java, whose compiled
expressions are not thread-safe, compiles one per thread:

| Engine | 1 thread | 2 threads | 4 threads | 8 threads | 1,000 virtual threads |
|---|---:|---:|---:|---:|---:|
| jlens, JSON text | 44,602 (1.0×) | 85,476 (1.9×) | 163,943 (3.7×) | 213,107 (4.8×) | 213,838 (4.8×) |
| jlens, Java objects parsed by Jackson | 16,712 (1.0×) | 33,055 (2.0×) | 62,974 (3.8×) | 84,169 (5.0×) | 84,168 (5.0×) |
| jsonata-java 0.9.10 | 19,065 (1.0×) | 36,865 (1.9×) | 70,718 (3.7×) | 94,715 (5.0×) | 94,549 (5.0×) |
| jsonata-jvm-compiler 1.0.7 | 19,527 (1.0×) | 38,799 (2.0×) | 74,686 (3.8×) | 100,466 (5.1×) | 100,970 (5.2×) |

jlens leads at every count of threads, by 2.1 times or more. Parsed by Jackson into Java maps
first, the parse, not the evaluation, is the cost.

#### Limits on the JVM

Measured by `cargo xtask java-compare limits` under Linux on the Ryzen, with the cases of
`python.py limits`. Each case
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
Measured for 0.32.0 (the median of 31 samples of at least 5 ms; V8 13.6, Node.js 24.22):

| Case | Pattern | Matches | `jlens-regress` | With an interrupt | V8 |
|---|---|---:|---:|---:|---:|
| literal | `status=200` | 200 | 5.1 µs | 5.2 µs (+3.7%) | 11.5 µs |
| literal, no match | `status=500` | 0 | 507 ns | 510 ns (+0.5%) | 7.2 µs |
| case-insensitive literal | `lazy dog`, `i` | 200 | 6.4 µs | 6.6 µs (+2.5%) | 10.3 µs |
| classes | `\d{4}-\d{2}-\d{2}T[\d:]+Z` | 200 | 27.6 µs | 27.9 µs (+1.2%) | 16.2 µs |
| classes, no match | `\d{3}-\d{4}` | 0 | 3.5 µs | 3.5 µs (+0.0%) | 1.6 µs |
| alternation of words | `\b(?:fox\|dog\|cat)s?\b` | 400 | 129 µs | 131 µs (+1.6%) | 15.5 µs |
| words | `\b\w+\b` | 2,400 | 181 µs | 180 µs (−0.7%) | 79.5 µs |
| captures | `user=(\w+) id=(\d+)` | 200 | 28.0 µs | 27.8 µs (−1.1%) | 11.7 µs |
| lazy | `path=(.*?) ` | 200 | 35.8 µs | 35.7 µs (−0.3%) | 9.4 µs |
| lines | `^.*status=200$`, `m` | 200 | 29.4 µs | 29.2 µs (−0.6%) | 15.5 µs |
| backreference, no match | `(\w+)\s+\1` | 0 | 443 µs | 447 µs (+0.9%) | 151 µs |
| lookahead | `\w+(?=,)` | 200 | 423 µs | 416 µs (−1.8%) | 136 µs |
| lookbehind | `(?<=the )\w+` | 200 | 154 µs | 158 µs (+2.2%) | 53.1 µs |
| Unicode text | `\S+` | 2,000 | 157 µs | 159 µs (+1.5%) | 71.4 µs |
| Unicode text, case-insensitive | `γράμματα`, `i` | 200 | 21.0 µs | 21.8 µs (+3.8%) | 7.4 µs |
| Unicode property | `\p{L}+`, `u` | 2,000 | 216 µs | 230 µs (+6.5%) | 289 µs |
| anchored, short text | `^1[0-9]$` on `17` | 1 | 44 ns | 46 ns (+5.5%) | 39 ns |

| Compilation | Pattern | `jlens-regress` | V8 `new RegExp` | V8, and a first `exec` |
|---|---|---:|---:|---:|
| date-time | `^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?(Z\|[+-]\d{2}:\d{2})$` | 6.8 µs | 1.9 µs | 18.2 µs |
| e-mail, case-insensitive | `^[a-z0-9._%+-]+@(?:[a-z0-9-]+\.)+[a-z]{2,}$`, `i` | 7.6 µs | 1.3 µs | 28.2 µs |
| Unicode property | `\p{L}+`, `u` | 3.2 µs | 36.3 µs | 79.8 µs |

- V8 compiles a pattern to machine code, and is 1.7 to 8 times faster wherever the engine runs
  its instructions at many positions of the text: alternations, classes, lookarounds,
  backreferences. `jlens-regress` is a backtracker that interprets its instructions. It is
  faster where it can search for a literal first (`memchr`): 2.3 times on `literal`, 14 times
  on a literal that does not occur; and on `\p{L}+`.
- jlens compiles a pattern once per expression, when the expression is compiled. V8's
  `new RegExp` only parses the pattern; the first `exec` compiles it.
- **The interrupt** costs about 1.5% over these 17 cases (the geometric mean; 0.5% in a second
  pair of runs), from 1.8% faster to 6.5% slower per case, most of that spread from how the
  compiler allocates the backtracker's registers, not from the check itself.

Through jlens, `$match`, `$replace`, `$split` and `$contains` are linear in the text since 0.27,
and with a regex literal they build only what their result holds: on 20 KB of text with 1,558
matches of `/\w+/`, `$replace` takes 0.23 ms against 0.48 ms in jsonata-js, and `$match`, which
returns an object for each match, 0.81 ms against 0.36 ms (measured for 0.28 against a
`Document`, on the Ryzen). Since 0.31, three allocations of each match are left, those of its object; with
`mimalloc`, `$match` took 0.38 ms, so the gap left is the Windows heap.

### What the limits cost

Every evaluation counts its steps for the step limit ([ADR 0018](../adr/0018-step-limit.md)).
Without a limit the budget is too large to be spent, so a step costs the same comparison with or
without one. The timeout reads the clock at every 256th step rather than at every step, so a
timeout fires at most 256 steps late. On a loop of 300,000 small steps, a timeout that never
expires costs 0.3% natively and 2.4% in WebAssembly, where the clock is a call out of the module,
and a step limit that is never reached at most 1.1% (measured for 0.20). A regular expression
under a timeout or a cancellation token checks it every 1,024 steps of the backtracker
([ADR 0009](../adr/0009-bounded-regex.md)), the [engine's](#the-regular-expression-engine) interrupt.

## Using the API efficiently

- **Compile once.** `Expression::compile` (`new Expression` in JavaScript) parses and
  post-processes. A compiled expression is immutable: keep it, share it between threads (it is
  `Send + Sync`), and evaluate it as often as needed.
- **Reuse documents.** `evaluate` converts the parts of the input that the expression reads.
  For an expression that reads all of a 24 KB document, that costs about 40 µs natively, and
  about 160 µs in WebAssembly. To evaluate several expressions against one document, such as a
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
the narrower the reads, the faster the parse:

| Parse | `Document::parse` | `Document::parse_cut` |
|---|---:|---:|
| The 24 KB document, cut by what `names over items` reads | 65 µs | 37 µs |
| The same, cut by the union of every case (the transform reads it whole) | 65 µs | 65 µs |
| 1,000 records of 40 fields (1 MB), cut by what `$sum(records[status = 'open'].amount)` reads | 3.18 ms | 1.19 ms |

A rule that reads a document whole leaves nothing to skip, and then `parse_cut` costs what
`parse` does.

The WebAssembly and Python packages parse this way: `Document.fromJson` and `Document.from_json`
take a `Read`, and `evaluateJson` and `evaluate_json` cut their input by what the expression
reads. On the Ryzen, in Python, 1,000 records of 40 fields (790 KB) parse in 5.5 ms whole and
1.6 ms cut by the rule above, and `evaluate_json` evaluates it on them in 1.6 ms in all; in
Node.js, `evaluateJson` evaluates the rule on 1,000 such records (1.4 MB) in 5.7 ms.

`jlens eval` parses its input this way too, by what the expression reads. Measured end to end
on the Ryzen, process start included, before and after:

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
rebuilt at each iteration:

| `host build`: building the document | Time |
|---|---:|
| `serde_json::to_value` alone, for reference | 9.3 µs |
| `to_value` + `Document::new` | 19.4 µs |
| `serde_json::to_string` + `Document::parse` | 18.6 µs |
| `Document::from_serialize` | 9.3 µs |

`host rebuild` builds the document and evaluates the first 1, 4 or 10 of ten small expressions
(filters, sorts, `[0]`) against it, as an application does between two changes of its state.
For comparison, the last two rows build no document: lazy `evaluate` over the
`serde_json::Value`, and `evaluate_source` over a `Source` written over the state, which reads
only the fields each expression reaches:

| `host rebuild` | 1 | 4 | 10 |
|---|---:|---:|---:|
| `to_value` + `Document::new` + `evaluate_document` | 20.1 µs | 23.4 µs | 32.2 µs |
| `Document::from_serialize` + `evaluate_document` | 10.2 µs | 13.4 µs | 22.1 µs |
| `to_value` + `evaluate` | 10.9 µs | 16.7 µs | 29.9 µs |
| `evaluate_source` | 1.6 µs | 7.5 µs | 22.2 µs |

For up to four evaluations per state, a `Source` is the fastest; at ten, a document built with
`from_serialize` is level with it.

`host binding` reads one of the ten records through a variable (`$self.score > 50 and
$self.active`). A variable of `Bindings` is a `serde_json::Value`, of which each evaluation
converts [what the expression reads](#variables-against-a-document);
`evaluate_document_with` binds the record's part of the document
(`Document::pointer("/items/3")`), which is not converted:

| `host binding` | Time |
|---|---:|
| `$self` from `Bindings` | 0.31 µs |
| `$self` bound to `doc.pointer("/items/3")` | 0.15 µs |
| `items[3]` read from the document, for reference | 0.30 µs |

### Variables against a `Document`

An evaluation against a `Document` cannot read a variable lazily, as `evaluate_with` does: a lazy
value borrows the `Bindings`, and a document's values are `'static`
([ADR 0019](../adr/0019-documents-from-rust-values.md#variables-of-evaluate_document)). Since
0.27 it converts what [`reads()`](../adr/0016-fields-read.md#amendment-the-evaluation-reads-its-variables-by-the-analysis-027)
says the expression reads of each variable, analysed once for each expression; before, it
converted each variable whole.

A variable of 1,000 records of seven fields, about 130 KB, against a document of one field, on
the Ryzen, with functions and blocks aligned, in µs, the fastest of 15 rounds of one process:

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

- **Allocation.** Values are reference-counted (`Rc`). Objects keep their fields in insertion
  order, in a vector searched in order; one of more than 8 fields also keeps an index of its keys
  ([ADR 0022](../adr/0022-small-map.md)). Their keys are shared strings: converting a document
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
  seed of `$random`, a timeout), so `customer.name` against a `Document` takes 0.10 µs natively.
  From Python a call adds about 0.25 µs (its arguments, the interpreter released during the
  evaluation, the result built); from Java the C API and the call add about 0.12 µs, and the
  Ryzen's 16 threads on one `Document` made 20.8 million evaluations of `customer.name` per
  second.
- **Calls of lambdas.** A call of a lambda takes its frame and its arguments' list from those
  that a call before it gave back, and allocates them only when none is free or something still
  holds them, a closure for one; a name of up to 14 bytes stays in the frame's entry. A variable
  read in a lambda's body knows which parameter holds it. Host functions are
  looked up on a miss, so that binding sixteen of them costs nothing per evaluation
  ([ADR 0006](../adr/0006-host-functions.md)).
- **Deep copies.** A transform (`|...|`) copies its input with `$clone` before changing it, as
  jsonata-js does; jlens copies the value itself, with the values a copy through JSON text would
  give, where jsonata-js writes and parses the text. On the benchmark's transform the copy takes
  about a fifth of the time; most of the rest is building the result.
- **Long inputs.** `$distinct` keeps its first 16 distinct items by comparing, and looks the rest
  up by a hash, of the first values comparing reads when they tell the items apart, following
  them into the level below a field the items share: on the Ryzen, 100,000 strings take
  15 ms, where jsonata-js, quadratic, takes 32 s. `&` in a loop holds its two sides and makes its
  text once, as V8's ropes do. `$substring` walks the text only as far as its result. What `cargo
  xtask growth` still finds quadratic is so in any engine, as each step copies what it has built:
  `$reduce` that builds an array with `$append` or an object with `$merge`, and `in` over a long
  list for each item. An expression that does this over large input can be bounded with
  `max_steps` or `max_array_length`.
- **Link-time optimization.** Every release build uses LTO, the C API and the WebAssembly module
  since 0.27 (2 to 24% faster). Its larger frames make a deep recursion overflow V8's stack 4 to 9%
  sooner; the default `maxDepth` of 500 keeps a third of headroom ([WebAssembly guide](../guides/wasm.md#limits-and-memory)).

## 0.33 against 0.32.0

Measured for the release (roadmap 0.33 #12) on the M1, against builds of 0.32.0 made the same
way, each run on a new copy of its build; the full comparison is in `docs/notes/perf-0.33.md`.
0.33 changed nothing on the path of an evaluation: `jlens-core` is the same code, and the plan's
work is in the schema, its language service and the packages around them.

Natively every case is within 3.3% in each mode, `recursion` the furthest and faster, which is
what two builds of the same source differ by when the version in their text changes. Nothing is
marked in Python (-3.4 to +2.5%) or in Java (-3.2 to +3.5% on the cases; from JSON text, 1.7 to
1.9% fewer evaluations a second on one to four threads, and as many on eight). In WebAssembly,
whose module is 8% larger with what an expression uses of its schema, no case moves more than
jsonata-js, the control, does.

One evaluation is faster: from JSON text in the Java package, with bindings that hold host
functions, the input is parsed for the fields the functions' arguments read, where a call at the
top of the expression kept it whole. The 24 KB order then parses in 27 to 50 µs, against 69 µs.
