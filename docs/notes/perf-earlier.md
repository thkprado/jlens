# Performance notes: 0.8 to 0.25

Measurements of earlier releases that the [performance page](../src/architecture/performance.md) carried until 0.27.0: how a
change was found and what it gained when it was made. The numbers are those of their
release, on the machine of the page, and older than its tables.

## Lazy input and the allocator (0.8)

When measured for 0.8, on this machine, such an expression cost more with lazy input than it did when `evaluate`
converted the whole input first: measured with `cargo xtask ab` against the revision before
lazy input (d0e9789), `filter + aggregate` is 7% slower, `strings` 10% and `regex` 4%. The
conversion is not the cause:
- The whole cost comes from step 2 of [ADR 0010](../src/adr/0010-lazy-input.md), and it does not depend on reading
  lazily. Converting the whole input eagerly at the current revision costs the same.
- Converting the document alone (`Document::new` and its drop) takes 94 µs at both
  revisions, and `evaluate_document` is as fast as before.
- The cause is Windows' default allocator. With `mimalloc` as the global allocator in both
  programs, the gap falls to 1–3%, which is the cost of the lazy values themselves, and every
  one of these cases runs in about half the time.

An application that calls `evaluate` on whole documents can set a faster global allocator;
the library leaves that choice to it. The `jlens` command sets one.

## The allocator of the command (0.17)

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

## WebAssembly: how the input is read, 0.17 to 0.26

jsonata-js works on the JavaScript objects it is given, so it never pays for converting the
input. jlens-wasm `evaluate` reads the input object as the expression reaches it: an object or
array of up to 16 values through one call to a JavaScript helper, a larger one whole, through
`JSON.stringify` and a parse into jlens's representation. The path case reads two small
objects. An expression that reads every item pays what converting the whole input does:
`JSON.stringify` of the input takes 41 µs, and parsing that text 219 µs. A path step reads
its one field alone, since 0.26, as with the native and Python inputs: `path` takes 4.9 µs
instead of 7.3 µs, the other cases within 2.1% (the median of four alternated runs, on a day
when both builds and jsonata-js measured about 7% above the table). A long array of objects
is read in one call with the fields the expression reads of each item, since 0.26 too, rather
than whole through `JSON.stringify` ([ADR 0010](../src/adr/0010-lazy-input.md#follow-up-long-arrays-of-objects-026-5)):
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

`evaluate` is faster than jsonata-js on fifteen of the seventeen cases, by up to 11× on recursion,
32× on `distinct items`, whose `$distinct` is quadratic in jsonata-js, and 440× on the index held
in a variable, which jsonata-js tests against every item. It is slower on two, `path` and the index into 10,000 items, which do almost no work, so the overhead
of calling into WebAssembly dominates. `names over items`, which reads every item and does little
with each, was slower by 5.3× in 0.25.0, as converting the input was nearly all of its time; since
0.26 #5, which reads the items' one field without converting them, it is faster (51 against 62
µs).

In 0.18.0 it was also slower on `number formatting`, by 11%; it now takes 84% less time (roadmap
0.19 #2 and #4, 0.26 #5).

Against a `Document` converted once, jlens-wasm is faster than jsonata-js on all seventeen cases,
by 14× on number formatting and filter + aggregate, 13× on sort + map and the shipping rule, 11×
on recursion and group-by, and 380× on `distinct items`. On `transform` its lead is 3.6×, since
0.26 #6 (42% in 0.25.0), and on the index into 10,000 items 22%.

From 0.19 on, `evaluate` and `evaluateDocument` build their result straight into
JavaScript values ([ADR 0017](../src/adr/0017-result-builder.md)): the builder writes it as a list
of operations, its numbers and its strings, and one call to a JavaScript helper builds the
values, decoding each key once. Before, the result was built as `serde_json` values, printed
as JSON text and given to `JSON.parse`. The cases with large results gained most: against a
`Document`, the object constructor took 466 µs, the transform 1.24 ms and `$` alone 376 µs,
against 305 µs, 947 µs and 114 µs after it. In 0.18.0, `transform` was slower than jsonata-js with
`evaluate`.

`evaluateJson` takes JSON text and returns JSON text. From 0.18.0 on, it parses the input
without the fields the expression does not read ([ADR 0016](../src/adr/0016-fields-read.md)). It
was faster than `evaluate` on eight of the fourteen cases in 0.25.0, and since 0.26 #5, which
reads long arrays with the fields the expression reads, it is slower on all of them but
`distinct items` (270 against 292 µs), which reads whole items; it still writes its result through
`serde_json` values. In 0.17.0, which parsed the whole text,
`path` took 296 µs, `names over items` 307 µs and `regex` 487 µs; the shipping rule, which reads
most of the input, takes the same time as then. On `path`, parsing
24 KB of text still costs far more than the evaluation, and on the two index cases parsing
the text of 10,000 items takes 4.0 ms, where `evaluate` reads the one item it picks: an input
evaluated more than once is best converted once into a `Document`.

## Python: results, values and the allocator (0.19)

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
  object ([ADR 0010, amended](../src/adr/0010-lazy-input.md#amendment-one-field-at-a-time-017)).
  The same change serves `evaluate` on a `serde_json::Value`: natively, `names over items`
  went from 152 to 18.5 µs with `cargo xtask ab --mode evaluate`, and no case was slower.
- `jsonatapy` also reads items through borrowed pointers, without changing their reference
  counts and without locking a `list` while it reads it. That takes `unsafe` code, which jlens
  forbids, and is not safe under free-threaded Python, where another thread can change the list.

From 0.19 on, the arrays and objects of a level cost one allocation each, not two: the source
writes them as members of one value it allocates for the level, which holds their Python
values (`input::Sources`, [ADR 0010, amended](../src/adr/0010-lazy-input.md#amendment-one-allocation-per-member-019)),
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
amended](../src/adr/0014-python-package.md#amendment-mimalloc-as-the-allocator-019)); mimalloc 2
outside Windows from 0.24.1 on ([ADR 0014, follow-up](../src/adr/0014-python-package.md#follow-up-mimalloc-2-outside-windows-0241)). Measured the
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

## The regular expression engine in 0.23

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

## What the limits cost (0.20)

From 0.20 on, every evaluation counts its steps for the step limit
([ADR 0018](../src/adr/0018-step-limit.md)). Without a limit the budget is too large to be spent,
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
  [Python table](../src/architecture/performance.md#python-against-the-other-implementations) is measured without it.

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
steps of the backtracker ([ADR 0009](../src/adr/0009-bounded-regex.md)): 1.7 % on average on the
[engine's benchmark](../src/architecture/performance.md#the-regular-expression-engine), with its spread explained there.

## Documents from Rust values against 0.20 (0.21)

The rest of the library runs as fast as in 0.20. `cargo xtask ab v0.20.0 .`, with the 0.21
code complete and ten rounds, puts every case of `evaluate` between 3.3% faster (`names over
items`) and 1.5% slower (`group-by`), except `transform` at 3.1% slower, which was 0.7% slower
over 20 rounds. With `--mode document`, every case is between 1.0% faster and 1.9% slower,
except `path`, 16% faster (240 ns against 287 ns). That was not looked into: no code on its
path changed but the order in which variables and functions are bound.

## Where the time goes, in detail

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
- **Names over many items.** A path step that reads an array passes it on as it is: `items`
  returns the input's array, and the step after it maps over that array's items instead of a
  copy of them. A step whose results hold no array to flatten makes them its sequence without
  copying them again. Before 0.16, `$count(items.code)` took 9.8 µs and now takes 7.0 µs,
  against 4.6 µs in jsonata-core; `$count(items)` went from 0.94 to 0.67 µs (0.39 µs). Looking
  the field up is not what remains: `$count(items.nope)` takes 4.9 µs, against 5.6 µs. A field
  found costs about 10 ns more per item, to keep the value, and `items.code` takes 22 µs,
  against 14 µs, most of it converting the 200 strings of its result.
- **Converting the input.** `evaluate` converts the parts of the input that the expression
  reads, as it reads them ([ADR 0010](../src/adr/0010-lazy-input.md)), and a path step reads only
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
  ([ADR 0010](../src/adr/0010-lazy-input.md#amendment-one-item-at-a-time-024)): from lazy JSON, and
  through a source that tells its length and reads one item. Built aligned against the
  revision before, `items[0].code` over 10,000 items takes 1.31 µs instead of 618 µs with
  `evaluate`, and 1.95 µs instead of 609 µs with `--mode source`, near the 0.67 µs it takes
  against a `Document`; the other cases are within 2%. A predicate that picks more than 16 items
  reads the array whole at once, unless, since 0.27 #8, they are at most one item in ten of it
  ([ADR 0010](../src/adr/0010-lazy-input.md#follow-up-a-predicate-that-picks-many-items-027-8)):
  `number formatting`, `items[[0..19]]` over 200 items, takes 19.2% less with `evaluate`, 42% less
  over Java maps, 20.8% less in Python and 5.1% less in WebAssembly. An index, also one held in a
  variable, still reads the array whole past 16 items, as it cannot tell how many will follow.

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
  ([ADR 0006, amended](../src/adr/0006-host-functions.md#amendment-host-functions-looked-up-on-a-miss-025)):
  `$a + $b` with sixteen functions bound takes 0.40 µs instead of 2.82 µs, 0.39 µs with none.
  In 0.25 every lookup carried the bindings to reach them, which made `recursion` (`$f(15)`,
  about 2,000 calls of a lambda, each looking variables up) slower than in 0.24.0: about 2%
  built aligned, after inlining `Frame::lookup` and `variable`, and 4.0% with `evaluate` and
  2.9% against a `Document` built as an application builds it. Since 0.26 the root frame holds
  the functions itself, shared with the `Bindings` through an `Arc`
  ([ADR 0006, follow-up](../src/adr/0006-host-functions.md#follow-up-the-functions-in-the-root-frame-026-2)),
  and lookups are as in 0.24.0. Against 0.24.0 (`cargo xtask ab v0.24.0 .`, ten rounds),
  `recursion` is within 1.1% in both modes, aligned or not, and every other case within 2.3%;
  `$a + $b` takes the same 0.38 to 0.39 µs with none, one or sixteen functions bound.
