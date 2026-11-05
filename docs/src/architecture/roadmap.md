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

## 0.28: what 0.27 left in `$distinct` and `$match`, and the regex flags `u` and `v`

0.27 made `$distinct` and `$match` linear and measured what they still cost. `$distinct` reads
an item whole when items differ below a field they share or inside an array, and it trails
jsonata-js when an array has few distinct values. `$match` makes five allocations for each match.
0.28 takes those three. Each ends in a fix, measured before and after, or with its cause recorded
in [`perf-0.28.md`](https://github.com/thkprado/jlens/blob/main/docs/notes/perf-0.28.md); the starting points are in
[`perf-0.27.md`](https://github.com/thkprado/jlens/blob/main/docs/notes/perf-0.27.md).

0.28 also accepts the regex flags `u` and `v`. Without them, no pattern can name a letter outside
ASCII: `\w` is ASCII in JavaScript, and `\p{…}` needs the Unicode mode; `v` adds the difference
and intersection of classes (`[\p{L}--\p{Script=Latin}]`) and emoji sequences. jsonata-js
rejects both flags, so an expression valid in both engines keeps its result. The API work below
stays deferred.

| # | Item | Done when | Status |
|---|---|---|---|
| 1 | Upstream at the plan's start: `regress` after `95c0404`, stacker#155 and #156, jsonata-js on npm | Each `regress` commit worth porting ported one at a time, or none; the watch list updated | **done**: no `regress` commit, stacker unchanged, npm still 2.2.2; of jsonata-js's unreleased fixes, `$pad`'s `D2016` now reports an integer |
| 2 | `$distinct` with first fields that follow into nested values: 25 items of 100 KB that differ under a shared field (`{kind, list}`) or in an array (`[id, ...]`) take 2.65 ms against a `Document`, 17.5 µs by comparing | Those cases near the list's time; the other cases of 0.27's table and `ab` within the floor; conformance and 20,000 differential cases clean | **done**: they follow into the level below a shared field; 25 of `{kind, list}` 15 µs against a `Document`, 6.5 ms with `evaluate` (list 25.6 µs, 5.8 ms); flat records within the floor |
| 3 | `$distinct` with few distinct values: 100,000 items of `i % 8` take 1.48 ms, jsonata-js 229 µs | A fix measured against jsonata-js and with `ab`, or the cause recorded | **done**: each comparison was a `deep_eq` call and strings a `memcmp`; scalars now compare inline, short strings byte by byte; 100,000 of `i % 8` 312 µs against a `Document`, jsonata-js 355 µs on the same machine; `ab` within the floor |
| 4 | `$match` without five allocations per match (the matched text, the object, its map's table and entries, the groups): 0.70 µs per match, jsonata-js 0.21 to 0.30 | Allocations per match counted before and after; `growth`'s `$match` and `ab`'s `regex over a long text` measured; results unchanged | **done**: the search allocated too; the searches of a text now keep the backtracker's storage, and matches without groups share one empty array: 6.5 → 4 allocations a match, `$match` −24%, `regex over a long text` −36%; the object's four left |
| 5 | The regex flags `u` and `v`, an extension: `\p{L}`, `\p{Script=…}` and the other Unicode escapes, for text in any script, and with `v` set operations on classes; `\w` stays ASCII, as in JavaScript | The lexer, the VS Code grammar and the property tests take `u` and `v`, and reject them together or a flag twice, as JavaScript does; `compatibility.md` records them; `regex.md` gains a section on Unicode text, its examples in several scripts | **done**: `regex.md`'s "Unicode text" in Latin, Greek, Cyrillic and Japanese; its emoji example found a `jlens-regress` bug, fixed: `\p{RGI_Emoji}` outside a class matched `👍` of `👍🏽` alone |
| 6 | The tables measured again, unattended, after the dependencies are brought up to date: jlens and the control columns; other engines only where their version changed | `ab` against 0.27.0 in every mode, Python and Java; the performance page's tables and version header updated; each case within the floor or its cause recorded | |
| 7 | Release 0.28.0 | The release checklist; the channels checked | |

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
