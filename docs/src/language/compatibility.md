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

Evaluation differences will be listed here as the evaluator is completed.
