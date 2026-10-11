# Compatibility with jsonata-js

jlens targets the behaviour of jsonata-js **2.2.2**, as specified by its official test suite.
This page lists every known difference. Each difference is either:
- a documented override in the [conformance suite](../architecture/conformance.md), or
- an extension that does not change the meaning of valid expressions.

## Strings and Unicode

| Topic | jsonata-js | jlens |
|---|---|---|
| String representation | UTF-16, may contain lone surrogates | UTF-8 (`String`). A lone surrogate written as `\uD800` becomes U+FFFD |
| `\uXXXX` surrogate pairs | Two code units | Combined into one character, so `"𝄞"` is `𝄞` |
| Error `position` | UTF-16 offset | The same UTF-16 offset, plus a byte `span` |

Two suite cases pass a lone surrogate to `$encodeUrl` / `$encodeUrlComponent` to provoke an
error. jlens cannot represent that input, so it skips those two cases.

## Parsing

- **Error recovery** (`parse_recover`) produces the same codes and positions as jsonata-js on
  its recovery tests. It also recovers from lexer errors and post-processing errors, which
  jsonata-js throws even in recover mode.
- An expression that starts right after a recovered error reuses that error. jsonata-js reports
  an extra `S0211` with an `undefined` token in that situation.
- **Doc-comments** (`/** ... */`) are parsed as [audit annotations](annotations.md). Like any
  comment, they never change how an expression is parsed or evaluated.

## Extensions

These features exist only in jlens. Expressions that use them remain valid JSONata, because the
extensions live in comments or in the host API, not in the language syntax.

- Audit annotations and the warnings `W0101`–`W0103`.
- `Ast` node ids and spans, and `parse_recover` diagnostics with severities.
- Static warnings (`W0201`–`W0205`) and the language service.
- `Document`, for evaluating many expressions against one converted document.

## Evaluation

| Topic | jsonata-js | jlens |
|---|---|---|
| Object key order | JavaScript order: integer-like keys first, ascending, then insertion order | Insertion order, always |
| Recursion limit | The JavaScript stack (`RangeError`) | `D1011` after `Options::max_depth` evaluation levels (50,000 by default; 1,000 in Wasm) |
| Nesting limit | The JavaScript stack | `U1001` beyond 1,000 levels of nested subexpressions |
| Very long strings and arrays | Limited by the JavaScript engine (a `RangeError`, or running out of memory) | `U1001` beyond `Options::max_string_length` (256 MiB) or `Options::max_array_length` (16,777,216 items) |
| A very large result, such as an object nested in itself many times | Limited by the JavaScript engine when the result is serialized | `U1001` beyond `Options::max_result_size` (256 MiB of JSON text) |
| A transform that makes an object contain itself (`$ ~> |$|{'self': $}|`) | A circular object, which fails when serialized | `U1001` |
| Error `position` and `token` inside function calls | Overwritten with the call site | Filled in only when the inner error has none, so it points at the real cause |
| Invalid regular expression | JavaScript `SyntaxError` without a code | `S0302` with the engine message in `message` |
| A function as the final result | The function object | `None`, like `JSON.stringify` |
| Very large integers in results | Printed in full up to 1e21 | `serde_json` numbers. Integers above 2^53 become floats (same value, possibly in exponent notation) |
| `$toMillis` of a date-time without an offset | Local time (per `Date.parse`) | UTC, so results never depend on the machine's time zone |
| `$split(s, "")` | Splits into UTF-16 code units | Splits into characters (astral characters stay whole) |
| `$base64encode` | Takes the low byte of each UTF-16 code unit | The same; text outside Latin-1 does not round-trip, as in jsonata-js |
| `$random`, `$shuffle` | `Math.random` | A per-evaluation xorshift generator seeded from the clock |
| Regular expressions | JavaScript `RegExp` | `regress` (ECMAScript syntax and semantics). Match positions are UTF-16 offsets |

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
