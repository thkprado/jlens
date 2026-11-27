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
