# Performance notes: 0.36

How the measurements of roadmap 0.36 were made: methods, traps and intermediate numbers that the
[performance page](../src/architecture/performance.md) and the guides leave out. The item numbers
are those of the plan, in [the roadmap](../src/architecture/roadmap.md#036-what-adoption-asks).

## #3: what a document costs a pool of threads

**Method.** A probe crate outside the workspace (path dependency on `jlens-capi`'s `rlib`, so
mimalloc is its allocator, as in the JAR) calls the C API's functions as the Java package does:
a buffer made and filled with the text, `document_new` (or `document_take`), the buffer freed,
one evaluation of `$count(items[price > 50])` with a JSON reply. Memory is mimalloc's
`mi_process_info`: the process's committed memory, after `mi_collect(true)` on every thread for
"live", and its peak, both less the commit before the first document. The text, 16.2 MB, is read
from a file, so that making it sets no peak of its own: 80,000 items of eight fields (`id`,
`sku`, `name`, `price`, `qty`, `tags` of three strings, `attrs` of three fields, `note`). "Own":
T threads each make, evaluate and hold a document at once; "shared": the first thread makes it,
all T evaluate it. "Cut" keeps the read of the expression, `items.price`. One process per row;
Windows 11, Ryzen 7 3700X (memory, not time: the machine's load does not change it).

Native memory in MB, and in times the text per thread (one run each; the cut rows with several
threads move by about 0.3 times from run to run, as threads finish their conversion at
different moments):

| Case | T | Live | Peak, `document_new` | Peak, `document_take` |
|---|---|---|---|---|
| own, whole | 1 | 110 (6.8) | 150 (9.3) | 133 (8.2) |
| own, whole | 4 | 440 (6.8) | 555 (8.6) | 505 (7.8) |
| own, whole | 16 | 1,763 (6.8) | 2,149 (8.3) | 2,010 (7.7) |
| own, cut | 1 | 32 (2.0) | 56 (3.5) | 49 (3.0) |
| own, cut | 4 | 127-145 (2.0-2.2) | 300 (4.6) | 195 (3.0) |
| own, cut | 16 | 566-589 (2.2-2.3) | 910 (3.5) | 779 (3.0) |
| shared, whole | 4 | 388 (6.0) | 437 (6.7) | 436 (6.7) |
| shared, whole | 16 | 1,503-1,509 (5.8) | 1,735 (6.7) | 1,736 (6.7) |
| shared, cut | 4 | 75 (1.2) | 126 (1.9) | 123 (1.9) |
| shared, cut | 16 | 260-302 (1.0-1.2) | 464 (1.8) | 495 (1.9) |

One thread, step by step (MB over the commit before, `document_take`): the buffer 17.3; the
document made 115.9 whole, 38.7 cut; evaluated 133.5 and 49.5; after `mi_collect`, 110.3 and
32.1. So the conversion is 5.8 times the text whole, 1.0 cut; the text kept is 1; the evaluation's
working memory, 18 MB whole and 11 MB cut, is the rest of the peak, freed when it returns.
`document_new` adds the copy of the buffer, 17 MB, to the peak of every document made, and
nothing to a shared document's threads, which do not make it.

**What it decided.** `document_take`: the C API takes the bytes of the host's buffer for the
text, which the Java package no longer frees after. The other candidates are left for the
maintainer, with these numbers:

- **A document for one thread, which drops its text once converted**, would save the text, 1 in
  6.8 of a whole document and 1 in 2 of a cut one, for threads that each make their own. But
  the C API converts per operating system thread, and a virtual thread may run on another
  carrier at its next evaluation: such a document would fail there, where a Java thread is not
  one carrier.
- **A core `Document` shared between threads** (`Send + Sync`, an ADR): one conversion for all
  threads, and no text kept: the shared rows fall to about one conversion, 94 MB whole for 16
  threads instead of 1.5 GB, and the own rows lose their text. Its cost is in the evaluator's
  hot path, reference counts made atomic, to measure with `cargo xtask ab`.
- **A smaller conversion**: at 5.8 times the text, about 1,200 bytes per item of 210 bytes of
  text, it is the largest share of every row.

## #12: `$formatNumber` from the shortest decimal digits

`cargo xtask ab HEAD .` on the M1, evaluate mode, case `number formatting`
(`items[[0..19]].$formatNumber(unitPrice * quantity, '#,##0.00')`), 10 rounds, the working tree
with #12 against `dac6349`, the code before it:

| case | before (median) | after (median) | B/A | before (fastest) | after (fastest) | B/A |
|---|---:|---:|---:|---:|---:|---:|
| number formatting | 16.7 µs | 12.5 µs | -25.4% | 16.6 µs | 12.4 µs | -25.2% |

The number is converted once to its shortest digits (`ryu_js`), then scaled and rounded on
those digits. Before, `round` converted it to a string and back twice to shift the decimal point,
and `write_fixed` formatted the exact binary value with `format!`.

## #15: `regress`'s commits after `95c0404`

The engine's benchmark on the M1, built aligned as the performance page says, `ac02f66` (before
the ports) against `9819d5b` (the last one), three rounds alternated, the median of the rounds;
each round's times stayed within 0.5 % of the others:

| case | before | after | change | with an interrupt, before | after | change |
|---|---:|---:|---:|---:|---:|---:|
| literal | 5.1 µs | 5.0 µs | -2.0% | 5.2 µs | 5.1 µs | -1.9% |
| literal, no match | 508 ns | 506 ns | -0.4% | 509 ns | 508 ns | -0.2% |
| case-insensitive literal | 6.4 µs | 6.2 µs | -3.1% | 6.5 µs | 6.4 µs | -1.5% |
| classes | 27.7 µs | 28.2 µs | +1.8% | 28.7 µs | 28.4 µs | -1.0% |
| classes, no match | 3.5 µs | 3.5 µs | 0.0% | 3.5 µs | 3.5 µs | 0.0% |
| alternation of words | 129.9 µs | 136.0 µs | +4.7% | 133.9 µs | 138.2 µs | +3.2% |
| words | 178.9 µs | 177.3 µs | -0.9% | 183.9 µs | 180.5 µs | -1.8% |
| captures | 27.7 µs | 27.3 µs | -1.4% | 27.9 µs | 27.9 µs | 0.0% |
| lazy | 35.5 µs | 36.3 µs | +2.3% | 35.3 µs | 36.7 µs | +4.0% |
| lines | 29.6 µs | 29.0 µs | -2.0% | 28.8 µs | 29.7 µs | +3.1% |
| backreference, no match | 441.8 µs | 484.6 µs | +9.7% | 443.1 µs | 501.0 µs | +13.1% |
| lookahead | 416.8 µs | 410.5 µs | -1.5% | 415.1 µs | 424.1 µs | +2.2% |
| lookbehind | 153.8 µs | 148.3 µs | -3.6% | 164.4 µs | 161.8 µs | -1.6% |
| Unicode text | 155.5 µs | 153.4 µs | -1.4% | 159.7 µs | 155.2 µs | -2.8% |
| Unicode text, case-insensitive | 21.1 µs | 22.1 µs | +4.7% | 21.5 µs | 22.4 µs | +4.2% |
| Unicode property | 215.9 µs | 211.0 µs | -2.3% | 218.6 µs | 215.4 µs | -1.5% |
| anchored, short text | 44 ns | 43 ns | -2.3% | 47 ns | 45 ns | -4.3% |
| compile: date-time | 6.9 µs | 6.5 µs | -5.8% | 6.8 µs | 6.6 µs | -2.9% |
| compile: e-mail, case-insensitive | 7.5 µs | 7.4 µs | -1.3% | 7.6 µs | 7.4 µs | -2.6% |
| compile: Unicode property | 3.2 µs | 3.2 µs | 0.0% | 3.2 µs | 3.2 µs | 0.0% |

The geometric mean of the 20 rows is -0.3 % without an interrupt and +0.3 % with one.

The same benchmark at each of the 21 commits, aligned, once each, tells which port moves a case:

- `backreference, no match` (`(\w+)\s+\1` over prose, which never matches) goes from 443 to
  475 µs with `1ff2853` (a capture group's undo saves its start or its end alone, pushing an entry
  only when the bound changes) and to 495 µs with `9209939` (a loop's undo split by how it was
  entered), then stays at 480 to 500 µs. These are the commits that let a backtracking entry
  shrink to 16 bytes, which the later ones build on.
- `Unicode text, case-insensitive` (`γράμματα` with `i`) goes from 21.1 to 22.2 µs with
  `f0972db`, which packs a `CharSet` instruction's four characters into 16 bytes, to unpack at
  each match.
- `lazy` (`path=(.*?) `) rises to 40 µs with the same two commits as the backreference and comes
  back to 37 µs with `e84e9d8`, which makes the lazy loop of one character lazy.
- `alternation of words` (`\b(?:fox|dog|cat)s?\b`) moves between 130 and 250 µs from one commit
  to the next with no pattern, by 90 % at commits that do not touch its path (`7ac9163` 248,
  `5a76444` 167), and `ac02f66` itself took 130 µs in the build of the table and 162 µs in the
  bisection's: its speed follows the layout of the build, even aligned. The +4.7 % above is
  not a cost of the ports.

Through jlens, `cargo xtask ab ac02f66 9819d5b` on the M1, evaluate mode, 10 rounds:

| case | before (median) | after (median) | B/A | before (fastest) | after (fastest) | B/A |
|---|---:|---:|---:|---:|---:|---:|
| regex, unaligned | 66.7 µs | 69.4 µs | +4.0% | 66.0 µs | 68.2 µs | +3.3% |
| regex over a long text, unaligned | 86.2 µs | 88.9 µs | +3.2% | 85.7 µs | 88.8 µs | +3.5% |
| regex, aligned | 66.2 µs | 68.1 µs | +2.9% | 66.0 µs | 67.5 µs | +2.2% |
| regex over a long text, aligned | 86.3 µs | 90.8 µs | +5.3% | 86.0 µs | 90.6 µs | +5.3% |

The other 15 cases stayed within 1.5 % unaligned; aligned, `path` read +2.1 %, a case with no
regular expression. `regex over a long text` is `$replace(text, /\w+/, 'x')`, but the engine
alone does not slow on `\w+`: the same bisection with a case `\w+` over the prose (added to
`engine.json` for the run only) gives 161.7 µs before and 159.7 µs after, with no step at any
commit. What `ab` shows is outside the engine's search: in `Searcher`, through which jlens
searches and the benchmark does not, or in the layout of the jlens binary. #10 measures the
release against 0.35.0 and shows whether it stays.

## #17: the date/time pictures of jsonata-js#869

`$toMillis` no longer builds a regular expression from its picture for each call: a matcher
walks the parts, in time linear in the timestamp's length times the parts. The benchmark has no
date case, so `cargo xtask ab` measures nothing here; a rough run on the Ryzen (Windows, release
CLI, the fastest of five runs of 20,000 calls, less the 41 to 56 ms of starting the process and
`$count([1..20000])`) compares 0.35.0 with this change:

| expression, 20,000 calls | 0.35.0 | #17 |
|---|---:|---:|
| `$toMillis('2018-12-10 14:00:NN', '[Y]-[M]-[D] [H]:[m]:[s]')` | 271 ms | 103 ms |
| `$toMillis('Monday, 10th December 2018', '[FNn], [D1o] [MNn] [Y]')` | 324 ms | 99 ms |
| `$fromMillis(…, '[FNn], [D1o] [MNn] [Y] [h]:[m01] [P]')` | 56 ms | 62 ms |

`$fromMillis` changed only in its width checks and the roman bound; its 6 ms are within this
machine's noise, and #10 checks them on the M1. The pictures that took longest before, in a
debug build: `[Y` and 600,000 times `0.` and `]` took 414 s and now under 2 s (a set in place of
a list for the regularity of separators), and `[f,150000000]` 63 s and about 4 GB, now `D3142`.

## #18: a transform's matches checked against its copy

jsonata-js#870 checks, before a transform changes anything, that every match is an object or
array of the copy, by collecting all of the copy's objects into a set. jlens's first port walked
the copy until it had found every match, with the standard library's hasher: on the M1, `cargo
xtask ab d3f9a65 .` gave the benchmark's transform (`$ ~> |items|…|`) +9.9% in `evaluate`
mode (187 → 206 µs) and +11.0% in `document` mode (170 → 188 µs). A location that is a path of
names, `$`, `*` or `**`, whose steps may filter, reads only values of the copy, which holds no
function, so the check is skipped for it; the walk uses foldhash for what remains. Then, 10
rounds each:

| case | mode | d3f9a65 | #18 | B/A |
|---|---|---:|---:|---:|
| transform | `evaluate` | 188 µs | 187 µs | −0.5% |
| transform | `document` | 170 µs | 169 µs | −0.4% |
| `$ ~> \|items[quantity > 0]\|…\|`, a scratch case | `evaluate` | 198 µs | 198 µs | −0.1% |

Before filters took the fast path, the scratch case paid the walk: 198 → 210 µs, +5.7%. A
location that may reach outside the copy (`$$`, a variable, a function call, a constructor) still
pays it, in time linear in the copy.
