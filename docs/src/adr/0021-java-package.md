# 0021 — The Java package: a C API called through Panama

**Status:** Accepted (2026-10-25), after the spike at the end. The context below is as it was
proposed; the spike's results led to the decision.

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

- An exception in the C API crate only. `forbid` cannot be lowered by an inner `allow`, so the
  crate declares its own lints instead of `workspace = true`: the workspace's, with
  `unsafe_code = "deny"`. The exports module alone carries `#[allow(unsafe_code)]`. That
  `allow` covers the bodies of the module's functions too, so a `macro_rules!` writes each
  export as one call to a safe function, and a test checks that `unsafe` appears nowhere in
  the crate's code but in that attribute. The spike measured both halves (below).
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

## Decision

- **The package requires Java 25**, and CI tests it on 25 and on each later release.
- **Panama is the only backend.** Through Panama, a `Document` beats jsonata-java on its own
  objects by 2.6 to 70 times, and JSON text beats jsonata-java parsing the same text by 1.2 to
  2.7 times (item 2). Concurrent evaluations scale to 150,000 to 205,000 a second on 16
  hardware threads, or on 1,000 virtual threads, 1.8 to 2.5 times jsonata-java (item 7). A
  downcall costs 9 ns and an upcall 24 to 28 ns (item 6), so JNI has nothing to win.
- **One C API crate, `crates/jlens-capi`**, not published to crates.io, holds the exports.
  - **The lint.** It declares the workspace's lints itself, with `unsafe_code = "deny"`. Only
    its exports module allows `unsafe_code`, for `#[unsafe(no_mangle)]`, and a `macro_rules!`
    writes each export as one call to a safe function. A test fails if `unsafe` appears
    anywhere else in the crate's Rust files, all of them, not only `src/` (item 1).
  - **Buffers.** They are owned by Rust and known by handle; Java writes and reads them
    through the address Rust exposes. No Rust code dereferences a pointer it is given.
  - **The tables of handles** (buffers, expressions) are global, since a virtual thread can
    change carriers between two calls. Their locks are held only to find an entry, and each
    buffer has a lock of its own. A table locked across the work stopped all scaling in
    item 7.
  - **Documents** follow ADR 0014: the parsed JSON behind an `Arc`, converted once per OS
    thread into a thread-local cache.
  - **mimalloc** is the library's allocator, as in the Python wheels. The large cases ran up
    to 2.7 times slower on Windows than on Linux, which points at Windows' system allocator
    (item 2).
- **Inputs.**
  - JSON text (a `String`, or UTF-8 `byte[]`) is the default, parsed only for the fields the
    expression reads (ADR 0016).
  - A `Document` is for an input evaluated many times.
  - Java objects (`Map`, `List`, strings, numbers, booleans, `null`) are accepted too,
    through a `Source` read with one upcall per level. It is documented as the slower way
    when an expression reads most of its input (item 2). Its Rust side takes the Python
    package's levers, interned keys and one `Sources` per level, since the upcall is 2% of a
    level and Rust's allocations most of the rest (item 6).
- **Results.** They come back as JSON text, or as Java objects. Whether those objects are
  built by the result builder (ADR 0017) through upcalls, or decoded from the JSON text in
  Java, is measured when the package is built. `$` on a large document is the case to watch:
  it is the one case where jsonata-java won in item 2, because it returns its input.
- **Distribution.** One JAR carries the native libraries of the seven release targets, about
  1 MB each compressed, about 7 MB in all (item 3). It extracts the right one at first use.
  Per-platform JARs with a Maven classifier can follow if the size matters to users. The
  install section documents `--enable-native-access` and the warning users see without it.
- **Limits.** The native default depth of 50,000 holds on every kind of thread (item 4). A
  robustness test runs evaluations on a thread with a small `-Xss`.
- **Chicory is not shipped.**
  - It fails the condition for a sole backend: even with `-XX:-DontCompileHugeMethods`, it
    stays 3.6 to 4.8 times below jsonata-java (item 7).
  - It needs that JVM flag, so it no longer spares users a launch flag.
  - Each instance holds about 5 MiB (item 6), and its default depth would be about 200
    (item 4).
  - It can be weighed again if users ask for a JAR with no native code or for a platform
    without a binary, or if Chicory's speed changes. The same C API crate compiles to
    `wasm32-wasip1` unchanged (item 1). Items 3 to 6 say what such a backend needs: a module
    built with `opt-level = "s"`, a depth limit near 200 on default stacks, and dropping an
    instance after an overflow.
- **JNI is not used.**
- **The Java API is frozen with the other contracts outside Rust** (ADR 0015), so the package
  starts after the API stability work of roadmap 0.26, not before. (Superseded by the
  amendment below: the package ships before that work, in beta.)

## Consequences

- Java 21 users are not served. If demand appears, Panama cannot reach them. A Chicory backend
  (Java 11 and later) or JNI would.
- **What users get.**
  - A JAR of about 7 MB, with native code for seven targets.
  - A launch flag, `--enable-native-access`: a later Java release will refuse native access
    without it.
  - No binary for Linux on s390x or ppc64le.
- **What the project takes on.**
  - CI builds the library for each target and runs a Java test suite on each.
  - The release checklist gains a Maven Central step, which, like `cargo publish`, cannot be
    undone.
  - The book gains a Java chapter beside the WebAssembly and Python ones.
- **Beyond Java.** The C API could later serve other languages that call C (.NET, Go, Ruby).
  That is not a goal of this ADR, but the crate is kept free of Java specifics with that in
  mind.
- **Two costs in `jlens-core` showed up.** They are for the core, outside this ADR, and they
  apply to the Python package too.
  - On a `Document` of 100,000 items, the path `items[0]` builds the whole sequence of items
    before taking the first: 0.9 to 2.4 ms.
  - The `Sink` allocates once for each key, each string and each nested source (keys are
    interned: see the [amendment](#amendment-what-the-sink-allocates-024)).
- The spike's code is kept outside the repository. Its results are below.

## The spike

Run on 2026-10-25, before this ADR was accepted, on Windows x86-64 and Linux x86-64 (WSL 2),
with Java 25 and the release build:

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

### Results

**Item 1** (2026-10-25; Rust 1.99.1, Chicory 1.7.5 with its interpreter; Windows x86-64 with
Temurin 25.0.4, and Linux x86-64 (Ubuntu 26.04 on WSL 2) with Ubuntu's OpenJDK 25.0.4, with the
same results on both). The crate had five exports: `jlens_buffer_new`,
`jlens_buffer_address`, `jlens_buffer_len`, `jlens_buffer_free`, and `jlens_evaluate`, which
takes the handles of an expression and an input in JSON and returns a buffer with
`{"result": ...}` or `{"error": {"code", "position", "message"}}`.

- **The lint.** With the crate's own lints (`unsafe_code = "deny"`) and `#[allow(unsafe_code)]`
  on the exports module, `cargo clippy --all-targets` reports no warning.
  - An `unsafe {}` block elsewhere in the crate fails (`error: usage of an unsafe block`).
  - The same block inside the exports module compiles, as expected.
  - The source test catches the second case. It must scan every Rust file of the crate, not
    only `src/`: a `#[path]` module outside `src/` escaped a scan of `src/` alone.
- **Buffers by handle.** The table is global behind a `Mutex`, not thread-local, since a
  virtual thread can change carriers between two calls. A buffer is a `Vec<u8>` that is never
  resized, so its address stays valid until it is freed. The address is handed out with
  `expose_provenance`, since foreign code writes through it. The crate has no other `unsafe`.
- **Panama.** Java wrote the inputs with `MemorySegment.ofAddress(address).reinterpret(len)`
  and read the results the same way, through `invokeExact` downcalls. The results were right,
  including `é`, `☕` and `😀` in both directions, and an error envelope with `S0207`. 1,000
  virtual threads each made their own evaluation, sharing the table, with no wrong result.
  Without `--enable-native-access` Java 25 printed four lines, starting with `WARNING: A
  restricted method in java.lang.foreign.SymbolLookup has been called`, and ran on.
- **Chicory.** The same crate built for `wasm32-wasip1` with no change. The module exports
  `memory` and the five functions, with no `_initialize`. It imports six WASI functions from
  Rust's standard library: `random_get`, `environ_get`, `environ_sizes_get`,
  `clock_time_get`, `fd_write` and `proc_exit`. Chicory's `WasiPreview1` provides them.
  Through `Memory.write` and `readBytes`, the same cases gave the same results, `$millis()`
  read the clock through WASI, and 1,000 evaluations in a row gave no wrong result.
- **Sizes**, in a release build with `lto`, unstripped and not tuned: 2.2 MB for the Windows
  DLL, 2.6 MB for the Linux `.so`, 1.7 MB for the WebAssembly module.

**Item 2** (2026-10-25, the same machine and versions; jsonata-java 0.9.10; the system
allocator, not mimalloc).

- **Method.** One thread, one second of warm-up, then the best of five runs of at least 0.3 s
  each. The document has `n` items with five fields each: `id`, `name` (`"item 17"`), `price`
  (0 to 99.99), `tags` (two strings) and `stock` (an object with two fields). That makes 97 KB
  of JSON for 1,000 items and 10.1 MB for 100,000.
- **What each column includes.**
  - Every jlens column returns its result as JSON text, decoded into a Java `String`.
    jsonata-java returns Java objects.
  - "JSON text" starts from a Java `String`: it encodes it in UTF-8, copies it into a Rust
    buffer, and parses only the fields the expression reads (ADR 0016).
  - "Source" reads the `Map`s and `List`s that jsonata-java's own parser (`Json.parseJson`)
    built. It makes one upcall per level, and Java encodes the level into a Rust buffer.
  - "Document" is parsed once, before the timing.
  - "jsonata-java, parsing" parses the `String` and then evaluates.
- **Chicory** runs the module that its build-time compiler turned into JVM classes. Every
  function compiled: none was left to the interpreter. The generated files take 2.4 MB, 1.7 MB
  of it in one class. With HotSpot's defaults, its methods exceed the 8,000 bytes of bytecode
  above which HotSpot does not JIT-compile (`DontCompileHugeMethods`), so they run in the
  JVM's own interpreter. The table gives Chicory with `-XX:-DontCompileHugeMethods`. Without
  that flag, JSON text takes 13 to 25 ms on 1,000 items (4 to 5.5 times longer, but for `$`)
  and 1.8 to 2.0 s on 100,000, and a `Document` up to 2.5 times longer (2.9 on Linux).

Microseconds per evaluation on Windows x86-64:

| Expression | Panama, JSON text | Panama, Source | Panama, Document | Chicory, JSON text | Chicory, Document | jsonata-java | jsonata-java, parsing |
|---|---:|---:|---:|---:|---:|---:|---:|
| 1,000 items (97 KB) | | | | | | | |
| `items[0].name` | 529 | 120 | 4.8 | 3,535 | 42 | 78 | 759 |
| `$count(items)` | 277 | 113 | 1.2 | 2,391 | 5.9 | 80 | 756 |
| `$sum(items.price)` | 482 | 1,490 | 34 | 4,106 | 606 | 146 | 820 |
| `items[price > 98].id` | 633 | 1,601 | 127 | 6,095 | 1,085 | 324 | 1,004 |
| `$` | 3,255 | 4,521 | 1,858 | 17,713 | 9,751 | 145 | 803 |
| 100,000 items (10.1 MB) | | | | | | | |
| `items[0].name` | 63,183 | 18,391 | 2,362 | 354,761 | 3,708 | 33,594 | 76,769 |
| `$sum(items.price)` | 62,936 | 217,786 | 12,844 | 410,671 | 39,213 | 51,672 | 92,613 |
| `$` | 464,131 | 641,011 | 264,827 | 1,992,563 | 1,008,859 | 66,108 | 86,128 |

Linux x86-64 (WSL 2) gives the same picture.

- Panama takes 2 to 43% less time there, and 2.7 times less on `items[0].name` over the
  100,000-item `Document` (885 µs against 2,362).
- Chicory without the flag takes within 7% of the Windows times. With it, it mostly takes
  within 17% of them, but `$sum(items.price)` on a `Document` of 1,000 items takes 313 µs
  against 606.
- jsonata-java takes within 12% of the Windows times. The exception is parsing and summing
  the large document, which takes 1.9 times longer.
- The difference points at Windows' system allocator, which the native library would replace
  with mimalloc, as the Python wheels do (ADR 0014, amendment).

What this says:

- **Panama on a `Document` beats jsonata-java on its own Java objects** in every case but `$`.
  On 1,000 items it is 16 times faster on `items[0].name`, about 70 times on `$count(items)`, 4.3
  times on `$sum(items.price)` and 2.6 times on the filter. On 100,000 items it is 14 times
  faster on `items[0].name` and 4 times on `$sum(items.price)`.
- **`$` is not a fair comparison.** jsonata-java returns its input object, while jlens
  serializes the whole document and Java decodes it. A result written straight into Java
  objects (ADR 0017's builder, through upcalls) is the fair comparison, and it is not measured
  yet.
- **Panama on JSON text beats jsonata-java when both start from a `String`.**
  - On 1,000 items it is 1.4 to 2.7 times faster.
  - On 100,000 items, `items[0].name` is 1.2 times faster and `$sum(items.price)` 1.5
    times.
  - Only `$`, which returns the whole document, is slower.
- **A `Source` over Java objects only pays when the expression reads little.**
  - `items[0].name` and `$count(items)` take 120 and 113 µs, against 529 and 277 µs from
    JSON text.
  - Reading every item costs about 1.5 µs per level (one upcall, plus encoding in Java). So
    `$sum(items.price)` takes 3 times longer than from JSON text, and 10 times longer than
    jsonata-java. Python's level reading was close (1,770 µs for the same case).
  - The package would therefore take Java objects as an input, but neither as its default
    nor as its fast path.
- **Chicory fails the decision's condition for a sole backend.**
  - JSON text, the common case, is 3 to 6 times slower than jsonata-java parsing, even with
    the flag (22 times for `$`).
  - A `Document` is faster than jsonata-java when the expression reads little (`$count`,
    `items[0].name`). When it reads every item, it is 3 to 4 times slower on 1,000 items, and
    faster on 100,000 (`$sum(items.price)`: 39 ms against 52).
  - Chicory also needs a JVM flag to reach these numbers, which removes its one advantage
    over Panama: no launch flag.
- **The path `items[0]` on a `Document` of 100,000 items costs 0.9 to 2.4 ms**, as it did in
  Python (2,480 µs, ADR 0014). The core builds the whole sequence of items before taking the
  first. That belongs to `jlens-core`, outside this ADR.

The full tables, Linux included, are in the spike's `bench/results-*.md`.

**Item 3** (2026-10-25, the same module and versions as item 2).

- **No function is left to the interpreter.** The build passes with
  `interpreterFallback=FAIL`. The module gives 2,963 methods and 1.47 MB of bytecode. The
  largest method takes 25,776 bytes, 39% of the JVM's limit of 65,535.
- **29 methods exceed HotSpot's 8,000-byte limit for JIT compilation**, and they are hot
  paths. Without `-XX:-DontCompileHugeMethods` they run in the JVM's interpreter, which
  explains item 2's slow Chicory columns. They include:
  - `Evaluator::path`, `Evaluator::group` and `Evaluator::apply_inner`;
  - the pruned parse's `CutSeed` and serde_json's `f64_from_parts`;
  - the parser (`Parser::expression`, `led_step`) and the `Processor`;
  - ten of `jlens-regress`'s backtracking functions, and its parser;
  - `$replace`, `$formatNumber` and the date-time parser.
- **Built with `opt-level = "s"`**, as the WebAssembly package is, the module gives 4,718
  smaller methods: 16 exceed 8,000 bytes, and the largest takes 20,725. Without the flag,
  JSON text then takes 2.9 to 6.3 ms on 1,000 items instead of 13 to 25 ms, about what
  `opt-level = 3` takes with the flag (`$` stays near 20 ms either way). A `Document` still needs the flag:
  `$sum(items.price)` takes 1,087 µs without it, 386 µs with it (one run of each, Windows).
  The verdict of item 2 stands.
- **Sizes**, as a JAR compresses them (deflate):

  | Part | Raw | Compressed |
  |---|---:|---:|
  | Chicory: the generated classes and module metadata, `opt-level = 3` | 2.39 MB | 0.78 MB |
  | Chicory: the same, `opt-level = "s"` | 2.22 MB | 0.69 MB |
  | Chicory's runtime, `wasm`, `wasi` and `log` JARs | | 0.43 MB |
  | Panama: the Windows DLL | 2.28 MB | 0.93 MB |
  | Panama: the Linux `.so`, stripped | 2.40 MB | 0.97 MB |

- **What a user downloads.**
  - With Chicory: one JAR of about 1.2 MB, the same everywhere, with its runtime.
  - With Panama: about 1 MB per platform, so about 7 MB for one JAR with the seven
    targets, or about 1 MB per platform JAR with a Maven classifier.
  - Both are about the size of a Python wheel (1.1 to 1.3 MB, ADR 0014).

**Item 4** (2026-10-25, the same versions; the module linked with a 4 MiB stack, as
`jlens-wasm` is).

- **Method.** The shapes are those of the WebAssembly guide's depth table, plus a recursion
  through `$map`, each with and without a full audit. Each probe runs a fresh instance with a
  given depth limit and records whether the evaluation stops with `D1011` or overflows the
  JVM's stack (`StackOverflowError`).
  - The JVM's own stack is used: 1 MiB for the main thread and for a new platform thread,
    and its default for virtual threads.
  - In one JVM, three passes search every shape: the first runs cold, the later ones once the
    JIT has compiled more of the module. Each shape then runs 20 times at the smallest depth
    found.
  - Separately, each shape runs alone in a new JVM.
- **The worst case comes while the JIT is warming up**, neither cold nor warm. The smallest
  depth that fits was 273 on Windows and 263 on Linux, for the order-by
  (`[$n, $n]^($f($n - 1))`). It was the same within 4% on virtual threads, on platform
  threads and with `-XX:-DontCompileHugeMethods`.
  - A shape that runs alone in a new JVM fits more: 494 to 606 for the order-by and the
    `$map` recursion.
  - Once warm, most shapes fit 600 to 1,500.
  - At a limit of 250, every shape stopped with `D1011` in a new JVM, on Windows and Linux,
    on the main thread and on a virtual thread.
- **So on the JVM's default stacks, Chicory needs a default depth near 200**, against
  WebAssembly's 500 in V8 and 50,000 natively. 200 leaves the margin that 500 leaves against
  V8's 670. `DEFAULT_MAX_DEPTH` is chosen by `target_arch` alone, and V8 and Chicory both run
  `wasm32`, so the C API would pass its own default in `Options`.
- **Or the package evaluates on threads of its own, with larger stacks.** On a platform
  thread with an 8 MiB stack, the worst case fits 2,468 in a new JVM: the order-by, audited.
  - Once the JIT is warm, no shape overflows at all: the module's own guard, which watches its
    4 MiB stack, stops every evaluation with `D1011` first.
  - The cost is a handoff to that pool for each evaluation, from a virtual thread too. Items 6
    and 7 can measure it.
- **After a `StackOverflowError`, the instance still answers, but it is damaged.**
  - `1 + 1` and recursions within their limit still work.
  - The trap skips the code that restores the module's stack pointer, the `__stack_pointer`
    global. Each overflow left 420 to 715 KB of the 4 MiB stack in use, so a few overflows use
    it up.
  - What the evaluation held on the heap is lost, and an overflow in the middle of the
    allocator would leave it inconsistent.
  - The host can save the pointer before each call and restore it after an overflow; the
    spike did, and the stack was whole again. That does not repair the heap, so the package
    drops the instance instead, and its pool makes a new one.
- **The `__stack_pointer` at rest is 4,194,304, the size of the stack.** So the stack sits at
  the start of linear memory and grows down to address 0, as `--stack-first` places it and as
  `stack::exhausted` assumes. That answers the first half of item 5.
- **Panama** stops every shape, audited or not, at the default depth of 50,000 with `D1011`.
  That holds on the main thread, on a new platform thread and on a virtual thread, whose
  carrier's stack `stacker` extends.

**Item 5** (2026-10-25, the same versions; Chicory with HotSpot's default flags).

- **`--stack-first`**: yes, as item 4 showed. The `__stack_pointer` at rest equals the stack's
  size.
- **The workload** is `$sum([1..200].($sum([1..100000].($ * 2))))`, about 20 million steps.
  It takes 32.0 s on Chicory and 2.3 s through Panama on Windows, and 29.1 s and 1.9 s on
  Linux.
- **The timeout fires through the WASI clock.** On Chicory, `Instant::now` reads WASI's
  `clock_time_get`, which `WasiPreview1` provides.
  - Timeouts of 10, 100 and 1,000 ms stopped the evaluation with `D1012` every time, 5 runs
    each.
  - On Chicory the error came at most 5.1 ms late on Windows and 9.2 ms late on Linux.
    Through Panama it came at most 1.4 ms and 0.6 ms late.
  - The instance answered `1 + 1` afterwards: `D1012` is an ordinary error, not a trap.
- **Reading the clock costs nothing measurable.** The clock is read every 256 steps (ADR
  0018). Against no timeout at all, a timeout of an hour changed the time by -0.6% to +0.1% on
  Chicory and by +0.6% to +1.8% through Panama, within the noise of a single run.

**Item 6** (2026-10-25, the same versions; Windows, then Linux. On Linux the classes were
copied off WSL's `/mnt` mount first: read from there, they made the first evaluation 28 times
slower and each new instance 2.7 times slower.)

- **A Chicory instance**, warm, takes 0.55 to 0.58 ms to create, and 0.64 to 0.67 ms with a
  first `1 + 1`. That is 13 to 90 times one of item 2's small evaluations on a Chicory
  `Document` (5.9 to 42 µs).
  So a pool makes its instances ahead of time, and replacing one after an overflow (item 4)
  costs under a millisecond.
- **The first one in a JVM** costs more:
  - loading the module (`JlensModule.load()`): 108 to 140 ms;
  - creating the first instance: 49 to 64 ms;
  - its first evaluation: 46 to 51 ms.
  That is about 0.2 s of start-up, which a pool can pay when the application starts.
- **Each idle instance holds 4.8 MiB of the Java heap**, its linear memory: 71 pages of
  64 KiB, the 4 MiB stack of item 4 and the module's data. Parsing a `Document` of 1,000
  items grows it to 86 pages, 5.4 MiB.
  - A pool therefore costs about 5 MiB per instance, before its documents: 16 instances take
    80 MiB.
  - A smaller stack would save most of that. On the JVM's default stacks, only 0.4 to 0.7 MB
    of it was in use when the JVM's own stack overflowed (item 4). The stack size is
    therefore a choice between memory and the depth that 8 MiB threads allow.
- **Panama's calls are cheap**:
  - a downcall that does nothing: 9 ns;
  - an upcall that does nothing, made from Rust: 24 to 28 ns.
- **Reading one level through the reader** is mostly Rust's work, not the boundary's.
  - The reader's Java work for one item of item 2's document takes 223 to 300 ns: 137 to
    179 ns to encode its five fields (79 bytes), and 75 to 93 ns for the three downcalls
    that copy it into a Rust buffer. That includes the upcall.
  - Item 2's `$sum(items.price)` read 1,000 such items through the `Source`, at about
    1.45 µs per item on Windows and 1.1 µs on Linux, the evaluation itself deducted (its
    `Document` time).
  - By subtraction, 0.9 to 1.2 µs of each item is spent in Rust: decoding the level and the
    `Sink`'s allocations. Each key and string is an `Rc<str>`, and each nested array or
    object is a boxed source of its own.
  - The upcall is 2% of a level. The Python package's levers apply: keys interned for the
    expression, the arrays and objects of a level written as members of one `Sources`, and
    mimalloc. They are not measured here.

**Item 7** (2026-10-25, the same versions; 16 hardware threads, a Ryzen 7 3700X).

- **Method.** One compiled `$sum(items.price)` is evaluated on 9.5 KB of JSON text (100
  items of item 2's document), over and over.
  - The workers are 1 to 16 platform threads, or 1,000 virtual threads.
  - The rate is counted over 3 s, after 2 s of warm-up.
  - Chicory serves the workers from a pool of 16 instances, one per hardware thread. A
    virtual thread waiting for an instance unmounts from its carrier.
  - jsonata-java parses the text and evaluates, with one `Jsonata` object per thread.
- **A first run did not scale at all.** Panama gave 20,383 evaluations per second on one
  thread and 18,758 on 16. The spike's buffer table held its global lock while
  `evaluate_json` parsed the input. With one lock per buffer, and the table's lock held only
  to find a buffer, it scales as below. The package's C API must not serialize work behind
  its handle tables.

Evaluations per second (and the speed-up over one thread):

| Engine | 1 thread | 4 threads | 16 threads | 1,000 virtual threads |
|---|---:|---:|---:|---:|
| Windows | | | | |
| Panama | 19,795 | 71,596 (3.6×) | 150,036 (7.6×) | 149,892 (7.6×) |
| Chicory | 481 | 1,681 (3.5×) | 3,236 (6.7×) | 3,126 (6.5×) |
| Chicory, `-XX:-DontCompileHugeMethods` | 2,476 | 10,494 (4.2×) | 22,996 (9.3×) | 22,663 (9.2×) |
| jsonata-java | 11,922 | 45,023 (3.8×) | 85,472 (7.2×) | 86,319 (7.2×) |
| Linux (WSL 2) | | | | |
| Panama | 22,505 | 82,500 (3.7×) | 204,750 (9.1×) | 204,165 (9.1×) |
| Chicory | 454 | 1,536 (3.4×) | 3,125 (6.9×) | 2,974 (6.6×) |
| Chicory, `-XX:-DontCompileHugeMethods` | 2,544 | 9,600 (3.8×) | 22,831 (9.0×) | 20,880 (8.2×) |
| jsonata-java | 11,541 | 41,580 (3.6×) | 82,595 (7.2×) | 83,387 (7.2×) |

- **Pinning costs nothing here.** 1,000 virtual threads calling Panama give the rate of 16
  platform threads: the work is CPU-bound and each call is short (about 50 µs), so the 16
  carriers stay busy. A host function that blocks while its virtual thread is pinned would
  hold a carrier for that time; that is not measured.
- **Panama beats jsonata-java at every thread count.** It is 1.7 to 2.0 times faster on one
  thread, 1.8 times on 16 threads on Windows, and 2.5 times on Linux, where it scales further
  (9.1×).
- **A pool of 16 Chicory instances serves 1,000 virtual threads** at the rate of 16 platform
  threads. Chicory stays 3.6 to 4.8 times below jsonata-java even with the flag, and 25 to 26
  times below without it.
- **The warning** Java 25 prints without `--enable-native-access` is item 1's: four lines on
  standard error, once per module.

## Amendment: before the API work (2026-10-25)

### Context

The decision tied the package to the API stability work (ADR 0015), now roadmap 0.26. That
work is deferred until applications built on jlens have used jlens, with no date, so the
package would wait with it. The Python and WebAssembly packages did not wait: they shipped in
beta, and the API work covers them when it comes.

### Decision

1. **The package is roadmap 0.25, before the API work.** It ships in beta, with the
   workspace's 0.x versions, as the Python package does. Its API may change until 1.0, and
   ADR 0015 adds it to the contracts outside Rust that it freezes.
2. **Results as Java objects are part of the package**, not left for later. How they are
   built, through the result builder (ADR 0017) with upcalls or by decoding the JSON text in
   Java, is measured in 0.25 #5.

### Consequences

- When 0.26 breaks the API, the Java package changes with the Python and WebAssembly ones.
  `jlens-capi` is not published, so only the Java API's users see the change.
- The package becomes one more use of the contracts outside Rust that ADR 0015 decides from.
- Every release from 0.25 on publishes to Maven Central too: one more step in the checklist,
  and one more channel to check.

## Amendment: what the `Sink` allocates (0.24)

### Context

The Consequences and the results of spike item 6 say that the `Sink` allocates once for each
key. It does not: keys are interned. An evaluation keeps a table of the keys its sources have
written, and allocates an `Rc<str>` for each distinct key once (up to 1,024 of them); after
that a key costs a hash and a lookup in the table, and no allocation. Roadmap 0.24 #3 profiled
a level read through the `Sink` ([performance](../architecture/performance.md#where-the-time-goes)).

### Decision

1. **The spike's results stand corrected.** The `Sink` allocates once for each string, each
   array or object written (the lazy value), and each source written with `Sink::array` or
   `Sink::object`, which `Sink::array_of` and `Sink::object_of` avoid
   ([ADR 0010, amended](0010-lazy-input.md#amendment-one-allocation-per-member-019)). A key is
   allocated once per evaluation. The spike's reader wrote each nested value with its own
   source, so part of the 0.9 to 1.2 µs per item it found in Rust was those boxes.
2. **The levers decided above stand.** The Java package's reader writes the arrays and
   objects of a level as members of one `Sources`, and it also gives the `Sink` the size of
   each level (`Sink::reserve`), which since 0.24 makes room for the level's keys too.

### Consequences

- Most of what remains of a level is allocating and freeing a lazy value for each array or
  object, about 99% of the time of 10,000 objects that the expression does not read. mimalloc,
  already decided for the library, is the lever for it; the `Sink`'s own work is a few
  percent.
- An index into a long array reads the array's whole level, so `items[0]` costs as much as
  writing every item. A reader that could write one item alone, as `Source::read_field`
  writes one field, would avoid it: roadmap 0.24 #4 measures it.

## Results: the C API (0.25)

### Options and documents (0.25 #2)

- **Options are an object of the C API**, made once from JSON text with the names of the
  WebAssembly package's options (`timeout` in milliseconds, `maxDepth`, `maxSteps`,
  `maxStringLength`, `maxArrayLength`, `maxResultSize`, `now`, `seed`) and passed to each
  evaluation by handle, 0 for the defaults. An evaluation then reads no options: an
  application sets them once and evaluates many times. Options that are not in their form
  fail with `U2005`, an unknown name included, so that a misspelt limit does not pass
  unnoticed.
- **Documents** are JSON text behind an `Arc`, converted once per OS thread, as decided. A
  thread drops its conversion of a freed document the next time it makes, frees or evaluates
  against a document; a carrier thread that never does keeps it until it exits. (0.25 #3: at
  its next call of any export; see below.)
- **The reads** come back as the JSON of `Expression::reads`, and their union is made from a
  JSON array of them, as `union_reads` does in Python. A document is pruned to the `input` of
  either.

### mimalloc (0.25 #2)

`jlens_evaluate` on JSON text, called through the exports from Rust on the maintainer's
Windows machine, with the spike's documents and expressions. Each evaluation copies the text
into a buffer first, as the Java package will. Microseconds; the best of two alternated runs,
each the best of five runs of 0.3 s after a second of warm-up.

| Items | Expression | System allocator | mimalloc | Change |
|---:|---|---:|---:|---:|
| 1,000 | `items[0].name` | 465 | 338 | −27% |
| 1,000 | `$count(items)` | 253 | 220 | −13% |
| 1,000 | `$sum(items.price)` | 459 | 362 | −21% |
| 1,000 | `items[price > 98].id` | 613 | 503 | −18% |
| 1,000 | `$` | 2,976 | 1,953 | −34% |
| 100,000 | `items[0].name` | 56,892 | 37,237 | −35% |
| 100,000 | `$sum(items.price)` | 58,507 | 40,012 | −32% |
| 100,000 | `$` | 452,303 | 225,514 | −50% |

On a `Document`, where nothing is parsed, the same cases take 7 to 53% less: `$sum(items.price)`
over 100,000 items 5,646 µs against 12,021, and `$` 128,301 against 252,318. mimalloc is the
library's global allocator, with `local_dynamic_tls`, as in the Python package.

### Freed documents (0.25 #3)

A thread now drops its conversions of freed documents at its next call of any export, a buffer's
included, not only of the documents' exports. A count of the documents dropped, bumped when a
document's text is dropped (its last `Arc`, so after an evaluation that still held it returns),
tells each export whether the thread has conversions to sweep: one atomic load per call when
nothing was freed. A conversion is then kept only by a thread that never calls jlens again, at
most one per idle carrier thread. Releasing it at once on every thread was set aside: a
`jlens_core::Document` is not `Send`, an `Arc` inside it would slow down every evaluation, and
an export that releases the calling thread's conversions would not reach the carriers of
virtual threads.

`items[0].name` on a `Document` of 1,000 items, through the exports from Rust: 616 to 623 ns
before, 603 to 609 ns after. The lookup of a conversion no longer sweeps the cache on every
evaluation, which pays for the load.

### The comparison harness (0.25 #4)

`cargo xtask java-compare` runs `benches/compare/java` (its README says how): the spike's
documents, the workload of `benches/cases.json`, and the spike's throughput on platform and
virtual threads, with each engine's result checked against jlens's first. Maven Central,
searched again, has a JVM implementation the spike did not measure:
`jsonata-jvm-compiler` 1.0.7, which translates an expression to Java and compiles it with
`javac` at run time, over Jackson 2. The harness measures it with jsonata-java 0.9.10 and
JSONata4Java 3.0.0; the README lists the packages left out and why. Every result agreed with
jlens's, except two cases JSONata4Java rejects (`$formatNumber` on a path's value, the
transform operator).

Measured on Windows x86-64, Java 25.0.4, the machine of the spike:

- **The harness reproduces the spike.** jsonata-java, unchanged since, takes within 4% of the
  spike's times on its objects of 100,000 items and on parsing 1,000, 9% longer on parsing
  100,000 for `items[0].name`, and 5 to 11% longer on its objects of 1,000 items (83 against
  78 µs on `items[0].name`). On threads it gives 12,030 evaluations
  per second on one and 85,981 on 16, against 11,922 and 85,472.
- **jlens is faster than in the spike**, as #2 measured: on JSON text of 1,000 items it takes
  232 to 529 µs where the spike took 277 to 633, and 2,041 µs on `$` (3,255); on threads,
  25,350 evaluations per second on one, 224,252 on 16 and 217,297 on 1,000 virtual threads
  (19,795, 150,036, 149,892). A `Document` now costs the same whatever its size:
  `items[0].name` takes 0.88 µs on 1,000 items and 0.87 on 100,000, where the spike's took
  4.8 µs and 2,362.
- **jsonata-jvm-compiler is the one to beat on an input converted once.** On a Jackson tree it
  takes 0.03 µs on `path` (jlens's `Document` 0.54), 8.7 against 27 µs on
  `$sum(items.price)` over 1,000 items, and 1.8 (number formatting) to 26 times
  (`recursion`) less than jlens on every case of `cases.json` but one: on
  `($i := 0; items[$i].code)` it takes 43 µs against 1.2. Its expression is a Java class,
  which the JIT compiles with the application.
- **From JSON text, jlens is the fastest where the parsing is the cost.** It reads only the
  fields an expression reads (ADR 0016), and the others parse the whole text with Jackson
  or their own parser: on `path` it takes 32 µs against 99 to 158, and 366 against 686 to
  786 µs on `items[0].name` over 1,000 items. Where the evaluation is the cost,
  jsonata-jvm-compiler wins: 138 against 910 µs on `recursion`, 169 against 400 on
  `sort + map`. On 100,000 items jlens takes 40 to 42 ms against 70 to 108 for the others,
  but on `$`.
- **On threads, jlens leads all three**: 224,252 evaluations per second on 16 threads, against
  121,630 for jsonata-jvm-compiler, 85,981 for jsonata-java and 79,938 for JSONata4Java.
- **`$` is still the case to watch.** jsonata-java and the two Jackson engines return their
  input; jlens writes the whole document as JSON text (1,092 µs on 1,000 items, 128 ms on
  100,000). That is #5's to measure.

### Results as Java objects (0.25 #5)

Three ways of building a result as Java objects were measured with #4's harness, each against a
`Document`, on the maintainer's Windows machine:

- **Upcalls**: the result builder (ADR 0017) calls one Java function for each value, key and
  end of an array or object, and Java builds the objects on a stack of its own.
- **JSON text decoded in Java**: the C API writes JSON text, as before, and Jackson's
  `readValue(text, Object.class)` decodes it, a well-optimized decoder the package would not
  depend on, so the upper bound of this way.
- **Values**: the result builder writes a tagged binary encoding into the reply, which Java
  decodes with no parser and no upcall: a tag byte for each value, lengths and numbers
  little-endian, and each distinct key once, then by its index, so that Java makes one `String`
  per key for the whole result.

Microseconds per evaluation, the best of five runs of 0.3 s after a second of warm-up:

| Items | Expression | JSON text, not decoded | Values | Upcalls | JSON text + Jackson |
|---:|---|---:|---:|---:|---:|
| 1,000 | `items[0].name` | 0.85 | 0.84 | 0.95 | 0.97 |
| 1,000 | `items[price > 98].id` | 116 | 119 | 121 | 117 |
| 1,000 | `$` | 1,134 | 498 | 2,644 | 1,878 |
| 100,000 | `$sum(items.price)` | 5,863 | 5,715 | 5,797 | 5,805 |
| 100,000 | `$` | 132,685 | 56,450 | 282,832 | 206,302 |

On `benches/cases.json` the values are within the noise of JSON text on small results, and
faster on large ones: `object constructor` 126 µs against 163 (214 with Jackson), `transform`
485 against 570 (677).

- **Values are kept**: they are the fastest, or within the noise of the fastest, on every
  case, and on large results faster even than JSON text that is not decoded at all, since a
  value is written with no formatting and a key once. Upcalls cost
  about 25 ns each and one is made for each value and key, so they are the slowest on large
  results; decoding JSON text pays for writing it and parsing it again.
- **The C API** takes the format as an argument of `jlens_evaluate` and
  `jlens_evaluate_document`: 0 for JSON text, 1 for values, whose reply starts with byte 4.
  The encoding is documented in `jlens-capi`'s `values` module.
- **The Java types**: objects are `LinkedHashMap`s, their fields in order; arrays `ArrayList`s;
  integral numbers in a `long`'s range `Long`s, the others `Double`s. A `BigDecimal` was set
  aside: jlens's numbers are `f64`s, so it would be slower and suggest a precision the value
  does not have, and `Double.toString` already prints the shortest decimal that reads back
  the same. The Python package's bound of 10^21 for an `int` does not fit a `long`; past
  2^63 a number is a `Double`. Undefined and `null` are both `null`, as in Python;
  `evaluateJson` tells them apart. The objects are made for each evaluation, and the caller may
  modify them.
- **`$` on a `Document`** now takes 502 µs on 1,000 items, against 154 for jsonata-java, and
  56.6 ms on 100,000, against 67.1 ms. The Jackson engines take 0.02 to 0.08 µs because they
  return their input. jlens can do the same only when its input is Java objects, which #6 adds:
  a result that is the input unchanged is then returned as is.

### Java objects as input (0.25 #6)

`Expression.evaluate(Object)` reads maps, lists, strings, numbers, booleans and `null` one level
at a time, as the evaluation reaches them, through one upcall per level (`source.rs` in
`jlens-capi`):

- **One `Sources` per evaluation.** Java numbers each map and list as it meets them, and every
  array or object of the input is a member of one `Sources` in Rust, known by its number: a
  level costs Rust no allocation of its own, the levers of the spike's item 6 and of the
  amendment above. Each level written gives its size to `Sink::reserve`.
- **One field or one item alone.** The reader has four operations: a whole level, one field of
  a map (`read_field`; Java gets the key's UTF-8 bytes from Rust's memory), the length of a list
  and one of its items (`length`, `read_item`), so a path or an index reads what it picks. A list
  that is not `RandomAccess` (a `LinkedList`) tells no length, and is read whole. A reader may
  also decline a field or an item; the level is then read whole.
- **One buffer.** Java encodes a level into a byte array and copies it into one C API buffer per
  evaluation, made larger when a level does not fit: a level costs one upcall and no downcall.
- **The input returned as itself.** A map or list the result holds unchanged comes back as the
  object itself, through `Builder::input` ([ADR 0017, amended](0017-result-builder.md#amendment-the-input-returned-as-itself-025)):
  `$` returns the input, as jsonata-java does, and a filter returns a new list of the input's
  maps.
- **Failures.** A type the reader does not read (a `Date`, an `int[]`), a key that is not a
  string, or a map or list inside itself fails with `U1004` when the evaluation reads it, with
  a message that names it; an exception never crosses the upcall. A cycle is detected through
  the chain of levels a value was read from, as `json.dumps` does in Python. Input nested deeper
  than a result may be fails with `U1001` when it is written; returned as itself, it is not
  written. On threads of 128 KiB to 1 MiB, inputs nested 100,000 levels deep stop with `U1001`
  or give their result, with the upcalls made from the stack segments jlens grows into
  (`jlens_core::stack`): the JVM runs them there.

Microseconds per evaluation with #4's harness on the maintainer's Windows machine, Java 25.0.4;
"objects" reads the maps and lists that Jackson's `readValue(json, Object.class)` built, and the
spike's "Source" column (above) is repeated for comparison:

| Items | Expression | Spike, Source | Objects | `Document` | jsonata-java | jsonata-jvm-compiler |
|---:|---|---:|---:|---:|---:|---:|
| 1,000 | `items[0].name` | 120 | 2.52 | 0.86 | 83.7 | 0.03 |
| 1,000 | `($i := 0; items[$i].name)` | | 2.92 | 1.22 | 149 | 0.65 |
| 1,000 | `$count(items)` | 113 | 1.94 | 0.85 | 89.0 | 0.03 |
| 1,000 | `$sum(items.price)` | 1,490 | 327 | 27.5 | 152 | 8.0 |
| 1,000 | `items[price > 98].id` | 1,601 | 398 | 117 | 329 | 15.4 |
| 1,000 | `$` | 4,521 | 0.80 | 483 | 157 | 0.03 |
| 100,000 | `items[0].name` | 18,391 | 2.59 | 0.89 | 33,685 | 0.04 |
| 100,000 | `($i := 0; items[$i].name)` | | 2.90 | 1.23 | 40,436 | 217 |
| 100,000 | `$sum(items.price)` | 217,786 | 48,241 | 5,736 | 55,551 | 4,142 |
| 100,000 | `$` | 641,011 | 0.80 | 54,329 | 66,889 | 0.02 |

- **What reads little costs the same whatever the size**: `items[0].name` takes 2.5 µs on 1,000
  or 100,000 items, against 120 µs and 18 ms in the spike, which read each level whole, and
  34 ms for jsonata-java. `$` takes 0.8 µs, the cost of an evaluation that returns its input.
- **What reads everything costs 0.3 to 0.5 µs a level**, against the spike's 1.5:
  `$sum(items.price)` takes 327 µs on 1,000 items, 4.5 times less than in the spike but twice
  jsonata-java's 152 µs, and 48 ms on 100,000, ahead of its 56 ms. A `Document` stays the fast
  way for an input evaluated many times, or read whole.

### Host functions (0.25 #7)

Host functions written in Java follow ADR 0006 and ADR 0011, as the Python and WebAssembly
packages do, through one more upcall (`host.rs` in `jlens-capi`):

- **`Bindings`, made once.** Variables and host functions are bound by a `Bindings` object,
  built by a builder and immutable, that holds a native object: its variables are written to
  Rust when it is built, so an evaluation does not convert them again, and one `Bindings` serves
  any number of evaluations, of any expression, on any thread. Every `evaluate` method takes
  one. There is no `registerFunction`: it would make an `Expression` mutable, shared between
  threads, and a `Bindings` built once plays its part. A name holds a value or a function, and a
  name given with its `$` is taken without it, as `registerFunction` does in Python and
  WebAssembly.
- **`HostFunction`, a functional interface**, `Object call(List<Object> arguments)`, bound with
  an arity or a signature: a Java lambda has no arity Java can read, where Python reads the
  parameters of a function. It throws no checked exception, so that `evaluate` declares none. A
  host function does not receive the context of its call (`HostFunction::without_input`), as in
  Python: a signature with `-` passes it as an argument.
- **Values both ways.** The arguments reach Java as values (0.25 #5), with two more tags: an
  undefined argument, and a function with its arity, which Java turns into a `JsonataFunction`.
  The names `Function` and `Callable` would clash with `java.util.function.Function` and
  `java.util.concurrent.Callable`. Java writes its results, the arguments of the functions it
  calls and the variables in the same encoding, which Rust decodes with no parser; a value
  nested more than 1,000 levels deep fails with `U1001`, since Rust would drop it by
  recursion. Java's writer walks with its own stack, as `Values` does.
- **Calling a function received.** The core's `Callable` borrows the call (ADR 0011). Each host
  call is a frame of a scoped thread-local in Rust, numbered, and Java's `JsonataFunction` calls
  `jlens_call` with that number. A virtual thread is pinned during the downcall, so the upcall,
  and the downcall it makes, run on the same carrier. A call after the host function returned,
  or from another thread, fails with `U1005`: Java checks it first, Rust again.
- **Exceptions thrown again, unchanged.** An exception a host function throws is kept by the
  evaluation, in Java, under a number that the error carries back through Rust
  (`hostException` in the envelope), and `evaluate` throws the same object again. An error of a
  function a host function called gets a number from Rust, kept while a host call runs on the
  thread; when the host function lets it through, Java replies with the number, and the error
  keeps its code and position, as `failures` does in Python. A `CompletionStage` or a `Future`
  as a result fails with `U1002`, as a promise or a coroutine does.

Host functions run while their virtual thread is pinned: one that blocks holds its carrier for
that time, which the Java chapter says (0.25 #13).

Measured on the maintainer's Windows machine, Java 25.0.4. The evaluations that take no bindings
did not change: #4's harness on 1,000 items, 02a69e1 against this item, gave the same times
within the noise, on a `Document` (`items[0].name` 0.89 µs before, 0.84 to 0.88 after) and on
Java objects (2.63 before, 2.52 to 2.56 after). Microseconds per evaluation with no input, the
best of five runs of 0.3 s:

| Expression | Bindings | µs |
|---|---|---:|
| `1 + 2` | none | 0.48 |
| `$a + $b` | two variables | 0.67 |
| `$a + $b` | two variables, one function | 0.78 |
| `$a + $b` | two variables, four functions | 1.10 |
| `$a + $b` | two variables, sixteen functions | 2.17 |
| `$add(1, 2)` | two variables, four functions | 1.99 |
| `$add(1, 2)`, signature `<nn:n>` | the same | 2.15 |
| `$apply(function($x) { $x + 1 }, 1)` | the same | 2.93 |
| `$sum([1..100].$add($, 1))` | the same | 93.7 |
| `$sum([1..100].($ + 1))` | the same | 12.7 |

- **A host call costs about 0.8 µs**: an upcall, the arguments written and read as values both
  ways, and the result. Calling back a function it received costs as much again.
- **Bound functions cost about 0.09 µs each per evaluation, called or not**: the core binds
  every host function in the evaluation's root frame before it starts (`eval.rs`), a cost the
  Rust, Python and WebAssembly packages share. The Java side's own cost, the evaluation's
  `Host` in a `ScopedValue`, is about 0.02 µs. Looking a host function up in the bindings when
  a name is not found, instead of binding them all, would remove it; that is a change to the
  core, left for its own item. 0.25 #9 made it ([ADR 0006, amended](0006-host-functions.md#amendment-host-functions-looked-up-on-a-miss-025),
  and [below](#host-functions-looked-up-on-a-miss-025-9)).

### Host functions looked up on a miss (0.25 #9)

The core looks a host function up in the bindings when no frame holds its name, and keeps it in
the root frame, instead of binding every one before the evaluation ([ADR 0006, amended](0006-host-functions.md#amendment-host-functions-looked-up-on-a-miss-025)).
Measured on the maintainer's Windows machine, Java 25.0.4, microseconds per evaluation with no
input, the best of five runs of 0.3 s, the JAR of 4701ddc against this item's, both run from the
class path by the same program (which is why the times are above those of #7's table):

| Expression | Functions bound | Before µs | After µs |
|---|---:|---:|---:|
| `1 + 2` | none | 0.79 | 0.78 |
| `$a + $b` | none | 0.96 | 0.96 |
| `$a + $b` | one | 1.10 | 1.03 |
| `$a + $b` | four | 1.37 | 1.03 |
| `$a + $b` | sixteen | 2.50 | 1.04 |
| `$add(1, 2)` | four | 2.21 | 2.05 |
| `$add(1, 2)` | sixteen | 3.27 | 2.06 |

- **A function not named costs nothing**, however many are bound. In Rust, `$a + $b` takes
  0.40 µs with one, four or sixteen functions bound, against 0.39 µs with none (2.82 µs with
  sixteen before).
- **What stays is Java's own, about 0.07 µs** once the bindings hold a function, whatever their
  number: the evaluation's `Host` in a `ScopedValue` and the frames' thread-local set in Rust.

### Audits (0.25 #8)

Audits follow the Python and WebAssembly packages' `evaluate_with_audit`, through one more
export, `jlens_evaluate_audited` (`audit.rs` in `jlens-capi`):

- **The input crosses whole.** The core audits an evaluation of JSON values, and the header
  hashes the input's JSON text, so `evaluateWithAudit` writes its Java objects as values first
  (the encoding of 0.25 #7's bindings), not through the reader of 0.25 #6. A `Document` is
  refused: it may be pruned, and its hash would not be the input's.
- **One reply of values.** Rust writes the result, the error, the audit tree and the
  explanation as one object of values, which Java decodes with `Values` and turns into records.
  The error is its envelope as JSON text, the one `JlensException` already reads. The tree's
  spans are UTF-16 offsets, converted in Rust with a table of the source's offsets.
- **Records, and a sealed interface for decisions.** `AuditResult`, `AuditTree` with its
  `Header`, `AuditNode`, `Explanation` with its `Line`, `AuditOptions`, `CaptureLimits` and
  `Span` are records; levels, outcomes and branches are enums. The ADR's Decision said that an
  `AuditNode` would be a sealed hierarchy. It is one record instead, since every node has the
  same fifteen components; what a node decided is the sealed hierarchy, `AuditNode.Decision`:
  a `Comparison` with its two operands, a `ShortCircuit`, a `Conditional` with its branch, or a
  `Binding` with its variable, `null` for a node that decided none of these. The core sets at
  most one of the four fields, so a `switch` over them is exhaustive.
- **Errors are reported, not thrown**, as in Python and WebAssembly: an error of the evaluation
  is the result's `error`, a `JlensException`. A host function's exception does not throw
  either; the error has its code, and the exception as its cause, the same object.
- **Deep trees.** An audit nests as deep as the evaluation did. Java builds the nodes from a
  queue, children first, never by recursion, and `AuditNode.toString` prints one node, not its
  descendants. A full audit of a recursion 1,000,000 calls deep stops with `D1011`, and one
  10,000 calls deep succeeds with a truncated tree, on threads of 128 KiB and 1 MiB.

Measured on the maintainer's Windows machine, Java 25.0.4, the rule of the Python and
WebAssembly suites on its order of three items, as Java objects, with one variable bound.
Microseconds per evaluation, the best of five runs of 0.3 s, and the Python package's
`evaluate` and `evaluate_with_audit` on the same machine:

| Evaluation | Java µs | Python µs |
|---|---:|---:|
| `evaluate`, no audit | 5.8 | 3.6 |
| audited, level `off` | 22.8 | 15.5 |
| audited, level `decisions` | 49.3 | 40.6 |
| audited, level `full` | 188.8 | 190.3 |

An audit at level `off` costs 17 µs more than an evaluation: the input written whole and hashed,
and the tree and the explanation written, read and made into records. Java's part is about
7 µs more than Python's at `off` and `decisions`, and nothing at `full`, where capturing values
in Rust dominates.

### The allocator outside Windows (0.25 #11)

Found while testing the JAR (#12): mimalloc 3.3.2, the version `libmimalloc-sys` 0.1.49
bundles, crashes on glibc in a library loaded at run time once the thread that loaded it has
ended. A later thread's first allocation finds a heap whose thread data is null and faults in
`_mi_subproc`, with the `local_dynamic_tls` model as with initial-exec. The JVM crashed at the
third short-lived thread after the loading thread ended; the Python package 0.24.0 segfaults
when `import jlens` runs in a thread that ends. Windows and musl do not crash, nor does the
`jlens` command, an executable. The suites never saw it: their threads live as long as the run.

`ThreadsTest.loadedOnAThreadThatEnds` and `test_loaded_on_a_thread_that_ends` load the library
on a thread that ends, then evaluate on eight new threads, one after another, in a new process.
Both fail with 3.3.2 on glibc (the JVM aborts with 134; the PyPI wheel of 0.24.0 and a wheel
built with `maturin build` exit with 139) and pass with every candidate below.

**Decision**: mimalloc 2 (2.3.2, the crate's `v2` feature) in `jlens-capi` and `jlens-python`
on every target but Windows, where mimalloc 3 stays. One target-specific dependency in each
`Cargo.toml`, no code. The `jlens` command keeps mimalloc 3: its release binaries are built
alone (`-p jlens-cli`), while a build of the whole workspace on Linux unifies the feature and
gives it mimalloc 2.

The C API's evaluations through its exports, as in [the table of #2](#mimalloc-025-2): JSON
text copied into a buffer per evaluation, then a `Document`. Microseconds, the best of two
alternated runs, each the best of five runs of 0.3 s after a second of warm-up. 3.6.0 is
upstream's release of 2026-10-23, put in place of 3.3.2's sources in a copy of
`libmimalloc-sys`; it passes the regression tests.

Linux (WSL, glibc):

| Items | Expression | 3.3.2 | 3.6.0 | 2.3.2 | jemalloc | glibc |
|---:|---|---:|---:|---:|---:|---:|
| 1,000 | `items[0].name` | 307 | 292 | 297 | 355 | 381 |
| 1,000 | `$count(items)` | 207 | 206 | 205 | 238 | 222 |
| 1,000 | `$sum(items.price)` | 338 | 324 | 329 | 366 | 378 |
| 1,000 | `items[price > 98].id` | 470 | 457 | 462 | 500 | 510 |
| 1,000 | `$` | 1,834 | 1,792 | 1,759 | 2,047 | 2,223 |
| 100,000 | `items[0].name` | 34,049 | 33,132 | 33,993 | 44,650 | 39,727 |
| 100,000 | `$sum(items.price)` | 37,434 | 35,823 | 37,140 | 45,703 | 40,780 |
| 100,000 | `$` | 216,536 | 210,379 | 209,392 | 264,134 | 255,974 |
| 100,000 | `$sum(items.price)`, `Document` | 5,591 | 5,204 | 5,170 | 7,188 | 10,503 |
| 100,000 | `$`, `Document` | 123,499 | 121,221 | 116,460 | 145,703 | 151,893 |

Windows (the system allocator from a pair of runs alternated with 3.3.2 and 2.3.2 alone):

| Items | Expression | 3.3.2 | 3.6.0 | 2.3.2 | System |
|---:|---|---:|---:|---:|---:|
| 1,000 | `items[0].name` | 318 | 296 | 344 | 441 |
| 1,000 | `$sum(items.price)` | 339 | 333 | 366 | 448 |
| 1,000 | `$` | 1,973 | 1,941 | 1,997 | 3,006 |
| 100,000 | `items[0].name` | 34,488 | 33,544 | 38,594 | 54,764 |
| 100,000 | `$sum(items.price)` | 36,477 | 36,366 | 41,964 | 57,964 |
| 100,000 | `$` | 226,735 | 223,382 | 267,865 | 437,288 |
| 100,000 | `$`, `Document` | 125,484 | 131,275 | 135,176 | 248,734 |

The Python package with Python values in (`python.py speed`, WSL, Python 3.14, the median of
three alternated runs): with mimalloc 2, every case takes the same time as with 3.3.2 within
1%, or less: `recursion` 682 µs against 734, the index cases 0.1 µs less; the wheel is 7 KB
smaller.

- **On Linux the three mimalloc versions are within 8%** of each other, mimalloc 2 the fastest
  on `$` and on a `Document`. jemalloc (`tikv-jemallocator` with `disable_initial_exec_tls`) is
  8 to 39% slower than mimalloc 2, and glibc's allocator 8 to 103%.
- **On Windows mimalloc 2 is up to 18% slower on JSON text**, and 8% on `$` over a `Document`;
  3.6.0 is within 5% of 3.3.2. Since
  3.3.2 does not crash there, Windows keeps it.
- **3.6.0 was not taken**, though it passes: `libmimalloc-sys` has not published it, so it
  would mean carrying a copy of the crate with upstream's C sources, through
  `[patch.crates-io]`, for the whole workspace and the Python package's source distribution;
  and it is two days old, its release notes silent on this fault. When `libmimalloc-sys`
  bundles a v3 that passes the two regression tests, every target can move back to one
  version.
- macOS takes mimalloc 2 too, unmeasured here: whether 3.3.2 has the fault there was not
  tested, and mimalloc 2 is the version known to pass.

### The JAR (0.25 #12)

`.github/workflows/jar.yml`, called by CI on each push to `main`, builds the C API of the seven
release targets as the wheels are built, packs them in one JAR, then runs the JUnit suite on
each target against that JAR: the suite's classes are compiled, the package's replaced by the
JAR's, and `LoaderTest` checks that the loader picked the platform CI names in
`JLENS_TEST_PLATFORM`. The JAR's directories are Rust's `{OS}-{ARCH}`, with `-musl` after them
on musl (`linux-aarch64-musl`).

- **glibc 2.28**, as the manylinux wheels: the glibc libraries are built in the
  `manylinux_2_28` containers, with Rust installed there by rustup, and the job fails when a
  symbol needs a newer glibc than 2.28.
- **musl** is built in `rust:alpine`. A library loaded by a JVM linked against musl must link
  musl dynamically (`-crt-static` off), and then Rust links its unwinder from `libgcc_s`, which
  Alpine's JDK packages do not depend on: of the Alpine JDKs, only Temurin's brings it. A
  linker script named `libgcc_s.so`, put first on the search path, links Rust's own
  `libunwind.a`, the one a static build uses, so the library needs musl alone, which the job
  checks. The export tests, built the same way in the same container, check that a panic is
  still caught (`U2004`). The suite runs in `alpine:3.24` on its `openjdk25-jdk`, which has no
  `libgcc`.
- **The loader tells musl from glibc** by the dynamic loader mapped into the process
  (`/ld-musl-` in `/proc/self/maps`). Files on disk do not tell: a glibc system can have musl
  installed.
- **Extraction.** The library is copied to `jlens-<user>-<SHA-256 of its bytes>` in
  `java.io.tmpdir`. Each JVM writes its own temporary file there, then moves it in place
  atomically; a failed move is accepted when the file in place has the library's size, as on
  Windows, where a library another JVM moved in first and holds open cannot be replaced. The
  temporary directory is shared on Linux and macOS, so another user could make the directory
  first and put a library of their own in it: the loader makes it with permissions `rwx------`,
  and refuses one that is a link, that another user owns (compared with the owner of a
  temporary file it makes) or that the group or others may write to, with an
  `UnsatisfiedLinkError` that names the ways out (remove it, or set `java.io.tmpdir` or
  `jlens.library`). Windows has no POSIX permissions there; its temporary directory is the
  user's own.
- **`LoaderTest`**: the platform detected has its library in the JAR; eight JVMs started at
  once leave one library and no partial file; a directory others may write to is refused, then
  accepted again once restored.

Run locally against one JAR carrying the Windows, glibc and musl x86-64 libraries, all 57 tests
pass on Windows, in WSL (glibc, against the `manylinux_2_28` library, whose newest glibc symbol
is 2.28) and on Alpine 3.24 (`openjdk25-jdk`, and earlier Amazon Corretto's Alpine build).
Before #11, the glibc run crashed in mimalloc 3. ARM and macOS run in CI, and macOS on ARM also
on a MacBook Air M1.

**macOS on ARM: room for the JVM below a call back.** The first CI run failed one test there,
only there: `$string($)` over Java lists nested 2,000 deep, on threads of 128 and 256 KiB,
failed with `U1004`, the reader's `StackOverflowError`; when the reader's handler overflowed
too, the exception left the upcall and the JVM aborted (`upcallLinker.cpp`, "Unrecoverable
uncaught exception"). HotSpot sizes its guard zones and its shadow zone in 4 KiB units rounded
up to the page size, and macOS uses 16 KiB pages on ARM: Java needs about 128 KiB free below
the caller's frame there (red, yellow and reserved zones 48 KiB, shadow zone 80 KiB), against
96 KiB with 4 KiB pages. `jlens_core::stack::grow` moves to a new segment when less than 128 KiB
is left, and the crash report showed exactly 128 KiB free at the call. Every call back into the
host, the reader's and host functions', now goes through `upcall` in `jlens-capi`, which moves
to a new segment of 4 MiB when less than 256 KiB is left. The core keeps its 128 KiB. Host
functions never failed, at any depth from 0 to 3,000 on either stack, but take the same route.
On Windows the calls cost the same within noise (`items[0].name` over Java objects 2.48 against
2.49 µs, `$sum(items.price)` over 1,000 items 324 against 316 µs, a host call 1.73 against
1.71 µs, the best of three alternated runs).

The JAR's size: each library is about 3 MB, about 1.1 to 1.25 MB compressed (Linux x86-64
3.19 MB, 1.22 MB in the JAR; Windows 2.74 MB, 1.07 MB), so about 8.5 MB for the seven, more
than the 7 MB the decision estimated. Per-platform JARs remain the way out if users ask.
