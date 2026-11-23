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

## 0.36: what adoption asks

jlens now has its first users outside its own repository, and they ask for things 35 releases did
not. A rule author follows a warning to the letter and gets another wrong expression. An
application in Java evaluates rules against large documents from threads that consume queues,
where each document holds its text beside its conversion, and it compares two versions of a
record through an index that every rule builds again. A crate uses `jlens-regress` alone, pinned to one version,
and skips text it cannot search with a step budget. And an adopter needs a version to pin,
releases it can plan for, and to know which parts of the API are meant to last.

From this plan on, an item names in its status the choices it made that the maintainer has not
reviewed, so that they are reviewed with the item and not a release later.

| # | Item | Done when | Status |
|---|---|---|---|
| 1 | The dependencies at their newest versions and the upstream watch, before the rest | The dependencies of step 1 of the release checklist; stacker, `regress`, jsonata-js (npm, main, #838) checked | done, on 2026-11-16. Cargo moved clap to 4.6.8 (`fuzz/` had nothing to move); two pins Dependabot does not see moved too: TypeScript 5.9.2 to 7.0.2 for the check of `types.d.ts` in CI, and `ovsx` 1.2.0 to 1.4.0 in the release workflow. npm, the Python build, both `pom.xml` files, the Maven Wrapper and the actions were at their newest stable versions, with no advisory and no Dependabot alert or proposal open; `@types/vscode` and `@types/node` stay at the floors of `engines.vscode`. Upstream: stacker#155 and #156 still open, `regress` has nothing after `95c0404` and no release after 0.12.0, jsonata-js is 2.2.2 on npm with main at `6d9d829` and #838 open at `662db05`. Its new reports are #9 |
| 2 | `W0215` that says which case it is, and shows the rewrite | The same code and severity, with a text for a side that is always an array (`tags = "x"` is always false) apart from one that may be several values; the hint writes the expression with `in`, sides swapped where the array is on the left (`"x" in tags`), and for `!=` a form that keeps what it gives for an absent field; tests; `gen-docs`; the schema guide | done, on 2026-11-17. An operand known to be an array and no sequence "is an array", with what follows under each operator ("which never equals one value", "which has no order", "not a number", for `in` "which is looked for whole among the items on the right"); the hint of `=` is `'a' in tags`, of `!=` `$not('a' in tags)` after `$exists` of each side whose type may be undefined (`$exists(items.qty) and $not(2 in items.qty)`); a test evaluates each rewrite against documents and finds the comparison's answer wherever the operand is not an array. To review: the template's detail `cause` became `reason`, so the error page shows `The {{{side}}} of {{{token}}} {{{reason}}}; {{{hint}}}`; the array texts cover every operator, not only `=`; the left side of `in` keeps "test each one in a filter", with no rewrite; no `$exists` for a required field (`[] != 'a'` and `$not('a' in [])` are both true); a side past 40 characters is written `…`, as in `$max(…)` |
| 3 | What a document costs a pool of threads | The native memory of a large document, at its peak while it is made and while it lives, measured in two cases: 1 to 16 threads that each make and evaluate their own documents, as consumers of a queue do, and one document evaluated by 1 to 16 threads, which the C API converts once per thread. Today the C API copies the host's buffer and keeps that text whole beside the conversion, even a cut one, so that another thread can convert it. The Java guide states the cost, and that it is native memory, which counts against a container's limit and not against the Java heap. Changes, if the numbers ask for them, decided with them: no copy of the host's buffer; a document for one thread, which drops its text once converted; an ADR if the core's `Document` must be shared between threads | done, on 2026-11-22. On 16 MB of JSON text (80,000 items), a whole document is 6.8 times the text in native memory (the conversion 5.8, the text 1), one cut to `items.price` 2 (1 and 1); each thread that makes its own holds that, and each further thread that evaluates a shared one a conversion more: 16 threads, 1.8 GB whole and 0.57 GB cut for their own, 1.5 GB and 0.3 GB for one shared. The C API's `jlens_document_take` takes the host's buffer for the text, which the Java package uses: the native peak of making one falls by the text, 150 to 133 MB whole, 56 to 49 MB cut. The Java guide states the cost; the numbers and the method are in `docs/notes/perf-0.36.md`. To review: a document for one thread, which would save the text (1 in 6.8 whole, 1 in 2 cut), is not made, as a virtual thread may evaluate it again on another carrier, which would have no text to convert; a core `Document` shared between threads (an ADR, atomic counts on the hot path) and a smaller conversion are left as candidates for 0.37; `jlens_document_new` stays for a host that keeps its buffer |
| 4 | An evaluation's result kept as a `Document` | In the core, the C API and Java: a result used as the input or a variable of later evaluations with no conversion, such as an index of a record's items by key built once for many rules; measured against the index built by each rule | planned |
| 5 | ADR 0015 proposed: what 1.0 will promise | Item #1 of the deferred plan below, with no change to the API: the inventory of public items and contracts, each stable, hidden or `#[non_exhaustive]`, and the gaps adopters logged (host function context, offset conversion, a diagnostic's details as data, a quick fix as data, such as `W0215`'s rewrite); the ADR left proposed for the maintainer | planned |
| 6 | `jlens-regress` versioned on its own | A version of its own, raised only when the crate changes; the release workflow publishes it only then; `CONTRIBUTING.md`; its README says how upstream fixes are ported | done, on 2026-11-22. The crate's manifest has its version, 0.35.0 until it changes, and the `regress` entry of the workspace asks for it; ADR 0020 has an amendment. `cargo xtask regress-version` packages the crate and compares it with the one crates.io has at that version: it fails when they differ, and tells the release workflow to publish only a version crates.io lacks; CI runs it, and `cargo semver-checks` now checks the crate too. The README says how ports are reviewed, tested and released. To review: the README, Cargo's records of the commit and the lockfile, and the manifest as written are not compared, so that a review that ports nothing publishes no version, and the README ships with the next one; before 1.0 a fix or an addition raises the patch (0.35.1), a break the minor; a change in how Cargo normalizes manifests would ask for a version with no change to the crate |
| 7 | Searches of UTF-16 text with a budget in `jlens-regress` | Interruptible forms of `find_from_utf16` and `find_from_ucs2`, as `find_from_interruptible`; tests with the `utf16` feature; released as #6 says | done, on 2026-11-22. `find_from_utf16_interruptible` and `find_from_ucs2_interruptible`, with the `utf16` feature, find the first match from a code unit as `find_from_utf16(text, start).next()` and `find_from_ucs2(text, start).next()` do, and stop when their interrupt says so, as `find_from_interruptible` does on a `&str`; from a low surrogate, the UTF-16 search starts at the pair's high surrogate. Tests stop catastrophic patterns and lookarounds, and compare every start of texts with surrogate pairs with the iterators; CI runs them (`--test interrupt` beside `--test tests`). The crate is 0.35.1, a patch, as `cargo semver-checks` agrees, and the README's row of upstream's `49bd5bf` (budgeted UTF-16 searches) names them. jlens's own searches go through `Searcher`, unchanged, so nothing on its path is measured. To review: the names; an interrupt, not upstream's count of steps; no `Searcher` over UTF-16 text, which would keep its storage between searches, and no anchored forms (upstream's `match_at_utf16`), until a user asks |
| 8 | A release cadence and tiers of channels | `CONTRIBUTING.md`: a minor release every two or three weeks, patches for fixes in between; the channels in two tiers (crates.io, npm, Maven Central, PyPI first; Homebrew, Scoop, ghcr.io and the binaries after), and what the release checks of each | planned |
| 9 | Three bugs of jsonata-js 2.2.2 reported after its main's `6d9d829`, found by #1 | `$formatInteger` of a number from 1e21 writes its digits, as `$formatNumber` already does in jlens: `$formatInteger(1e21, '#,##0')` is "1,000,000,000,000,000,000,000", where both engines write "1e,+21" (jsonata-js#863, pull request #864); the row of `compatibility.md` covers both functions, with a narrow rule in the differential test. With an exponent picture, the mantissa stays below 10^N, as XPath F&O 3.1 §4.7.5 says: `$formatNumber(10, '0.0e0')` is "1.0e1", not "10.0e0" (#865, pull request #866, which also turns `$formatNumber(10, '#.0e0')` into "0.1e2"): taken if #866 is approved when the item starts, otherwise the maintainer decides. A two-digit year that `$toMillis` reads (`"23"` with `[Y01]` is 1923, #862) stays as 2.2.2 gives it until jsonata-js chooses a rule | planned |
| 10 | The plan's code checked whole, and the release's measure | The robustness properties with `PROPTEST_CASES=100000` and the fuzz workflow on the final code; the performance page's tables from the M1 | planned |
| 11 | Release 0.36.0 | The release checklist; the channels checked | planned |

## Next: the jsonata-js release after 2.2.2

jlens follows jsonata-js 2.2.2, the version on npm. Its main branch has since merged fixes, and
0.35 took the two that change results, pull requests #841 and #861, ahead of the release that
will ship them: from a fixed commit of main, with rules in the differential test for the answers
2.2.2 does not give. It also took pull request #838 (a step after a grouping), approved and not
yet merged. When that release is on npm, jlens follows it whole: if #838 is not in it, or changed
before it merged, the difference stays recorded or jlens follows the change.

| # | Item | Done when | Status |
|---|---|---|---|
| 1 | The jsonata-js release after 2.2.2, once published | The official suite of that release vendored (`cargo xtask update-suite`) and the baseline grown; each fix's cases pass, or a difference is recorded with its reason in `compatibility.md`; the differential test and the book's examples run against that release, and the rules, the `differs:` marks and the rows of `compatibility.md` that 0.35 #8 added are removed where the release gives jlens's answers, and the cases of #838 kept in `jlens-core`'s tests are dropped once the suite has them; the version jlens follows changed wherever it is named | waiting for a jsonata-js release |

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
from what they needed, missed or worked around, not guessed beforehand. They have now, and #1
starts as 0.36 #5; the rest waits for the ADR.

| # | Deliverable | Definition of done | Status |
|---|---|---|---|
| 1 | ADR 0015, what 1.0 promises: what is stable, hidden or `#[non_exhaustive]`, below | The ADR accepted, with the list of changes that #2 to #4 make | proposed in 0.36 #5 |
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
