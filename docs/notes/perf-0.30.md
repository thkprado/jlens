# Performance notes: 0.30

How the measurements of roadmap 0.30 were made: methods, traps and intermediate numbers that the
[performance page](../src/architecture/performance.md) leaves out. The page holds the current tables
and what a user needs; the item numbers are those of [the roadmap's
history](../src/architecture/roadmap-history.md#030).

## 0.30 against 0.29.0

Measured for roadmap 0.30 #7, at c5246cc, after the dependencies were brought up to date (five
patches in `Cargo.lock`: `cc`, `flate2`, `smallvec`, `toml_datetime`, `toml_edit`; none is compiled
into `jlens-core`, and `cc` 1.11.1 only fixes a WebAssembly target's name), each run on a new copy
of its build, against builds of 0.29.0 made the same way on the same machine. A difference is read
against [what a build shows against
itself](../src/architecture/performance.md#comparing-two-revisions).

0.30 changes no code that an evaluation runs. Between v0.29.0 and HEAD, the sources of `jlens-core`,
`jlens-regress`, `jlens-capi`, `jlens-python` and the Java package are unchanged; the items touch
`jlens-schema`, the Wasm declarations and tests, and the web editor. Every difference below is
therefore read as placement, unless the runs agree on it in every comparison and mode, and then a
control build decides.

**Natively**, on Windows, `cargo xtask ab v0.29.0 HEAD`, ten rounds, in each mode, built as an
application builds it (two comparisons) and aligned (one). Of the 153 medians, none is past the
floor but `group-by` with `evaluate`, −3.6% and −3.4%, which reads +0.5% aligned. `recursion` reads
−2.0 to −2.8% in every mode and comparison, −1.4 to −2.2% aligned: the Windows heap's draw. The
aligned run with `source` read `path` −16.4% and `index into 10,000 items` −10.2% with the fastest
rounds equal (+1.5%, −0.1%): 0.29.0's aligned program drew the slow speed of a bimodal process in
most rounds.

Under Linux (WSL 2), against a `Document`, two comparisons and one aligned: every case within 2% but
`path`, −2.2% and −2.8% (178 → 174 ns), +1.1% aligned, and three slower ones. `regex` +2.4%, +2.5%
and +2.2% aligned (79.8 → 81.8 µs); `object constructor` +3.6% and +3.3%, +2.0% aligned; `regex over
a long text` −0.7% and −0.5%, +5.3% aligned (146 → 154 µs, whose plain runs read 164 µs on both
sides).

**In Python**, under Linux, `python.py ab`, two comparisons of five rounds: `regex over a long text`
+4.9 to +6.0% in all four numbers (160 → 169 µs), marked in each; `group-by` against a `Document`
+3.5% (marked) and +2.6%; `names over items` +2.6% and +4.1% with HEAD's spread at 6 and 8%; `regex`
with Python values +3.3% in both. Everything else within 2.8%.

**In Java**, under Linux, `java-compare ab`, two comparisons of five rounds, every part, against a
`Document` and over Java maps. Marked in both comparisons, and nothing else: `regex over a long
text` in all four ways, +10.2% and +10.3% against a `Document` (156 → 172 µs), +7.1% and +8.9% from
JSON text, +8.2 to +9.7% over Java maps. Unmarked but past 3% in both: `$count(items)` against a
`Document` +4.2% and +3.6% (0.50 → 0.52 µs), `filter + aggregate` from JSON text +3.0% and +3.6%.
The threads within 1.4% in the first comparison; in the second, 4 threads of `jlens` 3.5% fewer
evaluations a second and 2 threads of `jlens-objects` 5.3% more, with spreads of 4 to 12%.

**The control.** `regex over a long text` moved in every Linux build that links jlens into a library
with LTO (the wheel and the C API under Java), by the same amount in both comparisons, and the
opposite way to 0.29 (−6% in Python, −4% in Java). To tell layout from work, a third wheel was built
from v0.29.0's tree with only the five versions of `Cargo.toml` set to 0.30.0 (`cargo update -w`, so
with 0.29.0's lock), and compared with `python.py ab`: against 0.29.0, +5.6% and +5.5% (161 → 170
µs), marked, and `distinct items` +4.1% with Python values; against HEAD, −2.8% and −2.4%, with the
control's spread at 34%. The version's text alone gives the slower layout. The case reads 169 µs as
it did for 0.28.0 (170 µs).

WebAssembly has no measured floor, and is compared in the release's table alone, the median of three
runs against 0.29.0's table. `evaluateJson` and `evaluateDocument` are within 2.1% (`regex over a
long text` against a `Document`, 238 → 243 µs) or faster, by up to 3.6% (`names over items` from
JSON, 165 → 159 µs). Half the cells of `evaluate` read 2 to 4.5% slower: `filter + aggregate` 134 →
140 µs (its runs spread 10%), `strings` 185 → 190 µs, `object constructor` 263 → 270 µs. jsonata-js,
the control, read −4.1 to +0.5% (`object constructor` 1.21 → 1.16 ms).

The release's other tables, in the page: natively, `cargo xtask bench-table`, run twice. The first
run read every case 2 to 5% slower than 0.29.0's table (`regex` 179 µs, `recursion` 794 µs) where
`ab`, an hour before, put both releases equal; the second, run just after it, is within 2% of
0.29.0's table but for `sort + map` against a `Document` (380 → 391 µs) and `regex over a long text`
(209 → 214 and 210 → 215 µs), which `ab` puts within the floor: the day's machine. The page takes
the second. One of its copies drew slow times on four cases (`recursion` against a `Document` 917
µs, the heap's draw). Python, `python.py speed`, three runs of jlens, which agreed within 2%, and
one of jsonata-python, the control, within 2.3% of its 0.29.0 numbers. Java, `cargo xtask
java-compare` with jlens and jsonata-java, the control, then with jlens and jlens-objects, one best
of five runs each. jlens over a `Document` and from JSON text appears in both; they agreed within 3%
but for `path` against a `Document` (0.40 and 0.36 µs, 0.35 in both `ab` comparisons), and the page
takes the faster. jsonata-java within 7% but for `path` from JSON text (154 → 184 µs). jsonatapy,
jsonata2py, jsonata2js and jsonata-jvm-compiler have the versions they had for 0.27.0, so their
columns stay.
