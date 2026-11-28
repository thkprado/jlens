# Performance notes: 0.37

How the measurements of roadmap 0.37 were made: methods, traps and intermediate numbers that the
[performance page](../src/architecture/performance.md) and the guides leave out. The item numbers
are those of the plan, in [the roadmap](../src/architecture/roadmap.md).

## #4: the Java package without critical downcalls

**What changed on the path.** The three calls that read a buffer (its address, its length, its
release) were made with `Linker.Option.critical`, which skips the thread's transition out of Java
(0.26 #7, about 20 ns). Each may sweep the conversions of a closed document, and each locks or
frees, while it holds the garbage collector off; #4 made them ordinary downcalls. Since 0.26 #10
an evaluation replies into a scratch buffer, so the evaluation of a `Document` makes none of
them; an evaluation of JSON text makes two (the input buffer's address and its release), Java
objects as input two per evaluation, a host function's call three or more. #4 also finds a
thread's conversion of a document by its handle, with one atomic load, instead of through the
thread's cache of the table's objects.

**Method.** On the M1, `cargo xtask java-compare ab A.jar B.jar <part>`, one part at a time
(`documents`, `cases`, `threads`), 5 rounds in alternation, a JVM and a fresh copy of the native
library per run, engines `jlens` and `jlens-objects`. A is `abf9f6d` (the code of `d342dba`, the
commit before #4), B the same with #4's commit applied; each JAR built by `cargo xtask java
package -DskipTests` in its own clone. About 35 minutes in all. Logs: M1
`~/workspaces/logs/037-4/`.

**Results.** One difference marked (above 5%, with no run of one JAR reaching the other's): `path`
against a `Document`, 0.210 to 0.198 µs (-5.6%), the conversion found by handle. Over JSON text,
every case of `cases` is within ±0.8% and every row of `threads` within ±0.9%: the transitions
the two calls make again do not show beside the text's parsing. Java objects over JSON text read
from -0.7% to +3.9% in `documents` and `cases`, within their spreads (up to 9.9%); those over
converted objects read from -4.7% to +3.6%. Host functions are not in the harness, so their
calls were not timed.

## #5: the size limits while a result is built

**Memory.** A scratch crate over `jlens-core` with a counting global allocator (live bytes and
their peak) ran each case with a small limit, before and after. Before, every case built far
past its limit before `U1001`: `$replace` 20 MB against 100,000 bytes (a 10,000-byte
replacement for each of 1,000 matches, or 1,000 `$0`s for one match of 10,000 bytes), `$split`
42-49 MB and `$match` 258 MB against 1,000 items (a part or a match per character of 1 MB),
`$pad` 54 MB against 1,000 bytes (nine million `é`), `$fromMillis` 4.5 MB against 100,000
bytes (10,000 `[Y,100]`), and `[0..1999][($i := $; [1..2000].$i)]` 389 MB with no error at all
against 10,000 items. After, each peak is that of its input (3 MB for the 1 MB strings that
`$pad` makes, 100-500 KB for the others). `$join`'s overflow is on 32-bit targets only, so it
was read, not measured.

**Speed.** On the M1, `cargo xtask ab` of `4c9774e` (before) against the fix, 10 rounds, with
ten cases over `text-20k.json` added to `cases.json` for the run only (`$replace` with a string,
`$0` and a function, `$split` by text, characters and regex, `$match`, `$pad`, `$fromMillis`,
a filter with two positions). The first version checked each piece it appended, with the
error built inline: `$replace` was slower, `regex over a long text` +6.7%, a string pattern
+13.1%, `<$0>` +10.4%, about 1.3 ns a check; every other case within ±2%. With one check per
match on the string path and the error in a `#[cold]` function, a second run of the
`$replace` cases: `regex over a long text` +0.4%, a string pattern +1.2%, `<$0>` +1.3%, a
function +0.4%, `$split` by regex +0.7%. `regex` (a `$contains` filter, untouched) read +1.6%
and +2.2% in the two runs. Logs: M1 `~/workspaces/logs/037-5/`.

## #7: regex literals compiled once per expression

**What changed on the path.** Each evaluation compiled each regex literal the first time it met
it, and looked it up in a map keyed by a copy of its pattern and flags each time the node was
evaluated (two `String`s cloned per item of `items[$contains(code, /re/)]`). Now the compiled
pattern sits in the expression's `Compiled`, by node, found by binary search over the tree's
regex nodes and the node's address, filled by the first evaluation that meets it, on any
thread; each evaluation keeps its own `Rc` of it by node id, as it does for string literals.
`datetime.rs`'s two fixed patterns are `LazyLock` statics.

**Method.** On the M1, `cargo xtask ab` of `26aa9f7` (before) against the change, 10 rounds,
evaluate and document modes, with two cases added to `cases.json` for the run only: `dates
parsed` (`$sum(items.$toMillis('2024-05-06T07:08:09.123+02:00'))`) and `integer words`
(`$sum(items.$parseInteger('one hundred and twenty-three', 'w'))`). Logs: M1
`~/workspaces/logs/037-7/`.

**A trap in `eval_inner`.** The first version moved the node's code into a method that the
compiler left out of line. `regex` was 13-14% faster, but `group-by` +8.7%, `shipping rule`
+9.1%, `filter + aggregate` +3.7% and `tuples` +2.0%, none of which has a regex, the same in
document mode and in a build with every function and block aligned (so not placement).
Bisected: the `datetime.rs` part alone was within 1%; keeping the old map in `Evaluator`
(same struct shape), keeping the match keys by value, and removing the shared cache's plumbing
each left the slowdown. `eval_inner`'s frame had dropped from 0x380 to 0x170 bytes, its callees
were the same but for the regex code, and `sample` put the extra time in `eval_inner`'s own
samples (291 to 565 of a 5 s run of `group-by`). `#[inline(never)]` changed nothing;
`#[inline(always)]` (frame 0x2d0) brought the four cases back within 0.6%. Code added to or
removed from `eval_inner` can move unrelated cases by 10% through its register allocation:
measure every case, not only the one the change is for.

**Results**, inlined, evaluate mode (document mode within a point of each): `dates parsed`
1.82 ms to 148 µs (-91.8%), `integer words` 1.22 ms to 790 µs (-35.5%), `regex` 69.5 to 59.0 µs
(-15.1%; -12.5% without the cache across evaluations, so most of it is the copied key), every
other case from -4.5% (`recursion`, placement) to +1.7% (`sort + map`).

## #20: `jlens-regress`'s `utf16` feature and the searches of `&str`

**What changed.** With the `utf16` feature, a search of `&str` had no start predicate: `next_match`
went straight to trying every position, and the predicate itself had no literal to work from,
since the feature keeps literals as chars instead of forming literal bytes. 0.35.2 ports
`542243a`, `3e43472`, `c57833c` and `851f87b` (a start predicate in code units for `&[u16]`, and
the byte predicate for `&str` again), and, its own change, gives a char its UTF-8 bytes in the
byte predicate under the feature, a run of chars taken whole, as the code unit predicate does.
Upstream's master still has that gap. The default build, which jlens uses, compiles to the same
code.

**Method.** On the M1, the engine's benchmark, aligned, three binaries per tree: without the
feature, with it searching `&str`, and with it searching UTF-16 (`JLENS_REGRESS_UTF16`, added to
the benchmark for this item), before (`11ffb0d` with the new benchmark) and after, alternated,
3 runs each (the median of the runs' medians). Logs: M1 `~/workspaces/logs/037-20/ab2/` (`ab/`
holds the run before the char predicate).

| Case | Without the feature, before | after | `&str` with the feature, before | after | after, against without | UTF-16, before | after |
|---|---:|---:|---:|---:|---:|---:|---:|
| literal | 5.0 µs | 5.0 µs | 61.0 µs | 6.5 µs | +30.0% | 61.5 µs | 12.5 µs |
| literal, no match | 505 ns | 505 ns | 64.8 µs | 505 ns | +0.0% | 65.8 µs | 6.5 µs |
| case-insensitive literal | 6.1 µs | 6.1 µs | 51.0 µs | 6.6 µs | +8.2% | 48.6 µs | 10.4 µs |
| classes | 28.3 µs | 28.2 µs | 71.1 µs | 28.9 µs | +2.5% | 67.4 µs | 31.4 µs |
| classes, no match | 3.5 µs | 3.5 µs | 44.6 µs | 3.5 µs | +0.0% | 43.6 µs | 6.2 µs |
| alternation of words | 137 µs | 136 µs | 135 µs | 138 µs | +2.0% | 141 µs | 143 µs |
| words | 175 µs | 175 µs | 175 µs | 175 µs | −0.2% | 183 µs | 190 µs |
| captures | 27.4 µs | 27.4 µs | 77.7 µs | 29.0 µs | +5.8% | 84.1 µs | 34.8 µs |
| lazy | 36.2 µs | 36.2 µs | 81.1 µs | 38.0 µs | +5.0% | 83.0 µs | 43.2 µs |
| lines | 28.9 µs | 29.0 µs | 30.9 µs | 31.0 µs | +6.9% | 33.8 µs | 34.3 µs |
| backreference, no match | 489 µs | 486 µs | 484 µs | 486 µs | −0.1% | 498 µs | 489 µs |
| lookahead | 410 µs | 409 µs | 394 µs | 410 µs | +0.3% | 410 µs | 433 µs |
| lookbehind | 151 µs | 149 µs | 181 µs | 156 µs | +4.6% | 188 µs | 164 µs |
| Unicode text | 151 µs | 152 µs | 171 µs | 152 µs | −0.1% | 175 µs | 181 µs |
| Unicode text, case-insensitive | 22.3 µs | 22.3 µs | 62.4 µs | 22.4 µs | +0.4% | 48.3 µs | 10.3 µs |
| Unicode property | 210 µs | 211 µs | 256 µs | 210 µs | −0.4% | 294 µs | 293 µs |
| anchored, short text | 42 ns | 41 ns | 41 ns | 41 ns | +0.0% | 43 ns | 44 ns |

**Results.** Without the feature, every case within ±1.2%. With it, `&str` searches had been up to
128 times slower (`literal, no match`) and are now within 8% of the build without it, but
`literal`, +30%: its program still matches the literal char by char, where literal bytes compare
it at once, which the feature gives up by upstream's design. The ports alone (`ab/`) left the
literals (`literal` 12 times, `literal, no match` 129 times, `captures` and `lazy`, which begin
with `user=` and `path=`, 2.8 and 2.2 times); the char predicate brought them back. UTF-16
searches with a predicate take 48% to 90% less; those whose predicate is a class most of the
text matches (`\w`, `\S`) or none take 1% to 5.5% more (`lookahead` +5.5%, `words` +3.5%,
`Unicode text` +3.7%), the cost of upstream's scan for code units before each attempt, kept as
upstream wrote it. Compiling is unchanged. jlens itself builds the crate without the feature, so
nothing of jlens's own benchmark changes.

## #14: what the WebAssembly module is made of

**Method.** Each release's module is the `jlens_bg.wasm` of its npm package (`npm pack
jlens-wasm@X.Y.Z`), built by the release workflow; 0.37's commits were built here with `cargo
xtask wasm` (Rust 1.100.0, binaryen 132 from npm), whose 0.36.0 is 1,013 bytes larger than
the published one and 0.1 KiB smaller gzipped. Sizes are gzip at its best level, as `cargo xtask wasm` measures them, and brotli at
quality 11. The variants were built from HEAD (`4d0029c`) with parts of `crates/jlens-wasm/src`
left out by hand, not committed. The split by crate is `twiggy top` (twiggy 0.8.0) on the module
before `wasm-bindgen` and `wasm-opt`, which still has its function names: each function goes to
the first crate of its name outside `core`/`alloc`/`std`, so that a generic function goes to the
crate whose type it was made for. Scripts: the scratch directory of the session, not kept.

**Each release.**

| Release | Bytes | gzip | brotli | `jlens.global.js` gzip |
|---|---:|---:|---:|---:|
| 0.20.0 | 1,371,895 | 510.9 KiB | 376.0 KiB | — |
| 0.24.0 | 1,381,313 | 516.0 KiB | 379.4 KiB | — |
| 0.27.0 | 1,390,779 | 515.5 KiB | 384.0 KiB | 745.9 KiB |
| 0.28.0 | 1,425,835 | 522.4 KiB | 388.6 KiB | 758.5 KiB |
| 0.29.0 | 1,442,710 | 530.1 KiB | 393.9 KiB | 768.1 KiB |
| 0.30.0 | 1,449,757 | 533.1 KiB | 395.7 KiB | 771.8 KiB |
| 0.31.0 | 1,446,819 | 531.8 KiB | 395.4 KiB | 770.2 KiB |
| 0.32.0 | 1,459,424 | 534.5 KiB | 396.6 KiB | 776.4 KiB |
| 0.33.0 | 1,582,410 | 582.4 KiB | 432.3 KiB | 842.6 KiB |
| 0.34.0 | 1,679,209 | 615.8 KiB | 455.7 KiB | 892.4 KiB |
| 0.35.0 | 1,690,082 | 619.6 KiB | 458.7 KiB | 898.4 KiB |
| 0.36.0 | 1,733,237 | 637.7 KiB | 470.2 KiB | 921.9 KiB |

The two large steps are the language service's: 0.33's schemas (`uses`, recursive `$ref`, documents
and their loader with its own URI resolver), +47.9 KiB, and 0.34's types, +33.4 KiB.

**0.37, item by item.**

| Commit | Item | Bytes | gzip | Change |
|---|---|---:|---:|---:|
| `b786433` | 0.36.0 | 1,734,250 | 637.6 KiB | |
| `2bac089` | #5, the size limits while building | 1,734,103 | 637.9 KiB | +0.3 KiB |
| `26aa9f7` | #8, template strings | 1,742,683 | 641.8 KiB | +3.9 KiB |
| `11ffb0d` | #7, regex literals compiled once | 1,744,655 | 642.6 KiB | +0.8 KiB |
| `ce7dc3c` | #20, `jlens-regress` 0.35.2 | 1,744,662 | 642.7 KiB | +0.1 KiB |
| `7626b55` | #9, the reader's gaps | 1,749,230 | 644.5 KiB | +1.8 KiB |
| `f740632` | #9, `fields()` | 1,771,834 | 653.2 KiB | +8.7 KiB |
| `d8a4ed3` | #13, `rename` and `sites_with` in the crates | 1,779,284 | 655.5 KiB | +2.3 KiB |
| `bda62da` | #13, `rename` in the module | 1,823,123 | 673.3 KiB | +17.8 KiB |
| `4d0029c` | #19, fuzzing (HEAD) | 1,823,130 | 673.3 KiB | +0.0 KiB |

#4, #10, #11, #12 and #15 leave the module as it was. 0.37 adds 35.7 KiB, half of it `rename`:
until the module exported it, `d8a4ed3`'s rewriting was dead code that the linker dropped, and the
binding inlines all of it (18.5 KB of code in one function before `wasm-opt`, about 30 KB with
the sorts of its edits and the recorder of sites). `cargo xtask wasm` measures with `flate2`,
whose output is up to 1.5 KiB longer than these (639 KiB for `b786433`): HEAD's 673 KiB leaves
about 6.5 KiB of the 680 KiB budget.

**By part.** Leaving parts out of HEAD's module:

| Module | Bytes | gzip | brotli |
|---|---:|---:|---:|
| Whole (HEAD) | 1,823,130 | 673.3 KiB | 496.0 KiB |
| Without the language service (`service.rs`: `LanguageService`, `inferSchema`, `functions`, `usesChanges`, …) | 1,382,940 | 505.2 KiB | 373.6 KiB |
| Evaluation only (also without `evaluateWithAudit`, `ast`, `reads`, `unionReads`, `parseRecover`) | 1,313,577 | 475.8 KiB | 350.8 KiB |

The language service is a quarter of the module, 168 KiB gzipped: `jlens-schema` and the
monomorphized code it brings (`serde_json` for its replies, `indexmap`, its own uses of
`regress` for `pattern`). The audit, the syntax tree and the fields read cost 29 KiB more;
`reads` itself stays, as an evaluation uses it to read only the fields an expression needs.

Code by crate before `wasm-opt`, in bytes (names and `wasm-bindgen`'s sections left out):

| Crate | Whole | Without the language service | Evaluation only |
|---|---:|---:|---:|
| `jlens_core` | 566,111 | 545,816 | 524,473 |
| `jlens_schema` | 272,621 | 0 | 0 |
| `regress` | 288,025 | 253,111 | 253,111 |
| `serde_json` | 90,584 | 61,582 | 43,188 |
| `jlens_wasm` | 68,985 | 33,434 | 31,013 |
| `core` and `alloc` | 91,038 | 86,146 | 83,241 |
| `indexmap`, `hashbrown`, `foldhash` | 69,015 | 43,280 | 42,645 |
| `js_sys` | 32,712 | 32,712 | 32,712 |
| data | 401,492 | 392,532 | 390,316 |

The data, 397,060 bytes in the final module, is mostly tables: 140,703 zero bytes, 42,565 bytes
of text (messages, the function catalog's descriptions, names), and `regress`'s Unicode property
tables, 22,860 ranges in 182,880 bytes, about 38.5 KiB gzipped, which `\p{…}` needs at any time
since a pattern can name any property. What remains for an evaluation is `jlens_core`, `regress`
and those tables, about 476 KiB gzipped.

**For ADR 0015 (0.38 #2).** A module without the language service would be 505 KiB gzipped
(374 KiB with brotli), 25% less than one with it. Two modules from one crate, by a Cargo feature,
would cost the release a second build and package, and a page that evaluates and edits would load
both or the larger one. Not decided here. The budget, 680 KiB, leaves HEAD about 6.5 KiB.

## #16: the release measured

Measured on 2026-11-28 on the M1 at `285e0b7` against v0.36.0. Thermal pressure nominal at each
step; Rust 1.100.0, Node.js 24.22.3, OpenJDK 25.0.5.1, Python 3.14.10. New versions since 0.36:
`jsonatapy` 2.2.17 (built from its source distribution); jsonata-js 2.3.0, jsonata2js 0.1.2,
`jsonata2py` 0.1.3, jsonata-python 0.7.2, jsonata-java 0.9.11 and jsonata-jvm-compiler 1.0.7
have not changed. Scripts and logs: M1 `~/workspaces/logs/037-16/` (`m037-16.sh`, `m037-16c.sh`
for the compile bisect, `m037-16w.sh` and `rx.mjs` for the WebAssembly one).

- **Natively**, `cargo xtask ab v0.36.0 HEAD`, 10 rounds, twice in each mode: `regex` −13.3 to
  −13.6% (`evaluate`), −15.3 and −15.4% (`document`), −10.9 and −11.1% (`source`), #7; `path`
  −3.6% (`evaluate`), −5.7% (`document`), −1.8 and −2.0% (`source`); every other case within
  2.9% (`index in a variable` from a `Source`, +2.9% in one run, −0.4% in the other).
- **Dates**, the five cases of 0.36 #10 added to `benches/cases.json` for the run only, 10
  rounds: `$toMillis` of an ISO 8601 text −88.1% (`evaluate`, 1.89 ms → 224 µs) and −88.4%
  (`document`), its pattern a `LazyLock` since #7; the other four within 1%.
- **Python**, `python.py ab`, two comparisons of five rounds: `regex` −10.3% with Python values
  and −14.7 to −14.9% against a `Document`, marked in both. Marked in one of the two only: the
  shipping rule with Python values −7.0% (−3.6% in the other), `strings` −8.4% and −9.6% in the
  second, where a round of 0.36.0 spread 10.5 to 11.8%. Every other case within 3.4%.
- **WebAssembly**, `compare.mjs --pkg --engines jsonata-js`, four processes of each build
  alternated: `regex` −11% with `evaluate` (133 → 118 µs), −8% with `evaluateJson`, −18%
  against a `Document` (94 → 77 µs); `regex over a long text` +3.8% with `evaluate` (177 →
  184 µs, every run of 0.37 above every run of 0.36.0) and +2.5% against a `Document`; every
  other case within 2.1%. jsonata-js 2.3.0, the control, within 2.3% of its 0.36.0 column.
- **Java**, `cargo xtask java-compare ab`, every part, five rounds, once: the cases mark `regex`
  in all four forms (−6.6 to −13.5%), `path` against a `Document` (−6.9%), and `filter +
  aggregate` over converted Java objects, +9.2% (52.2 → 57.0 µs, spreads 2.3 and 1.7%); the
  table's two runs, of the same code built from the tree, give 51.9 and 51.1 µs, 0.36.0's
  time, so a draw of the JIT in that run rather than a change. The documents mark nothing (−4.8
  to +2.6%); the threads within 2.1%.
- **The tables.** Natively, `bench-table`, three copies within 1.9%. Against 0.36's table,
  `regex` (67 → 59 µs, 58 → 50 µs) and `compile/path`, 0.59 → 0.64 µs (its copies 620 to
  642 ns). Python: three runs of jlens and `jsonatapy`, one of jsonata-python, within 1.4% of its
  numbers for 0.36.0; `jsonatapy` 2.2.17 within 3.5% of 2.2.15 but the transform, 724 → 511 µs
  (661 → 429 µs converted once), and `python.py limits` gives 2.2.15's answers. Java: two JVMs,
  the faster of each time; jsonata-java within 4.6% of its 0.36.0 column (−4.6% on `path`, +4.1% on `number formatting`), its `filter + aggregate` 30% slower in one JVM.
- **The regular expression engine**, aligned, three runs without an interrupt and two with: the
  first plain run read `literal` 8.1 µs where the other two read 5.1 and 5.3 µs (the median
  kept); the medians within 2% of 0.36.0's table but `Unicode text` (155 → 151 µs) and
  `anchored, short text` (43 → 41 ns); the interrupt 2.8% over the 17 cases where it was 2.4%,
  `lookbehind` +9.8% and `anchored, short text` +9.8% in both pairs.

**`compile/path`.** Criterion's `compile` group at 0.36.0, at each of #8, #7 and #13 and its
parent, and at HEAD, one build after the other in a target directory of their own: `compile/path`
614 ns at 0.36.0, 603 ns at HEAD, every commit between 603 and 629 ns; `group-by` 3.07 to
3.13 µs and the shipping rule 21.2 to 21.5 µs throughout. The table's 0.64 µs against 0.59 is the
spread between builds of the same code, not a commit.

**WebAssembly, `regex over a long text`.** `$length($replace(text, /\w+/, 'x'))` over 20 KB, in
`rx.mjs`: the case alone, 15 rounds of 200 ms, in a module built at each commit around #5, #7 and
#20, four processes of each in alternation (medians, µs, `evaluate` / `evaluateDocument`):

| Build | `evaluate` | `evaluateDocument` |
|---|---:|---:|
| v0.36.0 | 169.7 | 143.8 |
| `1ed7251`, #5's first commit (the limits while `$replace` builds) | 173.6 | 146.4 |
| `2bac089`, #5 (the limits once per match) | 174.6 | 148.3 |
| `26aa9f7`, #8 | 173.0 | 147.2 |
| `11ffb0d`, #7 | 172.0 | 146.5 |
| `ce7dc3c`, #20 (`jlens-regress` 0.35.2) | 175.8 | 149.8 |
| HEAD | 175.7 | 150.9 |

Two steps of about 2%, each about the spread between processes: #5's checks as `$replace`
appends to its text, and #20's ports, whose new code is almost all under the `utf16` feature,
which jlens does not build, so more likely the module's layout than more work. Natively the case
is −0.9 to −1.0% in every mode,
and #5 was measured within 1.3% on the M1 (`$replace`). Left as it is: 7 µs on 20 KB of text,
in WebAssembly only.

**The sizes**, the JAR and the gzipped `.wasm` against 0.36.0's and the WebAssembly guide's
table: measured after the rename's fixes, on the final code, as the module and the native library
carry `rename`.
