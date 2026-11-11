# Roadmap

What comes next for jlens, and what is not planned. Finished plans, one paragraph per release,
are in the [roadmap history](roadmap-history.md), and every change in the
[changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md). A plan's table lists its
items, one row each: what it delivers, when it is done, and its state.

## Priorities

Three principles order the work after 0.2:

1. **The core is a library for custom integrations.** Applications embed `jlens-core` in their
   own pipelines. It must stay small and decoupled, let the application set every limit, never
   panic, and change its API only on purpose.
2. **The CLI is a tool for automation and AI agents.** It never prompts, it gives the same
   output for the same input, and it has one documented machine-readable contract: what goes to
   standard output, what goes to standard error, and what each exit code means.
3. **Editors come after.** The language server is done; the VS Code extension is 0.15.

For ad-hoc JSON queries, `jq` remains the usual tool. jlens is for JSONata that will run
somewhere else, such as a rules engine or a jsonata-js service. It checks those expressions,
tests them and explains their results before they ship.

## 0.33: expressions checked again when their schema changes

An application may store expressions written against a schema that another system sends it. When
a new version of the schema arrives, it has two questions about each expression: does it still
pass the diagnostics, and did anything it uses change, even if it still passes? A field that
went from a boolean to a string leaves `active = true` valid, and always false.

Today the first answer needs `jlens check` or the WebAssembly package, as the Java package
leaves the schema and the language service out. Nothing gives the second: `reads()` tells the
fields, and only the Rust crate tells what the schema says of each (`Schema::shape_at`).

0.33 brings the schema and the diagnostics to the Java package, through the C API, and adds what
an expression uses of its schema, in a form that two versions of a schema are compared by. A
schema that fails to load leaves nothing to check, so the two kinds that jlens refuses whole
today, a recursive schema and one that refers to another document, load too.

| # | Item | Done when | Status |
|---|---|---|---|
| 1 | The dependencies at their newest versions and the upstream watch, before the rest | Step 1 of the release checklist; stacker, `regress` and jsonata-js checked | done, on 2026-11-10: nothing to move, as 0.32.0 was released the same day. Cargo (here and in `fuzz/`), npm, the Python lock, both `pom.xml` files, the Maven Wrapper and the actions are at their newest stable versions, with no advisory and no Dependabot proposal open; the Maven plugins' 4.0 betas and Maven 4.0's release candidate are left for their releases. Upstream: stacker#155 and #156 still open, `regress` has nothing after `95c0404`, jsonata-js is 2.2.2 with nothing after `c5be116` |
| 2 | ADR 0023: the schema and the language service through the C API and in the Java package | The ADR accepted: what crosses (a schema, the diagnostics, #5 and #6; whether completion, hover and signature help do), in which form, the objects' lifetimes and threads, and what `jlens-schema` adds to the library's size, measured | done, on 2026-11-10: [ADR 0023](../adr/0023-schema-through-the-c-api.md). A schema and a service are objects by handle, shared by every thread; diagnostics, #5 and #6 cross as JSON text with UTF-16 spans; completion, hover, signature help and schema inference stay out. `jlens-schema` adds 5.2% to the Windows library (5.9% compressed) |
| 3 | `jlens-capi`: a schema from JSON Schema text, a language service with the variables and host functions declared, and its diagnostics | As ADR 0023 says; the C API's tests, one asserting that both are `Send` and `Sync`; whether a service shares its schema or copies it, measured on the largest fixture; nothing but the size paid by a host that does not use them | done, on 2026-11-10: five exports, `jlens_schema_new` and `_free`, `jlens_service_new` and `_free`, `jlens_service_diagnostics` ([ADR 0023](../adr/0023-schema-through-the-c-api.md#results-the-exports-033-3)). A service copies its schemas: the copy costs a sixth of the load, 3 µs on the largest fixture. Measuring it found the diagnostics quadratic in an object's fields, now a look-up: a step against 2,000 fields 3.3 ms → 17 µs. The Windows library is 7.1% larger than 0.32.0 (7.8% compressed), against the 5.2% of the probe |
| 4 | Java: `Schema` and `LanguageService` | Diagnostics as records, their spans in UTF-16 offsets, their codes those of the other packages; JUnit; the Java guide, with "What the package leaves out" updated | done, on 2026-11-10: `Schema.fromJson`, `LanguageService.of` and its builder (the input's schema, the variables with a schema or none, the host functions with a signature or none) and `diagnostics`, a list of `Diagnostic` records ([ADR 0023](../adr/0023-schema-through-the-c-api.md#results-the-java-package-033-4)); a function's `params` and `summary`, which only completion and signature help show, are not declared from Java. The guide has "Checking expressions against a schema" |
| 5 | What an expression uses of its schema | For each field the expression reads: its path and what the schema says of it (types, enumeration and whether it is closed, domain, format, whether it and the fields on its way are required). Where the typing follows the value, the fields it names, those of a predicate included; where it does not (`*`, `**`, `$eval`), the value whole, as `reads()` gives it. One JSON form; property tests; the schema guide | done, on 2026-11-10: `LanguageService::uses` takes an expression's `reads()` and gives `Uses`, a map from each path read (`items[].type`, as `Schema::shape_at` reads it) to what the schema says there, for the input and for each variable ([the schema guide](../guides/schema.md#what-an-expression-uses-of-its-schema)). It follows the analysis of the reads, not the typing of the diagnostics, as an answer that misses a field is silent. Each field on the way has its own path and its own `required`, so that a change shows where it was made; a field the schema does not declare is marked unknown. A path takes a name with a dot between backquotes |
| 6 | What changed for an expression between two schemas | Two answers of #5 compared: the paths added, removed and changed, and what changed in each. Equal answers mean that nothing the expression uses changed. Tests with a type, an enumeration, a domain and `required` changed | done, on 2026-11-10: `Uses::changes` compares the answer kept with that of the new schema and gives `UsesChanges`, for the input and for each variable: the paths added and removed, with what the schema says of each, and the paths changed, with each member that changed as its old and its new value ([the schema guide](../guides/schema.md#what-changed-between-two-schemas)). `{}` means no change. Types and enumerations are compared as sets, so their order alone is not a change, and an enumeration also gives the values added and removed. A field the expression names and the new schema drops is a path changed to unknown; one under a value read whole is a path removed |
| 7 | #5 and #6 wherever the schema is: the Rust crate, the WebAssembly package, the Java package and the CLI | The same JSON from each, pinned by a test; #6 takes the two answers as JSON, with no schema or service at hand; `types.d.ts`; a CLI command with `--json`; the guides | done, on 2026-11-10: `jlens schema uses` and `jlens schema changes OLD NEW [--json]`; `LanguageService.uses(reads)` and `usesChanges` in the WebAssembly package; `LanguageService.uses(expression)`, `Uses` and `UsesChanges` records in the Java package, over two exports of the C API ([ADR 0023](../adr/0023-schema-through-the-c-api.md#results-what-an-expression-uses-at-every-end-033-7)). `fixtures/uses/cases.json` pins the answers of nine expressions against two versions of a schema, and each end's test compares its own with them. `Uses::from_json` is the one reader of an answer kept. The Windows library is now 12.2% over 0.32.0, the answers adding 161 KB |
| 8 | A recursive schema loads (`"$ref": "#"`, a definition that refers to itself); today `$ref nesting is too deep` | The shapes followed as deep as the expression goes, with no stack overflow and no endless walk: property tests; completion and `W0201` at any depth; under a value read whole, #5 lists the paths down to where the schema repeats | done, on 2026-11-10: a `$ref` that leads back to itself is kept as a reference by name to a definition of the schema, which the typing, `Schema::shape_at`, the overrides and what an expression uses follow ([ADR 0024](../adr/0024-recursive-schemas.md)); every other `$ref` is copied in place as before. Under a value read whole, what is under a definition is listed once, at the place nearest to that value. A variable is declared with its schema, as its definitions go with it. The property tests load schemas whose places refer to each other at random |
| 9 | A `$ref` to another document; today `cannot resolve $ref` | jlens never fetches: the application gives the documents by URI. A document it did not give leaves its place unknown, with nothing warned under it, and the load reports it and no longer fails whole. `$id` and `$anchor` resolved, or reported the same way. A load bounded by the size of its schema: a `$ref` that is not recursive is copied in its place, so definitions that each refer twice to the next double at each level (found by #8: 20 levels, 1.9 KB, load in 2.2 s, and 30 would not end); such a target is kept once, as a definition is (ADR 0024), or the load fails with a message | done, on 2026-11-10, in the Rust crate: `SchemaLoader` takes the documents, each with its URI, and gives the schema with a `LoadReport` ([ADR 0025](../adr/0025-references-to-other-documents.md); [the schema guide](../guides/schema.md#a-schema-in-several-documents)). A `$ref` is resolved against the `$id` of the schema it is in or the URI of its document, to a JSON Pointer or an `$anchor`. One that cannot be followed is a reference to a name the schema does not define, which was already read as unknown (ADR 0024), whatever is written beside it; the report gives its place, its text, the URI it resolves to and the document to give. `Schema::from_json_schema` is the load with no other document, so every package loads such a schema, with the place unknown; the documents and the report reach them in #10. A load reads at most 16 times the values of its documents (no less than 100,000): past that, the targets copied most are kept as definitions. The 20 levels load in 3 ms, as 400 do; the worst case, just under the budget, in 40 ms |
| 10 | The documents of a schema and the report of its load, wherever a schema is read | #9 serves the Rust crate alone: elsewhere a `$ref` to another document loads with its place unknown and nothing says so. The C API takes the documents by URI with the schema and gives the report as JSON (ADR 0023 amended); Java: `Schema` made with its documents, and the `$ref`s it did not follow; the WebAssembly package the same, with `types.d.ts`; the CLI and the language server read a `$ref` to a file beside the schema's, never from the network, and `jlens check` and the server's problems print the report; a schema of two documents and its report among the shared cases; the guides | done, on 2026-11-10: one JSON form of the report at every end ([ADR 0025](../adr/0025-references-to-other-documents.md#results-the-documents-and-the-report-at-every-end-033-10); [the schema guide](../guides/schema.md#a-schema-in-several-documents)). `jlens_schema_new` takes the schema's URI and its documents, each in a buffer, and `jlens_schema_report` gives the report; `Schema.loader()`, `unresolved()` and `missingDocuments()` in Java; a fourth argument of `LanguageService` and `schemaReport` in the WebAssembly package. The CLI and the language server read the file at the same relative path from the schema's as between the two URIs, with or without an `$id`, and a variable's schema refers to files from the folder of `jlens.json`; what is not followed is a warning on standard error or a message, and changes no exit code. `SchemaLoader::load_asking` is the loop behind it, which asks for no more than 256 documents. `fixtures/uses/cases.json` has the schema of two documents, under `references`. The Windows library is now 16.4% over 0.32.0 |
| 11 | A host function that reads only its arguments, in what an expression uses | In the C API and the Java package a host function receives its arguments and nothing else, but the reads of an expression take the call of a function they do not know as reading its context whole: `$describe(items[0])` marks the whole input as used, and every change of the schema as a change for the expression. `jlens_service_uses` takes the functions declared to the service as reading their arguments alone (`reads_with`), as the WebAssembly package does with the functions of the bindings: the call then uses `items` whole and nothing else. Whether `Expression.reads()` in Java does the same for an expression's bindings, decided and written in the Java guide; the shared cases, with a function given a field; the guides | planned |
| 12 | The plan's code checked whole, and the release's measure | The fuzz workflow on the final code; a recursive schema among the shared cases of what an expression uses (`fixtures/uses/cases.json`), which #8 tests in the Rust crate alone; the size of the C API's library and of the JAR against 0.32.0, and what the JSON forms of #5 and #6 cost of the 161 KB they added (#7); the performance page's tables from the M1 | planned |
| 13 | Release 0.33.0 | The release checklist; the channels checked | planned |

## 0.34: the types the diagnostics know

The diagnostics check a function's arguments (`W0205`), the fields (`W0201`) and the values
compared with an enumeration (`W0206`). They do not check the operands of an operator, and they
do not know the type of an expression's result. With `total` a string in the schema, `total >
1000` fails with `T2009` when it runs and `total + 1` with `T2001`; with `active` a string,
`active = true` is always false. None of the three is warned about.

An application whose expressions must give a boolean, as the conditions of a rules engine must,
learns that one does not only by evaluating it against the right document: `$contains(notes,
"urgent")` is undefined for the document without `notes`. Most conditions are comparisons joined
by `and` and `or`, which always give a boolean, so the typing can prove most of them without a
document.

0.34 makes the typing check operands, know whether a value may be undefined, and compare the
result with the type the application expects. The other items are the places where the typing
stops today: the parent operator, an expression that will run as the body of a step, and a
schema's objects that are open to other fields.

| # | Item | Done when | Status |
|---|---|---|---|
| 1 | The dependencies at their newest versions and the upstream watch, before the rest | As 0.33 #1 | planned |
| 2 | Operands whose types the operator cannot take | New `W02xx` warnings, only where the types are known, as for `W0205`: an order comparison between a string and a number, or with a value that has no order (`T2009`, `T2010`); arithmetic on what is not a number (`T2001`, `T2002`); a field compared by `=`, `!=` or `in` with a literal of another type, which is always false. A field that may be absent passes. No warning on the suite's expressions that evaluate without a type error; `gen-docs`; the schema guide | planned |
| 3 | A value that may be undefined, in the typing | The typing tells a value that is always there from one that may be undefined: an optional field and what is under it, a filter, a function that gives undefined for an undefined argument (from the catalog). Nothing is warned by this alone; tests | planned |
| 4 | The type an expression's result is expected to have | Declared to the service (`LanguageService`, `jlens.json` by file pattern, the WebAssembly and Java packages). Three answers: always that type; never, a warning; maybe another type or undefined, a warning that names the operand that causes it. A path that gives one value or an array, as sequences do, told from an array constructor. The result's type also given to the application as a value; tests over the suite; the schema guide | planned |
| 5 | The parent operator typed | `%.` completes the fields of the parent, and `W0201` is reported under it; where the parent is not known, nothing is warned | planned |
| 6 | An expression typed as the body of a step | The service takes a context path (`items[]`): the expression is completed and diagnosed with an item as its context and the document as its parent, as when it is evaluated inside `items.( … )`, without the application wrapping the text and shifting every offset. The WebAssembly and Java packages, `jlens.json`; the schema guide | planned |
| 7 | An object open to other fields | An object with `additionalProperties` or `patternProperties` given as a schema, or with no `properties` at all, gets no `W0201` for a field it does not list, and `additionalProperties` gives such a field its type. An object with `properties` alone is warned about as now; the decision recorded in the schema guide | planned |
| 8 | A load that says what it did not read | A survey of the JSON Schema keywords jlens does not read (`if`/`then`/`else`, `dependentSchemas`, `not`, `prefixItems`, `nullable`, `$dynamicRef`…): each is read, ignored where that cannot cause a false warning, or reported by the load beside the schema, with its place, in the `LoadReport` of 0.33 #9, as a new member of the JSON form that every end gives since 0.33 #10 (the C API, `Schema` in Java, `schemaReport` in the WebAssembly package, the warnings of the CLI and the language server). The schema guide lists the keywords | planned |
| 9 | #2 to #8 in every place the service is: the language server, the VS Code extension, the WebAssembly package, the Java package and the CLI | The same diagnostics from each, on shared cases; `types.d.ts`; the guides | planned |
| 10 | The plan's code checked whole, and the release's measure | The robustness properties with `PROPTEST_CASES=100000` and the fuzz workflow on the final code; the time of `diagnostics` on the largest fixtures against 0.33.0, as it runs at each keystroke, with its fixed part (about 17 µs for `1`, found by 0.33 #3) and the suggestion of an unknown field, which reads every name; the performance page's tables from the M1 | planned |
| 11 | Release 0.34.0 | The release checklist; the channels checked | planned |

## In exploration: a regular expression engine of jlens's own

`jlens-regress` is a backtracker. A catastrophic pattern runs within the evaluation's limits
([ADR 0009](../adr/0009-bounded-regex.md)), but it still runs to them. Its positions are code
points, which jlens converts to UTF-16. On the engine's benchmark, V8 is 2 to 11 times faster.

An engine of jlens's own could find a match in linear time wherever the pattern allows it: an
automaton finds the match, and the backtracker is left only to extract its groups, or to run
the patterns that need it (backreferences, lookarounds). It would also have the interrupt and
UTF-16 positions built in, with no `unsafe`.

This is research, not a release. The work lives apart from `jlens-core`, and the corpus, the
differential test and the benchmark of 0.22 measure it. It ends in a decision: an ADR that adopts
the engine in place of `jlens-regress`, or drops it. Adopted, the swap becomes a release of its
own.

| # | Stage | Done when | Status |
|---|---|---|---|
| 1 | The linear-time path on the patterns it can take, with the interrupt and UTF-16 positions | Its share of the corpus's patterns and their answers, against V8's | |
| 2 | Every pattern: the backtracker for the rest | The corpus (19,108 cases) and `cargo xtask regex-differential` with 20,000 cases clean, or each difference recorded with its reason | |
| 3 | Speed and limits, against `jlens-regress` and V8 | The engine's benchmark and `interrupt_cost`; catastrophic patterns timed | |
| 4 | The decision | An ADR: the engine replaces `jlens-regress`, or is dropped, with the measurements | |

## Deferred: an API to keep, toward 1.0

After twenty releases, nothing says which parts of jlens are meant to last. `cargo
semver-checks` runs in CI, but before 1.0 it only asks for a new minor version, and
`jlens-core` exposes workspace plumbing (`stack`, `debug`, the AST's internals) next to its API.
This work decides what 1.0 will promise, breaks the API once to get there, and writes the
promise down. It takes the number of the next release when it starts. Applications built on jlens are separate projects: they use `jlens-core` as any
application does, and what they lack comes back here as general features.

**Deferred** until applications built on jlens have used it: what to keep stable is decided
from what they needed, missed or worked around, not guessed beforehand.

| # | Deliverable | Definition of done | Status |
|---|---|---|---|
| 1 | ADR 0015, what 1.0 promises: what is stable, hidden or `#[non_exhaustive]`, below | The ADR accepted, with the list of changes that #2 to #4 make | |
| 2 | `jlens-core` changes as ADR 0015 says, in one breaking batch | `cargo semver-checks` reports only the intended breaks; the changelog has a migration section; the crate guide's snippets updated | |
| 3 | `jlens-schema` and `jlens-lsp` changes as ADR 0015 says | As #2 | |
| 4 | The contracts outside Rust, as ADR 0015 says: the Wasm API, the Python package, the Java package, the JSON of audits and `jlens.json`; a test pins each, so that a change shows in review | Tests that fail on an unintended change of each contract | |
| 5 | A stability page in the book, from ADR 0015, and the rules for changing a stable item in `CONTRIBUTING.md` | `doc-check` green; the page linked from the introduction | |
| 6 | A source that fails with a JSONata code, below; a change to public traits, so it goes with #2's batch | The same input nested too deep fails with the same code from every input and package; a cycle stays `U1004` | |
| 7 | The release that breaks once. 1.0.0 follows when ADR 0015's criterion is met | The release checklist, with the migration notes in the release notes | |

**#1, what ADR 0015 decides.** Exploration: list the public items of `jlens-core`, `jlens-schema`
and `jlens-lsp` (rustdoc JSON or `cargo public-api`) and the contracts outside Rust: the Wasm API
(`types.d.ts`), the Python package, the CLI (ADR 0008), the JSON of audits and explanations,
`jlens.json`, the error codes. For each item: stable, hidden (`#[doc(hidden)]`, `pub(crate)`) or
`#[non_exhaustive]` where it may grow. What semver covers and what it does not (messages, `Display`,
`Debug`, performance, jsonata-js quirks), the MSRV policy, deprecations.

**#6, a source that fails with a JSONata code.** Found by 0.26 #9: `Source` and `Sources` fail with
a message, which the core always turns into `U1004`, so input nested more than 1,000 levels deep
fails with `U1001` as JSON text and with `U1004` as Java objects or a C API host's values, read as
the expression goes; the C API also gives one message for a cycle and for deep input, as it cannot
tell them apart. The error type of the reading methods carries a code (`U1004` by default, `U1001`
for nesting), and each source says which limit it met; in the C API, a node inside itself that the
host does not report (the Java `Reader` does) is seen as deep input. The WebAssembly package has the
same gap for its exceptions: it keeps the first one a level throws, which since 0.27 #6, where a
level may be read in case it is needed
([ADR 0010](../adr/0010-lazy-input.md#amendment-a-level-read-in-case-it-is-needed-027)), may not
be the level that stopped the evaluation. A change to public traits, so it goes with #2's batch. Done when:
the same input nested too deep fails with the same code from JSON text, a `Document`, Python,
JavaScript and Java values and a C API host; a cycle that the host or package detects stays `U1004`;
the exception rethrown in WebAssembly is that of the level that failed; tests in each package.

## Not planned

- **Asynchronous functions.** A host function that returns a promise fails with `U1002`, and
  jlens does not plan to change that. Supporting it needs an evaluator that can suspend, which
  would slow down every evaluation, and only WebAssembly would gain from it: a Rust host
  function can block. An application can fetch what its functions need before evaluating.
  Should real use call for it, the cheaper way is an `evaluateAsync` that stops at the first
  promise, awaits the pending ones and evaluates again with their results.
- **Validating documents against a schema.** `jlens-schema` reads a JSON Schema to help people
  write expressions: completion, hover and diagnostics of the expression. It does not check that
  a document follows the schema, which JSON Schema validators do. The diagnostics hold for
  documents that follow their schema; an application that cannot trust its documents validates
  them first, or checks the type of each result.
- **Compiling an expression to its host's code.** jsonata-jvm-compiler and jsonata2js translate
  an expression to Java or JavaScript and let the host's JIT compile it, which makes them faster
  where an expression does much work per item. Rust has no compiler at run time: compiling when
  the application is built would serve only expressions known then, and generating WebAssembly
  would help that package alone, where a Content-Security-Policy allows it. jlens lowers what
  each node costs in the core instead (0.32), which every package gains from.
