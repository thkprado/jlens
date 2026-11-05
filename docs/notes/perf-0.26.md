# Performance notes: 0.26

How the performance work of roadmap 0.26 was measured, moved here from the [performance page](../src/architecture/performance.md)
after 0.27.0. The item numbers are those of
[the roadmap's history](../src/architecture/roadmap-history.md#026).

## 0.26 against 0.25.0

Measured for the release (roadmap 0.26 #22), at 238de7f with the release's dependencies, each run
on a new copy of its build, against builds of 0.25.0 made the same way on the same machine. A
difference is read against [what a build shows against itself](../src/architecture/performance.md#comparing-two-revisions).
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
  0.26 #4, which [ADR 0021 records](../src/adr/0021-java-package.md#amendment-maps-written-with-what-the-expression-reads-026-4):
  it reads the list of 200 items whole for the 20 it uses, and writes the map of a `$` with its
  level. `path` is not: it rises only inside the harness, and there, from JSON text, by 5 to 11
  µs where the evaluation takes 1.5, which no work of the package can be. 0.27 #4 found why:
  [what the JVM ran before it](perf-0.27.md#path-over-java-maps-in-the-harness-027-4).

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

## A time follows the file (0.26 #15)

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

## Copies in the native table (0.26 #16)

The copies settle what a median of one file cannot, the file ([above](../src/architecture/performance.md#comparing-two-revisions)).
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

## The main thread on musl (0.26)

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

## The fixed cost from Python and Java (0.26)

Measured in 0.26 on one system (Windows 11, the
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
  ([ADR 0021](../src/adr/0021-java-package.md#results-a-reply-in-one-call-and-handles-found-without-a-lock-026-10)).
- The same native library loaded from two files, which differ only in their timestamps, made
  `evaluateFromJson` over 200 items 4.5% slower or faster depending on the file, whichever
  JAR carried it: Java comparisons between two builds have that much noise, as natively
  between two layouts.

## Deep copies (0.26 #6)

A transform (`|...|`) copies its input with `$clone` before changing it, as
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

## `recursion` and the frames (0.26 #18)

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

## The calls of `python.py` (0.26 #19)

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
[What the limits cost](../src/architecture/performance.md#what-the-limits-cost) do. Only `jsonatapy`'s `path` in the second
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

## A lambda's frame (0.26 #20)

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

The names of blocks, of tuples and of the host's variables were left, one allocation for each name
bound, and 0.27 #7 took them.
