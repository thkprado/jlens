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
| 3 | Released packages prove where they were built: the npm package is published with provenance, and the binary archives carry build attestations | The next release shows provenance on npm and attestations on GitHub; CONTRIBUTING says how to verify them | in progress |

## Later

Not commitments, in order of priority:

- **Host functions that call back** the JSONata functions they receive as arguments, through a
  callable handle. This extends the host function API of ADR 0006 without changing the
  functions already written against it.
- **The CLI from npm**, as a `bin` in the WebAssembly package that runs under Node.js, so that
  `npx` runs it where no Rust toolchain or prebuilt binary is available.
- **A VS Code extension** that starts `jlens lsp` for `.jsonata` files, with a TextMate grammar
  for syntax highlighting. The other editors with an LSP client need no extension.
- **Lazy conversion of input documents**, so that evaluations that read a small part of a large
  document do not pay for converting all of it. This changes the value model
  ([ADR 0003](../adr/0003-value-model.md)). `Document` already lets an application convert a
  document once for many evaluations.
- **Asynchronous functions**, as in jsonata-js, whose functions may return promises. This needs
  an evaluator that can suspend, and an ADR first. Until then, a host function that returns a
  promise fails with `U1002`.
