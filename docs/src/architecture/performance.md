# Performance

jlens is a tree-walking interpreter, like jsonata-js. This chapter covers three things: how
performance is measured, what the numbers are, and how to use the API to get the most from it.

## Benchmarks

The workload is in `benches/` at the repository root. It is shared by the native benchmarks and
by the comparisons with jsonata-js, with the other Python packages and with the JVM
implementations:
- `benches/cases.json` lists fourteen expressions: path navigation, a name mapped over the
  items, filters with aggregation, grouping, sorting, string and regex functions, number formatting, an object constructor, a
  transform, a recursive function, the shipping rule from the guides, and two indexes into a
  long array, one of them held in a variable.
- `benches/orders-200.json` is a 24 KB store order with 200 items, the input of every case but
  the last two.
- `benches/items-10000.json` is an array of 10,000 small items, the input of the last two
  cases, `items[0].code` and `($i := 0; items[$i].code)`: they cost little only when an index
  does not touch the other items, which 200 items would hide.
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
```

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

- `--mode` picks how the cases read their input. `evaluate` (the default) reads a
  `serde_json::Value`, converting the levels the expression reaches. `document` reads a
  `Document` converted once, before the measurement. `source` reads the `serde_json::Value`
  through a `Source` written in the program, one level at a time, as the Python package reads
  its `dict`s and `list`s: the arrays and objects of a level are members of one `Sources`, a
  path step reads one field of an object alone, and an index one item of an array (for a
  revision that has `Source::read_item`, 0.24). It measures what a level read through a `Sink`
  costs, which the packages for other languages pay on every evaluation.

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

A time also follows the file. Six copies of one program, the same bytes, differ by up to 5.6%
(`path`) and 4.9% (`recursion`), the medians of sixteen runs of each copy, where one file run
under six labels differs by less than 1%, `transform` aside (3.6%). Windows gives an image its
address once per file, not once per process, but that is not the whole cause: with the address
fixed by the file's header, one copy of six ran `names over items` 8.6% slower.

Linux (Ubuntu 26.04 under WSL 2, on the same machine, 0.26 #15) shows what remains. A program
has a new address in every process there, and copies differ all the same: of twelve copies, one
ran `index into 10,000 items` 10.4% and `index in a variable` 8.5% slower in all of its sixteen
runs, another `strings` 5.0% slower, where one file under six labels differs by 1.5% at most,
`transform` aside (2.6%). With four copies measured in eight sets of runs, the file's pages
dropped from the page cache between sets, so that its path, inode and bytes stay and its memory
is new, the slowness comes and goes between the sets of one file, by about 5% in
`index in a variable`, and the files come together: `strings` from 2.2% with the pages kept to
0.9% and 0.2%, `path` from 2.2% to 0.5% and 0.2%. So a time follows the memory the code was
given, which stays with a file while it is cached. Which part of the processor sees it was not observed; on Windows
the memory is inferred, by elimination. It would explain why one wheel measured 425, 449 and
456 µs on `transform` at three times of one day.

So each run of `ab` has a new copy of its program, which turns this into spread, and the median
of the rounds settles it: on Windows, with new copies, the six medians differ by 1.5% at most;
under WSL by 1.9% at most, `strings` aside (2.9%) and but for one label of `index in a variable`,
6.1% off, half of whose new copies ran slowly in turn. A script made those copies, deleting the
one before, which `ab` does not, and may have been given the same memory again; that was not
observed. A revision compared with itself then shows the noise of a comparison, which it did not
before 0.26, when both sides ran one file. On the machine below, with ten rounds, eleven
comparisons in the three modes: most cases agree within 1.7%, `transform` within 2.9% and the
object constructor within 4.1%. Under WSL, nine comparisons: most within 1.7%, the largest 2.6%
(`recursion`).

The Python and Java packages are compared the same way, wheel against wheel and JAR against
JAR, with `python.py ab` and `cargo xtask java-compare ab`, each run on a new copy of the native
library. The runs alternate, five rounds by default, and each prints the median of each number,
the difference and the spread, to the nanosecond. `*` marks a difference on which no run of one
build reaches the other's, above 3.5% in Python and 5% in Java, past what a build shows against
itself: nine numbers in ten within 2.1% and 2.8% on Windows and 2.6% and 2.8% under WSL, the
largest 3.2% and 6.2%, and 2.9% and 4.8%. With those marks, nothing is marked on either system
([`benches/compare/README.md`](https://github.com/thkprado/jlens/blob/main/benches/compare/README.md#two-builds-of-a-package)).
An unmarked difference is one the runs do not settle. The native table below has a new copy for
each run as well. The WebAssembly, Python and Java tables come from one file each, so that a
change of a few percent from an earlier table can be that file's; the release compares the Python
and Java packages with the previous release by the commands above.

Code placement moves cases too: a change can make a case that runs none of its code 5 to 8%
slower or faster, because the release profile (LTO, one codegen unit) lays the whole program out
again. To set that aside, build both programs with functions and blocks aligned, as the engine's
benchmark below does, by setting `RUSTFLAGS` for the run. The programs that `ab` keeps in
`target/ab/bin` are named by commit, not by flags: delete the commit's program before an aligned
run, and again after it, so that neither kind of run reuses the other's.

The numbers below were measured on an AMD Ryzen 7 3700X under Windows 11, with Rust 1.99 and
Node.js 24, for jlens 0.26.0. They are meant
for comparing alternatives on one machine. They are not guarantees, and your hardware will give
different absolute values.

### Native (criterion)

The table is measured with `cargo xtask bench-table`, in about ten minutes. It builds the program
of `cargo bench -p jlens-core --bench evaluate` once, runs a new copy of it three times, keeping
every copy until the last run, and prints, for each benchmark, the time criterion gives in each
run and their median. One run alone can be far off: for 0.25.0, one run gave 1.19 ms on `regex`
against a `Document`, where the next two gave 179 and 182 µs, and later one gave 1.99 ms on
`recursion` where the next two gave 1.03 ms. The median sets such a run aside when the other two
agree. A case where most runs are more than 2% from the median is marked, and run again with
`--runs 5`. For 0.26.0, three copies gave `recursion` against a `Document` 785, 882 and 885 µs,
which the mark does not catch, as two runs agree; the table below is the median of five copies,
884 to 907 µs. `cargo xtask ab`, which runs one case at a time, gives 800 µs for that build; why
the benchmark's program takes longer there was not looked for.

The copies settle what a median of one file cannot, the file ([above](#comparing-two-revisions)).
Eight tables of one commit, alternated, on the machine below (0.26 #16), each compared with the
others of its method, for the 42 cases:

| Each table from | Nine differences in ten within | Largest | Cases over 3% in some pair |
|---|---:|---:|---:|
| one file, three runs, as before 0.26 | 3.1% | 8.9% | 12 |
| three new copies | 1.8% | 3.2% | 2 |
| five new copies | 1.7% | 3.7% | 1 |

Three copies are kept: two more add six minutes for little, as the slow runs of `wide
Document::parse`, up to 7% off in the fourth and fifth runs of three tables, came in a row across
copies, so that they are the machine's, not a file's. Two tables of one commit can still differ
by 3%, so that a move of that size from the previous table goes to `ab`, as below.

The programs are built with the release profile, without the alignment flags, as an
application builds them. Aligned, the same code measured up to 7% faster (`path` against a
`Document`), most cases by 0 to 3%. Unaligned, a case can move from one release to the next by
code placement alone ([above](#comparing-two-revisions)). A case more than 2% away from the
previous table is compared with `cargo xtask ab <previous tag> .`, built aligned, before the
difference is put down to a change.

Each `evaluate` call reads its `serde_json::Value` input as the expression reaches it, one
object or array at a time ([ADR 0010](../adr/0010-lazy-input.md)). `evaluate_document`
evaluates against a [`Document`](#reusing-documents) converted beforehand.

| Case | `evaluate` | `evaluate_document` |
|---|---:|---:|
| path (`customer.name`) | 0.62 µs | 0.23 µs |
| names over items (`$sum(items.quantity)`) | 18.2 µs | 6.1 µs |
| filter + aggregate | 63 µs | 44 µs |
| group-by | 102 µs | 82 µs |
| sort + map | 428 µs | 400 µs |
| strings | 117 µs | 95 µs |
| regex | 196 µs | 179 µs |
| number formatting | 44 µs | 33 µs |
| object constructor | 247 µs | 221 µs |
| transform | 486 µs | 442 µs |
| recursion (`$f(15)`, Fibonacci) | 792 µs | 899 µs |
| shipping rule | 146 µs | 108 µs |
| index into 10,000 items (`items[0].code`) | 1.32 µs | 0.70 µs |
| index in a variable (`($i := 0; items[$i].code)`) | 1.77 µs | 1.10 µs |

| Other measurements | Time |
|---|---:|
| Compile `customer.name` | 1.1 µs |
| Compile the shipping rule (30 lines) | 41 µs |
| `Document::parse` of the 24 KB document | 176 µs |
| `Document::new` from a `serde_json::Value` | 115 µs |
| `serde_json::from_str` of the same text, for reference | 208 µs |
| Shipping rule, audit level `decisions` | 1.69 ms (12× without a tracer) |
| Shipping rule, audit level `full` | 7.95 ms (54× without a tracer) |

An audit costs about what it did in 0.8 (1.95 ms and 7.56 ms; level `full` 5% more), but the
ratios are much larger: without a tracer, evaluation takes the shortcuts described under
[Where the time goes](#where-the-time-goes), and with one it takes the general path, so that
the audit reports every node.

The path case reads one field of the input, `customer`, and one of that object, `name`:
`evaluate` takes 0.62 µs, where converting the whole input first took 115 µs (91 µs to
convert, 17 µs to drop, 0.9 µs to evaluate). A path step reads only the field it looks up
([ADR 0010, amended](../adr/0010-lazy-input.md#amendment-one-field-at-a-time-017)), so
`names over items` reads one field of each item and takes 18.2 µs, against 114 µs in 0.16. An
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
- `path` cost about 11 µs more on musl with either allocator, until 0.26: a fixed cost of each
  evaluation on the main thread, which the allocator did not explain (below).
- macOS was not measured. Its binaries use `mimalloc` too, so that every platform runs the same
  allocator.

#### The main thread on musl (0.26)

The fixed cost on musl came from the stack, not from the allocator. To grow the stack on
demand, `stacker` asks `pthread_getattr_np` where the thread's stack ends, once per thread. For
the main thread, glibc answers with the limit the kernel grows the stack to (`RLIMIT_STACK`,
8 MB by default); musl answers with the part mapped so far. After a shallow start that left
125 KB, less than the 128 KiB that `stack::grow` keeps free, so every evaluation ran on a new
4 MiB stack segment, mapped and unmapped each time. Other threads were not affected: musl
knows the size they were created with.

Now the first thread that reaches `stack::grow`, when it is the main thread and `RLIMIT_STACK`
is at least 4 MiB, uses 1 MiB of its stack first, which the kernel maps and musl then counts.
Deeper recursion still runs on new segments. The pages used stay resident: the process keeps
about 1 MiB more memory, where a new segment was mapped and unmapped at each evaluation. Only
the first thread is looked at, so that later calls cost an atomic load: a thread-local, which
a library loaded at run time reaches through a call, made group-by 2% slower from Java. A
process whose main thread evaluates only after another thread has keeps the cost there. The
cause is [stacker#155](https://github.com/rust-lang/stacker/issues/155), still open at 0.26.0, with
no answer to the cause and measurements given there. [stacker#156](https://github.com/rust-lang/stacker/pull/156),
open too, answers `None` only when the stack pointer is outside the bounds it has, which musl's
main thread is not, so that it would leave this as it is (read from its diff at 3b927ba, not
run). `stacker` 0.1.25 is its newest release, so that jlens keeps this workaround and its
musl-only `rustix` dependency. A fix there that reads the size from `RLIMIT_STACK` would replace
it; one that made every call on such a thread answer `None` would need another, as `maybe_grow`
takes that as no room left.

`cargo xtask ab` before and after (0.26 #8), `evaluate`, the median of ten rounds, musl in WSL
on the machine above, and the same revision with `mimalloc`:

| Case | musl, before | musl, after | + mimalloc |
|---|---:|---:|---:|
| path | 11.4 µs | 594 ns | 339 ns |
| names over items | 71.0 µs | 55.2 µs | 10.6 µs |
| filter + aggregate | 134 µs | 110 µs | 45.9 µs |
| group-by | 258 µs | 228 µs | 80.4 µs |
| sort + map | 600 µs | 568 µs | 266 µs |
| strings | 201 µs | 173 µs | 65.1 µs |
| regex | 249 µs | 221 µs | 101 µs |
| number formatting | 119 µs | 94.6 µs | 27.5 µs |
| object constructor | 557 µs | 514 µs | 139 µs |
| transform | 980 µs | 915 µs | 283 µs |
| recursion | 1.01 ms | 975 µs | 743 µs |
| shipping rule | 205 µs | 191 µs | 105 µs |
| index into 10,000 items | 13.3 µs | 1.09 µs | 633 ns |
| index in a variable | 13.8 µs | 1.56 µs | 869 ns |

Every case takes about 11 µs less. With `mimalloc`, as the release binaries are built, `path`
takes 339 ns, where the 0.17 table above gave 11.7 µs.

The libraries pay the same cost where an application evaluates on the process's main thread.
On Alpine 3.24 (Podman on the machine above, Python 3.14, OpenJDK 25), with the wheel and the
C API library built in `rust:alpine` before and after, the median of three runs:

| Evaluation | Before | After |
|---|---:|---:|
| Python, main thread, `path` against a `Document` | 13.9 µs | 0.41 µs |
| Python, main thread, `path` against a `dict` | 14.7 µs | 0.73 µs |
| Python, main thread, group-by against a `Document` | 91.1 µs | 73.6 µs |
| Python, another thread, `path` against a `Document` | 0.43 µs | 0.42 µs |
| Python, another thread, group-by against a `Document` | 70.9 µs | 73.3 µs |
| Java, `main`, `path` against a `Document` | 0.47 µs | 0.48 µs |
| Java, `main`, group-by against a `Document` | 76.7 µs | 76.2 µs |

Java never paid it: the `java` launcher runs `main` on a thread of its own. Python's other
threads did not either; their group-by measured 3% slower after, but 1.4% of that also with
the new code built and never called, which points to the layout of the library.

### WebAssembly against jsonata-js (Node.js)

Both run in the same Node.js process, on the same input. The WebAssembly module is the release
build optimized by `wasm-opt -Os`. The comparison script first checks
that both produce the same result for each case. The jsonata-js times include the `await`
that its asynchronous `evaluate` requires.

| Case | jsonata-js 2.2.2 | jlens-wasm `evaluate` | jlens-wasm `evaluateJson` | jlens-wasm `evaluateDocument` |
|---|---:|---:|---:|---:|
| path | 2.1 µs | 4.8 µs | 111 µs | 1.4 µs |
| names over items | 63 µs | 52 µs | 199 µs | 13 µs |
| filter + aggregate | 626 µs | 139 µs | 282 µs | 47 µs |
| group-by | 1.00 ms | 192 µs | 330 µs | 97 µs |
| sort + map | 4.84 ms | 506 µs | 649 µs | 426 µs |
| strings | 574 µs | 204 µs | 351 µs | 97 µs |
| regex | 621 µs | 253 µs | 382 µs | 184 µs |
| number formatting | 550 µs | 99 µs | 255 µs | 41 µs |
| object constructor | 1.18 ms | 309 µs | 542 µs | 222 µs |
| transform | 1.32 ms | 770 µs | 908 µs | 440 µs |
| recursion | 7.75 ms | 670 µs | 784 µs | 664 µs |
| shipping rule | 1.65 ms | 301 µs | 416 µs | 135 µs |
| index into 10,000 items | 2.2 µs | 6.4 µs | 4.80 ms | 1.9 µs |
| index in a variable | 3.02 ms | 7.2 µs | 4.81 ms | 2.3 µs |

jsonata-js works on the JavaScript objects it is given, so it never pays for converting the
input. jlens-wasm `evaluate` reads the input object as the expression reaches it: an object or
array of up to 16 values through one call to a JavaScript helper, a larger one whole, through
`JSON.stringify` and a parse into jlens's representation. The path case reads two small
objects. An expression that reads every item pays what converting the whole input does:
`JSON.stringify` of the input takes 42 µs, and parsing that text 272 µs. A path step reads
its one field alone, since 0.26, as with the native and Python inputs: `path` takes 4.9 µs
instead of 7.3 µs, the other cases within 2.1% (the median of four alternated runs, on a day
when both builds and jsonata-js measured about 7% above the table). A long array of objects
is read in one call with the fields the expression reads of each item, since 0.26 too, rather
than whole through `JSON.stringify` ([ADR 0010](../adr/0010-lazy-input.md#follow-up-long-arrays-of-objects-026-5)):
with `evaluate`, `names over items` takes 52 µs instead of 337, number formatting 100 instead
of 369, the shipping rule 302 instead of 467, and the six other cases that read the order's
items 30% to 62% less; the transform and recursion within 1.3%. The cases that take a few
microseconds pay about 0.3 µs more each: `path` 4.8 instead of 4.5 µs, the indexes into
10,000 items 6.5 and 7.3 instead of 6.1 and 6.8 µs (the median of four alternated runs).
Building empty `Bindings` for each call allocates nothing since 0.26 #11, which `evaluate` gains
too: in four alternated runs of the builds on either side of it, `path` 4.70 → 4.60 µs, the
indexes 6.45 → 6.30 and 7.15 → 7.00 µs, and the other cells of `evaluate` within 2.8%. That is 0.1
µs, less than these runs can settle: jsonata-js, the same in both, read 2.10 and 1.85 µs on `path`,
and WebAssembly has no floor measured as the other packages do. An index into an
array reads the items it picks alone, since 0.24: with `evaluate`, `items[0].code` over
10,000 items takes 6.7 µs instead of 6.19 ms, and `($i := 0; items[$i].code)` 7.3 µs instead
of 6.19 ms, against 2.2 µs and 2.9 ms in jsonata-js, which tests every item for an index held
in a variable. The source writes the arrays and objects of a level as members of one
`Sources` too; the cases of the table moved by between −3.6% and +0.4% (the median of four
runs each).

`evaluate` is faster than jsonata-js on twelve of the fourteen cases, by up to 12× on recursion,
and by 420× on the index held in a variable, which jsonata-js tests against every item. It is
slower on two, `path` and the index into 10,000 items, which do almost no work, so the overhead
of calling into WebAssembly dominates. `names over items`, which reads every item and does little
with each, was slower by 5.3× in 0.25.0, as converting the input was nearly all of its time; since
0.26 #5, which reads the items' one field without converting them, it is faster (52 against 63
µs).

In 0.18.0 it was also slower on `number formatting`, by 11%; it now takes 82% less time (roadmap
0.19 #2 and #4, 0.26 #5).

Against a `Document` converted once, jlens-wasm is faster on all fourteen cases, by up to 13× on
number formatting and filter + aggregate, 12× on recursion and the shipping rule, 11× on sort +
map and 10× on group-by. On `transform` its lead is 3×, since 0.26 #6 (42% in 0.25.0), and on the
index into 10,000 items 16%.

From 0.19 on, `evaluate` and `evaluateDocument` build their result straight into
JavaScript values ([ADR 0017](../adr/0017-result-builder.md)): the builder writes it as a list
of operations, its numbers and its strings, and one call to a JavaScript helper builds the
values, decoding each key once. Before, the result was built as `serde_json` values, printed
as JSON text and given to `JSON.parse`. The cases with large results gained most: against a
`Document`, the object constructor took 466 µs, the transform 1.24 ms and `$` alone 376 µs,
against 305 µs, 947 µs and 114 µs after it. In 0.18.0, `transform` was slower than jsonata-js with
`evaluate`.

`evaluateJson` takes JSON text and returns JSON text. From 0.18.0 on, it parses the input
without the fields the expression does not read ([ADR 0016](../adr/0016-fields-read.md)). It
was faster than `evaluate` on eight of the fourteen cases in 0.25.0, and since 0.26 #5, which
reads long arrays with the fields the expression reads, it is slower on all of them; it still
writes its result through
`serde_json` values. In 0.17.0, which parsed the whole text,
`path` took 296 µs, `names over items` 307 µs and `regex` 487 µs; the shipping rule, which reads
most of the input, takes the same time as then. On `path`, parsing
24 KB of text still costs far more than the evaluation, and on the two index cases parsing
the text of 10,000 items takes 4.8 ms, where `evaluate` reads the one item it picks: an input
evaluated more than once is best converted once into a `Document`.

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

Measured for 0.26.0 (at 238de7f with the release's dependencies, whose code is the release's),
on Python 3.14.8, under Linux: Ubuntu 26.04 under WSL 2, on the machine above. `jsonatapy` has
wheels up to Python 3.13 and was built from its source distribution. The input is the document as
Python values; for jlens and `jsonatapy`, also an input converted once (`Document`, `JsonataData`).
From this table on, `python.py` passes a keyword only when its option is given, as an application
calls, which takes about 40 ns from `path` against 0.25.0's table, the script's doing and not the
package's (see [The calls of `python.py`](#the-calls-of-pythonpy-026-19)). jlens and `jsonatapy`
are the median of three runs, which agreed within 8% (`jsonatapy`'s index into 10,000 items
converted once 0.4 to 0.5 µs); `jsonata-python` and `jsonata-rs` one run:

| Case | jsonata-python 0.7.1 | jsonata-rs 0.1.4 | jsonatapy 2.2.11 | jsonatapy, `JsonataData` | jlens `evaluate` | jlens `evaluate_document` |
|---|---:|---:|---:|---:|---:|---:|
| path | 1.46 ms | 215 µs | 0.7 µs | 0.4 µs | 0.8 µs | 0.5 µs |
| names over items | 1.93 ms | 248 µs | 24 µs | 4.7 µs | 22 µs | 5.6 µs |
| filter + aggregate | 4.60 ms | 400 µs | 122 µs | 88 µs | 65 µs | 38 µs |
| group-by | 6.00 ms | 845 µs | 166 µs | 108 µs | 108 µs | 72 µs |
| sort + map | 24.4 ms | 1.87 ms | 456 µs | 393 µs | 291 µs | 239 µs |
| strings | 4.52 ms | 467 µs | 165 µs | 138 µs | 83 µs | 54 µs |
| regex | 4.99 ms | 3.93 ms | 2.70 ms | 2.65 ms | 125 µs | 100 µs |
| number formatting | 4.10 ms | 437 µs | 154 µs | 145 µs | 34 µs | 22 µs |
| object constructor | 5.93 ms | 818 µs | 184 µs | 141 µs | 155 µs | 117 µs |
| transform | 5.84 ms | 1.81 ms | 1.01 ms | 885 µs | 369 µs | 254 µs |
| recursion | 40.8 ms | 2.25 ms | 743 µs | 842 µs | 553 µs | 554 µs |
| shipping rule | 9.44 ms | 689 µs | 337 µs | 238 µs | 166 µs | 93 µs |
| index into 10,000 items | 29.2 ms | 4.92 ms | 310 µs | 0.4 µs | 1.3 µs | 0.8 µs |
| index in a variable | 48.7 ms | 5.73 ms | 614 µs | 507 µs | 1.6 µs | 1.0 µs |

- With Python values in, jlens is the fastest on thirteen of the fourteen cases, by 22 times on
  `regex`, 4.5 times on number formatting, 2.7 times on the transform, twice on the shipping rule,
  and 240 and 380 times on the two index cases, which read one item where `jsonatapy` reads the
  10,000; `jsonatapy` on `path` (0.7 against 0.8 µs). On `recursion` jlens takes 26% less time, 553
  against 743 µs, since the frames of 0.26 #18 and #20, against 6% for 0.25.0; the two took the
  same time for 0.24.0 (727 µs), and `jsonatapy` was ahead by 3% or less for 0.23.0. At 0a1c6ad,
  before items 9 to 11 of roadmap 0.19,
  jlens took 1.1 µs on `path`, 35 µs on `names over items` and 203 µs on the object
  constructor, and was the fastest on nine cases; in 0.17 (c76a814) it took 357 µs on the
  object constructor, 167 µs on number formatting and 868 µs on the transform, and was the
  fastest on eight; before a path step read its one field of a `dict` (see below), it took
  138 µs on `names over items` and was the fastest on five.
- With the input converted once, jlens is the fastest on eleven of the fourteen cases, by up to
  6.6 times on number formatting and 26 times on `regex`, and on the object constructor by 117 against
  141 µs; `jsonatapy` is faster on `path`, `names over items` and the index into 10,000 items
  (0.4 against 0.8 µs).
- On every case but `path` and the two index cases, jlens `evaluate` takes 4 (`recursion`, the
  shipping rule) to 31 times (`regex`) less time than `jsonata-rs`, and 16 to 120 times less
  than `jsonata-python`. Both read the whole input before evaluating: on `path` they take 0.2
  and 1.5 ms where jlens takes 0.8 µs, and on the index into 10,000 items 4.9 and 29 ms where
  it takes 1.3 µs.

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
`**options`, so that a call with options builds no tuple and no `dict`. `python.py speed` then
passed `timeout=None, max_depth=None` to jlens. Measured before and after on the same system, the
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
amended](../adr/0014-python-package.md#amendment-mimalloc-as-the-allocator-019)); mimalloc 2
outside Windows from 0.24.1 on ([ADR 0014, follow-up](../adr/0014-python-package.md#follow-up-mimalloc-2-outside-windows-0241)). Measured the
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

### Java against the other implementations

`cargo xtask java-compare` compares the Java package with two other JSONata implementations for
the JVM on Maven Central: jsonata-java, a port of jsonata-js
to Java objects, and jsonata-jvm-compiler, which translates an expression to Java and compiles it
with `javac` at run time, over Jackson trees.
[`benches/compare/README.md`](https://github.com/thkprado/jlens/blob/main/benches/compare/README.md)
says how to run it, and which packages it leaves out and why. It first checks that each gives
jlens's result for each case, and both do.

Measured for 0.26.0 (at 238de7f with the release's dependencies, whose code is the release's), on
OpenJDK 25.0.4 under Linux:
Ubuntu 26.04 under WSL 2, on the machine above (16 hardware threads), in one JVM with `-Xmx4g`.
Each time is the best of five runs of 0.3 s after a second of warm-up. Each engine returns its result as its
own objects, not serialized: jlens as maps, lists and boxed values. With the input converted
once: Java maps for jsonata-java and for jlens's `evaluate(Object)`, a Jackson tree for
jsonata-jvm-compiler, and a `Document` for `evaluate(Document)`:

| Case | jsonata-java 0.9.10 | jsonata-jvm-compiler 1.0.7 | jlens `evaluate(Object)` | jlens `evaluate(Document)` |
|---|---:|---:|---:|---:|
| path | 7.2 µs | 0.05 µs | 1.5 µs | 0.39 µs |
| names over items | 21 µs | 1.8 µs | 32 µs | 6.0 µs |
| filter + aggregate | 112 µs | 4.6 µs | 100 µs | 41 µs |
| group-by | 218 µs | 13 µs | 137 µs | 74 µs |
| sort + map | 777 µs | 50 µs | 329 µs | 268 µs |
| strings | 120 µs | 8.9 µs | 132 µs | 60 µs |
| regex | 182 µs | 19 µs | 149 µs | 105 µs |
| number formatting | 124 µs | 14 µs | 58 µs | 24 µs |
| object constructor | 178 µs | 8.1 µs | 181 µs | 120 µs |
| transform | 401 µs | 37 µs | 426 µs | 253 µs |
| recursion | 1.53 ms | 37 µs | 584 µs | 614 µs |
| shipping rule | 296 µs | 20 µs | 223 µs | 99 µs |
| index into 10,000 items | 177 µs | 0.06 µs | 2.0 µs | 0.66 µs |
| index in a variable | 1.01 ms | 51 µs | 2.3 µs | 0.91 µs |

- jsonata-jvm-compiler is the fastest on every case but the index in a variable, by 1.7 (number
  formatting) to 17 times (`recursion`) against a jlens `Document`: its expression is a Java
  class, which the JIT compiles with the application. On `($i := 0; items[$i].code)` it takes
  51 µs against 0.91.
- Against jsonata-java, the other interpreter, jlens over a `Document` is faster on every case,
  the transform too since 0.26 #6, 253 against 401 µs. It takes two fifths of the time on
  `recursion`, a third on `sort + map` and the shipping rule, and a fifth on number formatting.
- Over Java maps, jlens takes 1.2 to 5.3 times as long as over a `Document` on the cases that
  read the order's items (`names over items` 32 against 6.0 µs). jsonata-java, which reads maps in
  Java, is then faster on four of the fourteen cases (`names over items`, strings, and by 2 to 6%
  the object constructor and the transform); jlens on the ten others, by 2.6 times on `recursion`
  and twice on number formatting. Since 0.26 #4, a map met in a level is written with the fields
  the expression reads, so that most fields cost no call back into Java
  ([ADR 0021, amended](../adr/0021-java-package.md#amendment-maps-written-with-what-the-expression-reads-026-4)):
  `names over items` took 45 µs in 0.25.0, the shipping rule 324, the cases that read the items
  5 to 32% less now, and number formatting 30% more
  ([0.26 against 0.25.0](#026-against-0250)).

From JSON text, parsed at each evaluation, by its own parser for jsonata-java, by Jackson for
jsonata-jvm-compiler, and by `evaluateFromJson` for jlens, which skips the fields the expression
does not read:

| Case | jsonata-java 0.9.10 | jsonata-jvm-compiler 1.0.7 | jlens `evaluateFromJson` |
|---|---:|---:|---:|
| path | 179 µs | 133 µs | 33 µs |
| names over items | 166 µs | 121 µs | 73 µs |
| filter + aggregate | 261 µs | 125 µs | 134 µs |
| group-by | 394 µs | 135 µs | 169 µs |
| sort + map | 976 µs | 173 µs | 355 µs |
| strings | 268 µs | 130 µs | 161 µs |
| regex | 331 µs | 140 µs | 177 µs |
| number formatting | 268 µs | 131 µs | 104 µs |
| object constructor | 330 µs | 127 µs | 219 µs |
| transform | 543 µs | 157 µs | 383 µs |
| recursion | 1.67 ms | 153 µs | 649 µs |
| shipping rule | 481 µs | 145 µs | 220 µs |
| index into 10,000 items | 2.50 ms | 2.10 ms | 1.98 ms |
| index in a variable | 3.35 ms | 2.22 ms | 1.98 ms |

- jlens is the fastest where the parsing is the cost: `path` takes 33 µs against 133 and 179,
  `names over items` 73 against 121 and 166, and the two index cases, which read one item of
  10,000, 2.0 ms against 2.1 to 3.4. Where the evaluation is the cost, jsonata-jvm-compiler is:
  153 against 649 µs on `recursion`, 173 against 355 on `sort + map`.
- jlens is faster than jsonata-java on every case, the transform too since 0.26 #6, 383 against
  543 µs.
- jsonata-jvm-compiler reads 10 to 28 µs higher on `path`, `names over items` and filter +
  aggregate than in 0.25.0's table, which was measured with Jackson 2.21.2 where this one has
  2.22.4; whether Jackson is the cause was not looked for.

On the documents of ADR 0021's spike, 1,000 and 100,000 items of five fields each, with the input
converted once:

| Expression | jsonata-java 0.9.10 | jsonata-jvm-compiler 1.0.7 | jlens `evaluate(Object)` | jlens `evaluate(Document)` |
|---|---:|---:|---:|---:|
| **1,000 items (97 KB of JSON)** | | | | |
| `items[0].name` | 84 µs | 0.05 µs | 2.0 µs | 0.67 µs |
| `($i := 0; items[$i].name)` | 146 µs | 0.70 µs | 2.3 µs | 0.95 µs |
| `$count(items)` | 84 µs | 0.05 µs | 1.6 µs | 0.64 µs |
| `$sum(items.price)` | 158 µs | 8.6 µs | 149 µs | 27 µs |
| `items[price > 98].id` | 339 µs | 13 µs | 276 µs | 102 µs |
| `$` | 152 µs | 0.05 µs | 0.80 µs | 485 µs |
| **100,000 items (10,099 KB of JSON)** | | | | |
| `items[0].name` | 38.3 ms | 0.05 µs | 2.0 µs | 0.69 µs |
| `($i := 0; items[$i].name)` | 41.8 ms | 302 µs | 2.3 µs | 0.95 µs |
| `$sum(items.price)` | 52.9 ms | 4.96 ms | 20.8 ms | 5.23 ms |
| `$` | 66.6 ms | 0.04 µs | 0.80 µs | 54.9 ms |

From JSON text:

| Expression | jsonata-java 0.9.10 | jsonata-jvm-compiler 1.0.7 | jlens `evaluateFromJson` |
|---|---:|---:|---:|
| **1,000 items (97 KB of JSON)** | | | |
| `items[0].name` | 765 µs | 747 µs | 358 µs |
| `($i := 0; items[$i].name)` | 833 µs | 753 µs | 359 µs |
| `$count(items)` | 767 µs | 736 µs | 231 µs |
| `$sum(items.price)` | 844 µs | 784 µs | 372 µs |
| `items[price > 98].id` | 1.03 ms | 788 µs | 521 µs |
| `$` | 807 µs | 736 µs | 1.37 ms |
| **100,000 items (10,099 KB of JSON)** | | | |
| `items[0].name` | 95.2 ms | 73.7 ms | 38.8 ms |
| `($i := 0; items[$i].name)` | 84.5 ms | 74.9 ms | 39.0 ms |
| `$sum(items.price)` | 95.3 ms | 78.9 ms | 41.3 ms |
| `$` | 122 ms | 73.7 ms | 157 ms |

- On an input converted once, an index or `$count` takes the same time on 100,000 items as on
  1,000 in jlens, and in jsonata-jvm-compiler but for the index in a variable (0.70 µs, then
  302 µs). jsonata-java takes time in proportion to the items: 84 µs on 1,000 and 38 ms on
  100,000 for `items[0].name`.
- `$` returns the input. jlens over Java maps and jsonata-jvm-compiler return it as it is, in
  under a microsecond; jsonata-java takes 152 µs and 67 ms. jlens over a `Document` builds the
  whole document as Java objects, 485 µs on 1,000 items and 55 ms on 100,000, and from JSON text
  too, which only there takes longer than the others (157 against 74 and 122 ms).
- From JSON text, jlens takes about half the time of the others on 100,000 items, 39 to 41 ms
  against 74 to 95, but on `$`.

Evaluations per second of `$sum(items.price)` on the JSON text of 100 items (9.5 KB), on 1 to 16
platform threads and on 1,000 virtual threads, each engine parsing the text at each evaluation;
jsonata-java, whose compiled expressions are not thread-safe, compiles one per thread:

| Engine | 1 thread | 2 threads | 4 threads | 8 threads | 16 threads | 1,000 virtual threads |
|---|---:|---:|---:|---:|---:|---:|
| jlens, JSON text | 26,028 (1.0×) | 51,109 (2.0×) | 96,994 (3.7×) | 165,316 (6.4×) | 228,162 (8.8×) | 239,146 (9.2×) |
| jlens, Java objects parsed by Jackson | 10,345 (1.0×) | 19,951 (1.9×) | 36,866 (3.6×) | 64,528 (6.2×) | 86,311 (8.3×) | 84,658 (8.2×) |
| jsonata-java 0.9.10 | 11,562 (1.0×) | 22,557 (2.0×) | 42,747 (3.7×) | 72,849 (6.3×) | 79,830 (6.9×) | 80,613 (7.0×) |
| jsonata-jvm-compiler 1.0.7 | 12,442 (1.0×) | 24,038 (1.9×) | 45,488 (3.7×) | 76,816 (6.2×) | 106,201 (8.5×) | 108,491 (8.7×) |

- jlens leads at every count of threads: 228,162 evaluations per second on 16 threads, against
  106,201 for jsonata-jvm-compiler and 79,830 for jsonata-java, and 239,146 on 1,000 virtual
  threads.
- Parsed by Jackson into Java maps first, jlens gives 86,311 on 16 threads, ahead of jsonata-java
  since 0.26 #4 (64,916 in 0.25.0); the parse, not the evaluation, is then the cost.

How each behaves at the limits, measured by `cargo xtask java-compare limits` on the same
system, with the cases of `python.py limits`. Each case runs in a JVM of its own, on its main
thread with the default stack, killed after 30 seconds. jlens reads Java objects
(`evaluate(Object)`), jsonata-java maps, and jsonata-jvm-compiler a Jackson tree, built the same
way, since text cannot hold a cycle. The messages are cut short:

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

- jlens fails with a JSONata error code in each case but the endless loop with no timeout, and
  refuses input nested more than 1,000 levels, as in Python. No JVM crashed: a
  `StackOverflowError` is thrown to the caller, who must catch an `Error` rather than an
  exception to survive it, and it can leave an application's own state half updated.
- jsonata-java fails on the cycle even for `n`, which reads one field of the input.
- jsonata-jvm-compiler turns a stack overflow into `U1001`, as `jsonatapy` does. It has no tail
  calls, so the endless tail call stops there too, and so would a loop of 100,000 tail calls. It
  has no depth limit.
- jsonata-java takes a depth limit and a timeout (`Frame.setRuntimeBounds`), and reports them
  with messages, not codes. The same cases on Windows gave the same outcomes.

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
- **The fixed cost from Python and Java.** Measured in 0.26 on one system (Windows 11, the
  Ryzen 7 3700X of the tables, Python 3.15.2, OpenJDK 25.0.5), `customer.name` against a
  `Document` takes 0.23 µs natively, 0.18 µs with mimalloc, which both packages use; 0.55 µs
  from Python and 0.48 µs from Java. Timed in parts with variants of each package:
  - From Python, about 0.19 µs is the call itself: its eleven optional arguments, the bindings
    and options converted (about 60 ns, though none are given, 30 ns since 0.26 #11) and the result built.
    `evaluate_document` releases the interpreter during the evaluation, so that other threads
    run, and attached again to build the result: about 75 ns each time, twice. Since 0.26 a
    result that is not an array or an object is built once the evaluation has returned, which
    attaches once: `customer.name` takes 0.49 instead of 0.55 µs, `1` 0.36 instead of 0.47 µs,
    the index into 10,000 items 0.84 instead of 0.92 µs. Empty bindings allocated the `Arc` that
    holds their functions: natively, `Bindings::new()` and its release took 26 ns with mimalloc
    (40 ns with the system allocator), and building them at each call made `1` 30% slower than
    sharing one. Since 0.26 #11 the `Arc` comes with the first function, and empty bindings cost
    4.5 ns: from Python, `customer.name` against a `Document` takes 0.44 instead of 0.47 µs,
    `1` 0.31 instead of 0.35 µs, `nothing` over a `dict` 0.42 instead of 0.47 µs.
  - In 0.26 #7, empty bindings shared by every call made some cases over Python values 3% to 5%
    slower. That was the layout of the code, not the bindings: natively, `1` with shared
    bindings, which neither change touches, takes 99 ns in one build of 0.26 #11 and 113 ns in
    another, depending only on how the code is placed. Between builds of one change, a case of
    `python.py speed` moves by up to 4%, in either direction, with alignment or without.
  - From Java, the C API takes about 0.19 µs beyond the evaluation, timed from Rust: finding the
    expression and the document by their handles, each behind the lock of its table, encoding
    the result as values, and storing the reply as a buffer (an allocation and a lock), which
    Java reads with three more calls, its address, its length and its release (about 55 ns
    without the JVM). Java's side is about 0.1 µs: the five calls, which leave Java's thread
    state, and the result decoded. Since 0.26 the three calls that read the reply are critical
    (`Linker.Option.critical`), which skips that transition: `customer.name` takes 0.44 instead
    of 0.48 µs, the indexes into 10,000 items 2.6% less time. Since 0.26 #10 an evaluation
    replies into a scratch buffer Java keeps, and each thread finds its expression and document
    again without the tables' locks: timed from Rust, the C API's share beyond the evaluation is
    67 instead of 177 ns, and `customer.name` takes 0.34 µs from Java. Those locks were shared by
    every thread: on one `Document`, 16 Java threads made 1.0 million evaluations of
    `customer.name` per second, fewer than one thread's 2.2 million, and now make 20.8 million
    ([ADR 0021](../adr/0021-java-package.md#results-a-reply-in-one-call-and-handles-found-without-a-lock-026-10)).
  - The same native library loaded from two files, which differ only in their timestamps, made
    `evaluateFromJson` over 200 items 4.5% slower or faster depending on the file, whichever
    JAR carried it: Java comparisons between two builds have that much noise, as natively
    between two layouts.
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
- **Values read through a source.** A `Source` writes each level through a `Sink`, which
  builds a value for each item. A key is looked up in the evaluation's table of keys: one
  `Rc<str>` per distinct key, then a hash and a lookup for each field. A string is an
  `Rc<str>`. An array or object is a lazy value of its own: 120 bytes for an object, and two
  allocations for an array. Profiled in 0.24 with `perf` (Linux, in WSL):
  - Reading the 10,000 items of `items[0].code` through a source spends 60% of its time
    allocating and writing the 10,000 lazy objects and 39% freeing them. The `Sink`'s own work
    is a few percent: reading the same `serde_json::Value` lazily, with no source, costs about
    as much (385 µs, against 415 µs through the source).
  - In `host_state`'s `evaluate_source`, looking keys up takes 2.3% and growing the table of
    keys 0.9%; the evaluator takes most of the rest.

  Two costs went in 0.24. Each lazy object drew a new hasher seed for its map, 1.2% of the
  10,000 items; the evaluation now draws one and its objects copy it. The table of keys grew one
  rehash at a time while the first object was read; it now makes room for that object's fields
  at once. Measured with `cargo xtask ab` built aligned against the revision before:
  - with `evaluate`, `names over items` takes 17.4 against 18.3 µs and filter + aggregate 61.6
    against 63.3 µs, and every case is between 4.8% faster and 1.1% slower;
  - with `--mode source`, `names over items` takes 26.6 against 27.7 µs, the other cases within
    2%;
  - `host_state`'s `evaluate_source` takes 0.8 to 1.8% less;
  - nothing changes against a `Document`, which reads no lazy values; `recursion` measured 4.9%
    slower there over 20 rounds, from code placement, and 0.5% with `evaluate`.

  In Python (`python.py speed`, WSL, three runs of each), with Python values in, the index into
  10,000 items takes 444 to 446 µs against 489 to 497 µs, and the other cases are within 1.5%.

  Measured and not kept: a lazy array in one allocation instead of two, with its pending items
  in the array itself. That made `host_state`'s `evaluate_source` 2 to 4% faster, but every
  array 16 bytes larger, those an evaluation builds too, and `names over items` against a
  `Document` 2 to 3% slower in every run. What remains is one allocation for each array or
  object a level holds.
- **An index into a long array.** Since 0.24, an index reads the items it picks
  ([ADR 0010](../adr/0010-lazy-input.md#amendment-one-item-at-a-time-024)): from lazy JSON, and
  through a source that tells its length and reads one item. Built aligned against the
  revision before, `items[0].code` over 10,000 items takes 1.31 µs instead of 618 µs with
  `evaluate`, and 1.95 µs instead of 609 µs with `--mode source`, near the 0.67 µs it takes
  against a `Document`; the other cases are within 2%. A predicate that picks more than 16 items
  reads the array whole at once.

  An index held in a variable (`items[$i]`) does the same since 0.24 #5: a predicate built from
  literals and variables gives the same value for each item, so it is evaluated once and picks
  its items, unless it reads `$` (the item) or its input is a tuple stream, which binds `#$i`
  and `@$o` for each item. Before, it was tested against every item: `($i := 0;
  items[$i].code)` over 10,000 items took 2.29 ms with `evaluate`, 2.31 ms with `--mode source`
  and 1.70 ms against a `Document`, and takes 1.97 µs, 2.30 µs and 1.28 µs, built aligned
  against the revision before. The other cases are within 2% in every mode; against a `Document`,
  `path` and `object constructor`, 2.6% and 5.3% slower over 10 rounds, were within 0.1% over
  20.

  The Python package's source reads an item of a `list` or a `tuple` alone since 0.24 #10. With
  Python values in (`python.py speed`, WSL, the median of three alternated runs, both builds
  aligned), `items[0].code` over 10,000 items takes 1.3 µs instead of 413 µs, and
  `($i := 0; items[$i].code)` 1.7 µs instead of 414 µs, near the 0.9 and 1.2 µs they take
  against a `Document`; the other cases are within 2.3%. Built without the alignment flags,
  `names over items`, which reads no item alone, measured 4.6% slower in every run; aligned, it
  takes 21.9 µs in both builds.

  Against 0.23.0 (`cargo xtask ab v0.23.0 HEAD`, the 0.24 code complete, ten rounds), the two
  index cases are the only large change. With `evaluate`, `--mode document` and `--mode source`,
  `items[0].code` takes 1.29 µs, 682 ns and 1.89 µs, against 887 µs, 40.1 µs and 830 µs, and
  `($i := 0; items[$i].code)` 1.93 µs, 1.27 µs and 2.30 µs, against 2.38 ms, 1.58 ms and
  2.39 ms. Every other case is between 5.0% faster (`object constructor` against a `Document`)
  and 1.9% slower (`transform` with `evaluate`), except `strings` with `--mode source`, 5.1%
  slower over ten rounds and 1.3% faster over 20. `names over items` is 4.7% faster with
  `evaluate` and 2.3% with `--mode source`, from the hasher seed the lazy objects now share.
- **Host functions.** Since 0.25, the root frame looks a host function up when no frame holds
  its name, instead of binding every function before each evaluation
  ([ADR 0006, amended](../adr/0006-host-functions.md#amendment-host-functions-looked-up-on-a-miss-025)):
  `$a + $b` with sixteen functions bound takes 0.40 µs instead of 2.82 µs, 0.39 µs with none.
  In 0.25 every lookup carried the bindings to reach them, which made `recursion` (`$f(15)`,
  about 2,000 calls of a lambda, each looking variables up) slower than in 0.24.0: about 2%
  built aligned, after inlining `Frame::lookup` and `variable`, and 4.0% with `evaluate` and
  2.9% against a `Document` built as an application builds it. Since 0.26 the root frame holds
  the functions itself, shared with the `Bindings` through an `Arc`
  ([ADR 0006, follow-up](../adr/0006-host-functions.md#follow-up-the-functions-in-the-root-frame-026-2)),
  and lookups are as in 0.24.0. Against 0.24.0 (`cargo xtask ab v0.24.0 .`, ten rounds),
  `recursion` is within 1.1% in both modes, aligned or not, and every other case within 2.3%;
  `$a + $b` takes the same 0.38 to 0.39 µs with none, one or sixteen functions bound.
- **Deep copies.** A transform (`|...|`) copies its input with `$clone` before changing it, as
  jsonata-js does. jsonata-js copies by writing the JSON text and parsing it
  (`JSON.parse($string(value))`), and jlens did the same until 0.26. Timed in parts against a
  `Document` (0.26 #6, the fastest of seven runs), the benchmark's transform,
  `$ ~> |items|{'total': unitPrice * quantity}, ['tax']|` over 200 items, took 778 µs: the copy
  about 430 µs (writing the text about 100, parsing it with `serde_json` and converting it the
  rest), building the result about 180 µs, as `$` alone does, the update about 120 µs and the
  deletion 30 µs. Since 0.26, `$clone` copies the value itself, with the values the text gave
  (numbers rounded to 15 significant digits, functions as `""`, undefined left out of objects
  and `null` in arrays), counting
  the length the text would have so that it fails where `$string` would. The copy takes about
  85 µs, and the case 447 µs. Against the code before (`cargo xtask ab`, ten rounds, built
  without the alignment flags), the transform takes 543 µs instead of 921 against a `Document`
  (−41%) and 568 µs instead of 1.14 ms with `evaluate` (−50%), where the copy reads the
  input's JSON directly; the other cases are within +1.1%. In Java (`java-compare cases`, six runs of
  each JAR alternated, the fastest), 487 → 272 µs on a `Document` and 626 → 414 µs from JSON
  text, where jsonata-java takes 395 and 527 µs.

  What remains is not the copy. A copy that shares what the update does not change, which the
  roadmap considered, would save at most those 85 µs, and would have to keep apart the places
  where a value is reached twice (`{'x': $o, 'y': $o} ~> |x|{'a': 2}|` changes `x` alone) and
  round the numbers it shares. The update spends about 0.2 µs per item on the object it
  builds and the fields it sets, and about as much again when the field is new and the copied
  object grows. Building the result is the conversion every result pays.

## 0.26 against 0.25.0

Measured for the release (roadmap 0.26 #22), at 238de7f with the release's dependencies, each run
on a new copy of its build, against builds of 0.25.0 made the same way on the same machine. A
difference is read against [what a build shows against itself](#comparing-two-revisions).
0.26 #17 and #18 measured the same comparisons before the frames of #20.

**Natively**, on Windows, `cargo xtask ab v0.25.0 HEAD`, ten rounds, in each mode, built as an
application builds it (two comparisons) and aligned (one):

| Mode | Transform, usual build | Transform, aligned | `recursion`, usual build | `recursion`, aligned |
|---|---:|---:|---:|---:|
| `evaluate` | −51.3%, −51.6% | −50.0% | −21.3%, −21.2% | −24.8% |
| `document` | −47.7%, −46.6% | −46.9% | −21.0%, −20.9% | −25.2% |
| `source` | −40.8%, −41.6% | −41.2% | −21.6%, −21.3% | −25.0% |

`sort + map`, whose `$map` calls a lambda for each item, is −2.8 to −6.8% in all nine, the gain of
#20. Of the 99 other numbers, 17 are more than 2% from 0.25.0: `group-by` in five, +2.1 to +2.6%,
built as usual with `evaluate` and against a `Document`, and aligned against a `Document` (+0.4
and +1.5% in the two other aligned modes); the object constructor in three (−2.7 to −5.0%); the
shipping rule in two (+2.1 and −6.0%); the index into 10,000 items in two against a `Document`
(+2.5 and +2.8%); `strings` in two (−2.1%), the index in a variable in two (−2.3 and −2.6%), and
`path` once (+2.1%). Only the −5.0% and −6.0% are past the 4.1% a build shows against itself, and
both are one reading in the faster direction.

**In Python**, under Linux, `python.py ab`, two comparisons of five rounds:

- Faster, marked in both: the transform −35.3% and −35.6% with Python values, −44.3% and −44.1%
  against a `Document`; `recursion` −18.7% and −19.2% with Python values, −18.9% and −19.2% against
  a `Document`; against a `Document` too, `path` −10.9% and −10.0%, the index into 10,000 items
  −8.7% and −7.7%, and the index in a variable −6.4% and −7.0%: the cost of a call that 0.26 #7
  and #11 took out. `sort + map` −5.3% and −5.2% against a `Document`, marked once; `names over
  items` −3.9% and −3.2% with Python values, marked once.
- Nothing is marked slower, and no number is more than 1.3% slower. At #17, `recursion` against a
  `Document` was marked at +12.1% and +12.5%.

**In Java**, under Linux, `java-compare ab`, two comparisons of five rounds, against a `Document`
and over Java maps, 108 numbers each:

- Faster, marked in both: the transform −27.2 to −46.2%; `recursion` −18.3 to −23.1% in every mode;
  against a `Document`, `path` −20.0% and −13.9%, the indexes and `$count(items)` −6.2 to −16.6%,
  and `sort + map` −6.6% and −7.5%; over Java maps from 0.26 #4, the cases that read the items −5.1 to −33.2%, and
  evaluations per second of `$sum(items.price)` +27.1% and +28.3% on sixteen threads, +31.0% and
  +25.3% on 1,000 virtual threads.
- Slower, marked in both, over Java maps alone: number formatting +29.3% and +32.8% (44 → 58 µs),
  +11.5% and +15.0% from JSON text read into maps; `$` +12.5 to +14.2% (0.72 → 0.81 µs); and `path`
  +6.4% and +10.5% (1.39 → 1.48 and 1.53 µs), +5.4% and +10.1% from JSON text, marked once. No
  number against a `Document`, or from JSON text through `evaluateFromJson`, is marked slower.
- Timed directly, one JVM per run, ten alternated runs of each JAR, each on a new copy of its
  library: `path` over Java maps 1.445 → 1.466 µs (+1.4%), `$` 0.726 → 0.893 µs (+23.0%), and
  number formatting 42.5 → 55.7 µs (+30.9%). So number formatting and `$` are real, the cost of
  0.26 #4, which [ADR 0021 records](../adr/0021-java-package.md#amendment-maps-written-with-what-the-expression-reads-026-4):
  it reads the list of 200 items whole for the 20 it uses, and writes the map of a `$` with its
  level. `path` is not: it rises only inside the harness, and there, from JSON text, by 5 to 11
  µs where the evaluation takes 1.5, which no work of the package can be. Its cause was not found.

**Threads.** `java-compare ab` from the commit before #20, `threads` part, five rounds: every number
within 4.5%, nothing marked. Its expression, `$sum(items.price)`, calls no lambda, so that #20 does
not show there. `recursion` on one `Expression` and one `Document` shared by Java threads, timed
directly, evaluations per second, five alternated runs of each JAR, each on a new copy of its
library:

| Threads | Before #20 | The release | |
|---:|---:|---:|---:|
| 1 | 1,421 | 1,609 | +13.2% |
| 4 | 4,529 | 5,676 | +25.3% |
| 8 | 5,966 | 9,637 | +61.5% |
| 16 | 8,563 | 12,842 | +50.0% |

The runs of the two JARs do not overlap at any count. So what #20 measured from a Rust probe, 44%
more on eight threads, holds from Java, and more.

WebAssembly has no measured floor, and is compared in the release's table alone.

The updates of 0.26.0's dependencies change nothing here: the WebAssembly module is the same to the
byte with and without them, and `cargo xtask ab` compares the two locks within 1.8% in every
mode.

### `recursion` and the frames (0.26 #18)

#17 found one wheel taking 685 µs on `recursion` called with keyword arguments and 760 µs
without, with the same instructions, branches and cache misses, and 0.25.0 taking 690 µs either
way. It read that as a position of the stack, from one pass over 16-byte steps of the
environment's size. That reading was wrong. Six positions, each repeated six times without
address randomization, give the whole range at each position: 682 to 766 µs at one, and at
another 671 to 686 µs and then 768 µs. The time belongs to the process: within one, each call
keeps its time over alternated sets, in either order, also pinned to one CPU. The call without
keywords takes 672 to 785 µs from process to process, the call with them about 690 µs. In a
native program that moves the stack in 16-byte steps within one process, the time stays
within 718 to 737 µs.

Sixteen processes of one call, on the counters of the processor (a Ryzen 7 3700X), by how they
follow the cycles, which ranged over 2.33 to 2.74 G:

| Counter | Correlation | Slowest quarter against fastest |
|---|---:|---:|
| Fetch stalled, any cause | +1.00 | +23.5% |
| Fetch stalled, branch prediction queue empty | +0.98 | +178% |
| Micro-operations from the decoder | +0.99 | +8.7% |
| Micro-operations from the op cache | +0.68 | +0.4% |
| L1 branch target buffer corrections | +0.98 | +4.9% |
| Branch mispredictions | −0.56 | −2.5% |
| Store-to-load conflicts | −0.62 | −2.0% |
| L1 data accesses | +0.98 | +1.2% |

So the front end stalls; the data caches and memory dependencies do not explain it. A profile
of the two calls side by side, on a wheel without `mimalloc`, where they differ most steadily,
puts 550 of the 595 samples between them at two addresses, about 70 cycles each per call of
`$f`: the entry of `memset`, which sets the control bytes of the table a frame held its variables
in, and the first read of the arguments in `apply_inner`. With `mimalloc`, the entry of `memset`
and a lookup in that table. The seed of that table's hasher, which `foldhash` makes for each table
from the stack pointer and a thread-local, looked like the link to the stack, but with a fixed
seed the time still ranges over 700 to 793 µs.

Without the table, it does not. A frame now holds up to eight variables in a list, and moves
them to a table past that. Both calls take about 604 µs in each of eight processes, and natively,
in eight processes, `recursion` takes 533 to 542 µs under Linux, where it took 698 to 718 µs.
Which part of the front end the table reached, and why 0.25.0 escaped it, stays unknown; only
this processor was measured. On Windows the same change gives 4.6%.

The cost of `mimalloc` on `recursion` is another thing: without the table it stands at 8.8%
over glibc's allocator (582 against 535 µs), from one process to the next.

A list costs more than a table once it holds many names, as each lookup searches them one by
one. A block of `:=` and twenty lookups under Linux, the median of five alternated runs, timed
to 0.1 µs: two names 1.2 → 1.1 µs, four 1.4 → 1.3 µs, eight 1.7 → 1.8 µs, and 1.7 → 1.9 µs when
every lookup is of the last of the eight. Past eight, the move to a table costs once: nine names
1.8 → 1.9 µs, sixteen 2.4 → 2.5 µs. A frame of a lambda rarely holds more than a few.

### The calls of `python.py` (0.26 #19)

#17 found `python.py` calling `evaluate` with `timeout` and `max_depth` as `None` and
`evaluate_document` with neither, so that the column over Python values showed a call that
applications do not make. The same went for `jsonatapy`, whose `evaluate` took `timeout` and
`max_stack_depth`. On both packages a keyword is now passed only when its option is given.
`jsonata-python` and `jsonata-rs` were called so already.

What the change moves, at 2c4c001 under Linux: one wheel of jlens and `jsonatapy` 2.2.11, each
called both ways by a copy of `python.py` that chose the call by label, `python.py ab`'s
alternation over new copies of the package, two comparisons of five rounds in opposite orders.
With keywords → without, and the two differences:

| Case | jlens | `jsonatapy` |
|---|---:|---:|
| path | 0.886 → 0.842, 0.878 → 0.843 µs (−5.0%, −4.0%) | 0.702 → 0.682, 0.711 → 0.673 µs (−2.8%, −5.3%) |
| index into 10,000 items | 1.366 → 1.334, 1.367 → 1.348 µs (−2.4%, −1.4%) | 310 → 301, 310 → 299 µs (−3.1%, −3.5%) |
| index in a variable | 1.666 → 1.632, 1.690 → 1.639 µs (−2.0%, −3.0%) | 619 → 608, 620 → 607 µs (−1.8%, −2.2%) |
| recursion | 620 → 626, 623 → 622 µs (+0.9%, −0.1%) | 735 → 739, 732 → 735 µs (+0.6%, +0.5%) |

The keywords cost about 40 ns of a call, as the `max_depth` and `max_steps` of
[What the limits cost](#what-the-limits-cost) do. Only `jsonatapy`'s `path` in the second
comparison is past the mark. `jsonatapy`'s two index cases take 300 and 600 µs, where 40 ns does
not show: their differences are within their spread, up to 12%. Every other case over Python
values is within 2.9%, on either package, and the columns against a `Document` or `JsonataData`,
the same call on both sides, within 4.5%. `recursion` does not move: #18 took out the whole of
what the keywords had made of it.

The Python table reads this to 0.1 µs: from 0.26.0 on jlens's `path` reads 0.8 instead of 0.9 µs,
its two index cases may read 0.1 µs less by rounding, and no other number of either package moves
by the change. Against the table of 0.25.0, those 40 ns of `path`, and the 20 to 50 ns of the
index cases with Python values, come from the script, not from the package. `python.py ab` calls
both builds with one script, and so compares them as it did.

### A lambda's frame (0.26 #20)

A call of a lambda made four allocations: the list of its arguments, the frame, the frame's list
of variables, and a copy of the name of each parameter. The frame now keeps the arguments in the
list that carried them, and takes their names from the function, so that a call of one parameter
makes two, and a call of two parameters two instead of five. The frame shares the function by a
reference count of the evaluation's own, where the call took a share of the syntax tree, which
every thread evaluating the expression counts on one cache line.

The share of each part, measured apart on Windows with `mimalloc`, which the packages use: a probe
of one process per build, the median of five alternated runs, each on a new copy, at 1861ee8.
"Names and list" has the frame's new form and still takes the share of the syntax tree:

| Case, per evaluation | Before | Names and list | Both | Allocations per call |
|---|---:|---:|---:|---:|
| `recursion` | 815 µs | 705 µs | 710 µs | 4.0 → 2.0 |
| `$map`, one parameter, 200 items | 51.1 µs | 42.5 µs | 43.2 µs | 6.9 → 4.9 |
| `$map`, two parameters | 64.3 µs | 49.1 µs | 50.6 µs | 6.1 → 3.1 |
| `$reduce`, two parameters | 51.7 µs | 38.8 µs | 38.7 µs | 5.1 → 2.1 |
| `$filter` | 46.8 µs | 38.5 µs | 39.3 µs | 4.2 → 2.2 |
| A block binding one name in the lambda | 94.4 µs | 85.9 µs | 86.4 µs | 7.1 → 5.1 |
| A name bound with no block around it | 61.4 µs | 64.2 µs | 64.9 µs | 5.1 → 5.1 |
| `recursion`, evaluations per second, 1 thread | 1,205 | 1,391 | 1,399 | |
| 4 threads sharing the expression | 4,149 | 4,413 | 5,233 | |
| 8 threads | 5,534 | 5,923 | 8,513 | |
| 16 threads | 7,890 | 8,455 | 10,753 | |

So the allocations are the whole gain on one thread, 13% of `recursion`, more than the 10% the
roadmap estimated, and the share of the syntax tree costs nothing there but most of what threads
lose: on eight threads, 44% more evaluations. A name that a lambda's body binds with no block
around it moves the frame to the list, about 18 ns more per call; a block has a frame of its own,
so that only a body that is the binding itself pays.

`cargo xtask ab 1861ee8 .`, ten rounds, `recursion`, with the system allocator as `ab` builds it:

| | `evaluate` | `document` | `source` |
|---|---:|---:|---:|
| Windows | −19.8% | −20.3% | −19.9% |
| Windows, aligned | −19.2% | −18.3% | −18.0% |
| Linux | −8.4% | −9.2% | −9.9% |
| Linux, aligned | −9.5% | −8.0% | −7.7% |

`recursion` takes 964 → 773 µs on Windows and 560 → 513 µs under Linux with `evaluate`. Windows's
allocator gains the most from each allocation less: `sort + map`, whose `$map` calls a lambda for
each item, takes 1.5 to 5.6% less there and within 1.1% under Linux. Every other case stays within 2.5%, but
for four readings, none repeated in another run, mode or build. On Windows, `object constructor`
against a `Document` built as usual read +5.6% in one of three runs, −1.1% and −0.1% in the others;
its rounds spread widely, the fastest of that run 251 against 274 µs. Under Linux, `transform`
read +3.4% aligned with `evaluate` (−0.1% built as usual), `object constructor` +3.2% built as
usual against a `Document` (+0.4% aligned), and `filter + aggregate` +2.7% built as usual from a
source (−0.3% aligned). Within 2.5% but in all three runs: on Windows, against a `Document` built
as usual, `filter + aggregate` +1.5 to +2.5% and `group-by` +1.3 to +2.1%. Neither calls a lambda,
and aligned they read −0.0% and +0.8%, so that this is placement.

The names of blocks, of tuples and of the host's variables are still copied, one allocation for
each name bound. They could be shared by an `Arc<str>` in the syntax tree, which would count on
one cache line for every thread at each binding, what the threads above just got rid of; or be
kept in place when short, a type of its own for one allocation of the five a call in the block
case above makes. Neither was built.
