# 0021 — A Java package: a C API called through Panama, or run on Chicory

**Status:** Proposed. A spike decides between the options below; the measurements it needs
are listed at the end.

## Context

JSONata on the JVM has two implementations: jsonata-java (dashjoin), a port of jsonata-js in
pure Java, and JSONata4Java (IBM), an implementation on ANTLR. jlens would bring its
conformance, audits, schemas and speed to Java, where rules engines and integration services
live. As with Python (ADR 0014), the package would be bindings over `jlens-core`, not a second
implementation: one engine, one conformance baseline, one changelog.

**The oldest Java served is 25**, the current long-term release. Java 21 is left out. The
package has no users to keep yet, and 25 gives it the Foreign Function and Memory API (Panama,
JEP 454, final since Java 22) with no preview flag. Three ways reach the JVM without rewriting
the engine:

- **Panama**: `jlens-core` compiled as a native library per platform, with a C API
  (`extern "C"` functions), which Java calls through `Linker` and `MethodHandle`s.
- **Chicory**: `jlens-core` compiled to WebAssembly and run by Chicory (1.7.5, 2026-03-24), a
  WebAssembly runtime in pure Java (Java 11 and later, no native dependency). Chicory has an
  interpreter, a compiler to JVM bytecode at run time, and one at build time
  (`chicory-compiler-maven-plugin`) that ships the module as `.class` files.
- **JNI**: the native library called through the Java Native Interface, with the `jni` crate
  (0.22.4, 2026-03-16).

**Panama and Chicory can share their Rust side.** One C API crate compiles natively for
Panama and to `wasm32-wasip1` for Chicory. The exported functions are the same, and so is the
way bytes cross the boundary (below). Each Java backend is then a thin layer that moves bytes,
either through a `MemorySegment` or through Chicory's `Memory`. Both sit behind one Java API.
JNI needs glue of its own on both sides: Rust functions that receive a `JNIEnv`, and Java
`native` methods. It cannot share that glue with a WebAssembly build. Its advantage was serving
Java 8 to 21, which the floor of 25 no longer needs. So JNI is weighed only as the fallback if
Panama fails the spike.

All options share the Java API. It mirrors the WebAssembly and Python packages: `Expression`,
`Document`, the options of `EvaluateOptions`, host functions, audits, and a `JlensException`
that carries the JSONata `code`, `position`, `start`, `end`, `token` and `value`. Java strings
are UTF-16, so error offsets mean what they mean in jsonata-js, with no conversion for the
user. Records and sealed interfaces type the results: an `AuditNode` is a sealed hierarchy,
and the options are a record with a builder.

### The C API and `unsafe_code = "forbid"`

The exports take `#[unsafe(no_mangle)]`, which the workspace lint refuses. No generator writes
them for us, as wasm-bindgen and PyO3 do. Two ways out:

- A crate-level exception in the C API crate only, for that attribute. It is recorded here,
  and a test checks that the crate has no `unsafe` block.
- A small proc-macro crate that writes the exports. ADR 0014's spike showed that code expanded
  from another crate's macro does not trip the lint. That hides the exception rather than
  states it, so the first way is preferred.

The rest of the C API needs no `unsafe`. Rust never dereferences a pointer it is given:

- **Buffers are Rust's.** Java asks for a buffer of `n` bytes and gets a handle and the
  buffer's address, as integers. It writes the input there: through
  `MemorySegment.ofAddress(address).reinterpret(n)` on the native build, and through Chicory's
  `Memory` at that offset on the WebAssembly build. Rust then reads its own `Vec<u8>`, found by
  the handle. Results come back the same way.
- **Host functions** are registered once as function pointers (Panama upcall stubs, or Chicory
  host functions imported by the module), typed `extern "C" fn` in Rust.

### The boundary

**Panama** calls the engine directly. An input crosses as UTF-8 JSON text, written into a
Rust buffer as above; Jackson or `toString` writes it on the Java side. Reading Java objects
one level at a time through `Source` (ADR 0010) is possible with upcalls, one per level read.
Its cost against the JSON text is the spike's first question, since ADR 0014 found reading
levels 2 to 17 times faster in Python when the expression reads part of the document.

**Chicory** sees only the module's linear memory, so its input is JSON text, copied into that
memory. A `Source` would need one imported call per level read, from WebAssembly into Java.

**On both**, a `Document` parsed once avoids that cost for an input evaluated many times.

### Threads

`jlens_core::Expression` is `Send` and `Sync`. `jlens_core::Document` holds `Rc`s and is
neither. Java servers share objects between threads as a matter of course, and on 25 they
run requests on virtual threads.

**Panama** can reuse ADR 0014's design. The Java `Document` holds the parsed JSON behind an
`Arc`, each OS thread keeps its converted `jlens_core::Document` in a thread-local cache, and
one `Expression` serves every thread. A virtual thread in a downcall is pinned to its carrier
(the OS thread under it) until the call returns. So the cache holds one copy per carrier, a
small pool, not one per virtual thread. A host function written in Java runs while its virtual
thread is pinned: if it blocks on I/O, it holds its carrier for that time, which the Java
chapter would say.

**Chicory** gives each WebAssembly instance one linear memory, which only one thread may use at
a time. The module is Java code, so nothing is pinned. A pool lends an instance for each
evaluation, and each instance parses its own copy of the expression and of each `Document`.
Memory grows with the pool. Whether an instance can be cloned from an initialized one, instead
of being instantiated again, is a spike question.

### Limits and robustness

**Panama** runs on the calling thread's native stack, the carrier's for a virtual thread.
`stack::grow` (stacker) extends that stack as it does natively, so the native
`DEFAULT_MAX_DEPTH` of 50,000 applies. A thread created with a small `-Xss` is a case for the
robustness tests, as musl threads were (see the musl note of the Python chapter).

**Chicory** runs the module's frames on the JVM's stack in compiled mode, and jlens cannot
measure that stack. This is the situation of the WebAssembly package in V8, where
`DEFAULT_MAX_DEPTH` is 500 for that reason. The limit for the JVM would be measured, and a
`StackOverflowError` in the middle of an evaluation leaves the instance's state (its stack
pointer, its allocator) undefined: the instance is dropped. Linear memory caps a module at
4 GiB (`wasm32`), which bounds the size of a document.

**Timeouts** read the clock (`web_time`). On `wasm32-unknown-unknown` that crate calls
JavaScript's `performance.now()` through wasm-bindgen, which Chicory cannot provide. The
Chicory build targets `wasm32-wasip1`, whose `std::time` reads the clock through WASI, and
Chicory implements WASI preview 1. Whether rustc links that target with `--stack-first`, as
`stack::exhausted` assumes, is a spike question.

### Native access (JEP 472)

On Java 24 and later, Panama's restricted methods (`Linker.downcallHandle`,
`MemorySegment.reinterpret`) and `System.load` print a warning on standard error, once per
module, unless the application is started with `--enable-native-access=<module>`. That is
`io.github.thkprado.jlens` when the JAR is a named module, and `ALL-UNNAMED` on the class path.
A later release will refuse that access by default (`IllegalCallerException`). The
`Enable-Native-Access` manifest attribute only takes effect in the JAR launched with
`java -jar`, not in a library's JAR. A user of the native package therefore adds the flag to
their launch command. A user of Chicory does not.

### Distribution

**Panama** (as JNI would) ships the native library of every release target inside the JAR:
Linux x86-64 and ARM64 (glibc and musl), macOS x86-64 and ARM64, and Windows x86-64, the
targets of the Python wheels, whose CI matrix can be reused. At its first load the library is
extracted to a temporary directory and loaded through `SymbolLookup.libraryLookup`. That makes
one JAR of several megabytes, or one JAR per platform with a Maven classifier. Other platforms
have no binary: Linux on s390x and ppc64le, common in the companies that run Java on IBM
hardware.

**Chicory** ships one JAR for every platform the JVM runs on, with the module compiled to
bytecode at build time. It needs no temporary file, which also suits restricted environments
(a read-only `/tmp`, `noexec` mounts). Chicory's documentation says the build-time compiler
needs no reflection and so works more easily with `native-image`. The evaluator's functions may
exceed the JVM's 64 KiB limit on a method's bytecode. The compiler can then leave those
functions to the interpreter (`interpreterFallback`), which is slower.

**Both** publish to Maven Central through the Central Portal. That requires a verified
namespace (`io.github.thkprado`, verified through the GitHub account), GPG signatures, and
sources and Javadoc JARs. The Java code is built by Maven, in `crates/jlens-java/java/`.

### Performance

The point of jlens on the JVM is to be faster than jsonata-java while being more conformant.
Panama keeps native speed and pays at the boundary. Chicory pays at the boundary (JSON text
both ways) and in execution: its documentation lists being "the fastest runtime" among its
non-goals. Neither cost is known for jlens, so the spike measures both against jsonata-java on
the workload of `benches/compare`.

## Decision (proposed)

Pending the spike:

- **The package requires Java 25**, and CI tests it on 25 and on each later release.
- **One C API crate** (`crates/jlens-capi`, not published to crates.io) holds the exports, with
  buffers and handles owned by Rust, and the crate-level exception for `no_mangle` recorded
  above.
- **The Java package calls it through Panama**, if the measurements show the boundary costs
  less than jsonata-java's evaluation on the common cases. Native speed, one `Expression` for
  every thread, and no glue code beyond the C API are the reasons. Those are what users of a
  rules engine on a server would choose jlens for. The install section documents
  `--enable-native-access`, with the warning users see without it.
- **A Chicory backend is cheap to add later**, since it compiles the same crate to
  `wasm32-wasip1` behind the same Java API. It waits until users ask for a platform without a
  binary or for a JAR with no native code. It then costs a second artifact, its test matrix and
  the stack-depth tuning above.
- **Chicory becomes the only backend** if the native-access flag or the native libraries are a
  blocker for the intended users, *and* Chicory's compiled module is still faster than
  jsonata-java.
- **JNI is the fallback** only if Panama's downcalls or upcalls cost too much in the spike.
- **The Java API is frozen with the other contracts outside Rust** (ADR 0015), so the package
  starts after the API stability work of roadmap 0.24, not before.

## Consequences

- Java 21 users are not served. If demand appears, the C API cannot reach them through Panama;
  they would need the Chicory backend (Java 11 and later) or JNI.
- With Panama: a JAR of a few megabytes with native code for seven targets, a Java test suite
  that CI runs on each target, and a launch flag for every user.
- With Chicory: one portable JAR, slower evaluations by an amount the spike measures, a
  `wasm32-wasip1` build of `jlens-core` that CI tests, and a pool of instances.
- The C API could later serve other languages that call C (.NET, Go, Ruby). That is not a goal
  of this ADR, but the crate is kept free of Java specifics with that in mind.
- Either way, the book gains a Java chapter beside the WebAssembly and Python ones, and the
  release checklist gains a Maven Central step, which, like `cargo publish`, cannot be undone.

## The spike

To be run before this ADR is accepted, on Windows x86-64 and Linux x86-64, with Java 25 and
the release build:

1. A C API crate with one export, a crate-level exception for `no_mangle`, and a check that an
   `unsafe` block in the crate is still refused. Buffers by handle, read from Java through
   `MemorySegment` and through Chicory's `Memory`.
2. Microseconds per evaluation, best of five, on the documents of ADR 0014's table (1,000 and
   100,000 items). Columns: Panama on JSON text; Panama with a `Source` over `Map`/`List`
   through upcalls; Panama on a `Document`; Chicory (build-time compiler) on JSON text;
   Chicory on a `Document`; jsonata-java.
3. How many of jlens's functions the build-time compiler leaves to the interpreter, and the
   size of the compiled JAR.
4. The depth at which Chicory overflows the default JVM stack, with and without an audit, to
   set its `DEFAULT_MAX_DEPTH`. Also: does the instance stay usable after a
   `StackOverflowError`?
5. Whether `wasm32-wasip1` links with `--stack-first`, and whether a timeout fires through
   Chicory's WASI clock.
6. The cost of a new Chicory instance, and of a Panama upcall per level read, to size the pool
   and the `Source`.
7. Throughput with virtual threads: many concurrent evaluations of one `Expression`, Panama
   (pinned carriers) against a Chicory pool, and the warning Java 25 prints without
   `--enable-native-access`.
