# Changelog

All notable changes are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.13.0]

Host functions call the functions they receive (ADR 0011), in Rust and in WebAssembly: a helper
written in the host language can apply a lambda, a built-in function or a partial application
passed to it, within the evaluation's limits.

### Added

- The site links the repository: the book from its menu bar, the web editor from its header.
- Host functions call the functions they receive (ADR 0011). In Rust, `HostCall::function(i)`
  gives a `Callable` for an argument that is a function (a lambda, a built-in function, a
  partial application, a chain or a host function); `Callable::call` applies it to JSON
  arguments, within the evaluation's depth, timeout, cancellation and size limits.
- In WebAssembly, a JavaScript host function receives a JSONata function argument as a
  JavaScript function that calls it, with an `arity` property, as in jsonata-js (`Callable` in
  the TypeScript declarations). It is synchronous, and calling it after the host function has
  returned throws the new error `U1005`.
- The web editor offers a host function, `$try(step, fallback)`, which calls the function it
  receives and catches its error, and a Host functions example that uses it.

### Changed

- In WebAssembly, a function passed to a host function no longer arrives as `undefined`: a host
  function that tested for `undefined` sees a function instead.

## [0.12.0]

jlens is online: the book and the web editor are published at
<https://thkprado.github.io/jlens/>, built from each release tag. The editor evaluates in a Web
Worker and shares expressions by address, without a server.

### Added

- `cargo xtask site` builds the book and the web editor as one static site in `target/site`,
  with the editor in `editor/`; `cargo xtask serve --site` previews it. The web editor imports
  the package by its name, `jlens-wasm`, through its import map.
- The book and the web editor are published at <https://thkprado.github.io/jlens/> by the new
  `Pages` workflow, at each release tag. The crates and the npm package name the site as
  their homepage.
- The web editor shares expressions: its Share button puts the expression, the input document
  and the bindings in the address (`#s=`, compressed), so that no server stores them, and
  opening that address restores them. Its header links the book.

### Changed

- The web editor evaluates in a Web Worker, so a slow expression no longer blocks typing; an
  evaluation still running when the expression changes is dropped.

## [0.11.1]

A fix for dates formatted with a timezone that is not a number.

### Fixed

- `$fromMillis` and `$now` fail with the new code `D3150` when the timezone does not start with
  a number (`$fromMillis(0, undefined, "two")`). They wrote a date with a garbled offset
  ("1970-01-01T00:00:00.0000N:aN"), where jsonata-js writes "NaN" in every component (see the
  [compatibility page](docs/src/language/compatibility.md#evaluation)).

## [0.11.0]

Common expressions evaluate up to three times faster: a field of a single value is a lookup,
comparisons with a string literal build no value, order-by evaluates each key once, and an
evaluation sets up only what it uses. Against a `Document` converted once, the WebAssembly
package is faster than jsonata-js on every case of the
[performance page](docs/src/architecture/performance.md).

### Changed

- The order-by operator (`items^(price)`) evaluates each item's keys once, on first use, instead
  of evaluating both items' keys at every comparison: the sort + map benchmark takes 842 µs
  instead of 1.90 ms. Results and errors are unchanged; a key with a side effect, such as one
  using `$random`, is drawn once per item (see the
  [compatibility page](docs/src/language/compatibility.md#evaluation)).
- A block (`(...)`) creates a frame for its variables only when its expressions may bind one:
  `items.(unitPrice)` takes 77 µs instead of 83 µs. `NodeKind::Block` has a new field, `frame`,
  set by post-processing.
- A field name evaluated on a single value (`price` in `items[price > 10]`) is a lookup, without
  the sequences of a general path: the filter + aggregate benchmark takes 56 µs instead of
  161 µs, the shipping rule 152 µs instead of 414 µs, group-by 575 µs instead of 842 µs.
- `=` and `!=` with a string literal (`type = 'electronic'`) compare with the literal's text
  instead of building a string value each time: the filter + aggregate benchmark takes 44 µs
  instead of 56 µs, the shipping rule 127 µs instead of 150 µs.
- The result is converted to JSON in one pass, counting its size against
  `Options::max_result_size` while converting it, instead of measuring it first: returning the
  whole 200-order input (`$`) takes 236 µs instead of 271 µs.
- An evaluation sets up less before it starts: the system clock is read once, the first time
  `$now`, `$millis`, `$random` or `$shuffle` needs it, instead of twice at the start; the start
  time only with a timeout; and the input (`$$`) is kept in the root frame instead of being bound
  as a variable. Evaluating `1` takes 106 ns instead of 350 ns, the path benchmark 676 ns
  instead of 927 ns. Without `Options::now`, the timestamp is the clock when the evaluation
  first needs it rather than when it starts.

## [0.10.0]

Deep recursions fail with `D1011` instead of overflowing the stack: a recursion through a
higher-order function counts its depth, and WebAssembly's default depth limit leaves room for
V8's optimized frames.

### Changed

- `jlens-wasm`: the default evaluation depth limit (`maxDepth`) is 500 instead of 800. Once V8
  has optimized the module, its frames are about 1.6 times larger, and an audited recursion
  through an order-by, an object constructor, a group or a transform overflowed V8's stack below
  depth 800 with a `RangeError`, which leaves the instance unusable, instead of failing with
  `D1011`. The [WebAssembly guide](docs/src/guides/wasm.md#limits-and-memory) lists the depths
  measured.

### Fixed

- A recursion through a higher-order function whose lambda ends in a tail call
  (`function($n){ $map([$n], function($v){ $f($v) }) }`) overflowed the stack instead of failing
  with `D1011`: it aborted a native process, and raised a `RangeError` in WebAssembly. No
  expression stayed on the stack between two levels, so the depth never grew. Each call back
  from a native function (`$map`, `$filter`, `$reduce`, `$sort`, `$replace`...) now counts as a
  level of depth.

## [0.9.0]

Numbers and dates at the edges: a number that overflows to infinity and a date beyond
JavaScript's range fail with `D1001` where they arise, large numbers are formatted with every
digit, and the loops and recursions over floating-point numbers end on NaN and infinity.

### Changed

- A number that overflows to infinity fails with `D1001` where it overflows: the arithmetic
  operators (`1e308 * 10`, `1 / 0`), `$sum`, `$round` and `$parseInteger`. jsonata-js passes the
  infinity on, and JSON writes it as `null` in a result. `$average` of numbers whose sum
  overflows now gives their mean
  ([compatibility notes](docs/src/language/compatibility.md#evaluation)).
- `$fromMillis` fails with `D1001` for a date beyond JavaScript's range (±8.64e15 ms after the
  time zone), where jsonata-js writes "NaN" for every component. jlens used to write a year
  computed past the range (`$fromMillis(1e17)` was "0843-11-07T09:46:40.000Z") with NaN for the
  day of the year and the week. Within the range every component is exact, also near its ends,
  where jsonata-js writes NaN for the day of the year and the week
  ([compatibility notes](docs/src/language/compatibility.md#evaluation)).

### Fixed

- `$formatNumber` writes the digits of a number from 1e21 (`$formatNumber(1e21, '#,##0.00')` is
  "1,000,000,000,000,000,000,000.00"), where jsonata-js passes JavaScript's exponent notation
  through the picture ("1e,+21.00"). `$formatBase` writes every digit of a large number exactly:
  it used to stop at 2^128 (`$formatBase(1e300, 16)` was a run of `f`), and jsonata-js writes
  zeros below the number's precision in a radix that is not a power of two
  ([compatibility notes](docs/src/language/compatibility.md#evaluation)).
- `$formatNumber` with an exponent picture no longer loops forever on an infinite number (from a
  `Source`, or `1e308 * 10` before the change above), found by reviewing every loop and recursion
  over floating-point numbers.
- Parsing no longer overflows the stack on a lambda whose body is a long chain of conditions
  (`? :`, `?:`, `??`) when the lambda itself sits deep in the expression; found by fuzzing.

## [0.8.0]

Proven robustness and fast single evaluations: coverage-guided fuzzing found and fixed panics,
stack overflows and slow cases, and `evaluate` now reads only the parts of its input that an
expression reaches, natively and in WebAssembly.

### Added

- `Expression::evaluate_source` and the `jlens_core::input` module evaluate an input in another
  representation than `serde_json`: a `Source` writes one level at a time to a `Sink`, when the
  evaluation reaches it. A level that a source fails to read stops the evaluation with the new
  code `U1004` ([ADR 0010](docs/src/adr/0010-lazy-input.md)).

### Fixed

- `jlens lsp` keeps running when a bug makes a request panic: it answers that request with an
  `InternalError`, and logs a notification that panics, instead of stopping.
- Panics found by fuzzing: a date/time picture marker that starts with a comma
  (`$fromMillis(0, '[,2]')`) or gives fractional seconds a name presentation
  (`$fromMillis(0, '[fn]')`, now `D3133`) or a width beyond the string size limit
  (`'[Y,99999999999]'`, now `U1001`), `$parseInteger` with a picture whose digits are not
  ASCII on text with other characters (`$parseInteger('12', '##０')`), `$toMillis` with a year
  of dozens of digits (dates beyond JavaScript's range, ±8.64e15 ms, now give undefined, as in
  jsonata-js), and `$formatNumber`
  rounding a very large number to many decimals (`$formatNumber(1e300, '#.000…')`).
- A stack overflow and an endless loop found by fuzzing: numbers in words of a date field that a
  date beyond JavaScript's range leaves NaN (`$fromMillis(1e17, '[dw]')`, or a huge time zone
  offset), and letters for infinity (`$formatInteger(1e308 * 10, 'a')`). Both now write
  nothing, as roman numerals do; jsonata-js overflows its stack and loops forever.
- A long width with a grouping separator (`$fromMillis(0, '[D0:0,1000000]')`) took quadratic
  time: separators are now inserted in one pass. Found by fuzzing.
- Stack overflows found by fuzzing: recovering from hundreds of unclosed brackets
  (`parse_recover("((((…")`) copied the tree once per level, and a chain of thousands of
  operators (`1 + 1 + … + 1`) built a tree deep enough to overflow the stack when dropped.
- Nested `??` and `?:` no longer make the syntax tree grow exponentially: each copied its left
  side into a condition, so 22 levels of `((a ?: 1) ?: 1)…` built 29 million nodes. They now
  evaluate their left side once (jsonata-js evaluates it twice).

### Changed

- `Expression::evaluate` and the other entry points that take `serde_json` input read it
  lazily, one level at a time, converting only the objects and arrays the expression reaches
  ([ADR 0010](docs/src/adr/0010-lazy-input.md)). A path into the benchmark document takes 1.9 µs
  instead of 115 µs; expressions that read every item cost about as much as before. Variables
  given in `Bindings` are read the same way. `evaluate_document` is unchanged.
- `jlens-wasm`: `evaluate` reads its input object during the evaluation, one object or array at a
  time, instead of converting it through `JSON.stringify` first. A path into the benchmark
  document takes 8 µs instead of 310 µs; the other cases cost the same, within 3%. The input is
  still read as `JSON.stringify` reads it, but a cycle or a `BigInt` throws only when the
  expression reads the object or array that holds it. The package has a `snippets/` directory,
  for the JavaScript that reads the input.
- Converting an input document is about 20% faster, natively and in WebAssembly: the objects of
  a document share their keys instead of allocating each. `$each`, `$sift`, `$keys` and
  transforms no longer copy keys either.
- `jlens-wasm` is built with `opt-level = "s"` and `wasm-opt -Os` instead of `"z"` and `-Oz`:
  evaluations are 5–38% faster, and the module grows from about 410 to 460 KiB gzipped.
- `jlens-wasm`: the default evaluation depth limit (`maxDepth`) is 800 instead of 1,000. While V8
  recompiled the module, an audited recursion near 1,000 levels could overflow the JavaScript
  stack with a `RangeError` instead of failing with `D1011`. An expression nested more than 800
  levels deep needs a larger `maxDepth` to evaluate.
- `$fromMillis` fails with `D3132` for a picture marker whose component is unknown, also when it
  has a presentation (`'[Qx]'`), where it wrote nothing; jsonata-js writes "undefined".
- The nesting limit (1,000 levels, `U1001`) also counts the operators of a chain: a chain of
  more than 1,000 terms, such as `1 + 1 + … + 1`, is rejected. jsonata-js accepts about 2,000
  before its stack overflows.
- `$round` returns a very large number unchanged when rounding it to many decimals overflows
  (`$round(1e300, 20)` is `1e300`); jsonata-js returns `null`.
- `jlens-core`: `??` and `?:` parse to the new `NodeKind::Fallback` (with `FallbackOp`) instead
  of a `NodeKind::Condition`.

## [0.7.0]

Closer to the JSONata documentation: jlens stops reproducing three jsonata-js behaviours that
come from JavaScript rather than from the language.

### Changed

- Three jsonata-js quirks that expose JavaScript are no longer reproduced; the compatibility
  notes list them as differences. A regular expression applied to a value that is not a string
  (`x ~> /re/`) gives undefined again, instead of matching JavaScript's `String(value)`
  (`undefined`, `[object Object]`). `$formatNumber` with irregular grouping writes a separator
  only with a digit on its left: `$formatNumber(1234, '#,##,##0')` is "1,234", not "12,34,".
  `$substringBefore` and `$substringAfter` without `chars` give the string unchanged, instead of
  searching for the text "undefined".

## [0.6.0]

Bounded regular expressions and signed packages: a catastrophic regular expression now stops
within the timeout, the differential tests against jsonata-js reach regular expressions,
pictures and decoders, and the released packages prove where they were built.

### Added

- Released packages prove where they were built. The npm package `jlens-wasm` is published
  with provenance, and each binary archive of a GitHub release has a build attestation
  (`gh attestation verify <archive> --repo thkprado/jlens`); see the installation page.

### Fixed

- A regular expression that backtracks catastrophically, such as `/^(a+)+$/` on a long run of
  `a` followed by `!`, now stops within the evaluation's timeout (`D1012`) or when its
  cancellation token is cancelled (`U1003`). Before, a single match could outlast the timeout by
  minutes or more, in Rust, in the CLI and in WebAssembly. Matches run on `jlens-regress`, a
  fork of `regress` whose backtracker polls the limits (ADR 0009); without a timeout or a
  token, nothing changes.
- Regular expressions run on `regress` 0.12.0 instead of 0.10.5, with upstream's fixes: the
  position restored when a lazy loop backtracks, `i` case folding, nested and negated character
  classes, `\x` without hex digits. A pattern nested tens of thousands of levels deep overflowed
  the stack; patterns nested more than 256 levels are now a syntax error (`S0302`).
- Three differences from jsonata-js found by the differential tests. A regular expression
  applied to a value that is not a string (`x ~> /re/`) now matches the value converted to a
  string as JavaScript does (`undefined`, `[object Object]`, array items joined by commas),
  instead of giving undefined. `$formatNumber` with irregular grouping (`'#,##,##0'`) places the
  separators as jsonata-js does when the number has fewer digits than a grouping position.
  `$formatInteger` in letters (`'a'`, `'A'`) counts in floating point, so numbers above
  2^53 give jsonata-js's letters.
- `$formatInteger` in Roman numerals (`'I'`) of a huge number (`1e21`) tried to write an `M`
  per thousand, which ran past the timeout until memory ran out; it now fails with `U1001`
  beyond `Options::max_string_length`. jsonata-js overflows its stack.
- `$base64decode` reads each UTF-16 code unit of its argument as its low byte, as jsonata-js
  does in Node: a character outside Latin-1 may count as a Base64 digit (`Ĵ` as `4`) instead of
  being ignored.

## [0.5.0]

Guardrails for untrusted expressions and checked compatibility: a cap on the size of a result,
every size cap in the WebAssembly package and the CLI, safer releases, and fixes found by
comparing generated expressions with jsonata-js.

### Added

- A size limit on the result (`jlens-core`): `Options::max_result_size` (256 MiB of compact
  JSON by default) counts a value each time it appears, so an expression that nests the same
  object many times fails with `U1001` instead of exhausting memory when the result is
  converted. It also limits the arguments of host functions; `$string`, `$clone` and the
  transform operator check the JSON text they write against `Options::max_string_length`.

- The WebAssembly package takes every evaluation option of `jlens-core` except cancellation:
  `maxStringLength`, `maxArrayLength` and `maxResultSize` (`U1001`), `now` (milliseconds, a
  `Date` or an ISO 8601 timestamp) and `seed`.

- `jlens eval` and `jlens test` take the size caps: `--max-string-length`,
  `--max-array-length` and `--max-result-size` (`U1001`).

- A security policy, `SECURITY.md`: vulnerabilities are reported privately through GitHub.
  CI now checks the minimum Rust version (1.88) and, with `cargo-deny`, the advisories and
  licenses of the dependencies.

### Fixed

- `*` and `**` over an array nested in itself many times fail with `U1001` beyond
  `Options::max_array_length`, instead of aborting the process when memory runs out.

- Found by differential testing against jsonata-js: `$power(1, x)` and `$power(-1, x)` fail
  with `D3061` when `x` is missing, as `Math.pow` gives NaN there. `$substringBefore` and
  `$substringAfter` without `chars` search for the text "undefined", as jsonata-js does.

## [0.4.0]

Catching mistakes before they run: `jlens check` finds unknown functions, wrong arguments
and undeclared variables with the configuration the editors use, `jlens test` runs test cases,
and each release has prebuilt binaries.

### Added

- `jlens check` and the language service warn about calls of unknown functions (`W0202`, with a
  "did you mean" suggestion) and about calls with a number of arguments the function's
  signature does not accept (`W0203`). Host functions declared to the service, or in
  `jlens.json`, are known.
- They also warn about arguments whose known type the function's signature does not accept
  (`W0205`, as in `$length(1234)` or `$sum(items.name)` with string names), and, when variables
  are declared, about references to variables that are not (`W0204`).
- `jlens check` reads the `jlens.json` of the current directory, or the file given with
  `--config`: files get the schema, variables and host functions that the language server gives
  them. `--schema` replaces the configured schema, and `--path` gives an expression of the
  argument the configuration of a file path. The `config` module of `jlens-lsp` is public.
- `jlens test` runs files of test cases: an expression, and for each case an input, bindings
  and the expected result, undefined result or error code. It reports the failures as text, or
  with `--json` as one JSON object whose expected and actual outcomes have the shape of the
  `eval --json` envelope. A case file may fix `now` and `seed`.
- Each GitHub release has prebuilt `jlens` binaries for Linux (x86-64 and ARM64, static musl
  builds), macOS (Intel and Apple silicon) and Windows (x86-64), with a `SHA256SUMS` file;
  `cargo binstall jlens-cli` installs them.

## [0.3.0]

A language server for editors, reproducible evaluations, and a command line made for scripts
and other programs.

### Added

- Language server: `jlens lsp` (crate `jlens-lsp`) offers completion, diagnostics, hover and
  signature help to editors. A `jlens.json` per workspace folder maps file patterns to a
  schema, variables and host functions. See the editors guide and ADR 0007.
- Reproducible evaluations (`jlens-core`): `Options::now` fixes the timestamp of `$now()` and
  `$millis()`, and `Options::seed` seeds `$random()` and `$shuffle()`.
- `jlens eval --json` prints one JSON envelope per evaluation on standard output:
  `{"ok":true,"result":…}`, `{"ok":true,"undefined":true}` or `{"ok":false,"error":{…}}`,
  with the error's code, message, position, line and column. An input document that is not
  valid JSON is reported with the new code `U2001`. With `--ndjson`, each envelope also
  carries `line`, the number of its input line. See ADR 0008.
- `jlens eval --now TIME` and `--seed N` fix the clock and the random generator, so that a
  command can be replayed with the same output.
- `jlens check -f` takes several files and glob patterns (expanded by `jlens`), and
  `--deny-warnings` makes warnings fail with exit code 1.
- `jlens errors [CODE] [--json]` lists the error and warning codes, or describes one.
- Cancellation (`jlens-core`): a `CancelToken` passed in `Options::cancel_token` stops an
  evaluation from another thread, with the new code `U1003`.
- Size limits (`jlens-core`): an evaluation that builds a string longer than
  `Options::max_string_length` (256 MiB by default) or an array longer than
  `Options::max_array_length` (16,777,216 items) fails with `U1001` instead of running out
  of memory.

### Changed

- `Options` (`jlens-core`) is `#[non_exhaustive]`, so new options can be added without a
  breaking change. Build it with `Options::new().timeout(..).max_depth(..)` instead of a struct
  literal; its fields stay public.
- `jlens check --json` prints one compact array for all the files, and each diagnostic
  carries `file`, `line` and `column`.
- `jlens eval` fails with exit code 2, instead of waiting, when it would read its input from a
  terminal.

## [0.2.0]

Host functions: the application can give expressions its own functions, in Rust, in the
language service and in JavaScript.

### Added

- Host functions (`jlens-core`): the application provides its own functions through
  `Bindings::function`, with an optional signature that is checked like a built-in function's.
  They work with higher-order functions, partial application and `~>`, and take precedence over
  built-in functions of the same name. See ADR 0006.
- Language service (`jlens-schema`): `LanguageService::with_function` declares host functions
  (`FunctionDecl`), which completion, hover and signature help show like built-in functions.
- WebAssembly: JavaScript functions in the bindings, and `Expression.registerFunction`, are
  called as host functions. Their exceptions are rethrown as is; a function that returns a
  promise fails with the new code `U1002`. `LanguageService` takes host function declarations
  as a third argument.
- Web editor: an example selector, with the samples of try.jsonata.org (Invoice, Address, Schema,
  Library and an adapted Bindings) next to the store order rule. The chosen example is kept in
  the address (`#invoice`).

### Changed

- Language service: a call to a declared variable or host function that has the name of a
  built-in function is no longer typed as the built-in's result.
- WebAssembly: bindings that are a function, rather than an object, are a `TypeError`.

## [0.1.1]

No change to the language or the APIs.

### Changed

- The examples in the API documentation (`jlens-core` annotations, the `jlens-schema` overview),
  the book, the fixtures and the web editor use an online store order.
- `cargo xtask wasm-test` prints the browser's console when the browser tests hang.

### Fixed

- Web editor: the text cursor is visible in dark mode.

## [0.1.0]

The first release.

### Language (`jlens-core`)

- A complete JSONata 2.2 engine, ported from jsonata-js 2.2.2: paths, predicates, sorting,
  grouping, the parent operator, positional and context bindings, functions and closures,
  partial application, function composition, transforms, regular expressions (ECMAScript
  syntax), and every built-in function. It passes 1,688 of the 1,690 cases of the official test
  suite; the other two are documented overrides.
- Recovering parsing (`parse_recover`) with diagnostics, and a public syntax tree with node ids
  and spans.
- Evaluation audits: a `Tracer` trait, audit trees at the `decisions` or `full` level with
  capture bounds, explanations, JSON Lines event logs, and an optional `tracing` integration.
  Labels and messages for checks come from `/** @label ... @message ... */` annotations.
- `Document`, to convert an input once and evaluate many expressions against it.
- Guardrails: timeouts, depth limits, a nesting limit, and stack checks, so deep recursion
  fails with an error instead of crashing.

### Schemas and editor support (`jlens-schema`)

- Schema inference from sample documents, overrides, and JSON Schema 2020-12 import and export.
- A language service: completion, diagnostics (including unknown fields, with suggestions),
  hover and signature help.

### Command line (`jlens-cli`)

- `jlens eval` (with NDJSON streaming, `--filter`, `--explain`, `--audit` and `--trace-log`),
  `jlens check`, `jlens functions`, `jlens schema infer` and `jlens schema complete`, and
  `jlens ast`.

### Browser (`jlens-wasm` on npm)

- An ES module with TypeScript declarations: `Expression`, `Document`, `LanguageService`,
  `inferSchema`, `parseRecover` and `functions`. Offsets are in UTF-16 code units.
- `examples/web-editor`: a CodeMirror 6 editor with completion, diagnostics, live results,
  explanations and an audit tree viewer.

### Documentation

- The jlens book: a language reference with verified examples, and guides for the CLI, the
  crates, WebAssembly, auditing and schemas, plus the architecture, performance and decision
  records.

[Unreleased]: https://github.com/thkprado/jlens/compare/v0.13.0...HEAD
[0.13.0]: https://github.com/thkprado/jlens/compare/v0.12.0...v0.13.0
[0.12.0]: https://github.com/thkprado/jlens/compare/v0.11.1...v0.12.0
[0.11.1]: https://github.com/thkprado/jlens/compare/v0.11.0...v0.11.1
[0.11.0]: https://github.com/thkprado/jlens/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/thkprado/jlens/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/thkprado/jlens/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/thkprado/jlens/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/thkprado/jlens/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/thkprado/jlens/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/thkprado/jlens/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/thkprado/jlens/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/thkprado/jlens/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/thkprado/jlens/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/thkprado/jlens/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/thkprado/jlens/releases/tag/v0.1.0
