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
