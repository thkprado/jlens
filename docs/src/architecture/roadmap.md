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

## 0.32: the tables on the M1, and what each node costs

From 0.32 on, the performance page's tables are measured on the Apple M1 alone, which 0.31 #7
found quieter (two comparisons within 1.3 points, against up to 3 on the Ryzen) and steady through
three hours. x86_64 keeps a check without tables, while the Ryzen is available, for a release
whose plan changed the hot path without measuring every such change on the Ryzen: `cargo xtask
ab` against the previous tag in every mode and `python.py ab` under Linux on it, about 30
minutes, investigating only what passes the floor. Its result is a section of the release's
`docs/notes/perf-X.Y.md`, not the performance page. A release is compared with an older one by
measuring the older tag on the M1, not by the Ryzen's tables.

The engines that compile an expression to their host's code (jsonata-jvm-compiler, jsonata2js)
are 3 to 5 times faster than jlens on most cases that do much work per item, about 10 on
`recursion`. A profile of 0.31.0
([`perf-0.32.md`](https://github.com/thkprado/jlens/blob/main/docs/notes/perf-0.32.md)) puts the
variable lookup at a tenth of `recursion` and next to nothing elsewhere. Most of the time goes to
what every node costs: the dispatch, the check of its result, and the guards (depth, steps,
cancellation, the stack), which alone take 10% of `recursion` and 5% of the shipping rule. Items
#3 to #6 lower that cost in `jlens-core`, so that every package gains. #8 and #9 come first, so
that each of them is measured against up-to-date dependencies, from #8's commit, and kept only
past the floor. If #3 and #4 together gain less than 5%, the series stops
there, and the performance page says the interpreter is near its limit.

| # | Item | Done when | Status |
|---|---|---|---|
| 1 | The performance page on the M1; after every item that changes the code | The release's measure item fills every table from the M1, states the Ryzen as an `ab` check only (needed for 0.32: `eval.rs` changed after its last run there, and no Python ran on x86), and folds "On an Apple M1" into the tables | planned |
| 2 | `filter + aggregate` with Python values on the M1 | Its 3 to 4% against 0.30.0, found by 0.31 #7, explained or removed | done: removed; the case is 9.5 to 9.7% faster than 0.30.0, and 0.31's cost was not found ([`perf-0.32.md`](https://github.com/thkprado/jlens/blob/main/docs/notes/perf-0.32.md#2-filter--aggregate-with-python-values-against-0300)) |
| 3 | Cheaper guards for each node: the stack check and `guardrails` | `cargo xtask ab` on the M1 and the Ryzen: `recursion` and the shipping rule faster past the floor, nothing slower; the limits' errors and the robustness properties as before; or reverted | done: one address compared and one flag read at each node; `recursion` 11 to 17% faster, the shipping rule 2 to 7% ([`perf-0.32.md`](https://github.com/thkprado/jlens/blob/main/docs/notes/perf-0.32.md#3-cheaper-guards-for-each-node)) |
| 4 | One `Rc<Frame>` fewer for each lambda call | As #3; the depth WebAssembly reaches by default (500) kept | done: a call or a block that binds takes a frame whose scope ended unshared; `recursion` 7 to 13% faster ([`perf-0.32.md`](https://github.com/thkprado/jlens/blob/main/docs/notes/perf-0.32.md#4-one-frame-fewer-for-each-lambda-call)) |
| 5 | Aggregates over a field on a path of their own: `$sum`, `$average`, `$max`, `$min` and `$count` of `x.f`, `$count(x[f = literal])` | As #3; their results and errors those of the general path; `cargo xtask differential` with 20,000 cases clean | done: chosen when the call is evaluated, the syntax tree as it was; `names over items` 7 to 50% faster, the shipping rule up to 4% ([`perf-0.32.md`](https://github.com/thkprado/jlens/blob/main/docs/notes/perf-0.32.md#5-aggregates-over-a-field)) |
| 6 | A lambda's parameters read by their place | As #3; `$eval` sees the variables of its block as before | done: a read in a lambda's body carries its parameter's index, which the calling frame confirms; `recursion` 5 to 9% faster; `Attrs` gains a private field, a breaking change, so the version is 0.32.0 ([`perf-0.32.md`](https://github.com/thkprado/jlens/blob/main/docs/notes/perf-0.32.md#6-a-lambdas-parameters-read-by-their-place)) |
| 7 | Every package after #3 to #6 and #10 | `python.py ab`, `cargo xtask java-compare ab` and `compare.mjs --pkg` against 0.31.0 | done, on the M1: Python and Java gain as the core does, `recursion` 23 to 30% faster, the shipping rule 5 to 22%; WebAssembly gained up to 5%, which became #16 ([`perf-0.32.md`](https://github.com/thkprado/jlens/blob/main/docs/notes/perf-0.32.md#7-every-package)) |
| 8 | The dependencies at their newest versions, before #3 | Step 1 of the release checklist; `cargo xtask ab` against 0.31.0 if the code built changes | done: only Jackson changed, in the Java comparison harness, so #3's base is 0.31.0 |
| 9 | Upstream, before #3: stacker#155 and #156, `regress` after `95c0404`, jsonata-js after `c5be116` | Each commit worth porting ported, each jsonata-js fix checked against jlens; the watch list updated | done, 11-08: nothing new; #3 keeps the musl workaround in `stack.rs` ([`perf-0.32.md`](https://github.com/thkprado/jlens/blob/main/docs/notes/perf-0.32.md#8-and-9-dependencies-and-upstream)) |
| 10 | A path step without the stall 0.27 found | As #3, on `path` and `names over items`; or the cause recorded and left | done: `Value`'s tag takes a word, not a byte, and every node's result gains; `recursion` 5 to 9% faster, the shipping rule 6 to 16%, `filter + aggregate` and `group-by` 5 to 17% ([`perf-0.32.md`](https://github.com/thkprado/jlens/blob/main/docs/notes/perf-0.32.md#10-a-values-tag-a-word-wide)) |
| 11 | An evaluation against documents with no input | `$` undefined as with `evaluate(None, ..)`; a test, the crate guide | done, already there: `Document::undefined()` since 0.27 |
| 12 | Maven Central's limit in the release checklist | The checklist reads the count on the Central Portal before the tag, and says what to do without room | done: [`CONTRIBUTING.md`](https://github.com/thkprado/jlens/blob/main/CONTRIBUTING.md#publishing-the-java-package); without room, the Java package skips the version |
| 14 | A second sort in a tuple stream (`items#$i.($x)^(price)^(*)`): `T2008` in jsonata-js, a value in jlens; found by #5's differential | jsonata-js's result, or the difference in `compatibility.md` with a rule in `differential.rs` | done: jlens keeps its result, the documentation's; the [compatibility](../language/compatibility.md) row for an order-by after a tuple step now names a second order-by ([`perf-0.32.md`](https://github.com/thkprado/jlens/blob/main/docs/notes/perf-0.32.md#14-15-and-22-what-the-differential-found)) |
| 15 | A position after an order-by in a tuple stream (`items#$i^(p)#$j`): never bound in jsonata-js, and copied without a row in `compatibility.md`; found by #14 | `$j` bound, or the copy listed as a quirk reproduced on purpose; a test | done: `$j` bound, as the documentation says, with its row in `compatibility.md` |
| 16 | Why WebAssembly gains little from the series; found by #7 | A profile of `recursion` in Node; then a change that gains past the floor in WebAssembly and loses nothing natively, or the cause recorded and left | done: a call takes back a list of arguments, and `eval` is inlined on `wasm32`; in WebAssembly `recursion` 13 to 16% faster, the shipping rule 3 to 7% ([`perf-0.32.md`](https://github.com/thkprado/jlens/blob/main/docs/notes/perf-0.32.md#16-why-webassembly-gained-little)) |
| 17 | `T0410`'s `value` against a `Document`; found by #16's property tests | The same error from `evaluate` and from a `Document`; a test | done: an argument past the parameters of `$append`, `$reverse` or `$shuffle` is read whole; at 0.31.0 too |
| 18 | The plan's code checked whole, before #1 | `cargo xtask differential` with 20,000 cases and the fuzz workflow on the final code; tests of closures that outlive their call, `$eval` in a lambda and recursion through closures | done: nothing found ([`perf-0.32.md`](https://github.com/thkprado/jlens/blob/main/docs/notes/perf-0.32.md#18-the-plans-code-checked-whole)) |
| 19 | A tuple step that shares the tuple before it | Where the 1,237 allocations of `tuples` go, counted first; the change only if the copies of the tuples before are the largest part | done: stopped; the largest part is the 600 frames a filter and a group-by make, which is #20 ([`perf-0.32.md`](https://github.com/thkprado/jlens/blob/main/docs/notes/perf-0.32.md#19-where-the-allocations-of-tuples-go)) |
| 20 | The frames of a tuple stream kept for reuse; found by #19 | As #3 on `tuples` and the tuple cases; a frame that a closure or a thunk still holds not reused | done: `tuples` from 1,243 allocations to 539 and 10 to 13% faster, a sort in a tuple stream 14 to 18%, `$map` over objects 7 to 17% ([`perf-0.32.md`](https://github.com/thkprado/jlens/blob/main/docs/notes/perf-0.32.md#20-the-frames-of-a-tuple-stream-kept-for-reuse)) |
| 21 | A cycle through a transform's copy, kept after the evaluation (`x ~> \| $ \| {'g': function(){1}} \|`); found by #20, at 0de804f too | The memory returned when the evaluation ends, with the result unchanged; a test | done: an object into which an update stores a function has its fields cleared when the evaluation ends; a test counts what each case leaves alive ([`perf-0.32.md`](https://github.com/thkprado/jlens/blob/main/docs/notes/perf-0.32.md#21-a-cycle-through-a-transforms-copy)) |
| 22 | Two divergences of #20's differential: nested bindings in an object constructor's key and value, and an order-by over a block's result holding `[]` | The cause of each found; jsonata-js's result, or the difference in `compatibility.md` with a rule in `differential.rs` | done: both are jsonata-js quirks already in `compatibility.md`, met in a new place, and jlens keeps its results; both rows widened ([`perf-0.32.md`](https://github.com/thkprado/jlens/blob/main/docs/notes/perf-0.32.md#14-15-and-22-what-the-differential-found)) |
| 13 | Release 0.32.0 | The release checklist, with room on Maven Central or the Java package skipping 0.32.0 (#12); the channels checked | planned |

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
| 1 | The dependencies at their newest versions and the upstream watch, before the rest | Step 1 of the release checklist; stacker, `regress` and jsonata-js checked as in 0.32 #9 | planned |
| 2 | ADR 0023: the schema and the language service through the C API and in the Java package | The ADR accepted: what crosses (a schema, the diagnostics, #5 and #6; whether completion, hover and signature help do), in which form, the objects' lifetimes and threads, and what `jlens-schema` adds to the library's size, measured | planned |
| 3 | `jlens-capi`: a schema from JSON Schema text, a language service with the variables and host functions declared, and its diagnostics | As ADR 0023 says; the C API's tests; nothing paid by a host that does not use them | planned |
| 4 | Java: `Schema` and `LanguageService` | Diagnostics as records, their spans in UTF-16 offsets, their codes those of the other packages; JUnit; the Java guide, with "What the package leaves out" updated | planned |
| 5 | What an expression uses of its schema | For each field the expression reads: its path and what the schema says of it (types, enumeration and whether it is closed, domain, format, whether it and the fields on its way are required). Where the typing follows the value, the fields it names, those of a predicate included; where it does not (`*`, `**`, `$eval`), the value whole, as `reads()` gives it. One JSON form; property tests; the schema guide | planned |
| 6 | What changed for an expression between two schemas | Two answers of #5 compared: the paths added, removed and changed, and what changed in each. Equal answers mean that nothing the expression uses changed. Tests with a type, an enumeration, a domain and `required` changed | planned |
| 7 | #5 and #6 wherever the schema is: the Rust crate, the WebAssembly package, the Java package and the CLI | The same JSON from each, pinned by a test; `types.d.ts`; a CLI command with `--json`; the guides | planned |
| 8 | A recursive schema loads (`"$ref": "#"`, a definition that refers to itself); today `$ref nesting is too deep` | The shapes followed as deep as the expression goes, with no stack overflow and no endless walk: property tests; completion and `W0201` at any depth | planned |
| 9 | A `$ref` to another document; today `cannot resolve $ref` | jlens never fetches: the application gives the documents by URI. A document it did not give leaves its place unknown, with nothing warned under it, and the load reports it and no longer fails whole. `$id` and `$anchor` resolved, or reported the same way | planned |
| 10 | The plan's code checked whole, and the release's measure | The fuzz workflow on the final code; the size of the C API's library and of the JAR against 0.32.0; the performance page's tables from the M1 | planned |
| 11 | Release 0.33.0 | The release checklist; the channels checked | planned |

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
| 8 | A load that says what it did not read | A survey of the JSON Schema keywords jlens does not read (`if`/`then`/`else`, `dependentSchemas`, `not`, `prefixItems`, `nullable`, `$dynamicRef`…): each is read, ignored where that cannot cause a false warning, or reported by the load beside the schema, with its place. `jlens check` prints the report; the schema guide lists the keywords | planned |
| 9 | #2 to #8 in every place the service is: the language server, the VS Code extension, the WebAssembly package, the Java package and the CLI | The same diagnostics from each, on shared cases; `types.d.ts`; the guides | planned |
| 10 | The plan's code checked whole, and the release's measure | The robustness properties with `PROPTEST_CASES=100000` and the fuzz workflow on the final code; the time of `diagnostics` on the largest fixtures against 0.33.0, as it runs at each keystroke; the performance page's tables from the M1 | planned |
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
