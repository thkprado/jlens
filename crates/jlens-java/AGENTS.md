# AGENTS.md — jlens-java

Guidance for working in the Java package. The repository-wide rules are in the root
[AGENTS.md](../../AGENTS.md); the decisions are in ADR 0021.

## Map

- `java/`: the Maven project (`io.github.thkprado:jlens`), the module
  `io.github.thkprado.jlens`, built with the Maven Wrapper (`mvnw`, `mvnw.cmd`; no JAR is
  committed, the scripts download Maven and check its SHA-256).
- `java/src/main/java/io/github/thkprado/jlens/`: the public classes (`Expression`, `Document`,
  `Options`, `Read`, `Reads`, `JlensException`), `Native` (the C API's downcalls, the loading of
  the library), `Values` (the decoder of results as Java objects) and `Json` (the JSON the
  package reads and writes itself).
- `java/src/test/java/`: the JUnit suite, in the module's package.
- `java/native/<os>-<arch>/`: the native libraries the JAR carries (not committed), put there by
  `cargo xtask java` for the host; the names are Rust's `std::env::consts::{OS, ARCH}`.

## Testing

`cargo xtask java` builds `jlens-capi` in release mode, copies the library into `java/native/`,
and runs `mvnw -B verify`; arguments replace `verify` (`cargo xtask java test -Dtest=OptionsTest`).
It needs a JDK 25 or later on `PATH` or in `JAVA_HOME`. CI runs it on Linux and Windows, on
Java 25 and on the latest release.

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
- **API changes** need the Javadoc, the JUnit suite and, from 0.25 #11, the Java chapter of the
  book.
- **The version** in `pom.xml` is the workspace's; `cargo xtask java` fails otherwise.
