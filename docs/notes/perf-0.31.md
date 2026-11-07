# Performance notes: 0.31

How the measurements of roadmap 0.31 were made: methods, traps and intermediate numbers that the
[performance page](../src/architecture/performance.md) leaves out. The item numbers are those of
[the roadmap](../src/architecture/roadmap.md#031-a-small-map-for-objects).

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
