# AGENTS.md — jlens-java

Guidance for working in the Java package. The repository-wide rules are in the root
[AGENTS.md](../../AGENTS.md); the decisions are in ADR 0021.

## Map

- `java/`: the Maven project (`io.github.thkprado:jlens`), the module
  `io.github.thkprado.jlens`, built with the Maven Wrapper (`mvnw`, `mvnw.cmd`; no JAR is
  committed, the scripts download Maven and check its SHA-256).
- `java/src/test/java/`: the JUnit suite, in the module's package.
- `java/native/<os>-<arch>/`: the native libraries the JAR carries (not committed), put there by
  `cargo xtask java` for the host, or by the JAR workflow for the seven release targets; the
  names are Rust's `std::env::consts::{OS, ARCH}`, with `-musl` after them on musl
  (`linux-x86_64-musl`), which the loader detects.

## Testing

`cargo xtask java` builds `jlens-capi` in release mode, copies the library into `java/native/`,
and runs `mvnw -B verify`; arguments replace `verify` (`cargo xtask java test -Dtest=OptionsTest`).
It needs a JDK 25 or later on `PATH` or in `JAVA_HOME`. CI runs it on Linux and Windows, on
Java 25 and on the latest release; on each push to `main`, the JAR workflow (`jar.yml`) runs the
suite against the release JAR on its seven targets, macOS and musl included.

The system property `jlens.library` points the package at a library outside the JAR.

## Rules

- **Java 25 is the floor.** Do not add code for older releases.
- **Every public item has Javadoc**, with `@param`, `@return` and `@throws`: the compiler runs
  with `-Xdoclint:all/protected -Xlint:all -Werror`.
- **Native objects are `AutoCloseable` with a `Cleaner` behind them.** The cleaning action never
  refers to the object it cleans (a `record` holding the handle). A method that passes a handle
  to the C API ends with `Reference.reachabilityFence(this)`, so that the Cleaner cannot free
  the handle during the call. Using a closed object throws `IllegalStateException`; `U2002`
  from the C API means an object was closed during the call, and becomes the same exception.
- **The restricted methods stay in `Native`**, the one class that suppresses the `restricted`
  warning (JEP 472).
- **Offsets in `JlensException` are UTF-16**, as the C API gives them: no conversion in Java.
- **Results are Java objects or JSON text** (ADR 0021, results of 0.25 #5). `evaluate` and
  `evaluateFromJson` decode the C API's values (`Values`): `LinkedHashMap`, `ArrayList`,
  `String`, `Long` for an integral number in a `long`'s range, `Double` otherwise, `Boolean`,
  and `null` for both undefined and `null`. `evaluateJson` returns JSON text: `null` for
  undefined, the text `"null"` for JSON's null. A decoder walks with its own stack, never by
  recursion, as results nest up to 1,000 levels.
- **Java objects as input** (`evaluate(Object)`, 0.25 #6) are read by `Reader`, called back by
  the C API on the evaluating thread, which finds it in a `ScopedValue`. An upcall must never
  throw: every failure becomes a level the evaluation fails with (`U1004`). A map or list of the
  input that the result holds unchanged comes back as itself (`INPUT` in `Values`).
- **Every call back into Java goes through `upcall` in `jlens-capi`**, which keeps 256 KiB of
  stack free for the JVM: with the 16 KiB pages of macOS on ARM, Java needs more below the caller
  than the core keeps, and a `StackOverflowError` that leaves an upcall aborts the JVM.
- **Host functions** (0.25 #7) are run by `Host`, called back on the evaluating thread, which
  finds the evaluation's `Host` in a `ScopedValue` and the function by its `Bindings`' context.
  The upcall never throws: an exception becomes an error reply with its number, and `evaluate`
  throws the same object again. A `JsonataFunction` works on its thread while its call runs.
- **Audits** (0.25 #8) are records, decoded from one reply of values; the input is written
  whole, as the core audits JSON values. An error, a host function's exception included (as the
  cause), is the result's, never thrown. Spans are UTF-16, converted by the C API. Trees nest as
  deep as the evaluation: build and walk them with a stack or a queue, never by recursion, and
  keep `toString` of a node to the node.
- **API changes** need the Javadoc, the JUnit suite and the Java chapter of the book
  (`docs/src/guides/java.md`). Its snippets live in `GuideTest.java`, between `ANCHOR`
  comments, so that the suite runs them.
- **The version** in `pom.xml` is the workspace's; `cargo xtask java` fails otherwise.
- **Releases go to Maven Central** from `release.yml`, with the profile `release` of `pom.xml`
  (sources, Javadoc, signatures, the upload; `CONTRIBUTING.md`). The JAR must stay
  reproducible, as the job compares it with the JAR the suite ran against: keep
  `project.build.outputTimestamp`, and put nothing that changes between builds into it.
