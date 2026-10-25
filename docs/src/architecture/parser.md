# Parsing

Parsing turns source text into the tree the evaluator walks. It has three stages, each ported
from `src/parser.js` in jsonata-js:

```text
source ──► lexer ──tokens──► Pratt parser ──raw tree──► process ──► Ast
            (lexer.rs)         (parser.rs)               (process.rs)  (ast.rs)
```

| Entry point | Behaviour |
|---|---|
| `jlens_core::parse(src)` | Stops at the first error and returns it |
| `jlens_core::parse_recover(src)` | Never fails. Errors become `Error` nodes plus diagnostics |
| `Expression::compile(src)` | `parse`, then wraps the tree for evaluation |
| `jlens_core::debug::{tokens, raw_tree}` | Inspection, used by `jlens ast --tokens/--raw` |

## Lexer

The lexer produces one token at a time, on demand, because one character is ambiguous: `/`
starts a regular expression in operand position (`/ab+c/i`) and means division in operator
position (`a / b`). The parser passes an `infix` flag with each request.

Token kinds are names, variables (`$x`), strings, numbers, the literals `true`, `false` and
`null`, regexes and operators. `and`, `or` and `in` are lexed as operators; the parser turns them
back into names when they appear where an operand is expected, as in `and.or`.

Differences from the reference lexer:

- **Spans.** Every token records its full byte range. The reference implementation only records
  the position just after a token.
- **Doc-comments.** `/** ... */` comments are kept and attached to the next token, for
  [annotations](../language/annotations.md). Ordinary comments are skipped as before.
- **Surrogates.** `𝄞`-style escape pairs are combined into one character. A lone
  surrogate becomes U+FFFD, because Rust strings cannot hold one.

## Pratt parser

JSONata's grammar is mostly operators, and a Pratt parser (top-down operator precedence) handles
it with a table of *binding powers*:

| Binding power | Operators |
|---|---|
| 80 | `[` (filter), `(` (call), `@`, `#` |
| 75 | `.` |
| 70 | `{` (group-by), unary `-` operand |
| 60 | `*` `/` `%` |
| 50 | `+` `-` `&` |
| 40 | `=` `!=` `<` `<=` `>` `>=` `in` `^` `~>` `?:` `??` |
| 30 | `and` |
| 25 | `or` |
| 20 | `?` |
| 10 | `:=` |

Each token can define two parse functions:
- `nud`, which parses the token in prefix position, such as a literal, `-x`, `(block)`, `[array]`
  or `{object}`;
- `led`, which parses it in infix position, such as `a + b`, `a[...]`, `f(...)` or `a{...}`.

The core loop is short:

```text
expression(rbp):
    t = current token; advance
    left = t.nud()
    while rbp < binding power of current token:
        t = current token; advance
        left = t.led(left)
    return left
```

A few constructs are desugared while parsing:
- `x ?? y` and `x ?: y` become a fallback node (`NodeKind::Fallback`) that evaluates `x` once,
  then keeps its value if it exists (`??`) or is true as a Boolean (`?:`). jsonata-js desugars
  them into the conditions `$exists(x) ? x : y` and `x ? x : y`, with a copy of `x`: nesting
  them doubles the tree at each level;
- `function($a){...}` is recognised as a lambda when the callee of a call is the name
  `function` or `λ`.

### Annotations and binding power

When the first token of an expression carries a doc-comment, the loop above runs in two phases.
First it parses with binding power `max(rbp, 30)`, which stops at `and` or `or`, and attaches the
annotation to that result. Then it continues with the original `rbp`. Pratt parsing builds the
same tree regardless of where the loop is split, so the annotation marks a subtree without
changing the shape of the tree. The test `annotations_never_change_the_tree` checks this
property.

## Post-processing

`process.rs` is the port of `processAST`. It turns the raw tree into the evaluator's tree:

- **Paths.** Nested `.` operators become one `Path` node with a flat list of steps. String
  steps become names, and number or boolean steps are rejected (`S0213`).
- **Predicates.** `step[expr]` becomes a filter *stage* on the step. On a non-path expression
  such as `$var[0]`, it becomes a *predicate* that applies to the whole result.
- **Bindings and clauses.** `@$v` and `#$i` set `focus` and `index` on the step and switch it to
  tuple mode. `^(...)` becomes a `Sort` step, and `{...}` after a step becomes its `group`.
- **The parent operator `%`.** Each `%` gets an ancestor *slot*. The processor walks back through
  the enclosing path to the step whose input is the wanted ancestor, and marks it to bind that
  slot's label (`!0`, `!1`, …). That step and the steps after it run as a tuple stream. Slots
  live in `Ast::slots`, an arena, because the reference implementation shares and mutates slot
  objects across nodes.
- **Tail calls.** A function call in tail position of a lambda (either branch of a conditional,
  or the last expression of a block) is wrapped in a *thunk*. The evaluator's trampoline runs
  thunks in a loop, so tail-recursive functions don't grow the stack. A call with a predicate or a
  group-by is not wrapped: the trampoline makes the call alone, which would drop them.

Finally, nodes receive sequential `NodeId`s in pre-order. Audit trees will refer to nodes by
these ids.

## Error recovery

In recover mode, an error does not abort parsing. The behaviour follows jsonata-js:
- the error is recorded;
- the rest of the input is consumed;
- the current token becomes an *error marker* with binding power 10, which wraps whatever was
  parsed before it.

So `Account.Order[0;].Product` yields `path(Account, Order[0])` plus error `S0202`.

jlens recovers in more cases than jsonata-js:

- Lexer errors, such as an unterminated string while the user is typing, are recovered.
- Post-processing errors (`S0209`, `S0210`, `S0213`, `S0215`–`S0217`) become diagnostics and
  error nodes.
- An expression that starts at the error marker reuses the recorded error, instead of reporting
  an extra `S0211` for an `undefined` token.

The seven recovery cases from jsonata-js's `test/parser-recovery.js` are ported in
`crates/jlens-core/tests/parser.rs`. They check the same error codes, positions and partial
trees. The same test file also parses every expression of the official suite: all valid
expressions must parse, and all invalid ones must fail with the expected `S0xxx` code.
