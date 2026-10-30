# Comparisons with other JSONata implementations

The harnesses here compare jlens's packages with the other implementations for their platform,
on the workload of `../cases.json` and its inputs. Each one first checks that every
implementation gives jlens's result, then times it. The tables they print are those of the
[performance page](../../docs/src/architecture/performance.md).

| Harness | Compares | Needs |
|---|---|---|
| `compare.mjs` | the WebAssembly package with jsonata-js, in Node.js | Node.js, `npm install` here, `cargo xtask wasm` |
| `python.py` | the Python package with the other Python packages | a virtual environment per package (the script's docstring) |
| `java/` | the Java package with the other JVM implementations | a JDK 25 or later |

The same directory holds two checks against jsonata-js, which are not benchmarks:
`differential.mjs`, run by `cargo xtask differential`, and `doc-examples.mjs`, which runs the
book's examples.

## JavaScript

```sh
cargo xtask wasm && cd benches/compare && npm install && node compare.mjs
```

The script's header lists what each column times and its options.

## Python

```sh
python benches/compare/python.py speed jlens=<python> jsonatapy=<python> ...
```

The script's docstring says how to set up the environments, and what `speed` and `limits` print.

To compare two builds of the package, install each wheel in an environment of its own:

```sh
python benches/compare/python.py ab A=<python> B=<python>   # three rounds; --rounds N before them
```

See [below](#two-builds-of-a-package) for what it prints.

## Java

```sh
cargo xtask java-compare                                  # every part, every engine
cargo xtask java-compare documents --sizes 1000           # one part, the smaller document
cargo xtask java-compare cases --engines jlens,jsonata-java
```

The command builds the C API and the Java package's JAR (`crates/jlens-java/java/target`),
then this Maven project, which fetches the other implementations into `target/deps` (emptied
first, so that a JAR of an earlier version is not left on the classpath), and runs `compare.Compare`
in one JVM with `-Xmx4g`. The JAR is never installed into the local Maven repository, so that a
local build cannot hide the package published on Maven Central.

The engines (`--engines`, jlens first, as the reference):

| Name | Package | Its input |
|---|---|---|
| `jlens` | `io.github.thkprado:jlens`, this repository | a `Document`, or JSON text |
| `jlens-objects` | the same, over Java objects (`Expression.evaluate(Object)`), read one level at a time | maps, lists and boxed values, which Jackson 2 parses |
| `jsonata-java` | `com.dashjoin:jsonata`, a port of jsonata-js to Java objects | maps, lists and boxed values |
| `jsonata-jvm-compiler` | `io.github.vlad-public-code:jsonata-jvm-compiler`, which compiles an expression to a Java class with `javac` at run time | Jackson 2's `JsonNode` |

Their versions are properties of `java/pom.xml`. Searched on Maven Central on 2026-10-25, these
were left out: `dev.vepo:jsonata.java` 0.1.0, whose results have no JSON form (`items.id` reads
as `1, 2`) and which gives `{"k":[1,1]}` for `items{'k': $count($)}`; `io.hyperfoil.tools:jjq-jsonata`,
which translates JSONata to jq at compile time; IBM's `com.ibm.jsonata4java:JSONata4Java`, no
longer compared from roadmap 0.25 #14 on (its results until then are in ADR 0021); Apache
Camel's `camel-jsonata`, which runs JSONata4Java; and the WebJars of jsonata-js.

The parts (the first three when none is given):

- `documents`: the documents of ADR 0021's spike, 1,000 and 100,000 items of five fields each
  (`--sizes` picks others), the spike's expressions and `($i := 0; items[$i].name)`.
- `cases`: the workload of `../cases.json`.
- `threads`: evaluations per second of `$sum(items.price)` on the JSON text of 100 items, on 1
  to all hardware threads and on 1,000 virtual threads, as in the spike. An engine whose
  compiled expressions are not thread-safe (jsonata-java) compiles one per thread.
- `limits`, run only when named: how each engine behaves at the limits, the cases of
  `python.py limits` (input nested deeply, cyclic input, unbounded recursion, an endless loop,
  with and without a depth limit or a timeout). Each case runs in a JVM of its own, killed after
  30 seconds, and the table says what came out: a result, an error code, an exception, the
  JVM's exit code, or still running. jlens reads Java objects there, as cyclic input needs;
  jsonata-jvm-compiler gets a Jackson tree built the same way.

`documents` and `cases` print microseconds per evaluation, the best of five runs of 0.3 s after
a second of warm-up, with two columns per engine: on the input converted once, and on JSON text
parsed at each evaluation. A result that differs from jlens's shows as `differs`, an exception as
`error`, and what they were goes to the standard error. Each engine returns its result as its own
objects, which are not serialized: jlens as maps, lists and boxed values (`Expression.evaluate`,
`evaluateFromJson`).

Keep the machine idle while it runs: the three default parts take about 20 minutes in all, and
`limits` about four more.

To compare two builds of the package, keep a copy of each JAR, as the next build replaces the
one in `crates/jlens-java/java/target`:

```sh
cargo xtask java-compare ab <A.jar> <B.jar>               # every part, jlens and jlens-objects
cargo xtask java-compare ab <A.jar> <B.jar> cases --rounds 7
```

It builds the harness as above, takes the package's own JAR out of `target/deps`, and runs a JVM
per run with one of the two first on the classpath, and a new copy of that JAR's native library
through `-Djlens.library`. `--engines` and `--sizes` are the harness's;
`limits` measures no time, and is refused. A JAR older than the harness must still have what
`Engine.java` calls.

## Two builds of a package

`python.py ab` and `java-compare ab` do for a package what `cargo xtask ab` does natively: the
runs alternate, A then B, then B then A, five rounds by default, so that a drift of the machine
reaches both, and each run has a new copy of its build's native library. They print each number's
median for each build, the difference, and each build's spread, the slowest run against the
fastest, to the nanosecond, where the tables above round (0.9 against 1.0 µs would read as 11%).
`*` marks a difference above 3.5% in Python and 5% in Java that the runs agree on: no run of one
build reaches the other's. An unmarked difference, however large, is one the runs do not settle.
A JVM run is sometimes slow as a whole, by up to 1.4 times, which shows as a spread, not a
difference; run more rounds. In `threads`, the numbers are evaluations per second, so that more
is better there.

The copies matter because a time follows the file of the native library: one wheel against a
copy of itself, each with its one file, marked `transform` at -6.5% (0.26 #13). With a new copy at
each run, a build against itself (Windows, 0.26 #14) gives these differences, and the marks are
set past them:

| Command | Comparisons | Half of the numbers within | Nine in ten within | Largest |
|---|---|---:|---:|---:|
| `python.py ab`, 5 rounds | 3, 28 numbers | 1.1% | 2.1% | 3.2% |
| `java-compare ab`, 5 rounds | 2, 108 numbers | 0.8% | 2.8% | 6.2% |

The largest difference in Java is in `threads`, where the runs spread by up to 59%, and is not
one the runs agree on; the largest they agreed on is 4.3%. Three rounds are too few: two sets of
three runs fall apart by chance one time in ten (with five, one in 126), and they marked 1 of 84
and 7 of 324 numbers in a build against itself. A difference under the mark can be real, on a
number under 2 µs above all; it is settled when two comparisons give it. A build compared with
itself needs no copy of its own: `A=<python> B=<python>`, or the same JAR twice.
