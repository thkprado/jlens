# Performance notes: 0.33

How the measurements of roadmap 0.33 were made: methods, traps and intermediate numbers that the
[performance page](../src/architecture/performance.md) leaves out. The item numbers are those of
the plan, whose table the tag keeps in
[the roadmap](https://github.com/thkprado/jlens/blob/v0.33.0/docs/src/architecture/roadmap.md#033-expressions-checked-again-when-their-schema-changes);
[the roadmap's history](../src/architecture/roadmap-history.md) sums it up. The plan changed
no line of `jlens-core`: its measurements are of the schema's load and of the C API's library,
which the ADRs [0023](../src/adr/0023-schema-through-the-c-api.md),
[0024](../src/adr/0024-recursive-schemas.md) and
[0025](../src/adr/0025-references-to-other-documents.md) hold, and those of #12, below.

## #12: JSON text parsed for what the host functions read

An evaluation of JSON text through the C API (`evaluateJson` and `evaluateFromJson` in Java) parses
the fields the expression reads. It took them from the reads worked out when the expression was
compiled, with no bindings, which take the call of a function that is not built in as reading its
context whole: with a host function called at its top, the whole input. #11 gave `jlens_reads` the
bindings and left the evaluations as they were, to be measured.

On the M1, in a program of its own over `jlens-core` (the median of 21 samples of 20 to 2,000
runs; `$rate` a host function marked `without_input`, as every function of the C API is), in
microseconds:

| Input | Expression | The analysis | Parsed whole | Cut, no bindings | Cut, with the bindings | The evaluation |
|---|---|---:|---:|---:|---:|---:|
| the order, 23 KB | `$rate(customer.name)` | 1.0 | 68.7 | 68.7 | 27.3 | 0.28 |
| | `$rate(items[0].code)` | 1.0 | 68.3 | 68.2 | 41.8 | 0.50 |
| | `$sum(items.$rate(quantity))` | 1.1 | 68.4 | 67.8 | 40.1 | 21.5 |
| | `$sum(items[type = 'electronic'].(unitPrice * $rate(quantity)))` | 1.9 | 68.3 | 67.9 | 50.1 | 19.2 |
| 10,000 items, 291 KB | `$rate(items[0].code)` | 1.0 | 1,256 | 1,256 | 1,098 | 0.50 |
| | `$sum(items.$rate(quantity))` | 1.1 | 1,250 | 1,252 | 850 | 903 |

- Cut with no bindings, the input is parsed whole, at the top of the expression and inside a
  step alike: a step's context is the item, so `items.$rate(quantity)` keeps every field of each
  item.
- Cut with the bindings, the order parses in 40 to 73% of the time, and the 10,000 items, whose
  text is mostly the fields read, in 68 to 87%.
- The analysis takes 1 to 2 µs for these expressions and 8.4 µs for the shipping rule (1,391
  bytes), with or without bindings: too much to run at each evaluation of a small input, where an
  evaluation takes 0.3 µs.

So the C API cuts by the bindings, when they hold host functions, and keeps the answer in the
compiled expression: the reads for the first four lists of host functions it is evaluated with.
The analysis depends on the bindings only through the names and signatures of their host
functions, so a list is known by those, written once when the bindings are made and shared with
the expressions that keep it; bindings made again with the same functions find the reads kept.
Bindings with no function, and no bindings, go by the reads of the compilation as before, with
nothing looked up. An expression evaluated with a fifth list of functions pays the analysis at
each evaluation with it, and the reads kept are released with the expression.

The reads were first kept by thread, the last eight pairs of an expression and bindings found by
the two objects' addresses. That lost its place with more expressions than eight on a thread,
as a rules engine has, and with bindings made for each document. A program that evaluates rules
of the form `$h(customer.tier)[0] = 'gold' and total > N` once each against a document of 83
bytes, in microseconds an evaluation (the median of 41 documents, the second of two rounds):

| Rules | Bindings | Kept by thread | Kept in the expression |
|---:|---|---:|---:|
| 4 | shared | 1.00 | 1.02 |
| 200 | shared | 2.13 | 1.03 |
| 200 | new for each document | 2.13 | 1.03 |
| 4 | new for each document | 2.26 | 1.15 |

The property test (`json_text_cut_for_host_functions_gives_what_the_whole_document_gives`)
evaluates 22 expressions that call a host function against JSON text and against the document
parsed whole, and fails when the reads are replaced by no field. `cargo xtask java-compare ab`,
whose cases call no host function, shows the path untouched ([below](#java)).

## #12: the size of the C API's library

The Windows library (x86-64, built as `cargo xtask java` builds it; compressed with `gzip -9`):

| Library | Raw | Compressed |
|---|---:|---:|
| 0.32.0 | 3,125,760 bytes | 1,167,801 |
| 0.33.0 | 3,664,384 (+17.2%) | 1,379,571 (+18.1%) |

On the M1, the macOS library goes from 2,734,592 to 3,133,344 bytes (+14.6%), the JAR with that
library alone from 1,237,257 to 1,455,594 (+17.6%), and the WebAssembly module, which had the
schema already, from 1,460,812 to 1,583,753 (+8.4%). The JAR of seven targets is built by the
`jar.yml` workflow from a push; 0.32.0's is 8,739,574 bytes.

ADR 0023 estimated 5 to 6% for the schema and the diagnostics, before the plan added what an
expression uses (#5 to #7, #11), recursive schemas (#8) and the loader (#9, #10). What the two
exports of #7 cost, by building the library with each cut down in turn:

| Left out, each row with the ones above | Raw | Compressed |
|---|---:|---:|
| The changes written as JSON | −11,264 | −3,556 |
| The comparison of two answers (`Uses::changes`) | −55,296 | −21,357 |
| An answer read from JSON (`Uses::from_json`) | −34,816 | −13,001 |
| An answer written as JSON | −8,192 | −4,991 |
| What an expression uses (`uses_of`) | −96,256 | −37,509 |
| Both exports | −205,824 | −80,414 |

The JSON forms, written and read, are 54 KB of the 206 KB, a quarter; the rest is the work the
exports are there for. The comparison stands out: 55 KB to compare two maps of small records,
as each member of a `Use` is compared through a generic `Change<T>` that the compiler copies for
each type. It is left as it is, and noted for 0.34's measure.

## #12: the plan's code checked whole

- **Fuzzing.** The workflow's six targets, run as it runs them for 10 minutes each under WSL
  from the seeds of `cargo xtask fuzz-seed` (0.65 to 2.3 million inputs each), found nothing.
  The `language_service` target reached none of the plan's code, as it types against a schema
  inferred from a sample: it now also runs the diagnostics, completion and hover against a
  recursive schema with a `$ref` to a document it was not given, and, for a text that compiles,
  checks that what it uses of each schema reads back from its JSON form and has no change
  against itself. Its 650,126 inputs include that.
- **Properties.** `jlens-schema`'s robustness properties with 100,000 cases, and its tests of
  recursive schemas and of references with 20,000, pass; so do `jlens-capi`'s with 100,000.
  The first run of `what_an_expression_uses` failed after 11,950 cases on `$rate(type)`: #11's
  assertion that a declared function, reading its arguments alone, adds no path to the answer
  for the whole input was wrong for a field the schema does not have, which the narrow answer
  names as unknown and the whole input, listed by what the schema has, does not. The property
  now excepts unknown paths; the code was right.
- **A recursive schema among the shared cases.** `fixtures/uses/cases.json` has, under
  `recursive`, a tree of nodes in two versions and five expressions: two that follow named
  fields three levels down, and three that read a value whole (`children`, `$`, `**.size`),
  under which the definition is listed once. The Rust crate, the C API, the CLI, the WebAssembly
  package and the Java package give the same answers and the same changes.

## #12: the release measured

Measured on 2026-11-11 at 2f7f5cf, against v0.32.0 (56df196), on the M1. `jlens-core` and
`jlens-regress` are the same code in both, so the comparisons show what a build's layout and the
version's text move, and the C API's change above. No other engine had a new version: their
columns keep 0.31's runs. The Ryzen ran no comparison, as the plan changed nothing on the
evaluation's path.

### Natively

`cargo xtask ab v0.32.0 HEAD`, 10 rounds, two comparisons; B against A, the median, in percent:

| Case | `evaluate` | `document` | `source` |
|---|---:|---:|---:|
| path | +0.4, +0.0 | +1.0, +1.0 | +2.1, +2.9 |
| names over items | +0.4, +0.5 | +0.1, +0.1 | +0.8, +1.1 |
| filter + aggregate | -1.3, -0.7 | -2.3, -1.9 | +0.3, -0.0 |
| group-by | +0.1, +0.2 | -0.6, -0.7 | +0.4, +0.2 |
| sort + map | +1.6, +1.4 | +1.0, +1.1 | +1.2, +2.3 |
| strings | -0.0, +0.1 | -0.2, +0.1 | +0.5, +0.6 |
| regex | +0.5, +0.4 | +2.6, +0.6 | +2.1, +2.3 |
| number formatting | +0.3, +1.3 | +0.5, -0.4 | +0.4, +0.8 |
| object constructor | -0.1, +0.0 | -0.7, -0.5 | +0.2, -0.4 |
| transform | -0.9, -0.8 | -0.4, -0.5 | -0.0, -0.1 |
| recursion | -3.3, -3.1 | -1.6, -2.9 | -1.4, -2.2 |
| shipping rule | -2.0, -1.7 | -2.1, -1.7 | -0.1, -0.3 |
| index into 10,000 items | -0.4, +0.6 | +1.0, +1.0 | +2.4, +2.7 |
| index in a variable | -1.4, +0.3 | +0.0, -0.5 | +1.5, +2.1 |
| regex over a long text | -0.2, -0.0 | +0.3, +0.1 | +0.2, +0.2 |
| distinct items | +1.2, +1.0 | +0.3, +0.6 | +0.4, -0.2 |
| tuples | -0.3, +0.3 | +0.1, -0.0 | +0.0, -0.0 |

With the same source on both sides, this is what two builds differ by on the M1 when only the
version in their text does: up to 3.3% (`recursion`, faster), and up to 2.9% slower (`path` and
the index into 10,000 items read through a `Source`). The floor of 0.8% that a
revision compared with itself shows is that of one build run twice, not of two builds.

### Python

`python.py ab`, two comparisons of five rounds (Python 3.14.10), in percent; none is marked:

| Case | `evaluate` | `document` |
|---|---:|---:|
| path | -1.0, -0.9 | -1.7, -1.8 |
| names over items | -0.3, -0.6 | -2.7, -2.5 |
| filter + aggregate | -0.5, -0.5 | +0.0, -0.0 |
| group-by | -1.3, -1.4 | -1.3, -1.4 |
| sort + map | -2.0, -2.5 | -2.1, -2.2 |
| strings | -1.2, -1.3 | -1.3, -1.2 |
| regex | -3.4, -3.4 | -2.4, -2.4 |
| number formatting | -1.4, -1.6 | -1.6, -1.5 |
| object constructor | -1.3, -0.1 | -0.1, +0.5 |
| transform | +0.8, +1.2 | -1.2, -0.4 |
| recursion | +0.1, +0.7 | -1.3, -0.2 |
| shipping rule | -1.5, -1.0 | -0.7, -0.4 |
| index into 10,000 items | -1.5, -0.8 | -1.2, -1.5 |
| index in a variable | -1.2, -1.4 | -1.8, -1.8 |
| regex over a long text | +0.0, -0.1 | -0.1, -0.5 |
| distinct items | +2.4, +2.5 | -1.4, -1.4 |
| tuples | -1.4, -1.2 | -1.6, -1.2 |

### WebAssembly

`compare.mjs --pkg --engines jsonata-js`, four processes a build, alternated, each on a new copy
of its package; the medians, 0.33.0 against 0.32.0:

| Case | `evaluate` | `evaluateJson` | `evaluateDocument` | jsonata-js, the control |
|---|---:|---:|---:|---:|
| path | +2.1% | +0.0% | +0.0% | +0.0% |
| names over items | -0.2% | -0.4% | +1.4% | -3.1% |
| filter + aggregate | -0.4% | -0.2% | -0.5% | -4.4% |
| group-by | -0.8% | -1.3% | -2.2% | -2.6% |
| sort + map | -0.5% | +0.1% | +1.0% | -1.0% |
| strings | -0.2% | +0.4% | -0.7% | -0.7% |
| regex | -0.5% | -0.4% | -0.2% | -0.5% |
| number formatting | -1.6% | -1.1% | +0.0% | -2.5% |
| object constructor | -0.9% | -1.2% | -1.1% | -2.5% |
| transform | +0.5% | -0.0% | +1.0% | -0.9% |
| recursion | +0.5% | +1.1% | +0.9% | -1.9% |
| shipping rule | -1.6% | -2.0% | -2.2% | -2.7% |
| index into 10,000 items | -1.5% | -0.9% | +0.0% | +0.0% |
| index in a variable | +0.0% | -1.3% | +0.0% | -2.0% |
| regex over a long text | +0.7% | +0.5% | +1.2% | +0.1% |
| distinct items | +0.8% | -0.8% | -0.3% | -1.1% |
| tuples | +1.1% | +0.8% | +0.6% | -0.3% |

The module is 8.4% larger and no case moves more than the control does.

### Java

`cargo xtask java-compare ab`, every part, five rounds (28, 17 and 9 minutes for the cases, the
documents and the threads). None of the 68 differences of the cases is marked: they run from
-3.2% (`names over items` against a `Document`) to +3.5% (`filter + aggregate` against a
`Document`), and `recursion` is 1.6 to 2.5% slower in its four columns, where it is 1.4 to 3.3%
faster natively: the layout of another library.

- The documents: one difference marked, faster (`($i := 0; items[$i].name)` over Java objects
  from the JSON text of 100,000 items, -6.2%); the others from -6.2 to +2.0%.
- The threads, from JSON text: 1.7 to 1.9% fewer evaluations a second on one, two and four
  threads (spreads of 0.2 to 1.5%), -0.1% on eight and +2.4% on 1,000 virtual threads; over Java
  objects, within 0.7%. These evaluations have no bindings and take the path they took: the
  one test added to it is whether there are bindings with functions.

### The tables

- Natively: `cargo xtask bench-table`, three copies, which agree within 1.4% on the cases (3.3%
  on the parse of the wide document), no case marked. Against the runs of 0.32's table, no case is
  more than 1.6% away.
- WebAssembly: the four processes of 0.33.0 above, within 4% but `path`, printed to a tenth (2.3
  to 2.5 µs with `evaluate`, 0.6 or 0.7 µs against a `Document`). jsonata-js, the control, is
  within 2.5% of its numbers for 0.32.0.
- Python: three runs of jlens alone, within 2.4%, the first with jsonata-python, the control,
  within 3% of its numbers for 0.32.0. `path` against a `Document` now prints 0.3 µs, not 0.4
  (0.354 to 0.348 µs in the comparison), which puts it level with `jsonatapy` on the page.
- Java: `java-compare` with every engine in one JVM, twice. In the first, jsonata-java, the
  control, was 10 to 19% slower than in the second on six cases against converted input
  (`filter + aggregate`, `sort + map`, `strings`, `object constructor`, `recursion`, the
  shipping rule), and up to 22% slower than for 0.32.0, with jlens and jsonata-jvm-compiler
  where they were; in the second it was back within 7% of 0.32.0's numbers, and slower on the
  threads instead (17,155 against 19,065 evaluations a second on one). A JVM
  compiles jsonata-java one of two ways, so the page takes each time from the faster of the two
  JVMs, and each engine's threads from the JVM where it made more.
- The regular expression engine and the limits were not measured again: `jlens-regress` and
  `jlens-core` are the code of 0.32.0.
- The thermal pressure stayed nominal, checked at each step; Rust 1.99.1, Node.js 24.22.2,
  OpenJDK 25.0.5, macOS 27.2.
