# Performance notes: 0.35

How the measurements of roadmap 0.35 were made: methods, traps and intermediate numbers that the
[performance page](../src/architecture/performance.md) leaves out. The item numbers are those of
the plan, in [the roadmap](../src/architecture/roadmap.md#035-the-choices-of-034-reviewed).

## #7: documents as variables in the Java package

On the M1 (macOS 27, OpenJDK 25.0.5), idle, thermal pressure nominal at the end. Two JARs, each
built by `cargo xtask java` in a clone of its own: 0.34.0 (`v0.34.0`) and `6d515da` with the
item's changes.

### A variable in `Bindings` against a `Document`

`cargo xtask java-compare variables`, a part added by the item and run only when named: jlens
alone, on expressions that read a variable `$table` holding the spike's documents, with
`{"id": n / 2}` as the input, a `Document` in both columns. In the first the variable is in
`Bindings`, built once from the Java objects Jackson parses
(`expression.evaluate(input, bindings)`); in the second it is a `Document`
(`expression.evaluate(input, Map.of("table", table))`). Microseconds per evaluation, the best of
five runs of 0.3 s after a second of warm-up, two JVMs:

| Expression | In `Bindings` | A `Document` |
|---|---:|---:|
| 1,000 items (97 KB of JSON) | | |
| `$table.items[0].name` | 68.03, 67.03 | 0.66, 0.60 |
| `$table.items[id = $$.id].name` | 163, 163 | 62.75, 62.90 |
| `$count($table.items)` | 15.78, 15.74 | 0.60, 0.60 |
| `$sum($table.items.price)` | 68.73, 68.75 | 8.34, 8.29 |
| 100,000 items (10,099 KB of JSON) | | |
| `$table.items[0].name` | 8,333, 8,345 | 0.74, 0.86 |
| `$table.items[id = $$.id].name` | 17,391, 17,733 | 9,728, 9,722 |
| `$count($table.items)` | 1,603, 1,635 | 0.61, 0.75 |
| `$sum($table.items.price)` | 9,194, 9,053 | 3,386, 3,363 |

- A variable of `Bindings` is converted at each evaluation as far as the expression reads it
  ([ADR 0019](../src/adr/0019-documents-from-rust-values.md#variables-of-evaluate_document)),
  and what an expression reads is known by field, not by item: `$table.items[0].name` converts
  `name` of every item, and `$count($table.items)` every item with none of its fields. A
  `Document` is bound as it is.
- Where the expression walks every item, the conversion is what the two columns differ by:
  about 60 µs of 1,000 items and 5.7 ms of 100,000 for one field of each (`$sum`), about 100 µs
  and 7.7 to 8.0 ms for the two fields the filter reads.
- Against the same path on the input (`items[0].name`, 0.36 and 0.50 µs in the comparison
  below), the variable costs 0.2 to 0.4 µs more. Timed from Rust, on the C API alone (a test
  that calls the exports 200,000 times, the fastest of 25 batches, in ns):

  | Call | ns |
  |---|---:|
  | `1`, `jlens_evaluate_document` | 129 |
  | `1`, `jlens_evaluate_document_with`, no list | 133 |
  | `1`, with an empty list | 144 |
  | `1`, with one document, not read | 219 |
  | `1`, with two documents, not read | 269 |
  | `items[0].name`, `jlens_evaluate_document` | 309 |
  | `$table.items[0].name`, with one document | 450 |
  | `$table.items[0].name & $other.id`, with two documents | 671 |

  A bound document costs 50 to 75 ns whether it is read or not. Of them, about 17 ns are the
  root frame put on the evaluator's list of frames that may be part of a cycle, which `bind`
  does for any array or object, though a value of the input can hold no frame: binding the
  root's variables without it measures 202, 258, 432 and 654 ns on the four rows with
  documents. That is a change to the core, for every package, left to #9. The rest is spread
  over the name, the frame's variables, finding the document and its conversion, and the drop.
- The list of documents was first parsed into a `Vec` of boxed names and handles, then resolved
  into two more: three allocations and one for each document. Copying the names into one text
  while resolving the documents, two allocations however many documents, measured 7 to 14 ns
  less on the rows with documents (226 → 219, 281 → 269, 460 → 450, 685 → 671 ns), and was
  kept. Both tables are of the final code; from Java, two JVMs of the first form gave the same
  numbers, within 0.1 µs on the rows under a microsecond. The comparison below ran with the
  first form, which differs only in the new export.

### `evaluate(Document)` against 0.34.0

`cargo xtask java-compare ab <0.34.0> <0.35> documents --engines jlens`, then `cases`, five
rounds each (8 and 14 minutes). `jlens_evaluate_document` is not changed by the item; its
evaluation goes through `evaluate_document_with` with no variables, as `evaluate_document` does
in the core.

- The documents: none of the 20 differences is marked, from −3.6% (`$count(items)` from JSON
  text) to +4.3% (`items[0].name` on 100,000 items converted, 0.50 and 0.52 µs, whose runs
  spread by 31 and 40%: each JVM converts the document anew, and the path's time follows where
  it lands). On 1,000 items converted, `items[0].name` 0.364 → 0.356 µs, the index in a variable
  0.484 → 0.470, `$count(items)` 0.314 → 0.308, `$sum(items.price)` 7.80 → 7.80.
- The cases: of 34 differences, from −7.9% (`path` from JSON text, 33.1 → 30.4 µs, where
  0.34.0's runs spread by 9.8%) to +1.4%, one is marked: `regex` on a `Document`, 51.4 → 48.6 µs
  (−5.4%), faster, in code that no item of 0.35 has touched so far; the same case from JSON
  text moves −1.8%. `path` on a `Document` 0.201 → 0.200 µs.

## #8: three fixes of jsonata-js after 2.2.2

The fixes touch the evaluation of paths: each filter is told whether another follows it, the
filter of a constant or a computed index answers whether it selected one position, a tuple step
looks at whether an array was constructed, and the first step of a path whether it is a path
with a group-by. On the M1, idle, thermal pressure nominal, `cargo xtask ab f51d97c .` (#7's
commit against it with the item's changes), 10 rounds, twice in each mode:

- `evaluate`: every case within ±1.7% of its median in both runs, but two. `strings` is 4.5%
  and 4.7% faster in both, in code the item does not touch: layout. `index in a variable`
  (`($i := 0; items[$i].code)`, a constant filter, the one case on the code that changed) is
  +3.2% and +0.0%, its fastest round +0.2% and −0.6%.
- `document`: every case within ±1.3%, but `index in a variable`, +2.0% and +1.5% (fastest
  +1.5% and +1.7%), 410 → 418 ns.

Again with functions and blocks aligned (`RUSTFLAGS="-C llvm-args=-align-all-functions=6 -C
llvm-args=-align-all-nofallthru-blocks=5"`, the cached programs moved aside), on that case and
two controls, twice in each mode: `index in a variable` is +1.0% and +1.2% (418 → 423 ns)
against a document, +0.3% and −1.3% from JSON values; `path` and `index into 10,000 items` are
within ±1% but one run at −2.0%. So about 4 ns of a constant filter on a document is the
work of the item, the boolean of a selected position; the rest moved with the layout.
