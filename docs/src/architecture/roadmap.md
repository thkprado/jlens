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

## Priorities

Three principles order the work after 0.2:

1. **The core is a library for custom integrations.** Applications embed `jlens-core` in their
   own pipelines. It must stay small and decoupled, let the application set every limit, never
   panic, and change its API only on purpose.
2. **The CLI is a tool for automation and AI agents.** It never prompts, it gives the same
   output for the same input, and it has one documented machine-readable contract: what goes to
   standard output, what goes to standard error, and what each exit code means.
3. **Editors come after.** The language server is done; the VS Code extension waits.

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
| 4 | A string literal does not allocate each time it is evaluated. Today it builds a new `Rc<str>`, so `items[type = 'electronic']` allocates one string per item. Exploration: a string shared by the evaluation, or a comparison with the literal that builds no value | The allocation gone and measured with `ab`, or the cost and the options recorded on the performance page | |
| 5 | The result is converted in one pass. Converting the whole document as a result takes 210 µs, against 152 µs in jsonata-core: its size is measured in a first walk, then it is converted in a second | The size counted while converting, stopping at `Options::max_result_size` as before; the property tests of the size cap pass unchanged; a large result faster with `ab` | |
| 6 | The fixed cost of an evaluation is cut. `1` takes 0.35 µs, against 0.07 µs in jsonata-core: the clock is read twice for `$now` and the seed of `$random`, and the root frame and the binding of `$` allocate | The clock read once, and only what an evaluation uses set up; the path case faster with `ab` | |
| 7 | The performance page shows the new numbers, natively and in WebAssembly, with the comparison with jsonata-js | The tables measured again; the "Where the time goes" section updated | |

## Later

Not commitments, in order of priority:

- **Host functions that call back** the JSONata functions they receive as arguments, through a
  callable handle. This extends the host function API of ADR 0006 without changing the
  functions already written against it.
- **The CLI from npm**, as a `bin` in the WebAssembly package that runs under Node.js, so that
  `npx` runs it where no Rust toolchain or prebuilt binary is available.
- **A VS Code extension** that starts `jlens lsp` for `.jsonata` files, with a TextMate grammar
  for syntax highlighting. The other editors with an LSP client need no extension.
- **Asynchronous functions**, as in jsonata-js, whose functions may return promises. This needs
  an evaluator that can suspend, and an ADR first. Until then, a host function that returns a
  promise fails with `U1002`.
- **A faster allocator for the CLI.** Evaluation spends most of its time allocating. On
  Windows, linking the `jlens` binary with `mimalloc` made every case of `benches/` 20 to 45%
  faster, and `Document::parse` went from 180 to 121 µs; `evaluate` on the filter, strings
  and regex cases took half the time, and the cost lazy input left on them fell from 5–8% to
  1–3% (see the performance page). Worth doing only if Linux shows a
  similar gain, measured first. The library leaves the global allocator to the application, so
  the performance page would recommend it to embedders. `mimalloc` builds C code, so each
  release target and `cargo-deny` need checking.
