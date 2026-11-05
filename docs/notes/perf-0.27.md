# Performance notes: 0.27

How the performance work of roadmap 0.27 was measured: methods, traps and intermediate
numbers that the [performance page](../src/architecture/performance.md) leaves out. The page holds the
current tables and what a user needs; the item numbers are those of
[the roadmap's history](../src/architecture/roadmap-history.md#027).

## 0.27 against 0.26.0

Measured for roadmap 0.27 #19, at d8d965c, before the release brings its dependencies up to date,
each run on a new copy of its build, against builds of 0.26.0 made the same way on the same
machine. A difference is read against [what a build shows against itself](../src/architecture/performance.md#comparing-two-revisions).

**Natively**, on Windows, `cargo xtask ab v0.26.0 HEAD`, ten rounds, in each mode, built as an
application builds it (two comparisons) and aligned (one). The medians of the cases 0.27 changed:

| Case | `evaluate` | `document` | `source` |
|---|---:|---:|---:|
| `regex over a long text` (#10, #11) | 56.5 ms → 351 µs, −99.4% | 56.4 ms → 346 µs, −99.4% | 56.7 ms → 347 µs, −99.4% |
| `distinct items` (#6, #12) | 618 → 127 µs, −79.3 to −79.8% | 494 → 15.9 µs, −96.8% | 629 → 133 µs, −78.4 to −79.0% |
| `tuples` (#14) | 258 → 207 µs, −19.5 to −20.3% | 245 → 196 µs, −19.9 to −21.5% | 268 → 220 µs, −17.1 to −18.4% |
| number formatting (#8) | 44.5 → 35.8 µs, −18.4 to −19.6% | +1.3 to +1.9% | 46.8 → 40.3 µs, −13.8 to −14.7% |
| `regex` (#11) | −9.5 to −11.9% | −10.7 to −11.6% | −10.3 to −11.1% |

`strings` is −4.1 to −8.1% in all nine, `sort + map` −2.9 to −5.3%, the index in a variable −1.9
to −6.8%. Against a `Document`, number formatting picks 20 items of a list already converted, so
that #8 has nothing to save there. Of the other 90 numbers, none is more than 2.5% slower but
two, both against a `Document` and both within what a build shows against itself: the transform
+3.6% and +2.5%, −0.4% aligned, and the object constructor +3.3% once (−0.9%, and −1.1% aligned).

Under Linux (WSL 2), against a `Document`, two comparisons and one aligned: the object constructor
+3.5% and +4.6%, past the 2.6% a build shows there, and +2.2% aligned, which is placement;
neither Python nor Java shows it (−3.4 to +2.1%). `group-by` +0.8%, +0.4% and +3.0% aligned.

**In Python**, under Linux, `python.py ab`, two comparisons of five rounds, and one of wheels
built aligned:

- Faster, marked in both: `regex over a long text` −99.6% (57.7 ms → 214 µs), `distinct items`
  −74.9% and −75.1% with Python values, −96.1% against a `Document` (375 → 14.6 µs); number
  formatting −18.5% and −20.2% with Python values; `regex` and `tuples` −4.3 to −9.1%, each
  marked in three of its four numbers.
- Marked slower once, not in the other comparison: the index into 10,000 items with Python
  values, +5.3% (1.30 → 1.37 µs; +0.8% in the other); against a `Document`, `filter + aggregate`
  +6.0% (+1.7%) and `group-by` +4.5% (+4.4%, unmarked). `group-by` against a `Document` is the
  one the runs agree on, and its work did grow: under `perf stat`, 2.2% more instructions and 4.2%
  more branches, natively too (the probe of `ab` 70.1 → 71.4 µs, at the same instructions per
  cycle). The share of samples in dropping values goes from 2.6% to 4.6%, the rest matching function by function: #16 made a `Value`'s
  drop a call, as [its section](#-in-a-loop-027-16) records, and `group-by` drops
  arrays of shared items for each group, which 0.26.0's `Array::drop` left to the compiler's drop
  of the slice. #16 measured dropping shared items by hand, which cost the index cases 4 to 5%, so
  it stays.
- Aligned, `recursion` is marked at +7.2% and +8.0%, where the usual builds give +0.2 to +1.7%.
  The work is the same: 0.4 to 0.6% more instructions, 0.6% more branches, no more branch misses.
  0.26.0's aligned wheel alone is the fast one, 3.34 to 3.46 billion cycles for 1,500
  evaluations against 3.58 to 3.63 built as usual, where HEAD takes 3.59 to 3.73 both ways: a
  placement, as 0.26 #17 found for the same case. The index into 10,000 items is marked aligned
  too, +6.4% with Python values and +4.2% against a `Document`; natively under Linux it is +0.5%,
  +0.0%, and +3.3% aligned. The wheels are built with LTO, which the Java case below lacked
  ([#20](#the-c-apis-library-and-link-time-optimization-027-20)), so this one stays a placement.

**In Java**, under Linux, `java-compare ab`, two comparisons of five rounds and one of the `cases`
part alone, against a `Document` and over Java maps:

- Faster, marked in all: `regex over a long text` −99.5 to −99.6%, `distinct items` −57.1 to
  −96.3%, `tuples` −4.8 to −11.8% (unmarked once); over Java maps, number formatting −41.0 to −42.1% (56.9 → 32.9
  µs, #8), and −12.0 to −14.9% from JSON text read into maps; `regex` against a `Document` −8.0 to
  −9.1%.
- Slower, marked in all: against a `Document`, `$sum(items.price)` +16.1% and +16.4% on 1,000
  items (25.6 → 29.8 µs), +8.7% and +7.6% on 100,000, and `names over items`, `$sum(items.quantity)`,
  +14.1%, +12.6% and +10.0% (5.8 → 6.6 µs). Timed directly, one JVM per run, it is the same: 5.70
  to 5.79 against 6.30 to 6.43 µs. The C API's library built at each commit of 0.27 and loaded
  into one JAR places it at #16: 5.50 to 5.71 µs up to `a5548b1`, 6.17 to 6.23 µs from `29630e5`.
  What #16 changed there is not found. The work is the same, within 2% of instructions for each
  evaluation, but they run at 2.33 instructions a cycle instead of 2.57, the samples gathering on
  loads in `path`, in `guardrails` and in the lookup of a field and its `bcmp`, whose calls are as
  many. It is neither code alignment (libraries built aligned: +11 to +13%), nor where the
  evaluation starts on the stack (from 0 to 9 Java frames deeper, +11 to +13% at each), nor the
  allocator, nor the core: natively, over the same parsed `Document`, with the system allocator
  or `mimalloc` as the library uses it, HEAD is 1 to 3% faster.
  #20 [found it](#the-c-apis-library-and-link-time-optimization-027-20): the library was built
  without LTO, and #16 moved the layout of its code to a slow one. Built with LTO, the JAR is 2 to 5% faster
  than 0.26.0's in these cases.
- `recursion` over Java maps +3.7 to +5.1% in the three, marked once, close to the 4.8% a JAR
  shows against itself; built with LTO (#20), 5.6% faster than 0.26.0.
- Threads, `$sum(items.price)` from JSON text: nothing marked, −1.8 to +5.9%.

WebAssembly has no measured floor, and is compared in the release's table alone.

## `recursion` against a `Document` (0.27 #3)

0.26.0's table showed `recursion` taking 899 µs against a `Document` and 792 µs with `evaluate`,
where the evaluation does the same work (0.27.0's drew 791 µs for both). A `Document` does not cost those 100 µs. In the benchmark's
program, at 377d5e4 with Windows's allocator, the same bytes take about 790 µs in one process and
about 890 µs in the next:

- What the benchmark runs before the case does not decide it: run alone, with criterion's filter,
  the case still gives 785, 896 and 910 µs.
- The file does not either, nor the name of the program or the size of the environment. The length
  of `CARGO_TARGET_DIR` changes the odds, as it changes one string that criterion holds. Over 32
  lengths, three processes each, `evaluate` stays at 780 to 805 µs in 94 of 96 processes, while
  `evaluate_document` gives about 790 or about 890 µs: never 890 at eight of the nine lengths from
  4 to 12, most of the time from 13 to 15 and from 22 to 24.
- Without the `evaluate` case in the process and at one length, it took 806 to 834 µs in 34 runs,
  a third level.

The allocations of one evaluation, recorded in each process, show what is drawn. A call's frame
is a block of 112 bytes, 1,975 times per evaluation. Of 42 processes, in the slow ones these
blocks were 74 addresses up to 2 to 9 MB apart, and in the others 43 or 58 addresses within 8 KB,
with nothing in between:

| Blocks of 112 bytes | Processes | `recursion` against a `Document` |
|---|---:|---:|
| within 8 KB | 31 | 777 to 800 µs |
| in places 2 to 9 MB apart | 11 | 891 to 942 µs |
| the same, without the `evaluate` case (73 addresses) | 6 | 806 to 818 µs |

So the blocks of one size, once the heap is as full as it happens to be, can lie in two places, and
evaluating then costs about 50 ns more per frame, about 10 ns at the third level. That the
crossing itself takes that time is inferred, as the system heap shows nothing of what it does inside. It is the allocator's in any
case: with `mimalloc` as the global allocator, 36 processes at the lengths that drew the slow
level most often all take 718 to 737 µs, in both modes.

The program of `cargo xtask ab`, whose heap is not the benchmark's, did not meet this: 220
processes at 3d3d80f, in the three modes, with the case alone or with all of them, took 740 to
779 µs. Should a change bring the draw to it, `recursion` would read 14% slower alone, and
`--b-allocator mimalloc` would tell. Under Linux it was not looked for; there, nine comparisons in
`ab` of a revision with itself agree within 2.6%.

The table stays as it is: a Rust application on Windows with the system allocator can draw either
time, so that it is what such an application gets, and the frame belongs to the evaluator, which
has no way to choose its place in the heap. The `jlens` command and the Python and Java packages
use `mimalloc`. What changes is how the table is read: `bench-table` marks two groups of runs, and
a case with a slow group goes to `ab`.

## `path` over Java maps in the harness (0.27 #4)

`java-compare ab` gave `path` over Java maps +6.4 and +10.5% against 0.25.0, where a program that
times it alone gave +1.4%. The two differ in what the JVM has run before the case. What the
compiler makes of the package depends on it, and no work of the package was added.

`customer.name` over Java maps, 0.25.0 against the code of 0.26.0, five alternated runs of each
JAR, each in a JVM of its own on a new copy of its library, timed as the harness times it, under
Linux. `apart` means that no run of one JAR reaches the other's:

| What the JVM ran first | 0.25.0 | 0.26.0 | |
|---|---:|---:|---:|
| Nothing | 1.412 µs | 1.382 µs | −2.2%, apart |
| `path` through a `Document` and from JSON text | 1.400 µs | 1.384 µs | −1.1% |
| One expression of `documents`, for each of the six | | | −0.9 to +1.9% |
| The six over Java maps | 1.426 µs | 1.488 µs | +4.3% |
| The six over Java maps, again | 1.445 µs | 1.495 µs | +3.5%, apart |
| The six from JSON text | 1.413 µs | 1.482 µs | +4.9%, apart |
| `documents` on 1,000 items, then the case by call sites of its own | 1.393 µs | 1.472 µs | +5.7%, apart |
| `documents` on 1,000 items, with `-XX:+UseParallelGC` | 1.401 µs | 1.455 µs | +3.9% |
| `documents` as the harness runs it | 1.391 µs | 1.464 µs | +5.3%, apart |
| The six over Java maps, with `-XX:InlineSmallCode=16000` | 1.486 µs | 1.497 µs | +0.7% |

- It takes several expressions: none of the six does it alone. It needs neither a `Document`, nor
  the harness's call sites, nor Jackson's parse of the document, and the collector does not
  change it.
- A compilation log of one run of each shows how. After one expression, the compiled
  `Values.decode` of 0.26.0 takes 736 bytes. After the six, which give it other results to
  decode, it takes 5,568, past the 2,500 of `InlineSmallCode`, so that C2 leaves it out of
  `Expression.evaluate` ("already compiled into a big method"), which then takes in 2,891 bytes of
  bytecode instead of 4,460. Compiled after the six, the same method of 0.25.0 takes in 5,689.
- With that limit raised, the two JARs take the same time, but 0.26.0 is not the one that moves:
  1.495 → 1.497 µs, and 0.25.0 1.445 → 1.486 µs. So a JVM that has seen several expressions costs
  both about 5% on this case, and at the default limit 0.25.0 escapes part of it, by 0.05 µs,
  through what the compiler happens to inline. Why the code of 0.26.0 falls on the other side was
  not looked for.
- From JSON text, the 5 to 11 µs were not a cost. In the runs above the difference goes from −9.0
  to +5.9%, either way round, as one JVM's parse by Jackson takes from 98 to 165 µs with either
  JAR. The spread of those numbers, 11.9% at #22, already said so.

So no code changes, and the harness stays as it is: a JVM that runs one expression is the
exception, and the number inside the harness is the one an application with several expressions
gets. It does mean that a case over Java maps can move by 5% with what runs before it, and that
what a program timing one case alone says is its best case. To compare two builds on one case,
time it after others too.

## Variables against a `Document`, in the packages (0.27 #5)

left it so. The variable as a `Document` converts nothing. A variable read whole costs what it did: run alone, as a process that ran
the cases before it draws another heap, 1,050 against 1,058 µs. A number takes 13 ns more, the
lookup of what is read of it. With no variables nothing is looked up, and `cargo xtask ab main .`
over ten rounds finds every case between −5.1 and +1.0% with `--mode document`, −2.8 and +0.7%
with `evaluate`, and −2.4 and +1.0% with `source`, none past the floor.

The packages evaluate this way, and gain what their own conversion leaves. Java holds its bindings
as a converted object, Python converts a `dict` at each call, in µs:

| Package | Case | Before | After |
|---|---|---:|---:|
| Java, `evaluateJson` | `$v.total` | 630 | 0.83 |
| | `$sum($v.items.price)` | 659 | 157 |
| | the variable not read | 630 | 0.46 |
| | `$self.hp > 10` | 1.35 | 0.80 |
| | no variables | 0.43 | 0.45 |
| Python, `evaluate_document` | `$v.total` | 2,000 | 1,325 |
| | `$sum($v.items.price)` | 2,020 | 1,500 |
| | the variable not read | 2,000 | 1,325 |
| | `$self.hp > 10` | 2.52 | 2.06 |
| | no variables | 0.46 | 0.45 |

The application that asked for this kept a record in its document because a variable cost more:
its battles took 16 to 25% longer with the record in a variable, at the revision before this one,
and now take 1 to 7% longer, what is left being its own serialization of the record for the
variable. With the record in the variable they take 7 to 16% less. Its code that passes no
variables measures +0.8 to +1.9% aligned, over seven rounds on new copies, within the floor.

## `$distinct` (0.27 #6, #12)

Until 0.27 each item was compared with every item kept, so the time grew with
the square of the distinct items. Now the first 16 distinct items are kept by comparing, as an
array with few distinct items never gains from anything else, and past them items are looked up
by a hash, with `=` settling those that hash the same: of the first fields of an object when
those tell the items apart, as below, and otherwise of the whole value. The hash adds the fields
of an object up, so that their order does not count, and gives `0` and `-0` one number. When 16
are kept and 8 or fewer items are left, those are compared too: hashing the 16 costs more.
Against a `Document`, in µs but where marked, before 0.27 the fastest of a process, after it the
median of seven processes, each on a new copy of the program
(`$distinct($)`, the result converted; the items half repeated in `nested`, and eight values
repeated in `few`):

| Items | 10, before | after | 1,000, before | after | 100,000, before | after |
|---|---:|---:|---:|---:|---:|---:|
| numbers | 1.49 | 1.50 | 1,795 | 64.5 | 17.5 s | 9.51 ms |
| `i % 8` | 1.30 | 1.32 | 17.1 | 16.1 | 1.59 ms | 1.48 ms |
| strings | 2.02 | 2.02 | 3,618 | 96.8 | 35.7 s | 14.8 ms |
| objects of three fields | 6.58 | 6.80 | 13,060 | 501 | 392 s | 84.6 ms |
| nested objects | 7.19 | 7.39 | 7,027 | 558 | 247 s | 101 ms |

jsonata-js 2.2.2 is quadratic as well: on the same arrays, 1.7 µs, 156 µs and 1.71 s for numbers,
2.1 µs, 2.92 ms and 32.2 s for strings, and 128 ms for 1,000 objects, whose 100,000 did not
finish in five minutes. With few distinct values it stays ahead, 229 µs against 1.48 ms on
100,000 items, as it did before. Around the switch, from 8 to 64 items, each kind within 2.7%
of the list up to 24 items against a `Document`, and within 4.2% with `evaluate`. Strings and
objects gain 14 to 16% at 25 items, numbers 17% at 32, and nested objects half repeated from 48
items on.

A hash of the whole value reads each item to its end, where comparing stops at the first
difference. Measuring that (0.27 #12) found 25 items of 1 KB that differ in a first field at 29
times the list's time with `evaluate`, and of 100 KB at 4,500 times: the hash had made the common
shape of a record, an `id` and data below it, the slowest. So past the first 16, an array of
objects is first looked up by the first fields of the first item kept, up to eight, as far as
they are not arrays or objects. An item that starts with those fields is compared only with the
items kept that hold the same values there, found by key, so that the order of their fields does
not count. The comparisons left out are false before anything below those fields is read, and
those made are in the list's order, so that the result and the errors are the list's, with a
source as well, and nothing more of an item is read than comparing reads. An item that does not
start with those fields, or that 16 items share them with, goes on to the hash of the whole
value. Of a string, the first fields hash the length and 64 bytes.

In µs but where marked, the fastest of two processes: the list, as before 0.27 #6; the whole
hash, 0.27 #6; and now. `{id, rows}` is an `id` and a list of small records, `{id, 7,000 fields}`
numbers in fields of its own, `{id, text}` one long string, `{rows, id}` the `id` after the list,
`{kind, list}` a field all the items share and a list that starts with what differs, and `[id,
...]` an array:

| Items | Against a `Document`: list | whole hash | now | `evaluate`: list | whole hash | now |
|---|---:|---:|---:|---:|---:|---:|
| 25 of `{id, rows}`, 1 KB | 9.8 | 76.4 | 6.1 | 17.7 | 501 | 14.1 |
| 25 of `{id, rows}`, 100 KB | 9.1 | 21,783 | 5.5 | 17.3 | 79,158 | 13.7 |
| 128 of `{id, rows}`, 100 KB | 225 | 130,603 | 8.9 | 247 | 434,760 | 51.2 |
| 128 of `{id, 7,000 fields}`, 100 KB | 266 | 11,443 | 14.6 | 93,252 | 100,310 | 92,114 |
| 128 of `{id, text}`, 100 KB | 196 | 367 | 9.9 | 1,474 | 1,637 | 1,284 |
| 128 of `{rows, id}`, 10 KB | 382,007 | 13,921 | 13,571 | 526,823 | 42,338 | 41,815 |
| 25 of `{kind, list}`, 100 KB | 17.5 | 4,635 | 2,654 | 4,872 | 9,655 | 7,517 |
| 25 of `[id, ...]`, 100 KB | 5.5 | 4,630 | 2,645 | 4,787 | 9,626 | 7,617 |

Through a `Source`, one level at a time, the times follow `evaluate`: 128 of `{id, rows}` of 100
KB take 260 µs with the list and 65.2 µs now. `{id, 7,000 fields}` with `evaluate` is about the
same in all three, as both ways read the level of each item, which is the cost. The last two rows
are what is left: the difference is below a field all the items share, or in an array, so the
hash reads each item whole, about 6 ns per value. The time is one walk of the input, and linear,
where the list's grows with the square of the items; it costs from 1.3 to 1.6 times the list with
`evaluate` up to 128 such items of 10 KB or more, and with 128 of 1 KB it gains. Reaching that
difference would need first fields that follow the comparison into nested values, for items
whose whole cost is one walk of the input, so it stays recorded. The packages show the same,
before 0.27 #6 and now, the fastest of three processes in Python and two in Java:

| Items | Python objects, before | now | `Document`, before | now | Java maps, before | now | `Document`, before | now |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 25 of `{id, rows}`, 100 KB | 17.7 | 14.0 | 8.4 | 4.7 | 18.8 | 15.6 | 7.9 | 4.5 |
| 128 of `{id, rows}`, 100 KB | 240 | 53.1 | 220 | 8.5 | 219 | 57.4 | 211 | 7.7 |
| 128 of `{id, 7,000 fields}`, 100 KB | 79,741 | 78,325 | 300 | 13.6 | 91,217 | 86,630 | 273 | 13.9 |
| 128 of `{rows, id}`, 100 KB | 3.14 s | 409 ms | 2.51 s | 121 ms | 2.62 s | 491 ms | 1.97 s | 103 ms |
| 25 of `{kind, list}`, 100 KB | 9,900 | 13,013 | 17.1 | 2,660 | 8,640 | 10,385 | 13.3 | 1,851 |

The workload has a case of this now, `distinct items`, `$count($distinct(items))` over the 200
items of the orders: `cargo xtask ab` before 0.27 #6 and now, 656 → 176 µs with `evaluate`, 489 →
18.3 µs against a `Document`, 671 → 187 µs through a `Source`; `python.py ab`, 458 → 15.2 µs
against a `Document` and 595 → 148 µs over Python values; `java-compare ab`, 408 → 14.6 µs and 627
→ 193 µs over maps; `compare.mjs`, 25.9 µs against a `Document` where jsonata-js takes 9.17 ms.
`ab` against 0.27 #6 finds that case 26.6% faster against a `Document`, with `evaluate` and through
a `Source` −1.4 and −1.9%, and every other case within −1.7 and +2.7%, none past the floor. The
benchmark of the application that reported #5 and #6 runs no `$distinct`, and finds −0.1 to −0.5%.
Measured with short arrays too: from 10 to 25 items of any kind within 2.9% of 0.27 #6, past them
numbers +3.9% at most and strings +3.0%, under the 8.9% that copies of one program spread, and
objects up to −4.2%, nested ones up to −13.5%, the medians of seven processes against a
`Document`; with `evaluate`, at 25 and 1,000 items, within +1.4%.

A value can hold one array many times, at each of many levels, which `=` passes over by
comparing pointers. So the hash has a budget of 64 values per item, and past it remembers
arrays and objects by address, and the time stays linear in the array, however many paths lead
to a value. A value nested past the limit of 1,000 levels is not hashed: whether comparing it
fails depends on what it meets, so the items from there on go back to comparing, which gives the
same result and the same error. An array from a `Source` past 16 distinct items that the first
fields do not settle has each item read whole, as a hash needs. A level that cannot be read stays
unread and the items from there go back to comparing, so that it fails the evaluation only if
comparing reaches it
([ADR 0010](../src/adr/0010-lazy-input.md#amendment-a-level-read-in-case-it-is-needed-027)).

## Names in a frame (0.27 #7)

A name of up to 14 bytes now stays in the frame's entry, which takes no allocation, and a frame with
one name holds it with no list, which takes out a second one. An `Arc<str>` in the syntax tree was
not built: it would count on one cache line for every thread at each binding, what 0.26 #20 got
rid of. A name of 15 bytes or more is a pointer to a string, one allocation more than the copy it
replaces.

On Windows with `mimalloc`, which the packages use, built aligned, one process per build, the median
of seven alternated runs, each on a new copy, against a `Document` with two variables bound, on the
200 items of the workload:

| Case, per evaluation | Before | After | | Allocations |
|---|---:|---:|---:|---:|
| A block of one name in a lambda | 79.7 µs | 75.5 µs | −5.3% | 1,023 → 621 |
| A block of two names in a lambda | 105.0 µs | 105.4 µs | +0.4% | 1,223 → 821 |
| A name bound in a lambda with no block | 57.7 µs | 59.2 µs | +2.6% | 1,023 → 621 |
| A name of 34 bytes in a lambda's block | 80.5 µs | 78.9 µs | −1.9% | 1,023 → 1,021 |
| A block at the top, one name | 318 ns | 288 ns | −9.4% | 4 → 2 |
| A block of nine names | 1,117 ns | 1,105 ns | −1.1% | 14 → 5 |
| Four nested blocks, one name each | 881 ns | 789 ns | −10.4% | 13 → 5 |
| A tuple's focus, `items@$i` | 135.9 µs | 133.7 µs | −1.6% | 4,658 → 3,854 |
| A tuple's index, `items#$n` | 81.3 µs | 79.2 µs | −2.6% | 2,847 → 2,443 |
| Two host variables | 299 ns | 283 ns | −5.4% | 4 → 2 |
| A built-in looked up | 548 ns | 499 ns | −8.9% | 12 → 10 |
| `recursion`, which binds one name per evaluation | 708 µs | 697 µs | −1.5% | 3,952 → 3,950 |

Threads sharing an expression, a recursion with a block in the lambda, evaluations per second, the
median of seven alternated runs:

| Threads | Before | After | |
|---:|---:|---:|---:|
| 1 | 3,840 | 4,082 | +6.3% |
| 4 | 14,660 | 15,590 | +6.3% |
| 8 | 26,742 | 28,820 | +7.8% |
| 16 | 29,414 | 31,040 | +5.5% |

The time saved is less than the allocations suggest, as a copy of the name is not free, and an
allocation with `mimalloc` takes little: a block of one name makes two allocations less, about
21 ns of 398. Most of a tuple's allocations are not the frame's: each step builds a map of its
own, with a `Rc<str>` per key, which this item leaves as it was.

Names are compared by a loop, not by `memcmp`, which a call of the C runtime makes slower for
names this short: with `memcmp`, the block of nine names reads +6.4% where the loop reads −1.1%,
and a tuple's index +1.8% where the loop reads −2.6%.

`cargo xtask ab 7c624d8 .`, ten rounds, in percent, with the system allocator:

| | `recursion`: `evaluate` | `document` | `source` | `index in a variable`: `evaluate` | `document` | `source` |
|---|---:|---:|---:|---:|---:|---:|
| Windows | +2.8 | +2.6 | +2.5 | −3.2 | −6.1 | −3.7 |
| Windows, aligned, five rounds | +1.7 | +0.6 | −0.3 | −5.7 | −6.0 | −4.1 |
| Linux | −1.2 | +0.2 | −0.8 | −1.3 | −1.0 | −1.6 |

No case reaches the mark of 3.5%, in any mode, on Windows or under Linux. The largest are
under Linux from a source, built as usual, `strings` +3.4% and `shipping rule` +3.2%, and from a
`serde_json::Value` `filter + aggregate` +3.0%, within the floor and not hunted. The aligned runs
above were read on six cases, those two, `path`, `object constructor`, `transform` and
`shipping rule`: none takes more than 2.2% longer.

The first form of the name took 24 bytes, a `Box<str>` beside 22 bytes in place. `ab` read
`recursion`, which runs none of that code, at +5.7%, and aligned at +0.2%: placement. But the
probe with `mimalloc` kept +3.5 to +4.2% aligned, where a change with no effect read −0.8%. Going
by steps, the name alone took +0.9%, and the variants of a frame with no name or with one took the
rest, with or without a call's frame tested first, and with the binding kept out of `eval`, so
neither of those is the cause. A name of 16 bytes, with a long name behind one pointer, leaves the
frame's variables room and reads +0.4%, and so was kept. At 24, the new variant filled the 48 bytes
of its type exactly, where its discriminant has to go; that this is the reason was not traced.

## Over a long text (0.27 #10)

`$match`, `$replace` and `$split` with a regular expression grew with the square of the text:
each match counted its UTF-16 offsets from the start of the text, and `$replace` and `$split`
encoded the whole text again for each piece. A match now counts from where the last one ended,
and keeps that place in both units; the pieces are cut with a place that moves forward with the
matches. On words in Portuguese, Greek, Japanese and English (`/\w+/` for `$match` and
`$replace`, `/\s/` for `$split`; the fastest of three runs, and of five in jsonata-js; Node.js
24), with `evaluate` and the same results as jsonata-js:

| Text | Function | Matches | Before | After | jsonata-js |
|---:|---|---:|---:|---:|---:|
| 20 KB | `$match` | 1,558 | 28.4 ms | 2.46 ms | 0.66 ms |
| 20 KB | `$replace` | 1,558 | 58.2 ms | 2.04 ms | 1.01 ms |
| 20 KB | `$split` | 2,075 | 78.8 ms | 2.37 ms | 0.71 ms |
| 80 KB | `$match` | 6,222 | 410 ms | 9.75 ms | 1.22 ms |
| 80 KB | `$replace` | 6,222 | 922 ms | 8.14 ms | 2.45 ms |
| 80 KB | `$split` | 8,296 | 1.23 s | 9.31 ms | 1.77 ms |
| 320 KB | `$match` | 24,886 | 6.47 s | 40.3 ms | 6.47 ms |
| 320 KB | `$replace` | 24,886 | 14.6 s | 32.4 ms | 10.4 ms |
| 320 KB | `$split` | 33,182 | 19.2 s | 36.7 ms | 6.79 ms |

The time is now that of the matches, 1.1 to 1.6 µs each at every size, against 0.2 to 0.4 µs in
jsonata-js. The search is little of it: `find_from` for each match takes 105 ns for `\w+` and
70 ns for `\s`, where an iterator that keeps its backtracker takes 30 and 35 ns. The rest is
outside the engine: an object of five fields for each match, with its keys, strings and `next`,
and the call of that `next`. 0.27 #11 took that on, [below](#without-an-object-for-each-match-027-11).

The workload has a case of this now, `regex over a long text`, `$replace` on 20 KB. `cargo xtask
ab` against the code before, five rounds: 55.1 → 2.07 ms with `evaluate` and 55.2 → 2.05 ms
against a `Document`; `regex`, a short text per match, −1.1% and −0.2%, and `strings`, which runs
none of this, +1.1% and +1.0%.

## Without an object for each match (0.27 #11)

A regex literal is a function, and `$match`, `$replace`, `$split` and `$contains` called it back
for the first match and the `next()` of each match object for the rest, as jsonata-js does. When
the matcher is a regex literal, they now run its searches themselves (`regex::Matches`) and build
only what their result holds: `$split` and `$contains` no string at all but the pieces,
`$replace` with a replacement string the text it returns, and `$match` its objects of three
fields, whose keys the regex makes once. Each search still costs what the call did, its step and
its checks included, and is found as `next()` finds it, with the same `D1004` for an empty match
and the search past a limit; the step counts of `steps.json` are those of the code before. A
function as the replacement is given the match object, so it goes through the objects, whose
five keys the regex now also makes once; so does a matcher of the expression's own.

On the text of #10 (`text-20k.json` repeated; the fastest of seven runs, and of nine in
jsonata-js; Node.js 24):

| Text | Function | Matches | Before | After | jsonata-js |
|---:|---|---:|---:|---:|---:|
| 20 KB | `$match` | 1,558 | 2.81 ms | 1.09 ms | 0.48 ms |
| 20 KB | `$replace` | 1,558 | 2.30 ms | 0.36 ms | 0.58 ms |
| 20 KB | `$split` | 2,075 | 2.61 ms | 0.45 ms | 0.44 ms |
| 20 KB | `$replace` with a function | 1,558 | 2.30 ms | 1.78 ms | 1.85 ms |
| 80 KB | `$match` | 6,232 | 10.8 ms | 4.22 ms | 1.06 ms |
| 80 KB | `$replace` | 6,232 | 9.12 ms | 1.46 ms | 1.93 ms |
| 80 KB | `$split` | 8,300 | 10.3 ms | 1.72 ms | 1.74 ms |
| 80 KB | `$replace` with a function | 6,232 | 9.13 ms | 6.90 ms | 6.66 ms |
| 320 KB | `$match` | 24,928 | 44.8 ms | 17.3 ms | 5.12 ms |
| 320 KB | `$replace` | 24,928 | 36.4 ms | 5.72 ms | 8.44 ms |
| 320 KB | `$split` | 33,200 | 40.7 ms | 6.83 ms | 6.55 ms |
| 320 KB | `$replace` with a function | 24,928 | 35.7 ms | 27.1 ms | 28.2 ms |

Per match, `$replace` 1.47 → 0.23 µs and `$split` 1.26 → 0.22 µs, of which the search is 105 and
70 ns, against 0.37 and 0.21 µs in jsonata-js; `$replace` with a function 1.48 → 1.14 µs, as
jsonata-js. `$match` 1.80 → 0.70 µs, against 0.21 to 0.30 µs: what is left is the object it
returns for each match, five allocations (the match's string, the object, its map's table and
entries, and the list of groups), each freed again by `$count`. `cargo xtask growth` agrees on
its own text (`/o/` and `/ /`, 20 to 320 KB): `$replace` −87%, `$split` −85%, `$match` −68%,
`$replace` with a function −26%, `$contains` within its noise.

The two candidates of the row were measured apart where they apply alone: the keys made once took
`$replace` with a function from 1,502 to 1,152 µs on `growth`'s 20 KB, where the loop does not
reach; the loop alone, before them, took `$replace` 1,625 → 214 µs, `$split` 2,042 → 308 µs and
`$match` 1,650 → 532 µs, which the keys left unchanged (532 → 535 µs, within its runs).

`cargo xtask ab` against the code before, ten rounds, in the three modes: `regex over a long text`
2.06 ms → 342 µs with `evaluate`, 2.04 ms → 338 µs against a `Document` and 2.04 ms → 341 µs
through a `Source`, −83%; `regex`, short texts, −11.2%, −13.1% and −11.4%; every other case −2.0
to +1.6%, none at the mark of 3.5%.

## Growth with the input (0.27 #13)

`$distinct` and the regular expression functions grew with the square of their input until 0.27,
and were found by chance. `cargo xtask growth` looks for more: it runs each case of
`benches/growth.json` at 1,000, 4,000, 16,000 and 64,000 items (`--sizes` takes others), texts of
ten bytes per item, with `evaluate` and against a `Document`, and prints for each step between two
sizes the exponent of its time, 1 for linear and 2 for quadratic. 131 cases cover every built-in
function that takes an array, an object or a text, and the operators, path steps, tuples,
grouping, transforms, calls in depth and compiling long expressions. `node
benches/compare/growth.mjs` runs the same cases in jsonata-js, on the inputs that `cargo xtask
growth --write-inputs` leaves in `target/growth`. Each size takes the fastest of at least three
runs and 40 ms, all in one process: a process can draw a slower heap for all its sizes
([above](#recursion-against-a-document-027-3)), which moves a case's times and not its exponents.

Fixed by the sweep, at 64,000 items with `evaluate`:

| Case | Before | After | jsonata-js |
|---|---:|---:|---:|
| `$keys` of objects with a key each | 5,411 ms | 26.0 ms | 36.3 ms |
| `$keys` of an object of 64,000 keys | 5,402 ms | 7.48 ms | 8.63 ms |
| `$sift` over that object, then `$keys` | 5,390 ms | 27.8 ms | 140 ms |
| `$merge` of objects with a key each, then `$keys` | 5,406 ms | 31.9 ms | 33.3 ms |
| group-by with a key per item, then `$keys` | 6,389 ms | 47.8 ms | 288 ms |
| `nums[$ in $$.rev]` | 25,612 ms | 1,032 ms | 995 ms |
| `-1 in nums`, against a `Document` | 380 µs | 40.8 µs | 21.1 µs |
| a regex of 64,000 alternatives, compiled | stack overflow | 39.5 ms | 0.90 ms |

- `$keys` looked each key it found up in the list of those found before. The keys of one object
  are distinct and are no longer looked up; past 16 keys, those of an array of objects are looked
  up in a set. The five cases above that call it all took about 5.4 s.
- `in` copied its list into a new vector at each evaluation, which a predicate does for each item.
- `jlens-regress` made a disjunction a chain of alternations as deep as its terms, which its
  recursive walks of the tree, and dropping it, followed down until the stack ran out, between
  1,000 and 4,000 terms on the main thread of Windows, aborting the process. Each walk also asked
  at each node whether a capture group lay below it, so compiling 800 terms took 1.35 ms, and
  grew as n^1.7. A disjunction of more than 64 terms is now a balanced tree, its terms tried in
  the same order: 1,000 terms take 0.51 ms, and the exponent is 1.05. Shorter disjunctions keep
  upstream's chain, so that a common pattern compiles to what it compiled to.

Left to rows of their own: `$substring` collected the characters of its whole text at each call,
0.78 ms on 640 KB, so that a call at each place of a text is quadratic, as it is in jsonata-js
(0.27 #15, [below](#substring-027-15)); `&` in a `$reduce` copies the string it grows, 1.42 s for 64,000 short strings where
jsonata-js, whose strings are ropes, takes 110 ms (#16, [below](#-in-a-loop-027-16)); the URL functions took 5 to 12 times
jsonata-js's time, linearly (#17, [below](#the-url-functions-027-17)).

What stays:

- Shapes quadratic in any engine: `$reduce` that builds an array with `$append` (19.9 s at
  64,000 items, jsonata-js 5.7 s) or an object with `$merge` (7.0 s at 16,000, jsonata-js 22.5 s),
  since each step copies what it has built, and `in` over a long list for each item, which
  compares with each item of the list. An expression that does this over large input can use
  `max_steps` or `max_array_length` to bound it.
- The step from 16,000 to 64,000 items reads 1.3 to 2.0 in cheap cases over the records (a path,
  `$lookup`, a predicate, `$keys` of objects of the same key) and in short copies (`$append`,
  `$reverse`, a range): the input or the result leaves the processor's caches there. From 64,000
  to 1,024,000 items, the same cases read 0.95 to 1.2, and a sort step 1.1, as n log n.
- Where jsonata-js grows faster than jlens, as measured: `$sort`, the sort step, `$distinct`,
  `$spread` of an array of objects, `$eval` and compiling a long array, `$map` with the array as
  the third argument, and an index in a loop. Its `$sort` of 64,000 numbers takes 9.9 s, against
  16.1 ms.

## The tuples of a path (0.27 #14)

A step with `@`, `#` or an ancestor label makes a tuple for each item: a map of its own, with the
fields of the tuple before it and the step's own. Counted with a global allocator that records where
each allocation comes from, against a `Document` of the workload's 200 items, `items#$n.code` made
2,637 allocations, about 13 for each item:

| For each item | Allocations | After |
|---|---:|---:|
| The keys of the tuples, `@` twice and `n` once | 3 | 0 |
| A list holding the step's result, when it is not an array | 1 | 0 |
| The two tuples | 2 | 2 |
| The copies of the tuples' maps, two allocations each | 4 | 4 |
| The frame of the second step, and its list | 2 | 2 |
| The result's string, no part of the tuples | 1 | 1 |

The keys are now made once for each step and shared by each tuple with `Rc::clone`, which counts
on no line another thread reads, as each evaluation makes its own; a step's result that is not an
array is its one item, with no list of its own; and the frame that binds a tuple's fields is made
at their number, where binding them in turn grew its list from two names to four when a tuple has
three. On Windows, the probe with `mimalloc`, which the packages use, the median of five alternated
runs, each on a new copy of the program:

| Case, against a `Document` | Before | After | | Allocations |
|---|---:|---:|---:|---:|
| A tuple's focus, `items@$i.{'code': $i.code}` | 158.6 µs | 145.0 µs | −8.5% | 4,038 → 3,238 |
| A tuple's index, `items#$n.code` | 99.2 µs | 87.5 µs | −11.8% | 2,637 → 1,838 |
| `tuples`: both, a filter and a group-by | 170.6 µs | 149.9 µs | −12.1% | 2,445 → 1,545 |
| A group-by with no tuple, `items{type: $sum(quantity)}` | 27.2 µs | 27.7 µs | +1.7% | |

`tuples` is a new case of the workload, `items#$n@$i[$n % 2 = 0]{$i.type: $sum($i.(unitPrice *
quantity))}`, which `ab`, `compare.mjs`, `python.py` and `java-compare` read. `cargo xtask ab
e8c49af .`, ten rounds, with the system allocator, where an allocation costs more:

| Mode | `tuples` before | After | |
|---|---:|---:|---:|
| `evaluate` | 250 µs | 207 µs | −17.2% |
| `document` | 237 µs | 194 µs | −18.2% |
| `source` | 263 µs | 220 µs | −16.2% |

The other cases read −3.3 to +1.6%, none at the mark of 3.5%.

The second candidate of the roadmap, a tuple with no `IndexMap`, was not built. It would take at most
the copies of the maps, four of the nine allocations left for each item, and the frame that copies a
tuple's fields again. But a tuple stream is an array of objects that filters, sorts, group-by and
`$` read as objects, so a tuple of its own is a change to each of them, larger than this item's one
session.

## `$substring` (0.27 #15)

`$substring` collected every character of its text into a list at each call, and sliced the list
as jsonata-js slices its array of code points. It now walks the text only as far as the result
needs: from the start to the end of the result, or, for a negative start, back from the end to
where the result starts. `Chars::nth` skips characters by counting the bytes that start one,
which takes a ninth of the time that decoding them did in the loop below, 1,489 → 166 ms. The result is the
text itself when it is all of it. One case still counts the characters, with no list: a negative
start or a length that is not a whole number, where the text's length enters the arithmetic in
floating point and can round, `1 + (1 - 2^-53)` being 2. A property test compares the bytes taken
with the list's result, 20,000 cases of texts with characters of one to four bytes, starts and
lengths whole and fractional, of either sign, past either end, infinite and NaN; reading a
fractional length as a whole one, or dropping the start's return to 0, fails it.

`cargo xtask growth --case '$substring'`, at 64,000 items, a text of 640 KB:

| Case | Mode | Before | After | jsonata-js |
|---|---|---:|---:|---:|
| `$substring(unicode, 100, 100)` | `evaluate` | 725 µs | 15.6 µs | 2,964 µs |
| | `Document` | 745 µs | 1.0 µs | |
| `$substring(unicode, -100)` | `evaluate` | 815 µs | 15.6 µs | 2,895 µs |
| | `Document` | 808 µs | 0.9 µs | |
| `$substring($$.text, $, 1)` at each place | `evaluate` | 69,492 ms | 1,120 ms | 24,228 ms at 16,000 |
| | `Document` | 70,415 ms | 166 ms | |
| the same over `unicode` | `Document` | 55,048 ms | 251 ms | 16,924 ms at 16,000 |

Against a `Document` a single call is the same at every size. With `evaluate`, about 15 µs of
each call is reading `$$.text` from the `serde_json::Value`, which copies the 640 KB text at each
read, where a `Document` converted it once: one call alone takes those 15.6 µs, and the loop takes
15 µs per call more than against a `Document`. That stays: a text read once, or bound to a
variable that holds it converted, pays it once. A call at each place is still quadratic in either
engine, as each walks to its place: at 16,000 items, 19.3 ms against a `Document`, where
jsonata-js, which reads the whole text at each call, takes 24.2 s. `cargo xtask ab` on the workload, which calls no `$substring`: `strings`
−0.2%, the rest −1.2 to +2.7% but `recursion` +3.4%, and built aligned +1.3%, the others within
±0.3%.

## `&` in a loop (0.27 #16)

`&` copied both of its sides into a new string at each call, so that a loop that grows a string
with it copied all it had built at each step. Appending in place, the candidate the row named,
needs the left side to be the only reference to its string, which it never is in such a loop:
the lambda's frame holds `$a` while its body reads it. So a long result of `&` now holds its two
sides, and makes its text once, the first time something reads it, as V8's ropes do
([ADR 0003](../src/adr/0003-value-model.md#amendment-a-string-can-be-two-strings-joined-027)). A
short side goes into the short text next to it, so that a loop of short pieces holds a node for
each 256 bytes or so, not one of about 88 bytes for each piece. That folding costs a little,
copying up to 256 bytes at a step: `$a & $b` over 64,000 strings takes 31 ms with it, 24 ms
without.

`cargo xtask growth --case '$reduce with &'`, at 64,000 strings of a few bytes, with `evaluate`
and against a `Document`:

| Case | Before | After | jsonata-js |
|---|---:|---:|---:|
| `$reduce(strs, function($a, $b){ $a & $b }, '')` | 1,062 / 1,419 ms | 31.4 / 28.3 ms | 99.4 ms |
| the same, `$b & $a` | 1,011 / 1,072 ms | 31.9 / 28.3 ms | 100 ms |
| `$a & $r.key & $r.val` over records | 9,361 / 9,285 ms | 84.5 / 84.0 ms | 347 ms |

The exponents went from 1.8 to 2.6 to 1.0 to 1.1, and the case lost its mark of shapes quadratic
in any engine. A loop that reads the string at each step still copies it at each step, as it
does in jsonata-js. Tests: a property test grows strings on either side, around themselves, read
at some steps and not others, with and without a limit on their length, lazily and against a
`Document`, against a copy at each step; walking the parts in the wrong order fails it. Two
unit tests hold 100,000 nodes, read and dropped with no recursion, which a recursive drop
overflows, and count a node for each 256 bytes of short pieces.

What it cost elsewhere, found by `ab`. Each read of a string tests which kind it is: `$distinct`
over objects read +5.4% against a `Document`, as comparing and hashing their fields tested both
sides each time; equality now takes two texts first, which brought it to +0.3%. The compiler's
drop of a `Value` grew with the second kind, and stopped being inlined into the drop of a list
of values, so that each item of a dropped array became a call: `$sum(items.quantity)` against a
`Document` read +4.6% on Windows and +10.1% under Linux, where `perf` put the call under the
drop of the array of numbers. An array whose items are all scalars is now freed without a drop
for each, which brought it to −3.2% and +0.2%. Dropping the items of other arrays by hand, each
by the drop of what it holds, took `$distinct`'s result back to +3.1% under Linux but cost
`items[0].code` over 10,000 items +4.0 to +4.6%, and was dropped; so under Linux, against a
`Document`, `distinct items` stays +5.7% (13.2 → 13.9 µs), the one case past the floor.

`cargo xtask ab` against the code before, ten rounds, the workload's other cases:

| Mode | Windows | Linux |
|---|---|---|
| `evaluate` | `strings` −6.0%, `sort + map` −5.0%, `recursion` −2.6%, the rest −1.7 to +0.9% | `strings` −5.4%, `sort + map` −3.0%, the rest −1.1 to +3.1% |
| `document` | `strings` −7.1%, `sort + map` −4.3%, `names over items` −3.2%, the rest −2.7 to +1.7% | `strings` −9.9%, `distinct items` +5.7%, `path` +4.1% (−1.2 to +4.1% over five runs), the rest −2.2 to +2.8% |
| `source` | `strings` −4.6%, `sort + map` −4.9%, the rest −2.7 to +0.7% | `sort + map` −1.9%, `strings` −1.6%, the rest −1.3 to +2.9% |

`strings` gains as `&` no longer copies its sides three times, into a `String`, into a longer
one and into the `Rc<str>`: a short result is made once.

## The URL functions (0.27 #17)

`$encodeUrlComponent` and `$encodeUrl` took a character at a time, looked it up in two short
strings, and formatted each escape with `format!`, a `String` allocated for each byte escaped.
They now look each byte up in a table of the 128 ASCII bytes, copy the runs of bytes kept as
they are whole, and write the two digits of an escape from a table, into a result whose length
is counted first. `$decodeUrlComponent` and `$decodeUrl` copied the text between escapes a
character at a time, read each escape's digits through `u8::from_str_radix`, and built a `Vec`
for each character past ASCII; they now copy the text up to the next `%` whole, read the digits
themselves, and gather a character's bytes in an array.

Reading the digits themselves fixed an error: `u8::from_str_radix` takes a sign, so that `%+1`
decoded to U+0001 where JavaScript, and jsonata-js, raise `D3140`.

`cargo xtask growth --case Url`, at 64,000 items, 640 KB of accented, Greek and Japanese words,
1.9 MB once encoded. Each case gives the `$length` of its result, and the two decoding cases
decode what the encoding one gives, so that their time holds the encoding's:

| Case | Mode | Before | After | jsonata-js |
|---|---|---:|---:|---:|
| `$encodeUrlComponent(unicode)` | `evaluate` | 82.5 ms | 2.75 ms | 27.4 ms |
| | `Document` | 82.1 ms | 2.78 ms | |
| `$encodeUrl(unicode)` | `evaluate` | 86.9 ms | 2.82 ms | 27.0 ms |
| | `Document` | 87.1 ms | 2.77 ms | |
| `$decodeUrlComponent($encodeUrlComponent(unicode))` | `evaluate` | 111.5 ms | 6.43 ms | 12.2 ms |
| | `Document` | 110.0 ms | 6.20 ms | |
| `$decodeUrl($encodeUrl(unicode))` | `evaluate` | 117.1 ms | 6.23 ms | 12.0 ms |
| | `Document` | 115.5 ms | 6.17 ms | |

Encoding takes 30 times less, about 4 ns a byte of its text; decoding alone, the difference of
the two cases, 29 to 30 ms before and 3.4 to 3.7 ms after, 8 times less. The exponents stay 1.0
to 1.2. In jsonata-js the encoding cases take longer than the decoding ones that hold them, as
its `$length` counts code points through an array, so that most of their time is counting the
1.9 MB encoded. A property test compares the escapes with those of the code before, 20,000
texts of every ASCII character and characters of two to four bytes, and decodes them back;
another decodes 20,000 texts of escapes well and badly formed, of reserved characters in either
case, of lone bytes and of signs, against the code before, but where an escape starts with `+`.
Dropping the run of kept bytes before an escaped `%`, or decoding `%23` in `$decodeUrl`, fails
them. The workload calls none of the four, so `cargo xtask ab` was not run for this row; #19
runs it on the release.

## The C API's library and link-time optimization (0.27 #20)

The release profile asks for `lto = true`, but `jlens-capi`, `jlens-python` and `jlens-wasm` are
each built as an `rlib` as well as a `cdylib`, for their tests, and with an `rlib` among a crate's
types `cargo build` leaves LTO out: `cargo build -v` shows no `-C lto`. `cargo rustc
--crate-type cdylib` builds the library alone, with it. `jar.yml` and `cargo xtask java` built the
C API with `cargo build`, so every JAR since 0.25.0 carried a library without LTO; both now build
it with `cargo rustc`. maturin passes `--crate-type cdylib` itself, so the wheels have always had
LTO. `cargo xtask wasm` built the module with `cargo build` too, until
[0.27 #21](#the-webassembly-module-and-link-time-optimization-027-21).

Without LTO, each crate is compiled apart and linked in an order of its own, and #16 moved that
layout to a slow one: against a `Document`, `$sum(items.quantity)` from Java took 6.21 to 6.37 µs
at d8d965c, the code of #19, where #16 with its `Text` cut back to one `Rc<str>` took 5.43 to 5.64
and `a5548b1`, before #16, 5.50 to 5.64. The work was the same, 3.2% more instructions for each
evaluation, but at 2.29 instructions a cycle instead of 2.55, with as many L1 misses for each
instruction and 10% more refills from L2, gathered on `IndexMap::get` and `bcmp`, whose code is
the same in both. Where the documents sit in the heap does not change it (the same documents
behind fillers of their size classes: 6.09 to 6.42 µs against 5.38 to 5.70 at each placement),
and the order of the code does: linked with lld's `--shuffle-sections`, three seeds of each, d8d965c
takes 5.38 to 5.56 µs and the one-`Rc<str>` build 5.30 to 5.53, every layout fast. Built with LTO,
d8d965c takes 5.27 to 5.38 µs, 5.27 to 5.45 linked by lld.

`java-compare ab` of 0.26.0's JAR, also built without LTO, against d8d965c's built with it, under
Linux, five rounds:

| Case | 0.26.0 | d8d965c with LTO | |
|---|---:|---:|---:|
| `items[0].name`, against a `Document` | 0.678 µs | 0.601 µs | −11.3% |
| `$count(items)`, against a `Document` | 0.623 µs | 0.524 µs | −15.9% |
| `names over items`, against a `Document` | 5.79 µs | 5.51 µs | −4.8% |
| `$sum(items.price)`, 1,000 items, against a `Document` | 25.6 µs | 25.0 µs | −2.1% |
| index into 10,000 items, against a `Document` | 0.652 µs | 0.579 µs | −11.3% |
| `recursion`, against a `Document` | 611 µs | 558 µs | −8.7% |
| `recursion`, over Java maps | 587 µs | 555 µs | −5.6% |
| `sort + map`, against a `Document` | 269 µs | 230 µs | −14.6% |
| `regex`, against a `Document` | 107 µs | 88.8 µs | −16.8% |
| `$sum(items.price)` from JSON text, 8 threads | 171,518 a second | 191,278 a second | +11.5% |

Against a `Document` and from JSON text into one, all 54 cases are faster, 43 of them marked,
and the six counts of threads 6.9 to 11.5% higher, five marked. Over Java maps, the small cases
are 7.5 to 14.9% faster; from JSON text into maps, where Jackson and the conversion in Java take
most of the time, the spike's cases are −1.4 to +1.8%; none of the 54 over maps is marked slower. The cases #19 found slower against a `Document`, `$sum(items.price)` +16% and `names over
items` +10 to +14%, are now 2 to 5% faster than 0.26.0, and `recursion` over Java maps, +3.7 to
+5.1% in #19, is 5.6% faster. Not timed on Windows or macOS, whose libraries `jar.yml` builds the
same way.

Found on the way, in both builds: each item `path` takes from the input stalls about seven times
on a store that cannot be forwarded to a load, as a `Value` returned is copied as its tag byte
and 16 bytes at offset 1, then read as 16 bytes at offset 0. It is work for each item, not the
regression, and is left for a session of its own.

## The WebAssembly module and link-time optimization (0.27 #21)

`cargo xtask wasm` built the module with `cargo build`, so that, as for the C API's library, the
`wasm-release` profile's `lto` was left out: every `jlens-wasm` published so far has no LTO. It now
builds the library alone with `cargo rustc --crate-type cdylib`, and `cargo rustc -v` shows `-C lto`.
After `wasm-opt -Os`, the module is 1,454,840 → 1,392,032 bytes, 542 → 518 KiB gzipped, against the
budget of 600 KiB; `jlens.global.js` 781 → 745 KiB gzipped.

`compare.mjs` with each package, one per Node.js process, the processes alternating, four each, on
Windows; the median of the four, the change from the module without LTO:

| Case | `evaluate` | `evaluateJson` | `evaluateDocument` | jsonata-js, the same in both |
|---|---:|---:|---:|---:|
| path | −9.4% | −19.6% | −7.1% | −9.5% |
| names over items | −7.8% | −15.5% | −23.5% | −0.2% |
| filter + aggregate | −4.4% | −16.7% | −9.9% | 0.0% |
| group-by | −5.7% | −15.8% | −11.6% | +1.3% |
| sort + map | −5.8% | −11.2% | −9.1% | +2.4% |
| strings | −7.8% | −13.7% | −11.1% | +2.7% |
| regex | −9.2% | −13.0% | −7.2% | +1.4% |
| number formatting | −6.2% | −15.2% | −6.1% | −1.9% |
| object constructor | −12.3% | −16.7% | −20.5% | −0.8% |
| transform | −17.9% | −16.1% | −18.5% | +0.8% |
| recursion | −1.6% | −3.9% | −2.7% | −0.1% |
| shipping rule | −3.7% | −14.2% | −11.3% | −0.6% |
| index into 10,000 items | −6.7% | −19.2% | −5.3% | −2.2% |
| index in a variable | −5.6% | −18.6% | −8.7% | −1.5% |
| regex over a long text | −11.2% | −8.2% | −12.5% | +1.1% |
| distinct items | −16.6% | −17.2% | −12.0% | +1.3% |
| tuples | −10.0% | −15.6% | −10.9% | −1.7% |

Every cell of jlens is faster, `recursion` the least. jsonata-js, in the same processes, moves
between −2.2% and +2.7%, but for `path`, 2.10 → 1.90 µs, which is a rounding of the tenth of a
microsecond: `evaluate`'s `path` 4.80 → 4.35 µs and `evaluateDocument`'s 1.40 → 1.30 µs are as
close to it. Parsing is where the gain is largest: `Document.fromJson` of the order 276 → 219 µs
(−21%), `new Document` from its object 319 → 264 µs (−17%), so `evaluateJson`, which parses at
each call, gains the most, the index into 10,000 items 4.92 → 3.98 ms. The transform 790 → 648 µs
with `evaluate`. The table above keeps 0.26.0's numbers until the release measures it again.

The cost is stack. The inlining LTO adds makes the evaluator's frames larger, so that recursion
overflows V8's stack sooner: an audited recursion, measured as the [WebAssembly
guide](../src/guides/wasm.md#limits-and-memory) does, overflows 4 to 9% sooner, order-by 716 → 663 levels, the
transform 746 → 678; unaudited, 7 to 8%, order-by 899 → 831. The default `maxDepth` of 500 still
leaves a third of headroom on the shape with the largest frames, and the test that runs it twenty
times, so that V8 optimizes it, passes; the guide's table now gives the depths of the module with
LTO.

## The documents of ADR 0021's spike on the JVM (0.27.0)

On the documents of ADR 0021's spike, 1,000 and 100,000 items of five fields each, with the input
converted once:

| Expression | jsonata-java 0.9.10 | jsonata-jvm-compiler 1.0.7 | jlens `evaluate(Object)` | jlens `evaluate(Document)` |
|---|---:|---:|---:|---:|
| **1,000 items (97 KB of JSON)** | | | | |
| `items[0].name` | 85 µs | 0.05 µs | 1.8 µs | 0.61 µs |
| `($i := 0; items[$i].name)` | 153 µs | 0.68 µs | 2.1 µs | 0.83 µs |
| `$count(items)` | 86 µs | 0.05 µs | 1.5 µs | 0.52 µs |
| `$sum(items.price)` | 171 µs | 8.6 µs | 143 µs | 25 µs |
| `items[price > 98].id` | 328 µs | 13 µs | 257 µs | 101 µs |
| `$` | 154 µs | 0.04 µs | 0.75 µs | 443 µs |
| **100,000 items (10,099 KB of JSON)** | | | | |
| `items[0].name` | 36.0 ms | 0.05 µs | 1.9 µs | 0.62 µs |
| `($i := 0; items[$i].name)` | 43.7 ms | 268 µs | 2.2 µs | 0.83 µs |
| `$sum(items.price)` | 55.7 ms | 4.99 ms | 21.0 ms | 5.11 ms |
| `$` | 70.4 ms | 0.04 µs | 0.74 µs | 49.2 ms |

From JSON text:

| Expression | jsonata-java 0.9.10 | jsonata-jvm-compiler 1.0.7 | jlens `evaluateFromJson` |
|---|---:|---:|---:|
| **1,000 items (97 KB of JSON)** | | | |
| `items[0].name` | 774 µs | 768 µs | 337 µs |
| `($i := 0; items[$i].name)` | 845 µs | 771 µs | 334 µs |
| `$count(items)` | 776 µs | 768 µs | 216 µs |
| `$sum(items.price)` | 861 µs | 810 µs | 363 µs |
| `items[price > 98].id` | 1.04 ms | 821 µs | 507 µs |
| `$` | 817 µs | 756 µs | 1.23 ms |
| **100,000 items (10,099 KB of JSON)** | | | |
| `items[0].name` | 109 ms | 75.3 ms | 44.1 ms |
| `($i := 0; items[$i].name)` | 86.3 ms | 75.9 ms | 36.5 ms |
| `$sum(items.price)` | 112 ms | 82.4 ms | 39.8 ms |
| `$` | 132 ms | 74.7 ms | 144 ms |

- On an input converted once, an index or `$count` takes the same time on 100,000 items as on
  1,000 in jlens, and in jsonata-jvm-compiler but for the index in a variable (0.68 µs, then
  268 µs). jsonata-java takes time in proportion to the items: 85 µs on 1,000 and 36 ms on
  100,000 for `items[0].name`.
- `$` returns the input. jlens over Java maps and jsonata-jvm-compiler return it as it is, in
  under a microsecond; jsonata-java takes 154 µs and 70 ms. jlens over a `Document` builds the
  whole document as Java objects, 443 µs on 1,000 items and 49 ms on 100,000, and from JSON text
  too, which only there takes longer than the others (144 against 75 and 132 ms).
- From JSON text, jlens takes about half the time of the others on 100,000 items, 37 to 44 ms
  against 75 to 112, but on `$`.
