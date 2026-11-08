# Performance notes: 0.31

How the measurements of roadmap 0.31 were made: methods, traps and intermediate numbers that the
[performance page](../src/architecture/performance.md) leaves out. The item numbers are those of
the plan, whose table the tag keeps in
[the roadmap](https://github.com/thkprado/jlens/blob/v0.31.0/docs/src/architecture/roadmap.md#031-a-small-map-for-objects);
[the roadmap's history](../src/architecture/roadmap-history.md#031) sums it up.

## What objects hold at 0.30.0 (0.31 #1)

Every object is an `Rc<Object>` whose fields are a `Map`, an `IndexMap<Rc<str>, Value>` with
foldhash: a vector of entries and a hash table of their indices, each allocated apart. On 64-bit
Windows, a `Value` is 24 bytes, an `Object` 104 (its `Map` 64), so an `Rc<Object>` is a block of
120 bytes; an entry is 48 (the hash, the key, the value), and the table of a map of three fields
52.

### How it was counted

Two programs outside the workspace, both against a `Document` of the workload's input, as `cargo
xtask ab --mode document` evaluates:

- A copy of `jlens-core` at v0.30.0 with a hook that records, in a table of its thread, the number
  of fields of each object when it is dropped, of each object `Object::get` looks a key up in (a
  path step), of each tuple a binding is read from, and of each pair of objects `deep_eq`
  compares. The workload's documents are counted apart: parsed, dropped, counted.
- `jlens-core` at 0.30.0, unchanged, under a global allocator that counts allocations and, for one
  evaluation, their sizes. Each case is evaluated once to warm up, once to count, then timed: 15
  samples of about 20 ms each, the median. Each program ran three times, each run on a new copy
  of it, with the system allocator and with `mimalloc`, alternated; the tables give the median of
  the three.

### Field counts

The workload's documents: `orders-200.json` has 200 items of 6 fields, its root of 3 and a customer
of 2; `items-10000.json` 10,000 items of 2 fields; `text-20k.json` one object of 1. The datasets of
the JSONata test suite, for a second view, hold 483 objects: 86% have 3 fields or fewer, 99.6% 6
or fewer, the largest 11.

| Case | Objects built, by fields | Lookups (`Object::get`), by fields | Other |
|---|---|---|---|
| `path` | none | 2 (2 and 3) | |
| `names over items` | none | 200 of 6, 3 of 3 | |
| `filter + aggregate` | none | 367 of 6, 1 of 3 | |
| `object constructor` | 200 of 3 | 600 of 6, 2 of 3 | |
| `group-by` | 3 of 2 | 600 of 6, 2 of 3 | |
| `transform` | 200 copies of 6, 200 updates of 1, 2 more | 400 of 6, 1 of 3 | |
| `distinct items` | none | 1 of 3 | 120 objects of 6 compared by `deep_eq` |
| `tuples` | 200 tuples of 3, 3 of 2, 1 of 1 | 300 of 6, 1 of 3 | 400 tuple bindings read from tuples of 3 |
| `$count($match(text, /\w+/))` | 1,558 of 3 | 1 of 1 | |
| `items#$n.code` | 400 tuples of 2 | 200 of 6 | 400 bindings read from tuples of 2 |

No object the workload builds, converts or reads has more than 6 fields. Lookups are as frequent as
construction: a path over the items hashes its key once for each item, in objects of 6 fields.

### Allocations and times

| Case | Allocations | of the evaluation | System heap | `mimalloc` |
|---|---:|---:|---:|---:|
| `path` | 2 | 1 | 0.25 µs | 0.20 µs |
| `names over items` | 13 | 13 | 6.3 µs | 5.4 µs |
| `filter + aggregate` | 22 | 22 | 44.0 µs | 42.9 µs |
| `object constructor` | 2,807 | 1,406 | 303 µs | 137 µs |
| `group-by` | 132 | 120 | 83.3 µs | 77.6 µs |
| `transform` | 5,037 | 2,824 | 448 µs | 298 µs |
| `distinct items` | 31 | 31 | 16.8 µs | 15.0 µs |
| `tuples` | 1,545 | 1,541 | 193 µs | 148 µs |
| `$count($match(text, /\w+/))`, 1,558 matches | 6,290 | 6,290 | 575 µs | 357 µs |
| `$count($match(text, /(\w)(\w*)/))` | 14,090 | 14,090 | 957 µs | 572 µs |

"Allocations" counts one evaluation with `evaluate_document`, the result converted to JSON; "of the
evaluation" the same evaluation with `evaluate_document_to` and a writer that drops the result. The
times include the conversion, as `ab` does. Where the heap is the system's, an allocation and its
free cost about 57 ns ([0.28](perf-0.28.md#match-and-the-allocations-of-each-match-028-4)); with
`mimalloc`, `object constructor` takes 55% less time and `$match` 38% less, while the cases that
allocate little differ by 2 to 20%.

The sizes tell the allocations of one object apart:

- `object constructor`, 7 for each object: the `Rc<Object>` (120 bytes), the map's table (52) and
  entries (144, three of 48), each of the three keys (24 each: `Rc::from` of the literal key in
  `object_of_item`), and the string `'item'` (24), made again for each object from its literal.
  `{'code': code, 'total': …}` takes 6, `{'code': code, 'kind': 'item'}` 7.
- `$match`, 4 for each match: the `Rc<Object>`, the table, the entries, and the matched string.
  Its keys are shared already.
- A tuple: the `Rc<Object>`, the table and the entries; the step's keys are shared (0.27 #14).
- A `transform` copy of an item: the same three, the entries for 6 fields (288 bytes, then 336
  when `total` makes 7).

So of the 7 allocations of an object built by a constructor, the map's hash table is one, and the
literal keys and string are four, made again for each object. A map without a hash table takes one
allocation from each object, two if its fields are kept in the object itself; sharing literal keys
and strings takes up to one per literal field of a constructor.

## The map's threshold and layout (0.31 #2)

Two scratch programs outside the workspace (release, `lto`, one codegen unit), with a stand-in
for `Value` of 24 bytes (a tag, a number or an `Rc<str>`). Each time is the median of 15 samples
of about 20 ms, the inner loop run 256 times between two readings of the clock; each program ran
twice, each run on a new copy of it. The decision and its summary are in
[ADR 0022](../src/adr/0022-small-map.md).

**Search in order against foldhash.** A vector of `(Rc<str>, V)` searched comparing lengths then
text, against `IndexMap<Rc<str>, V>` with foldhash, for 1 to 64 fields of 64 keys of the
workload's kind (`id`, `code`, `customer`, `quantity`, …, 2 to 11 bytes). The keys looked up are
other allocations of the same text, as a path step's key is. "Present" is the mean over every
key of the map; "absent" over four keys it does not have; "building" makes a map of _n_ fields
with a check for duplicates (`with_capacity` for both), then drops it.

| Fields | Present: order / foldhash | Absent | Building, system heap | Building, `mimalloc` |
|---:|---:|---:|---:|---:|
| 1 | 5.1 / 6.5 ns | 2.3 / 4.5 ns | 35 / 86 ns | 10 / 51 ns |
| 2 | 5.4 / 11.0 | 3.4 / 5.7 | 38 / 100 | 26 / 66 |
| 3 | 6.3 / 10.9 | 4.5 / 5.7 | 43 / 115 | 19 / 73 |
| 4 | 6.4 / 11.1 | 4.8 / 5.6 | 47 / 129 | 36 / 108 |
| 6 | 7.5 / 11.0 | 5.7 / 5.8 | 60 / 164 | 37 / 134 |
| 8 | 8.6 / 11.2 | 7.1 / 5.8 | 75 / 192 | 65 / 178 |
| 10 | 10.0 / 11.2 | 8.5 / 6.3 | 97 / 224 | 77 / 200 |
| 12 | 11.4 / 11.2 | 10.4 / 5.8 | 128 / 256 | 114 / 234 |
| 16 | 13.4 / 11.7 | 14.2 / 5.8 | 189 / 319 | 192 / 298 |
| 24 | 17.0 / 11.5 | 19.5 / 6.0 | 338 / 439 | 335 / 433 |
| 32 | 19.2 / 11.6 | 23.7 / 5.9 | 616 / 552 | 595 / 549 |
| 64 | 36.6 / 12.1 | 44.1 / 6.3 | 2,302 / 1,048 | 2,163 / 1,067 |

The table gives the first run; the second read lookups up to 13% slower (at 8 fields, foldhash 16.6
ns for a key present) and building within 7%.
A key present is found as fast either way at about 12 fields, one absent at about 7; building
in order is faster up to about 24. 8 is the threshold: past it, the map keeps an index.

**Fields kept in the object.** 200 objects shaped like an `Object` (a `RefCell` of the fields and
its 32 other bytes) in an `Rc`, built with _n_ fields and dropped, then the last key looked up in
each; the fields in a `Vec` with its capacity, or in a `SmallVec` of 3, 4 or 6 kept in the
object.

| Fields | Layout | Bytes | Build and drop, system heap | `mimalloc` | Lookup |
|---:|---|---:|---:|---:|---:|
| 2 | `Vec` | 72 | 86 to 95 ns | 41 ns | 5.4 ns |
| 2 | 3 inline | 184 | 67 to 73 | 42 | 5.6 |
| 2 | 4 inline | 224 | 74 to 75 | 37 to 43 | 5.4 |
| 3 | `Vec` | 72 | 93 to 104 | 43 | 9.2 |
| 3 | 4 inline | 224 | 85 to 87 | 44 to 53 | 9.1 |
| 6 | `Vec` | 72 | 112 to 133 | 68 to 69 | 8.0 |
| 6 | 4 inline | 224 | 149 to 152 | 72 to 93 | 8.8 |
| 6 | 6 inline | 304 | 101 to 123 | 81 to 88 | 8.4 |

On the system heap, where one run differs from the other by up to 10%, fields kept in the object
save 10 to 20 ns per object of 2 or 3 fields; with `mimalloc`, nothing. An object that holds more
than its inline fields costs 20 to 40 ns more, and a small object takes three times the memory.
The fields stay in the vector. With the map of #2, an `Object` is 72 bytes (104 with `IndexMap`).

## The map in place (0.31 #3)

`Map` became the map of every object, tuple and `$match` result. The hasher that the lazy
values of one evaluation shared went with `IndexMap`. Three operations the trial swap of #2 had
not shown were added for the object constructor (`insert_new`, `retain`, a field by position), and
`cloned_with_room` for the tuple step.

**Tuples.** The first count saved 104 allocations of `tuples`, not the 204 of its tuples: a
tuple step copies the tuple it extends with exactly its fields, then adds 1 to 3, and the copy's
vector grew again for them. The step now copies it with room for the fields it adds; `tuples`
went from 1,441 to 1,241 allocations, 13.7% faster than 0.30.0 instead of 6.7%.

**Allocations,** counted as in #1 (one evaluation with the result converted to JSON, then
without it):

| Case | 0.30.0 | #3 | of the evaluation, 0.30.0 | #3 |
|---|---:|---:|---:|---:|
| `object constructor` | 2,807 | 2,607 | 1,406 | 1,206 |
| `group-by` | 132 | 129 | 120 | 117 |
| `transform` | 5,037 | 4,635 | 2,824 | 2,422 |
| `tuples` | 1,545 | 1,241 | 1,541 | 1,237 |
| `$count($match(text, /\w+/))` | 6,290 | 4,732 | 6,290 | 4,732 |
| `$count($match(text, /(\w)(\w*)/))` | 14,090 | 12,532 | 14,090 | 12,532 |

One allocation fewer for each object built; `path`, `names over items`, `filter + aggregate` and
`distinct items` allocated no object and are unchanged.

**`cargo xtask ab` against 0.30.0,** 10 rounds, the median (the table of the performance page's
cases that moved by more than the floor):

| Case | `document` | `evaluate` | `source` |
|---|---:|---:|---:|
| `path` | -3.0% | -9.1% | -6.8% |
| `names over items` | -18.7% | -3.9% | +0.3% |
| `filter + aggregate` | -2.6% | +1.8% | +1.7% |
| `group-by` | -3.8% | +2.4% | +1.5% |
| `object constructor` | -6.6% | -6.9% | -4.6% |
| `transform` | -6.4% | -1.9% | -2.3% |
| `shipping rule` | -8.0% | +1.3% | +0.8% |
| `index into 10,000 items` | -1.9% | -5.8% | -5.7% |
| `distinct items` | -4.9% | -20.0% | -16.9% |
| `tuples` | -13.7% | -10.8% | -9.7% |

The other cases are within 2%. `distinct items` with a converted input compares the items read
from JSON, and each object read whole no longer builds a hash table.

`filter + aggregate` and `group-by` are 1.5 to 2.4% slower when the input is converted lazily, in
three runs of 10 and 20 rounds, and faster against a `Document`. Their lookups do not reach the
map: a lazy object reads a scalar from its input each time and keeps only arrays and objects. With
the functions and blocks aligned (`-align-all-functions=6 -align-all-nofallthru-blocks=5`),
`filter + aggregate` was +1.3% and +0.6%, `group-by` -0.1% in `source` and bimodal in `evaluate`
(median +6.4%, fastest round -0.4%). Recorded as placement, at the floor. Aligned, `recursion`
was 3 to 4% slower in every mode, and 1% faster alone in a process unaligned: it builds no
object, and its time follows where the heap puts its frames
([0.27](perf-0.27.md#recursion-against-a-document-027-3)).

**Python** (`python.py ab`, 5 rounds, Windows): nothing slower past the mark. Marked faster with a
`Document`: `object constructor` -13.3%, `distinct items` -17.6%, `tuples` -11.9%, `names over
items` -13.6%, `shipping rule` -10.1%, `transform` -9.5%; from Python values, `transform` -8.6%,
`distinct items` -10.9%, `tuples` -8.0%. `group-by` from Python values +2.4%, unmarked.

**Java** (`java-compare ab`, 5 rounds, Windows, the machine in use: spreads of 10 to 15% where
3% is usual): nothing marked slower. Marked faster: `$sum(items.price)` over 100,000 converted
items -40.9%, `object constructor` -14 to -17%, `transform` -12 to -15%, `distinct items` from
JSON text -19%, one thread of `$sum(items.price)` +11.9% evaluations per second. `recursion` +3.5
to +4.1% in its four variants, unmarked: the placement above.

## Objects compared in pairs (0.31 #4)

`deep_eq` looked each key of one object up in the other. Two objects of one shape hold their keys
in one order, so their fields are now compared in pairs, and a key is looked up only from the
first pair whose keys differ (a key read from JSON is shared, so most pairs compare a pointer).
The values compared, and their order, are those of the lookups, so results and errors do not
change. `$distinct` hashes objects without lookups and compares them with `deep_eq`.

**`cargo xtask ab` against #3** (c52b841), 10 rounds, the median; the cases below the line were
added to `cases.json` for the run only:

| Case | `evaluate` | `document` | `source` |
|---|---:|---:|---:|
| `distinct items` | -1.9% | -3.8% | -1.3% |
| `group-by` | -0.0% | -0.5% | -0.7% |
| `$count($distinct(items.{'type': type, 'color': color}))` | +0.1% | -0.8% | -0.1% |
| `$count(items.{'type': type, 'color': color}[$ = {'type': 'apparel', 'color': 'black'}])` | +0.5% | -0.7% | -1.4% |
| `$count($distinct($append(items, items)))` | +0.5% | -1.2% | |
| `$count(items[$ = $$.items[0]])` | +0.7% | -0.1% | |
| the same `$distinct` over 200 rows of 12 fields | -14.9% | -44.4% | |

Up to 8 fields a lookup hashes nothing and skips most keys on their length (the workload's six
keys have four lengths), so pairs gain little there: building the objects takes the time. Past 8
fields each lookup hashed its key, which pairs no longer do.

## Literal keys and strings made once (0.31 #5)

A string literal made a new `Rc<str>` each time it was evaluated: the literal keys of an object
constructor for each object, and `'item'` in `{'kind': 'item'}` too. An `Expression` is shared
between threads, so its tree cannot hold the `Rc<str>` a value takes. The evaluation now keeps the
string of each literal it evaluates in a vector indexed by node id, made the first time, and
clones it afterwards. Node ids are unique only within one tree, and `$eval` evaluates trees of its
own whose ids start at 0 as well, so each use compares the kept string's text with the literal's
(a length, then a few bytes); a different text makes a string again and takes the place. Ids
past 65,536 are not kept, so that a tree built by hand cannot make the vector large.

**Allocations,** counted as in #1, one evaluation of `object constructor`
(`items.{'code': code, 'kind': 'item', 'total': unitPrice * quantity}`) with the result converted
to JSON, then without it:

| Case | #4 | #5 | of the evaluation, #4 | #5 |
|---|---:|---:|---:|---:|
| `object constructor` | 2,607 | 1,814 | 1,206 | 413 |
| `transform` | 4,635 | 4,239 | 2,422 | 2,026 |

Each object of the constructor takes 2 allocations instead of 6: its `Rc<Object>` and its
vector of fields. `transform` saves the key `'total'` of each update and the array `['tax']`'s
string. The other cases of #1 allocate as many as before. In the same program, three runs each
alternated on new copies, `object constructor` went from 275 to 222 µs on the system heap
(-19%) and from 127 to 124 µs with `mimalloc` (-2%), where an allocation costs little.

**`cargo xtask ab`,** 10 rounds, the median:

| Case | `evaluate`, against #4 | `document` | `source` | `evaluate`, against 0.30.0 | `document` |
|---|---:|---:|---:|---:|---:|
| `object constructor` | -16.5% | -13.5% | -16.8% | -20.9% | -21.7% |
| `transform` | -2.6% | +0.1% | -3.9% | -2.1% | -7.7% |

Against #4 every other case is within 2.7%. Against 0.30.0 the cases that #3 and #4 moved keep
their gains; one `evaluate` run read every case of its B program 20 to 35% slow, the machine
busy, and was run again.

## The release measured (0.31 #7)

Measured on 2026-11-07 and 08 at cc5981a, against v0.30.0 (192726c), on the Ryzen 7 3700X that
every table up to 0.30.0 came from (Windows 11, and Ubuntu 26.04 under WSL 2), and on a MacBook Air
M1 (macOS 27), a new platform for the tables. Nothing needed bringing up to date first: `cargo
update` found nothing in `Cargo.lock` (nor past the MSRV), npm and Maven had no newer version of
what the benchmarks run, binaryen 132 is the latest on npm (133 is on GitHub only), and no other
engine had a new version, so their columns on the Ryzen keep the numbers of 0.27.0.

### Natively

`cargo xtask ab v0.30.0 HEAD`, 10 rounds, two comparisons of plain builds and, on the Ryzen, a
third built aligned (in parentheses); B against A, the median, in percent:

| Case | Windows `evaluate` | `document` | `source` | Linux `document` | M1 `evaluate` | M1 `document` | M1 `source` |
|---|---:|---:|---:|---:|---:|---:|---:|
| path | -8.5, -10.4 (-6.8) | -4.5, -2.5 (-3.0) | -6.1, -7.1 (-6.1) | -15.1, -12.9 (-10.3) | -11.8, -13.6 | -13.6, -12.6 | -6.6, -10.1 |
| names over items | -2.7, -3.1 (+0.3) | -18.1, -18.1 (-18.5) | -1.4, -1.7 (-0.5) | -17.5, -17.8 (-18.1) | -1.8, -1.5 | -20.9, -20.1 | -1.7, -1.9 |
| filter + aggregate | -0.3, -3.1 (+0.7) | -4.1, -5.7 (-2.9) | +0.4, -0.2 (-0.5) | -3.3, -3.2 (-7.6) | +0.4, +0.2 | -13.7, -13.1 | +0.2, +0.7 |
| group-by | -3.5, -4.1 (-0.3) | -7.0, -7.1 (-4.5) | -1.1, -1.3 (-1.3) | -1.7, -1.2 (-5.0) | +0.3, +0.9 | -11.1, -10.7 | +0.8, +0.8 |
| sort + map | -1.9, -3.8 (-1.3) | -2.0, -4.9 (-3.4) | +0.5, -0.6 (-1.6) | -5.9, -6.1 (-5.3) | -0.8, -1.1 | -6.3, -5.8 | -1.7, -1.1 |
| strings | -0.4, -0.6 (+0.1) | -1.5, -5.5 (-2.7) | +0.4, -0.2 (-1.0) | -1.6, +0.5 (-5.4) | -0.9, -0.8 | -11.5, -10.9 | -0.4, -0.1 |
| regex | -0.2, +0.0 (+1.1) | -1.7, -1.1 (-1.3) | -19.0, +0.3 (+2.0) | -2.1, -3.1 (-2.6) | -1.9, -2.4 | -3.9, -4.0 | -0.6, +0.6 |
| number formatting | -0.9, -3.0 (-0.5) | -0.2, -3.1 (-2.2) | -1.4, -1.0 (-0.8) | -4.6, -3.3 (-5.1) | -2.9, -2.2 | -3.1, -2.8 | -1.6, -1.3 |
| object constructor | -25.5, -24.4 (-23.2) | -22.9, -22.2 (-20.7) | -22.0, -20.7 (-21.9) | -17.0, -17.6 (-19.1) | -8.8, -9.0 | -14.8, -14.7 | -6.7, -6.6 |
| transform | -5.9, -6.0 (-8.8) | -10.3, -10.0 (-10.8) | -7.3, -7.4 (-6.8) | -15.0, -13.9 (-14.4) | -9.0, -9.0 | -9.0, -8.9 | -10.1, -10.6 |
| recursion | -0.5, -0.0 (+0.5) | -0.7, -0.7 (+1.8) | +6.0, -0.5 (+0.9) | +9.1, +9.7 (-0.5) | +0.6, +0.5 | +0.4, +0.4 | +0.5, +0.5 |
| shipping rule | -0.2, -0.5 (+3.2) | -8.7, -8.0 (-8.4) | +0.3, +0.9 (-0.3) | -6.0, -6.8 (-10.3) | +1.1, +1.3 | -17.6, -17.3 | +1.2, +1.1 |
| index into 10,000 items | -5.8, -6.4 (-6.0) | -2.3, -3.0 (+0.9) | -6.1, -6.9 (-2.9) | -9.2, -8.8 (-9.0) | -3.2, -3.8 | +0.6, +2.7 | -4.7, -2.9 |
| index in a variable | -4.8, -3.8 (-3.6) | -1.0, -1.6 (+1.8) | -4.3, -3.4 (-2.1) | -1.9, -1.2 (-0.4) | -4.9, -5.0 | -1.0, -0.8 | -5.0, -3.1 |
| regex over a long text | +5.4, +4.9 (-3.2) | +5.1, +5.7 (-3.0) | +5.4, +5.0 (-4.1) | -8.1, -8.4 (+5.2) | -0.1, +0.0 | +0.2, -0.1 | +0.0, +0.2 |
| distinct items | -19.9, -20.9 (-19.5) | -8.3, -8.6 (-7.9) | -16.9, -16.5 (-18.5) | -8.2, -7.4 (-8.5) | -13.2, -13.1 | -4.9, -4.8 | -12.1, -11.7 |
| tuples | -10.9, -10.8 (-11.2) | -13.5, -13.3 (-14.1) | -10.6, -11.0 (-10.8) | -11.0, -10.6 (-12.8) | -7.2, -7.2 | -11.4, -11.3 | -6.8, -6.5 |

- `object constructor` (#3, #5), `distinct items` (#3, #4), `tuples` (#3), `transform` and the
  cases against a `Document` that read objects of six fields (`names over items`, the shipping
  rule, `group-by`, `strings`) are faster in every comparison on every machine.
- Layout, settled by the aligned run: `regex over a long text` +4.9 to +5.7% on Windows in every
  mode, −3.0 to −4.1% aligned; `recursion` +9.1 and +9.7% under Linux, −0.5% aligned. Under Linux
  `regex over a long text` turned the other way, −8% plain and +5.2% aligned: the bimodal case
  (149 to 165 µs) of 0.30.
- Draws: Windows `source` read `regex` −19.0% and `recursion` +6.0% in one comparison, with the
  fastest rounds equal (+0.2%, −0.5%); the other comparison and the aligned one put both within 2%.
  Windows `evaluate` read the shipping rule +3.2% aligned only.
- The M1's two comparisons agree within 1.3 points on every case. Nothing there is slower than
  +2.7% (`index into 10,000 items` against a `Document`, +0.6 in the other comparison); `recursion`
  and the shipping rule read +0.4 to +1.3% in every comparison.

### Python

`python.py ab`, two comparisons of five rounds on each machine (Linux: Python 3.14.8; the M1:
Python 3.14.10), and a third on the M1 with both wheels built aligned.

- Under Linux, nothing slower than +1.6% (the shipping rule with Python values, in one comparison;
  −1.5% in the other). Marked in both: `object constructor` −9.5 to −16.6%, `transform`
  −8.4 to −11.5%, `tuples` −8.0 to −11.5%, `names over items` against a `Document` −15%,
  `distinct items` −4.7 to −5.7%, `path` with Python values −4.2 to −4.5%.
- On the M1, the same gains, and against a `Document` larger ones (`shipping rule` −17 to −19%,
  `group-by` −12 to −13%, `names over items` −13 to −16%). One case is slower: `filter + aggregate`
  with Python values, +4.0% (marked), +3.6% and, aligned, +3.0% (unmarked, A 42.8 against B 44.0
  µs). It is not layout, which the aligned build would have moved, and not the Python package's
  own code, which 0.31 did not change; but the M1's native `source` mode, which reads values one
  level at a time as the package does, reads it +0.2 and +0.7%, and Linux −0.3 and −2.2%. Not
  found; left for a later plan, as the case reads 367 objects of six fields from a Python dict
  each, the shape #3 made cheaper elsewhere.
- The M1's second comparison ran across 4 minutes of sleep (the lid was closed): three cells
  spread 37 to 67%, and its medians agree with the first comparison's.

### Java

`cargo xtask java-compare ab`, every part, five rounds: two comparisons under Linux, one on the M1
(55 minutes each). Under Linux, 49 and 28 differences marked, all faster: `$sum(items.price)` over
100,000 items in a `Document` −35%, `object constructor` −14 to −19%, `distinct items` from JSON
text −20%, `transform` −12 to −15%; from JSON text on 1 to 16 threads and on 1,000 virtual
threads, 5.5 to 9.9% more evaluations a second. Unmarked and past +2.5%: `names over items` over
Java objects +3.7% (spreads 5 and 6%), and `items[0].name` over Java objects from JSON text +3.1%
(spreads 9 and 13%), each in one comparison only. On the M1, 42 marked, all faster (the threads
+5.9 to +7.3%), and four cells between +3.0 and +3.5%, unmarked, three of them with spreads of 9
to 14%.

### WebAssembly

The release's table, the median of three runs, against 0.30.0's. Faster: `object constructor`
−16% with `evaluate` (270 → 226 µs), −13% from JSON, −24% against a `Document` (174 → 133 µs);
`tuples` −14, −13 and −18%; `transform` −15, −12 and −16%; `distinct items` −14, −16 and −15%;
`names over items` against a `Document` −12%; `evaluateJson` on the 10,000 items −16% (3.93 → 3.29
ms), as `Document.fromJson` builds each item's object in one allocation fewer. Slower by at most
2.2%: `regex over a long text` with `evaluate` (272 → 278 µs), `recursion` (+1.5 to +2.1%).
jsonata-js, the control, read from −3.2% (`names over items`) to +2.3% (`recursion`).

### The tables

- Natively, on Windows: `cargo xtask bench-table`, three copies, which agreed within 3% but on `distinct items` with `evaluate` (6%)
  and the indexes into 10,000 items (5 to 8%, two of them marked); run again with `--runs 5`,
  within 2% of the first run but on `index into 10,000 items` against a `Document` (733 → 689
  ns; 0.68 µs in 0.30.0's table), within 5.5%. The page takes the five copies for the indexes.
  Against 0.30.0's table, every case more than 2% away is one that `ab` puts faster, and so are
  `Document::parse` (170 → 141 µs) and `Document::new` (112 → 88 µs), which make each object
  with one allocation fewer. On the M1, three copies, within 2.6%.
- WebAssembly, on Windows: three runs of `compare.mjs --engines jsonata-js`, within 5% but
  `distinct items` with `evaluateJson` (8%).
- Python, under Linux: three runs of jlens alone, within 3% but the index in a variable (7%), and
  one with jsonata-python, the control, within 2.8% of its 0.30.0 numbers (`path` 1.49 → 1.53 ms,
  the index into 10,000 items 28.7 → 29.5 ms).
- Java, under Linux: `java-compare` with jlens and jsonata-java, then with jlens and
  jlens-objects; jlens's two JVMs within 3% on every case, and the page takes the faster.
  jsonata-java, the control, within 4% of its 0.30.0 numbers (`recursion` 1.48 → 1.53 ms, `sort +
  map` 747 → 770 µs).

## On an Apple M1

The M1 is measured for the first time for 0.31.0, to decide whether all measuring moves there. It
was reached over SSH and ran the same harnesses: `cargo xtask ab` and `bench-table`, `compare.mjs`
with both other engines, `python.py speed` with every package (`jsonatapy` built from its source
distribution, as on Linux; every package has what arm64 needs), and `java-compare` with every
engine in one JVM. The whole run took about 2 h 40 min, and 25 min more for the aligned Python control, alongside
the Ryzen's 3 h 15 min.

**How it behaved.**

- **Noise.** Two `ab` comparisons agree within 1.3 points on every case, and a revision against
  itself within 0.8% (measured before the plan). Under Windows the same comparisons differ by up
  to 3 points, and by draws of the heap of up to 20.
- **Heat.** No throttling: the performance cores held 3,204 MHz in all 22 samples where one was
  busy past 80% (`macmon pipe`, every 30 s); the CPU stayed at 36 to 53 °C and reached 76 °C only
  in the Java threads part, all eight cores busy at 14.7 W. The thermal pressure stayed nominal.
- **What disturbed it.** `airportd` took 50 to 67% of a core, AWDL in conflict with Symless
  Synergy; turned off (`ifconfig awdl0 down`) and the Mac moved to a cable, the Wi-Fi off.
  Spotlight indexed every new clone and `target` directory (`mds` 94% of a core) until
  `~/workspaces` went into its privacy list. The lid, closed, put the Mac to sleep despite
  `caffeinate -i`, during a Python comparison. The native `ab` runs came before these fixes, and
  agree within 1.3 points anyway.
- **One failure.** The first `bench-table` failed in a second: two rustup proxies installed the
  `rust-src` component at once, and the second's rename failed. It ran again after the other runs.
- **A harness bug.** On 8 hardware threads, `java-compare`'s `threads` part ran 4 threads twice
  (1, 2, 4, half of 8, 8); it now runs each count once.

**Its tables,** with every engine (native in the page). "jlens 0.30.0" in the harnesses' headers
is cc5981a, which still carried 0.30.0's version.

WebAssembly, the median of three runs, within 6% but jsonata2js's `index in a variable` (24%) and
`tuples` (27%):

| Case | jsonata-js | jsonata2js | jlens-wasm `evaluate` | jlens-wasm `evaluateJson` | jlens-wasm `evaluateDocument` |
|---|---:|---:|---:|---:|---:|
| path | 1.30 µs | 200 ns | 2.40 µs | 72.7 µs | 700 ns |
| names over items | 41.5 µs | 3.20 µs | 30.7 µs | 102 µs | 5.30 µs |
| filter + aggregate | 402 µs | 8.80 µs | 91 µs | 149 µs | 28.9 µs |
| group-by | 667 µs | 13.8 µs | 130 µs | 180 µs | 60.1 µs |
| sort + map | 3.09 ms | 60.6 µs | 296 µs | 356 µs | 240 µs |
| strings | 367 µs | 11.1 µs | 129 µs | 186 µs | 53.6 µs |
| regex | 385 µs | 10.1 µs | 134 µs | 201 µs | 95.6 µs |
| number formatting | 340 µs | 75.4 µs | 50.2 µs | 132 µs | 24.8 µs |
| object constructor | 745 µs | 51.5 µs | 156 µs | 287 µs | 89.7 µs |
| transform | 798 µs | 204 µs | 364 µs | 502 µs | 198 µs |
| recursion | 5.61 ms | 42.1 µs | 453 µs | 520 µs | 448 µs |
| shipping rule | 1.11 ms | 26.7 µs | 198 µs | 209 µs | 69.5 µs |
| index into 10,000 items | 1.30 µs | 145 µs | 3.40 µs | 2.28 ms | 900 ns |
| index in a variable | 1.99 ms | 197 µs | 3.70 µs | 2.33 ms | 1.10 µs |
| regex over a long text | 319 µs | 135 µs | 176 µs | 179 µs | 153 µs |
| distinct items | 6.5 ms | 55.5 µs | 180 µs | 165 µs | 17.3 µs |
| tuples | 664 µs | 27.6 µs | 170 µs | 227 µs | 109 µs |

Python, the median of three runs (jsonata-python one), within 3% but jlens's `path` against a
`Document` (0.3 or 0.4 µs), `regex` and the shipping rule with Python values (8%), and the index in a
variable and number formatting (11%):

| Case | jsonatapy | jsonatapy converted once | jsonata2py | jsonata-python | jlens | jlens converted once |
|---|---:|---:|---:|---:|---:|---:|
| path | 500 ns | 300 ns | 500 ns | 1.06 ms | 500 ns | 400 ns |
| names over items | 15.2 µs | 3.70 µs | 13.2 µs | 1.4 ms | 15.2 µs | 3.00 µs |
| filter + aggregate | 58.8 µs | 35.3 µs | 61.7 µs | 3.12 ms | 41.8 µs | 20.8 µs |
| group-by | 87.5 µs | 60.8 µs | 151 µs | 4.27 ms | 73.7 µs | 43.1 µs |
| sort + map | 310 µs | 229 µs | 266 µs | 15.9 ms | 185 µs | 142 µs |
| strings | 81.6 µs | 53.9 µs | 64.1 µs | 3.07 ms | 52.5 µs | 30.7 µs |
| regex | 90.9 µs | 74.1 µs | 158 µs | 3.2 ms | 73.5 µs | 52.7 µs |
| number formatting | 72.8 µs | 68 µs | 90.2 µs | 2.77 ms | 17.7 µs | 15.6 µs |
| object constructor | 153 µs | 124 µs | 182 µs | 3.98 ms | 101 µs | 71.2 µs |
| transform | 734 µs | 664 µs | 347 µs | 3.76 ms | 211 µs | 143 µs |
| recursion | 581 µs | 578 µs | 1.39 ms | 28.2 ms | 339 µs | 345 µs |
| shipping rule | 184 µs | 101 µs | 221 µs | 6.88 ms | 122 µs | 52.6 µs |
| index into 10,000 items | 178 µs | 400 ns | 700 ns | 20.6 ms | 800 ns | 500 ns |
| index in a variable | 624 µs | 437 µs | 1.29 ms | 36 ms | 1.00 µs | 600 ns |
| regex over a long text | 97.4 µs | 96.7 µs | differs: 4150 | differs: 4150 | 87.7 µs | 88.6 µs |
| distinct items | 496 µs | 402 µs | 307 µs | 1.48 ms | 78 µs | 10.6 µs |
| tuples | 266 µs | 240 µs | 277 µs | 4.25 ms | 94 µs | 74.6 µs |

Java, microseconds per evaluation, the best of five runs, every engine in one JVM (OpenJDK
25.0.5, 8 hardware threads):

| Case | jlens 0.30.0, converted | jlens 0.30.0, JSON text | jlens 0.30.0 (objects), converted | jlens 0.30.0 (objects), JSON text | jsonata-java 0.9.10, converted | jsonata-java 0.9.10, JSON text | jsonata-jvm-compiler 1.0.7, converted | jsonata-jvm-compiler 1.0.7, JSON text |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| path | 0.21 | 32.93 | 0.89 | 76.14 | 8.96 | 99.64 | 0.03 | 71.66 |
| names over items | 2.88 | 44.45 | 18.12 | 92.17 | 17.97 | 103 | 1.32 | 73.62 |
| filter + aggregate | 20.96 | 69.24 | 56.31 | 131 | 58.73 | 145 | 4.11 | 76.72 |
| group-by | 44.01 | 91.78 | 84.66 | 161 | 129 | 216 | 13.33 | 89.75 |
| sort + map | 141 | 185 | 185 | 257 | 426 | 511 | 39.33 | 115 |
| strings | 31.58 | 84.02 | 73.25 | 150 | 68.64 | 152 | 7.34 | 81.62 |
| regex | 51.77 | 93.64 | 72.99 | 154 | 111 | 194 | 11.95 | 86.71 |
| number formatting | 14.21 | 60.22 | 20.61 | 97.77 | 92.99 | 174 | 8.64 | 84.13 |
| object constructor | 70.48 | 120 | 109 | 184 | 99.88 | 184 | 6.88 | 80.94 |
| transform | 134 | 194 | 219 | 294 | 237 | 319 | 26.74 | 101 |
| recursion | 343 | 380 | 339 | 412 | 775 | 1,023 | 44.72 | 118 |
| shipping rule | 52.53 | 112 | 143 | 219 | 161 | 247 | 15.07 | 90.46 |
| index into 10,000 items | 0.40 | 895 | 1.23 | 1,416 | 149 | 1,445 | 0.04 | 1,396 |
| index in a variable | 0.55 | 895 | 1.37 | 1,422 | 646 | 1,951 | 35.12 | 1,377 |
| regex over a long text | 87.97 | 117 | 115 | 124 | 116 | 134 | differs | differs |
| distinct items | 10.78 | 69.26 | 94.80 | 171 | 25.90 | 109 | 20.16 | 97.65 |
| tuples | 75.01 | 123 | 107 | 183 | 153 | 238 | differs | differs |

Evaluations per second of `$sum(items.price)` on the JSON text of 100 items (the harness ran 4
threads twice, as above; the second run, within 0.7% of the first, is left out):

| Engine | 1 thread | 2 threads | 4 threads | 8 threads | 1,000 virtual threads |
|---|---:|---:|---:|---:|---:|
| jlens 0.30.0 | 41,893 (1.0×) | 81,837 (2.0×) | 154,812 (3.7×) | 200,930 (4.8×) | 200,923 (4.8×) |
| jlens 0.30.0 (objects) | 15,900 (1.0×) | 31,705 (2.0×) | 59,778 (3.8×) | 79,603 (5.0×) | 79,119 (5.0×) |
| jsonata-java 0.9.10 | 18,705 (1.0×) | 36,474 (1.9×) | 69,500 (3.7×) | 91,050 (4.9×) | 90,757 (4.9×) |
| jsonata-jvm-compiler 1.0.7 | 18,225 (1.0×) | 36,620 (2.0×) | 68,263 (3.7×) | 91,114 (5.0×) | 91,178 (5.0×) |
