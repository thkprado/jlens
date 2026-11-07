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
