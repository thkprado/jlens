# Performance notes: 0.35

How the measurements of roadmap 0.35 were made: methods, traps and intermediate numbers that the
[performance page](../src/architecture/performance.md) leaves out. The item numbers are those of
the plan, whose table the tag keeps in
[the roadmap](https://github.com/thkprado/jlens/blob/v0.35.0/docs/src/architecture/roadmap.md#035-the-choices-of-034-reviewed);
[the roadmap's history](../src/architecture/roadmap-history.md#035) sums it up.

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

## #9: the variables of the root frame

`bind` put a frame on the evaluator's list of frames that may be part of a cycle for any array or
object, and the variables of an evaluation went through it, though a value from JSON or a
document holds no frame. They are now set before the root frame is made (`11118fb`); a first
form, `cbc9155`, set them through a borrow of the frame's variables. On the M1, a scratch
program over `jlens-core` (the release profile with `lto` and one codegen unit; the median of 25
batches of about 5 ms, five processes of each build alternated, each on a new copy), against
`c5c40b1`, the commit before the item, in ns:

| Call | `c5c40b1` | `11118fb` |
|---|---:|---:|
| `1` against a `Document`, no variable | 63.7 | 62.9 |
| `1`, a number bound to a document | 75.6 | 75.7 |
| `1`, an object bound to a document | 91.0 | 77.0 |
| `1`, two objects bound to documents | 121.6 | 105.8 |
| `$table.items[0].name`, an object bound to a document | 380.0 | 364.3 |
| `$table.items[0].name & $other.id`, two documents | 604.2 | 581.8 |
| `1` from JSON values, an object in `Bindings` | 141.6 | 128.6 |
| `$table.items[0].name`, the same | 666.3 | 658.0 |
| `items[0].name` against a `Document`, no variable | 293.6 | 298.6 |

- 13 to 16 ns less for each evaluation with an array or object as a variable, as #7 found from
  the C API.
- Without a variable, `cargo xtask ab c5c40b1 11118fb` built aligned (the flags of #8), twice in
  each mode, on three short cases: `path` against a `Document` +1.9% and +2.9% (103 → 105 ns,
  its fastest rounds +3.0%), the index into 10,000 items +0.7% and +2.0%, the index in a
  variable within 1.4%; from JSON values all three within ±1.3%. `cbc9155`, with the borrow,
  gave the same: `path` +1.0%, the index into 10,000 items +2.4% (296 → 303 ns). So 2 to 3 ns
  of the smallest evaluation against a `Document` moved with the change, against 15 ns gained
  with a variable. Plain, `ab cbc9155 11118fb` has every case within ±1.2% but `names over
  items` against a `Document`, +2.6%.

## #9: the diagnostics against 0.34.0

The program of 0.34 #14 over `LanguageService::diagnostics` (`with_context` renamed
`with_context_path` for 0.35), the same schema and rules, five processes of each build
alternated, on the M1. In µs, the median of the five:

| Case | 0.34.0 | `cbc9155` |
|---|---:|---:|
| `1`, no schema | 0.23 | 0.23 |
| `1` | 0.27 | 0.26 |
| `customer.login` | 1.06 | 1.05 |
| `$count(items) > 2` | 2.17 | 2.14 |
| `$contains(customer.name, "VIP")` | 2.68 | 2.70 |
| `$sum(items[unitPrice > $minimum].($rate(type) * unitPrice * quantity))` | 6.22 | 6.23 |
| the same with three misspelt names | 17.34 | 17.27 |
| the shipping rule | 35.69 | 35.46 |
| a field of an object of 200 fields | 1.07 | 1.06 |
| a field it does not have, suggested from the 200 | 29.06 | 28.52 |
| a field of an object of 2,000 fields | 1.07 | 1.06 |
| a field it does not have, suggested from the 2,000 | 153.8 | 147.9 |
| two fields of an object of 10 `patternProperties` | 5.16 | 5.21 |
| `$count(items) > 2`, a boolean expected | 2.18 | 2.14 |
| `unitPrice * quantity`, as the body of `items[]` | 1.53 | 1.51 |

Within 2% but the suggestion from 2,000 fields, 3.8% faster; the processes of a build agree
within 3.8%, but `1` without a schema, which takes 0.23 µs.

## #9: the plan's code checked whole

- **Properties.** With `PROPTEST_CASES=100000`, in release mode on the Ryzen: `jlens-schema`'s
  robustness (165 s), keywords (9 s) and references (28 s), `jlens-core`'s 47 robustness
  properties (122 s), `jlens-lsp`'s (42 s) and `jlens-capi`'s (24 s). All pass, among them
  `branches_type_as_their_documents`, which found `1[][0]` past about 2,000 cases in #5.
- **Fuzzing.** `language_service` gets an array of tuples and an `anyOf` of tuples in its typed
  schema, `evaluate` binds its document as `$doc` too, and `fuzz-seed` adds expressions over
  those names, which the suite does not name. The six targets of the workflow, 10 minutes each
  under WSL, three at a time, from the seeds: nothing found, from 0.51 million inputs
  (`language_service`) to 2.5 million (`parse`).

## #9: the release measured

Measured on 2026-11-15 on the M1 at `cbc9155` against v0.34.0 (`11118fb` differs only in how
the root frame's variables are set, above). No other engine has a new version: jsonata-js 2.2.2,
jsonata-python 0.7.2, jsonatapy 2.2.14, jsonata-rs 0.1.4. Thermal pressure nominal at each
step; Rust 1.100.0, Node.js 24.22.3, OpenJDK 25.0.5, Python 3.14.10.

- **Natively**, `cargo xtask ab v0.34.0 HEAD`, 10 rounds, twice in each mode: every case within
  2.1% of 0.34.0 or faster. `recursion` takes 6 to 8% less in every mode, `filter + aggregate`,
  `group-by` and the shipping rule 1 to 4% less.
- **Python**, `python.py ab`, two comparisons of five rounds: nothing marked, all within ±3.6%.
- **WebAssembly**, `compare.mjs --pkg --engines jsonata-js`, four processes of each build
  alternated: within 2.6% but `path`, printed to a tenth (2.3 or 2.4 µs, and 0.6 or 0.7 µs
  against a `Document`); `recursion` 2.4% faster. jsonata-js, the control, within 1.4%.
- **Java**, `cargo xtask java-compare ab`, every part, five rounds. The cases, run twice: one
  difference marked, faster, `regex` against a `Document` (-5.5%, and -4.1% unmarked in the
  first run), as #7 found. The first run had `object constructor` 3.8 to 9.2% slower in its four
  columns, unmarked, with the runs of 0.34.0 spreading 6 to 12%; the second has it within ±1.1%.
  The documents: nothing marked, -2.6 to +7.7% (`$` converted, whose runs spread 23%). The
  threads: within 1%.
- **The tables.** Natively, three copies of `bench-table`, within 1.5% but one copy 3% slower
  on `recursion` against a `Document` and on compiling `customer.name`; against the table of
  0.34, within 2.5%, each difference in the direction `ab` gives. WebAssembly, the four
  processes above. Python: three runs of jlens alone, then three more after the Java runs,
  which agree with the first within 1%; both are 2 to 8% above the table of 0.34.0 (`sort +
  map` 169 → 183 µs), and `python.py speed` of the wheel of 0.34.0, alternated with that of
  0.35, gives 179 to 180 µs for both: the machine on the day, not the code. jsonata-python, the
  control, within 2% of its numbers for 0.34. Java: two JVMs, the faster of each time, as for
  0.34; `recursion` in jsonata-java is 19 to 23% slower in one JVM or the other.

### On the Ryzen

The plan changed the evaluation of paths (#8) and measured it only on the M1, so the Ryzen 7
3700X compared the release with 0.34.0, natively under Windows (`cargo xtask ab v0.34.0 HEAD`,
twice in each mode) and in Python under Linux in WSL (Python 3.14.8, `python.py ab`, twice):

- Natively, every case within ±2.6% but three. `filter + aggregate` against a `Document`, +6.4%
  and +7.0% (30.0 → 31.9 µs, the fastest rounds as much), where the M1 has it 3 to 4% faster;
  `distinct items` from a `Source`, +3.6% in both; `regex over a long text` +1.4 to +3.4% in
  every mode, the case 0.32 found plain builds to move. Built aligned, twice: `filter +
  aggregate` -0.3% and +0.5% against a `Document`; `distinct items` from a `Source` -2.4% and
  +8.2%, its fastest rounds -5.5% and -3.6%: the two speeds of the Windows heap, as in 0.32.
- In Python, marked in both comparisons: `distinct items` against a `Document` +13.3% and
  +13.5% (13.5 → 15.3 µs), `recursion` +4.5 to +5.1% in both modes; in one, `regex over a long
  text`, `filter + aggregate`, number formatting and the index in a variable against a
  `Document`, +3.6 to +5.7%. `1e1709e` (#8) against 0.34.0 has `distinct items` at +0.5%, and
  `cbc9155` against `1e1709e` at +12.2%. The same two built aligned (the flags above, through
  `RUSTFLAGS` to maturin): `distinct items` -0.4%, `recursion` -0.1%, every case within ±1.5%
  but `regex over a long text`, +3.7% and +4.2%, unmarked. So the item moved where `$distinct`
  lands in the library, not what it does.
