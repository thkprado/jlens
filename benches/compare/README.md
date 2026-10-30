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
cargo xtask java-compare ab <A.jar> <B.jar> cases --rounds 5
```

It builds the harness as above, takes the package's own JAR out of `target/deps`, and runs a JVM
per run with one of the two first on the classpath. `--engines` and `--sizes` are the harness's;
`limits` measures no time, and is refused. A JAR older than the harness must still have what
`Engine.java` calls.

## Two builds of a package

`python.py ab` and `java-compare ab` do for a package what `cargo xtask ab` does natively: the
runs alternate, A then B, then B then A, three rounds by default, so that a drift of the machine
reaches both. They print each number's median for each build, the difference, and each build's
spread, the slowest run against the fastest, to the nanosecond, where the tables above round
(0.9 against 1.0 µs would read as 11%). `*` marks a difference above 2% that the runs agree on:
no run of one build reaches the other's. An unmarked difference, however large, is one the runs
do not settle. A JVM run is sometimes slow as a whole, by up to 1.4 times, which shows as a
spread, not a difference; run more rounds. In `threads`, the numbers are evaluations per second,
so that more is better there.

A mark does not prove that the code differs. The runs of a build share its native library's
file, and a time can follow the file: one wheel against a copy of itself in another environment,
the same bytes, marked `transform` at -6.5% and `object constructor` at -3.7% (Windows, three
rounds, 0.26 #13), and the same wheel measured 425, 449 and 456 µs on `transform` at three times
of one day, each with its runs within 2%. So read a mark under about 7% as a question: before
putting it down to a change, compare each build with a copy of itself, and see whether the mark
stays. For a Python build, copy its environment; the Java package unpacks its library to one
file per content, so a copy of a JAR shares it, and `-Djlens.library` must name the copy.
