# Performance notes: 0.34

How the measurements of roadmap 0.34 were made: methods, traps and intermediate numbers that the
[performance page](../src/architecture/performance.md) leaves out. The item numbers are those of
the plan, whose table the tag keeps in
[the roadmap](https://github.com/thkprado/jlens/blob/v0.34.0/docs/src/architecture/roadmap.md#034-the-types-the-diagnostics-know).
The plan's work is
in the typing of `jlens-schema`; in `jlens-core`, only the parse of a body (#8) and the two date
fixes (#2) changed, neither on the path the tables measure.

## #14: the diagnostics against 0.33.0

The diagnostics run at each keystroke. A program over `LanguageService::diagnostics` (a scratch
crate, the release profile with `lto` and one codegen unit) times each case as the median of 15
batches of about 5 ms; five processes of each build, alternated, each from a new copy of its
program, on the M1. Three builds: 0.33.0, `main` before this item (946f9d8), and the fixes below
(221f240). The schema is the repository's largest fixture (`fixtures/cli/order.schema.json`),
but where a row says otherwise; the rules declare `$minimum` and `$rate`, as
`fixtures/workspace` does. In microseconds, the median of the five processes:

| Case | 0.33.0 | 946f9d8 | 221f240 |
|---|---:|---:|---:|
| `1`, no schema | 6.49 | 6.52 | 0.23 |
| `1` | 6.48 | 6.54 | 0.26 |
| `customer.login` | 7.29 | 7.39 | 1.06 |
| `$count(items) > 2` | 8.99 | 10.34 | 2.15 |
| `$contains(customer.name, "VIP")` | 9.68 | 10.86 | 2.69 |
| `$sum(items[unitPrice > $minimum].($rate(type) * unitPrice * quantity))` | 20.33 | 22.12 | 6.23 |
| the same with three misspelt names (`fixtures/workspace/rules/mistakes.jsonata`) | 41.64 | 43.62 | 17.28 |
| the shipping rule (`fixtures/shipping-rule.jsonata`) | 42.73 | 51.63 | 35.52 |
| a field of an object of 200 fields | 7.12 | 7.33 | 1.07 |
| a field it does not have, suggested from the 200 | 65.03 | 65.29 | 28.77 |
| a field of an object of 2,000 fields | 7.12 | 7.33 | 1.07 |
| a field it does not have, suggested from the 2,000 | 630.70 | 631.14 | 153.49 |
| two fields of an object of 10 `patternProperties` | 9.70 | 63.59 | 5.19 |
| `$count(items) > 2`, a boolean expected (#5) | | 10.35 | 2.15 |
| `unitPrice * quantity`, as the body of `items[]` (#8) | | 7.79 | 1.53 |

The processes of a build agree within 2.5% (3.4% on the 2,000 names), but for one of 0.33.0 on
`1` without a schema, the first process of the run.

- **The fixed part** (about 17 µs on the Ryzen, found by 0.33 #3, 6.5 µs on the M1) was the
  catalog of built-in functions: `jlens_core::function_catalog` builds it anew, a copy of each
  signature, and the diagnostics copied it again, with every name and parameter, for the calls
  of the text, even when it had none. The catalog is now built once, and a call looks its
  function up by name; the whole list is built only to suggest a name for one that is unknown.
  The typing of a call read the catalog twice too.
- **Before the fixes, 0.34 was 5 to 21% slower than 0.33.0** where the text has operators or
  calls: the typing of #3 to #6 does more at each node. After them, every case is faster than
  0.33.0, from 17% (the shipping rule) to 96%.
- **`patternProperties`** (#9): each look-up of a field compiled every pattern of its object,
  about 28 µs a field for 10 patterns. The compiled patterns are now kept per thread, up to 256,
  the map emptied when it is full; a pattern that is not a regular expression is kept as such.
- **The suggestion for an unknown field** compares its name with every field's, by edit
  distance up to 2. A name whose length is more than two characters away is now skipped before
  the distance is computed, an ASCII name is lowercased without a new string, and the rows of the
  distance are reused and stop as soon as no cell is within 2. Among 2,000 fields, 631 → 153 µs;
  what is left is real work, as about a thousand of the names (`field10` to `field999`) are within
  two characters of `fieldx` and share its first five. A test checks the new distance against
  the previous one.

## #14: the size of the libraries

The C API's library on Windows (x86-64, built as `cargo xtask java` builds it; compressed with
`gzip -9`), 0.33.0 against 0.34's code at 221f240:

| Library | Raw | Compressed |
|---|---:|---:|
| 0.33.0 | 3,664,384 bytes | 1,379,571 |
| 0.34 | 3,970,048 (+8.3%) | 1,499,625 (+8.7%) |

On the M1, the macOS library goes from 3,134,032 to 3,354,896 bytes (+7.0%), the JAR with that
library alone from 1,464,175 to 1,571,836 (+7.4%), and the WebAssembly module from 1,589,939
to 1,680,520 (+5.7%): 588 to 617 KiB compressed, under the budget of 640 KiB that #9 set.

**The comparison of two answers of `uses`**, 55 KB in 0.33 #12, was looked at again by building
the Windows library with parts of it cut out:

| Left out | Raw |
|---|---:|
| The comparison of the enumerations | −9,728 |
| The comparison of each path's `Use` (the enumerations with it) | −23,552 |
| The whole comparison (`Uses::changes` giving no change) | −62,976 |

So the comparison of a path is 23 KB, and the other 40 KB are the maps by path that the answer
is built of, `added`, `removed` and `changed` for the input and each variable, with their JSON
writing, which an answer with no change never reaches and the compiler then leaves out:
generic code of `BTreeMap` and of serde, made once for each type of value. Writing the comparison
of a `Use` by hand, without what the compiler derives for it, saved 1.5 KB, and was not kept.
The comparison is left as it is: 1.6% of the library.

## #14: the plan's code checked whole

- **Properties.** With `PROPTEST_CASES=100000`, in release mode: `jlens-schema`'s robustness
  and keyword properties (3 min 40 s), `jlens-lsp`'s and `jlens-capi`'s robustness, and
  `jlens-core`'s 47 robustness properties (3 min 42 s); `jlens-schema`'s recursive schemas and
  references with 20,000. All pass.
- **Fuzzing.** The `language_service` target reached none of 0.34's code: it typed no result
  and no context, and its schemas had no `anyOf`, open object or tuple. It now also runs the
  diagnostics, `result_type` (whose spans it checks), completion and hover with a boolean
  expected, and as the body of `lines[]` with a number expected, over a schema with `anyOf` and
  `oneOf`, `patternProperties` (one of them not a regular expression), `additionalProperties`,
  `prefixItems` and `if`/`then`/`else`. The workflow's six targets, run as it runs them for 10
  minutes each under WSL from the seeds of `cargo xtask fuzz-seed`, three at a time, found
  nothing: from 0.54 million inputs (`language_service`) to 2.6 million (`parse`) each.

## #14: the release measured

Measured on 2026-11-14 at 221f240, against v0.33.0, on the M1. `jlens-core` changed only in
the parse of a body and in two date functions, neither on the cases' path; `jlens-regress` is
the same code. No other engine had a new version but jsonata-python, the Python control, 0.7.1
to 0.7.2. The Ryzen ran no comparison, as the plan changed nothing on the evaluation's path.

The M1's Rust moved from 1.99.1 to 1.100.0 since 0.33's tables. Both sides of each comparison
are built with it, so the comparisons show nothing of it; the tables do: `transform` with
`evaluate` takes 186 µs, where the table of 0.33.0 had 227, and `ab` gives 0.33.0's code 190 µs
as well. `recursion` is 2% slower in both modes, for 0.33.0's code too.

### Natively

`cargo xtask ab v0.33.0 HEAD`, 10 rounds, two comparisons; B against A, the median, in percent:

| Case | `evaluate` | `document` | `source` |
|---|---:|---:|---:|
| path | +0.8, +0.8 | +0.0, +0.0 | -1.0, -0.3 |
| names over items | +0.5, +1.6 | +0.0, +0.0 | -0.0, +0.7 |
| filter + aggregate | +0.3, +1.9 | -0.0, +1.5 | +0.0, +0.5 |
| group-by | +0.0, +0.9 | +0.6, +1.6 | +0.0, +0.1 |
| sort + map | +0.2, +1.1 | -0.4, +0.8 | -0.2, -0.4 |
| strings | +0.3, +1.6 | -0.2, +1.3 | +0.1, -0.7 |
| regex | +0.9, +2.4 | +1.5, +2.2 | +0.6, +1.5 |
| number formatting | +0.9, +1.2 | +0.9, +0.7 | +1.1, +0.5 |
| object constructor | -0.2, +1.9 | -0.3, +1.5 | -0.2, -0.3 |
| transform | +0.0, +1.6 | -0.1, +2.0 | -0.0, -0.0 |
| recursion | +0.4, +0.5 | -0.0, -0.2 | -0.0, +0.8 |
| shipping rule | +0.1, +1.7 | -0.1, +1.4 | +0.2, -0.1 |
| index into 10,000 items | +0.0, +2.9 | -0.3, +0.7 | +0.3, +0.9 |
| index in a variable | +0.3, +3.4 | -0.2, +0.2 | -0.6, -0.2 |
| regex over a long text | +0.6, +2.3 | +0.6, +0.4 | +0.6, +0.7 |
| distinct items | -0.0, +1.3 | +0.4, +1.6 | +0.7, +0.7 |
| tuples | +0.2, +1.5 | -0.1, +1.2 | -0.2, -0.1 |

Within 3.4%. The second comparison has `evaluate` 0.5 to 3.4% slower on every case, and
`document` up to 2.2%, where the first has both within 1.5%: the run, not the code, as `source`
stayed within 1.5% in both.

### Python

`python.py ab`, two comparisons of five rounds (Python 3.14.10), in percent:

| Case | `evaluate` | `document` |
|---|---:|---:|
| path | -2.9, -1.7 | +2.1, +2.8 |
| names over items | -2.5, -2.4 | +0.4, +2.1 |
| filter + aggregate | -1.4, -1.4 | +0.3, +0.2 |
| group-by | -0.4, -0.1 | -0.2, -0.1 |
| sort + map | -0.7, -0.9 | -0.8, -1.2 |
| strings | -1.0, -1.7 | -0.1, -0.7 |
| regex | -1.6, -1.8 | -0.9, -0.9 |
| number formatting | -1.0, -0.9 | -1.2, -1.2 |
| object constructor | -0.6, -0.1 | -0.0, +0.1 |
| transform | -1.6, -0.3 | -1.3, +1.5 |
| recursion | +0.5, +0.1 | -1.2, -0.5 |
| shipping rule | -2.1, -1.0 | -0.7, +0.5 |
| index into 10,000 items | -3.8, -1.8 | +0.4, +0.9 |
| index in a variable | -3.1, -1.2 | +0.4, +0.8 |
| regex over a long text | -0.3, -0.3 | -0.3, -0.0 |
| distinct items | -1.7, -1.7 | -0.2, -0.1 |
| tuples | -0.1, -0.4 | +0.4, +0.2 |

One difference marked in the first comparison, faster: the index into 10,000 items with
`evaluate`, −3.8%, which is −1.8% in the second.

### WebAssembly

`compare.mjs --pkg --engines jsonata-js`, four processes a build, alternated, each on a new copy
of its package; the medians, 0.34 against 0.33.0:

| Case | `evaluate` | `evaluateJson` | `evaluateDocument` | jsonata-js, the control |
|---|---:|---:|---:|---:|
| path | +0.0% | +0.8% | -7.7% | +0.0% |
| names over items | +0.2% | +0.3% | +0.0% | +1.8% |
| filter + aggregate | +1.3% | +1.1% | +1.3% | +2.4% |
| group-by | -0.1% | -0.3% | +0.2% | +4.2% |
| sort + map | +0.0% | +0.6% | +2.1% | +0.6% |
| strings | -0.8% | -0.8% | -0.5% | +0.8% |
| regex | -1.3% | -0.9% | -0.4% | -1.1% |
| number formatting | -0.1% | -0.3% | -0.2% | +0.4% |
| object constructor | -0.1% | +0.1% | +0.1% | +1.2% |
| transform | +0.4% | +0.4% | +0.3% | +0.0% |
| recursion | +0.2% | -0.1% | +0.4% | +0.8% |
| shipping rule | +0.2% | -0.1% | +0.4% | +0.9% |
| index into 10,000 items | +0.0% | -0.2% | +0.0% | +0.0% |
| index in a variable | -1.4% | -0.2% | +0.0% | +0.0% |
| regex over a long text | +1.2% | +0.5% | +0.6% | -0.0% |
| distinct items | +1.3% | +1.5% | +1.2% | +0.9% |
| tuples | +0.7% | +0.9% | +0.7% | +1.1% |

The module is 5.7% larger, and no case moves more than 2.1% but `path` against a `Document`,
0.65 to 0.60 µs, printed to a tenth; jsonata-js, the control, moves up to 4.2%.

### Java

`cargo xtask java-compare ab`, every part, five rounds (28, 17 and 9 minutes for the cases, the
documents and the threads). None of the 68 differences of the cases is marked: they run from
−1.4 to +2.6%. The documents: none marked, from −9.2% (`$` converted, whose runs spread by 13
to 17%) to +2.1%. The threads: within 1.0% on every count, from JSON text and over Java objects.

### The tables

- Natively: `cargo xtask bench-table`, three copies, which agree within 2.6% (`tuples`, one copy
  slower), no case marked. Against 0.33's run on the M1, every case is within 2.0% but
  `transform` with `evaluate` (227 → 186 µs, Rust 1.100, above), `compile/path` (+4.1%, its
  copies spread 2.5%), `serde_json::from_str` (-3.5%), number formatting (-2.4 and -3.3%),
  `path` against a `Document` (+2.6%), and `recursion` (+2.2% in both modes), the object
  constructor against a `Document` and `Document::new` without its drop (+2.3%). On the cases among
  them, `ab` finds no difference past 1% between 0.33.0 and 0.34 in its first comparison.
- WebAssembly: the four processes of 0.34 above, within 4% but two cases against a `Document`
  printed to a tenth (`path` 0.6 or 0.7 µs, the index in a variable 1.0 or 1.1 µs). jsonata-js,
  the control, is within 4.2% of its numbers for 0.33.0 (Node.js 24.22.3, was 24.22.2).
- Python: three runs of jlens alone, within 3.7% (`object constructor` against a `Document`, one
  run slower), the first with jsonata-python 0.7.2, the second with 0.7.1, the control, within 3%
  of its numbers for 0.33.0. 0.7.2 is up to 9% slower than 0.7.1 (`group-by`, `sort + map`),
  and is the version the table now names. `path` against a `Document` prints 0.4 µs again, not
  0.3 (0.348 to 0.355 µs in the comparison), which puts `jsonatapy` ahead on it on the page.
- Java: `java-compare` with every engine in one JVM, twice, as for 0.33. In the first,
  jsonata-java was 7 to 16% slower than in the second on six cases against converted input
  (`filter + aggregate`, `sort + map`, `strings`, `object constructor`, `transform`, the shipping
  rule); in the second, jsonata-jvm-compiler took 35 µs on the index in a variable, against
  6.8 µs in the first (34 µs on 0.33's page). The page takes each time from the faster JVM. In both,
  jsonata-java takes 897 µs or more on `recursion` against converted input, where 0.33's page has
  780; the other cases of the control are within 4% of it, but `path` (-12%) and `distinct items`
  (-12%), faster.
- The regular expression engine and the limits were not measured again: `jlens-regress` is the
  code of 0.32.0, and nothing on the evaluation's path changed.
- The thermal pressure stayed nominal, checked at each step; Rust 1.100.0, Node.js 24.22.3,
  OpenJDK 25.0.5, macOS 27.2.1.
