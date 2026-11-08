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
three hours. x86_64 keeps a check without tables: while the Ryzen is available, each release runs
`cargo xtask ab` against the previous tag in every mode and `python.py ab` under Linux on it,
about 30 minutes, and investigates only what passes the floor. A release is compared with an
older one by measuring the older tag on the M1, not by the Ryzen's tables.

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
| 1 | The page on the M1 | The next release's measure item fills every table from the M1 (jlens and the controls; the other engines from 0.31's M1 runs until their versions change), states the M1 as the machine and the Ryzen as an `ab` check only, and folds the section "On an Apple M1" into the tables | planned |
| 2 | `filter + aggregate` with Python values on the M1 | Its 3 to 4% against 0.30.0, found by 0.31 #7, explained or removed | planned |
| 3 | Cheaper guards for each node: the stack check and `guardrails`; after #8 and #9 | `cargo xtask ab` against #8's commit in each mode, on the M1 and the Ryzen: `recursion` and the shipping rule faster past the floor, nothing slower; `D1011` and `U1006` where they were, the robustness properties with `PROPTEST_CASES=100000`; or reverted, with the reason in `perf-0.32.md` | planned |
| 4 | One `Rc<Frame>` fewer for each lambda call (6% of `recursion`); after #3 | As #3; the depth WebAssembly reaches by default (500) kept | planned |
| 5 | Aggregates over a field as nodes of their own, chosen when the expression is compiled: `$sum`, `$average`, `$max`, `$min` and `$count` of `x.f`, `$count(x[f = literal])` | As #3, on the cases that use them; their results and errors those of the general path; `cargo xtask differential` with 20,000 cases clean | planned |
| 6 | Variables in fixed slots, resolved when the expression is compiled; only if #3 to #5 leave the lookup above a tenth of `recursion` | As #3; `$eval` sees the variables of its block as before | planned |
| 7 | Every package after #3 to #6 | `python.py ab` and `cargo xtask java-compare ab` against 0.31.0, and `compare.mjs --pkg` for the WebAssembly package; what each gains or loses in `perf-0.32.md` | planned |
| 8 | The dependencies at their newest versions, before #3 | As step 1 of the release checklist says: `cargo update` here and in `fuzz/`, npm, both `pom.xml`, the Maven Wrapper and the actions; `cargo xtask ab` against 0.31.0 in each mode, what passes the floor explained | done: only Jackson, 2.23.0 in the Java comparison harness; both `Cargo.lock`s, npm, the Maven Wrapper and the actions were already at their newest, so the code built is 0.31.0's and no `ab` was run; #3's base is 0.31.0 |
| 9 | Upstream, before #3: stacker#155 and #156, which #3 depends on, `regress` after `95c0404`, jsonata-js's commits after `c5be116` and its version on npm | Each `regress` commit worth porting ported one at a time, or none; each jsonata-js fix checked against jlens; the watch list updated | planned |
| 10 | A path step without the stall 0.27 found: a `Value` written as a tag byte and 16 bytes, then read as 16 bytes at once, about 7 times an item | As #3, on `path` and `names over items`; or the cause recorded and left | planned |
| 11 | An evaluation against documents with no input: `evaluate_document_with` needs one, so variables bound to documents cannot go with an undefined `$`, as `evaluate(None, ..)` allows | An addition to the API, nothing changed; `$` undefined as with `evaluate(None, ..)`; a test, the crate guide; the packages checked for the same gap | planned |
| 12 | Maven Central's limit in the release checklist | Before the tag, the checklist reads the count on the Central Portal, and says what to do when there is no room | done: [`CONTRIBUTING.md`](https://github.com/thkprado/jlens/blob/main/CONTRIBUTING.md#publishing-the-java-package); without room, the Java package skips the version |
| 13 | Release 0.32.0 | The release checklist, with room on Maven Central or the Java package skipping 0.32.0 (#12); the channels checked | planned |

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
- **Compiling an expression to its host's code.** jsonata-jvm-compiler and jsonata2js translate
  an expression to Java or JavaScript and let the host's JIT compile it, which makes them faster
  where an expression does much work per item. Rust has no compiler at run time: compiling when
  the application is built would serve only expressions known then, and generating WebAssembly
  would help that package alone, where a Content-Security-Policy allows it. jlens lowers what
  each node costs in the core instead (0.32), which every package gains from.
