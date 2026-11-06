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

## 0.29: completing values, and domains the application provides

The language service completes fields, variables, functions and operators, but never a value, and
an application cannot tell it where a field's values come from. 0.29 completes the values of an
`enum` or `const` and warns about a value outside one, lets the application provide the values of
an open domain, and gives the web editor schema overrides that show both, with a warning for an
override whose path the samples do not have. It also makes public the
conversion between byte and UTF-16 offsets that the Wasm package keeps to itself. The API work
below stays deferred.

| # | Item | Done when | Status |
|---|---|---|---|
| 1 | Upstream at the plan's start: `regress` after `95c0404`, stacker#155 and #156, jsonata-js's commits after `c5be116` and its version on npm | Each `regress` commit worth porting ported one at a time, or none; each jsonata-js fix checked against jlens; the watch list updated | **done**: no `regress` commit after `95c0404`, on `master` or its other branches; stacker#155 and #156 unchanged; no jsonata-js commit after `c5be116`, npm still 2.2.2 |
| 2 | The language service completes the values of an `enum` or `const`, below | Tests for each position below in `jlens-schema` and the language server; the web editor opens the list after `=` and `"`; a robustness case; the book's completion text updated | **done**: kind `value`, the walker's comparisons; the language server also completes on a quote |
| 3 | A warning for a value outside a declared `enum` or `const`, below; after #2 | `W0206` in the message catalog, with a suggestion; tests in `jlens-schema` for `=`, `!=` and `in`, for a declared and an inferred enumeration, and for a value that is not a literal; a robustness case; `gen-docs --check` and the book's warnings text updated | **done**: `Shape::enumeration_inferred`, `x-jlens-enum-inferred` in JSON Schema; suggestions for strings only |
| 4 | Open domains: values the application provides, below; after #2 | `x-jlens-domain` read and written by `jlens-schema`, and `domain` in the schema overrides; tests for the value request in `jlens-schema` and the Wasm suite; the web editor completes from an asynchronous source; the book's completion and Wasm guides updated | **done**: `Shape::domain`, `Completions::value_request`, `value_literal` (Wasm: `valueLiteral`); the editor's CodeMirror module takes a `values` source, `domains.js` stands in for one, and #5's example gives it a field |
| 5 | The web editor's schema overrides: a panel, and an example that uses them, below | The "Free shipping" example shows labels, descriptions, a hidden field, a closed and (after #4) an open domain; shared addresses carry the overrides, and older ones still open; `share.test.mjs` covers both | **done**: the panel beside the bindings, its error that of `inferSchema`; `overrides` left out of an address when empty; the editor check opens a shared address with overrides |
| 6 | A public conversion between byte and UTF-16 offsets, below | The type public in `jlens-core`, documented, with tests for ASCII text, text outside ASCII, a surrogate pair and the offset past the end; the Wasm package using it, its suite unchanged; the crate guide shows it | **done**: `jlens_core::Offsets` (`new`, `to_utf16`, `to_byte`, `is_ascii`); a text over 4 GiB walks the text instead of a table; the Wasm package keeps its span rewriting as functions over it |
| 7 | Overrides whose path is not in the samples, below | A function in `jlens-schema` and the Wasm API (additive: `apply_overrides` and `inferSchema` keep their results) that lists them, with the nearest path the samples have as a suggestion; `jlens schema infer` and the web editor's panel warn about them; tests in `jlens-schema`, the Wasm suite and the CLI; the schema guide updated | **done**: `Schema::missing_override_paths` and Wasm `missingOverridePaths`, the suggestion a misspelt name or a missing `[]`, and only a path that exists; a property test; the editor check shares a misspelt path |
| 8 | The tables measured again, unattended, after the dependencies are brought up to date: jlens and the control columns; other engines only where their version changed | `ab` against 0.28.0 in every mode, Python and Java; the performance page's tables and version header updated; each case within the floor or its cause recorded | |
| 9 | Release 0.29.0 | The release checklist; the channels checked | |

**#2, completing values.** The language service suggests fields, variables, functions and
operators, but never a value: a field's `enum` shows only in its documentation, and inside quotes
nothing is completed. A new completion kind, `value`, offers the values of the left-hand side's
enumeration (a schema's `enum` or `const`, or one inferred from samples) on the right of `=`, `!=`
and `in`: `type = |`, `type in [|]`, `type in ["apparel", |]`, and inside quotes, `type = "ap|"`,
where the whole literal is replaced. The value is inserted as a JSONata literal (strings quoted and
escaped). `examples` are not offered, as they are not every value. The walker reports comparisons
with the left-hand side's type; evaluation does not change. The kind reaches the Wasm API's
`types.d.ts`, the language server (`ENUM_MEMBER`) and the web editor's CodeMirror mapping.

**#3, a value outside the enumeration.** A misspelt value, `type = "aparel"`, is accepted in
silence: the comparison is always false, and nothing says why. With #2's comparisons, the language
service warns, as `W0206`, when the right-hand side is a literal outside the left-hand side's
`enum` or `const`, and suggests the nearest value, as `W0201` does for fields. Only declared
enumerations count: a schema's `enum` or `const`, or one set in the overrides. One inferred from
samples is a guess, so the `Shape` records where its enumeration comes from, and an inferred one
is never warned about; neither is an open domain (#4) nor a value that is not a literal.

**#4, open domains.** An `enum` is a closed list, known when the schema is written. Some fields
take values from a list only the application can fetch, often large, and filtered by what the
user types. Such a field carries `x-jlens-domain` in its JSON Schema, or `domain` in the
overrides of an inferred schema, an identifier only the application understands, kept in the
`Shape` as `domain`. At #2's positions, when the left-hand side has a domain, completion returns,
beside its items, a value request: the domain, the field's path, the prefix typed and the range to
replace. The service stays synchronous; the application fetches and caches the values, and the
Wasm package gives a helper that writes a value as a JSONata literal. A value outside an open
domain is never warned about; hover names the domain. The language server is left out until an
editor needs it.

**#5, the web editor's overrides.** The web editor infers its schema from the input document, so
none of its examples shows a label, a description or a hidden field, and a schema edited by hand
would be overwritten, or go stale, at the input's next change. Overrides avoid both: a panel
beside the bindings holds a JSON object of overrides by path (`"items[].type": {"label": ...}`),
and the schema becomes `inferSchema([input], overrides)`, so the structure still follows the
document and the overrides add what it means. A malformed override is shown as the panel's
error. The "Free shipping" example gets overrides; after #4, one of its fields has an open
domain, which a stand-in asynchronous source answers after a short delay.

**#6, converting offsets.** The Rust crates count offsets in bytes; a browser editor, and an
error's `position`, count UTF-16 code units. The Wasm package converts between them, but with a
type of its own, so an application that builds the language service of `jlens-schema` into its
own WebAssembly module, rather than using the package, writes the conversion again. The type moves
to `jlens-core` and becomes public: built once per text, it converts both ways, and an offset
inside a surrogate pair maps to the start of its character. The Wasm package uses it; behaviour
does not change.

**#7, an override's path missing from the samples.** An override whose path the samples do not
have declares that path: this is how an optional field the samples lack gets a label or a domain.
A misspelt path is declared the same way, `"custmer.login": {"domain": "customers"}`: the
override never reaches `customer.login`, completion offers the phantom `custmer` beside
`customer`, and nothing says why the domain does not work. The path is still declared, as an
optional field may be meant, but a new function lists such paths, each with the nearest path the
samples have when there is one (``did you mean `customer.login`?``). `jlens schema infer` prints
them as warnings, and the web editor shows them in the overrides panel, apart from its errors.

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
