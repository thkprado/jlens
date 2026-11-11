# 0023 — The schema and the language service through the C API and in the Java package

**Status:** Accepted (2026-11-10).

## Context

The Java package left the schema and the language service out ([ADR 0021](0021-java-package.md);
"What the package leaves out" in its guide), to be added when a Java application asked for them.
One does now. It stores expressions written against a schema that another system sends it, and
when a new version of the schema arrives it checks each stored expression again, on a server
(roadmap 0.33): does the expression still pass the diagnostics, and did anything it uses change?

`jlens-schema` answers the first question in Rust, and the WebAssembly package and the language
server give its answer to editors. Roadmap 0.33 #5 and #6 add the second: what an expression uses
of its schema, and what changed in that between two schemas. A Java application reaches none of
this today but by running `jlens check` as a process.

What `jlens-schema` has, and what each part would cost to carry:

- **A `Schema`**, read from a JSON Schema document (`Schema::from_json_schema`), or inferred from
  sample documents (`Schema::infer`, `apply_overrides`).
- **A `LanguageService`**, made of a schema, the variables the expressions are evaluated with
  (each with the shape of its value, or none) and the host functions declared to it. It gives
  the diagnostics of a text, and, at an offset of it, completion, hover and signature help.
- Both are plain data once made, with no `Rc` and no interior mutability: `Send` and `Sync`,
  as a compiled `Expression` is, and unlike a `Document`.
- The crates `jlens-schema` depends on (`serde`, `serde_json`, `indexmap`) are already in the
  library, through `jlens-core`.

**What it adds to the library's size**, measured with a probe on the C API of 0.32.0 (Windows
x86-64, the release build as `cargo xtask java` makes it; compressed with `gzip -9`, close to
what the JAR's deflate gives):

| Library | Raw | Compressed |
|---|---:|---:|
| 0.32.0 | 3,125,760 bytes | 1,167,801 |
| With a schema from JSON Schema text, a service and its diagnostics | 3,287,552 (+5.2%) | 1,236,293 (+5.9%) |
| With completion, hover and signature help too | 3,376,640 (+8.0%) | 1,272,207 (+8.9%) |

The diagnostics bring the JSON Schema reader and the typing, 162 KB; the three editor operations
add 89 KB to that. Over the JAR's seven targets, of about 9.1 MB together, that is about 0.5 MB
for the first row and 0.7 MB for both. The code of #5 and #6 is not written yet and is not in
these numbers; roadmap 0.33 #11 measures the library and the JAR as released.

## Decision

1. **`jlens-capi` depends on `jlens-schema`, with no feature to leave it out.** One library and
   one JAR, as now. 5 to 6% of the library does not pay for a second build of seven targets, a
   second artifact on Maven Central and a loader that tells them apart.

2. **What crosses in 0.33: a schema read from JSON Schema, a language service, its diagnostics,
   and #5 and #6.**
   - **Completion, hover and signature help do not cross.** They serve a person typing, in an
     editor, which the WebAssembly package and the language server reach; the Java application
     that asked checks stored expressions on a server. Carrying them is cheap in size (the
     table above) but not in surface: five more record types in Java (`Completions`, its items,
     the value request and its conditions, `Hover`, `SignatureHelp`), their tests and their
     guide, in an API that ADR 0015 will have to keep. They can be added the same way (the text
     in a buffer, a UTF-16 offset, a JSON reply) when a Java application serves an editor.
   - **Schema inference does not cross either**, nor the overrides: `jlens schema infer` writes
     a JSON Schema from samples, and that file is what a Java application loads.

3. **Two new kinds of object, known by handle** as the others are: a schema and a service.
   - **A schema** is made from the JSON Schema text in a buffer (`jlens_schema_new`; since
     0.33 #10 with a second buffer, its sources, as
     [ADR 0025](0025-references-to-other-documents.md#results-the-documents-and-the-report-at-every-end-033-10)
     describes) and freed by its own export. It is a kind of its own, not an argument of the service, because a
     load has results of its own, which later items give by its handle: the documents a `$ref`
     named and the application did not give (#9), the keywords the load did not read (0.34 #8).
     Those items also change what a schema is made from (other documents, by URI), which then
     changes one export.
   - **A service** is made from a schema's handle, 0 for none, and the JSON text of its
     declarations in a buffer, 0 for none (`jlens_service_new`): an object with `variables`,
     an object from each name to the handle of the schema of its value, written as a decimal
     number, or `null` for a variable declared with no schema; and `functions`, an array of
     declarations with the fields of the WebAssembly package's `FunctionDeclaration` (`name`,
     `signature`, `params`, `summary`). Names are taken with or without `$`. An unknown member
     fails, as it does in the options, so that a misspelt one does not pass unnoticed.
   - **Neither can be changed once made.** A new version of a schema is a new schema and a new
     service.

4. **Lifetimes.** A service keeps what it needs of the schemas it was made from: freeing a
   schema, before or after, does not affect it. Whether it shares the schema or copies it is
   not visible to the host; #3 chooses, measuring the copy on the largest fixture. A call that
   runs on another thread keeps the object it uses until it returns, as an evaluation keeps
   its expression. In Java, `Schema` and `LanguageService` are `AutoCloseable` with a `Cleaner`
   behind them, as the package's other native objects.

5. **Threads.** One schema and one service serve every thread, platform or virtual, with no
   lock held while they work and no copy per thread: they are `Send` and `Sync`, which a test
   of `jlens-capi` asserts at compile time. Their tables are locked only to find an entry. The
   per-thread caches that evaluations find their objects in (ADR 0021, results of 0.26 #10) are
   not used here: they saved a lock on calls of under a microsecond, and a call that types an
   expression is not one.

6. **The form of what crosses is JSON text**, in both directions.
   - The expression's text crosses as UTF-8 in a buffer, as a source does for `jlens_compile`.
   - **Diagnostics** (`jlens_service_diagnostics`) are replied as the JSON array the other
     packages give: each with `severity`, `code`, `message` and `span`. A text with a syntax
     error is not a failure of the call: its errors are diagnostics. The package's own JSON
     reader decodes the reply into records; the values encoding (ADR 0021, results of 0.25 #5)
     is for results, which may be large and are the caller's data, and a list of diagnostics is
     neither.
   - **Spans are UTF-16 offsets**, converted by the C API, as an error's `position` and an
     audit's spans are: a Java `String` indexes them with no conversion.
   - **What an expression uses of its schema** (#5) is replied by the service as the one JSON
     form that item defines, the same from every package (#7).
   - **What changed between two schemas** (#6) takes two of those JSON texts in buffers and no
     handle, as `jlens_union_reads` does for reads: an application can store an expression's
     answer beside it and compare it with a new schema's answer when the old schema is gone.
     In Java the answers are records that give their JSON text back, as `Reads` does.
   - These replies come in a buffer of their own, not in the evaluations' scratch buffer:
     reading one costs three calls more, against the typing of an expression.

7. **Failures use the C API's codes.** Text that is not JSON fails with `U2001`; a JSON Schema
   that jlens refuses, or declarations not in their form, with `U2005`, whose `token` says
   which (`schema`, `service`), and whose message is the load's; a handle unknown or freed with
   `U2002`; an expression that is not UTF-8 with `U2003`. In Java, `Schema.fromJson` throws a
   `JlensException` with the code, as `Document.fromJson` does for text that is not JSON.

8. **A host that uses none of this pays the size and nothing else.** No evaluation export
   changes, nothing is added to `guarded`, and the two tables are empty statics.

## Consequences

- The library grows by about 5% and the JAR by about 0.5 MB, before #5 and #6.
- The Java package gains `Schema`, `LanguageService`, a `Diagnostic` record and the records of
  #5 and #6, with the same codes, messages and spans as the WebAssembly package, the language
  server and the CLI. A test pins the JSON that each gives for the same cases (#7).
- The guide's "What the package leaves out" shrinks to completion, hover, signature help and
  schema inference, with where each is found.
- The C API stays free of Java specifics: another host that calls C gets the diagnostics with
  the exports of this ADR and a JSON reader.
- The schema being its own object, the load reports of #9 and of 0.34 #8, and the expected
  result type and context path of 0.34 #4 and #6, which are more declarations of the service,
  need no new kind of object.
- The Python package still leaves the schema and the language service out. It would carry them
  the same way, a schema and a service as objects and JSON-shaped answers, when asked.

## Alternatives

- **One call that takes the schema's text and the expression's and replies with the
  diagnostics.** No object and no lifetime, but the schema is read again for each expression:
  an application that checks every stored expression against one schema reads it once per
  expression.
- **A second library, or a Cargo feature, for hosts that want the schema.** Decision 1: the
  size does not justify the second artifact.
- **Diagnostics as values, or through upcalls.** A decoder and an encoding more for a reply of
  a few hundred bytes; upcalls cost 25 ns each and bring the stack room of every call back into
  Java.
- **The variables' schemas as JSON Schema text inside the declarations**, as the WebAssembly
  package takes them. The Java `Schema` would have to keep its text to pass it again, and
  from #9 on a schema is no longer one text.
- **The whole language service now.** Decision 2: no Java application asks for completion, and
  each operation is surface to keep.

## Results: the exports (0.33 #3)

Five exports: `jlens_schema_new`, `jlens_schema_free`, `jlens_service_new`,
`jlens_service_free` and `jlens_service_diagnostics`, in `jlens-capi`'s `schema` module.

- **A service copies its schemas** (decision 4). Sharing would put the schema behind an `Arc`
  in `jlens-schema`'s `LanguageService`, a change to that crate's API for every user; the copy is
  made once per service and costs about a sixth of what loading the schema does. On the
  repository's largest schema fixture (`fixtures/cli/order.schema.json`, 2.4 KB): the load 22 µs,
  the copy 3 µs. On generated schemas of 200 and 2,000 fields (28 KB and 224 KB of text): the
  load 0.78 and 6.5 ms, the copy 0.13 and 1.1 ms (Ryzen, the release profile, the best of seven
  runs).
- **The handle of a variable's schema is a JSON number**, read as an unsigned 64-bit integer;
  a string or a fraction fails with `U2005`. A host whose integers are signed writes it
  unsigned (`Long.toUnsignedString` in Java). The declarations must be an object: an array fails
  too.
- **A handle named inside the declarations is checked after their form**, as it cannot be read
  before: declarations that are not JSON fail with `U2001` even when a variable would name a
  freed schema.
- **Measuring the copy found the diagnostics quadratic in an object's fields**: each field step
  listed the fields of its context, and the list was built by comparing every name with every
  other. One step against the object of 2,000 fields took 3.3 ms; with a look-up by name it
  takes 17 µs, the same as with 30 fields. A field the schema does not have still reads every
  name, for its suggestion (1.4 ms there). The diagnostics of `1` take about 17 µs with any
  schema, a fixed cost whose cause was not looked for; roadmap 0.34 #10 measures the
  diagnostics.
- **The size.** The Windows library is 3,346,432 bytes, 7.1% over 0.32.0 (1,258,732
  compressed, 7.8%), where the probe had measured 5.2% and 5.9%: the exports read the
  declarations and write the diagnostics as JSON, which the probe did not. Decision 1 stands;
  roadmap 0.33 #11 measures the library and the JAR as released. No evaluation export changed.
## Results: the Java package (0.33 #4)

`Schema`, `LanguageService` with its builder, and the `Diagnostic` record with its `Severity`.

- **A function is declared by its name and its signature alone.** The C API also takes `params`
  and `summary`, which only signature help and completion show, and those do not cross
  (decision 2): the Java builder has no place for them until they do.
- **The builder writes a variable's schema as its handle**, with `Long.toUnsignedString`, and
  keeps the schemas reachable until the service is made. A schema closed before `build()` throws
  an `IllegalStateException`, as any closed object does.
- **The list of diagnostics cannot be changed**, and a reply that is not in the form of
  decision 6 is a bug of the library, reported as an `IllegalStateException`.
- No export changed and the library is the one of #3.

## Results: what an expression uses, at every end (0.33 #7)

Two exports: `jlens_service_uses` and `jlens_uses_changes`.

- **`jlens_service_uses` takes the expression's handle**, not its text and not its reads: the
  C API works an expression's reads out once, when it compiles it, and Java has no bindings
  that would change them. The WebAssembly package takes the reads, as `reads(bindings)` there
  knows which functions do not read their context.
- **`jlens_uses_changes` takes two buffers** and no handle (decision 6). Text that is not JSON
  fails with `U2001`; JSON that is not an answer with `U2005` and the token `uses`. An array
  where an object is expected is refused: `Uses::from_json`, in `jlens-schema`, is the one
  reader of an answer that every end uses, as the derived one takes an array for a struct's
  fields in order.
- **In Java the answers are records**: `Uses` and `Use`, and `UsesChanges` with `PathChanges`,
  `UseChanges`, `Change` and `EnumChange` nested in it. The two sides of a change are
  `before()` and `after()`, `new` being a word of the language; the JSON form keeps `old` and
  `new`. A value of an enumeration is what the package's JSON reader gives (a `BigDecimal` for
  a number). `Uses.changes` writes both answers as JSON and reads the reply, so the comparison
  is the one of the other packages; an answer made by hand with a type JSON Schema does not
  have fails there with `U2005`. Java sorts a map's names by UTF-16 code unit and Rust by
  UTF-8 byte, so the text of an answer written by Java can differ in order from the C API's
  for names beyond the Basic Multilingual Plane: answers are compared by `changes` or
  `equals`, not by their text.
- **The same JSON from each**: `fixtures/uses/cases.json` holds two versions of a schema, of
  the variables and the host functions, and for nine expressions the answer with the first
  version and the changes with the second. The Rust crate, the C API, the Java package, the
  WebAssembly package (in Node.js and in the browser) and the CLI each answer them, and each
  test compares its answers with the file's.
- **The size.** The Windows library is 3,507,200 bytes, 12.2% over 0.32.0 (1,321,041
  compressed, 13.1%), and 4.8% over the library of #3: the answers of #5 and #6, read and
  written as JSON, add 161 KB, as much as the schema reader and the typing did. Decision 1
  estimated 5 to 6% before #5 and #6; roadmap 0.33 #12 measures the library and the JAR as
  released, and looks at what of these 161 KB the JSON forms cost.

## Results: a host function reads only its arguments (0.33 #11)

The results of #7 said that Java has no bindings that would change an expression's reads. It
has: a host function of the C API receives its arguments and nothing else, and the bindings
mark it so (`HostFunction::without_input`), but the reads worked out when an expression is
compiled know no function, and take the call of one as reading its context whole. An
application whose rules call its own functions then got the whole input as used, and every
change of the schema as a change for every rule.

- **`jlens_service_uses` takes the functions declared to the service as reading their
  arguments alone**, and the context a `-` of their signature may take. The service already
  knows them, so the export keeps its two handles and takes no bindings: what an expression
  uses is a question about the schema and the declarations, asked where no function is at
  hand to run. `LanguageService::uses_of`, in `jlens-schema`, is that answer, and the CLI gives
  it too, so that the same configuration gives the same answer at both ends. A function that
  is not declared still reads its context whole, as does one declared with a signature that
  is not one.
- **`jlens_reads` takes the bindings**, 0 for none, and Java has `Expression.reads(Bindings)`.
  `reads()` is unchanged. The evaluations that cut their own input went by it too, as an
  expression is compiled once for any bindings, and cutting by the bindings of each
  evaluation would work the reads out at each one; #12 measured it and cuts by the bindings,
  below. An application that parses a document for several rules gives `Document.fromJson`
  the reads with its bindings.
- **The shared cases** have `$rate(1)`, which uses nothing, `$rate(items[0].price)`, which uses
  that field, and `$other(items[0].price)`, a function that is not declared, which uses the
  whole input. The WebAssembly package answers them with `reads(bindings)`, the bindings
  holding the declared functions.
