# Roadmap history

What each plan of the [roadmap](roadmap.md) set out to do and what it shipped, one paragraph per
release. The [changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md) lists every
change. Item numbers cited elsewhere, such as "0.27 #6" in an ADR or a performance note, are rows
of the plan's table: the roadmap at the release's tag holds it
(`git show v0.27.0:docs/src/architecture/roadmap.md`), and each item's commits name it
(`git log --grep "Roadmap 0.27 #6"`).

## How 0.1 was built

jlens was built in phases. A phase was complete only when its documentation was written.

| # | Deliverable | Definition of done |
|---|---|---|
| 0 | Workspace, CI, documentation skeleton, conformance harness | CI green; conformance report |
| 1 | Lexer, Pratt parser, AST post-processing, spans, recover mode, annotations; `jlens ast` / `jlens check` | Syntax groups of the suite and the parser-recovery cases pass |
| 2 | Value model and evaluator (paths, sequences, predicates, operators, lambdas, closures, tail calls, sort, group-by, parent operator, transform, limits) | Suite groups that don't depend on functions pass |
| 3 | Function library, signatures, regex, date/time and number formatting | 100% of the suite minus documented overrides |
| 4 | Evaluation logging and auditing (tracer, audit tree, levels, annotations, explanations) | Snapshot tests; under 3% overhead with the tracer off |
| 5 | `jlens-schema`: inference, overrides, JSON Schema, static typing, completion/diagnostics/hover | Table-driven language-service tests |
| 6 | Full CLI | CLI snapshot tests |
| 7 | WebAssembly package (browser ESM), TypeScript types, web editor example | Headless browser tests; working demo |
| 8 | Documentation consolidation, benchmarks, publishing | Doc examples verified; link check green; published to crates.io and npm |

0.1.0 was the first release: the engine, audits, schemas and the language service, the CLI and
the WebAssembly package. 0.1.1 moved the examples, fixtures and web editor to an online store
order. ([0.1.0](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#010),
[0.1.1](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#011))

## 0.2

Host functions in Rust and WebAssembly ([ADR 0006](../adr/0006-host-functions.md)), and their
declarations in the language service; the web editor's example selector.
([Changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#020))

## 0.3

*A machine contract and core guardrails.* The first plan after the [priorities](roadmap.md#priorities)
were set. It shipped the language server `jlens lsp` and `jlens.json`
([ADR 0007](../adr/0007-language-server.md)), the CLI's machine contract
([ADR 0008](../adr/0008-cli-machine-contract.md)): `eval --json`, `--now` and `--seed`, `check`
over several files, `jlens errors`; and cancellation and size caps in `jlens-core`.
([Changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#030))

## 0.4

*Catching mistakes before they run.* `check` warns about unknown functions, wrong argument
counts and types, and undeclared variables, and reads `jlens.json`; `jlens test` runs files of
test cases; prebuilt binaries install with `cargo binstall`.
([Changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#040))

## 0.5

*Safe releases, enforced limits and proven compatibility.* A cap on the size of a result
(`U1001`); the size caps, clock and seed in the WebAssembly package and the CLI; differential
testing against jsonata-js (`cargo xtask differential`); every binary built before a release
publishes anything; a security policy.
([Changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#050))

## 0.6

*Bounded regular expressions, wider differential tests, signed packages.* Regular expressions
bounded by the timeout and cancellation ([ADR 0009](../adr/0009-bounded-regex.md)), on
`jlens-regress`, a fork of `regress` 0.12.0; differential tests that reach regular expressions,
pictures and decoders; npm provenance and build attestations for the binaries.
([Changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#060))

## 0.7

No plan of its own: three jsonata-js quirks that expose JavaScript stopped being reproduced (a
regular expression applied to a value that is not a string, irregular grouping in
`$formatNumber`, `$substringBefore` and `$substringAfter` without `chars`), as
[compatibility](../language/compatibility.md) records.
([Changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#070))

## 0.8

*Proven robustness and fast single evaluations.* Coverage-guided fuzzing in a weekly workflow,
and the panics, stack overflows and slow cases it found fixed; `jlens lsp` survives a panicking
request; `evaluate` reads its input lazily, one level at a time
([ADR 0010](../adr/0010-lazy-input.md)), and `evaluate_source` reads other representations.
([Changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#080))

## 0.9

*Numbers and dates at the edges, and tools for the next steps.* A number that overflows to
infinity and a date beyond JavaScript's range fail with `D1001` where they arise; `$formatNumber`
and `$formatBase` write every digit of a large number; loops over floating-point numbers end on
NaN and infinity; `cargo xtask fuzz-replay` replays fuzz inputs on any platform.
([Changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#090))

## 0.10

*Performance and robustness debts*, taken as explorations that end in a fix or in the cause
written down. A recursion through a higher-order function fails with `D1011` instead of
overflowing the stack; WebAssembly's default `maxDepth` is 500; `cargo xtask ab` compares the
speed of two revisions.
([Changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#0100))

## 0.11

*Faster evaluation of common shapes.* A comparison with
[jsonata-core](https://github.com/txjmb/jsonata-core), another Rust port, found it 2 to 5 times
faster on five cases, which `eval.rs` traced to work no result depends on. Each item removed one
such cost, measured with `cargo xtask ab` and with no change to results, errors or evaluation
order: field lookups on a single value, comparisons with a string literal, order-by keys
evaluated once, blocks without frames, the result converted to JSON in one pass, less set-up
per evaluation. jsonata-core's second evaluator for simple expressions was left out, as two
evaluators of one language can disagree. 0.11.1 made `$fromMillis` and `$now` fail with `D3150`
on a timezone that is not a number.
([0.11.0](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#0110),
[0.11.1](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#0111))

## 0.12

*jlens online.* The book and the web editor published on GitHub Pages at each release tag, built
from the tag's files, so that the editor runs the engine its book describes; the editor
evaluates in a Web Worker, everything in the browser, and shares expressions by address.
([Changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#0120))

## 0.13

*Host functions that call back.* A host function can call the JSONata function it receives, in
Rust (`HostCall::function`, `Callable`) and in WebAssembly, within the evaluation's limits
([ADR 0011](../adr/0011-host-callbacks.md)), without changing the functions written against
ADR 0006.
([Changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#0130))

## 0.14

*More ways to install the CLI*, in order of reach, each published by `release.yml` from the
attested archives, never rebuilt: npm (`jlens-cli`, [ADR 0012](../adr/0012-npm-distribution.md)),
Homebrew, Scoop and the container image `ghcr.io/thkprado/jlens`.
([Changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#0140))

## 0.15

*jlens in VS Code, and a faster CLI allocator.* The extension `thkprado.jlens` on the Marketplace
and Open VSX, with the release binary bundled ([ADR 0013](../adr/0013-vscode-extension.md)), and
mimalloc as the allocator of the `jlens` command.
([Changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#0150))

## 0.16

*The last gaps with jsonata-core, and string literals.* Measured again after 0.15.0, jlens led
on seven of ten cases. Group-by stopped copying each group at every item, and paths of names,
literal group keys and names mapped over many items got faster, closing the gaps.
([Changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#0160))

## 0.17

*A Python package.* The package `jlens` on PyPI, in beta ([ADR 0014](../adr/0014-python-package.md)),
following the WebAssembly package: the same objects, the same errors. A path step reads its one
field of an input object, and values nested thousands of levels deep no longer overflow the
stack.
([Changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#0170))

## 0.18

*The fields an expression reads*, from the expression alone
([ADR 0016](../adr/0016-fields-read.md)), in Rust, WebAssembly, Python and the CLI, for one
expression or the union of several: an upper bound, exact for the paths, comparisons and
aggregations rules are mostly made of, tested by evaluating each suite case on its input cut to
the fields reported. JSON text is parsed without the fields no evaluation reads.
([Changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#0180))

## 0.19

*Closing on jsonatapy*, which was faster on four of twelve Python cases. Profiles found three
causes, each removed: results are written straight into Python and JavaScript values
([ADR 0017](../adr/0017-result-builder.md)); predicates of literals are evaluated once, object
constructors over one item skip grouping, and `$formatNumber` is faster; in Python, keyword-only
options, interned keys and mimalloc.
([Changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#0190))

## 0.20

*A deterministic step limit.* The timeout reads the clock, so the same evaluation can pass on one
machine and fail on another. `max_steps` (`U1006`) gives the same answer everywhere, with or
without a tracer, in Rust, WebAssembly, Python and the CLI
([ADR 0018](../adr/0018-step-limit.md)); the timeout now reads the clock every 256 steps; `$eval`
lets the limits of its text through; WebAssembly options are read without JSON.
([Changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#0200))

## 0.21

*Documents built from Rust values.* An application that rebuilt its document often spent more
on `serde_json::Value` and `Document::new` than on evaluating. `Document::from_serialize` builds
the document straight from Rust values, and `evaluate_document_with` binds documents or parts of
one to variables without converting them ([ADR 0019](../adr/0019-documents-from-rust-values.md)).
0.21.1 published the command on npm, `npx jlens-cli`, once npm allowed the name of its Windows
package, and 0.21.2 moved the npm packages to trusted publishing, without a token.
([0.21.0](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#0210),
[0.21.1](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#0211),
[0.21.2](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#0212))

## 0.22

*Regular expressions on jlens's own terms.* `jlens-regress` kept as jlens's own fork
([ADR 0020](../adr/0020-own-regress-fork.md)), with upstream's fixes ported; checked against V8 by
a corpus of test262's answers and a differential test, which found ten bugs; a benchmark of the
engine, and a cheaper interrupt.
([Changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#0220))

## 0.23

*Faster lookarounds*, the cases of the engine's benchmark furthest from V8: the matcher reuses
their backtracking stacks and saved groups instead of allocating them each time, halving their
time.
([Changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#0230))

## 0.24

*Reading the application's values faster*, two costs of `jlens-core` that ADR 0021's spike
found. An index into a long array reads the items it picks, alone, through `Source::read_item`,
in Rust, WebAssembly and Python, whether it is a number or held in a variable; a predicate reads
its array without copying it; a group-by on a tail call applied. 0.24.1 moved the Python package
to mimalloc 2 outside Windows.
([0.24.0](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#0240),
[0.24.1](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#0241))

## 0.25

*A Java package.* `io.github.thkprado:jlens` on Maven Central, in beta, for Java 25 and later
([ADR 0021](../adr/0021-java-package.md)): one JAR with the libraries of seven targets, over a C
API that Java calls through Panama, with a benchmark against the JVM implementations in the
repository; host functions looked up when an expression names them.
([Changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#0250))

## 0.26

*What measuring 0.25 left open*: a regression in `recursion`, a case slower than jsonata-js, and
the cases where the JVM comparison put jlens behind. A lambda's frame is kept in a list that
shares its arguments, `recursion` 21 to 25% faster; `$clone` copies without the JSON text, the
transform 40 to 52% faster; WebAssembly and Java read the fields an expression reads, in one
call; Java threads find their handles without a shared lock. Every comparison runs a new copy of
each build at each run, and what such a comparison can settle was measured on Windows and Linux.
How it was measured is in
[`docs/notes/perf-0.26.md`](https://github.com/thkprado/jlens/blob/main/docs/notes/perf-0.26.md).
([Changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#0260))

## 0.27

*What use asked for, and what 0.26 left open.* Applications met variables that cost more against
a `Document`, a quadratic `$distinct` and a WebAssembly package that a page opened from disk
could not load. Linear where the square of the input showed: `$match`, `$replace` and `$split` on
long texts, `$distinct`, `$keys`, `$substring`, `&` in a loop, `in` and the URL functions, found
in use and by `cargo xtask growth`; variables against a `Document` converted as far as the
expression reads them; the Java and WebAssembly packages built with link-time optimization; the
WebAssembly package as one classic script, `jlens.global.js`; three new engines in the
comparisons. How it was measured is in
[`docs/notes/perf-0.27.md`](https://github.com/thkprado/jlens/blob/main/docs/notes/perf-0.27.md).
([Changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#0270))

## 0.28

*What 0.27 left in `$distinct` and `$match`, and the regex flags `u` and `v`.* `$distinct`
follows the first fields of its items into the levels below them when the items share a field or
are arrays, 25 records of 100 KB that differ there 15 µs against a `Document` where they took
12 ms, and compares numbers and short strings inline, 100,000 items of eight values 312 µs where
they took 2.2 ms; the searches of a text keep the backtracker's storage, and matches without
groups share one empty array, `$match` 6.5 → 4 allocations a match and `regex over a long text`
36% faster. The regex flags `u` and `v`, an extension that jsonata-js rejects, bring the Unicode
properties and the set operations on classes; their emoji example found a bug in
`jlens-regress`, fixed. How it was measured is in
[`docs/notes/perf-0.28.md`](https://github.com/thkprado/jlens/blob/main/docs/notes/perf-0.28.md).
([Changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#0280))

## 0.29

*Completing values, and domains the application provides.* The language service, which completed
fields, variables, functions and operators but never a value, completes the values of an `enum`
or `const` on the right of `=`, `!=` and `in`, inside quotes too, and warns as `W0206` about a
literal outside a declared one; an enumeration inferred from samples is marked
`x-jlens-enum-inferred` and never warned about. A field whose values only the application can
fetch names an open domain (`x-jlens-domain`, or `domain` in the overrides), and completion
returns a value request for the application to answer. The web editor gained a schema overrides
panel and a "Free shipping" example that uses labels, a hidden field and an open domain; an
override whose path the samples lack, often a misspelt one, is listed with the path it most
likely meant. `jlens_core::Offsets` made public the conversion between byte and UTF-16 offsets
that the Wasm package kept to itself. No evaluation code changed, and the tables moved within the
floor. How it was measured is in
[`docs/notes/perf-0.29.md`](https://github.com/thkprado/jlens/blob/main/docs/notes/perf-0.29.md).
([Changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#0290))

## 0.30

*Value requests with their context.* 0.29's value request said which field's domain to fetch,
but not what the expression already said about it. A request now carries its conditions: the
fields compared with literals by `=` or `in` and joined to the value by `and` in the same
predicate, read from the unfinished expression by the recovering parser, so that a domain can
depend on another (`categories[group = 'X' and subgroup = '|` asks for the subgroups of `X`). It
says whether the value is an element of `in [...]`, where several may be inserted, and lists the
literals the array already holds, which the application leaves out as the service leaves out
declared values. The web editor's "Open domains" example shows each. A schema override's `type`
takes one type name, as JSON Schema writes it, besides a list. No evaluation code changed, and the
tables moved within the floor but for cases that draw one of two speeds. How it was measured is in
[`docs/notes/perf-0.30.md`](https://github.com/thkprado/jlens/blob/main/docs/notes/perf-0.30.md).
([Changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md#0300))
