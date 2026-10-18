# Changelog

All notable changes are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- A deterministic step limit (ADR 0018): `Options::max_steps` stops an evaluation with the new
  code `U1006` once it has taken that many steps. A step is each node evaluated, each function
  a built-in or a host function calls back, and each 1,024 steps of a regular expression's
  backtracking, so a catastrophic pattern stops too. The same expression, input and options
  take the same steps on every machine, with or without a tracer: the fast paths charge the
  steps they skip. Without a limit, evaluation is as fast as before.
- The audit header reports `steps`, the steps the evaluation took, given to tracers by the new
  `Tracer::steps`. The WebAssembly and Python audits have it too.
- The WebAssembly package takes the step limit as the option `maxSteps`, and takes the same
  steps as the native engine: its tests run the table of exact counts of `jlens-core`, now in
  `crates/jlens-core/tests/steps.json`, in Node.js and in the browser.
- The Python package takes the step limit as the keyword argument `max_steps`, and its tests
  run the same table. `jlens eval` and `jlens test` take it as `--max-steps N`.
- The error envelope of `jlens eval --json` and `jlens test --json` carries the error's `token`
  and `value` when it has them, as the WebAssembly and Python packages do: the step budget of
  `U1006`, or the value at fault of a type error.
- `cargo xtask ab --timeout MS` measures both revisions with a timeout.

### Changed

- A timeout reads the clock every 256 steps instead of at every step, so it fires at most 256
  steps late. On a loop of small steps it cost 47% natively and 149% in WebAssembly; it now
  costs about 1%. On the benchmark workload with a timeout, every case is 3 to 37% faster.
  The cancellation token is still checked at every step.
- `$eval` no longer hides the limits of the evaluation in `D3121`: when its text reaches the
  depth (`D1011`), the timeout (`D1012`), a size limit (`U1001`), a cancellation (`U1003`) or
  the step limit (`U1006`), the error keeps its code, located at the call of `$eval`. Other
  errors of the text are still `D3121`, as in jsonata-js, which wraps the limits too.
- The WebAssembly package reads the options of `evaluate`, `evaluateJson`, `evaluateDocument`
  and `evaluateWithAudit` from the object itself instead of through `JSON.stringify` and a
  parse. `{}` costs about 0.2 µs instead of 0.6 µs, one option 0.4 µs instead of 1.1 µs, and
  `now` given as a `Date` 0.6 µs instead of 31 µs. What `JSON.stringify` decided is now decided
  on purpose:
  - `NaN`, `Infinity` and `-Infinity` are a `TypeError` for every option, as an invalid `Date`
    is for `now`; `JSON.stringify` made them `null`, which silently meant the default.
  - Each option is read once, as a property: a getter is called, an inherited option is read,
    and an exception a getter throws is rethrown as it is. `toJSON` is not called, on the
    options or on an option, and the other properties of the object are not read, so a cycle
    among them is no longer an error.
  - Options must be an object, not an array or a function (a function meant no options).
  - A timeout too long for the clock (above about 10¹⁹ seconds) no longer panics: it never
    expires. A count beyond the address space, such as `maxDepth: 2 ** 40`, limits as much as
    the largest one instead of being a `TypeError`.

## [0.19.0]

jlens gets faster where `jsonatapy`, the Python package of jsonata-core, was ahead: a
result is written straight into Python and JavaScript values, a predicate of literals is
evaluated once, an object constructor over one item skips grouping, `$formatNumber` takes
half the time, and the Python package reads `dict` fields with interned keys and allocates
with mimalloc. With Python values in, jlens is now the fastest on ten of the twelve cases of
`benches/compare/python.py`. The CLI is still not on npm (see 0.14.0).

### Added

- Results written straight into another representation (ADR 0017): implement
  `output::Builder`, and evaluate with `Expression::evaluate_source_to` or
  `evaluate_document_to`, whose closure writes the result with it, within the size limit of a
  result. `evaluate` and the other entry points write their `serde_json` result through the same
  walk, which makes the object constructor case about 6% faster natively.
- `input::Sources`, with `Sink::array_of` and `Sink::object_of`: a source can write the
  arrays and objects of a level as members of one value, so that each costs one allocation
  instead of two (ADR 0010, amended).

### Changed

- A predicate built from literals alone, such as `[[0..19]]`, is evaluated once instead of
  once per item: the number formatting case of the benchmarks takes less than half the time.
- An object constructor over a single item, as in `items.{...}`, builds its object without
  grouping: the object constructor case of the benchmarks takes 22% less time (25% against a
  `Document`), and 20% in Python (24% against a `Document`).
- Python: `evaluate` and `evaluate_document` build their result as Python objects directly,
  without a `serde_json` value in between, and create each key once per result. The object
  constructor case of `benches/compare/python.py` takes 27% less time (31% against a
  `Document`), the transform 14% (24%).
- WebAssembly: `evaluate` and `evaluateDocument` build their result as JavaScript values
  directly (ADR 0017), through one call to a JavaScript helper that decodes each key once,
  instead of printing JSON text for `JSON.parse`. Against a `Document`, the object constructor
  case of `benches/compare/compare.mjs` takes 35% less time and the transform 24%; returning
  the whole input, `$`, 70%. `evaluateJson` still writes JSON text through `serde_json` values.
- Python: the arrays and objects of a level read from Python values cost one allocation each
  instead of two: `$count(items)` over 200 `dict`s takes 19% less time, names over items 6%.
- `$formatNumber` parses its picture once per evaluation, rounds without heap strings and
  detects ties from the bits of the number: the number formatting case of the benchmarks takes
  51% less time (58% against a `Document`), and 41% in Python (53% against a `Document`).
  `$round` with decimals is about three times faster.
- Python: the options of `evaluate`, `evaluate_json`, `evaluate_document` and
  `evaluate_with_audit` are keyword-only parameters instead of `**options`, so that a call
  builds no `dict` for them: two options cost 0.04 µs per call instead of 0.19 µs, and `path`
  of `benches/compare/python.py` takes 0.9 µs instead of 1.1. An unknown keyword is still a
  `TypeError`, whose message now names the method.
- Python: each `Expression` keeps an interned `str` for every field name it reads, made on its
  first evaluation, so that a step into a `dict` neither builds nor hashes a `str`. In
  `benches/compare/python.py`, `names over items` takes 18% less time, the object constructor,
  `group-by`, `filter + aggregate` and `strings` 11% to 15% less, and the shipping rule 20% less.
- Python: the package uses mimalloc as its allocator, as the `jlens` command does (ADR 0014,
  amended). In `benches/compare/python.py` on Linux, ten cases take 4% to 19% less time,
  `recursion` 7% more and the smallest evaluations 0.02 to 0.07 µs more; a wheel is 60 to 70 KB
  larger.

### Fixed

- `$formatNumber` with a `zero-digit` just below U+D800, whose digits would be lone
  surrogates, panicked; those digits are now U+FFFD, as a lone surrogate in a string literal is.
- `$fromMillis` fails with `D1001` for NaN, which `$toMillis` returns for a date that does not
  exist (`$fromMillis($toMillis('2024-13-01'))`), as for a date beyond JavaScript's range. It
  wrote the epoch, "1970-01-01T00:00:00.000Z"; jsonata-js writes "NaN" in every component.
- An order-by whose keys are NaN (`$^(0 / 0)`) orders its items as jsonata-js does, which
  reads NaN as neither equal to nor less than another key and swaps both ways; jlens read it as
  equal and kept the order.

## [0.18.0]

jlens tells an application which fields an expression reads, worked out from the expression
alone: in Rust, JavaScript, Python and on the command line, for one rule or the union of
several. The same answer lets it parse JSON text without the fields no evaluation reads, which
`evaluateJson`, `evaluate_json` and `jlens eval` now do on their own. The CLI is still not on
npm (see 0.14.0).

### Added

- The fields an expression reads (ADR 0016): `Expression::reads` returns a `Reads`, what the
  input and each binding need, as a tree of fields whose leaves are read whole, with the items
  of an array sharing a node. Where the analysis cannot follow (`*`, `**`, `$keys`, `$string`,
  `$eval`, a value given to a host function, a transform, an object in the result), it reads
  the value whole, so the answer is an upper bound: every case of the suite gives the same
  result or error on its input cut down to it. `Reads::union` combines the reads of several
  expressions, `Read::cut` cuts a value down to a read, and both have a JSON form.
- `HostFunction::without_input` marks a host function that does not read the context of its
  call, and `Expression::reads_with` takes the bindings into account, so that a call to a
  marked function reads only its arguments.
- `Expression.reads` and `unionReads` in the WebAssembly package, `Expression.reads` and
  `union_reads` in the Python package, and `jlens reads`, given an expression or several files
  (a result for each, or their union with `--union`). JavaScript and Python functions cannot
  read the context of their call, so both packages mark every one.
- `Document::parse_cut` parses JSON text without the fields a read leaves out: a 1 MB document
  cut to two fields of each record parses in a quarter of the time. `Document.fromJson` in the
  WebAssembly package and `Document.from_json` in the Python package take a read too, so that
  one document serves the union of several expressions.

### Changed

- `evaluateJson`, `evaluate_json` and `jlens eval` parse their input without the fields the
  expression does not read: `jlens eval '$sum(items.price)'` on a 10 MB document runs in 27 ms
  instead of 76, and `--ndjson` in about half the time. The fields skipped are still checked
  for syntax, but a number too large for a float or a lone surrogate escape (`"\ud800"`) in
  one of them no longer fails the parse. `--explain`, `--audit` and `--trace-log` still parse
  the whole input, since they show the values.

## [0.17.0]

jlens comes to Python: the package `jlens` on PyPI evaluates expressions against Python values,
reading only what the expression reaches. A path step now reads its one field of an input object
instead of the whole object, and values nested thousands of levels deep no longer overflow the
stack. The CLI is still not on npm (see 0.14.0).

### Added

- The Python package `jlens` on PyPI (ADR 0014): `Expression` compiles an expression and
  evaluates it against Python values, read as the evaluation goes (`evaluate`), JSON text
  (`evaluate_json`) or a `Document`; bindings, Python functions as host functions, options and
  limits, audits (`evaluate_with_audit`), and `JlensError` with the JSONata code, position and
  token. `abi3` wheels for Python 3.10 and later and `abi3t` wheels for free-threaded Python
  3.15, on the seven release targets, and a source distribution. A guide in the book.
- `input::Source::read_field`: a source can write one field of an object, by its key, which a
  path step reads without the rest of the object.

### Changed

- A path step reads its one field of an input object not read yet, instead of the object's
  every field (ADR 0010, amended): `evaluate` on a `serde_json::Value` takes 18.5 µs on the
  benchmark's names over items, against 152 µs, and 151 µs on the shipping rule, against
  216 µs. Python's `evaluate` reads a `dict` the same way. Arrays and objects read so are kept
  and read once; a scalar is read again each time.

### Fixed

- Values nested thousands of levels deep, read from the input or built by a tail-recursive
  function, no longer overflow the stack. Returning, comparing, serializing, cloning or passing
  one to a host function fails with `U1001` beyond 1,000 levels; dropping one never fails.
- `evaluateWithAudit` in the Wasm package converts to UTF-16 only the spans of the audit's
  nodes: a captured value with a `span` or `range` field (`{start, end}`) was rewritten too
  when the expression held non-ASCII text.

## [0.16.0]

jlens closes its last speed gaps with jsonata-core, another JSONata engine in Rust: group-by is
no longer quadratic in the size of a group, and paths of names and names mapped over many
items take a fraction of the time they did. The CLI is still not on npm (see 0.14.0).

### Changed

- Group-by (`items{type: ...}`) joins the items of each group once, instead of copying the
  group at every item, which made it quadratic in the size of the group. The group-by case of
  the benchmark runs 64% faster, and a group of 200 items 8 times faster.
- A path of names, such as `customer.name` or `$i.code`, looks its fields up directly while no
  array is met, instead of building a sequence at each step: the path case of the benchmark
  runs 67% faster (0.68 to 0.23 µs), sort + map 33%.
- A grouping with a literal key, such as `'total'` in `items{type: {'total': ...}}`, finds its
  group by the literal's text instead of building a string per item: group-by runs 12% faster.
- A path step that reads an array passes it on without copying it, and the step after it maps
  over its items in place: `$sum(items.quantity)` runs 28% faster, `$count(items)` 29%.
- The benchmark has two new cases: a name mapped over the items, and an object constructor.

## [0.15.0]

jlens comes to VS Code: an extension on the Visual Studio Marketplace and Open VSX highlights
JSONata and runs the language server from the bundled release binary. The `jlens` command
itself is faster, with mimalloc as its allocator. The CLI is still not on npm (see 0.14.0).

### Added

- A VS Code extension, `thkprado.jlens` on the Visual Studio Marketplace and Open VSX
  (ADR 0013): highlighting of `.jsonata` files, and the diagnostics, completions and hovers of
  `jlens lsp`, which it starts from the release binary bundled in the package of each release
  platform, from `jlens.server.path`, or from the `PATH`. VS Code checks `jlens.json` against
  its JSON Schema as it is edited. Each release publishes the extension with the same version.
- `cargo xtask ab --b-allocator mimalloc` measures the allocator alone, and `--target` builds
  for another local target, such as musl.

### Changed

- The `jlens` command uses mimalloc as its global allocator on every platform. Evaluations that
  allocate run 28 to 91% faster with the Linux release binaries, which are static musl
  binaries, and 14 to 56% faster on Windows. The library still leaves the choice of allocator
  to applications.

## [0.14.0]

The `jlens` command installs from the package managers people already use: npm
(`npx jlens-cli`, not published yet: see below), Homebrew, Scoop and a container image on ghcr.io. Each channel carries the
attested release binaries, never rebuilt.

### Added

- The pages of the crates on crates.io and of `jlens-wasm` on npm link the book and the web
  editor; `jlens-cli` names the CLI guide as its documentation.
- The `jlens` command on npm (ADR 0012): `npx jlens-cli` or `npm install --save-dev jlens-cli`
  runs the release binary of the platform, which npm installs from one of the packages
  `jlens-cli-<os>-<cpu>` (Linux x64 and arm64, macOS x64 and arm64, Windows x64), published
  with provenance and without install scripts. Not available yet: npm's spam detection held
  back `jlens-cli-win32-x64` at the release, so `jlens-cli` is not published; the four other
  platform packages are.
- The `jlens` command on Homebrew, for macOS and Linux: `brew install thkprado/jlens/jlens`
  installs the release binary from the tap `thkprado/homebrew-jlens`, which each published
  release updates.
- The `jlens` command on Scoop, for Windows: after
  `scoop bucket add jlens https://github.com/thkprado/scoop-jlens`, `scoop install jlens/jlens`
  installs the release binary, and each published release updates the bucket.
- A container image, `ghcr.io/thkprado/jlens`, for `linux/amd64` and `linux/arm64`: the static
  release binary and nothing else, tagged with the version, the minor version and `latest`, and
  attested like the archives.

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

[Unreleased]: https://github.com/thkprado/jlens/compare/v0.19.0...HEAD
[0.19.0]: https://github.com/thkprado/jlens/compare/v0.18.0...v0.19.0
[0.18.0]: https://github.com/thkprado/jlens/compare/v0.17.0...v0.18.0
[0.17.0]: https://github.com/thkprado/jlens/compare/v0.16.0...v0.17.0
[0.16.0]: https://github.com/thkprado/jlens/compare/v0.15.0...v0.16.0
[0.15.0]: https://github.com/thkprado/jlens/compare/v0.14.0...v0.15.0
[0.14.0]: https://github.com/thkprado/jlens/compare/v0.13.0...v0.14.0
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
