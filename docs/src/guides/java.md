# Java

The Java package `io.github.thkprado:jlens` runs the jlens engine on the JVM, through a C API
that Java calls with the Foreign Function and Memory API, Panama
([ADR 0021](../adr/0021-java-package.md)). It covers evaluation, host functions written in Java,
and audit trees with their explanations. Its API is the one of the [Python](python.md) and
[WebAssembly](wasm.md) packages, in Java's style: one immutable `Expression` evaluated from any
thread, options as a record with a builder, results as maps and lists, and offsets counted in
UTF-16 code units, the unit of a `String`.

```xml
<dependency>
  <groupId>io.github.thkprado</groupId>
  <artifactId>jlens</artifactId>
  <version>0.25.0</version>
</dependency>
```

It needs Java 25 or later. The package is in beta until jlens 1.0: its API may still change in a
minor release, as the [changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md) will
say. The [installation page](../getting-started/installation.md#java) lists the platforms its
JAR has native libraries for.

The package calls native code. Start the JVM with `--enable-native-access`, or Java prints a
warning at first use, and a later Java release will refuse the access
([JEP 472](https://openjdk.org/jeps/472)):

```sh
java --enable-native-access=io.github.thkprado.jlens -p app.jar:jlens.jar -m app   # on the module path
java --enable-native-access=ALL-UNNAMED -cp app.jar:jlens.jar app.Main            # on the class path
```

An application packaged as an executable JAR can put `Enable-Native-Access: ALL-UNNAMED` in its
manifest instead; a library's JAR cannot do it for the application.

The snippets in this chapter come from `GuideTest.java` in `crates/jlens-java/java/src/test/`,
which the package's JUnit suite runs.

## Compile once, evaluate many times

```java
{{#include ../../../crates/jlens-java/java/src/test/java/io/github/thkprado/jlens/GuideTest.java:quick_start}}
```

An `Expression` is compiled once and evaluated any number of times, from any number of threads,
virtual ones included. Compiling throws a `JlensException` on syntax errors.

```java
Expression expression = Expression.compile(source);   // compile once
expression.evaluate(input, bindings, options);        // Java objects in, Java objects out
expression.evaluateFromJson(text, bindings, options); // JSON text in, Java objects out
expression.evaluateJson(text, bindings, options);     // JSON text in, JSON text out
expression.evaluate(document, bindings, options);     // a Document, parsed once
expression.source();                                  // the source text
```

- **The input** is Java objects (`Map`, `List`, `String`, numbers, `Boolean`, `null`), JSON text
  (a `String` or UTF-8 `byte[]`), or a `Document`. See [how the input is read](#how-the-input-is-read).
- **The bindings** are variables and [host functions](#host-functions), made once:
  `Bindings.of(Map.of("limit", 10))` makes `$limit` available. Each method also comes without
  them, and without options.
- **The options** are a record: the [limits](#limits), and the clock and seed that make an
  evaluation reproducible.
- **The result** is built for each evaluation, and the caller may modify it: an object is a
  `LinkedHashMap`, its fields in their order; an array an `ArrayList`; a number a `Long` when it
  is integral and in a `long`'s range, a `Double` otherwise (`1.5`, `1e20`); and a `String`, a
  `Boolean` or `null`.

### Undefined and null

JSONata tells an undefined result (the expression produced nothing) from `null`. `evaluate` and
`evaluateFromJson` return `null` for both; `evaluateJson` keeps them apart, with `null` for an
undefined result and the text `"null"` for `null`:

```java
{{#include ../../../crates/jlens-java/java/src/test/java/io/github/thkprado/jlens/GuideTest.java:undefined}}
```

### Options

```java
{{#include ../../../crates/jlens-java/java/src/test/java/io/github/thkprado/jlens/GuideTest.java:options}}
```

- `timeout` (a `Duration`) stops the evaluation with `D1012`.
- `maxSteps` stops it with `U1006` once it has taken that many steps, the same on every machine;
  see [limits](#limits).
- `maxDepth`, `maxStringLength`, `maxArrayLength` and `maxResultSize` bound the evaluation; see
  [limits](#limits).
- `now` (an `Instant`) fixes `$now()` and `$millis()`.
- `seed` fixes `$random()` and `$shuffle()`.

A component left `null` keeps its default, and a negative one throws an
`IllegalArgumentException`. The package makes each distinct `Options` a native object once, so
an application that keeps a few of them pays nothing per evaluation for them.

## How the input is read

`evaluate` reads Java objects during the evaluation, one level at a time, when the expression
first reaches it, as the Python and WebAssembly packages do
([ADR 0010](../adr/0010-lazy-input.md)). A path such as `customer.name` reads one entry of two
maps, however large the rest of the input is, and an index (`items[0]`, `items[$i]`) one item of
a list.

| Java | JSONata |
|---|---|
| `Map` with `String` keys | object, in the map's order |
| `List` | array |
| `String` | string |
| `Integer`, `Long`, `Short`, `Byte`, `Double`, `Float`, `BigInteger`, `BigDecimal` | number, a `double`; NaN and the infinities become `null`, as in JSON |
| `Boolean` | boolean |
| `null` | `null` |

Anything else, such as a `Date`, a key that is not a `String`, or a map or list inside itself,
has no JSON form, and a map or list nested more than 1,000 levels deep is not read, as JSON text
that deep is not parsed. Two consequences follow from reading the input as the expression goes:

- **Errors appear when their part is read.** A value that cannot be read stops the evaluation
  with `U1004` when the expression reads it; one it never reaches does no harm.
- **The input must not change during the evaluation.** A [host function](#host-functions), or
  another thread, that modifies it can change what the rest of the evaluation reads. Copy the
  input first if something must modify it.

```java
{{#include ../../../crates/jlens-java/java/src/test/java/io/github/thkprado/jlens/GuideTest.java:input}}
```

Evaluations never modify their input. A map or list of the input that the result holds unchanged
comes back as itself, not as a copy, as `$` returns the input: modifying it modifies the input.

### JSON text

`evaluateJson` takes the input as JSON text and returns the result as JSON text, and
`evaluateFromJson` returns Java objects from JSON text. For a document you already have as text,
such as an HTTP response body, they avoid building Java objects only to read them. While they
parse, they skip the fields the expression does not [read](#the-fields-an-expression-reads),
checking them as JSON syntax only:

```java
{{#include ../../../crates/jlens-java/java/src/test/java/io/github/thkprado/jlens/GuideTest.java:json_text}}
```

Invalid JSON, or a `byte[]` that is not UTF-8, throws a `JlensException` with code `U2001`.

### Reusing a document

To evaluate several expressions against one document (a set of rules, say), or one expression
several times, parse it once:

```java
{{#include ../../../crates/jlens-java/java/src/test/java/io/github/thkprado/jlens/GuideTest.java:document}}
```

A `Document` is JSON text parsed once, kept in native memory. An evaluation against it reads no
Java objects, so it is the fastest input: `items[0].name` takes under a microsecond on any size of
document ([Performance](../architecture/performance.md#java-against-the-other-implementations)).
Evaluations never modify a document.

## Native objects

`Expression`, `Document` and `Bindings` hold native memory. They are `AutoCloseable`: close them
when done, with `try`-with-resources or when the application stops. One never closed is freed
once it becomes unreachable, by a `Cleaner`, so forgetting to close costs memory for a while,
never a crash. Using a closed object throws an `IllegalStateException`.

The JAR carries the native library of each platform it supports. At first use, the package copies
the library of the running platform to a directory named after its content in `java.io.tmpdir`,
private to the user, and loads it from there. It refuses a directory that another user could
write to. Set the system property `jlens.library` to the path of the library to load it from
elsewhere, such as where the temporary directory is mounted `noexec`.

## Threads

`Expression`, `Document` and `Bindings` objects can be shared between threads, virtual threads
included:

```java
{{#include ../../../crates/jlens-java/java/src/test/java/io/github/thkprado/jlens/GuideTest.java:threads}}
```

An evaluation runs on the calling thread, the carrier thread for a virtual thread, and
evaluations of one expression on several threads proceed in parallel. A virtual thread stays on
its carrier during an evaluation, as during any native call.

A `Document` used by several threads is converted once more on each operating system thread,
the first time that thread evaluates against it, and a thread keeps its conversion until the
document is closed. After that, each thread drops its conversion at its next call into jlens: a
carrier thread that never evaluates again keeps at most one conversion per document closed.

The evaluation grows its stack as it needs, so threads with a small stack (`-Xss`, or a
platform thread's `stackSize`) work: an evaluation nested too deep stops with `D1011` or
`U1001`, never with a crash.

## Host functions

Java functions in the bindings are called from the expression like built-in functions. Each has
an arity or a signature:

```java
{{#include ../../../crates/jlens-java/java/src/test/java/io/github/thkprado/jlens/GuideTest.java:host_functions}}
```

- **Arguments and the result** cross as values: the arguments are the Java objects of a result
  (`LinkedHashMap`, `ArrayList`, `String`, `Long`, `Double`, `Boolean`, `null`), and the result
  is read as the input is, whole, when the function returns. `null` stands for undefined, in both
  directions. A function argument arrives as a `JsonataFunction`
  ([below](#functions-as-arguments)). A result with no JSON form throws an
  `IllegalArgumentException`.
- **A signature** checks the arguments before each call, with the errors of the built-in
  functions (`T0410`, `T0412`). A parameter with `-` receives the context value when the argument
  is missing: `<n-:n>` makes `price.$double()` work. Without a signature, the arity tells
  higher-order functions how many arguments to pass: `$map` passes the index only to functions
  of two or more. An invalid signature throws a `JlensException`.
- **Asynchronous functions are not supported.** A function that returns a `CompletionStage` or
  a `Future` fails with `U1002`. Fetch what the expression needs before evaluating it.
- **Precedence:** a variable and a function cannot share a name in one `Bindings`, as binding one
  replaces the other; both replace a built-in function of the same name. There is no
  `registerFunction`: `Bindings` are made once and serve every expression.

A host function bound but not called costs nothing: the evaluation looks it up when a name is
not found in its own frames ([ADR 0006](../adr/0006-host-functions.md#amendment-host-functions-looked-up-on-a-miss-025)).

### Exceptions

An exception thrown by a host function is thrown again by the evaluation, the same object,
unchanged:

```java
{{#include ../../../crates/jlens-java/java/src/test/java/io/github/thkprado/jlens/GuideTest.java:host_exceptions}}
```

`evaluateWithAudit` reports it in its `error`, a `JlensException` whose cause is the exception.
Inside `$eval`, it becomes the error of `$eval` (`D3121`).

### Functions as arguments

A host function can call a JSONata function it receives, such as a step to retry or a fallback:
a lambda, a built-in function, a partial application, a chain or another host function. It
arrives as a `JsonataFunction`:

```java
{{#include ../../../crates/jlens-java/java/src/test/java/io/github/thkprado/jlens/GuideTest.java:functions_as_arguments}}
```

- **The call is part of the evaluation**, as when `$map` calls its function. The context value
  is `null`, and a lambda keeps the variables of its definition. The call counts as a level of
  depth, so a recursion through a host function ends with `D1011`; it checks the timeout; its
  result is limited by `maxResultSize`.
- **Arguments and the result** are converted as for a host function; `arity()` is the number of
  parameters the function declares.
- **Errors are thrown** as a `JlensException` with their codes, or as the exception a host
  function threw. The host function can catch them, or let them through, which ends the
  evaluation with that error.
- **It is valid until the host function returns, on its thread.** Called later, or from another
  thread, it throws a `JlensException` with code `U1005`.

## Errors

Errors are `JlensException`s, an unchecked exception. They carry the JSONata error fields; every
offset counts UTF-16 code units, the unit of `String`, so `source.substring(start, end)` is the
part of the source the error is about:

```java
{{#include ../../../crates/jlens-java/java/src/test/java/io/github/thkprado/jlens/GuideTest.java:errors}}
```

Evaluation errors carry the value too, as JSON text, when there is one (for `T2001`, the value
that is not a number):

```java
{{#include ../../../crates/jlens-java/java/src/test/java/io/github/thkprado/jlens/GuideTest.java:error_value}}
```

`getMessage()` is the message. Match on `code()`, never on the message. The
[error reference](../language/errors.md) lists every code; the package's own are `U2001` (input
that is not JSON), `U2004` (a failure inside the native library, which never ends the JVM) and
`U2005` (options or reads not in their form).

## The fields an expression reads

`reads()` tells which fields of its input and of each binding an expression may read, without
evaluating it ([ADR 0016](../adr/0016-fields-read.md)): an application can fetch only those from
a database, or know which rules depend on a field. `Reads.union` combines the answers of several
expressions.

```java
{{#include ../../../crates/jlens-java/java/src/test/java/io/github/thkprado/jlens/GuideTest.java:reads}}
```

- **The answer is an upper bound.** A `Read` is `Read.WHOLE`, the value whole, or `Read.Fields`,
  only the fields it lists, of an object, or of each item of an array; empty fields need the value
  but none of its fields, as `$count` does. On its input and bindings cut down that way, an
  expression gives the same result, or fails with the same error code. Where the analysis cannot
  follow a value, under `*` or `**`, in a constructed object or in the result, it reads the value
  whole. `input()` is `null` when the expression does not read its input.
- **A host function** does not receive the context of its call, so a call to one reads only its
  arguments, and the context a `-` parameter may take.
- **`everyBinding()`** says the expression may read any binding whole, as `$eval` can: keep them
  all. `toJson()` gives the JSON form, which the other packages and the CLI share.

`Document.fromJson(text, read)` parses JSON text cut by a `Read`. It skips the fields the `Read`
leaves out, and the document serves each expression that went into the union as the whole one
would:

```java
{{#include ../../../crates/jlens-java/java/src/test/java/io/github/thkprado/jlens/GuideTest.java:document_read}}
```

The fields skipped are checked as JSON syntax only: a number too large for a `double`, which a
full parse rejects, passes there.

## Explaining results

`evaluateWithAudit` evaluates and records how the result was reached; see the
[audit guide](audit.md). Evaluation errors are reported in the result, not thrown:

```java
{{#include ../../../crates/jlens-java/java/src/test/java/io/github/thkprado/jlens/GuideTest.java:audit}}
```

It takes Java objects, with `Bindings`, `Options` and `AuditOptions`: a level (`OFF`,
`DECISIONS`, the default, or `FULL`) and the `CaptureLimits` (`maxItems`, `maxString`,
`maxDepth`, `maxRejected`, `maxNodes` and `maxSource`). The input is converted whole first, so a
`Document` is refused. The result is an `AuditResult` record:

- `result()` and `error()`, a `JlensException` or `null`;
- `audit()`, the `AuditTree`: its `header()` (the level, the result, when it ran and for how
  long, the steps taken), its `root()` node, `nodeCount()` and `truncated()`. An `AuditNode` is a record: what it
  evaluated (`kind`, `source`, `span` in UTF-16 offsets, `label`), its `input`, `output` and
  `outcome`, the filters it ran, its children, and what it decided as an `AuditNode.Decision`, a
  sealed interface: a `Comparison` and its operands, a `ShortCircuit`, a `Conditional`'s branch or
  a `Binding`;
- `explanation()`, whose `text()` is the text of `jlens eval --explain`, and whose `lines()` are
  structured lines (`depth`, `passed`, `text`, `message`, `node`) for a user interface.

A tree nests as deep as the evaluation did: walk it with a stack or a queue, not by recursion.

## Limits

| Limit | Default | Error |
|---|---|---|
| Evaluation depth (`maxDepth`) | 50,000 | `D1011` |
| Evaluation time (`timeout`) | none | `D1012` |
| Evaluation steps (`maxSteps`) | none | `U1006` |
| Length of a string built (`maxStringLength`, bytes of UTF-8) | 256 MiB | `U1001` |
| Items of an array built (`maxArrayLength`) | 16,777,216 | `U1001` |
| Size of the result as JSON text (`maxResultSize`, bytes) | 256 MiB | `U1001` |
| Expression nesting | 1,000 levels | `U1001` |
| Nesting of arrays and objects in a value that is read, returned, compared or serialized | 1,000 levels | `U1001` |

The size of the result counts a value each time it appears, as the JSON text does, and also
limits the arguments passed to a host function. The stack grows as needed, so the depth limit is
that of the native targets, on any thread; see the
[evaluator chapter](../architecture/evaluator.md#guardrails). There is no cancellation from
another thread, and interrupting the thread does not stop an evaluation: use `timeout` or
`maxSteps`.

A timeout reads the clock, so the same evaluation can pass on one machine and fail on another.
`maxSteps` does not depend on the machine: each node evaluated is a step, and so is each function
a built-in or a host function calls back, and each 1,024 steps of a regular expression's
backtracking. The same expression, input and options take the same steps on every machine,
whichever `evaluate` method reads the input, and fail with `U1006` once the budget is spent; the
error's value is the limit. The header of an audit reports the steps an evaluation took
(`audit().header().steps()`), to set a budget from. See [ADR 0018](../adr/0018-step-limit.md). The
[performance chapter](../architecture/performance.md#java-against-the-other-implementations)
compares the package's speed and its behaviour at these limits with the other JSONata
implementations for the JVM.

## What the package leaves out

The language service and schema inference are not in the Java package: editors reach them
through the [language server](editors.md), and `jlens schema infer` infers a schema from the
command line. They can be added when a Java application asks for them.

## Building from source

From a checkout, with Rust and a JDK 25 or later:

```sh
cargo xtask java          # builds the native library, then runs mvnw verify in crates/jlens-java/java
```

The JAR in `crates/jlens-java/java/target` then carries the library of the host platform only.
