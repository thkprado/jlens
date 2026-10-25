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

## Java

```sh
cargo xtask java-compare                                  # every part, every engine
cargo xtask java-compare documents --sizes 1000           # one part, the smaller document
cargo xtask java-compare cases --engines jlens,jsonata-java
```

The command builds the C API and the Java package's JAR (`crates/jlens-java/java/target`),
then this Maven project, which fetches the other implementations, and runs `compare.Compare`
in one JVM with `-Xmx4g`. The JAR is never installed into the local Maven repository, so that a
local build cannot hide the package published on Maven Central.

The engines (`--engines`, jlens first, as the reference):

| Name | Package | Its input |
|---|---|---|
| `jlens` | `io.github.thkprado:jlens`, this repository | a `Document`, or JSON text |
| `jsonata-java` | `com.dashjoin:jsonata`, a port of jsonata-js to Java objects | maps, lists and boxed values |
| `jsonata4java` | `com.ibm.jsonata4java:JSONata4Java`, IBM's | Jackson 3's `JsonNode` |
| `jsonata-jvm-compiler` | `io.github.vlad-public-code:jsonata-jvm-compiler`, which compiles an expression to a Java class with `javac` at run time | Jackson 2's `JsonNode` |

Their versions are properties of `java/pom.xml`. Searched on Maven Central on 2026-10-25, these
were left out: `dev.vepo:jsonata.java` 0.1.0, whose results have no JSON form (`items.id` reads
as `1, 2`) and which gives `{"k":[1,1]}` for `items{'k': $count($)}`; `io.hyperfoil.tools:jjq-jsonata`,
which translates JSONata to jq at compile time; Apache Camel's `camel-jsonata`, which runs one of
the engines above; and the WebJars of jsonata-js.

The parts (all of them when none is given):

- `documents`: the documents of ADR 0021's spike, 1,000 and 100,000 items of five fields each
  (`--sizes` picks others), and the spike's expressions.
- `cases`: the workload of `../cases.json`.
- `threads`: evaluations per second of `$sum(items.price)` on the JSON text of 100 items, on 1
  to all hardware threads and on 1,000 virtual threads, as in the spike. An engine whose
  compiled expressions are not thread-safe (jsonata-java, JSONata4Java) compiles one per
  thread.

`documents` and `cases` print microseconds per evaluation, the best of five runs of 0.3 s after
a second of warm-up, with two columns per engine: on the input converted once, and on JSON text
parsed at each evaluation. A result that differs from jlens's shows as `differs`, an exception as
`error`, and what they were goes to the standard error. jlens returns its result as JSON text, so
its times include writing it; the others return their own objects, which are not serialized.

Keep the machine idle while it runs: every part takes about 20 minutes in all.
