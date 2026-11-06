# Performance notes: 0.29

How the measurements of roadmap 0.29 were made: methods, traps and intermediate numbers that the
[performance page](../src/architecture/performance.md) leaves out. The page holds the current
tables and what a user needs; the item numbers are those of
[the roadmap](../src/architecture/roadmap.md#029-completing-values-and-domains-the-application-provides).

## 0.29 against 0.28.0

Measured for roadmap 0.29 #8, at 3f8dfc7, after the dependencies were checked (all were already
at their newest versions), each run on a new copy of its build, against builds of 0.28.0 made the
same way on the same machine. A difference is read against
[what a build shows against itself](../src/architecture/performance.md#comparing-two-revisions).

0.29 changes no code that an evaluation runs. Between v0.28.0 and HEAD, `jlens-core` gains
`offsets.rs` (0.29 #6), one line in `lib.rs` and one in `error.rs`; `jlens-regress`,
`jlens-capi`, `jlens-python` and the Java package's sources are unchanged, and `Cargo.lock` moves
only jlens's own versions. The other items touch `jlens-schema`, the language server, the Wasm
bindings and the web editor. Every difference below is therefore read as placement, unless the
runs agree on it in every comparison and mode.

**Natively**, on Windows, `cargo xtask ab v0.28.0 HEAD`, ten rounds, in each mode, built as an
application builds it (two comparisons) and aligned (one). Of the 153 medians, none is past the
floor: the largest are `index into 10,000 items` −3.8% and `object constructor` +3.8% against a
`Document` in the second comparison (floor 4.1% for the object constructor), and `path` −2.2% and
−2.7% with `evaluate`. The aligned run with `evaluate` read `path` +27.7%, `strings` +19.8% and
`regex` +16.8% with the fastest rounds equal (−0.5%, −0.1%, −0.8%): HEAD's aligned program drew
the slow speed of a bimodal process in most rounds. Run again on the same programs, the three
read −4.4%, −1.9% and −2.7%.

Under Linux (WSL 2), against a `Document`, two comparisons and one aligned: every case within
2.9% but two: `path`, +6.0% and +2.9% (168 and 173 → 178 ns), and −3.9% aligned (180 → 173 ns). 0.28
saw the same case move by 2.4% between consecutive commits that did not touch it. Aligned,
the second, `regex over a long text`, is −5.0%, which the plain comparisons put at +1.2% and +0.3%.

**In Python**, under Linux, `python.py ab`, two comparisons of five rounds: `regex over a long
text` −5.8 to −6.1% in all four numbers (170 → 160 µs), marked twice, the spread of one build
14% in each comparison, as the run draws one of two speeds. `regex` with Python values −3.7%,
marked once, and −2.4% in the other comparison. Everything else within 3.3%.

**In Java**, under Linux, `java-compare ab`, two comparisons of five rounds, every part, against
a `Document` and over Java maps. Marked: `path` from JSON text −5.4% and −5.0% (32.2 → 30.5 µs),
and 4 threads of `jlens`, 6.0% fewer evaluations a second in the second comparison (3.6% fewer,
unmarked, in the first; 8 threads −2.0%, 16 and the virtual threads unchanged). Unmarked but in
both comparisons: reading JSON text +2.7 to +4.7% on the spike's documents and `index into 10,000
items` (+4.1%, +4.3%), and `regex over a long text` against a `Document` −4.2% and −4.3%. Over Java
maps, the cases from JSON text read faster in both comparisons, unmarked: `path` −6.0% and −5.0%,
number formatting −5.1% and −4.4%, `names over items` −4.3% and −2.6%, with spreads of up to 23%.

WebAssembly has no measured floor, and is compared in the release's table alone, the median of
three runs against 0.28.0's table: jlens's cells are within 2% or faster, by up to 6% (number
formatting with `evaluate`, 91 → 86 µs; `evaluateJson` on 10,000 items 4.10 → 3.91 ms), but for
`distinct items` against a `Document`, 25 → 26 µs, and `regex over a long text` against a
`Document` (234 → 238 µs, +1.7%). jsonata-js, the control, moved by 2.5% at most. The first run's
`evaluateJson` on `path`, the first case, took 138 µs against 88 in the others.

The release's other tables, in the page: natively, `cargo xtask bench-table`, three copies,
within 2% of 0.28.0's or up to 3.4% faster (the transform 495 → 478 µs, `sort + map` against a
`Document` 393 → 380 µs, recursion 792 → 770 µs), which `ab` puts within the floor: the day's
machine. `recursion` against a `Document` drew 898 µs in one copy, the system allocator's slow
draw. Python, `python.py speed`, three runs of jlens and one of jsonata-python, the control,
within 3% of its 0.28.0 numbers. jlens's 160 µs on `regex over a long text` puts it ahead of
`jsonatapy` (165 µs) in both columns, by the same placement as above. Java, `cargo xtask
java-compare` with jlens and jsonata-java, the control, then with jlens and jlens-objects for the
column over Java maps, one best of five runs each: jsonata-java within 7% but for its recursion
(1.59 → 1.43 ms) and its index in a variable (1.13 ms → 934 µs, which read 962 µs for 0.27.0).
jsonatapy, jsonata2py, jsonata2js and jsonata-jvm-compiler have the versions they had for
0.27.0, so their columns stay.
