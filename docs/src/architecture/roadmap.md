# Roadmap

## How 0.1 was built

jlens was built in phases. A phase was complete only when its documentation was written.

| # | Deliverable | Definition of done | Status |
|---|---|---|---|
| 0 | Workspace, CI, documentation skeleton, conformance harness | CI green; conformance report | **done** |
| 1 | Lexer, Pratt parser, AST post-processing, spans, recover mode, annotations; `jlens ast` / `jlens check` | Syntax groups of the suite and the parser-recovery cases pass | **done** |
| 2 | Value model and evaluator (paths, sequences, predicates, operators, lambdas, closures, tail calls, sort, group-by, parent operator, transform, limits) | Suite groups that don't depend on functions pass | **done** |
| 3 | Function library, signatures, regex, date/time and number formatting | 100% of the suite minus documented overrides | **done** |
| 4 | Evaluation logging and auditing (tracer, audit tree, levels, annotations, explanations) | Snapshot tests; under 3% overhead with the tracer off | **done** |
| 5 | `jlens-schema`: inference, overrides, JSON Schema, static typing, completion/diagnostics/hover | Table-driven language-service tests | **done** |
| 6 | Full CLI | CLI snapshot tests | **done** |
| 7 | WebAssembly package (browser ESM), TypeScript types, web editor example | Headless browser tests; working demo | **done** |
| 8 | Documentation consolidation, benchmarks, publishing | Doc examples verified; link check green; published to crates.io and npm | **done** (0.1.0) |

## Releases

The [changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md) lists every change.

| Version | Highlights |
|---|---|
| 0.1.0 | The first release: the engine, audits, schemas and the language service, the CLI and the WebAssembly package |
| 0.1.1 | Examples, fixtures and the web editor use an online store order |
| 0.2.0 | Host functions in Rust and WebAssembly ([ADR 0006](../adr/0006-host-functions.md)), and their declarations in the language service; the web editor's example selector |
| 0.3.0 | The language server `jlens lsp` and `jlens.json` ([ADR 0007](../adr/0007-language-server.md), [editors guide](../guides/editors.md)); the CLI's machine contract ([ADR 0008](../adr/0008-cli-machine-contract.md)): `eval --json`, `--now` and `--seed`, `check` over several files, `jlens errors`; cancellation and size caps in `jlens-core` |
| 0.4.0 | `check` warns about unknown functions, wrong argument counts and types, and undeclared variables, and reads `jlens.json`; `jlens test` runs files of test cases; prebuilt binaries, installable with `cargo binstall` |
| 0.5.0 | A cap on the size of a result (`U1001`); the size caps, clock and seed in the WebAssembly package, and the size caps in `jlens eval` and `jlens test`; differential testing against jsonata-js (`cargo xtask differential`); every binary built before a release publishes anything; a security policy |
| 0.6.0 | Regular expressions bounded by the timeout and cancellation ([ADR 0009](../adr/0009-bounded-regex.md)), on `jlens-regress`, a fork of `regress` 0.12.0; differential tests that reach regular expressions, pictures and decoders; npm provenance and build attestations for the binaries |
| 0.7.0 | Three jsonata-js quirks that expose JavaScript are no longer reproduced: a regular expression applied to a value that is not a string, irregular grouping in `$formatNumber`, `$substringBefore` and `$substringAfter` without `chars` |
| 0.8.0 | Coverage-guided fuzzing in a weekly `Fuzz` workflow, and the panics, stack overflows and slow cases it found fixed; `jlens lsp` survives a panicking request; `evaluate` reads its input lazily, one level at a time ([ADR 0010](../adr/0010-lazy-input.md)), natively and in WebAssembly, and `evaluate_source` reads other representations |
| 0.9.0 | A number that overflows to infinity and a date beyond JavaScript's range fail with `D1001` where they arise; `$formatNumber` and `$formatBase` write every digit of a large number; loops over floating-point numbers end on NaN and infinity; a parser stack overflow found by fuzzing fixed; `cargo xtask fuzz-replay` replays fuzz inputs on any platform |
| 0.10.0 | A recursion through a higher-order function fails with `D1011` instead of overflowing the stack; WebAssembly's default `maxDepth` is 500; `cargo xtask ab` compares the speed of two revisions |
| 0.11.0 | Faster evaluation of common shapes: field lookups on a single value, comparisons with a string literal, order-by keys evaluated once, blocks without frames, the result converted to JSON in one pass, less set-up per evaluation |
| 0.11.1 | `$fromMillis` and `$now` fail with `D3150` on a timezone that is not a number |
| 0.12.0 | The book and the web editor published at <https://thkprado.github.io/jlens/> at each release tag; the editor evaluates in a Web Worker and shares expressions by address |
| 0.13.0 | Host functions call the functions they receive, in Rust (`HostCall::function`, `Callable`) and in WebAssembly (a JavaScript function), within the evaluation's limits ([ADR 0011](../adr/0011-host-callbacks.md)) |
| 0.14.0 | The `jlens` command from npm (`jlens-cli`, [ADR 0012](../adr/0012-npm-distribution.md)), Homebrew, Scoop and the container image `ghcr.io/thkprado/jlens`, all from the attested release binaries |
| 0.15.0 | The VS Code extension `thkprado.jlens` on the Marketplace and Open VSX, with the release binary bundled ([ADR 0013](../adr/0013-vscode-extension.md)), and mimalloc as the allocator of the `jlens` command |
| 0.16.0 | Faster group-by (no longer quadratic in the size of a group), paths of names, literal group keys and names mapped over many items, closing the gaps with jsonata-core |

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

## 0.3: a machine contract and core guardrails

| # | Deliverable | Definition of done | Status |
|---|---|---|---|
| 1 | ADR 0008, the CLI's machine contract: a JSON envelope for results and errors, exit codes, the roles of stdout and stderr, never reading a terminal | ADR accepted; the CLI guide has a "For scripts and agents" section | **done** |
| 2 | `Options` becomes `#[non_exhaustive]`, with builder methods. A fixed clock (`$now`, `$millis`) and a random seed (`$random`, `$shuffle`) | The same expression, input and options give byte-identical results; crate guide snippets updated | **done** |
| 3 | `jlens eval --json`: one compact envelope per evaluation, `{"ok":true,"result":…}`, `{"ok":true,"undefined":true}` or `{"ok":false,"error":{code, message, position, line, column}}`. With `--ndjson`, one envelope per line, carrying the line number | Snapshot tests for a result, undefined, a syntax error, an evaluation error and invalid input | **done** |
| 4 | `jlens eval --now` and `--seed` | CLI tests with `$now()` and `$random()` | **done** |
| 5 | `eval` fails with code 2 instead of waiting when standard input is a terminal and neither `-i` nor `--no-input` is given | CLI test | **done** |
| 6 | `jlens check` takes several files and glob patterns, adds `file`, `line` and `column` to its JSON, prints it compact, and has `--deny-warnings` | Snapshot tests; the CI loop in the CLI guide becomes one command | **done** |
| 7 | `jlens errors [CODE] [--json]`, from the error catalog | Snapshot tests | **done** |
| 8 | Cancellation from another thread, and caps on the size of built strings and arrays (`U1001`) | Property tests; an expression that builds a huge value stops with `U1001` | **done** |
| 9 | The public API is reviewed before it grows further: what `jlens-core` exposes, and `Send`/`Sync` of `Expression` and `Document`, asserted in tests. `cargo-semver-checks` runs in CI | CI job green | **done** |

## 0.4: catching mistakes before they run

| # | Deliverable | Definition of done | Status |
|---|---|---|---|
| 1 | `check` warns about unknown functions, with a suggestion (`$summ` → `$sum`), and about calls with the wrong number of arguments | Language-service and CLI tests; no warning on host functions declared in `jlens.json` | **done** |
| 2 | `check` warns about undeclared variables, when declarations are given, and about argument types that cannot match a signature, when the types are known | Table-driven tests without false positives on the conformance suite's expressions | **done** |
| 3 | `check --config jlens.json`, and the `jlens.json` of the current directory by default: the CLI and the editors see the same schema, variables and functions | CLI tests sharing a fixture with the language server tests | **done** |
| 4 | `jlens test`: runs a file of cases (input, bindings, expected result or error code) and reports the failures, as text or as `--json` | Snapshot tests; guide page | **done** |
| 5 | Prebuilt binaries for Linux, macOS and Windows attached to each release, installable with `cargo binstall` | A release with the binaries | **done** |

## 0.5: safe releases, enforced limits and proven compatibility

| # | Deliverable | Definition of done | Status |
|---|---|---|---|
| 1 | The release workflow builds every binary before it publishes anything, and CI builds the release targets on each push to `main` | `crates` and `npm` need `binaries`; the CI job is green on `main` | **done** |
| 2 | The library's guarantees are checked: a CI job with the declared minimum Rust version (1.88), `cargo-deny` for advisories and licenses, and a `SECURITY.md` for reporting vulnerabilities | CI jobs green; `SECURITY.md` linked from the README and CONTRIBUTING | **done** |
| 3 | A cap on the total size of a result, which counts a shared value each time it appears, so that a short expression that nests the same array cannot exhaust memory when the result is serialized (`U1001`) | Property tests; such an expression stops with `U1001` | **done** |
| 4 | The WebAssembly package takes every option of `jlens-core` but cancellation (`maxStringLength`, `maxArrayLength`, the size cap of item 3, `now`, `seed`), and `jlens eval` and `jlens test` take the size caps | Tests in the JS suite and the CLI; the guides list the options | **done** |
| 5 | Differential testing against jsonata-js 2.2.2: generated expressions and inputs run in both engines, which must agree on the result or the error code | A `cargo xtask` command and a scheduled CI job; each divergence fixed, or documented in the compatibility page and the overrides | **done** |

## 0.6: bounded regular expressions, wider differential tests, signed packages

| # | Deliverable | Definition of done | Status |
|---|---|---|---|
| 1 | A regular expression cannot outlast the limits of an evaluation. Today the timeout and cancellation are checked between evaluation steps, never inside a match, so catastrophic backtracking (`$contains('aaaaaaaaaaaaaaaaaaaaaaaaaa!', /^(a+)+$/)`) runs for minutes, and freezes the page in WebAssembly, whatever the timeout. The design (a linear-time matcher where the pattern allows it, a step budget, or both) is decided first, in an ADR | Property tests; such a match stops with an error code within the timeout, in Rust and in WebAssembly; the compatibility page documents the difference | **done** |
| 2 | The differential tests reach the code that was hardest to port: regular expression literals (`$match`, `$replace`, `$split`, `$contains`, `~>`), valid pictures for `$formatNumber`, `$formatInteger` and `$fromMillis`, and the functions the generator leaves out (`$sift`, `$each`, `$match`, `$base64decode`, `$decodeUrl`, `$decodeUrlComponent`, `$encodeUrl`) | Generator extended; each divergence fixed, or documented in the compatibility page with a reasoned exception | **done** |
| 3 | Released packages prove where they were built: the npm package is published with provenance, and the binary archives carry build attestations | The next release shows provenance on npm and attestations on GitHub; CONTRIBUTING says how to verify them | **done** |

## 0.8: proven robustness and fast single evaluations

| # | Deliverable | Definition of done | Status |
|---|---|---|---|
| 1 | Coverage-guided fuzzing backs the promise never to panic. The property tests run a fixed number of cases in CI, and the code keeps invariants with `expect` and `unreachable!` that only a panic would reveal broken: in the language server, a panic on a half-typed expression stops the server; in WebAssembly, it traps and leaves the instance unusable. `cargo-fuzz` targets cover the parser (normal and recover mode), compiling and evaluating under limits, `jlens-regress`, the pictures of `$formatNumber`, `$formatInteger` and `$fromMillis`, date parsing in `$toMillis`, and the language service. `jlens lsp` answers a request that panics with an error and keeps running | A scheduled `Fuzz` workflow, like `Differential`; every crash it found fixed, with a regression test; an LSP test for a panicking request | **done** |
| 2 | The cost of a single evaluation is measured and cut where the value model allows it. `evaluate` converts the whole input before it reads any of it: on the 24 KB document of the benchmarks, a path takes 153 µs natively and 402 µs in WebAssembly, against 0.9 µs and 2.5 µs with a `Document`, and 2.1 µs in jsonata-js. In WebAssembly the input goes through `JSON.stringify`, `serde_json` and the conversion to values | The performance page shows the time of each step; the steps that can go without changing the value model are gone; no benchmark case is slower | **done** |
| 3 | Lazy conversion of input documents: an evaluation converts only the parts of its input it reads. This changes the value model ([ADR 0003](../adr/0003-value-model.md)); the design comes first, in [ADR 0010](../adr/0010-lazy-input.md) | ADR accepted; with `evaluate`, the path case is within 10× of `evaluate_document`, natively and in WebAssembly; property tests; no other benchmark case more than 3% slower | **done** |

## 0.9: numbers and dates at the edges, and tools for the next steps

| # | Deliverable | Definition of done | Status |
|---|---|---|---|
| 1 | CI lints the workflows: `actionlint`, with `shellcheck` for the scripts of `run` steps | A CI job; the workflows pass it | **done** |
| 2 | `cargo xtask release-notes` writes links that work on the GitHub release page. The changelog links pages relatively (`](docs/src/...)`), which break there; for 0.8.0 they were fixed by hand in the draft | Relative links become absolute at the tag (`blob/vX.Y.Z/...`); a unit test | **done** |
| 3 | A fuzz input can be replayed without libFuzzer. The targets cannot link on Windows, so a crash is reproduced today with a throwaway program that repeats the target's calls | One command runs a saved input (such as `last-input` from a failed run) through the body of any target, on a 1 MB stack like the fuzzing thread, on any platform; the fuzzing section of the conformance page says how | **done** |
| 4 | Loops and recursion over floating-point numbers end on NaN and infinity. Fuzzing found two that did not (`number_to_words`, `decimal_to_letters`); the others are found by reading the code rather than waiting for the weekly run | Every loop or recursion whose end depends on an `f64` reviewed; each one that could run forever or overflow the stack fixed, with a regression test | **done** |
| 5 | Decide where an infinite number stops. `1e308 * 10` is infinity in both engines, and jsonata-js raises `D1001` only when an infinite number is the operand of another operator, so it reaches functions and comes out as a result. jlens may fail at the operator that overflows instead | Behaviour of both engines recorded; the decision made and implemented; the compatibility page, and the differential rules when jlens differs | **done** |
| 6 | `$formatNumber` formats large numbers exactly. `$formatNumber(1e21, '#,##0.00')` gives `"1e,+21.00"` in both engines: the exponent of JavaScript's number-to-string conversion leaks into the picture | The digits of the number formatted through the picture; the compatibility page and the differential rules | **done** |
| 7 | One model for dates beyond JavaScript's range. Today `$fromMillis(1e17, '[Y0001]')` computes a year ("0843") while the day of the year and the week are NaN, because only part of the date code clips like V8; jsonata-js gives NaN everywhere | One model chosen (clip like JavaScript, or never clip) and applied to every date component; the compatibility page | **done** |

## 0.10: performance and robustness debts

Explorations: each one may end in a fix, or in the cause written down and the debt accepted.
Both outcomes close the item.

| # | Deliverable | Definition of done | Status |
|---|---|---|---|
| 1 | A/B benchmarks as a command. Criterion is bimodal on the maintainer's machine, so changes are measured by building one program per variant and running them alternately, by hand | A command (an `xtask` or a script in `benches/`) that builds two revisions and compares them by alternating runs; the performance page says how | **done** |
| 2 | The cost lazy input left on evaluations that read their whole input: `regex` +4%, `filter` +2.8%, `strings` +2.7% when measured, above the 3% that 0.8 allowed | Measured again with the A/B command; the cost removed, or recorded on the performance page with its cause | **done** |
| 3 | WebAssembly's default `maxDepth` back to 1000. Since lazy input, an audited recursion near 1,000 levels overflows V8's stack while the function is optimized, so the default became 800; the cause of the larger frames was not found | The cause found and the default restored, or the cause (or what was ruled out) recorded in the WebAssembly guide | **done** |
| 4 | Two crashes of the fuzzing runs, both stack overflows without a message, never reproduced: one while `parse` loaded its corpus, one in `pictures` | The cached corpora and any input saved since (`last-input`) replayed with the replay command; a crash that reproduces fixed, with a regression test; otherwise what was tried recorded in the fuzzing section | **done** |
| 5 | Two `cargo xtask ab` runs at once both fail when they build the same revision: each adds and removes the same temporary worktree in `target/ab` | Concurrent runs either share the build (a lock, or a worktree per run) or the second stops with a clear message; the performance page says whether runs may overlap | **done** |

## 0.11: faster evaluation of common shapes

A comparison with [jsonata-core](https://github.com/txjmb/jsonata-core) 2.2.11, another Rust
port, on the cases of `benches/` (`evaluate_document` against its `evaluate`, both converting the
result to `serde_json::Value`) found it 2 to 5 times faster on five cases: path (0.9 against
0.1 µs), filter + aggregate (164 against 93 µs), group-by (570 against 123 µs), sort + map
(1.79 ms against 655 µs) and the shipping rule (433 against 254 µs). Both engines are about
equal on the other cases, and jlens is faster on regex. Measuring
parts of these expressions traced the gaps to a few places in `eval.rs`, where work is repeated
or allocated that no result depends on. Each item removes one of them without changing any
result, error or evaluation order that jsonata-js shows. jsonata-core's other technique, a
second evaluator for simple expressions, is left out: two evaluators of the same language can
disagree.

Every item is measured with `cargo xtask ab` against the commit before it, and checked with the
conformance suite and a `cargo xtask differential` run. When a tracer is attached, evaluation
takes the general path, so audits keep reporting every node.

| # | Deliverable | Definition of done | Status |
|---|---|---|---|
| 1 | Sort keys are evaluated once per item. The order-by operator (`^(...)`) evaluates the keys of both items at every comparison, as jsonata-js does: about 3,000 evaluations to sort 200 items, where 200 suffice. `items^(tax)` takes 1.05 ms, against 208 µs in jsonata-core | Each item's keys computed on first use and kept, so the keys are evaluated in the same order and the first error is the same; tests for `T2007` and `T2008`; sort + map faster with `ab`; the compatibility page notes that a key using `$random` is drawn once per item | **done** |
| 2 | A block without a binding creates no frame. `(...)` allocates a frame each time it is evaluated, so `items.(unitPrice)` takes 85 µs where `items.unitPrice` takes 16 µs | Post-processing marks the blocks whose own expressions bind a variable (`:=`), and only those create a frame; tests for scoping and for closures defined in a block; `ab` | **done**: `items.(unitPrice)` 83 → 77 µs; the rest of the gap is the one-step path of #3. A block that calls a function keeps its frame when the expression names `$eval`, which binds in its caller's frame |
| 3 | A field name evaluated on an object is a lookup. A bare name such as `unitPrice` is a path of one step, which builds and flattens sequences: about five allocations per evaluation. `items.(unitPrice * quantity)` takes 159 µs, against 39 µs in jsonata-core | A one-step path of a name, without stages, on an input that is not an array, returns the field when its value is not an array, and takes the general path otherwise; unit tests for the sequence and singleton rules it skips; filter + aggregate, group-by and the shipping rule faster with `ab` | **done**: filter + aggregate 161 → 56 µs, group-by 842 → 575 µs, shipping rule 414 → 152 µs, strings 198 → 104 µs (`ab --mode document`); a property test checks that evaluation agrees with an audit, which takes the general path |
| 4 | A string literal does not allocate each time it is evaluated. Today it builds a new `Rc<str>`, so `items[type = 'electronic']` allocates one string per item. Exploration: a string shared by the evaluation, or a comparison with the literal that builds no value | The allocation gone and measured with `ab`, or the cost and the options recorded on the performance page | **done**: `=` and `!=` compare with the literal's text, untraced: filter + aggregate 56 → 44 µs, strings 105 → 92 µs, shipping rule 150 → 127 µs (`ab --mode document`). Other uses (arguments, `&`, object values) still build a string: sharing one per evaluation needs a slot per literal, since the AST is `Send` and values use `Rc` |
| 5 | The result is converted in one pass. Converting the whole document as a result takes 210 µs, against 152 µs in jsonata-core: its size is measured in a first walk, then it is converted in a second | The size counted while converting, stopping at `Options::max_result_size` as before; the property tests of the size cap pass unchanged; a large result faster with `ab` | **done**: returning the whole input (`$`) 271 → 236 µs (`ab --mode document`); the transform case, whose result is built, is unchanged, since its time goes to building it |
| 6 | The fixed cost of an evaluation is cut. `1` takes 0.35 µs, against 0.07 µs in jsonata-core: the clock is read twice for `$now` and the seed of `$random`, and the root frame and the binding of `$` allocate | The clock read once, and only what an evaluation uses set up; the path case faster with `ab` | **done**: the clock is read on first use, the start time only with a timeout, and the input is kept in the root frame instead of a binding of `$`: `1` 350 → 106 ns, path 927 → 676 ns (`ab --mode document`) |
| 7 | The performance page shows the new numbers, natively and in WebAssembly, with the comparison with jsonata-js | The tables measured again; the "Where the time goes" section updated | **done**: natively, `evaluate_document` is 2 to 4 times faster than in 0.8 on filter + aggregate, sort + map and the shipping rule; jlens-wasm `evaluateDocument` is now faster than jsonata-js on all ten cases |

## 0.12: jlens online

Today the book and the web editor exist only in the repository: the README links no site, CI
builds the book only to check it, and trying the editor takes a clone, a Rust toolchain and
`cargo xtask wasm`. This release publishes both on GitHub Pages, at each release tag, as
[try.jsonata.org](https://try.jsonata.org) does for jsonata-js. The editor shows what sets
jlens apart (completion from an inferred schema, diagnostics, the explanation and audit tree of
every result), and everything runs in the browser: no expression or document leaves the
visitor's machine.

The site is the book at the root and the editor under `editor/`, built from the files of the
tag, so the editor runs the engine that the book of that version describes. The first
deployment, and enabling Pages in the repository settings, wait for the maintainer.

| # | Deliverable | Definition of done | Status |
|---|---|---|---|
| 1 | One command builds the site. `cargo xtask site` builds the book and the WebAssembly package and assembles them in `target/site`: the book at the root, the editor in `editor/` with the package in `editor/pkg/`. The editor imports the package by a bare name (`jlens-wasm`) mapped in its import map, so the same files run from the repository (`../../crates/jlens-wasm/pkg/jlens.js`) and from the site (`./pkg/jlens.js`): the command rewrites only that entry | The command; `cargo xtask serve` can serve `target/site` (a flag), and `wasm-test --example` also loads the built editor; the editor README and `CONTRIBUTING.md` say how to preview the site; the book's introduction and the WebAssembly guide link the editor | **done**: the book links the editor at its published address, which works from the first deployment (#2) |
| 2 | A `Pages` workflow publishes the site at each `v*` tag, and by hand on a tag (`workflow_dispatch`), with `actions/upload-pages-artifact` and `actions/deploy-pages`. `book.toml` sets `site-url = "/jlens/"`, so the book's 404 page works under the project path | The workflow passes `actionlint`; the maintainer has set Pages to "GitHub Actions"; a first deployment of the last tag, made with the maintainer's confirmation, serves the book and the editor at `https://thkprado.github.io/jlens/`; `homepage` in `Cargo.toml` and in the npm package, and the README, point to it (crates.io and npm show it from the next release) | **done**: the first deployment waits for the 0.12.0 tag (#5), since 0.11.1 has neither the workflow nor `cargo xtask site`; the `github-pages` environment must allow `v*` tags |
| 3 | The editor stays responsive. Evaluation runs on the page's thread with a 2 s timeout, so a slow expression freezes typing for up to 2 s at each keystroke. Evaluation moves to a Web Worker, and an evaluation still running when the expression changes is dropped (the worker is replaced) | Typing never waits for an evaluation; the timeout and `D1011` are reported as now; `wasm-test --example` passes for the repository and the site | **done** |
| 4 | Expressions can be shared. A "Share" button puts the expression, the input and the bindings in the address: compressed with `CompressionStream` (`deflate-raw`), in base64url, after `#`, so that no server stores them. Opening such an address restores them; the named examples (`#invoice`) keep working. The header links the book and says that evaluation runs in the browser | A round-trip test of the encoding; a size limit (some tools cut addresses beyond about 8 KB: the button says so instead of giving a broken link); the editor README documents the format | **done** |
| 5 | Release 0.12.0, which deploys the site of its tag | The release checklist of `CONTRIBUTING.md`, with a step for the `Pages` run; the site serves 0.12.0 | **done**: step 7 of the checklist checks the `Pages` run |

## 0.13: host functions that call back

A host function cannot call a JSONata function it receives as an argument: the argument arrives
as `None` (`undefined` in WebAssembly), as ADR 0006 decided. Rule engines need it for helpers
written in the host language that apply a function given to them (`$retry($fetch, 3)`,
`$memoize(function($k) {...})`, a lookup with a fallback function). This release adds a
callable handle without changing the functions already written against ADR 0006.

| # | Deliverable | Definition of done | Status |
|---|---|---|---|
| 1 | The design, in ADR 0011. First, what jsonata-js does: in `benches/compare`, register a JavaScript function, pass it a lambda, a built-in function (`$uppercase`) and a partial application, and record what it receives and whether it can call each. Then the Rust API; a proposal to check: `HostCall::function(i) -> Option<Callable<'_>>` and `Callable::call(&self, args: &[Option<JsonValue>]) -> Result<Option<JsonValue>>`, keeping the JSON boundary. The handle borrows the call, so it cannot outlive it. The nested evaluation counts toward the same depth, timeout, cancellation and size limits; its tracer events nest under the host call's node; its errors come back to the host function as `Err` with their codes, and pass through when the host function returns them | ADR 0011 accepted by the maintainer, with the jsonata-js findings; any difference planned for the compatibility page | **done**: [ADR 0011](../adr/0011-host-callbacks.md); four differences planned for the compatibility page (#2, #3) |
| 2 | The Rust implementation, as ADR 0011 decides | Unit tests: lambdas, built-in and host functions, partial applications and closures passed back; errors and their codes; a recursion through a host function ends with `D1011`; a timeout inside the callback; a property case in `robustness.rs`; an audit snapshot with a nested call; a snippet in `guide.rs` and the host function section of the crate guide; `cargo xtask ab` shows no change on the untraced path | **done**: `HostCall::function` and `Callable`; the crate guide's [functions as arguments](../guides/crate.md#functions-as-arguments) |
| 3 | The same in WebAssembly. A JavaScript host function receives a JavaScript function for a JSONata function argument; calling it after the host function has returned throws, with a jlens code (`U`) | The four updates of a Wasm API change (`crates/jlens-wasm/AGENTS.md`); tests in `suite.mjs` for Node and the browser; the WebAssembly guide | **done**: a JavaScript function with an `arity` property; [functions as arguments](../guides/wasm.md#functions-as-arguments) |
| 4 | Release 0.13.0 | The release checklist; ADR 0006 points to ADR 0011 | **done** |

## 0.14: more ways to install the CLI

Today the `jlens` command comes from the release archives, `cargo binstall` or `cargo install`.
Tools like [jq](https://jqlang.org/download/) also come from the package managers people already
use. Each channel adds a step to every release, and one more place where a release can fail. So
they come in order of reach, each published by `release.yml` from the archives already built
and attested, never rebuilt, and each one stays only if it updates without manual work.

| # | Deliverable | Definition of done | Status |
|---|---|---|---|
| 1 | The CLI from npm, designed. The idea noted earlier, a `bin` in `jlens-wasm`, needs either a second CLI in JavaScript, which could drift from the contract of ADR 0008, or `jlens-cli` compiled to WASI, which Node.js runs only with an experimental warning. The third way is the one of esbuild and Biome: a small `jlens` package whose `optionalDependencies` are one package per platform, each holding the release binary, picked by `os` and `cpu` | The decision recorded in ADR 0012, with the npm names checked to be free | **done**: [ADR 0012](../adr/0012-npm-distribution.md); `jlens` is taken on npm, so the command comes from `jlens-cli` and five `jlens-cli-<os>-<cpu>` packages |
| 2 | The npm packages, published by `release.yml` with provenance, after the binaries | `npm pack` of every package in CI, and a job that installs the packed packages on Linux, macOS and Windows and runs `jlens --version`; the installation page; `npx jlens-cli --version` works after the release | **done**: `cargo xtask npm-cli` builds the six packages from the archives; CI packs them and runs the installed `jlens` on Linux, macOS and Windows; `release.yml` publishes them with provenance. `npx jlens-cli` is checked with the 0.14.0 release (#6) |
| 3 | Homebrew. A tap, `thkprado/homebrew-jlens` (created by the maintainer), with a formula for the macOS and Linux archives and their checksums from `SHA256SUMS`; `release.yml` updates it with a token limited to that repository | `brew install thkprado/jlens/jlens` installs the release; the formula's `test` runs `jlens --version`; the installation page; the token's scope documented in `CONTRIBUTING.md` | **done**: `cargo xtask homebrew` writes the formula from `SHA256SUMS`; the `Homebrew` workflow commits it to the tap when a release is published (a draft's archives cannot be downloaded yet), and CI installs and tests the formula with the CI archives on Linux and macOS. `brew install` is checked with the 0.14.0 release (#6) |
| 4 | Windows: a Scoop bucket, `thkprado/scoop-jlens`, with a manifest that updates itself from the GitHub releases (`checkver`, `autoupdate`). winget is left out for now: each version is a pull request that Microsoft reviews | `scoop install jlens` works after adding the bucket; the installation page | **done**: `cargo xtask scoop` writes the manifest from `SHA256SUMS`, with a `checkver` and an `autoupdate` that Scoop's `checkver.ps1` turns into the same manifest; the `Scoop` workflow commits it to the bucket when a release is published, and CI installs the manifest with the CI archive on Windows. `scoop install` is checked with the 0.14.0 release (#6) |
| 5 | A container image, `ghcr.io/thkprado/jlens`: `FROM scratch` with the static Linux binaries (x86-64 and ARM64), built from the release archives and attested | `docker run --rm -i ghcr.io/thkprado/jlens eval '...'` works on both architectures; the installation page | **done**: `crates/jlens-cli/Dockerfile` unpacks the musl archive of the platform being built into a `scratch` image, so the multi-platform build runs no emulated step; `release.yml` pushes it with the run's token, tagged `X.Y.Z`, `X.Y` and `latest`, and attests it; CI builds it from its own archives and runs it on x86-64 and ARM64 runners. The pull from ghcr.io is checked with the 0.14.0 release (#6) |
| 6 | Release 0.14.0 | The release checklist, with a check of each channel after the release | **done**, except npm: Homebrew, Scoop, the container image and the archives checked; npm refused `jlens-cli-win32-x64` ("Package name triggered spam detection"), so `jlens-cli` was not published (#7) |
| 7 | The CLI on npm, after #6. The four other platform packages are on npm at 0.14.0; a support ticket asks npm to allow the name. Without an answer by 2026-10-20, the platform packages move to a scope (`@jlens/cli-<os>-<cpu>`), which changes ADR 0012. Before the next release tag, either the name is allowed, the packages are renamed, or the npm job of the CLI is left out of that release | `npx jlens-cli --version` works; the installation page and the release notes show the npm channel again | pending: left out of 0.15.0 and 0.16.0, as `release.yml` publishes the CLI to npm only when the repository variable `PUBLISH_NPM_CLI` is `true`. No longer planned: the 2026-10-20 deadline is dropped, the job stays off until npm answers, and the scope remains the way out |

## 0.15: jlens in VS Code, and a faster CLI allocator

`jlens lsp` has worked since 0.3, but VS Code starts language servers only from extensions, and
most JSONata users write in VS Code. Neovim, Helix and Emacs need a few lines of configuration;
VS Code needs an extension. Like the other channels, the extension ships the attested release
binary, never a rebuilt one. The milestone also settles the allocator question of 0.8: on
Windows, `mimalloc` made every benchmark 20 to 45% faster, and Linux has not been measured.

| # | Deliverable | Definition of done | Status |
|---|---|---|---|
| 1 | The extension, designed: where the server comes from (one VSIX per platform with the release binary, as rust-analyzer does, a `jlens` found on the `PATH`, or a download at first start), a setting that points to another `jlens`, the language id and file extensions next to other JSONata extensions, the publisher on the Visual Studio Marketplace and Open VSX, and where its TypeScript lives in this repository | The decision recorded in ADR 0013, with the names checked to be free on both registries | **done**: [ADR 0013](../adr/0013-vscode-extension.md); one VSIX per target (the musl binaries serve `linux-` and `alpine-`) plus one that uses the `PATH`; `thkprado.jlens` on both registries; the Marketplace credential is chosen in #5, as global Azure DevOps tokens end on 2026-12-01 |
| 2 | Syntax highlighting: a TextMate grammar for JSONata (comments, strings, numbers, names, `$variables` and function calls, operators, regular expressions) and a language configuration (comments, brackets, auto-closing pairs) | The grammar's snapshot tests run in CI over the expressions of the suite and the book's examples, and cover every token kind | **done**: `editors/vscode/syntaxes/jsonata.tmLanguage.json`, tested with vscode-textmate, the engine of VS Code. Snapshots of fixtures that use every scope of the grammar; over the 1,941 expressions of the suite and the book, no character is left without a scope, nothing stays open at the end, and the grammar finds the regular expressions jsonata-js's parser finds (a `/` is division or a regular expression depending on what precedes it) |
| 3 | The extension starts `jlens lsp` with `vscode-languageclient` for `.jsonata` files: the setting for the server's path, a command that restarts it, its log in an output channel, a clear message when no server is found, and a JSON Schema for `jlens.json` so that VS Code checks it as it is edited | An extension test (`@vscode/test-electron`, in CI on Linux, macOS and Windows) opens an expression and gets a diagnostic, a completion and a hover from the server; the editors guide | **done**: `editors/vscode/src/extension.ts`, over `vscode-languageclient` 10 (VS Code 1.91 or later). The server is looked for in `jlens.server.path` (user settings only), the bundled binary, then the `PATH`; the **jlens** output channel shows its version. The test opens `fixtures/workspace`, as the server's and the CLI's tests do, and checks the diagnostics, a completion, a hover, the restart command and the validation of `jlens.json` |
| 4 | Packaging: `cargo xtask vscode` builds one VSIX per release target from the archives, as `cargo xtask npm-cli` does, plus one without a binary for other platforms, which looks for `jlens` on the `PATH` | CI packs every VSIX and installs the one of its platform in the extension test | **done**: `xtask/src/vscode.rs` stages the extension and runs `vsce package` once per target with the binary of its archive in `server/`, and once without; the version is the workspace's. CI packs them on Linux, where the binaries keep their executable bit, then installs the package of each OS in VS Code and runs the extension test with the bundled binary (`JLENS_VSIX`), which it also checks to be executable |
| 5 | Publishing: `release.yml` publishes the VSIX files to the Marketplace and to Open VSX after the binaries, skipping a version already published; the credentials, chosen as ADR 0013 says, and the steps to renew them are documented in `CONTRIBUTING.md` | The release checklist checks the extension on both registries | **done**: the `vscode` job of `release.yml` builds the VSIX files with `cargo xtask vscode` and publishes them with `vsce` and `ovsx` `--skip-duplicate`, from the GitHub environment `vscode`, which accepts only version tags. The Marketplace is reached as a Microsoft Entra ID application federated with that environment; Open VSX with a token for the first version, then through trusted publishing. Neither keeps a long-lived secret; `CONTRIBUTING.md` says how to set them up, and the checklist installs the extension from the Marketplace and lists its targets on Open VSX |
| 6 | The allocator of the CLI, measured on Linux: a `cargo xtask ab` run on a Linux runner, with and without `mimalloc` as the global allocator of `jlens` only; the library keeps leaving the choice to applications | If Linux gains about as much as Windows, `jlens` links `mimalloc` on every release target, `cargo-deny` accepts it, and the performance page shows both platforms; otherwise the measurement goes to the performance page and the item closes | **done**: on the same machine, `mimalloc` makes the Linux release binary (musl) 28 to 91% faster, Windows 14 to 56% and a glibc build up to 25%, so `jlens` uses it on every release target; `cargo xtask ab` gained `--b-allocator mimalloc` and `--target`, and the [performance page](performance.md#the-allocator-of-the-command) shows the three platforms |
| 7 | Release 0.15.0. Before the tag, the npm channel of the CLI (0.14 #7) is settled or left out of the release | The release checklist, with the extension installed from both registries | **done**: the npm channel of the CLI is left out (0.14 #7) |

## 0.16: the last gaps with jsonata-core, and string literals

A new comparison with jsonata-core 2.2.11, after 0.15.0 and on the same cases as in 0.11, finds
jlens ahead on seven of the ten. Of the three left, sort + map is close (638 against 577 µs), and
two have a cause: group-by (375 against 119 µs), which copied each group at every item, and
the path case (0.7 against 0.1 µs), where the fixed cost of an evaluation dominates. 0.11 #6 cut
that cost from 350 to 106 ns for `1`, but a path still takes about seven times as long.

| # | Deliverable | Definition of done | Status |
|---|---|---|---|
| 1 | Group-by joins each group once. `Evaluator::group` appends each item to its group with `functions::append`, which copies the group, so a group of k items costs O(k²): `items{"n": $count($)}` takes 153 µs, against 38 µs in jsonata-core | Each group collects its items and builds its value once, with the same rules (undefined skipped, a single item kept as it is, arrays flattened one level); unit tests for those rules; group-by faster with `ab` | **done**: group-by 64% faster with `ab`, and 108 against 119 µs in jsonata-core; a group of 200 items 8 times faster. Tuple streams are reduced the same way |
| 2 | The fixed cost of a path. The path case takes 0.7 µs, against 0.1 µs in jsonata-core. Exploration: profile an evaluation of a short path on a `Document` and list what it sets up or allocates that no result depends on | The cost cut and measured with `ab` (the path case faster, no other case slower), or what takes the time, and why it stays, recorded on the performance page | **done**: path 67% faster with `ab` (681 to 227 ns), sort + map 33% (`$i.code`), shipping rule 7%, no case slower; 0.23 against 0.14 µs in jsonata-core. A path of names, or one starting with a variable, looks its fields up while no array is met. The rest (fixed cost, result conversion) and the per-item cost of `items.code` are on the performance page |
| 3 | String literals in arguments, `&` and object values do not allocate each time they are evaluated. 0.11 #4 removed the allocation only for `=` and `!=`; elsewhere a literal still builds a new string. Sharing one per evaluation needs a slot per literal, since the AST is `Send` and values use `Rc`. Exploration: measure its share of the time first, with cases added to `benches/` | Below about 5% of those cases, the share and the options recorded on the performance page; above it, the allocation gone and measured with `ab`, no other case slower | **done**: about 3% where a literal is an argument, a `&` operand or an object value per item, recorded on the performance page; above it, literal keys of a grouping, now looked up by their text: group-by −12%, the new object constructor case −5%, nothing slower (`ab`) |
| 4 | A name over many items. `items.code` takes about 110 ns per item, against about 65 ns in jsonata-core, and `$count(items)` 0.94 against 0.41 µs. Looking the field up without evaluating the step saves only 12%, so the cost is elsewhere. Exploration: count the allocations per item, then separate the cost of the call from the cost of the step | The cost cut and measured with `ab` (the cases that map a name faster, no other case slower), or what takes the time, and why it stays, recorded on the performance page | **done**: allocations were not the per-item cost; copies were. A field holding an array is returned as it is, a step passes its items to the next without copying them into a new sequence, and results with no array to flatten are not copied twice. New case names over items (`$sum(items.quantity)`) 28% faster with `ab --mode document`, shipping rule 8%, group-by 4%, nothing slower; `$count(items.code)` 9.8 to 7.0 µs (4.9 in jsonata-core), `$count(items)` 0.94 to 0.67 µs. The rest is on the performance page |
| 5 | Release 0.16.0. Before the tag, the npm channel of the CLI (0.14 #7) is settled or left out of the release | The release checklist; the performance page shows the new numbers | **done**: the native, WebAssembly and allocator tables measured again, with the two new cases; the npm channel of the CLI is left out (0.14 #7) |

## 0.17: a Python package

JSONata in Python has a pure-Python port, a wrapper of jsonata-js and a young Rust port of
jsonata-java. jlens would bring its conformance, audits, schemas and speed to Python, where data
pipelines and rules live, and more people using it would tell 0.18 which parts of the API to
keep. The package follows the WebAssembly package: the same objects, the same errors, beta until
1.0.

| # | Deliverable | Definition of done | Status |
|---|---|---|---|
| 1 | ADR 0014, the Python package, after a spike: PyO3 and maturin under the workspace's `unsafe_code = "forbid"` (or why the binding crate opts out of it); the crate (`crates/jlens-python`), the PyPI name (`jlens`, free) and the module; the API, mirroring the Wasm package; how a Python value is read (one level at a time, as in Wasm through `Source`, against a JSON round trip, measured in the spike); `Document`, which is not `Send`, and free-threaded Python; abi3 wheels and the oldest Python supported | The ADR accepted, with the spike's measurements | **done**: [ADR 0014](../adr/0014-python-package.md). PyO3 0.30 compiles under `forbid`; reading Python values a level at a time is 2 to 17 times faster than `json.dumps` and a parse, and as fast when the whole input is read; `Document` holds the parsed JSON and converts it once per thread, since an `unsendable` class panics on another thread; `abi3` wheels from 3.10 and `abi3t` from 3.15 (free-threaded) |
| 2 | Deeply nested values do not overflow the stack. Found by the spike of #1: an input 20,000 levels deep, returned whole (`$`, `$string($)`), overflows it, since the conversion of the result (`Value::to_json_spending`) and the serialization of `$string` recurse without `stack::grow`. So do values built by an expression (1,000 levels returned; a tail-recursive function builds any depth, and dropping it overflows), and every other recursive walk over values | Robustness cases on a `Source` input 100,000 levels deep, on values built that deep, and on the WebAssembly package's input, return or fail cleanly | **done**: values nested more than 1,000 levels deep fail with `U1001` where they are converted to JSON, compared or serialized (jsonata-js raises a `RangeError` before 5,000); the walks that only enter arrays keep their own stack and have no limit; arrays and objects are dropped without recursion |
| 3 | The binding: `Expression` (compile, `evaluate` reading Python values as it goes, `evaluate_json`), `Document`, bindings, the options and limits (timeout, size caps, clock, seed), JSONata errors as an exception with `code`, `position` and `token`; type stubs (`.pyi`, `py.typed`) | A pytest suite in CI on Linux, macOS and Windows; deep or cyclic values fail cleanly, never crash | **done**: `crates/jlens-python`, tested on Python 3.10 and 3.14 on each OS and on free-threaded 3.14 on Linux. A cycle, or a value JSON cannot hold, fails with `U1004` when the level that holds it is read; input nested more than 1,000 levels fails with `U1001` where it is converted; a `Document` and the bindings are converted whole and raise `TypeError` or `ValueError`; type stubs checked by `stubtest` |
| 4 | Host functions written in Python, with their signatures, calling the functions they receive (ADR 0011), and a Python exception carried through the evaluation; audits and explanations as Python values | pytest cases matching the host-function and audit cases of the Wasm suite | **done**: `register_function` and callables in the bindings, with an arity read from the parameters; a JSONata function reaches them as a `jlens.Function`, valid during the call only (`U1005` after it, or from another thread); a Python exception, even one that is not an `Exception`, comes out of the evaluation unchanged; `evaluate_with_audit` returns a `dict` typed by `jlens.AuditResult`, with spans in code points. Tested with the GIL and on free-threaded 3.14 |
| 5 | Wheels: `abi3` and `abi3t` wheels for the release targets (ADR 0014) and a source distribution, built in CI; each OS installs its wheel and runs the pytest suite on it | CI builds and tests every wheel | **done**: `.github/workflows/wheels.yml`, on each push to `main`: for the seven release targets (manylinux 2.28 and musllinux 1.2 on x86-64 and ARM64, macOS x86-64 and ARM64, Windows x86-64), a `cp310-abi3` wheel tested on Python 3.10 and a `cp315-abi3.abi3t` wheel tested on free-threaded 3.15 (musllinux in an Alpine container); the source distribution, tested on free-threaded 3.14. The package carries its `LICENSE` |
| 6 | Publishing: `release.yml` publishes to PyPI through trusted publishing, with attestations, skipping a version already published; `CONTRIBUTING.md` says how it is set up | The release checklist installs the package from PyPI and checks its attestations | **done**: the `Release` workflow builds and tests the wheels with the binaries, publishes nothing unless both build, then uploads the fourteen wheels and the source distribution from the environment `pypi` (tags `v*` only) through trusted publishing, each with a PEP 740 attestation, skipping files already on PyPI; the publisher is registered on PyPI as a pending one until the first release creates the project. `CONTRIBUTING.md` says how it is set up and how to publish by hand |
| 7 | Documentation: a Python chapter in the book, the installation page and the README; its snippets run in CI; the performance page compares the package with the other Python implementations on `benches/`, and, in a separate table next to the timings, how each one behaves at the limits: input nested deeply, cyclic values, and an evaluation that runs too long | `doc-check` green; the Python snippets run in CI; the limits table is measured, not assumed | **done**: the [Python chapter](../guides/python.md), whose snippets are scripts the pytest suite runs; `benches/compare/python.py` measures `jsonata-python`, `jsonata` and `jsonata-rs` against jlens, which is the fastest on every case (1.5 to 14 times faster than `jsonata-rs` with Python values in, 6 to 1,000 times faster than `jsonata-python`), and fails with a JSONata error at every limit, where `jsonata-rs` crashes the interpreter on deep or cyclic input. Measured on Linux: `jsonata` does not load jsonata-js on Windows |
| 8 | The WebAssembly package leaves the values an audit captures as they are. Found by #4: `evaluateWithAudit` converts the spans of the tree to UTF-16 with `Offsets::convert_spans`, which rewrites every `span` or `range` object it meets, so a captured value (`input`, `output`, `operands`, rejected samples) holding a field `span: {start, end}` comes out changed when the expression is not ASCII, its offsets converted and clamped to the source's length. The conversion walks the audit's nodes only, as the Python package does (`audit_spans_to_chars`); `ast()` and `parseRecover` keep their walk, since their trees hold no user values | A case in `tests/suite.mjs` with a captured `span` field and a non-ASCII expression; `cargo xtask wasm-test` green; the fix in `CHANGELOG.md` | |
| 9 | Release 0.17.0, with the Python package in beta | The release checklist, with the package installed from PyPI; the performance page measured again for 0.17.0: the native, allocator and WebAssembly tables and the numbers in "Where the time goes" | |

## 0.18: an API to keep, toward 1.0

After seventeen releases, nothing says which parts of jlens are meant to last. `cargo
semver-checks` runs in CI, but before 1.0 it only asks for a new minor version, and
`jlens-core` exposes workspace plumbing (`stack`, `debug`, the AST's internals) next to its API.
0.18 decides what 1.0 will promise, breaks the API once to get there, and writes the promise
down. Applications built on jlens are separate projects: they use `jlens-core` as any
application does, and what they lack comes back here as general features.

**Deferred** until applications built on jlens have used it: what to keep stable is decided
from what they needed, missed or worked around, not guessed beforehand.

| # | Deliverable | Definition of done | Status |
|---|---|---|---|
| 1 | ADR 0015, what 1.0 promises. Exploration: list the public items of `jlens-core`, `jlens-schema` and `jlens-lsp` (rustdoc JSON or `cargo public-api`) and the contracts outside Rust: the Wasm API (`types.d.ts`), the Python package, the CLI (ADR 0008), the JSON of audits and explanations, `jlens.json`, the error codes. For each item: stable, hidden (`#[doc(hidden)]`, `pub(crate)`) or `#[non_exhaustive]` where it may grow. What semver covers and what it does not (messages, `Display`, `Debug`, performance, jsonata-js quirks), the MSRV policy, deprecations | The ADR accepted, with the list of changes that #2 to #4 make | |
| 2 | `jlens-core` changes as ADR 0015 says, in one breaking batch | `cargo semver-checks` reports only the intended breaks; the changelog has a migration section; the crate guide's snippets updated | |
| 3 | `jlens-schema` and `jlens-lsp` changes as ADR 0015 says | As #2 | |
| 4 | The contracts outside Rust, as ADR 0015 says: the Wasm API, the Python package, the JSON of audits and `jlens.json`; a test pins each, so that a change shows in review | Tests that fail on an unintended change of each contract | |
| 5 | A stability page in the book, from ADR 0015, and the rules for changing a stable item in `CONTRIBUTING.md` | `doc-check` green; the page linked from the introduction | |
| 6 | Release 0.18.0, the release that breaks once. 1.0.0 follows when ADR 0015's criterion is met | The release checklist, with the migration notes in the release notes | |

## Not planned

- **Asynchronous functions.** A host function that returns a promise fails with `U1002`, and
  jlens does not plan to change that. Supporting it needs an evaluator that can suspend, which
  would slow down every evaluation, and only WebAssembly would gain from it: a Rust host
  function can block. An application can fetch what its functions need before evaluating.
  Should real use call for it, the cheaper way is an `evaluateAsync` that stops at the first
  promise, awaits the pending ones and evaluates again with their results.
