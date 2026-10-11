# Conformance testing

jlens vendors the language-neutral test suite of the reference implementation
([jsonata-js](https://github.com/jsonata-js/jsonata) **2.2.2**, MIT) in `tests/jsonata-suite/`.
That covers 102 groups and about 1,690 cases. The harness lives in
`crates/jlens-core/tests/conformance.rs` and mirrors `test/run-test-suite.js`:

- Each case is one test, named `<group>/<file>`, or `<group>/<file>#<index>` for files that hold
  an array of cases.
- The input comes from `data`. If there is no `data`, the input is the named `dataset`;
  `"dataset": null` means *undefined* input.
- `timelimit` and `depth` map to `Options::timeout` and `Options::max_depth`.
- Each case states exactly one expectation:
  - `result`, compared with deep equality where numbers compare as JavaScript numbers, so `1`
    equals `1.0`;
  - `undefinedResult`;
  - `code`, optionally with `token`;
  - `error`, an object whose fields must all match.

## The baseline ratchet

`tests/conformance/baseline.txt` lists the cases that must pass, and `cargo test` fails only if
one of them regresses. Every other case is reported as *ignored*. To work on a group, run it
with:

```sh
cargo test -p jlens-core --test conformance -- --include-ignored array-constructor/
```

To measure progress and lock in new passes:

```sh
cargo xtask conformance                    # writes tests/conformance/REPORT.md
cargo xtask conformance --update-baseline  # also rewrites baseline.txt
```

## Overrides

`tests/conformance/overrides.json` records every intentional deviation. Each entry needs a
`reason` and may `skip` the case, or replace its expectation with `alternateResult` or
`alternateCode`. The current overrides are:

| Case | Reason |
|---|---|
| `function-encodeUrl/case002`, `function-encodeUrlComponent/case002` | The data contains a lone UTF-16 surrogate (U+D800), which Rust's UTF-8 strings cannot represent. |

## Property tests

The suite checks that jlens gives the right answers. Property tests (`proptest`) check that no
input makes it crash, whatever the input:

| Test | What it generates | What must hold |
|---|---|---|
| `jlens-core/tests/robustness.rs` | Valid expressions from a small grammar, near-valid token soups, and arbitrary strings | Parsing (strict and recovering), evaluation, evaluation against a `Document`, and audited evaluation never panic; diagnostic spans stay inside the source |
| `jlens-schema/tests/robustness.rs` | Token soups and arbitrary strings | Completion, diagnostics, hover and signature help never panic at any byte offset; ranges stay inside the text |

They run 2,000 and 1,000 cases with `cargo test`. Set `PROPTEST_CASES` for longer runs:
`PROPTEST_CASES=100000 cargo test -p jlens-core --test robustness`.

The property tests found two bugs:
- In recovering parsing, an out-of-range number (`1e310`) looped forever.
- A transform such as `|items|{'p': $}|` built an object containing itself, which then
  overflowed the stack when converted to JSON. It now fails with `U1001`.

Both have regression tests.

## Fuzzing

Property tests draw their inputs at random. Coverage-guided fuzzing (`cargo-fuzz`, with
libFuzzer) keeps the inputs that reach new code and mutates them, so it finds the paths that
random inputs rarely reach. The targets live in `fuzz/`, a crate outside the workspace:

| Target | Input | What must hold |
|---|---|---|
| `parse` | An expression | Strict parsing never panics |
| `parse_recover` | An expression | Recovering parsing never panics; diagnostic spans stay inside the source |
| `evaluate` | An expression, optionally followed by a NUL byte and a JSON input document | Compiling, then evaluating with and without input, against a `Document` and audited, never panics |
| `regex` | A byte of flags, then a pattern and a text separated by a NUL byte | Compiling the pattern with `jlens-regress` and finding every match never panic; groups lie on character boundaries of the text |
| `pictures` | A picture, optionally followed by a NUL byte and a second string | `$formatNumber`, `$formatInteger`, `$parseInteger`, `$fromMillis` and `$toMillis` never panic on the picture, the text they parse or the time zone |
| `language_service` | Two bytes of offset, then a text | Diagnostics, completion, hover and signature help never panic, at any offset; their ranges lie inside the text |

Evaluations run with a timeout, a depth limit, size caps, and a fixed clock and seed. Each
target runs on a thread with a 1 MB stack, the size of the main thread's stack on Windows, so
that recursion that does not grow the stack overflows on Linux too. The build has debug
assertions, so an arithmetic overflow is a crash.

The `Fuzz` workflow runs every target for ten minutes each week, or for as long as asked when
started by hand. Each target starts from seeds that `cargo xtask fuzz-seed` writes to
`fuzz/corpus/`: the expressions and input documents of the JSONata suite, and a few regular
expressions and pictures. It also starts from the corpus its last run kept. A crash fails the
run, and the input that caused it is uploaded with the run.

cargo-fuzz needs nightly Rust and Linux or macOS. To reproduce a crash, on Linux:

```sh
cargo install cargo-fuzz
cargo xtask fuzz-seed
cargo +nightly fuzz run evaluate --sanitizer none --debug-assertions            # fuzz
cargo +nightly fuzz run evaluate --sanitizer none --debug-assertions <crash file>  # one input
```

Each crash becomes an ordinary test in the crate where the bug was, which runs with
`cargo test` on every platform.

## Differential testing

The suite covers the cases its authors thought of. Differential testing covers others: it
generates expressions and runs them in jlens and in jsonata-js 2.2.2, which must agree on the
result or, when the expression fails, on the error code.

```sh
(cd benches/compare && npm install)
cargo xtask differential                     # 1,000 cases, from a seed it prints
cargo xtask differential --cases 10000 --seed 42
```

The expressions come from a grammar like that of the property tests, without the functions that
read the clock or draw random numbers. The grammar also calls the functions that were hardest to
port with meaningful arguments: regular expression literals for `$match`, `$replace`, `$split`,
`$contains` and `~>`, valid pictures for `$formatNumber`, `$formatInteger` and `$fromMillis`,
round trips through `$parseInteger`, `$toMillis` and the decoders. Each case runs against one of a few input documents, or
none. The command writes the cases to `target/differential/cases.json`.
`benches/compare/differential.mjs` runs them in jsonata-js. The command then compares each
outcome with that of jlens. Results are compared as JSON, with numbers compared as values and
the keys of an object in order. Both engines run with a timeout and a depth limit, and a case
that reaches either limit is skipped. The divergences are printed and written to
`target/differential/divergences.json`. The command fails when there are any. A seed
reproduces a run.

Eight kinds of disagreement are known differences of jsonata-js, listed in the
[compatibility notes](../language/compatibility.md). They are not divergences; the command
counts them apart:

- **jsonata-js crashed**: it threw a JavaScript exception, which has no error code to compare.
- **jsonata-js hung**: the case ran for ten times the timeout, or ran out of memory, in a
  synchronous loop that the jsonata-js timeout cannot interrupt (`$match('abc', /b/) ?? 1`).
  The runner evaluates the cases in a worker thread, stops it, and goes on with the next case
  in a new one.
- **jsonata-js split a character**: a string of the jsonata-js result holds a lone surrogate,
  half of a character outside the Basic Multilingual Plane, which a jlens string cannot hold. A
  JavaScript regular expression without the `u` flag matches UTF-16 code units, so
  `$match('a𝄞', /[^a]/)` matches the first half of `𝄞`; jlens matches the whole character.
- **Regex left of `??` or `?:`**: the outcomes differ, and the expression has a regular
  expression literal on the left of `??` or `?:`. The jsonata-js parser copies that side through
  JSON, which turns the regex into `{}`, so the expression hangs (counted above) or gives another
  result.
- **Concurrent errors**: both engines failed with different codes, and the expression has an
  array constructor with several items or an object constructor. jsonata-js evaluates those
  parts concurrently, so when two of them fail it reports whichever fails first in its
  asynchronous schedule. jlens reports the first in source order.
- **jsonata-js changed its input**: the jsonata-js runner saw the input document change during
  the evaluation (a group-by on an empty array adds an item to it), and the outcomes differ.
- **Wildcard over a sequence**: the expression has a wildcard, and the jsonata-js result is
  jlens's with `true` items added. An array that jsonata-js builds from a path carries an own
  property `sequence: true`, which its wildcard enumerates with the items.
- **Dropped quirk**: the expression may meet a jsonata-js quirk that jlens does not reproduce:
  a regular expression literal applied with `~>` to anything but a string literal,
  `$formatNumber` with a literal picture whose integer part has irregular grouping
  (`'#,##,##0'`), or `$substringBefore` / `$substringAfter` whose `chars` is not a string
  literal.

The `Differential` workflow runs 20,000 cases from a new seed every week, and on demand with a
chosen number of cases and seed. When it fails, the cases and outcomes are attached to the run.
