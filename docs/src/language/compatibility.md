# Compatibility with jsonata-js

jlens targets the behaviour of jsonata-js **2.2.2**, as specified by its official test suite. It
copies that behaviour, quirks included, unless copying would cost precision, correctness or
performance: a result made inexact by JavaScript's rounding, a result that is wrong by the JSONata
documentation or exposes a JavaScript internal, or a slower common path. This page records
both kinds of decision:
- [Differences](#differences): where jlens behaves otherwise. A difference that affects the
  [conformance suite](../architecture/conformance.md) is also an override there, with its reason.
- [Extensions](#extensions): features of jlens alone, which do not change the meaning of valid
  expressions.
- [Quirks reproduced on purpose](#quirks-reproduced-on-purpose): jsonata-js behaviour that looks
  wrong, which jlens copies because its cost stays in unusual input or error paths, where users
  can expect the same answer from both engines.

## Differences

Most differences have one of four causes: Rust strings are UTF-8 where JavaScript's are UTF-16;
jlens never panics, overflows its stack or exhausts memory on user input, and reports a coded
error instead; an evaluation never changes its input; and a result never depends on the machine,
its time zone or JavaScript's internals.

### Strings and Unicode

| Topic | jsonata-js | jlens |
|---|---|---|
| String representation | UTF-16, may contain lone surrogates | UTF-8 (`String`). A lone surrogate written as `\uD800` becomes U+FFFD |
| `\uXXXX` surrogate pairs | Two code units | Combined into one character, so `"𝄞"` is `𝄞` |
| Error `position` | UTF-16 offset | The same UTF-16 offset, plus a byte `span` |

Two suite cases pass a lone surrogate to `$encodeUrl` / `$encodeUrlComponent` to provoke an
error. jlens cannot represent that input, so it skips those two cases.

### Parsing

- **Error recovery** (`parse_recover`) produces the same codes and positions as jsonata-js on
  its recovery tests. It also recovers from lexer errors and post-processing errors, which
  jsonata-js throws even in recover mode.
- An expression that starts right after a recovered error reuses that error. jsonata-js reports
  an extra `S0211` with an `undefined` token in that situation.
- **Doc-comments** (`/** ... */`) are parsed as [audit annotations](annotations.md). Like any
  comment, they never change how an expression is parsed or evaluated.

### Evaluation

| Topic | jsonata-js | jlens |
|---|---|---|
| Object key order | JavaScript order: integer-like keys first, ascending, then insertion order | Insertion order, always |
| Recursion limit | The JavaScript stack (`RangeError`) | `D1011` after `Options::max_depth` evaluation levels (50,000 by default; 1,000 in Wasm) |
| Nesting limit | The JavaScript stack | `U1001` beyond 1,000 levels of nested subexpressions |
| Very long strings and arrays | Limited by the JavaScript engine (a `RangeError`, or running out of memory) | `U1001` beyond `Options::max_string_length` (256 MiB) or `Options::max_array_length` (16,777,216 items) |
| A very large result, such as an object nested in itself many times | Limited by the JavaScript engine when the result is serialized | `U1001` beyond `Options::max_result_size` (256 MiB of JSON text) |
| A transform that makes an object contain itself (`$ ~> |$|{'self': $}|`) | A circular object, which fails when serialized | `U1001` |
| Several items of an array constructor, or values of an object constructor, fail | The items are evaluated concurrently; the error raised first in the asynchronous schedule wins, which need not be the first item's | The items are evaluated in order; the first failing item's error is reported |
| A group-by (`{...}`) over an empty array in the input | Adds an `undefined` item to that array of the input, which the rest of the expression sees (`($g := items{'k': 1}; $count(items))` gives 1 when `items` is `[]`; jlens gives 0) | The input is never changed |
| A wildcard over an array of sequences (`$map([1, 2], function($e){ items.$ }).*`) | Adds a `true` item after the items of each sequence: an internal flag of the sequence, which its wildcard enumerates | The items only |
| A JavaScript exception inside jsonata-js (`Cannot read properties of undefined`, `comp is not a function`) | Thrown without an error code | A coded error, or a result |
| A regular expression literal on the left of `??` or `?:` (`$match('abc', /b/) ?? 1`) | The parser copies the left side through JSON, which turns the regex into `{}`; `$match` and `$split` then loop until memory runs out, and the timeout cannot stop them | Evaluated as written |
| Error `position` and `token` inside function calls | Overwritten with the call site | Filled in only when the inner error has none, so it points at the real cause |
| Invalid regular expression | JavaScript `SyntaxError` without a code | `S0302` with the engine message in `message` |
| A function as the final result | The function object | `None`, like `JSON.stringify` |
| Very large integers in results | Printed in full up to 1e21 | `serde_json` numbers. Integers above 2^53 become floats (same value, possibly in exponent notation) |
| `$toMillis` of a date-time without an offset | Local time (per `Date.parse`) | UTC, so results never depend on the machine's time zone |
| A regular expression applied to a value that is not a string (`x ~> /re/`) | `RegExp.prototype.exec` converts the value with JavaScript's `String()`: `undefined` is matched as "undefined", so `missing ~> /de/` finds `de`; an object becomes "[object Object]", an array its items joined by commas | Undefined, as for a function given a missing argument. The conversion exposes JavaScript's `String()` |
| `$formatNumber` with irregular grouping (`'#,##,##0'`) on a number with no digit left of a grouping position | Separators at the start or in the wrong place: `$formatNumber(1234, '#,##,##0')` is "12,34," | "1,234": a separator needs a digit on its left, as the picture rules give. Indian grouping of small numbers is ordinary input |
| `$substringBefore`, `$substringAfter` when `chars` is undefined | Search for the text "undefined": `$substringBefore('a-undefined-b', missing)` is "a-" | The string unchanged. The search exposes JavaScript's `String(undefined)` |
| A date/time picture marker with an unknown component and a presentation (`$fromMillis(0, '[Qx]')`, `'[,2]'`) | The text "undefined" in the result. With no presentation (`'[Q]'`) it fails with `D3132` | `D3132` in every case. The text exposes JavaScript's `undefined`. `$toMillis` gives `D3133` for both, as jsonata-js does |
| `$parseInteger` with a picture whose digits are not ASCII, on text with characters below that digit family (`$parseInteger('12', '##０')`, `$parseInteger('１２x', '０')`) | A `RangeError` without a code (`Invalid code point`) | The character ends the number, as any other non-digit does: NaN for the first, 12 for the second |
| `$split(s, "")` | Splits into UTF-16 code units | Splits into characters (astral characters stay whole) |
| `$random`, `$shuffle` | `Math.random` | A per-evaluation xorshift generator seeded from the clock |
| Regular expressions | JavaScript `RegExp` | `jlens-regress`, a fork of `regress` 0.12.0 (ECMAScript syntax and semantics). Match positions are UTF-16 offsets. A pattern matches characters, not UTF-16 code units: `$match('a𝄞', /[^a]/)` matches `𝄞`, where jsonata-js matches its first half, a lone surrogate. A pattern nested more than 256 levels deep is a syntax error (`S0302`) |
| A regular expression that backtracks catastrophically, such as `/^(a+)+$/` on a long run of `a` followed by `!` | Runs until the match ends, which can take years, and blocks the thread | The same without limits. With `Options::timeout` or a cancellation token, the match stops with `D1012` or `U1003`, like any other step ([ADR 0009](../adr/0009-bounded-regex.md)) |

The suite runner's own guard raises `U1001` for its `timelimit`/`depth` cases; jlens enforces
the same limits through `Options` and reports `D1011`/`D1012`, which the conformance harness
accepts in place of `U1001`.

### Host functions

Functions provided by the application through the bindings (`Bindings::function` in Rust,
functions in the bindings object or `registerFunction` in the WebAssembly package):

| Topic | jsonata-js | jlens |
|---|---|---|
| Arguments and result | The JavaScript values themselves, which the function may modify | JSON copies: the function cannot change the values of the evaluation |
| A function as an argument (`$f($g)`) | The function, which the host can call | `None` (undefined) |
| The context value | `this`: the context value for a function in the bindings, `{environment, input}` for one registered with `registerFunction`. An input array arrives in its wrapper | `HostCall::input`, the context value itself, an input array included. In WebAssembly, `this` is `undefined`: a signature with `-` passes the context value as an argument |
| A function that returns a promise | The evaluation awaits it | `U1002` |
| An exception thrown by the function | Rethrown, with the `position` and `token` of the call | Rethrown as is in WebAssembly; `position` and `token` are added when missing |

## Extensions

These features exist only in jlens. Expressions that use them remain valid JSONata, because the
extensions live in comments or in the host API, not in the language syntax.

- Audit annotations and the warnings `W0101`–`W0103`.
- `Ast` node ids and spans, and `parse_recover` diagnostics with severities.
- Static warnings (`W0201`–`W0205`) and the language service.
- `Document`, for evaluating many expressions against one converted document.

## Quirks reproduced on purpose

Both engines behave as described here. Each row gives the reason for copying it.

| Topic | jsonata-js and jlens | Decision |
|---|---|---|
| `$formatInteger` in letters (`'a'`, `'A'`) of a number above 2^53 | Computed in floating point: `value - 1` rounds to `value`, and the letters follow that rounding | Kept. The number is already a rounded double, so exact arithmetic on it gains no real precision, and letter numbering of such numbers is not ordinary input |
| `$power(±1, e)` with `e` undefined | `D3061`: JavaScript's `Math.pow` gives NaN for 1^NaN, where IEEE `pow` gives 1 | Kept. Every base fails with `D3061` when the exponent is undefined, ±1 included |
| `$base64encode`, `$base64decode` | Take the low byte of each UTF-16 code unit (Node's `Buffer`), so `$base64decode` reads `Ĵ` (U+0134) as the digit `4`, and text outside Latin-1 does not round-trip | Kept. Latin-1 text gives the same Base64 in both engines, which data exchanged between them relies on; encoding UTF-8 instead would change the result for any text outside ASCII |
