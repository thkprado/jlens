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
| 1 | The dependencies at their newest versions and the upstream watch, before the rest | Step 1 of the release checklist; stacker, `regress` and jsonata-js checked | done, on 2026-11-11: nothing to move, as 0.33.0 was released the same day. Cargo (here and in `fuzz/`), npm, the Python lock, both `pom.xml` files, the Maven Wrapper and the actions are at their newest stable versions, with no advisory and no Dependabot proposal open. Upstream: stacker#155 and #156 still open, `regress` has nothing after `95c0404`, jsonata-js is 2.2.2 with nothing merged after `c5be116`. Four of its new pull requests fix bugs of 2.2.2: jlens already gives the fixed answer of irregular grouping in `$formatNumber` (#854) and the position of an error under `and` and `or` (#857), and has the other two, now #2 |
| 2 | Two date bugs of jsonata-js 2.2.2, found by #1 | `$fromMillis` writes fractional seconds (`[f]`) as the digits of the fraction, as the picture rules give: `[f01]` of 1,033 ms is "03", where both engines write "33" (jsonata-js#853, pull request #855). A negative offset with minutes (`-0330`) is no longer an hour off in `$fromMillis` and `$toMillis`: `$fromMillis(0, '[H]:[m]', '-0330')` is "20:30", not "19:30" (jsonata-js#852, #856). Both in `compatibility.md`, with a narrow rule in the differential test; the pull requests' cases pass, or a difference from them is recorded | done, on 2026-11-11: the 53 cases of the two pull requests pass, kept in `jlens-core`'s tests with their commits (`dates_fixed_upstream.json`); jlens follows them also where they go beyond the issues, in what `$toMillis` reads (`-00:30`, `-330` with `[Z001]`, minutes after `[Z01]`) and in widths and separators of `[f]`. The differential test now draws `-0330` and `[f01]`, and counts both as a dropped quirk; 20,000 cases, none differ. The date-time page has an example of each |
| 3 | Operands whose types the operator cannot take | New `W02xx` warnings, only where the types are known, as for `W0205`: an order comparison between a string and a number, or with a value that has no order (`T2009`, `T2010`); arithmetic on what is not a number (`T2001`, `T2002`); a field compared by `=`, `!=` or `in` with a literal of another type, which is always false. A field that may be absent passes. No warning on the suite's expressions that evaluate without a type error; `gen-docs`; the schema guide | done, on 2026-11-11: `W0207` (a side with no order), `W0208` (strings and numbers compared), `W0209` (a side of arithmetic not a number) and `W0210` (a literal of another type, unless the left side is a literal too or has a declared enumeration, which keeps `W0206`). An array stands for its items, as for `W0205`; arrays of arrays are not checked. Over the suite, one expected warning, kept in its test with the reason: `$type(data) = 'number' and data > 10`, whose right side would fail, says the case, and which the typing cannot see guarded, as it does not read `$type`. The operand of a negation that is not a number gets `W0211` (`D1002`) |
| 4 | A value that may be undefined, in the typing | The typing tells a value that is always there from one that may be undefined: an optional field and what is under it, a filter, a function that gives undefined for an undefined argument (from the catalog). Nothing is warned by this alone; tests | done, on 2026-11-11: each type carries the span of the operand that may make it undefined. Besides the three causes, a step over an array that may be empty, a value that may not be an object, and the arrays of several items flattened into an empty sequence (`items.tags`), as the engine gives them; `$sum(items.price)` is undefined where `items` is empty. Arithmetic and order comparisons pass undefined on, `=`, `in`, `and`, `or` and `&` never give it; a condition without `else`, `?:` and `??` as they choose. Built-ins: a table in `typing.rs` over the whole catalog, checked by a test, each row probed against the engine. The unit tests evaluate every case on documents that follow their schema: what the typing says is always there is defined on each, and what it says may be undefined is undefined on one, but for 8 cases where it cannot know (a constant index, `$lookup` of a required key). For #5: built-ins return no type yet (`$length` is a scalar, not a number), which their signatures give |
| 5 | The type an expression's result is expected to have | Declared to the service (`LanguageService`, `jlens.json` by file pattern, the WebAssembly and Java packages). Three answers: always that type; never, a warning; maybe another type or undefined, a warning that names the operand that causes it. A path that gives one value or an array, as sequences do, told from an array constructor. A built-in's result has the type its signature gives (#4 found them scalars of no type). An array may be empty, which makes `$sum(items.price) > 100` undefined, unless the schema says it is not: the load reads `minItems`, found by #4, and inference gives `minItems: 1` to an array no sample had empty, as it gives `required`, marked as inferred so it can be reviewed and overridden (decided by the user on 2026-11-11). An optional field of the items, a filter and nested arrays flattened still may make the result undefined. The result's type also given to the application as a value; tests over the suite; the schema guide | done in the Rust crate, on 2026-11-11; declaring it in `jlens.json`, the WebAssembly and the Java packages moves to #12, with the other ends. `LanguageService::with_result_type` adds `W0212` (never), `W0213` (another type too) and `W0214` (undefined, at its cause); `result_type` gives a `ResultType` (types, the spans that make it undefined or a sequence) and `ResultType::check` answers `Always`, `Never`, `Maybe` or `Unknown`. The typing tells a sequence (`Ty::sequence`: one value, or several in an array) from an array (`Ty::form`), and a value computed from an unknown one is unknown, so that no condition is warned about without a schema. Over the suite, 1,047 of 1,401 results are typed and 805 proved always defined, each checked against the case's result, as a property checks random expressions; the check found `$append` of an undefined argument, a filter of an array of arrays and tuple streams. On a sample of rule conditions, `W0214` fires on those with a filter or an optional field, and on no aggregate of a never-empty array |
| 6 | An operand that may be several values, found by #5 | An order comparison or arithmetic with a side that may be a sequence of several values fails (`T2010`, `T2001`): `items.qty > 1` is true for an order of one item and an error for two. `=` and `!=` compare the array, so `items.qty = 2` is false for two. A warning where the sequence's cause is known (its step), naming it, with what to write instead (`$sum`, `$count(items[qty > 1])`, `in`); none where a number filter keeps one item (`items[0].qty`). No warning on the suite's expressions that evaluate without error; `gen-docs`; the schema guide | done, on 2026-11-11: `W0215`, at the operand, names the step that makes the sequence (the step over an array, a filter that may keep several items, `*` over an object of more than one field, `**`, `$keys`) and suggests an aggregate, a filter or `in`; also for a negation and the left side of `in`, and `=`/`!=` only against one value. A field that holds an array is an array even of one item (`tags > 'a'` fails with `["a"]`, as jsonata-js does), which #3 had read as its items. The typing got more exact on the way: a filter of one value keeps it or nothing (`order[qty > 1]`), a number filter anywhere among a step's filters keeps one item (`items[qty > 1][0]`), `*` and `**` give sequences. A test builds 96 operands under five operators and checks the warning is given exactly where the operand is an array on a document of one or two items. The suite has no such operand among the cases that evaluate. Left alone: `&` writes an array as JSON text, and a range with an array fails (`T2003`) |
| 7 | The parent operator typed | `%.` completes the fields of the parent, and `W0201` is reported under it; where the parent is not known, nothing is warned | done, on 2026-11-11: the step that a `%` refers to binds, under the operator's label, the item it is evaluated against, as the evaluator does, and `%` reads it: in a later step, in the step's own filter (`items[%.id = 1]`), at any depth (`%.%`). The parent is always there, a value and not a sequence. Where the step is inside parentheses (`customer.(login).%`), the parent is unknown and nothing is warned. Over the suite's parent-operator cases that evaluate, no warning; the property tests draw `%` |
| 8 | An expression typed as the body of a step | The service takes a context path (`items[]`): the expression is completed and diagnosed with an item as its context and the document as its parent, as when it is evaluated inside `items.( … )`, without the application wrapping the text and shifting every offset. `%` in the body is the item's parent, as inside `items.( … )`, where alone the text fails with `S0217` (found by #7). The schema guide | done in the Rust crate, on 2026-11-11; declaring the path in `jlens.json`, the WebAssembly and the Java packages moves to #12, as #5's did. `LanguageService::with_context` takes the path as `Schema::shape_at` writes it (`orders[].items[]`, or `orders.items`); `jlens_core::parse_recover_as_body` parses a body whose `%` may refer to up to that many enclosing steps, and `Slot::enclosing` tells which, so the offsets stay the text's. A property test checks that a body's diagnostics are those of the same text inside `items.( … )`, shifted |
| 9 | An object open to other fields | An object with `additionalProperties` or `patternProperties` given as a schema, or with no `properties` at all, gets no `W0201` for a field it does not list, and `additionalProperties` gives such a field its type. An object with `properties` alone is warned about as now; the decision recorded in the schema guide | done, on 2026-11-11: `Shape::other_fields` holds `patternProperties` and `additionalProperties`, read, merged across `anyOf`, `oneOf` and `allOf`, and written back by the export; `additionalProperties: true` opens an object too, and patterns alone leave the other fields unknown. A name is matched with the regular expression engine of the evaluator, within a step budget: a name several patterns match, or whose match takes too long, has an unknown type. The field may be absent, for the result's type, and `*` over an open object gives a sequence. `shape_at`, hover and `uses` follow such fields; an object always empty in the samples of an inferred schema is open as well. A property test types expressions over open objects against documents that follow the schema |
| 10 | A load that says what it did not read | A survey of the JSON Schema keywords jlens does not read (`if`/`then`/`else`, `dependentSchemas`, `not`, `prefixItems`, `nullable`, `$dynamicRef`…): each is read, ignored where that cannot cause a false warning, or reported by the load beside the schema, with its place, in the `LoadReport` of 0.33 #9, as a new member of the JSON form that every end gives since 0.33 #10 (the C API, `Schema` in Java, `schemaReport` in the WebAssembly package, the warnings of the CLI and the language server). The schema guide lists the keywords | done, on 2026-11-12: read now, as each may allow what jlens would warn about: the fields of `then`/`else` (beside `if`), `dependentSchemas` and draft-07's `dependencies`, optional and at any depth; `prefixItems`, `additionalItems`, `unevaluatedItems` (a tuple with no schema for the rest has unknown items, unless `maxItems` closes it, which changes draft-07 tuples); `unevaluatedProperties`; OpenAPI's `nullable`. Left out, as they only narrow: `not`, the string, number and array bounds, `propertyNames`, `dependentRequired` and the annotations. Reported, with the place of their schema left unknown: `$dynamicRef`, `$recursiveRef` and a `type` that names no JSON type, as `unread` in `LoadReport` (`UnreadKeyword`), in the JSON form, in Java (`Schema.unread()`), in `types.d.ts`, and as warnings of the CLI and the language server; the shared fixture has one, so every package checks it. A property test types expressions against documents that follow a schema with these keywords |
| 11 | `anyOf` and `oneOf` read as one branch or another, found by #10 | The load merges the branches of all three combinators the same way, as `allOf` must be read: a field that one branch of `anyOf` requires is taken as always there, so the result's type claims it defined (`W0214` missed) where a document of the other branch lacks it, and a branch with no type (`{}`) adds nothing, so `anyOf: [{"type": "string"}, {}]` is a string and `c = 1` gets `W0210` and `W0212` for a value the schema allows. Under `anyOf` and `oneOf`, a field is required only where every branch requires it, and a branch that gives no type leaves the type unknown; `allOf` stays as it is. Unit tests, and a property test that types expressions against documents of each branch; the schema guide | done, on 2026-11-13: the branches of `anyOf` and `oneOf` are read apart and then joined: the types of any branch, a field required only where every branch requires it, the fields and items that several branches give joined the same way, at any depth, and an `enum` or a `const` the values of every branch, where each lists them (a branch of `null` alone lists `null`). A branch with no type leaves the type unknown, but for one with `const` or `enum`, typed by its values, and one that only refers to a definition; a branch `false` is skipped; what is beside the combinator still holds. The two cases of this row now get `W0214` and no `W0210`. A property test types expressions against a document of each branch; both new tests fail on the previous load. In the schema guide and the CHANGELOG ("Changed") |
| 12 | #3 to #11 in every place the service is: the language server, the VS Code extension, the WebAssembly package, the Java package and the CLI | The same diagnostics from each, on shared cases; the expected result type of #5 and the context path of #8 declared in each (`jlens.json` by file pattern, the WebAssembly and Java packages, the CLI) and `result_type` given as a value; `types.d.ts`; the guides | planned |
| 13 | The plan's code checked whole, and the release's measure | The robustness properties with `PROPTEST_CASES=100000` and the fuzz workflow on the final code; the time of `diagnostics` on the largest fixtures against 0.33.0, as it runs at each keystroke, with its fixed part (about 17 µs for `1`, found by 0.33 #3) the suggestion of an unknown field, which reads every name, and a field of an object with `patternProperties`, whose patterns each lookup compiles (found by #9); what `Uses::changes` adds to the C API's library, 55 KB to compare two answers (found by 0.33 #12); what the typing adds to the Wasm package, 583 to 602 KiB gzipped by #9, which raised the budget to 640 KiB; the performance page's tables from the M1 | planned |
| 14 | Release 0.34.0 | The release checklist; the channels checked | planned |

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
